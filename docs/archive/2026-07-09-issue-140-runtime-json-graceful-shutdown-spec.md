# Spec — #140: signal-robust removal of `runtime.json` (graceful shutdown hook)

- Issue: jaunder-org/jaunder#140
- Milestone: Emacs blogging front-end (#4)
- Status: draft (awaiting approval)
- Date: 2026-07-09

## Problem

`jaunder serve` writes `runtime.json` (`{ip, port}`, per ADR-0035) once the
listener binds, and a `RuntimeFileGuard` (`server/src/runtime_file.rs`) removes
it on `Drop`. But the serve loop is a bare
`axum::serve(listener, router).await?` (`server/src/commands.rs:490`): `Drop`
only runs when that future returns on its own. A `SIGINT`/`SIGTERM` terminates
the process **without unwinding**, so the guard's `Drop` never runs and
`runtime.json` is leaked. There is currently **no signal handling anywhere in
`server/`**.

ADR-0035 explicitly lists this as the first deferred follow-on: "signal-robust
removal (a graceful-shutdown hook so the file is reliably removed on
SIGTERM/SIGINT)".

## Goal

Give `jaunder serve` a graceful-shutdown hook so that a normal service stop
(systemd sending `SIGTERM`, or an interactive `Ctrl-C`) drains in-flight
requests, removes `runtime.json`, and exits cleanly — making the file a reliable
"an instance is running here" signal on the normal-stop path.

## Design decisions (resolved in the interview)

1. **Mechanism: axum `with_graceful_shutdown`, removal via the existing RAII
   guard.** The serve future becomes
   `axum::serve(listener, router).with_graceful_shutdown(shutdown_future)`. When
   the shutdown future resolves, axum stops accepting new connections, drains
   in-flight requests, and the serve future returns — the **normal unwind** then
   runs `RuntimeFileGuard::Drop`, which removes the file. We do **not** remove
   the file from inside a signal handler on the graceful path; the existing
   guard is the single removal path.

2. **Signals: `SIGINT` and `SIGTERM`, unix.** Handled via `tokio::signal::unix`
   (`tokio` is already built with `"full"`, so no new runtime dependency).
   `SIGINT` covers interactive `Ctrl-C`; `SIGTERM` covers `systemctl stop`. The
   shutdown wiring is `#[cfg(unix)]` (jaunder targets Linux/NixOS).

3. **Drain: unbounded.** After the first signal we stop accepting and wait for
   in-flight requests to finish, with **no** drain timeout. systemd's
   `TimeoutStopSec` bounds the wait in production (escalating to `SIGKILL`), and
   #141's stale-detection recovers any file left by a `SIGKILL`. No new
   timeout/config knob is introduced.

4. **Second signal forces immediate exit, still cleaning up.** A second
   `SIGINT`/`SIGTERM` received _while draining_ is a panic button: it does a
   **best-effort** removal of `runtime.json` (ignoring any error) and then calls
   `std::process::exit`, terminating immediately regardless of a stuck drain.
   Because `process::exit` skips `Drop`, this path removes the file explicitly
   so #140's guarantee holds even under a forced abort. This requires the
   shutdown coordinator to be able to remove the file without the RAII guard —
   via a shared best-effort removal primitive `remove_runtime_file(path)`
   (called by both `Drop` and the force path) and a `RuntimeFileGuard::path()`
   accessor so the supervisor can clone the path before the guard is moved into
   `serve_with_shutdown`.

   **Concurrency structure (the mechanism).** `with_graceful_shutdown` takes a
   _single_ future that resolves on the first signal and then drains internally
   — it gives no "first signal seen, now watch for a second" callback. So the
   trigger is decoupled from the signal watcher via a **shared trigger** (a
   `tokio::sync::oneshot` or `Notify`):
   - The future handed to `with_graceful_shutdown` simply awaits the trigger.
   - **One** supervising task owns the signal streams and awaits signals
     **sequentially** on the same streams: first `recv()` → log + fire the
     trigger (begin drain); a second `recv()` on the same streams → `WARN` +
     best-effort `remove(path)` + `std::process::exit`. Because both waits are
     sequential `recv()`s on the _same_ streams, signal #1 is consumed by the
     first wait and signal #2 by the second — no double-fire. (Two independent
     concurrent streams would both receive signal #1 and wrongly force-exit on
     the first signal; that is why it is one task, sequential.)
   - The serve future returning first (drain finished before any second signal)
     unwinds normally → `RuntimeFileGuard::Drop` removes the file; the parked
     supervising task is abandoned as the process returns from `main`.

