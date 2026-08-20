# Test-Support and CLI OTLP Telemetry Implementation Plan

> **For agentic workers:** Execute this plan task-by-task with `jaunder-iterate`
> (delegating individual tasks to a subagent via `jaunder-dispatch` when
> useful). Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Make every host process that writes the e2e database export its
existing storage spans into the e2e OTLP capture.

**Architecture:** Split shared OTLP process telemetry into `host::telemetry` and
leave server-only diagnostics in `server::observability`. `server` and
`test-support` both hold the same `TelemetryGuard` across their dispatch; the
production CLI path remains covered and the e2e VM asserts that the seed phase
produces both `jaunder` CLI and `test-support` storage spans.

**Tech Stack:** Rust (`host`, `server`, `test-support`, `tools/devtool`,
`xtask`), `tracing`, `tracing-subscriber`, `tracing-opentelemetry`,
OpenTelemetry OTLP/gRPC, NixOS e2e VM, Markdown docs/ADR projection.

## Review header

**Scope — in:** shared OTLP bootstrap extraction; server diagnostics split;
`test-support` dispatch guard; production CLI regression proof; e2e seed-span
verification; ADR-0011/ADR-0057/architecture/observability docs.

**Scope — out:** new Playwright → `test-support` traceparent protocol; storage
span renames; seed data changes; historical #766 re-analysis; new metrics.

**Tasks:**

1. Extract `host::telemetry` and keep server-scoped diagnostics server-owned.
2. Wire `test-support` and preserve production CLI telemetry behavior.
3. Gate e2e seed trace completeness and update observability docs.
4. Run final validation, review, PR, and merge gate.

**Key risks / decisions:** Do not move the diag layer or panic hook into the
shared guard: `test-support` and one-shot CLI commands must not write `diag.log`
when `JAUNDER_CAPTURE_DIR` is present. The e2e proof should run after
`seed_db()` and before Playwright; it must inspect the collector's JSONL after
forcing the collector to flush/stop or via a deterministic helper, not assume
the batch processor has already written.

## Global Constraints

- Implement the approved spec:
  `docs/superpowers/specs/2026-08-20-issue-769-test-support-cli-otel.md`.
- Preserve ADR-0011: no endpoint is no-op; exporter/setup/shutdown failures are
  fallback diagnostics only and never change command exit status.
- Preserve ADR-0057: only the server writes scoped diagnostics (`diag.log`);
  `test-support` may reset/query capture paths but must not become a diag
  writer.
- `host` may take external infrastructure dependencies but no workspace
  dependency above `common`/`macros` (ADR-0058).
- Use `devtool run -- <cmd>` for all cargo/git commands. Tick each task checkbox
  before its commit gate. No `Co-Authored-By` trailers.

---

## File structure

- Create: `host/src/telemetry.rs` — shared OTLP process telemetry bootstrap,
  `TelemetryGuard`, env parsing, fallback reporting, fmt/slow-span layers, and
  provider shutdown.
- Modify: `host/src/lib.rs` — export `telemetry` and update module docs.
- Modify: `host/Cargo.toml` — add only external infra dependencies moved from
  `server` (`tracing-log`, `tracing-subscriber`, `tracing-opentelemetry`,
  `opentelemetry_sdk`, `opentelemetry-otlp`, `log` only if needed by moved
  code).
- Modify: `server/src/observability.rs` — keep `with_http_observability`,
  request traceparent extraction, diag layer, panic hook, and server-specific
  wrapper around `host::telemetry`.
- Modify: `server/src/main.rs` — call server diagnostics bootstrap only for
  `serve`; call shared OTLP bootstrap for other runnable CLI commands.
- Modify: `server/src/commands.rs` if needed — expose a pure
  `Commands::is_serve` helper or equivalent dispatch classification without
  changing command output.
- Modify: `server/Cargo.toml` — remove dependencies no longer used directly by
  `server`; keep Axum/tower/tracing deps used by request/diag code.
- Modify: `test-support/src/main.rs` — bind
  `host::telemetry::init_tracing(false)` across `run`.
- Modify: `test-support/Cargo.toml` — add direct `tracing`/telemetry deps only
  if the binary needs them directly after using `host::telemetry` (prefer none).
