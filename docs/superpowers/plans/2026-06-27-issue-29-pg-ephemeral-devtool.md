# Issue #29 — `with-ephemeral-postgres` → `devtool` Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Move the throwaway-PostgreSQL lifecycle from `scripts/with-ephemeral-postgres` into `tools/devtool` (Rust), expose it as both an in-process API and a `devtool pg run` CLI subcommand, delete the script, and give the `tools/` workspace tests a place to run.

**Architecture:** A new `tools/devtool/src/pg.rs` module owns the cluster lifecycle: pure argv/URL/SQL builders (unit-tested) wrapped by `with_ephemeral(|env| …)`, which boots a cluster via `initdb`/`pg_ctl`/`psql`, runs a closure with the connection endpoints, and tears down via an RAII `Drop` guard plus a SIGINT/SIGTERM handler. `coverage::emit` calls the API in-process; `devtool pg run -- <cmd>` is a thin CLI over the same API. A new `tools-test` xtask step runs the `tools/` workspace unit tests.

**Tech Stack:** Rust, clap (derive), anyhow, tempfile, signal-hook, PostgreSQL 16 CLI tools (`initdb`/`pg_ctl`/`psql`), cargo xtask.

## Global Constraints

- **Commit trailers:** NO `Co-Authored-By` trailers (jaunder override of the global default).
- **Per-task gate:** `cargo xtask check --no-test` (clippy + fmt over all workspaces) must pass before each task's commit. Invoke bare; pass/fail is the exit code.
- **Final gate:** `cargo xtask validate` (static + coverage + e2e) must pass before the branch is shipped — this is what exercises the real `devtool pg` lifecycle end-to-end via the `coverage` check.
- **Branch:** work on `worktree-issue-29-pg-ephemeral-devtool`; never commit on `main`.
- **Spec:** [docs/superpowers/specs/2026-06-27-issue-29-pg-ephemeral-devtool-design.md](../specs/2026-06-27-issue-29-pg-ephemeral-devtool-design.md). Governed by [ADR-0028](../../adr/0028-devtool-vs-xtask-boundary.md).
- **Parity values (copy verbatim):** host `127.0.0.1`; default port `54329` (env override `JAUNDER_PG_TEST_PORT`, empty string ⇒ default); server settings `fsync=off full_page_writes=off synchronous_commit=off max_connections=200`; role/db both named `jaunder` (`CREATE ROLE jaunder LOGIN CREATEDB;` + `CREATE DATABASE jaunder OWNER jaunder;`); env vars `JAUNDER_PG_TEST_URL=postgres://jaunder@127.0.0.1:<port>/jaunder` and `JAUNDER_PG_BOOTSTRAP_TEST_URL=postgres://postgres@127.0.0.1:<port>/postgres`.

---

### Task 1: Add the `tools-test` xtask step (test execution home)

The `tools/` workspace tests run in no gate today. Add a step so the existing
`emit.rs` tests (and the `pg` tests added later) actually execute. Doing this first
means every later task's tests are live in `cargo xtask check`.

**Files:**
- Modify: `xtask/src/steps/host_tests.rs`

**Interfaces:**
- Consumes: `crate::sh::step`, `crate::result::CommandResult` (already used here).
- Produces: a `tools-test` step running `cargo test --manifest-path tools/Cargo.toml`.

- [x] **Step 1: Add the step**

In `xtask/src/steps/host_tests.rs`, after the existing `xtask-tests` push, add a
second push (update the doc comment to mention both workspaces):

```rust
pub fn run(sh: &Shell, result: &mut CommandResult) {
    result.push(step(
        sh,
        "xtask-tests",
        "cargo",
        &["test", "--manifest-path", "xtask/Cargo.toml"],
    ));
    // tools/ is its own virtual workspace, excluded from every Nix check
    // (coverage excludes /tools/, devtoolBin is doCheck=false), so without this
    // its unit tests — devtool's pg/coverage logic — gate nowhere.
    result.push(step(
        sh,
        "tools-test",
        "cargo",
        &["test", "--manifest-path", "tools/Cargo.toml"],
    ));
}
```

- [x] **Step 2: Verify the previously-dead emit.rs tests now run**

Run: `cargo xtask check --no-test`
Expected: exit 0; the sidecar shows a `tools-test` step. Confirm it executed the
existing `emit.rs` tests:

