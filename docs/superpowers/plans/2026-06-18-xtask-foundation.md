# xtask Foundation & Non-Coverage Ladder — Implementation Plan (Plan A)

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Stand up the `cargo xtask` driver (library + thin JSON-emitting CLI) and reproduce the non-coverage half of the verify ladder — `check`, and `validate`'s test/format/e2e steps — with tree-hash memoization, while leaving the existing scripts in place so the gate never loses coverage mid-migration.

**Architecture:** A standalone `./xtask` Cargo workspace (isolated `target/`, excluded from the root workspace) builds one binary whose `main.rs` only parses args and serializes results; all logic lives in the `xtask` library. Every command returns a typed `CommandResult` rendered both as a concise human summary and as a `.xtask/last-result.json` sidecar, with the process exit code mirroring `result.ok`. Orchestration shells out via `xshell`, mirroring the exact commands the current scripts run.

**Tech Stack:** Rust (stable), `clap` (derive), `xshell`, `serde` + `serde_json`, `anyhow`. Existing toolchain: `cargo-nextest`, `cargo leptos`, `cargo fmt`, `clippy`, `leptosfmt`, `prettier`, `cargo-deny`, PostgreSQL 16 (`initdb`/`pg_ctl`/`psql`).

## Global Constraints

- **Spec:** `docs/superpowers/specs/2026-06-18-testing-coverage-orchestration-design.md`. This plan implements only its foundation/non-coverage scope; the coverage engine is Plan B.
- **Do not retire any existing script in Plan A.** `scripts/verify`, `scripts/check-coverage`, `scripts/with-ephemeral-postgres`, `scripts/e2e-local.sh`, `scripts/format` stay; the pre-push hook keeps calling `scripts/verify`. Retirement + hook/flake switch happen in Plan B once coverage reaches parity.
- **`./xtask` is a separate workspace**, excluded from the root workspace (`exclude = ["xtask"]`), with the alias `xtask = "run --manifest-path xtask/Cargo.toml --"` in `.cargo/config.toml`. Keep its dependency set light (compile-time tax is paid on every invocation).
- **CI must never mutate the tree:** every step that can auto-fix takes a `Mode` (`Fix` | `Check`); the host path uses `Fix`, the Nix/CI path uses `Check`.
- **Output contract:** concise human summary by default; `--json` prints the envelope; the sidecar `.xtask/last-result.json` is written on every run regardless; exit code mirrors `result.ok` (0 = ok, 1 = failure, 2 = usage error). `.xtask/` is gitignored.
- **Backend parity:** the tests step must run the suite against SQLite *and* against a throwaway host PostgreSQL, mirroring `scripts/verify` today.
- **Commit after every task.** Do not commit on `main`; work on the `testing-coverage-orchestration` branch (already created).
- **Verify your own work:** run the relevant `cargo xtask …` command and inspect `.xtask/last-result.json` before marking a task done.

---

## File structure

- `xtask/Cargo.toml` — standalone workspace manifest for the driver.
- `xtask/src/main.rs` — arg parsing (clap) + serialization only.
- `xtask/src/lib.rs` — module wiring + the public `run(cli) -> CommandResult` entry point.
- `xtask/src/result.rs` — `CommandResult`, `StepResult`, `Mode`, sidecar writer, exit-code mapping.
- `xtask/src/sh.rs` — `xshell` step helpers (`run_step`, `run_parallel`).
- `xtask/src/postgres.rs` — `with_ephemeral_postgres` library helper (port of `scripts/with-ephemeral-postgres`).
- `xtask/src/steps/static_checks.rs` — fmt/leptosfmt/prettier/cargo-deny (Fix/Check), the `check` body.
- `xtask/src/steps/tests.rs` — nextest SQLite + host-PG parity.
- `xtask/src/steps/e2e.rs` — port of `scripts/e2e-local.sh`.
- `xtask/src/memo.rs` — tree-hash + last-green cache.
- `.cargo/config.toml` — the `xtask` alias (create or modify).
- `.gitignore` — add `.xtask/`.

---

## Task 1: Phase 0 — prove host↔Nix coverage congruence

This task builds no xtask code. It empirically validates the load-bearing assumption of Plan B (host coverage, run with network denied, reproduces the Nix baseline) before that plan is written. It uses **today's** `scripts/check-coverage`.

**Files:**
- Create: `docs/superpowers/specs/2026-06-18-phase0-congruence-findings.md`

**Interfaces:**
- Produces: a committed findings doc stating the exact divergence set and whether network-denial achieves congruence. Plan B consumes this.

- [ ] **Step 1: Capture the committed Nix baseline**

The committed `.coverage-manifest.json` *is* the Nix output. Snapshot it so the diffs below are unambiguous:

```bash
cp .coverage-manifest.json /tmp/baseline-nix.json
```

