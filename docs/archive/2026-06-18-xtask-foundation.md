# xtask Foundation & Nix-Dispatch Ladder — Implementation Plan (Plan A)

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Stand up the `cargo xtask` driver (library + thin JSON-emitting CLI) and reproduce the verify ladder with the new architecture — host does the inner loop (`check`: static + clippy); `validate` dispatches **all** test/coverage/e2e execution to the **Nix** checks (the environment that matches CI), GC-rooted and cachix-pulled, with whole-tree memoization. The existing scripts stay until Plan B retires them.

**Architecture:** A standalone `./xtask` Cargo workspace builds one binary whose `main.rs` only parses args and serializes results; all logic lives in the `xtask` library. `check` runs static checks + clippy on the host. `validate` runs `check`, then `nix build --out-link <gcroot> --accept-flake-config` the Nix `coverage` check (tests + coverage); `validate --full` additionally builds the Nix e2e + postgres-integration checks. xtask is host-side only and **never calls itself**; the Nix derivations run the raw tooling. Coverage *post-processing* (line-identity classification, auto-heal, JSON `coverage` block) is **Plan B** — in Plan A, `validate` relies on the existing Nix `coverage` check to gate regressions exactly as CI does today.

**Tech Stack:** Rust (stable), `clap` (derive), `xshell`, `serde` + `serde_json`, `anyhow`. Host toolchain: `cargo fmt`, `clippy`, `leptosfmt`, `prettier`, `cargo-deny`, `nix`. All tests/coverage/e2e: the Nix flake checks.

## Global Constraints

- **Spec:** `docs/superpowers/specs/2026-06-18-testing-coverage-orchestration-design.md`. This plan implements the foundation + Nix-dispatch ladder. Coverage post-processing (classification/auto-heal/JSON), the per-line baseline, CRAP host-side reporting, the CI cachix build/test push split, the `nextest`-check removal, and script retirement are **Plan B**.
- **Host runs no tests.** All unit/integration/coverage/e2e execution happens in the Nix checks. The host does only static checks + clippy.
- **Do not retire any existing script in Plan A.** `scripts/verify`, `scripts/check-coverage`, `scripts/with-ephemeral-postgres`, `scripts/e2e-local.sh`, `scripts/format` stay; the pre-push hook keeps calling `scripts/verify`. Retirement + hook switch are Plan B.
- **`./xtask` is a separate workspace**, excluded from the root workspace (`exclude = ["xtask"]`), with the alias `xtask = "run --manifest-path xtask/Cargo.toml --"` in `.cargo/config.toml`. Keep its dependency set light.
- **Every Nix invocation passes `--accept-flake-config`** (honors the `jaunder-org.cachix.org` substituter for the untrusted local user) **and `--out-link <stable path under .xtask/gcroots/>`** (a GC root so the closure survives `nix-collect-garbage`).
- **CI must never mutate the tree:** steps that can auto-fix take a `Mode` (`Fix` | `Check`); host uses `Fix`, the CI path uses `Check`.
- **Output contract:** concise human summary by default; `--json` prints the envelope; `.xtask/last-result.json` is written every run; exit code mirrors `result.ok` (0 ok, 1 failure, 2 usage). `.xtask/` is gitignored.
- **Commit after every task**, on branch `testing-coverage-orchestration` (never `main`).
- **Verify your own work:** run the relevant `cargo xtask …` command and inspect `.xtask/last-result.json` before marking a task done.

---

## File structure

- `xtask/Cargo.toml` — standalone workspace manifest.
- `xtask/src/main.rs` — arg parsing (clap) + serialization only.
- `xtask/src/lib.rs` — module wiring + `pub fn run(cli: Cli) -> anyhow::Result<CommandResult>`.
- `xtask/src/result.rs` — `CommandResult`, `StepResult`, `Mode`, sidecar writer, exit-code mapping.
- `xtask/src/sh.rs` — `xshell` step helper.
- `xtask/src/steps/static_checks.rs` — fmt/leptosfmt/prettier/cargo-deny/clippy (the `check` body).
- `xtask/src/steps/nix.rs` — `nix build` dispatch to the flake checks (the `validate` body).
- `xtask/src/memo.rs` — whole-tree key + last-green cache.
- `.cargo/config.toml` — the `xtask` alias.
- `.gitignore` — add `/.xtask/`.