- Modify: `tools/devtool/src/seed_e2e.rs` — make seed invocations carry a stable
  phase/process marker in the environment if needed for deterministic trace
  verification; do not change the seed data.
- Modify: `flake.nix` — after e2e seeding, verify `otel-traces.jsonl` contains
  seed storage spans from both `jaunder` and `test-support`.
- Modify: `xtask/src/traces/parse.rs` or add a narrow helper under
  `xtask/src/traces/` only if the seed verification should reuse Rust OTLP JSONL
  parsing instead of VM shell text matching.
- Modify: `docs/adr/0011-unified-observability.md`, `docs/ARCHITECTURE.md`,
  `docs/observability.md` — project the new telemetry home and diagnostic split.

## Interfaces and contracts

```rust
// host/src/telemetry.rs
pub struct TelemetryGuard { /* private fields */ }

#[must_use]
pub fn init_tracing(verbose: bool) -> TelemetryGuard;

// For server only: installs the same OTLP process telemetry plus one caller-owned
// diagnostic layer. Keep this interface narrow; if the exact Layer bound is too
// noisy, hide it behind a server-local wrapper and keep only `init_tracing` public.
pub fn init_tracing_with_server_layer(
    verbose: bool,
    layer: Option<BoxedTelemetryLayer>,
) -> TelemetryGuard;

pub type BoxedTelemetryLayer =
    Box<dyn tracing_subscriber::Layer<tracing_subscriber::Registry> + Send + Sync + 'static>;
```

```rust
// server/src/commands.rs or server/src/main.rs
impl Commands {
    pub(crate) fn is_serve(&self) -> bool;
}

// server/src/observability.rs
#[must_use]
pub fn init_server_tracing(verbose: bool) -> host::telemetry::TelemetryGuard;
```

The exact generic shape for `init_tracing_with_server_layer` may be tightened
while implementing; the externally important interface is that ordinary host
processes call `host::telemetry::init_tracing(false)` and only the server
`serve` path can attach the diag layer/panic hook.

### Task 1: Extract shared OTLP process telemetry

**Files:**

- Create: `host/src/telemetry.rs`
- Modify: `host/src/lib.rs`
- Modify: `host/Cargo.toml`
- Modify: `server/src/observability.rs`
- Modify: `server/Cargo.toml`
- Test: `host/src/telemetry.rs`
- Test: `server/src/observability.rs`

**Interfaces:**

- Produces: `host::telemetry::init_tracing(verbose: bool) -> TelemetryGuard`.
- Produces: server-only wrapper `server::observability::init_server_tracing`.
- Preserves: `server::observability::with_http_observability` unchanged for
  router callers.

- [x] **Step 1: Move pure telemetry tests first.** Copy/adapt the existing tests
      for endpoint precedence, blank/invalid endpoint fallback, log-format
      parsing, slow-threshold parsing, exporter setup fallback, valid endpoint
      startup, subscriber-install nonfatal behavior, and `TelemetryGuard`
      shutdown from `server/src/observability.rs` into `host/src/telemetry.rs`.
      Add one new host test:

      ```rust
      #[test]
      fn process_telemetry_does_not_create_diag_file_when_capture_dir_is_set() {
          common::test_support::with_env(|env| {
              let dir = tempfile::TempDir::new().expect("tempdir");
              env.set(host::capture::DIR_ENV, dir.path());
              env.remove("JAUNDER_OTEL_EXPORTER_OTLP_ENDPOINT");
              env.remove("OTEL_EXPORTER_OTLP_ENDPOINT");
              let _guard = host::telemetry::init_tracing(false);
              assert!(!dir.path().join("diag.log").exists());
          });
      }
      ```

      Run: `devtool run -- cargo nextest run -p host telemetry`.
      Expected: FAIL before the module exists or before diag is split.

- [x] **Step 2: Implement `host::telemetry`.** Move only OTLP process telemetry
      from `server::observability`: `TelemetryGuard`, endpoint/filter/log-format
      parsing, `LogTracer`, trace propagator setup, fmt layer, slow span layer,
      OTLP tracer/meter provider setup, fallback diagnostics, and shutdown. Do
      not move `diag_log_file`, `diag_layer`, `install_diag_panic_hook`, or HTTP
      request middleware. Add host dependencies allowed by ADR-0058; keep host
      free of `storage`, `server`, and `web` dependencies.