- [ ] **Step 2: Run host coverage WITH network (today's default) and snapshot**

```bash
scripts/check-coverage            # networked host run; rewrites .coverage-manifest.json on success
cp .coverage-manifest.json /tmp/host-networked.json
git checkout -- .coverage-manifest.json .crap-manifest.json
```

Expected: success. (If it fails for unrelated reasons, fix the environment first — this task assumes a green tree.)

- [ ] **Step 3: Run host coverage with network DENIED and snapshot**

`unshare -rn` creates a new network namespace with only a downed `lo`. The ephemeral PG's exported DSN is **TCP loopback** (`postgres://jaunder@127.0.0.1:54329/jaunder`), even though `scripts/with-ephemeral-postgres` *also* configures a unix socket in `PGDATA`. So bring `lo` up inside the namespace:

```bash
unshare -rn bash -c 'ip link set lo up && scripts/check-coverage'
cp .coverage-manifest.json /tmp/host-denied.json
git checkout -- .coverage-manifest.json .crap-manifest.json
```

Expected: success. If `unshare -rn` is not permitted or `ip` is unavailable, record that in the findings (it changes Plan B's network-denial mechanism) and try a fallback (`unshare --map-root-user --net`).

> **Plan B simplification to evaluate (record a recommendation in the findings):** the unix socket is already configured but unused — the DSN just uses TCP. If Plan B switches `JAUNDER_PG_TEST_URL` to the socket form (`postgres:///jaunder?host=$PGDATA`, which sqlx supports), the network-denied coverage run needs *no* loopback at all (plain `unshare -rn`, nothing to bring up). Note in the findings whether the socket DSN connects cleanly here, so Plan B can decide.

- [ ] **Step 4: Diff the three manifests**

```bash
echo '== networked host vs Nix baseline =='; diff <(jq -S . /tmp/host-networked.json) <(jq -S . /tmp/baseline-nix.json) || true
echo '== network-denied host vs Nix baseline =='; diff <(jq -S . /tmp/host-denied.json) <(jq -S . /tmp/baseline-nix.json) || true
```

Expected (the hypothesis): the **networked** diff shows higher coverage only on `common/src/websub/http.rs` and `server/src/commands.rs`; the **network-denied** diff is empty (host ≡ Nix).

- [ ] **Step 5: Write the findings doc**

Record: the exact files that diverged in each run, whether `unshare -rn` worked (and the working invocation), whether network-denied achieved an empty diff, and any newly-discovered divergence sources. State a clear verdict: *congruence via network-denial confirmed / needs adjustment*. This is the gate for starting Plan B.

- [ ] **Step 6: Commit**

```bash
git add docs/superpowers/specs/2026-06-18-phase0-congruence-findings.md
git commit -m "docs(spec): Phase 0 congruence findings — host network-denied vs Nix baseline"
```

---

## Task 2: Scaffold the xtask workspace

**Files:**
- Create: `xtask/Cargo.toml`, `xtask/src/main.rs`, `xtask/src/lib.rs`
- Create/Modify: `.cargo/config.toml`
- Modify: `.gitignore`

**Interfaces:**
- Produces: `cargo xtask` runs and prints help; `pub fn run(cli: Cli) -> anyhow::Result<CommandResult>` (stubbed to return an empty ok result in this task); `Cli` clap struct with a `Command` enum (`Check`, `Validate { full: bool }`, `E2e { vm: bool }`) and a global `--json` flag.

- [ ] **Step 1: Create the workspace manifest**

`xtask/Cargo.toml`:

```toml
[workspace]

[package]
name = "xtask"
version = "0.1.0"
edition = "2021"
publish = false

[dependencies]
clap = { version = "4", features = ["derive"] }
xshell = "0.2"
serde = { version = "1", features = ["derive"] }
serde_json = "1"
anyhow = "1"
```

- [ ] **Step 2: Exclude xtask from the root workspace**

In the root `Cargo.toml`'s `[workspace]` table, add `xtask` to `exclude` (create the key if absent):

```toml
exclude = ["xtask"]
```

- [ ] **Step 3: Add the cargo alias**

`.cargo/config.toml` (create if missing, else add the `[alias]` entry):

```toml
[alias]
xtask = "run --manifest-path xtask/Cargo.toml --"
```

- [ ] **Step 4: Gitignore the sidecar dir**

Append to `.gitignore`:

```
/.xtask/
```

- [ ] **Step 5: Write the CLI skeleton**

`xtask/src/main.rs`:

```rust
use clap::Parser;
use xtask::{run, Cli};

fn main() {
    let cli = Cli::parse();
    let json = cli.json;
    match run(cli) {
        Ok(result) => {
            result.report(json);
            std::process::exit(result.exit_code());
        }
        Err(err) => {
            eprintln!("xtask: {err:#}");
            std::process::exit(2);
        }
    }
}
```

`xtask/src/lib.rs`:

```rust
use clap::{Parser, Subcommand};

mod result;
pub use result::{CommandResult, Mode, StepResult};

#[derive(Parser)]
#[command(name = "xtask", about = "Jaunder dev orchestration")]
pub struct Cli {
    /// Emit the structured result envelope as JSON to stdout.
    #[arg(long, global = true)]
    pub json: bool,
    #[command(subcommand)]
    pub command: Command,
}

#[derive(Subcommand)]
pub enum Command {
    /// Tight inner loop: static checks + clippy.
    Check,
    /// The hub: check + tests + e2e (coverage added in Plan B).
    Validate {
        /// Also run the hermetic Nix VM checks.
        #[arg(long)]
        full: bool,
    },
    /// Run the end-to-end suite.
    E2e {
        /// Run in the Nix VM instead of on the host.
        #[arg(long)]
        vm: bool,
    },
}

pub fn run(cli: Cli) -> anyhow::Result<CommandResult> {
    match cli.command {
        Command::Check => Ok(CommandResult::new("check")),
        Command::Validate { .. } => Ok(CommandResult::new("validate")),
        Command::E2e { .. } => Ok(CommandResult::new("e2e")),
    }
}
```

(Task 3 fills in `result.rs`; this task may need a minimal stub of `CommandResult::new`/`report`/`exit_code` to compile — write it inline in `result.rs` per Task 3 and order the work so Task 3 lands the real module.)

- [ ] **Step 6: Verify it builds and runs**

Run: `cargo xtask check`
Expected: builds, prints a (currently empty) human summary, exits 0.
Run: `cargo xtask --help`
Expected: shows `check`, `validate`, `e2e` subcommands and the `--json` flag.

- [ ] **Step 7: Commit**

```bash
git add xtask/ Cargo.toml .cargo/config.toml .gitignore
git commit -m "feat(xtask): scaffold standalone xtask workspace + CLI skeleton"
```

---

## Task 3: Result envelope, JSON sidecar, exit codes

**Files:**
- Create: `xtask/src/result.rs`
- Test: inline `#[cfg(test)]` in `xtask/src/result.rs`

**Interfaces:**
- Produces:
  - `pub struct CommandResult { pub command: String, pub ok: bool, pub duration_ms: u128, pub memoized: bool, pub steps: Vec<StepResult> }`
  - `pub struct StepResult { pub name: String, pub ok: bool, pub skipped: bool, pub detail: Option<String> }`
  - `pub enum Mode { Fix, Check }`
  - `CommandResult::new(command: &str) -> Self` (ok=true, empty steps)
  - `CommandResult::push(&mut self, StepResult)` — appends and recomputes `ok = steps.iter().all(|s| s.ok || s.skipped)`
  - `CommandResult::report(&self, json: bool)` — writes `.xtask/last-result.json` always; prints JSON to stdout if `json`, else a concise human summary
  - `CommandResult::exit_code(&self) -> i32` — `if self.ok { 0 } else { 1 }`
  - `StepResult::ok(name)`, `StepResult::fail(name)`, `StepResult::skip(name)`, `.detail(s)` builder

- [ ] **Step 1: Write the failing test**

In `xtask/src/result.rs`:

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn result_ok_reflects_steps_and_serializes_flat() {
        let mut r = CommandResult::new("validate");
        r.push(StepResult::ok("tests").detail("691 passed"));
        r.push(StepResult::fail("e2e"));
        assert!(!r.ok, "a failing step must make the result not ok");
        assert_eq!(r.exit_code(), 1);

        let v: serde_json::Value = serde_json::to_value(&r).unwrap();
        assert_eq!(v["command"], "validate");
        assert_eq!(v["ok"], false);
        assert_eq!(v["steps"][0]["name"], "tests");
        assert_eq!(v["steps"][0]["detail"], "691 passed");
        assert_eq!(v["steps"][1]["ok"], false);
    }

    #[test]
    fn skipped_step_does_not_fail_result() {
        let mut r = CommandResult::new("check");
        r.push(StepResult::skip("clippy"));
        assert!(r.ok);
        assert_eq!(r.exit_code(), 0);
    }
}
```

- [ ] **Step 2: Run the test to verify it fails**

Run: `cargo test --manifest-path xtask/Cargo.toml result::tests`
Expected: FAIL (types/methods not yet defined).

- [ ] **Step 3: Implement the module**

```rust
use std::io::Write;
use std::path::Path;

use serde::Serialize;

#[derive(Clone, Copy)]
pub enum Mode {
    Fix,
    Check,
}

#[derive(Serialize)]
pub struct StepResult {
    pub name: String,
    pub ok: bool,
    pub skipped: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub detail: Option<String>,
}

impl StepResult {
    pub fn ok(name: &str) -> Self {
        Self { name: name.into(), ok: true, skipped: false, detail: None }
    }
    pub fn fail(name: &str) -> Self {
        Self { name: name.into(), ok: false, skipped: false, detail: None }
    }
    pub fn skip(name: &str) -> Self {
        Self { name: name.into(), ok: true, skipped: true, detail: None }
    }
    pub fn detail(mut self, detail: impl Into<String>) -> Self {
        self.detail = Some(detail.into());
        self
    }
}

#[derive(Serialize)]
pub struct CommandResult {
    pub command: String,
    pub ok: bool,
    pub duration_ms: u128,
    pub memoized: bool,
    pub steps: Vec<StepResult>,
}

impl CommandResult {
    pub fn new(command: &str) -> Self {
        Self { command: command.into(), ok: true, duration_ms: 0, memoized: false, steps: Vec::new() }
    }

    pub fn push(&mut self, step: StepResult) {
        self.steps.push(step);
        self.ok = self.steps.iter().all(|s| s.ok || s.skipped);
    }

    pub fn exit_code(&self) -> i32 {
        if self.ok { 0 } else { 1 }
    }

    pub fn report(&self, json: bool) {
        if let Err(err) = self.write_sidecar() {
            eprintln!("xtask: warning: could not write sidecar: {err}");
        }
        if json {
            println!("{}", serde_json::to_string_pretty(self).unwrap());
        } else {
            self.print_human();
        }
    }

    fn write_sidecar(&self) -> std::io::Result<()> {
        std::fs::create_dir_all(".xtask")?;
        let mut f = std::fs::File::create(Path::new(".xtask/last-result.json"))?;
        f.write_all(serde_json::to_string_pretty(self).unwrap().as_bytes())?;
        Ok(())
    }

    fn print_human(&self) {
        for s in &self.steps {
            let mark = if s.skipped { "skip" } else if s.ok { " ok " } else { "FAIL" };
            let detail = s.detail.as_deref().map(|d| format!(" — {d}")).unwrap_or_default();
            println!("[{mark}] {}{detail}", s.name);
        }
        let verdict = if self.ok { "PASSED" } else { "FAILED" };
        let memo = if self.memoized { " (memoized)" } else { "" };
        println!("xtask {} {verdict}{memo} in {} ms", self.command, self.duration_ms);
    }
}
```

- [ ] **Step 4: Run the tests to verify they pass**

Run: `cargo test --manifest-path xtask/Cargo.toml result::tests`
Expected: PASS (2 tests).

- [ ] **Step 5: Verify the sidecar end-to-end**

Run: `cargo xtask check && jq '.command, .ok' .xtask/last-result.json`
Expected: prints `"check"` and `true`.

- [ ] **Step 6: Commit**

```bash
git add xtask/src/result.rs
git commit -m "feat(xtask): result envelope, JSON sidecar, exit-code mapping"
```

---

## Task 4: `sh` helpers and the `check` subcommand (static + clippy)

Mirrors `scripts/verify` Phase 1 (fmt, leptosfmt, prettier, cargo-deny) + Phase 2 (clippy). Formatting auto-fixes in `Mode::Fix`, checks-only in `Mode::Check`.

**Files:**
- Create: `xtask/src/sh.rs`, `xtask/src/steps/static_checks.rs`
- Modify: `xtask/src/lib.rs` (wire `check`)

**Interfaces:**
- Consumes: `CommandResult`, `StepResult`, `Mode` (Task 3).
- Produces:
  - `sh::step(sh: &Shell, name: &str, args: &[&str]) -> StepResult` — runs a command, captures success/failure, never panics on non-zero exit.
  - `steps::static_checks::run(sh: &Shell, mode: Mode, result: &mut CommandResult)` — appends fmt/leptosfmt/prettier/deny/clippy steps.

- [ ] **Step 1: Write the `sh` helper**

`xtask/src/sh.rs`:

```rust
use xshell::{Cmd, Shell};

use crate::result::StepResult;

/// Run a command as a named step. Non-zero exit becomes a failed StepResult
/// rather than a panic, so one failing step does not abort the others.
pub fn step(sh: &Shell, name: &str, program: &str, args: &[&str]) -> StepResult {
    let cmd: Cmd = sh.cmd(program).args(args).quiet();
    match cmd.run() {
        Ok(()) => StepResult::ok(name),
        Err(err) => StepResult::fail(name).detail(err.to_string()),
    }
}
```

- [ ] **Step 2: Write the static-checks step set**

`xtask/src/steps/static_checks.rs` (commands copied from `scripts/verify`; `fmt`/`prettier` switch on `Mode`):

```rust
use xshell::Shell;

use crate::result::{CommandResult, Mode};
use crate::sh::step;

pub fn run(sh: &Shell, mode: Mode, result: &mut CommandResult) {
    // cargo fmt: --check in CI, write-in-place locally.
    let fmt_args: &[&str] = match mode {
        Mode::Check => &["fmt", "--all", "--", "--check"],
        Mode::Fix => &["fmt", "--all"],
    };
    result.push(step(sh, "fmt", "cargo", fmt_args));

    // leptosfmt: --check in CI, in-place locally.
    let leptos_args: &[&str] = match mode {
        Mode::Check => &["--check", "."],
        Mode::Fix => &["."],
    };
    result.push(step(sh, "leptosfmt", "leptosfmt", leptos_args));

    // prettier: --check in CI, -w locally.
    let prettier_args: &[&str] = match mode {
        Mode::Check => &["--check", "."],
        Mode::Fix => &["-w", "."],
    };
    result.push(step(sh, "prettier", "prettier", prettier_args));

    // cargo-deny: advisories/licenses/bans.
    result.push(step(sh, "cargo-deny", "cargo", &["deny", "check"]));

    // clippy: compiles the workspace; warnings are errors.
    result.push(step(
        sh,
        "clippy",
        "cargo",
        &["clippy", "--workspace", "--all-targets", "--", "-D", "warnings"],
    ));
}
```

> Note: confirm the exact fmt/leptosfmt/prettier/deny invocations against `scripts/verify`'s `run_step`/`run_parallel` lines before finalizing; copy them verbatim, adjusting only the Fix/Check switch.

- [ ] **Step 3: Wire `check` in `lib.rs`**

Replace the `Command::Check` arm and add the modules:

```rust
mod sh;
mod steps {
    pub mod static_checks;
}

// in run():
Command::Check => {
    let sh = xshell::Shell::new()?;
    let mut result = CommandResult::new("check");
    steps::static_checks::run(&sh, Mode::Fix, &mut result);
    Ok(result)
}
```

- [ ] **Step 4: Verify against the existing fast gate**

Run: `cargo xtask check`
Expected: on a clean tree, every step `ok`, exit 0, `.xtask/last-result.json` lists `fmt`/`leptosfmt`/`prettier`/`cargo-deny`/`clippy`.
Cross-check: `scripts/verify --fast` still passes (parity).

- [ ] **Step 5: Commit**

```bash
git add xtask/src/sh.rs xtask/src/steps/static_checks.rs xtask/src/lib.rs
git commit -m "feat(xtask): check subcommand — static checks + clippy with Fix/Check mode"
```

---

## Task 5: `with_ephemeral_postgres` library helper

Port of `scripts/with-ephemeral-postgres`: initdb cluster in a temp dir on `127.0.0.1:54329` (override via `JAUNDER_PG_TEST_PORT`) with a unix socket in `PGDATA`, role/db `jaunder`, exports `JAUNDER_PG_TEST_URL` + `JAUNDER_PG_BOOTSTRAP_TEST_URL`, runs a closure, tears the cluster down on every exit path.

**Files:**
- Create: `xtask/src/postgres.rs`
- Test: inline `#[cfg(test)]`

**Interfaces:**
- Produces: `pub fn with_ephemeral_postgres<T>(f: impl FnOnce(&PgEnv) -> T) -> anyhow::Result<T>` where `pub struct PgEnv { pub test_url: String, pub bootstrap_url: String }`. The cluster is stopped and the temp dir removed before the function returns (including on panic/early return).

- [ ] **Step 1: Write the failing test**

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ephemeral_pg_exposes_a_usable_url_and_tears_down() {
        let url = with_ephemeral_postgres(|env| {
            // psql can connect and the jaunder db exists.
            let status = std::process::Command::new("psql")
                .args(["-d", &env.test_url, "-c", "SELECT 1"])
                .status()
                .unwrap();
            assert!(status.success(), "psql should connect to the ephemeral cluster");
            env.test_url.clone()
        })
        .unwrap();
        assert!(url.starts_with("postgres://jaunder@127.0.0.1:"));
    }
}
```

- [ ] **Step 2: Run the test to verify it fails**

Run: `cargo test --manifest-path xtask/Cargo.toml postgres::tests`
Expected: FAIL (function not defined).

- [ ] **Step 3: Implement the helper**

```rust
use std::process::Command;

