# ADR-0035: Live Integration Testing of the Emacs Client via a Self-Booting Harness

- Status: accepted
- Deciders: mdorman, Claude
- Date: 2026-06-28

## Context and Problem Statement

The Emacs blogging front-end (milestone #4, units C/#74 publish and D/#75
reconcile) talks to the server over AtomPub: HTTP transport, HTTP Basic
app-password auth, media upload, and a safe-to-resume publish ordering. These
are the parts most likely to be wrong, yet the existing elisp gate (ADR-0031)
only runs **pure** ERT unit tests under `emacs --batch` with no server in scope,
and the Rust integration tests reach the AtomPub handlers **in-process** via
`tower::oneshot` — they never exercise the wire. Stubbing
`jaunder--http-request` in client tests would leave the real transport, auth,
and round-trip untested. Deferring those tests is exactly backwards: the complex
part must be tested live, and the harness to do so must exist **before** the
client logic is written against it (hence this is a foundation issue, #137,
blocking #74 and #75).

Two concrete gaps block a live test today:

- **No out-of-process app-password minter.** A user is creatable via
  `jaunder user-create`, but an app password (a labelled session token) can only
  be minted in-process (`create_session`) or via the authenticated web
  `create_app_password` server-fn. The DB stores only the SHA-256 token hash, so
  a raw-SQL seed would have to re-implement the security-sensitive hashing.
- **No way to discover an ephemeral port.** `serve --bind 127.0.0.1:0` picks a
  free port but the binary does not report it.

And structurally: a pure Nix `runCommand`/`craneLib` check is sandboxed with no
server, so server-backed elisp tests cannot run as one. The repository's
established "needs a running server" mechanism is the `nixosTest` VM (the e2e
checks).

## Decision Drivers

- Test the complex client behavior (transport, auth, round-trips) against a
  **real** server, not stubs.
- Reuse the existing hermetic, cachix-cached Nix-check model (`nixosTest`) the
  e2e suite already uses, rather than a bespoke host-only mechanism.
- Keep the fast pure-ERT loop serverless; isolate the heavier server-backed
  suite.
- Add only small, additive, independently-useful server affordances; don't
  change existing defaults.
- One harness usable both for fast host-side development and for the hermetic
  gate.

## Decision Outcome

**Add a self-booting elisp integration harness — the harness owns the full
server lifecycle — and run it inside a `nixosTest` VM, enabled by two small
server affordances.**

1. **Server affordances (Rust, additive).**
   - `jaunder app-password-create --username U --label L` — resolves the user,
     calls `create_session`, prints the raw token to stdout. Unit-tested.
     Reusable for ops and partial groundwork for client self-provisioning (#76).
   - **`serve` always writes a runtime-info file** — a small JSON file in the
     per-instance data dir (default `<JAUNDER_STORAGE_PATH>/runtime.json`,
     `--runtime-file`/`JAUNDER_RUNTIME_FILE` override), written atomically once
     the listener binds and removed on shutdown. Removal is **signal-robust as
     of #140**: `serve` has a graceful-shutdown hook that, on SIGINT/SIGTERM,
     drains in-flight requests and lets the serve loop return so the RAII guard
     removes the file; a second signal forces an immediate exit that still
     removes the file explicitly first. Contents are
     `{ "ip": <ip>, "port": <port>, "pid": <pid>, "start_time": <jiffies> }` —
     the address makes a `--bind …:0` subprocess discoverable race-free (a
     binding handshake), and the `pid` + `start_time` (from `/proc/<pid>/stat`
     field 22) identify the exact writer process for the start-up mutex. The
     write is best-effort (a failure is logged, not fatal); removal is
     best-effort too (a hard SIGKILL still skips it — recovered by the start-up
     mutex's stale detection). It is the forward-compatible base for follow-ons:
     **signal-robust removal — delivered in #140**; a **start-up mutex —
     delivered in #141**: on `serve` startup, if the file names a live writer
     (its `pid` is alive **and** has the recorded `start_time`) the server
     refuses to start; a dead/mismatched holder is treated as stale (warn +
     overwrite), and an unusable `/proc` is a hard fail. pid + start-time is
     used rather than `comm`/`exe` so a recycled pid running another `jaunder`
     is not mistaken for the writer. Remaining: a local admin token for a
     `jaunder shut-down` channel (#142, still deferred).

2. **The harness owns the server lifecycle.** A `jaunder-test--with-live-server`
   macro (under `elisp/test/`) spawns
   `jaunder serve --bind 127.0.0.1:0 --runtime-file … --environment dev`
   (ephemeral sqlite, dev auto-init) via `make-process`, waits on the runtime
   file then polls `GET /` for readiness, provisions a user + app password via
   the CLI, binds base-URL/username/token (and a temporary `auth-source` entry)
   for the test body, and tears the server + tempdir down in an
   `unwind-protect`. The binary is found via `JAUNDER_TEST_BINARY` (PATH
   fallback); if unresolved it errors loudly rather than skipping.

3. **A `nixosTest` VM check, not a systemd service.** Because the harness
   self-boots the server, the VM only needs `emacsForCi` + the `jaunder` binary
   on PATH — no `services.jaunder` unit and no Playwright (simpler than the e2e
   checks). A dedicated `run-integration-tests.el` runner loads
   `test/*-integration.el`, keeping the pure suite serverless. The check is
   wired into `cargo xtask validate` (server tier, alongside e2e), not the fast
   `check --no-test`. The same suite runs host-side
   (`JAUNDER_TEST_BINARY=target/debug/jaunder …`) for fast development
   iteration.

4. **A committed smoke test** proves boot + provision + auth end-to-end: an
   unauthenticated `GET /atompub/service` asserting the `j:extension` capability
   (ADR-0023), and an **authenticated** `GET /atompub/{user}/posts` (Basic
   `base64(user:token)`) returning an empty collection.

Rejected alternatives: a raw-SQL session seed (duplicates security-sensitive
token hashing, fragile); driving web login + `create_app_password` over HTTP
(couples the harness to the session-cookie/server-fn flow); the harness
pre-picking a free port (a TOCTOU window the runtime-info file removes); a
single-purpose `--port-file` (the JSON runtime file costs the same but is a
forward-compatible home for operational data — the start-up mutex and
admin-control channel deferred to follow-ons); a host-side-only xtask step for
the gate (loses the hermetic, cached reproducibility the `nixosTest` model
provides — host execution is retained for dev iteration but is not the gate); a
`nixosTest` with a systemd `services.jaunder` unit (unnecessary once the harness
owns the lifecycle).

## Consequences

- Good: Units C/#74 and D/#75 write publish/reconcile tests against a real
  server from day one; transport, auth, and round-trips are covered live.
- Good: reuses the `nixosTest` + cachix model; "green" now includes a live
  AtomPub round-trip.
- Good: `app-password-create` and the `serve` runtime-info file are
  independently useful (ops, #76, the deferred mutex/admin follow-ons, scripted
  deploys) beyond testing.
- Good: one harness serves both the fast host-side loop and the hermetic gate.
- Bad: the integration check boots a VM, so it is heavier and slower than pure
  ERT; it lives in the `validate` tier, not the inner loop (mitigated by the
  host-side dev path).
- Bad: a new server surface (`app-password-create`) must be kept secure — it
  mints a real credential; it bypasses no auth a CLI operator doesn't already
  have (the CLI already creates users and runs the server).

## Amendment — 2026-07-23 (#628): one server per suite, per-test fallback

The per-test lifecycle in Decision Outcome §2 flaked in CI: each of the 14 tests
booted its own server, so each was an independent chance to hit the
`auth readiness` poll timeout on a contended VM (partial failures like 1/14).
Two changes reduce this without altering what the tests assert:

- **Readiness budget** (`jaunder-test--wait`) is now a wall-clock deadline
  (default 30s, `JAUNDER_TEST_READY_TIMEOUT`-tunable) instead of a fixed
  100×0.1s iteration count, so a slow per-attempt connect (near its `plz`
  connect-timeout, now 2s) can't starve the poll count.
- **One shared server per suite.** `run-integration-tests.el` boots a single
  server via `jaunder-test--server-up`, binds the harness globals for the whole
  batch, and tears it down after — so the three readiness gates run **once**,
  not 14×. `jaunder-test--with-live-server` now _reuses_ an already-bound server
  and only self-boots when none is bound, preserving standalone interactive runs
  (`M-x ert` on one test).

Isolation implication: tests now share one DB and one `alice` user, so a new
test must stay collision-tolerant (assert on its own returned
ids/slugs/statuses, as all current tests already do) or opt back into its own
server via the fallback macro. Accordingly the §4 smoke assertion is now
"returns the user's posts collection (HTTP 200)", no longer "empty".
