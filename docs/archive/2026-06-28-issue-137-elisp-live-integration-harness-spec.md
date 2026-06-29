# Live integration test harness for the Emacs client — Design (issue #137)

* Status: proposed
* Deciders: mdorman, Claude
* Milestone: **Emacs blogging front-end** (#4). Foundation: **blocks #74** (Unit C,
  publish) and **#75** (Unit D, reconcile/pull).
* Related: ADR-0035 (this cycle), ADR-0031 (elisp separately-tested subproject),
  ADR-0023 (AtomPub wire extensions), ADR-0014 (AtomPub authentication).

## Goal

A reusable ERT harness that exercises the Emacs (elisp) client against a **real,
running Jaunder server** over HTTP — so the network, auth, and round-trip behavior
that Units C/D depend on is tested **live**, not only with stubs. This lands first;
#74 and #75 build their publish/reconcile tests on top of it.

The principle: don't defer testing of the genuinely complex part (HTTP I/O, auth,
resume ordering, server round-trips). The harness must exist and be proven working
before any client logic is written against it.

## Current state (what exists, what's missing)

Verified against the tree:

* **Server boot** — the `jaunder` binary (crate `server/`, `server/src/cli.rs`)
  takes `JAUNDER_DB` (default `sqlite:./data/jaunder.db`), `JAUNDER_STORAGE_PATH`,
  and `serve --bind <addr> --environment dev|prod`. In **dev** mode `serve`
  auto-initializes the schema if absent, so an ephemeral sqlite server is just
  `JAUNDER_DB=sqlite:$TMP/db serve --bind 127.0.0.1:0 --environment dev`. **Gap:**
  `--bind …:0` chooses a free port but the binary does not report which one.
* **User provisioning** — `jaunder user-create --username U --password P` exists
  (`server/src/commands.rs`, bypasses registration policy). **Gap:** there is **no**
  out-of-process way to mint an **app password** — the only minter is
  `SessionStorage::create_session(user_id, label)` (`storage/src/sessions.rs`),
  reached in-process by Rust tests or via the authenticated web `create_app_password`
  server-fn. The DB stores only the SHA-256 token *hash*, so a raw-SQL seed would
  have to re-implement `hash_token`.
* **Auth shape** — an app password is a labelled session token (32 random bytes,
  url-safe-base64, 43 chars). The client sends it as HTTP Basic
  `base64("<username>:<token>")`; verification is in `web/src/auth/server.rs`
  (`resolve_credential`; rejects a Basic-username / session-user mismatch).
* **Elisp gate** — `flake.nix` has pure `ert-check` / `elisp-fmt-check` Nix checks
  (`emacsForCi`, no server in the sandbox) plus mirrored host-side `ert` / `elisp-fmt`
  xtask steps (`xtask/src/steps/static_checks.rs`). **A pure Nix `runCommand` check
  cannot host a server.** The established "needs a running server" model is the
  `nixosTest` VM (the e2e checks, `mkE2eSqliteCheck`).
* **Boot-and-talk patterns** — only Playwright e2e talks to a live server (port 3000,
  readiness via `curl`/`wait_for_open_port`). Rust integration tests use in-process
  `tower::oneshot`, not HTTP. No reusable "spawn server on a free port" helper exists.

## Design

The harness **owns the server lifecycle**: it boots `jaunder serve` itself,
provisions credentials via the CLI, runs the test body, and tears everything down.
This single abstraction runs in two contexts — host-side for fast development
iteration, and inside a `nixosTest` VM for the hermetic gate (the VM only has to
provide Emacs + the `jaunder` binary; no systemd service needed).

### 1. Server changes (Rust)

* **`jaunder app-password-create --username U --label L`** — new CLI subcommand
  (`cli.rs` + `commands.rs`) that resolves the user, calls `create_session`, and
  prints the **raw token** to stdout (single line, nothing else). Has its own unit
  test. Reusable beyond tests: an operator can mint an app password without the web
  UI, and it is partial groundwork for #76 (client self-provisioning).
* **`serve` always writes a runtime-info file.** Once the listener binds, the server
  writes a small JSON file **atomically** (temp + rename) and removes it on clean
  shutdown. It lives in the **data directory** — default
  `<JAUNDER_STORAGE_PATH>/runtime.json` (the storage dir already holds the
  per-instance sqlite DB and media, so this is the natural per-instance anchor);
  `--runtime-file <path>` (env `JAUNDER_RUNTIME_FILE`) overrides the location.
  Contents, kept minimal for now: `{ "ip": "<ip>", "port": <port> }` (the bound
  address). This makes a `--bind …:0` subprocess discoverable race-free and doubles
  as a binding handshake. The write is **best-effort**: a failure is logged but does
  not stop serving.
  **Multi-instance.** Because the path lives in the per-instance data dir (or is set
  explicitly) and nothing keys on a global path, servers with distinct data dirs never
  collide. Two servers on the *same* data dir is already unsupported (sqlite
  single-writer). The harness passes an explicit `--runtime-file` inside its unique
  tempdir, so parallel test servers are isolated.
  **Forward-compatible, by design.** Being JSON, the file accommodates two
  deliberately **deferred** features (filed as follow-on issues — see Out of scope),
  each a clean additive field: (a) a start-up **mutex** — refuse to start when an
  existing `runtime.json` records a *live* `pid` (adds a `pid` field; PID-liveness
  check with stale-after-crash detection so a dead pid is overwritten with a warning
  rather than blocking restart); and (b) a local **admin token** for a
  `jaunder shut-down` control channel (adds an `admin_token` field). Neither is built
  in #137.

Both changes are small, additive, and independently useful; neither alters existing
defaults (see "don't reinforce defaults").

