# #140 — `runtime.json` Graceful-Shutdown Hook Implementation Plan

> **For agentic workers:** Execute this plan task-by-task with
> **jaunder-iterate** (delegating an individual task to a subagent via
> **jaunder-dispatch** when useful). Steps use checkbox (`- [ ]`) syntax for
> tracking.

**Goal:** `jaunder serve` removes `runtime.json` reliably on `SIGINT`/`SIGTERM`,
not only on a normal serve-loop return.

**Architecture:** Extract a host-testable `serve_with_shutdown` that runs
`axum::serve(...).with_graceful_shutdown(trigger)` and **owns** the
`RuntimeFileGuard` (so a normal return drops it → removes the file). A
`#[cfg(unix)]` `spawn_shutdown_supervisor` installs the signal streams
**synchronously** (no raise-before-handler race), then spawns one task that
fires the trigger on the first signal and force-exits (best-effort removing the
file) on a second. The real signal path is proven by in-process unit tests that
raise a real `SIGTERM`/`SIGINT` to self — isolated by nextest's process-per-test
model, non-fatal because the tokio handler is installed first.

**Tech Stack:** Rust, `axum` 0.8, `tokio` (already `"full"` → `tokio::signal`),
`nix` (dev-dep, for the test's `raise`), Nix checks.

**Spec:**
`docs/superpowers/specs/2026-07-09-issue-140-runtime-json-graceful-shutdown.md`
— this plan is "how"; the spec is "what/why". Task N ↔ acceptance criteria
noted inline.

## Global Constraints

_Every task's requirements implicitly include these._

- **No new runtime dependency** — nothing added to `[dependencies]`. The
  shutdown path uses `tokio::signal::unix` (tokio is already `"full"`). `nix` is
  added **only** to `server`'s `[dev-dependencies]` (spec AC7).
- **No `.unwrap()`/`.expect()` in production code** — clippy
  `unwrap_used`/`expect_used` are denied outside `#[cfg(test)]`. Handle every
  `Result` (incl. signal-handler registration) explicitly. (Both are fine in
  `#[cfg(test)]`.)
- **`cov:ignore` only** the supervisor's async wait-loop (incl. the forced-exit
  `process::exit` branch) and the unreachable `cmd_serve` serve-glue, each with
  a justifying comment. Coverable logic (`serve_with_shutdown` body,
  `spawn_shutdown_supervisor`'s synchronous stream setup, guard `Drop`,
  `remove_runtime_file`, `path()`) must **not** hide inside a `cov:ignore`
  region. (spec decision 6)
- **`#[cfg(unix)]`** gates the signal wiring and the signal tests; jaunder
  targets Linux/NixOS.
- **Unbounded drain** — no timeout, no config knob. (spec decision 3)
- **Commits:** Conventional Commits; run `cargo xtask check` clean before
  committing (**jaunder-commit**). **No `Co-Authored-By` trailer.**