5. **Logging.** One `INFO` on first-signal receipt ("received <signal>, shutting
   down gracefully"); one `WARN` on the forced second-signal exit. No PII
   (ADR-0011).

6. **Testability — the seam.** The graceful-shutdown wiring is extracted into a
   new host-testable function **in `server/src/commands.rs`** (beside
   `cmd_serve`), with roughly:

   ```rust
   async fn serve_with_shutdown(
       listener: TcpListener,
       router: Router,
       runtime_guard: RuntimeFileGuard,   // moved in: Drop fires on return
       shutdown: impl Future<Output = ()> + Send + 'static,
   ) -> anyhow::Result<()>
   ```

   It builds `axum::serve(listener, router).with_graceful_shutdown(shutdown)`,
   awaits it, and returns. It **owns** the guard, so a normal return drops the
   guard and removes the file — this is the covered removal path. `cmd_serve`
   clones the runtime-file path out (via `RuntimeFileGuard::path()`) **before**
   moving the guard into `serve_with_shutdown`, and hands that clone to the
   supervising task for the forced-exit removal.

   The supervisor is split so its **synchronous** part (installing the signal
   streams) can be host-covered while only the **async** wait-loop is
   `cov:ignore`'d: a
   `#[cfg(unix)] fn spawn_shutdown_supervisor(runtime_path: Option<PathBuf>) -> io::Result<oneshot::Receiver<()>>`
   creates the `SignalKind::interrupt`/`terminate` streams **synchronously** (so
   the handlers are installed before it returns — no raise-before-handler race),
   then `tokio::spawn`s the wait-loop and returns the trigger receiver. The
   `cmd_serve` glue that wires this to `prepare_server` output is not reached by
   any host test (the sole `cmd_serve` test fails early at `prepare_server`) and
   is `cov:ignore`'d, matching the repo's existing treatment of the serve loop
   (jaunder-uox1). The **only** `cov:ignore`'d logic is: the supervisor's async
   wait-loop (`recv`/`select`/trigger-send + the forced-exit
   `remove_runtime_file`
   - `process::exit`) and the `cmd_serve` glue. Everything else — the
     `serve_with_shutdown` body, `spawn_shutdown_supervisor`'s synchronous
     stream creation, guard `Drop`, `remove_runtime_file`, `path()` — is
     executed by host tests.

   **Testing — in-process Rust unit tests that send real signals (AC1, AC2,
   AC4).** `cargo nextest` runs each test in its own process, and once a
   `tokio::signal` handler is installed the signal's default "terminate"
   disposition is replaced (delivered to the handler, not killing the process).
   So a `#[cfg(unix)]` unit test in `server/src/commands.rs` can: bind a
   `TcpListener` on port 0, build a **trivial `Router::new()`** (no
   `prepare_server`, no DB), write the guard via `RuntimeFileGuard::write`, call
   `spawn_shutdown_supervisor` (installs the handlers), spawn
   `serve_with_shutdown` with the returned trigger, then
   `nix::sys::signal::raise(SIGTERM)` (resp. `SIGINT`) **to itself** and poll
   until `runtime.json` is gone — awaiting the serve task's `JoinHandle` so the
   removal (guard `Drop` on return) is deterministically observed. This
   exercises the **real** signal → handler → drain → removal path in-process.
   Two tests, one per signal.

   A Rust _subprocess_ test cannot serve at all: the `CARGO_BIN_EXE_jaunder`
   test build has `cheap-kdf` enabled (feature unification), so `main`
   fail-closes (`exit(1)`) before serving (`server/src/main.rs:13`; cf.
   `server/tests/misc/cli_subprocess.rs`) — which is why the real signal is
   raised **in-process**, not sent to a child.

   Two caveats: (a) the **second-signal forced-exit** branch is not
   unit-testable — it ends in `process::exit`, which would kill the test process
   — so it stays `cov:ignore`'d + inspection, with its `remove_runtime_file`
   call covered by AC6; the signal tests raise a single signal and exercise the
   graceful path. (b) These tests rely on nextest's **process-per-test
   isolation** (they raise a real signal to self); under a bare `cargo test`
   (libtest, shared process) two such tests could observe each other's signals.
   The gate runs nextest; a module comment states the requirement.

7. **Signal-send in the unit test: `nix` as a `[dev-dependency]`.** The `server`
   crate gains `nix` (with the `signal` feature) under `[dev-dependencies]`
   only, used as `nix::sys::signal::raise(Signal::SIGTERM)`. This is the only
   Cargo change; nothing is added to `[dependencies]`.

8. **Documentation.** Amend ADR-0035 (the owner of the runtime.json contract):
   update its **lifecycle** note to state that removal is now signal-robust on
   `SIGINT`/`SIGTERM` (delivered by #140), and mark that first deferred
   follow-on as done. No new ADR — the shutdown semantics are a fill-in of the
   lifecycle ADR-0035 already defines, not a fresh decision that supersedes it.

## Out of scope (explicitly deferred)

- **`pid` field + start-up mutex / stale detection** — #141.
- **`admin_token` + `jaunder shut-down` admin control channel** — #142.
- **Configurable drain timeout** — not requested; unbounded drain + systemd
  bound is sufficient.
- **Windows signal handling** — jaunder targets Linux/NixOS; the shutdown wiring
  is `#[cfg(unix)]`.
- **Graceful teardown semantics of the backup/feed schedulers** — they are
  dropped as today; #140 does not change scheduler shutdown.

## Acceptance criteria (observable)

Each is stated so ship-time conformance review can tell delivered from not.

- **AC1 — SIGTERM removes the file (real signal, in-process).** With the serve
  wiring running and `runtime.json` present, raising `SIGTERM` to the process
  drives the real handler → drain → `serve_with_shutdown` return, and
  `runtime.json` no longer exists once the serve task completes. _(in-process
  `#[cfg(unix)]` Rust unit test in `server/src/commands.rs` that
  `nix::signal::raise(SIGTERM)`s itself.)_
- **AC2 — SIGINT is handled equivalently (real signal, in-process).** Same as
  AC1 but raising `SIGINT`: `runtime.json` is removed. _(second in-process unit
  test — SIGINT is exercised, not merely inspected.)_
- **AC3 — in-flight requests drain.** The first signal stops accepting new
  connections and lets in-flight requests finish before the process exits. This
  is `axum::serve(...).with_graceful_shutdown(...)`'s documented library
  guarantee; #140 does not hand-roll a drain. _(verified by inspection that the
  serve path uses `with_graceful_shutdown` — not a bespoke drain assertion, per
  the deliberate choice not to re-test the framework.)_
- **AC4 — the removal wiring is host-covered.** The serve path is decomposed so
  that `serve_with_shutdown` (drain → return → guard `Drop` removes the file)
  and `spawn_shutdown_supervisor`'s synchronous stream setup are executed by the
  in-process signal tests (AC1/AC2), which await the serve task's completion —
  so the coverage of the removal wiring is deterministic for the stateless gate,
  with only the async wait-loop, forced-exit branch, and unreachable `cmd_serve`
  glue `cov:ignore`'d. _(same in-process tests; verified by
  `cargo xtask check`.)_
