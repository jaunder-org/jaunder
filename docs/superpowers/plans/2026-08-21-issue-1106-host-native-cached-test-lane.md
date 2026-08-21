# Host-Native Cached Test Lane Implementation Plan

> **For agentic workers:** Execute this plan task-by-task with `jaunder-iterate`
> (delegating individual tasks to a subagent via `jaunder-dispatch` when
> useful). Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Add `cargo xtask test-local` as a host-native, sccache-backed product
workspace test lane that preserves isolated PostgreSQL support without routing
the inner loop through Nix coverage.

**Architecture:** Make the ephemeral PostgreSQL wrapper concurrent-safe first,
then share the host Cargo cache environment currently embedded in static checks.
The new xtask step runs local `devtool pg run` via the tools workspace so
PostgreSQL helper edits are live, wraps `cargo nextest run`, and reports through
the normal xtask result envelope.

**Tech Stack:** Rust (`xtask`, `tools/devtool`), Cargo nextest, PostgreSQL 16
CLI tools, `sccache`, Jaunder's existing `CommandResult` / `StepResult` gate
model.

## Review header

**Scope — in:** concurrent-safe ephemeral PostgreSQL endpoint selection; shared
host Cargo cache env; `cargo xtask test-local` parsing/execution; CONTRIBUTING
docs for choosing the lane.

**Scope — out:** replacing `check`/`validate`; changing Nix coverage, doctest,
wasm, or e2e gates; adding `devenv`; using a persistent local PostgreSQL
instance; adding coverage or doctests to `test-local`.

**Tasks:**

1. Make `devtool pg run` allocate per-run loopback PostgreSQL endpoints instead
   of one fixed port.
2. Factor the host sccache worktree environment for reuse outside static checks.
3. Add `cargo xtask test-local` with default workspace and trailing nextest arg
   contracts.
4. Document the new lane and run the focused contracts plus the full check.

**Key risks / decisions:** `test-local` deliberately forces
`CARGO_INCREMENTAL=0` so `sccache` is effective across worktrees. The command
must run the local tools-workspace `devtool`, not a possibly stale PATH binary.
PostgreSQL port selection has an unavoidable bind-after-drop window unless
`pg_ctl` can inherit a bound socket, so the wrapper must retry on cluster-start
failure after selecting a new port. Unit tests cover the retry decision without
starting PostgreSQL, and a smoke proof runs two wrappers concurrently.

## Global Constraints

- Implement the approved spec:
  [`2026-08-21-issue-1106-host-native-cached-test-lane.md`](../specs/2026-08-21-issue-1106-host-native-cached-test-lane.md).
- Preserve `cargo xtask check`, `cargo xtask validate --no-e2e`,
  `cargo xtask validate`, Nix coverage, doctests, wasm tests, and e2e semantics.
- `test-local` is an inner-loop command only; do not wire it into git hooks in
  this issue.
- Automated test load must use throwaway PostgreSQL, not a persistent local
  production database.
- Run commands through `devtool run -- ...` for checks, and commit only the
  checked staged tree with no `Co-Authored-By` trailer.

---

## File structure

- Modify `tools/devtool/src/pg.rs` — replace the fixed port with per-run
  endpoint selection and unit-test URL/settings construction.
- Create `xtask/src/compile_cache.rs` — own reusable host Cargo cache
  environment construction.
- Modify `xtask/src/steps/static_checks.rs` — consume the shared compile-cache
  helper without changing static-check behavior.
- Create `xtask/src/steps/test_local.rs` — build and run the new xtask step.
- Modify `xtask/src/lib.rs` — add CLI parsing, command naming, dispatch, and
  parsing tests.
- Modify `CONTRIBUTING.md` — document when to use `test-local` and replace the
  raw `devtool pg run` workaround as the blessed inner-loop path.

## Interfaces and contracts

```rust
// tools/devtool/src/pg.rs
fn app_url(host: &str, port: u16) -> String;
fn bootstrap_url(host: &str, port: u16) -> String;
fn server_settings(host: &str, port: u16, pgdata: &Path) -> Vec<String>;
fn choose_free_port(host: &str) -> anyhow::Result<u16>;
fn with_ephemeral_on_free_port<T>(body: impl FnOnce(&PgEnv) -> anyhow::Result<T>) -> anyhow::Result<T>;

// xtask/src/compile_cache.rs
pub fn cargo_compile_env() -> (Vec<(String, String)>, Option<String>);

// xtask/src/steps/test_local.rs
pub fn args_for(trailing: &[String]) -> Vec<String>;
pub fn run(sh: &xshell::Shell, result: &mut CommandResult, trailing: &[String]);
```

`args_for([])` produces a local-devtool command equivalent to:

```bash
cargo run --quiet --manifest-path tools/Cargo.toml -p devtool -- \
  pg run -- cargo nextest run --workspace
```

`args_for(["-p", "storage"])` produces the same prefix but ends with
`cargo nextest run -p storage`.

### Task 1: Make ephemeral PostgreSQL concurrent-safe