- **Out of scope:** `pid`/start-up mutex (#141), `admin_token`/`shut-down`
  (#142), drain timeout, Windows.

## Task list (reviewer summary)

1. **`runtime_file.rs` — removal primitive + `path()` accessor** (prep refactor;
   AC6).
2. **`commands.rs` — `serve_with_shutdown` + `spawn_shutdown_supervisor` +
   `cmd_serve` rewrite + in-process real-signal tests; add `nix` dev-dep** (the
   behavior change + its definitive tests; AC1, AC2, AC3, AC4, AC5, AC7).
3. **Docs — amend ADR-0035 lifecycle note** (AC10).

**Key risks/decisions:**

- The real-signal test is **in-process** (raise to self), not a subprocess: the
  `CARGO_BIN_EXE_jaunder` test build fail-closes (cheap-kdf) before serving.
  Safe because nextest isolates each test in its own process and the tokio
  handler is installed before the raise (replacing the default terminate
  disposition).
- The signal supervisor installs streams **synchronously** (before returning) so
  the test's `raise` can't beat handler installation, and so the setup lines are
  host-covered while only the async wait-loop is `cov:ignore`'d.
- The forced-exit (second-signal) branch can't be unit-tested (`process::exit`);
  it stays `cov:ignore`'d, its `remove_runtime_file` call covered by Task 1
  (AC6).
- These signal tests assume nextest process isolation; a module comment says so.

---

### Task 1: `runtime_file.rs` — removal primitive + `path()` accessor

**Files:**

- Modify: `server/src/runtime_file.rs` (guard + `Drop` at `:29-69`; tests at
  `:71-127`)

**Interfaces:**

- Consumes: nothing new.
- Produces:
  - `pub(crate) fn remove_runtime_file(path: &std::path::Path)` — best-effort
    `remove_file`, ignoring errors.
  - `impl RuntimeFileGuard { pub fn path(&self) -> Option<&std::path::Path> }` —
    the active path (`None` for an inert guard).
  - `Drop for RuntimeFileGuard` now delegates to `remove_runtime_file`.

- [x] **Step 1: Write the failing tests** — append to the
      `#[cfg(test)] mod tests` in `server/src/runtime_file.rs`:

```rust
#[test]
fn remove_runtime_file_deletes_when_present() {
    let dir = TempDir::new().unwrap();
    let path = dir.path().join("runtime.json");
    std::fs::write(&path, "{}").unwrap();
    assert!(path.exists());
    remove_runtime_file(&path);
    assert!(!path.exists());
}

#[test]
fn remove_runtime_file_is_noop_when_absent() {
    let dir = TempDir::new().unwrap();
    let path = dir.path().join("runtime.json");
    // Must not panic and must not create the file; idempotent on repeat.
    remove_runtime_file(&path);
    remove_runtime_file(&path);
    assert!(!path.exists());
}

#[test]
fn path_is_some_for_active_guard_and_none_for_inert() {
    let dir = TempDir::new().unwrap();
    let active = RuntimeFileGuard::write(dir.path().join("runtime.json"), addr());
    assert!(active.path().is_some());
    let inert = RuntimeFileGuard::write(
        std::path::Path::new("/nonexistent-jaunder-xyz/sub/runtime.json").to_path_buf(),
        addr(),
    );
    assert!(inert.path().is_none());
}
```

- [x] **Step 2: Run the tests, verify they fail**

Run: `cargo nextest run -p jaunder --lib runtime_file` Expected: FAIL —
`remove_runtime_file` and `path` are not defined.

- [x] **Step 3: Implement against the tests**

Add the free fn and accessor, and route `Drop` through the fn (the single
removal primitive shared by `Drop` and — in Task 2 — the forced exit). Bodies
are one line each, pinned by the tests above:

```rust
/// Best-effort removal of the runtime file at `path`, ignoring errors (it may
/// already be gone). Shared by `RuntimeFileGuard::drop` and the forced-shutdown
/// path in `cmd_serve`, which must remove explicitly because `process::exit`
/// skips `Drop`.
pub(crate) fn remove_runtime_file(path: &Path) {
    let _ = std::fs::remove_file(path);
}
```

```rust
impl RuntimeFileGuard {
    // ...existing write / for_serve...

    /// The active runtime-file path, or `None` for an inert guard (write failed).
    /// Lets the shutdown supervisor clone the path before the guard is moved into
    /// the serve future.
    #[must_use]
    pub fn path(&self) -> Option<&Path> {
        self.path.as_deref()
    }
}

impl Drop for RuntimeFileGuard {
    fn drop(&mut self) {
        if let Some(p) = &self.path {
            remove_runtime_file(p);
        }
    }
}
```

Update the module doc (`:25-28`) so the "signal-robust removal is a deferred
follow-on (#140)" caveat reflects that #140 now delivers it (graceful path via
`Drop`, forced path via `remove_runtime_file`).

- [x] **Step 4: Run the tests, verify they pass**

Run: `cargo nextest run -p jaunder --lib runtime_file` Expected: PASS (new
tests + existing `writes_ip_and_port_json`, `removes_file_on_drop`,
`write_failure_yields_inert_guard`, `for_serve_*`).

- [x] **Step 5: Commit**

Run `cargo xtask check` first (fmt + clippy + coverage) so pre-commit passes
clean (**jaunder-commit**).

```bash
git add server/src/runtime_file.rs
git commit -m "refactor(serve): add remove_runtime_file primitive + guard path() accessor"
```

---

### Task 2: `commands.rs` — graceful shutdown + in-process signal tests

**Files:**

- Modify: `server/Cargo.toml` (`[dev-dependencies]`)
- Modify: `server/src/commands.rs` — `cmd_serve` (`:467-492`); add
  `serve_with_shutdown` and `spawn_shutdown_supervisor`; add tests to
  `#[cfg(test)] mod tests` (`:494`)

**Interfaces:**

- Consumes: `remove_runtime_file`, `RuntimeFileGuard::{write,path}` (Task 1);
  `PreparedServer` (`:352-362`); `tokio::signal::unix::{signal, SignalKind}`;
  `tokio::sync::oneshot`; `nix::sys::signal::{raise, Signal}` (test only).
- Produces:
  - `async fn serve_with_shutdown(listener: tokio::net::TcpListener, router: axum::Router, runtime_guard: crate::runtime_file::RuntimeFileGuard, shutdown: impl std::future::Future<Output = ()> + Send + 'static) -> anyhow::Result<()>`
    (owns the guard; drops it on return).
  - `#[cfg(unix)] fn spawn_shutdown_supervisor(runtime_path: Option<std::path::PathBuf>) -> std::io::Result<tokio::sync::oneshot::Receiver<()>>`
    (installs signal streams synchronously, spawns the wait-loop, returns the
    trigger receiver).

> **Guard note:** the `test-backend-pattern` guard scans only `server/tests/`
> and `storage/src/`, **not** `server/src/`, so these in-file tests need no
> backend annotation (and touch no DB).

- [x] **Step 1: Add the `nix` dev-dependency**

In `server/Cargo.toml`, under `[dev-dependencies]`:

```toml
nix = { version = "0.29", features = ["signal"] }
```

(Use the current `nix` release; confirm `cargo deny check` accepts it in Step
6.)

- [x] **Step 2: Write the failing tests** — append to `mod tests` in
      `server/src/commands.rs`. A module-level note documents the nextest
      requirement:

```rust
// The two shutdown tests below raise a REAL signal to their own process. This is
// safe only under `cargo nextest` (one process per test) — the tokio handler,
// installed synchronously by spawn_shutdown_supervisor before we raise, replaces
// the default terminate disposition so the raise is delivered to the handler
// instead of killing us. Under a bare `cargo test` (libtest, shared process) two
// such tests could observe each other's signals; the gate runs nextest.

#[cfg(unix)]
async fn assert_signal_removes_runtime_file(signal: nix::sys::signal::Signal) {
    let dir = TempDir::new().unwrap();
    let path = dir.path().join("runtime.json");
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let guard = crate::runtime_file::RuntimeFileGuard::write(path.clone(), addr);
    assert!(path.exists(), "guard wrote the runtime file");

    // Installs the SIGINT/SIGTERM handlers synchronously, so the raise below
    // cannot beat handler installation.
    let shutdown_rx = spawn_shutdown_supervisor(Some(path.clone())).unwrap();
    let handle = tokio::spawn(serve_with_shutdown(
        listener,
        axum::Router::new(),
        guard,
        async move {
            let _ = shutdown_rx.await;
        },
    ));

    nix::sys::signal::raise(signal).unwrap();

    // Await serve completion so removal (guard Drop on return) is observed
    // deterministically — not a timing poll.
    handle
        .await
        .unwrap()
        .expect("serve_with_shutdown returns Ok on graceful shutdown");
    assert!(!path.exists(), "runtime.json removed after {signal:?}");
}

#[cfg(unix)]
#[tokio::test]
async fn sigterm_drains_and_removes_runtime_file() {
    assert_signal_removes_runtime_file(nix::sys::signal::Signal::SIGTERM).await;
}

#[cfg(unix)]
#[tokio::test]
async fn sigint_drains_and_removes_runtime_file() {
    assert_signal_removes_runtime_file(nix::sys::signal::Signal::SIGINT).await;
}
```

- [x] **Step 3: Run the tests, verify they fail**

Run:
`cargo nextest run -p jaunder --lib -E 'test(drains_and_removes_runtime_file)'`
Expected: FAIL to compile — `serve_with_shutdown` / `spawn_shutdown_supervisor`
not defined.

- [x] **Step 4: Implement `serve_with_shutdown`, `spawn_shutdown_supervisor`,
      rewire `cmd_serve`**

Add `serve_with_shutdown` (signature pinned by the test; body is the graceful
serve — the owned guard drops on return):

```rust
/// Serves `router` on `listener`, draining in-flight requests when `shutdown`
/// resolves, then returns. Owns `runtime_guard`, so a normal return drops it and
/// removes the runtime file — the covered removal path.
async fn serve_with_shutdown(
    listener: tokio::net::TcpListener,
    router: axum::Router,
    runtime_guard: crate::runtime_file::RuntimeFileGuard,
    shutdown: impl std::future::Future<Output = ()> + Send + 'static,
) -> anyhow::Result<()> {
    axum::serve(listener, router)
        .with_graceful_shutdown(shutdown)
        .await?;
    Ok(())
    // `runtime_guard` drops here → removes runtime.json on the graceful path.
}
```

Add the supervisor. Streams are created **synchronously** (covered by the
tests); only the spawned wait-loop is `cov:ignore`'d (async signal region + the
`process::exit` the forced path ends in):

```rust
/// Installs SIGINT/SIGTERM handlers and returns a receiver that fires when the
/// first arrives (the graceful-shutdown trigger). A second signal forces an
/// immediate exit, best-effort removing the runtime file first (because
/// `process::exit` skips `Drop`). `runtime_path` is cloned from the guard before
/// it is moved into `serve_with_shutdown`.
///
/// # Errors
/// Returns an error if a signal handler cannot be installed.
#[cfg(unix)]
fn spawn_shutdown_supervisor(
    runtime_path: Option<std::path::PathBuf>,
) -> std::io::Result<tokio::sync::oneshot::Receiver<()>> {
    use tokio::signal::unix::{signal, SignalKind};
    // Installed synchronously so a caller (or test) can rely on the handlers
    // being active the moment this returns.
    let mut sigint = signal(SignalKind::interrupt())?;
    let mut sigterm = signal(SignalKind::terminate())?;
    let (tx, rx) = tokio::sync::oneshot::channel();
    tokio::spawn(async move {
        // cov:ignore-start -- async signal wait-loop; the forced branch ends in
        // process::exit and is unreachable by a survivable test. The synchronous
        // setup above and serve_with_shutdown are host-covered by the signal tests.
        tokio::select! { _ = sigint.recv() => {}, _ = sigterm.recv() => {} }
        tracing::info!("received shutdown signal; draining in-flight requests");
        let _ = tx.send(());
        tokio::select! { _ = sigint.recv() => {}, _ = sigterm.recv() => {} }
        tracing::warn!("second shutdown signal; forcing immediate exit");
        if let Some(p) = &runtime_path {
            crate::runtime_file::remove_runtime_file(p);
        }
        std::process::exit(0);
        // cov:ignore-stop
    });
    Ok(rx)
}
```

Rewire `cmd_serve` (`:484-491`). Keep the scheduler bindings; wire the
supervisor to the trigger and hand the guard to `serve_with_shutdown`. This
live-serve glue is not reached by any host test (the sole `cmd_serve` test fails
early at `prepare_server`), so wrap it `cov:ignore` — matching the repo's
existing treatment of the serve loop (jaunder-uox1):

```rust
    tracing::info!(bind = %bind, prod, "starting HTTP server");
    // cov:ignore-start -- live serve glue: unreachable by host tests (the sole
    // cmd_serve test returns early at prepare_server). The covered pieces live in
    // serve_with_shutdown + spawn_shutdown_supervisor (see the signal tests).
    let _backup_scheduler = backup_scheduler;
    let _feed_scheduler = feed_scheduler;
    #[cfg(unix)]
    {
        let runtime_path = runtime_guard.path().map(std::path::Path::to_path_buf);
        let shutdown_rx = spawn_shutdown_supervisor(runtime_path)?;
        serve_with_shutdown(listener, router, runtime_guard, async move {
            let _ = shutdown_rx.await;
        })
        .await
    }
    #[cfg(not(unix))]
    {
        serve_with_shutdown(listener, router, runtime_guard, std::future::pending::<()>())
            .await
    }
    // cov:ignore-stop
}
```

Notes for the implementer:

- The `PreparedServer` destructure keeps `listener`, `router`, `runtime_guard`;
  remove the old `let _runtime_guard = ...;` and the old
  `axum::serve(...).await?; Ok(())` lines (`:489-491`).
- No `unwrap`/`expect` in production: `spawn_shutdown_supervisor(..)?`
  propagates (`io::Error` → `anyhow` via `?`); the trigger send is
  `let _ = tx.send(())`.
- The `oneshot` channel lives **inside** `spawn_shutdown_supervisor`
  (`#[cfg(unix)]`), so the `not(unix)` build has no unused channel and no
  warning.

- [x] **Step 5: Run the tests, verify they pass**

Run:
`cargo nextest run -p jaunder --lib -E 'test(drains_and_removes_runtime_file)'`
Expected: PASS — both `sigterm_*` and `sigint_*` remove `runtime.json` after a
real raised signal. Run: `cargo nextest run -p jaunder` Expected: PASS (incl.
`prepare_server_*` and `cmd_serve_fails_when_not_initialized` in
`server/tests/misc/commands.rs`, spec AC8).

- [x] **Step 6: Verify the gate + dependency policy** (AC7, AC9)

Run: `cargo xtask check` Expected: green — clippy clean (no `unwrap`/`expect` in
the new production code); the coverage gate passes with only the supervisor
async-loop and the `cmd_serve` glue `cov:ignore`'d (the signal tests cover
`serve_with_shutdown` + the synchronous supervisor setup); `cargo deny` accepts
`nix`. Confirm `git diff server/Cargo.toml` shows the `nix` line under
`[dev-dependencies]` only — nothing in `[dependencies]`.