use anyhow::{bail, Context, Result};

pub struct PgEnv {
    pub test_url: String,
    pub bootstrap_url: String,
    pgdata: std::path::PathBuf,
}

impl Drop for PgEnv {
    fn drop(&mut self) {
        let _ = Command::new("pg_ctl")
            .args(["-D", &self.pgdata.to_string_lossy(), "-m", "immediate", "stop"])
            .status();
        let _ = std::fs::remove_dir_all(&self.pgdata);
    }
}

pub fn with_ephemeral_postgres<T>(f: impl FnOnce(&PgEnv) -> T) -> Result<T> {
    let host = "127.0.0.1";
    let port = std::env::var("JAUNDER_PG_TEST_PORT").unwrap_or_else(|_| "54329".into());
    let pgdata = tempdir()?;

    run("initdb", &["-D", &pgdata.to_string_lossy(), "-U", "postgres", "-A", "trust", "--no-sync"])?;

    let opts = format!(
        "-c listen_addresses={host} -c port={port} -c unix_socket_directories={pd} \
         -c max_connections=200 -c fsync=off -c full_page_writes=off -c synchronous_commit=off",
        pd = pgdata.to_string_lossy()
    );
    run("pg_ctl", &["-D", &pgdata.to_string_lossy(), "-w", "start", "-o", &opts])?;

    // Build the env now so Drop tears the cluster down even if role creation fails.
    let env = PgEnv {
        test_url: format!("postgres://jaunder@{host}:{port}/jaunder"),
        bootstrap_url: format!("postgres://postgres@{host}:{port}/postgres"),
        pgdata,
    };

    let sql = "CREATE ROLE jaunder LOGIN CREATEDB; CREATE DATABASE jaunder OWNER jaunder;";
    let status = Command::new("psql")
        .args(["-h", host, "-p", &port, "-U", "postgres", "-d", "postgres", "-v", "ON_ERROR_STOP=1", "-c", sql])
        .status()
        .context("running psql to create role/db")?;
    if !status.success() {
        bail!("failed to create jaunder role/database");
    }

    Ok(f(&env))
}