- [x] **Step 3: Keep server diagnostics in server.** Leave diag tests in
      `server/src/observability.rs` and route them through
      `init_server_tracing(false)`. Add/keep tests proving:

      ```rust
      #[test]
      fn init_server_tracing_creates_diag_file_when_capture_dir_is_set() { /* existing shape */ }

      #[test]
      fn init_server_tracing_survives_unopenable_diag_path() { /* existing shape */ }
      ```

      Run: `devtool run -- cargo nextest run -p jaunder observability::tests`.
      Expected: PASS.

- [x] **Step 4: Run focused host/server tests.**

      Run: `devtool run -- cargo nextest run -p host telemetry`.
      Expected: PASS.

      Run: `devtool run -- cargo nextest run -p jaunder observability::tests`.
      Expected: PASS.

- [x] **Step 5: Tick this checkbox, run the commit gate, and commit.**

      Run: `devtool run -- cargo xtask check`.
      Expected: PASS.

      Commit exactly:

      ```bash
      devtool run -- git add Cargo.lock host/src/telemetry.rs host/src/lib.rs host/Cargo.toml server/src/observability.rs server/Cargo.toml docs/superpowers/specs/2026-08-20-issue-769-test-support-cli-otel.md docs/superpowers/plans/2026-08-20-issue-769-test-support-cli-otel.md
      devtool run -- git commit -m "refactor(obs): split process telemetry into host (#769)"
      ```

### Task 2: Wire `test-support` and preserve CLI dispatch behavior

**Files:**

- Modify: `test-support/src/main.rs`
- Modify: `test-support/tests/cli.rs`
- Modify: `server/src/main.rs`
- Modify: `server/src/cli.rs` if `Commands::is_serve` is added
- Test: `test-support/src/main.rs`
- Test: `test-support/tests/cli.rs`
- Test: `server/src/main.rs`

**Interfaces:**

- Consumes: `host::telemetry::init_tracing(false)` from Task 1.
- Produces: `test-support` dispatch guard held across every subcommand.
- Produces: production CLI dispatch that uses `init_server_tracing` only for
  `serve` and `host::telemetry::init_tracing(cli.verbose)` for all other
  commands.

- [x] **Step 1: Add failing dispatch tests.** Add tests proving the two routing
      rules without depending on a live collector:

      ```rust
      // test-support/tests/cli.rs
      #[test]
      fn capture_path_initializes_telemetry_without_writing_diag_log() {
          let capture = tempfile::TempDir::new().expect("capture dir");
          let out = Command::new(env!("CARGO_BIN_EXE_test-support"))
              .args(["capture-path", "mail"])
              .env("JAUNDER_CAPTURE_DIR", capture.path())
              .env("JAUNDER_OTEL_EXPORTER_OTLP_ENDPOINT", "not a valid endpoint")
              .env_remove("OTEL_EXPORTER_OTLP_ENDPOINT")
              .output()
              .expect("spawn test-support binary");
          assert!(out.status.success(), "status: {:?}", out.status);
          assert!(!capture.path().join("diag.log").exists());
          let stderr = String::from_utf8(out.stderr).expect("stderr utf8");
          assert!(
              stderr.contains("tracing export disabled")
                  || stderr.contains("invalid configured value; export disabled"),
              "telemetry init fallback proves the guard ran; stderr: {stderr}"
          );
      }
      ```

      Add server dispatch coverage for both classification and the actual
      non-serve routing:

      ```rust
      #[test]
      fn serve_is_the_only_server_diagnostics_command() {
          assert!(Commands::Serve { /* minimal args */ }.is_serve());
          assert!(!Commands::SiteConfig { /* set action */ }.is_serve());
      }

      #[test]
      fn run_site_config_uses_process_telemetry_without_diag_log() {
          common::test_support::with_env(|env| {
              let capture = TempDir::new().expect("capture dir");
              let base = TempDir::new().expect("db dir");
              let storage = test_storage_args(&base);
              env.set(host::capture::DIR_ENV, capture.path());
              env.set("JAUNDER_OTEL_EXPORTER_OTLP_ENDPOINT", "not a valid endpoint");
              tokio::runtime::Runtime::new()
                  .expect("runtime")
                  .block_on(async {
                      run(Cli {
                          command: Some(Commands::Init {
                              storage: storage.clone(),
                              skip_if_exists: false,
                          }),
                          verbose: false,
                      })
                      .await
                      .expect("init db");
                      run(Cli {
                          command: Some(Commands::SiteConfig {
                              action: SiteConfigAction::Set {
                                  storage,
                                  key: SiteConfigKey::SiteRegistrationPolicy,
                                  value: "open".to_string(),
                              },
                          }),
                          verbose: false,
                      })
                      .await
                      .expect("site-config set");
                  });
              assert!(!capture.path().join("diag.log").exists());
          });
      }
      ```

      Use the repo's existing environment-lock helper; keep the env guard alive
      across the async dispatch (for example by using a local Tokio runtime inside
      the `with_env` closure).

      Run: `devtool run -- cargo nextest run -p test-support capture_path_initializes_telemetry_without_writing_diag_log`.
      Expected: FAIL until telemetry is wired, because the current binary emits no telemetry fallback diagnostic.

      Run: `devtool run -- cargo nextest run -p jaunder -E 'test(serve_is_the_only_server_diagnostics_command) | test(run_site_config_uses_process_telemetry_without_diag_log)'`.
      Expected: FAIL until helper/routing exists; the non-serve routing test fails on current code because `run` installs server diagnostics for `site-config`.