Run: `jq -r '.steps[] | select(.name=="tools-test") | "\(.name) ok=\(.ok)"' .xtask/last-result.json`
Expected: `tools-test ok=true`

- [x] **Step 3: Commit**

```bash
git add xtask/src/steps/host_tests.rs
git commit -m "test(xtask): run tools/ workspace unit tests via new tools-test step"
```

---

### Task 2: Pure lifecycle helpers in `pg.rs` (TDD)

Pure functions only — no process spawning. These are the unit-tested core; Task 3
wraps them in the orchestration.

**Files:**
- Create: `tools/devtool/src/pg.rs`
- Modify: `tools/devtool/src/main.rs` (add `mod pg;`)

**Interfaces:**
- Produces (used by Tasks 3–5):
  - `pub struct PgEnv { pub test_url: String, pub bootstrap_url: String }`
  - `pub(crate) const HOST: &str` = `"127.0.0.1"`
  - `fn resolve_port(raw: Option<&str>) -> u16`
  - `fn app_url(host: &str, port: u16) -> String`
  - `fn bootstrap_url(host: &str, port: u16) -> String`
  - `fn initdb_args(pgdata: &Path) -> Vec<String>`
  - `fn server_settings(host: &str, port: u16, pgdata: &Path) -> Vec<String>`
  - `fn psql_args(host: &str, port: u16) -> Vec<String>`
  - `const BOOTSTRAP_SQL: &str`

- [ ] **Step 1: Register the module**

In `tools/devtool/src/main.rs`, add below `mod coverage;`:

```rust
mod pg;
```

- [ ] **Step 2: Write the failing tests**

Create `tools/devtool/src/pg.rs` with only the test module (and the imports it
needs) so the build fails on the missing items:

```rust
use std::path::Path;

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    #[test]
    fn port_defaults_when_unset_or_empty() {
        assert_eq!(resolve_port(None), 54329);
        assert_eq!(resolve_port(Some("")), 54329);
        assert_eq!(resolve_port(Some("55000")), 55000);
    }

    #[test]
    fn urls_match_bash_parity() {
        assert_eq!(app_url(HOST, 54329), "postgres://jaunder@127.0.0.1:54329/jaunder");
        assert_eq!(
            bootstrap_url(HOST, 54329),
            "postgres://postgres@127.0.0.1:54329/postgres"
        );
    }

    #[test]
    fn initdb_args_trust_no_sync() {
        let a = initdb_args(&PathBuf::from("/tmp/pg"));
        assert_eq!(
            a,
            ["-D", "/tmp/pg", "-U", "postgres", "-A", "trust", "--no-sync"]
        );
    }

    #[test]
    fn server_settings_disable_durability() {
        let s = server_settings(HOST, 54329, &PathBuf::from("/tmp/pg")).join(" ");
        assert!(s.contains("-c listen_addresses=127.0.0.1"));
        assert!(s.contains("-c port=54329"));
        assert!(s.contains("-c unix_socket_directories=/tmp/pg"));
        assert!(s.contains("-c max_connections=200"));
        assert!(s.contains("-c fsync=off"));
        assert!(s.contains("-c full_page_writes=off"));
        assert!(s.contains("-c synchronous_commit=off"));
    }

    #[test]
    fn psql_args_stop_on_error() {
        let a = psql_args(HOST, 54329);
        assert_eq!(
            a,
            [
                "-h", "127.0.0.1", "-p", "54329", "-U", "postgres", "-d", "postgres",
                "-v", "ON_ERROR_STOP=1"
            ]
        );
    }

    #[test]
    fn bootstrap_sql_creates_role_and_db() {
        assert!(BOOTSTRAP_SQL.contains("CREATE ROLE jaunder LOGIN CREATEDB;"));
        assert!(BOOTSTRAP_SQL.contains("CREATE DATABASE jaunder OWNER jaunder;"));
    }
}
```

- [ ] **Step 3: Run tests to verify they fail**

Run: `cargo test --manifest-path tools/Cargo.toml -p devtool pg::`
Expected: FAIL — `cannot find function/value` for `resolve_port`, `app_url`, etc.

- [ ] **Step 4: Write the minimal implementation**

Prepend to `tools/devtool/src/pg.rs` (above the test module):