fn run(program: &str, args: &[&str]) -> Result<()> {
    let status = Command::new(program).args(args).status().with_context(|| format!("running {program}"))?;
    if !status.success() {
        bail!("{program} exited with {status}");
    }
    Ok(())
}

fn tempdir() -> Result<std::path::PathBuf> {
    let base = std::env::temp_dir().join(format!("jaunder-pg.{}", std::process::id()));
    std::fs::create_dir_all(&base)?;
    Ok(base)
}
```

> Note: if a more collision-proof temp dir is wanted, add the `tempfile` crate; the spec keeps deps light, so a pid-based dir is the default. Match `scripts/with-ephemeral-postgres` exactly on the `-o` server options.

- [ ] **Step 4: Run the test to verify it passes**

Run: `cargo test --manifest-path xtask/Cargo.toml postgres::tests`
Expected: PASS (requires `initdb`/`pg_ctl`/`psql` on PATH — present in the devShell).

- [ ] **Step 5: Commit**

```bash
git add xtask/src/postgres.rs
git commit -m "feat(xtask): with_ephemeral_postgres helper (port of with-ephemeral-postgres)"
```

---

## Task 6: `validate` tests step (SQLite + host-PG parity)

**Files:**
- Create: `xtask/src/steps/tests.rs`
- Modify: `xtask/src/lib.rs` (wire `validate`)

**Interfaces:**
- Consumes: `with_ephemeral_postgres`, `PgEnv` (Task 5); `CommandResult`/`StepResult`.
- Produces: `steps::tests::run(result: &mut CommandResult)` — appends a `tests-sqlite` step (`cargo nextest run --workspace`) and a `tests-postgres` step (the `jaunder` integration suite under an ephemeral cluster with `JAUNDER_PG_TEST_URL`/`JAUNDER_PG_BOOTSTRAP_TEST_URL` in env).

- [ ] **Step 1: Implement the tests step**

`xtask/src/steps/tests.rs`:

```rust
use std::process::Command;