- [x] **Step 2: Wire dispatch.** In `test-support/src/main.rs::run`, bind
      `let _telemetry = host::telemetry::init_tracing(false);` after clap has
      produced a runnable command and before the match. In `server/src/main.rs`,
      select `server::observability::init_server_tracing(cli.verbose)` for
      `serve` and `host::telemetry::init_tracing(cli.verbose)` for non-serve
      commands; keep the guard scoped across `command.execute().await`.

- [x] **Step 3: Run focused command tests.**

      Run: `devtool run -- cargo nextest run -p test-support`.
      Expected: PASS.

      Run: `devtool run -- cargo nextest run -p jaunder -E 'test(site_config_set_parses_positional_key_value) | test(serve_is_the_only_server_diagnostics_command) | test(run_site_config_uses_process_telemetry_without_diag_log)'`.
      Expected: PASS.

- [x] **Step 4: Tick this checkbox, run the commit gate, and commit.**

      Run: `devtool run -- cargo xtask check`.
      Expected: PASS.

      Commit exactly:

      ```bash
      devtool run -- git add test-support/src/main.rs test-support/tests/cli.rs server/src/main.rs server/src/cli.rs docs/superpowers/plans/2026-08-20-issue-769-test-support-cli-otel.md
      devtool run -- git commit -m "feat(obs): trace test-support seed writes (#769)"
      ```

### Task 3: Prove seed spans in the e2e capture and update docs

**Files:**

- Modify: `flake.nix`
- Modify: `tools/devtool/src/seed_e2e.rs` if stable process markers are needed
- Modify: `host/src/telemetry.rs` if stable process markers are needed
- Modify: `storage/src/site_config.rs` if the site-config seed path lacks a
  `storage.*` span
- Modify: `xtask/src/traces/parse.rs` or new narrow helper if Rust JSONL parsing
  is used from the VM/assertion path
- Modify: `docs/adr/0011-unified-observability.md`
- Modify: `docs/ARCHITECTURE.md`
- Modify: `docs/observability.md`
- Test: `tools/devtool/src/seed_e2e.rs` if modified

**Interfaces:**

- Consumes: both seed invocations in `flake.nix` explicitly exporting
  `JAUNDER_OTEL_EXPORTER_OTLP_ENDPOINT=http://127.0.0.1:4317` into the
  `devtool seed-e2e` process environment; the `jaunder.service` environment is
  not inherited by `machine.succeed(...)` seed children.
- Produces: deterministic e2e VM assertion that the seed phase's trace file
  includes:
  - at least one `storage.*` span emitted by `test-support`, and
  - at least one `storage.*` span emitted by `jaunder site-config set`.