---

## Task 1: Phase 0 — prove host↔Nix coverage congruence  ✅ COMPLETE (historical)

This task is **done** (commit `bbbca96`, findings `docs/superpowers/specs/2026-06-18-phase0-congruence-findings.md`). Its outcome **drove the architecture pivot**: host coverage congruence is mechanically fragile, so the design now runs all tests/coverage in Nix rather than denying network on the host. No further action; retained here for traceability. The follow-on congruence investigation is closed in beads (`jaunder-1bhw.10`); its mechanism is **not** built.

---

## Task 2: Scaffold the xtask workspace

**Files:**
- Create: `xtask/Cargo.toml`, `xtask/src/main.rs`, `xtask/src/lib.rs`
- Create/Modify: `.cargo/config.toml`
- Modify: `.gitignore`

**Interfaces:**
- Produces: `cargo xtask` runs and prints help; `pub fn run(cli: Cli) -> anyhow::Result<CommandResult>` (stub returning an empty ok result); `Cli` clap struct with a global `--json` flag and a `Command` enum (`Check`, `Validate { full: bool }`).

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

In the root `Cargo.toml`'s `[workspace]` table, add `xtask` to `exclude` (create the key if absent): `exclude = ["xtask"]`.

- [ ] **Step 3: Add the cargo alias**

`.cargo/config.toml` (create if missing, else add the entry):

```toml
[alias]
xtask = "run --manifest-path xtask/Cargo.toml --"
```

- [ ] **Step 4: Gitignore the sidecar/gcroot dir**

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
    /// Tight inner loop: static checks + clippy (host).
    Check,
    /// The hub: check + the Nix coverage check (tests+coverage). `--full` adds the Nix e2e + postgres-integration checks.
    Validate {
        #[arg(long)]
        full: bool,
    },
}

pub fn run(cli: Cli) -> anyhow::Result<CommandResult> {
    match cli.command {
        Command::Check => Ok(CommandResult::new("check")),
        Command::Validate { .. } => Ok(CommandResult::new("validate")),
    }
}
```

(Order Task 3 to land `result.rs` together with this so it compiles.)

- [ ] **Step 6: Verify it builds and runs**

Run: `cargo xtask check`
Expected: builds, prints an (empty) human summary, exits 0.
Run: `cargo xtask --help`
Expected: shows `check` and `validate` (with `--full`) and the `--json` flag.

- [ ] **Step 7: Commit**

```bash
git add xtask/ Cargo.toml .cargo/config.toml .gitignore
git commit -m "feat(xtask): scaffold standalone xtask workspace + CLI skeleton"
```

---

## Task 3: Result envelope, JSON sidecar, exit codes

**Files:**
- Create: `xtask/src/result.rs`
- Test: inline `#[cfg(test)]`

**Interfaces:**
- Produces:
  - `pub struct CommandResult { pub command: String, pub ok: bool, pub duration_ms: u128, pub memoized: bool, pub steps: Vec<StepResult> }`
  - `pub struct StepResult { pub name: String, pub ok: bool, pub skipped: bool, pub detail: Option<String> }`
  - `pub enum Mode { Fix, Check }`
  - `CommandResult::new(&str)`, `::push(StepResult)` (recomputes `ok = all steps ok||skipped`), `::report(json: bool)` (writes `.xtask/last-result.json` always; JSON to stdout if `json`, else human), `::exit_code() -> i32`
  - `StepResult::ok(name)`, `::fail(name)`, `::skip(name)`, `.detail(s)`

- [ ] **Step 1: Write the failing test**