```rust
pub(crate) const HOST: &str = "127.0.0.1";
const DEFAULT_PORT: u16 = 54329;

/// Connection endpoints handed to the wrapped command.
pub struct PgEnv {
    pub test_url: String,
    pub bootstrap_url: String,
}

const BOOTSTRAP_SQL: &str =
    "CREATE ROLE jaunder LOGIN CREATEDB;\nCREATE DATABASE jaunder OWNER jaunder;\n";

/// `JAUNDER_PG_TEST_PORT` with bash `${VAR:-54329}` semantics: unset OR empty ⇒ default.
fn resolve_port(raw: Option<&str>) -> u16 {
    match raw {
        Some(s) if !s.is_empty() => {
            s.parse().expect("JAUNDER_PG_TEST_PORT must be a valid TCP port")
        }
        _ => DEFAULT_PORT,
    }
}

fn app_url(host: &str, port: u16) -> String {
    format!("postgres://jaunder@{host}:{port}/jaunder")
}

fn bootstrap_url(host: &str, port: u16) -> String {
    format!("postgres://postgres@{host}:{port}/postgres")
}

fn initdb_args(pgdata: &Path) -> Vec<String> {
    vec![
        "-D".into(),
        pgdata.display().to_string(),
        "-U".into(),
        "postgres".into(),
        "-A".into(),
        "trust".into(),
        "--no-sync".into(),
    ]
}

/// `-c k=v` pairs for `pg_ctl -o`; durability disabled (cluster is discarded).
fn server_settings(host: &str, port: u16, pgdata: &Path) -> Vec<String> {
    [
        format!("listen_addresses={host}"),
        format!("port={port}"),
        format!("unix_socket_directories={}", pgdata.display()),
        "max_connections=200".to_string(),
        "fsync=off".to_string(),
        "full_page_writes=off".to_string(),
        "synchronous_commit=off".to_string(),
    ]
    .into_iter()
    .flat_map(|kv| ["-c".to_string(), kv])
    .collect()
}

fn psql_args(host: &str, port: u16) -> Vec<String> {
    vec![
        "-h".into(),
        host.into(),
        "-p".into(),
        port.to_string(),
        "-U".into(),
        "postgres".into(),
        "-d".into(),
        "postgres".into(),
        "-v".into(),
        "ON_ERROR_STOP=1".into(),
    ]
}
```

NOTE: `app_url`/`bootstrap_url`/`initdb_args`/`server_settings`/`psql_args`/
`BOOTSTRAP_SQL` are unused until Task 3 — add `#[allow(dead_code)]` on each (or a
module-level `#![allow(dead_code)]` at the top of `pg.rs`) so `tools-clippy`
(`-D warnings`) passes this task. Remove the allow in Task 3 once they are wired in.

- [ ] **Step 5: Run tests to verify they pass**

Run: `cargo test --manifest-path tools/Cargo.toml -p devtool pg::`
Expected: PASS (6 tests).

- [ ] **Step 6: Per-task gate + commit**

Run: `cargo xtask check --no-test`
Expected: exit 0.

```bash
git add tools/devtool/src/pg.rs tools/devtool/src/main.rs
git commit -m "feat(devtool): add pure ephemeral-postgres lifecycle helpers"
```

---

### Task 3: Cluster lifecycle — `with_ephemeral`, Drop guard, signal teardown

Wrap the Task 2 helpers in the real orchestration. Not unit-tested (no per-run
cluster boot — see spec); verified here by compile + clippy, and end-to-end by the
`coverage` check at ship.

**Files:**
- Modify: `tools/devtool/src/pg.rs`
- Modify: `tools/devtool/Cargo.toml` (add `tempfile`, `signal-hook`)

**Interfaces:**
- Consumes: all Task 2 helpers + `PgEnv`.
- Produces: `pub fn with_ephemeral<T>(body: impl FnOnce(&PgEnv) -> anyhow::Result<T>) -> anyhow::Result<T>` (used by Tasks 4–5).

- [ ] **Step 1: Add dependencies**

In `tools/devtool/Cargo.toml` under `[dependencies]`:

```toml
tempfile = "3"
signal-hook = "0.3"
```

- [ ] **Step 2: Implement the lifecycle**

Remove the dead-code allow from Task 2. Add to `tools/devtool/src/pg.rs` (above the
test module), and extend the imports at the top to:

```rust
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use anyhow::{bail, Context, Result};
use signal_hook::consts::{SIGINT, SIGTERM};
use signal_hook::iterator::Signals;
```

```rust
/// Owns a running ephemeral cluster's data dir; tears it down idempotently.
struct Cluster {
    pgdata: PathBuf,
    torn_down: AtomicBool,
}

impl Cluster {
    fn teardown(&self) {
        if self.torn_down.swap(true, Ordering::SeqCst) {
            return;
        }
        // Best-effort stop; the dir is discarded regardless.
        let _ = Command::new("pg_ctl")
            .args(["-D", &self.pgdata.display().to_string(), "-m", "immediate", "stop"])
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status();
        let _ = std::fs::remove_dir_all(&self.pgdata);
    }
}

impl Drop for Cluster {
    fn drop(&mut self) {
        self.teardown();
    }
}

fn run_checked(cmd: &mut Command) -> Result<()> {
    let status = cmd
        .stdout(Stdio::null())
        .status()
        .with_context(|| format!("spawning {cmd:?}"))?;
    if !status.success() {
        bail!("{cmd:?} failed with {status}");
    }
    Ok(())
}

fn bootstrap(host: &str, port: u16) -> Result<()> {
    use std::io::Write;
    let mut child = Command::new("psql")
        .args(psql_args(host, port))
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .spawn()
        .context("spawning psql for bootstrap")?;
    child
        .stdin
        .take()
        .context("psql stdin unavailable")?
        .write_all(BOOTSTRAP_SQL.as_bytes())
        .context("writing bootstrap SQL")?;
    let status = child.wait().context("waiting on psql")?;
    if !status.success() {
        bail!("psql bootstrap failed with {status}");
    }
    Ok(())
}

/// Boot a throwaway PostgreSQL 16 cluster, run `body` with its endpoints, and tear
/// down on every exit path (normal return, panic, or SIGINT/SIGTERM).
pub fn with_ephemeral<T>(body: impl FnOnce(&PgEnv) -> Result<T>) -> Result<T> {
    let port = resolve_port(std::env::var("JAUNDER_PG_TEST_PORT").ok().as_deref());
    let pgdata = tempfile::Builder::new()
        .prefix("jaunder-pg.")
        .tempdir()
        .context("creating PGDATA temp dir")?
        .keep(); // keep on disk; we remove it ourselves after stopping the server
    let cluster = Arc::new(Cluster {
        pgdata: pgdata.clone(),
        torn_down: AtomicBool::new(false),
    });

    run_checked(Command::new("initdb").args(initdb_args(&pgdata)))?;
    let settings = server_settings(HOST, port, &pgdata).join(" ");
    run_checked(Command::new("pg_ctl").args([
        "-D",
        &pgdata.display().to_string(),
        "-w",
        "start",
        "-o",
        &settings,
    ]))?;
    bootstrap(HOST, port)?;

    // Parity with the bash `trap cleanup INT TERM`: a dedicated thread tears the
    // cluster down on signal, then emulates the default disposition so the exit
    // code is right. The Drop guard still covers normal return + panic.
    let mut signals = Signals::new([SIGINT, SIGTERM]).context("installing signal handler")?;
    let sig_cluster = Arc::clone(&cluster);
    let handle = signals.handle();
    let joiner = std::thread::spawn(move || {
        if let Some(sig) = signals.forever().next() {
            sig_cluster.teardown();
            let _ = signal_hook::low_level::emulate_default_handler(sig);
        }
    });

    let env = PgEnv {
        test_url: app_url(HOST, port),
        bootstrap_url: bootstrap_url(HOST, port),
    };
    let result = body(&env);

    handle.close(); // unblock the signal thread on the normal path
    let _ = joiner.join();
    cluster.teardown();
    result
}
```

NOTE on `.keep()`: on older `tempfile` the method is `into_path()`. If
`cargo xtask check --no-test` reports `no method named keep`, use `.into_path()`
instead (same effect: take ownership of the dir so `TempDir::drop` won't delete it
while the server is still running).

- [ ] **Step 3: Per-task gate**

Run: `cargo xtask check --no-test`
Expected: exit 0 (compiles; clippy clean; Task 2 unit tests still pass under
`tools-test` when run, helpers now all used).