use crate::postgres::with_ephemeral_postgres;
use crate::result::{CommandResult, StepResult};

pub fn run(result: &mut CommandResult) {
    // SQLite: whole workspace.
    result.push(nextest("tests-sqlite", &["run", "--workspace"], &[]));

    // PostgreSQL parity: jaunder integration suite against a throwaway cluster.
    let pg_step = with_ephemeral_postgres(|env| {
        nextest(
            "tests-postgres",
            &["run", "-p", "jaunder"],
            &[
                ("JAUNDER_PG_TEST_URL", env.test_url.as_str()),
                ("JAUNDER_PG_BOOTSTRAP_TEST_URL", env.bootstrap_url.as_str()),
            ],
        )
    })
    .unwrap_or_else(|err| StepResult::fail("tests-postgres").detail(err.to_string()));
    result.push(pg_step);
}

fn nextest(name: &str, args: &[&str], env: &[(&str, &str)]) -> StepResult {
    let mut cmd = Command::new("cargo");
    cmd.arg("nextest").args(args);
    for (k, v) in env {
        cmd.env(k, v);
    }
    match cmd.status() {
        Ok(s) if s.success() => StepResult::ok(name),
        Ok(s) => StepResult::fail(name).detail(format!("nextest exited with {s}")),
        Err(e) => StepResult::fail(name).detail(e.to_string()),
    }
}
```

> Note: confirm the exact PG-suite selection against `scripts/verify` Phase 3 (`check-coverage`'s second pass runs the `jaunder` integration tests against PG). Match its `-p`/filter precisely so parity holds.

- [ ] **Step 2: Wire `validate` in `lib.rs`**

```rust
mod postgres;
mod steps {
    pub mod static_checks;
    pub mod tests;
    pub mod e2e; // added in Task 7
}