- **AC5 — a second signal still terminates with cleanup.** After graceful
  shutdown has begun, a second `SIGINT`/`SIGTERM` terminates the process and
  best-effort removes `runtime.json` before exiting. _(the forced-exit branch is
  `cov:ignore`'d — it ends in `process::exit`, and is observably identical to a
  graceful win (gone process, removed file); its removal call is
  `RuntimeFileGuard::remove`, covered directly by AC6. Verified by inspection +
  AC6, not by a race-dependent E2E.)_
- **AC6 — `remove_runtime_file` is idempotent and tested.** Calling it when the
  file is present removes it; calling it when the file is absent is a no-op that
  does not error; a subsequent guard `Drop` (which calls the same primitive) is
  harmless. `RuntimeFileGuard::path()` returns `Some` for an active guard and
  `None` for an inert one. _(unit tests in `server/src/runtime_file.rs`)_
- **AC7 — no new runtime dependency.** The shutdown path uses `tokio::signal`
  (tokio is already `"full"`); **nothing** is added to `[dependencies]`. `nix`
  is added **only** under `server`'s `[dev-dependencies]` (for the test's
  `raise`). _(Cargo.toml diff shows a dev-dep only; `cargo deny` passes.)_
- **AC8 — existing behavior preserved.**
  `prepare_server_writes_then_removes_runtime_file` and the existing
  `runtime_file.rs` unit tests still pass; normal serve-loop-return removal is
  unchanged. _(existing tests green)_
- **AC9 — gate green.** `cargo xtask validate --no-e2e` passes (static + clippy
  - coverage), with any `cov:ignore`/`crap:allow` markers carrying a
    justification, and no unapproved lint suppressions.
- **AC10 — ADR-0035 lifecycle note updated** to state removal is signal-robust
  on `SIGINT`/`SIGTERM` and to mark that first deferred follow-on done. _(diff
  to `docs/adr/0035-elisp-live-integration-harness.md`.)_