- [ ] **Step 4: Commit**

```bash
git add tools/devtool/src/pg.rs tools/devtool/Cargo.toml tools/Cargo.lock
git commit -m "feat(devtool): boot ephemeral postgres with Drop + signal teardown"
```

---

### Task 4: `devtool pg run -- <cmd>` CLI subcommand

**Files:**
- Modify: `tools/devtool/src/main.rs`
- Modify: `tools/devtool/src/pg.rs` (add `run_command`)

**Interfaces:**
- Consumes: `pg::with_ephemeral`, `PgEnv`.
- Produces: `pub fn run_command(cmd: &[String]) -> anyhow::Result<()>` (never returns on success — exits with the child's code).

- [ ] **Step 1: Add `run_command` to `pg.rs`**

```rust
/// CLI entry: run `cmd` with the ephemeral cluster's env, propagating its exit code.
pub fn run_command(cmd: &[String]) -> Result<()> {
    let code = with_ephemeral(|env| {
        let status = Command::new(&cmd[0])
            .args(&cmd[1..])
            .env("JAUNDER_PG_TEST_URL", &env.test_url)
            .env("JAUNDER_PG_BOOTSTRAP_TEST_URL", &env.bootstrap_url)
            .status()
            .with_context(|| format!("spawning {cmd:?}"))?;
        Ok(status.code().unwrap_or(1))
    })?;
    // with_ephemeral has already torn the cluster down by here, so exiting
    // (which skips destructors) is safe.
    std::process::exit(code);
}
```

- [ ] **Step 2: Wire the subcommand in `main.rs`**

Add a `Pg` arm to `Command` and a `PgCmd` enum; extend the `match`:

```rust
#[derive(Subcommand)]
enum Command {
    /// Coverage pipeline subcommands.
    #[command(subcommand)]
    Coverage(CoverageCmd),
    /// Ephemeral PostgreSQL subcommands.
    #[command(subcommand)]
    Pg(PgCmd),
}

#[derive(Subcommand)]
enum PgCmd {
    /// Run a command with a throwaway PostgreSQL 16 cluster.
    Run {
        /// Command (and its arguments) to run, after `--`.
        #[arg(trailing_var_arg = true, required = true)]
        cmd: Vec<String>,
    },
}
```

```rust
fn main() -> anyhow::Result<()> {
    let cli = Cli::parse();
    match cli.command {
        Command::Coverage(CoverageCmd::Emit { out }) => coverage::emit::run(&out),
        Command::Pg(PgCmd::Run { cmd }) => pg::run_command(&cmd),
    }
}
```

- [ ] **Step 3: Per-task gate + manual smoke**

Run: `cargo xtask check --no-test`
Expected: exit 0.

Manual smoke (host dev shell has `postgresql_16` on PATH):
Run: `cargo run --manifest-path tools/Cargo.toml -p devtool -- pg run -- bash -c 'echo "$JAUNDER_PG_TEST_URL"'`
Expected: prints `postgres://jaunder@127.0.0.1:54329/jaunder`, exits 0, leaves no
`/tmp/jaunder-pg.*` dir behind.

- [ ] **Step 4: Commit**

```bash
git add tools/devtool/src/main.rs tools/devtool/src/pg.rs
git commit -m "feat(devtool): add 'pg run' CLI over the ephemeral-postgres API"
```

---

### Task 5: Switch `coverage::emit` to the in-process API

**Files:**
- Modify: `tools/devtool/src/coverage/emit.rs:70-78`

**Interfaces:**
- Consumes: `crate::pg::with_ephemeral`, `PgEnv`; existing private `run_capture`.

- [ ] **Step 1: Replace the bash-shim call**

At the top of `emit.rs`, add `use crate::pg;`. Replace the block that builds
`nextest` (currently `run_capture(Command::new("bash").args(["scripts/with-ephemeral-postgres", …]))`)
with:

```rust
    // 2. Instrumented suite under an ephemeral PostgreSQL. Capture combined
    //    output for classification + the diagnostics bundle. A non-zero exit is
    //    NOT fatal here: a test failure or infra failure is reported via status.
    let nextest = pg::with_ephemeral(|env| {
        run_capture(Command::new("cargo").args([
            "llvm-cov",
            "--no-report",
            "nextest",
            "--show-progress",
            "none",
        ])
        .env("JAUNDER_PG_TEST_URL", &env.test_url)
        .env("JAUNDER_PG_BOOTSTRAP_TEST_URL", &env.bootstrap_url))
    })?;
```

(The `.env(...)` calls attach to the `Command` before `args`/after — both compile;
keep them chained on the `Command` as shown. `run_capture` already takes `&mut Command`.)

- [ ] **Step 2: Per-task gate**

Run: `cargo xtask check --no-test`
Expected: exit 0 (the 4 existing `emit.rs` classify/normalize tests still pass under
`tools-test`; no `bash`/`scripts/with-ephemeral-postgres` reference remains in
`emit.rs`).

Run: `rg -n 'with-ephemeral-postgres' tools/`
Expected: no matches.

- [ ] **Step 3: Commit**

```bash
git add tools/devtool/src/coverage/emit.rs
git commit -m "refactor(devtool): emit uses in-process pg::with_ephemeral, not the bash shim"
```

---

### Task 6: Delete the script, update docs/comments, full validate

**Files:**
- Delete: `scripts/with-ephemeral-postgres`
- Modify: `flake.nix` (~893–894 comment)
- Modify: `CONTRIBUTING.md:167`, `:306`

**Interfaces:** none (cleanup + docs).

- [ ] **Step 1: Delete the script**

```bash
git rm scripts/with-ephemeral-postgres
```

- [ ] **Step 2: Reword the flake comment**

In `flake.nix`, the coverage check's `nativeBuildInputs` comment currently reads
"devtool runs the whole test suite under an ephemeral PostgreSQL (via
`scripts/with-ephemeral-postgres`) so …". Change the parenthetical to "(via
`devtool pg`)". No derivation logic changes; `postgresql_16` stays in
`nativeBuildInputs`.