// in run():
Command::Validate { full } => {
    let sh = xshell::Shell::new()?;
    let mut result = CommandResult::new("validate");
    steps::static_checks::run(&sh, Mode::Fix, &mut result);
    steps::tests::run(&mut result);
    steps::e2e::run(&mut result); // Task 7
    if full {
        // Nix VM tier wired in Plan B; for now record a skipped step.
        result.push(crate::result::StepResult::skip("nix-vm"));
    }
    Ok(result)
}
```

- [ ] **Step 3: Verify**

Run: `cargo xtask validate --json | jq '.steps[].name'`
Expected: includes `tests-sqlite` and `tests-postgres`; both `ok` on a green tree.
Cross-check: matches what `scripts/verify` runs in its tests phase.

- [ ] **Step 4: Commit**

```bash
git add xtask/src/steps/tests.rs xtask/src/lib.rs
git commit -m "feat(xtask): validate tests step — SQLite + host-PostgreSQL parity"
```

---

## Task 7: `validate` e2e step (port of e2e-local.sh)

**Files:**
- Create: `xtask/src/steps/e2e.rs`
- Modify: `xtask/src/lib.rs` (wire the `E2e` subcommand to the same function)

**Interfaces:**
- Produces: `steps::e2e::run(result: &mut CommandResult)` — clears any straggler server on port 3000, sets the temp-storage env vars from `e2e-local.sh`, runs `cargo leptos end-to-end`, cleans up the temp dir.

- [ ] **Step 1: Implement the e2e step**

`xtask/src/steps/e2e.rs` (env vars and pkill copied from `scripts/e2e-local.sh`):

```rust
use std::process::Command;