- [x] **Step 1: Add the failing trace-completeness assertion.** Extend both
      sqlite and postgres e2e `seed_db()` commands to prefix
      `JAUNDER_OTEL_EXPORTER_OTLP_ENDPOINT=http://127.0.0.1:4317` alongside
      `JAUNDER_CAPTURE_DIR=/var/lib/jaunder/capture`, then verify the collector
      output after the seed command. Use a deterministic flush point:
      stop/restart the collector around the assertion, or add a narrow helper
      that waits until `otel-traces.jsonl` contains the expected spans. The
      assertion must fail closed if the file is missing, empty, malformed, or
      lacks either process's storage span. Prefer a Rust parser/helper over
      brittle text matching if the JSONL structure is more than a simple
      span-name scan.

      Run: `devtool run -- cargo xtask e2e sqlite chromium`.
      Expected: FAIL before Task 2's binary wiring reaches the VM, naming the
      missing `test-support` or `jaunder` seed span.

- [x] **Step 2: Add stable process attribution if the captured spans cannot
      distinguish the two binaries.** If span resource/service metadata already
      distinguishes `test-support` from `jaunder`, use it. Otherwise, set a
      bounded seed-process environment marker in `tools/devtool/src/seed_e2e.rs`
      for each child process and record it as a process-level telemetry
      attribute in `host::telemetry`. The marker values must be a closed set
      such as `"e2e.seed.jaunder" | "e2e.seed.test-support"`; never pass
      arbitrary user input into telemetry.

      If `seed_e2e.rs` changes, add/adjust unit tests for the env passed to each
      invocation.

      Run: `devtool run -- cargo nextest run --manifest-path tools/Cargo.toml seed_e2e`.
      Expected: PASS if the tool changed; skip only if no tool change was needed.

- [x] **Step 3: Update observability docs.** Add ADR-0011 addendum text stating
      that OTLP process telemetry now lives in `host::telemetry` and is held by
      `server`, production CLI commands, and `test-support`. State explicitly
      that scoped diagnostics remain server-owned per ADR-0057. Update
      `docs/ARCHITECTURE.md` Observability and Workspace rows, and
      `docs/observability.md` Backend/e2e capture prose to say seed processes
      now contribute storage spans to `otel-traces.jsonl`.

- [x] **Step 4: Run focused docs/tool checks and the e2e proof.**

      Run: `devtool run -- cargo xtask check --no-test`.
      Expected: PASS.

      Run: `devtool run -- cargo xtask e2e sqlite chromium`.
      Expected: PASS, including the new seed-span assertion.

- [x] **Step 5: Tick this checkbox, run the commit gate, and commit.**

      Run: `devtool run -- cargo xtask check`.
      Expected: PASS.

      Commit exactly:

      ```bash
      devtool run -- git add flake.nix tools/devtool/src/seed_e2e.rs host/src/telemetry.rs storage/src/site_config.rs docs/adr/0011-unified-observability.md docs/ARCHITECTURE.md docs/observability.md docs/superpowers/plans/2026-08-20-issue-769-test-support-cli-otel.md
      devtool run -- git commit -m "test(e2e): require seed storage spans (#769)"
      ```

### Task 4: Final validation and ship review

**Files:**

- Modify: `docs/superpowers/plans/2026-08-20-issue-769-test-support-cli-otel.md`
  (tick all completed boxes before each commit gate)
- No source edits unless review/gates require fixes.

**Interfaces:**

- Consumes: completed Tasks 1–3.
- Produces: final branch ready for `jaunder-ship` review and PR.

- [x] **Step 1: Run full local validation.**

      Run: `devtool run -- cargo xtask validate`.
      Expected: PASS.

- [x] **Step 2: Confirm clean final state.**

      Run: `devtool run -- git status --short`.
      Expected: no output.

      Run: `devtool run -- git log --oneline origin/main...HEAD`.
      Expected: focused commits for the telemetry split, test-support wiring, and
      seed-span/docs verification.

- [x] **Step 3: Start `jaunder-ship`.** Use the final diff and validation output
      for the required standards/spec reviews, then push, open the PR, monitor
      checks, and stop at the merge approval gate.