- [ ] **Step 3: Update CONTRIBUTING.md**

- Line ~167: change `scripts/with-ephemeral-postgres cargo nextest run -p jaunder`
  to `cargo run --manifest-path tools/Cargo.toml -p devtool -- pg run -- cargo nextest run -p jaunder`
  (or `devtool pg run -- cargo nextest run -p jaunder` if the surrounding prose
  already assumes a built `devtool` on PATH — match the existing phrasing).
- Line ~306: change "prefer `scripts/with-ephemeral-postgres`" to
  "prefer `devtool pg run -- …`".

- [ ] **Step 4: Confirm no dangling references**

Run: `rg -n 'with-ephemeral-postgres' --glob '!docs/archive/**'`
Expected: only `docs/adr/0028-devtool-vs-xtask-boundary.md` (historical table) — no
live code, flake, or CONTRIBUTING references.

- [ ] **Step 5: Final gate — full validate**

Run: `cargo xtask validate`
Expected: exit 0. This compiles `devtool` (Nix `devtoolBin`), runs the `coverage`
check which boots the **real** ephemeral cluster via the new `pg` module, the
`coverage-gate` (`tests-ok`), the `tools-test` host step, and both e2e checks.

Inspect the sidecar if anything is unclear:
Run: `jq '.steps[] | {name, ok}' .xtask/last-result.json`

- [ ] **Step 6: Commit**

```bash
git add scripts/with-ephemeral-postgres flake.nix CONTRIBUTING.md
git commit -m "chore(devtool): delete with-ephemeral-postgres script; point docs at devtool pg"
```

---

## Self-Review

- **Spec coverage:** `pg` module + API (T2/T3) ✓; `pg run` CLI (T4) ✓; Drop+signal
  cleanup (T3) ✓; `emit.rs` rewrite (T5) ✓; script deletion + flake/CONTRIBUTING
  updates (T6) ✓; pure unit tests (T2) ✓; `tools-test` gate step (T1) ✓; no e2e/no
  coverage-derivation-logic changes (respected) ✓; no new ADR (cross-ref only) ✓.
- **Type consistency:** `with_ephemeral<T>(impl FnOnce(&PgEnv) -> Result<T>) -> Result<T>`,
  `PgEnv { test_url, bootstrap_url }`, `run_command(&[String])`, helper signatures —
  identical across T2–T5.
- **Parity:** port/host/settings/SQL/env-var values match the spec's verbatim block
  and the original bash.
- **Placeholders:** none; every code step shows full code.