use crate::result::{CommandResult, StepResult};

pub fn run(result: &mut CommandResult) {
    result.push(run_inner());
}

fn run_inner() -> StepResult {
    // Clear any leftover server holding port 3000 (the [t]arget bracket trick
    // keeps pkill from matching itself); ignore "no process" failures.
    let _ = Command::new("pkill").args(["-f", "[t]arget/.*jaunder"]).status();

    let tmp = match tempdir() {
        Ok(t) => t,
        Err(e) => return StepResult::fail("e2e").detail(e.to_string()),
    };
    let storage = tmp.join("storage");
    let _ = std::fs::create_dir_all(&storage);
    let db = tmp.join("jaunder.db");

    let status = Command::new("cargo")
        .args(["leptos", "end-to-end"])
        .env("JAUNDER_STORAGE_PATH", &storage)
        .env("JAUNDER_DB", format!("sqlite:{}", db.display()))
        .env("JAUNDER_DB_PATH", &db)
        .env("JAUNDER_MAIL_CAPTURE_FILE", tmp.join("mail.jsonl"))
        .env("JAUNDER_WEBSUB_CAPTURE_FILE", tmp.join("websub.jsonl"))
        .status();

    let _ = std::fs::remove_dir_all(&tmp);

    match status {
        Ok(s) if s.success() => StepResult::ok("e2e"),
        Ok(s) => StepResult::fail("e2e").detail(format!("cargo leptos end-to-end exited with {s}")),
        Err(e) => StepResult::fail("e2e").detail(e.to_string()),
    }
}

fn tempdir() -> std::io::Result<std::path::PathBuf> {
    let dir = std::env::temp_dir().join(format!("jaunder-e2e.{}", std::process::id()));
    std::fs::create_dir_all(&dir)?;
    Ok(dir)
}
```

> Note: copy the exact env-var set and the exact `pkill` pattern from `scripts/e2e-local.sh`; adjust only the language. The `--vm` path is a Plan B concern (Nix VM e2e) — for now `E2e { vm: true }` records a skipped `nix-vm-e2e` step.

- [ ] **Step 2: Wire the standalone `E2e` subcommand**

```rust
Command::E2e { vm } => {
    let mut result = CommandResult::new("e2e");
    if vm {
        result.push(StepResult::skip("nix-vm-e2e"));
    } else {
        steps::e2e::run(&mut result);
    }
    Ok(result)
}
```

- [ ] **Step 3: Verify**

Run: `cargo xtask e2e && jq '.steps[0]' .xtask/last-result.json`
Expected: `e2e` step `ok` on a green tree (parity with `scripts/e2e-local.sh`).

- [ ] **Step 4: Commit**

```bash
git add xtask/src/steps/e2e.rs xtask/src/lib.rs
git commit -m "feat(xtask): validate/e2e step — host end-to-end (port of e2e-local.sh)"
```

---

## Task 8: Memoization (skip green re-runs on an unchanged tree)

**Files:**
- Create: `xtask/src/memo.rs`
- Modify: `xtask/src/lib.rs` (gate `validate` on the memo)
- Test: inline `#[cfg(test)]`

**Interfaces:**
- Produces:
  - `memo::tree_key() -> anyhow::Result<String>` — a hash over all git-tracked file contents + `Cargo.lock` + the toolchain id (`rustc -Vv`).
  - `memo::last_green(command: &str) -> Option<String>` — reads `.xtask/green-<command>.key`.
  - `memo::record_green(command: &str, key: &str)` — writes it.

- [ ] **Step 1: Write the failing test**

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tree_key_is_stable_for_unchanged_inputs() {
        let a = tree_key().unwrap();
        let b = tree_key().unwrap();
        assert_eq!(a, b, "hashing the same tree twice must be stable");
        assert!(!a.is_empty());
    }
}
```

- [ ] **Step 2: Run the test to verify it fails**

Run: `cargo test --manifest-path xtask/Cargo.toml memo::tests`
Expected: FAIL (function not defined).

- [ ] **Step 3: Implement memoization**

```rust
use std::process::Command;