- [x] **Step 7: Commit**

```bash
git add server/Cargo.toml Cargo.lock server/src/commands.rs
git commit -m "feat(serve): graceful shutdown removes runtime.json on SIGINT/SIGTERM"
```

---

### Task 3: Docs — amend ADR-0035 lifecycle note

**Files:**

- Modify: `docs/adr/0035-elisp-live-integration-harness.md` (lifecycle note
  ~`:60-72`; deferred-follow-ons list)

**Interfaces:** none (docs only).

- [x] **Step 1: Update the lifecycle note + follow-on status**

Edit the runtime.json **lifecycle** description so removal is stated as
signal-robust on `SIGINT`/`SIGTERM` (delivered by #140), and mark the first
deferred follow-on ("signal-robust removal") as **done (#140)**. Keep the
canonical `# ADR-0035: <title>` heading and the single `- Status: accepted` line
**unchanged** (the `adr-format` / `adr-readme-parity` gates fail on drift). Do
not renumber or retitle. No new ADR (spec decision 8).

- [x] **Step 2: Verify the ADR + prose gates**

Run `prettier -w docs/adr/0035-elisp-live-integration-harness.md` before staging
(avoids the pre-commit fail-restage double-commit), then: Run:
`cargo xtask check --no-test` Expected: green — `adr-format`,
`adr-readme-parity`, and `prettier` all pass.

- [x] **Step 3: Commit**

```bash
git add docs/adr/0035-elisp-live-integration-harness.md
git commit -m "docs(adr): ADR-0035 runtime.json removal is signal-robust (#140)"
```

---

## Final verification (after all tasks)

- [ ] `cargo xtask validate --no-e2e` — static + clippy + the coverage gate over
      the whole change. Green here is the "done" bar for #140 (spec AC9). _(The
      signal tests run in the host nextest/coverage pass, so no e2e/VM check is
      needed for #140.)_