In `xtask/src/result.rs`:

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn result_ok_reflects_steps_and_serializes_flat() {
        let mut r = CommandResult::new("validate");
        r.push(StepResult::ok("clippy").detail("0 warnings"));
        r.push(StepResult::fail("nix-coverage"));
        assert!(!r.ok);
        assert_eq!(r.exit_code(), 1);

        let v: serde_json::Value = serde_json::to_value(&r).unwrap();
        assert_eq!(v["command"], "validate");
        assert_eq!(v["ok"], false);
        assert_eq!(v["steps"][0]["name"], "clippy");
        assert_eq!(v["steps"][0]["detail"], "0 warnings");
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
Expected: FAIL (types/methods not defined).

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
Expected: `"check"` and `true`.

- [ ] **Step 6: Commit**

```bash
git add xtask/src/result.rs
git commit -m "feat(xtask): result envelope, JSON sidecar, exit-code mapping"
```

---

## Task 4: `sh` helper and the `check` subcommand (static + clippy)

Mirrors `scripts/verify` Phase 1 (fmt, leptosfmt, prettier, cargo-deny) + Phase 2 (clippy). Formatting auto-fixes in `Mode::Fix`, checks-only in `Mode::Check`.

**Files:**
- Create: `xtask/src/sh.rs`, `xtask/src/steps/static_checks.rs`
- Modify: `xtask/src/lib.rs` (wire `check`)

**Interfaces:**
- Produces:
  - `sh::step(sh: &Shell, name: &str, program: &str, args: &[&str]) -> StepResult` — runs a command, non-zero exit → failed `StepResult`, never panics.
  - `steps::static_checks::run(sh: &Shell, mode: Mode, result: &mut CommandResult)` — appends fmt/leptosfmt/prettier/cargo-deny/clippy steps.

- [ ] **Step 1: Write the `sh` helper**

`xtask/src/sh.rs`:

```rust
use xshell::Shell;

use crate::result::StepResult;

/// Run a command as a named step. Non-zero exit becomes a failed StepResult
/// rather than a panic, so one failing step does not abort the others.
pub fn step(sh: &Shell, name: &str, program: &str, args: &[&str]) -> StepResult {
    match sh.cmd(program).args(args).quiet().run() {
        Ok(()) => StepResult::ok(name),
        Err(err) => StepResult::fail(name).detail(err.to_string()),
    }
}
```

- [ ] **Step 2: Write the static-checks step set**

`xtask/src/steps/static_checks.rs` (commands copied from `scripts/verify`; `fmt`/`leptosfmt`/`prettier` switch on `Mode`):

```rust
use xshell::Shell;

use crate::result::{CommandResult, Mode};
use crate::sh::step;

pub fn run(sh: &Shell, mode: Mode, result: &mut CommandResult) {
    let fmt_args: &[&str] = match mode {
        Mode::Check => &["fmt", "--all", "--", "--check"],
        Mode::Fix => &["fmt", "--all"],
    };
    result.push(step(sh, "fmt", "cargo", fmt_args));

    let leptos_args: &[&str] = match mode {
        Mode::Check => &["--check", "."],
        Mode::Fix => &["."],
    };
    result.push(step(sh, "leptosfmt", "leptosfmt", leptos_args));

    let prettier_args: &[&str] = match mode {
        Mode::Check => &["--check", "."],
        Mode::Fix => &["-w", "."],
    };
    result.push(step(sh, "prettier", "prettier", prettier_args));

    result.push(step(sh, "cargo-deny", "cargo", &["deny", "check"]));

    result.push(step(
        sh,
        "clippy",
        "cargo",
        &["clippy", "--workspace", "--all-targets", "--", "-D", "warnings"],
    ));
}
```

> Confirm the exact fmt/leptosfmt/prettier/deny invocations against `scripts/verify` before finalizing; copy them verbatim, adjusting only the Fix/Check switch.

- [ ] **Step 3: Wire `check` in `lib.rs`**

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
Expected: on a clean tree, every step `ok`, exit 0; `.xtask/last-result.json` lists `fmt`/`leptosfmt`/`prettier`/`cargo-deny`/`clippy`.
Cross-check: `scripts/verify --fast` still passes (parity).

- [ ] **Step 5: Commit**

```bash
git add xtask/src/sh.rs xtask/src/steps/static_checks.rs xtask/src/lib.rs
git commit -m "feat(xtask): check subcommand — static checks + clippy with Fix/Check mode"
```

---

## Task 5: `validate` — dispatch to the Nix checks (GC-rooted, cachix-pulled)

`validate` runs `check`, then builds the Nix `coverage` check (tests + coverage, in the CI-matching sandbox). `validate --full` additionally builds the `e2e-sqlite`, `e2e-postgres`, and `postgres-integration` checks. Each `nix build` passes `--accept-flake-config` (cachix pull) and `--out-link .xtask/gcroots/<name>` (GC root). Coverage *post-processing* is Plan B; here the Nix `coverage` check's own pass/fail gates exactly as CI relies on today.

**Files:**
- Create: `xtask/src/steps/nix.rs`
- Modify: `xtask/src/lib.rs` (wire `validate`)

**Interfaces:**
- Consumes: `CommandResult`/`StepResult`; `Mode`.
- Produces: `steps::nix::run(full: bool, result: &mut CommandResult)` — appends a `nix-coverage` step (always) and, when `full`, `nix-e2e-sqlite` / `nix-e2e-postgres` / `nix-postgres-integration` steps.

- [ ] **Step 1: Implement the Nix dispatch**

`xtask/src/steps/nix.rs`:

```rust
use std::process::Command;

use crate::result::{CommandResult, StepResult};

/// The flake checks are Linux-only (`optionalAttrs isLinux` in flake.nix);
/// the project's CI host is x86_64-linux.
const SYSTEM: &str = "x86_64-linux";

pub fn run(full: bool, result: &mut CommandResult) {
    result.push(build_check("nix-coverage", "coverage"));
    if full {
        result.push(build_check("nix-e2e-sqlite", "e2e-sqlite"));
        result.push(build_check("nix-e2e-postgres", "e2e-postgres"));
        result.push(build_check("nix-postgres-integration", "postgres-integration"));
    }
}

/// `nix build --accept-flake-config --out-link .xtask/gcroots/<check> .#checks.<system>.<check>`.
/// --accept-flake-config honors the jaunder-org cachix substituter for the
/// untrusted local user; --out-link makes the closure a GC root.
fn build_check(step_name: &str, check: &str) -> StepResult {
    let _ = std::fs::create_dir_all(".xtask/gcroots");
    let out_link = format!(".xtask/gcroots/{check}");
    let installable = format!(".#checks.{SYSTEM}.{check}");
    let status = Command::new("nix")
        .args(["build", "--accept-flake-config", "--out-link", &out_link, &installable])
        .status();
    match status {
        Ok(s) if s.success() => StepResult::ok(step_name),
        Ok(s) => StepResult::fail(step_name).detail(format!("nix build {installable} exited with {s}")),
        Err(e) => StepResult::fail(step_name).detail(e.to_string()),
    }
}
```

> Confirm the check attribute names (`coverage`, `e2e-sqlite`, `e2e-postgres`, `postgres-integration`) against `flake.nix`'s `checks` set before finalizing, and confirm the `coverage` check gates coverage regressions (it is what CI relies on).

- [ ] **Step 2: Wire `validate` in `lib.rs`**

```rust
mod steps {
    pub mod static_checks;
    pub mod nix;
}

// in run():
Command::Validate { full } => {
    let sh = xshell::Shell::new()?;
    let mut result = CommandResult::new("validate");
    steps::static_checks::run(&sh, Mode::Fix, &mut result);
    steps::nix::run(full, &mut result);
    Ok(result)
}
```

- [ ] **Step 3: Verify**

Run: `cargo xtask validate --json | jq '.steps[].name'`
Expected: includes `fmt`/`clippy`/…/`nix-coverage`; on a green tree all `ok`. A GC root appears at `.xtask/gcroots/coverage`.
Run: `cargo xtask validate --full --json | jq '.steps[].name'`
Expected: additionally includes `nix-e2e-sqlite`, `nix-e2e-postgres`, `nix-postgres-integration`.
Cross-check: outcomes match `scripts/verify --full`'s Nix checks.

- [ ] **Step 4: Commit**

```bash
git add xtask/src/steps/nix.rs xtask/src/lib.rs
git commit -m "feat(xtask): validate dispatches tests/coverage/e2e to the Nix checks (GC-rooted, cachix-pulled)"
```

---

## Task 6: Memoization (skip green re-runs on an unchanged tree)

**Files:**
- Create: `xtask/src/memo.rs`
- Modify: `xtask/src/lib.rs` (gate `validate` on the memo)
- Test: inline `#[cfg(test)]`

**Interfaces:**
- Produces:
  - `memo::tree_key() -> anyhow::Result<String>` — a hash over all git-tracked file contents + `Cargo.lock` + the toolchain id (`rustc -Vv`).
  - `memo::last_green(command: &str) -> Option<String>`
  - `memo::record_green(command: &str, key: &str) -> anyhow::Result<()>`

- [ ] **Step 1: Write the failing test**

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tree_key_is_stable_for_unchanged_inputs() {
        let a = tree_key().unwrap();
        let b = tree_key().unwrap();
        assert_eq!(a, b);
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

/// A conservative whole-tree key: every git-tracked file's content (via the
/// blob SHAs in `git ls-files -s`), plus Cargo.lock and the exact toolchain.
/// Whole-tree (not per-crate) keeps it sound — coverage is a whole-suite property.
pub fn tree_key() -> Result<String> {
    let files = Command::new("git").args(["ls-files", "-s"]).output().context("git ls-files")?;
    let lock = std::fs::read("Cargo.lock").unwrap_or_default();
    let toolchain = Command::new("rustc").arg("-Vv").output().context("rustc -Vv")?;

    use std::hash::{Hash, Hasher};
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
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

> `DefaultHasher` is fine for a local cache key (not security-sensitive). Keep deps light per the spec.

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
// ... run static_checks + nix ...
if result.ok {
    memo::record_green("validate", &key)?;
}
```

Add `mod memo;` to `lib.rs`.

- [ ] **Step 6: Verify the skip end-to-end**

Run: `cargo xtask validate` (full run), then `cargo xtask validate` again with no edits.
Expected: second run prints `(memoized)`, exits 0 fast; `jq '.memoized' .xtask/last-result.json` is `true`. Touch any tracked file → next run is full again.

- [ ] **Step 7: Commit**

```bash
git add xtask/src/memo.rs xtask/src/lib.rs
git commit -m "feat(xtask): memoize validate against whole-tree key"
```

---

## Task 7: Parity verification & developer docs note

Confirms the new commands match the scripts they will eventually replace, and tells developers the new commands exist — without retiring anything.

**Files:**
- Modify: `CONTRIBUTING.md` (Testing section — add, do not remove)

- [ ] **Step 1: Run side-by-side parity**

```bash
scripts/verify --fast && cargo xtask check
scripts/verify --full   # host static/clippy/tests/coverage + Nix VM checks
cargo xtask validate --full   # host static/clippy + the same Nix checks
```

Expected: `cargo xtask check` ≡ `scripts/verify --fast` outcome; `cargo xtask validate --full`'s Nix-check outcomes match `scripts/verify --full`'s Nix checks.

- [ ] **Step 2: Add a CONTRIBUTING note (additive)**

Add a short paragraph under the Testing section: `cargo xtask check` / `validate` / `validate --full` are being introduced alongside the verify ladder — `check` runs static+clippy on the host, `validate` runs all tests/coverage/e2e via the Nix checks (matching CI). They will replace the ladder once Plan B (coverage post-processing + CI wiring) lands. Do **not** rewrite the existing ladder description yet — that is Plan B's documentation task.

- [ ] **Step 3: Commit**

```bash
git add CONTRIBUTING.md
git commit -m "docs: note the incoming cargo xtask check/validate commands"
```

---

## Self-review notes (for the implementer)

- **Scope:** Plan A stops before coverage post-processing (classification/auto-heal/JSON), the per-line baseline, host-side CRAP reporting, the CI cachix build/test push split, the `nextest`-check removal, the hook switch, and script retirement — all Plan B.
- **Parity is the safety net:** no script is removed, so each task cross-checks against the script/check it mirrors. If a step diverges, copy the script's exact command.
- **Order dependency:** Task 3 (`result.rs`) lands with Task 2 so it compiles; Task 5's `steps::nix` and Task 4's `steps::static_checks` are both referenced by `lib.rs` — keep the `mod steps { … }` block consistent as you add each.
- **Environment note for the implementer:** the Bash tool blocks `sed`/`grep`/`head`/`tail`/`awk` and rejects overly complex compound commands — use `rg`, `jq`, the Read tool, and simple commands. Nix builds are slow on first run but cachix-pulled (`--accept-flake-config`) and GC-rooted thereafter.