use anyhow::{Context, Result};

/// A conservative whole-tree key: every git-tracked file's content, plus
/// Cargo.lock and the exact toolchain. Whole-tree (not per-crate) keeps the
/// memo sound for coverage, which is a whole-suite property (Plan B).
pub fn tree_key() -> Result<String> {
    // `git ls-files -s` prints mode, blob SHA, stage, and path for every
    // tracked file — the blob SHA already content-hashes each file.
    let files = Command::new("git").args(["ls-files", "-s"]).output().context("git ls-files")?;
    let lock = std::fs::read("Cargo.lock").unwrap_or_default();
    let toolchain = Command::new("rustc").arg("-Vv").output().context("rustc -Vv")?;

    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    use std::hash::{Hash, Hasher};
    files.stdout.hash(&mut hasher);
    lock.hash(&mut hasher);
    toolchain.stdout.hash(&mut hasher);
    Ok(format!("{:016x}", hasher.finish()))
}

pub fn last_green(command: &str) -> Option<String> {
    std::fs::read_to_string(format!(".xtask/green-{command}.key")).ok().map(|s| s.trim().to_string())
}

pub fn record_green(command: &str, key: &str) -> Result<()> {
    std::fs::create_dir_all(".xtask")?;
    std::fs::write(format!(".xtask/green-{command}.key"), key)?;
    Ok(())
}
```

> Note: `DefaultHasher` is fine for a local cache key (not security-sensitive). If a stronger/portable hash is wanted later, swap in `sha2` — but that adds a dep; keep it light per the spec.

- [ ] **Step 4: Run the test to verify it passes**

Run: `cargo test --manifest-path xtask/Cargo.toml memo::tests`
Expected: PASS.

- [ ] **Step 5: Gate `validate` on the memo**

In the `Command::Validate` arm, before running steps:

```rust
let key = memo::tree_key()?;
if memo::last_green("validate").as_deref() == Some(key.as_str()) {
    let mut result = CommandResult::new("validate");
    result.memoized = true;
    return Ok(result); // ok=true, no steps — "tree unchanged since last green"
}
// ... run steps ...
if result.ok {
    memo::record_green("validate", &key)?;
}
```

Add `mod memo;` to `lib.rs`.

- [ ] **Step 6: Verify the skip end-to-end**

Run: `cargo xtask validate` (first run: full), then `cargo xtask validate` again with no edits.
Expected: second run prints `(memoized)`, exits 0 fast; `jq '.memoized' .xtask/last-result.json` is `true`. Touch any tracked file → next run is full again.

- [ ] **Step 7: Commit**

```bash
git add xtask/src/memo.rs xtask/src/lib.rs
git commit -m "feat(xtask): memoize validate against whole-tree key"
```

---

## Task 9: Parity verification & developer docs note

Confirms the new commands match the scripts they will eventually replace, and tells developers the new inner-loop command exists — without retiring anything yet.

**Files:**
- Modify: `CONTRIBUTING.md` (Testing section — add, do not remove)

**Interfaces:**
- Consumes: all prior tasks.

- [ ] **Step 1: Run side-by-side parity**

```bash
scripts/verify --fast && cargo xtask check
scripts/verify        # full non-VM gate (tests + coverage + e2e)
cargo xtask validate  # tests + e2e (no coverage yet — Plan B)
```

Expected: `cargo xtask check` ≡ `scripts/verify --fast` outcome; `cargo xtask validate`'s tests + e2e steps match `scripts/verify`'s test/e2e outcome (coverage intentionally absent until Plan B).

- [ ] **Step 2: Add a CONTRIBUTING note (additive)**

Add a short paragraph under the Testing section noting that `cargo xtask check` / `cargo xtask validate` are being introduced alongside the verify ladder and will replace it once coverage parity lands (Plan B). Do **not** rewrite the existing ladder description yet — that rewrite is Plan B's documentation task.

- [ ] **Step 3: Commit**

```bash
git add CONTRIBUTING.md
git commit -m "docs: note the incoming cargo xtask check/validate commands"
```

---

## Self-review notes (for the implementer)

- **Scope:** Plan A deliberately stops before coverage, network-denial, the Nix derivation switch, the pre-push hook switch, and script retirement — all Plan B, gated by Task 1's findings.
- **Parity is the safety net:** because no script is removed, every task can be cross-checked against the script it mirrors. If a step diverges from its script, copy the script's exact command/env.
- **Order dependency:** Task 3 (`result.rs`) must land with or before Task 2's stubs compile; Task 5 (`postgres.rs`) precedes Task 6; Task 7's `steps::e2e` is referenced by Task 6's `lib.rs` wiring, so land Task 7's module file or stub `steps::e2e::run` when wiring Task 6.