**Files:**

- Modify: `tools/devtool/src/pg.rs`
- Test: `tools/devtool/src/pg.rs`

**Interfaces:**

- Consumes: existing `pg::with_ephemeral` / `pg::run_command` shape.
- Produces: per-run port selection used by `with_ephemeral`; URL and server
  settings helpers that accept the selected port.

- [x] **Step 1: Write failing unit tests for endpoint selection and retry.**
      Replace the fixed-port regression test with tests that assert URL
      construction is a pure function of the chosen port, `server_settings`
      embeds that chosen port, and `choose_free_port(HOST)` returns a non-zero
      port that can be rebound after selection. Add an injected cluster-start
      helper test where the first selected port reports a bind/start failure and
      the second selected port succeeds, proving the wrapper retries with a new
      endpoint before surfacing failure. Keep these tests free of real
      PostgreSQL startup.

  Run:

  ```bash
  devtool run -- cargo nextest run --manifest-path tools/Cargo.toml pg
  ```

  Expected: FAIL until the fixed `PORT` dependency is removed from
  `with_ephemeral`.

- [x] **Step 2: Implement per-run loopback port selection with retry.** Remove
      the fixed `PORT` constant from the runtime path. Add
      `choose_free_port(host)` using `std::net::TcpListener::bind((host, 0))`,
      read `local_addr().port()`, and drop the listener before `pg_ctl` starts.
      Thread the selected port through `server_settings`, `bootstrap`, and
      `PgEnv`. Wrap cluster startup in a small retry loop that selects a fresh
      port and data directory when `pg_ctl -w start` fails in the port-binding
      window, then fails with the last error after the bounded retry budget.

- [x] **Step 3: Run focused passing tests.** Re-run:

  ```bash
  devtool run -- cargo nextest run --manifest-path tools/Cargo.toml pg
  ```

  Expected: PASS, including URL/settings tests proving no single fixed port is
  baked into the helper and retry tests proving a start failure can choose a new
  endpoint.

- [x] **Step 4: Smoke two concurrent wrappers.** Run two short wrapper commands
      concurrently from this checkout and confirm both succeed. Use a shell-free
      approach if implementing this as a devtool unit/integration test; if run
      manually during the task, start two separate command sessions and wait for
      both:

  ```bash
  devtool run -- cargo run --quiet --manifest-path tools/Cargo.toml -p devtool -- pg run -- true
  ```

  Expected: both concurrent invocations PASS, proving the wrapper can run two
  independent clusters through startup, bootstrap, wrapped-command execution,
  and teardown at the same time.

- [x] **Step 5: Commit the PostgreSQL endpoint change.** Tick this task, run:

  ```bash
  devtool run -- cargo xtask check --no-test
  ```

  Inspect and stage only this task's changes, then commit:

  ```bash
  git add tools/devtool/src/pg.rs docs/superpowers/plans/2026-08-21-issue-1106-host-native-cached-test-lane.md
  git commit -m "fix(devtool): allocate ephemeral postgres ports"
  ```

### Task 2: Share host Cargo cache environment construction

**Files:**

- Create: `xtask/src/compile_cache.rs`
- Modify: `xtask/src/lib.rs`
- Modify: `xtask/src/steps/static_checks.rs`
- Test: `xtask/src/compile_cache.rs`
- Test: `xtask/src/steps/static_checks.rs`

**Interfaces:**

- Consumes: existing static-check `compile_cache_env` behavior.
- Produces: `crate::compile_cache::cargo_compile_env()`, returning
  `RUSTC_WRAPPER=sccache`, `CARGO_INCREMENTAL=0`, optional `SCCACHE_BASEDIRS`,
  and optional warning detail.

- [x] **Step 1: Write failing shared-helper tests.** Move or duplicate the
      current static-check cache tests so they target `compile_cache`: parsing
      absolute existing worktree roots, ignoring missing/non-absolute roots,
      including the current checkout, and omitting profile debug overrides. Add
      a static-check regression test that compiling specs still opt into the
      shared helper while non-compiling specs do not.

  Run:

  ```bash
  devtool run -- cargo nextest run --manifest-path xtask/Cargo.toml compile_cache static_checks
  ```

  Expected: FAIL while the helper does not exist.

- [x] **Step 2: Factor the implementation.** Create `xtask/src/compile_cache.rs`
      with the existing `sccache_basedirs`, worktree parsing, and env assembly.
      Make `static_checks::run` call the shared helper and preserve its current
      warning-detail behavior exactly.

- [x] **Step 3: Run focused passing tests.** Re-run:

  ```bash
  devtool run -- cargo nextest run --manifest-path xtask/Cargo.toml compile_cache static_checks
  ```

  Expected: PASS, with no changed static-check command ordering.