### 2. Elisp harness (`elisp/test/`)

* A macro `jaunder-test--with-live-server (vars &rest body)` that, in an
  `unwind-protect`:
  1. creates a tempdir (`JAUNDER_DB=sqlite:<tmp>/jaunder.db`,
     `JAUNDER_STORAGE_PATH=<tmp>/data`) and a `runtime.json` path under it;
  2. spawns `jaunder serve --bind 127.0.0.1:0 --runtime-file <rf> --environment dev`
     via `make-process`, capturing stderr to a buffer for diagnostics;
  3. waits for `<rf>` to appear and parse (bounded poll), reads `ip`/`port`, then
     polls `GET http://ip:port/` until ready (bounded, like `run-e2e.sh`);
  4. runs `jaunder user-create` then `jaunder app-password-create` to provision a
     known user + token;
  5. binds the base URL, username, and token (and writes a temporary `auth-source`
     entry so the real `jaunder--auth-secret` path is exercised) for `body`;
  6. on exit: `delete-process` the server, remove the tempdir. On failure it surfaces
     the captured server stderr so a broken boot is diagnosable.
* The `jaunder` binary is located via env var `JAUNDER_TEST_BINARY`, falling back to
  PATH; if neither resolves, the harness **errors loudly** (never silently skips).
* A dedicated runner `elisp/scripts/run-integration-tests.el` (parallel to the
  existing `run-tests.el`) loads `test/*-integration.el`, keeping pure ERT and
  server-backed ERT separable — the fast pure suite stays serverless.

### 3. Nix check (`flake.nix`)

* A new `nixosTest` check (e.g. `elisp-integration`) whose VM has `emacsForCi` and
  the host-built `jaunder` binary on PATH (and `JAUNDER_TEST_BINARY` set to it). Its
  testScript runs `emacs --batch -Q -l elisp/scripts/run-integration-tests.el`. It is
  **simpler than the e2e checks** — no `services.jaunder` systemd unit, no Playwright;
  the harness self-boots the server inside the VM.
* Wired into `cargo xtask validate` (the heavy / server tier, alongside e2e), **not**
  the fast `cargo xtask check --no-test` iterate loop. CI runs it as part of the
  validate surface.
* During development the same suite runs host-side directly
  (`JAUNDER_TEST_BINARY=target/debug/jaunder emacs --batch -l …/run-integration-tests.el`)
  for fast iteration — this is the "pipecleaner" loop.

### 4. Committed smoke test (`elisp/test/jaunder-smoke-integration.el`)

Minimal, but exercises the whole path end-to-end so a regression in boot / provision
/ auth is caught:

* **Unauthenticated** `GET /atompub/service` → 200; assert the `j:extension` element
  advertises features `format-media-type` and `slug` (ADR-0023 capability discovery).
* **Authenticated** `GET /atompub/{user}/posts` with Basic `base64(user:token)` →
  200, empty collection. This is the load-bearing assertion: it proves
  `user-create` + `app-password-create` + Basic-auth verification work together over
  the wire.

Dev-time throwaway "pipecleaner" tests used to bring the harness up are **not**
committed.

### 5. ADR

ADR-0035 records the architectural decision: live integration testing of the Emacs
client via a **self-booting harness** (the harness owns the server lifecycle) run
inside a `nixosTest` VM, plus the two enabling server affordances
(`app-password-create`, the `serve` runtime-info file) and why a pure Nix check is
unsuitable.

## Edge cases / tests

* **Binary missing** — `JAUNDER_TEST_BINARY` unset and not on PATH ⇒ the harness
  errors with a clear message; it never silently passes by skipping.
* **Boot failure** — if the server exits before binding, `runtime.json` never
  appears; the bounded wait times out and the captured server stderr is surfaced.
* **Readiness vs binding** — `runtime.json` proves *bound*; the `GET /` poll proves
  *serving*. Both are required before the body runs.
* **Teardown on test failure** — the server process and tempdir are cleaned up via
  `unwind-protect` even when the body signals.
* **Auth negative path** (smoke-adjacent, cheap) — a Basic request with a wrong
  token is rejected (guards against the harness accidentally testing an unauthed
  server).
* The new server surface gets Rust unit tests: `app-password-create` mints a token
  usable for auth; `serve` writes a `runtime.json` with parseable `ip`/`port` (and
  removes it on clean shutdown).

## Out of scope (this issue)

Each of these is a **separable concern** filed as its own follow-on issue (the plan's
first task files the three new ones below); none is needed for the harness itself:

* **Signal-robust removal** — give `serve` a graceful-shutdown hook
  (`axum::serve(...).with_graceful_shutdown` on SIGTERM/SIGINT) so the runtime file
  is reliably removed on a normal stop, not only when the serve loop returns on its
  own. In #137 removal is best-effort (the harness cleans its own tempdir, so it does
  not depend on this). *New follow-on.*
* **Start-up mutex** via `runtime.json` — refuse to start when an existing file
  records a live `pid`, with stale-after-crash (dead-pid) detection. Pairs with
  signal-robust removal: graceful-remove (write side) + stale detection (read side)
  together make the file a reliable "is an instance running" signal. *New follow-on.*
* **Local admin control channel** — a random `admin_token` in `runtime.json` plus a
  `jaunder shut-down` command (and any further admin operations). *New follow-on.*
* Any client publish/reconcile logic (`jaunder--org->atom`, the publish flow) — that
  is #74, built on this harness.
* Client self-provisioning of app passwords (#76) — `app-password-create` is the
  server-side affordance only.
* Instrumenting the elisp client in the coverage report (#82) and the e2e tests in
  coverage (#83) — separate concerns, unaffected by this harness.