- [x] **Step 4: Commit the shared cache helper.** Tick this task, run:

  ```bash
  devtool run -- cargo xtask check --no-test
  ```

  Inspect and stage only this task's changes, then commit:

  ```bash
  git add xtask/src/compile_cache.rs xtask/src/lib.rs xtask/src/steps/static_checks.rs docs/superpowers/plans/2026-08-21-issue-1106-host-native-cached-test-lane.md
  git commit -m "refactor(xtask): share host cargo cache env"
  ```

### Task 3: Add `cargo xtask test-local`

**Files:**

- Create: `xtask/src/steps/test_local.rs`
- Modify: `xtask/src/lib.rs`
- Test: `xtask/src/steps/test_local.rs`
- Test: `xtask/src/lib.rs`

**Interfaces:**

- Consumes: `crate::compile_cache::cargo_compile_env()`.
- Produces: `Command::TestLocal { nextest_args: Vec<String> }` and
  `steps::test_local::run`.

- [ ] **Step 1: Write failing CLI and argument-construction tests.** Add parser
      tests proving `cargo xtask test-local` parses with an empty trailing
      vector and `cargo xtask test-local -- -p storage post_creation` preserves
      the exact trailing arguments. Add `args_for` tests proving no-arg mode
      appends `--workspace`, focused mode does not, and the command invokes
      local `devtool` through:

  ```text
  cargo run --quiet --manifest-path tools/Cargo.toml -p devtool -- pg run -- cargo nextest run ...
  ```

  Run:

  ```bash
  devtool run -- cargo nextest run --manifest-path xtask/Cargo.toml test_local parses_test_local
  ```

  Expected: FAIL while the command and module do not exist.

- [ ] **Step 2: Implement command dispatch.** Add `TestLocal` to `Command`,
      `Cli::command_name`, and `run`. Use `xshell::Shell`, create
      `CommandResult::new("test-local")`, call `steps::test_local::run`, then
      `finalize`. In `test_local::run`, build the local-devtool command with
      `args_for`, run it as one `StepResult` named `test-local` through
      `step_with_env`, and pass the shared cache env. If cache discovery returns
      a warning, append it to the step detail as static checks do.

- [ ] **Step 3: Run focused passing tests.** Re-run:

  ```bash
  devtool run -- cargo nextest run --manifest-path xtask/Cargo.toml test_local parses_test_local
  ```

  Expected: PASS, including default/passthrough command shape and command name.

- [ ] **Step 4: Smoke the command without running the whole workspace.** Run a
      narrow host-native command that exercises CLI dispatch, local devtool, the
      ephemeral PostgreSQL wrapper, and the cache env:

  ```bash
  devtool run -- cargo xtask test-local -- -p storage site_config_primitives_round_trip
  ```

  Expected: PASS. This storage test is backend-parametric, so the run proves the
  `test-local` command reaches both the SQLite and PostgreSQL harness cases. If
  this is still too broad in practice, use the narrowest backend-parametric
  storage test filter that compiles and runs quickly while still going through
  `test-local`.

- [ ] **Step 5: Commit the xtask command.** Tick this task, run:

  ```bash
  devtool run -- cargo xtask check --no-test
  ```

  Inspect and stage only this task's changes, then commit:

  ```bash
  git add xtask/src/lib.rs xtask/src/steps/test_local.rs docs/superpowers/plans/2026-08-21-issue-1106-host-native-cached-test-lane.md
  git commit -m "feat(xtask): add host-native test lane"
  ```

### Task 4: Document the lane and run the full check

**Files:**

- Modify: `CONTRIBUTING.md`
- Modify:
  `docs/superpowers/plans/2026-08-21-issue-1106-host-native-cached-test-lane.md`

**Interfaces:**

- Consumes: `cargo xtask test-local` from Task 3.
- Produces: contributor guidance that names when to use `test-local`,
  `check --no-test`, `check`, `validate --no-e2e`, and `validate`.

- [ ] **Step 1: Update documentation.** In the testing / verify-ladder guidance,
      add `cargo xtask test-local` as the repeated host-native product-test
      lane. In the PostgreSQL-backed Rust tests section, replace the raw
      `cargo run --manifest-path tools/Cargo.toml -p devtool -- pg run -- cargo     nextest ...`
      recommendation with `cargo xtask test-local` and focused examples using
      trailing nextest args. State that the command intentionally disables Cargo
      incremental so `sccache` can share Rust compiler work across worktrees.

- [ ] **Step 2: Run doc and parser checks.** Run:

  ```bash
  devtool run -- cargo nextest run --manifest-path xtask/Cargo.toml test_local compile_cache static_checks
  ```

  Expected: PASS.

- [ ] **Step 3: Run the full project check.** Run:

  ```bash
  devtool run -- cargo xtask check
  ```

  Expected: PASS. If formatters modify docs, inspect and stage those exact
  mechanical changes before committing.

- [ ] **Step 4: Commit the documentation and final checked state.** Tick this
      task, stage the checked tree, then commit:

  ```bash
  git add CONTRIBUTING.md docs/superpowers/plans/2026-08-21-issue-1106-host-native-cached-test-lane.md
  git commit -m "docs(contributing): document host-native test lane"
  ```
