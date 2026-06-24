# Coverage Pipeline Rust Migration — Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Replace the fragile `scripts/check-coverage` (bash/awk/jq) with a maintainable Rust `devtool`, make coverage failures self-describing and their first-hand data exfiltrable as CI artifacts, and fold in issues #2, #7, #3, #11.

**Architecture:** A new minimal `coverage` library crate holds the cross-context pieces (the `status.json` sentinel schema + path-normalization). A new `devtool` binary crate runs *inside* the Nix coverage sandbox and owns the `cargo llvm-cov` orchestration, always producing `$out` (reports + diagnostics bundle + `status.json`). The Nix `coverage` check splits into a **producer** (always succeeds, always emits) and a tiny **consumer** (`runCommand` that fails on a bad sentinel). `xtask` stays host-only and keeps ALL gate/heal/classify/diffmap logic, now reading `status.json` to categorize failures and copying diagnostics out on catastrophic infra failures.

**Tech Stack:** Rust (edition 2021), clap 4 (derive), serde/serde_json, anyhow, xshell; Nix flakes + crane (`mkCargoDerivation`, `runCommand`); `cargo-llvm-cov`, `cargo-nextest`, `cargo-crap`; GitHub Actions.

## Global Constraints

- **No `Co-Authored-By` trailers** in any commit (overrides global default).
- **Never commit on `main`.** All work lands on the worktree branch `worktree-coverage-rust-migration`.
- **Per-task gate for code touching the main/xtask workspaces:** `cargo xtask check --no-test` (fmt + clippy, no Nix coverage build). For the standalone `coverage`/`devtool` crates, gate with their crate-local commands (given in each task).
- **Don't reinforce defaults:** never set a value explicitly to what it already defaults to; lock load-bearing behavior with a test instead.
- New crates use **`edition = "2021"`**, no `rust-version`, `publish = false`.
- **The coverage report is always built by Nix from committed HEAD** (the flake ignores uncommitted tracked-file edits). Never diff against the working tree when mapping baseline lines.
- **Respect the `/xtask/` cache exclusion:** host-gate logic stays in `xtask` (excluded from the coverage derivation src). The `coverage` lib and `devtool` are *not* under `xtask/` and *are* included in the sandbox src.
- **Single coverage model:** gap-based `coverage-baseline.json` + `crap-manifest.json` (both committed at repo root). The legacy percent model is dead — delete it.
- Run all commands from the worktree root `/home/mdorman/src/jaunder/.claude/worktrees/coverage-rust-migration`.

---

## File Structure

**New crates (standalone, each its own one-crate workspace — NOT members of the main workspace, so their source is never instrumented by the coverage run):**

- `coverage/Cargo.toml` — new lib crate `coverage`.
- `coverage/src/lib.rs` — re-exports `status` + `pathnorm`.
- `coverage/src/status.rs` — `StatusCategory` enum + `CoverageStatus` struct (the `status.json` schema), serde.
- `coverage/src/pathnorm.rs` — `strip_prefix_lines` / `normalize_report_paths` (port of the bash path strip).
- `devtool/Cargo.toml` — new bin crate `devtool` (path-depends on `../coverage`).
- `devtool/src/main.rs` — clap CLI (`devtool coverage emit`), forward-compatible subcommand tree.
- `devtool/src/coverage/mod.rs` — `emit` module wiring.
- `devtool/src/coverage/emit.rs` — the emit orchestration + status/diagnostics detection.

**Modified:**

- `xtask/Cargo.toml` — add `coverage = { path = "../coverage" }`.
- `xtask/src/steps/nix.rs` — read `status.json`; `--keep-failed` + diagnostics copy-out; category-aware `StepResult`.
- `xtask/src/coverage/mod.rs` — consume the in-sandbox category; replace `git_diff_unified0` (HEAD→worktree) with anchor→HEAD mapping (#3/#11); heal-idempotence regression test (#7).
- `xtask/src/steps/static_checks.rs` — also fmt/clippy the `coverage` + `devtool` crates so `validate` covers them.
- `flake.nix` — producer/consumer split; build+run `devtool`; drop `gawk`/`jq`; delete `coverage-update`.
- `.github/workflows/ci.yml` — `actions/upload-artifact@v4` with `if: always()`.

**Deleted:**

- `scripts/check-coverage`.

---

## Phase A — Shared `coverage` library crate

### Task A1: Scaffold the `coverage` crate with the status schema

**Files:**
- Create: `coverage/Cargo.toml`
- Create: `coverage/src/lib.rs`
- Create: `coverage/src/status.rs`

**Interfaces:**
- Produces:
  - `coverage::status::StatusCategory` — `enum { TestsOk, TestFailure, Infra }`, serde rename_all = "kebab-case" (`tests-ok`, `test-failure`, `infra`).
  - `coverage::status::CoverageStatus` — `struct { category: StatusCategory, failed_tests: Vec<String>, infra_detail: Option<String> }`, serde.
  - `CoverageStatus::to_json(&self) -> String` (pretty + trailing newline), `CoverageStatus::from_json(s: &str) -> anyhow::Result<CoverageStatus>`.

- [ ] **Step 1: Write `coverage/Cargo.toml`**

```toml
[workspace]

[package]
name = "coverage"
version = "0.1.0"
edition = "2021"
publish = false

[dependencies]
serde = { version = "1", features = ["derive"] }
serde_json = "1"
anyhow = "1"
```

- [ ] **Step 2: Write the failing test in `coverage/src/status.rs`**

```rust
//! The in-sandbox coverage sentinel: what `devtool coverage emit` can know
//! without git/baseline context (only test pass/fail + infrastructure health).
//! Written to `$out/status.json`; read by both the Nix consumer derivation and
//! the host `xtask` gate.

use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum StatusCategory {
    TestsOk,
    TestFailure,
    Infra,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct CoverageStatus {
    pub category: StatusCategory,
    #[serde(default)]
    pub failed_tests: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub infra_detail: Option<String>,
}

impl CoverageStatus {
    pub fn to_json(&self) -> String {
        format!("{}\n", serde_json::to_string_pretty(self).expect("serialize status"))
    }

    pub fn from_json(s: &str) -> anyhow::Result<Self> {
        Ok(serde_json::from_str(s)?)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn roundtrips_through_json() {
        let s = CoverageStatus {
            category: StatusCategory::TestFailure,
            failed_tests: vec!["web_posts::case_3".into()],
            infra_detail: None,
        };
        let back = CoverageStatus::from_json(&s.to_json()).unwrap();
        assert_eq!(s, back);
    }

    #[test]
    fn category_serializes_kebab_case() {
        let s = CoverageStatus {
            category: StatusCategory::Infra,
            failed_tests: vec![],
            infra_detail: Some("ENOSPC".into()),
        };
        assert!(s.to_json().contains("\"infra\""));
    }
}
```

- [ ] **Step 3: Write `coverage/src/lib.rs`**

```rust
pub mod pathnorm;
pub mod status;
```

(Note: `pathnorm` is added in Task A2; create it now as an empty module file so the crate compiles.)

```rust
// coverage/src/pathnorm.rs (placeholder until Task A2)
```

- [ ] **Step 4: Run tests to verify they pass**

Run: `cargo test --manifest-path coverage/Cargo.toml`
Expected: PASS (2 tests).

- [ ] **Step 5: Lint + format gate**

Run: `cargo fmt --manifest-path coverage/Cargo.toml --check`
Run: `cargo clippy --manifest-path coverage/Cargo.toml --all-targets -- -D warnings`
Expected: both clean.

- [ ] **Step 6: Commit**

```bash
git add coverage/Cargo.toml coverage/src/lib.rs coverage/src/status.rs coverage/src/pathnorm.rs
git commit -m "feat(coverage): add shared status-sentinel schema crate"
```

### Task A2: Port path normalization into the `coverage` crate

The bash `normalize_report_paths` (check-coverage:62-79) strips the absolute Nix-sandbox prefix from `cargo llvm-cov report --text` file-header lines (those ending in `.rs:`). Port it to Rust.

**Files:**
- Modify: `coverage/src/pathnorm.rs`

**Interfaces:**
- Produces: `coverage::pathnorm::normalize_report_text(report: &str, abs_root: &str) -> String` — strips a leading `"{abs_root}/"` from every line ending in `.rs:`; leaves all other lines untouched; idempotent when paths are already relative.

- [ ] **Step 1: Write the failing test in `coverage/src/pathnorm.rs`**

```rust
//! Path normalization for the `cargo llvm-cov report --text` output: rewrite the
//! absolute Nix-sandbox `.rs:` file-header lines to repo-relative ones. Ports
//! the bash `normalize_report_paths` from the retired `scripts/check-coverage`.

/// Strip a leading `"{abs_root}/"` prefix from every file-header line (one
/// ending in `.rs:`). Non-header lines are passed through verbatim. Idempotent.
pub fn normalize_report_text(report: &str, abs_root: &str) -> String {
    let prefix = format!("{abs_root}/");
    let mut out = String::with_capacity(report.len());
    for line in report.split_inclusive('\n') {
        let (content, nl) = match line.strip_suffix('\n') {
            Some(c) => (c, "\n"),
            None => (line, ""),
        };
        if content.ends_with(".rs:") {
            out.push_str(content.strip_prefix(&prefix).unwrap_or(content));
        } else {
            out.push_str(content);
        }
        out.push_str(nl);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn strips_abs_prefix_from_header_lines_only() {
        let root = "/build/source";
        let input = "\
/build/source/server/src/x.rs:
    1|    5|fn f() {}
/build/source/web/src/y.rs:
    2|    0|let z = 1;
";
        let got = normalize_report_text(input, root);
        assert_eq!(got, "\
server/src/x.rs:
    1|    5|fn f() {}
web/src/y.rs:
    2|    0|let z = 1;
");
    }

    #[test]
    fn is_idempotent_on_relative_paths() {
        let root = "/build/source";
        let input = "server/src/x.rs:\n    1|    5|fn f() {}\n";
        assert_eq!(normalize_report_text(input, root), input);
    }
}
```

- [ ] **Step 2: Run tests to verify they pass**

Run: `cargo test --manifest-path coverage/Cargo.toml`
Expected: PASS (4 tests total).

- [ ] **Step 3: Lint + format gate**

Run: `cargo fmt --manifest-path coverage/Cargo.toml --check`
Run: `cargo clippy --manifest-path coverage/Cargo.toml --all-targets -- -D warnings`
Expected: clean.

- [ ] **Step 4: Commit**

```bash
git add coverage/src/pathnorm.rs
git commit -m "feat(coverage): port report path normalization to Rust"
```

---

## Phase B — `devtool` crate with `coverage emit`

### Task B1: Scaffold `devtool` with the CLI skeleton

**Files:**
- Create: `devtool/Cargo.toml`
- Create: `devtool/src/main.rs`
- Create: `devtool/src/coverage/mod.rs`

**Interfaces:**
- Produces: a binary invoked as `devtool coverage emit`. Exit code is always `0` on a *completed* emit (success is reported via `status.json`, not the exit code); non-zero only on a tool-launch error that prevented producing any output.

- [ ] **Step 1: Write `devtool/Cargo.toml`**

```toml
[workspace]

[package]
name = "devtool"
version = "0.1.0"
edition = "2021"
publish = false

[dependencies]
clap = { version = "4", features = ["derive"] }
serde = { version = "1", features = ["derive"] }
serde_json = "1"
anyhow = "1"
coverage = { path = "../coverage" }
```

- [ ] **Step 2: Write `devtool/src/main.rs`**

```rust
//! Internal in-sandbox dev tool. Runs inside the Nix coverage/e2e build
//! sandboxes where `xtask` (host-only) is unavailable. Subcommand tree is
//! deliberately extensible: `coverage emit` exists today; `pg`/`seed-e2e` are
//! planned migrations of the remaining shell scripts (tracked separately).

use clap::{Parser, Subcommand};

mod coverage;

#[derive(Parser)]
#[command(name = "devtool", about = "Jaunder in-sandbox dev tooling")]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Coverage pipeline subcommands.
    #[command(subcommand)]
    Coverage(CoverageCmd),
}

#[derive(Subcommand)]
enum CoverageCmd {
    /// Run the instrumented suite and emit reports + status + diagnostics.
    Emit {
        /// Directory to write emitted artifacts into (defaults to CWD).
        #[arg(long, default_value = ".")]
        out: String,
    },
}

fn main() -> anyhow::Result<()> {
    let cli = Cli::parse();
    match cli.command {
        Command::Coverage(CoverageCmd::Emit { out }) => coverage::emit::run(&out),
    }
}
```

- [ ] **Step 3: Write `devtool/src/coverage/mod.rs`**

```rust
pub mod emit;
```

- [ ] **Step 4: Stub `devtool/src/coverage/emit.rs` so it compiles**

```rust
pub fn run(_out: &str) -> anyhow::Result<()> {
    anyhow::bail!("not yet implemented")
}
```

- [ ] **Step 5: Verify it builds and the CLI parses**

Run: `cargo run --manifest-path devtool/Cargo.toml -- coverage emit --help`
Expected: clap help for `emit` prints; exit 0.

- [ ] **Step 6: Lint + format gate**

Run: `cargo fmt --manifest-path devtool/Cargo.toml --check`
Run: `cargo clippy --manifest-path devtool/Cargo.toml --all-targets -- -D warnings`
Expected: clean.

- [ ] **Step 7: Commit**

```bash
git add devtool/Cargo.toml devtool/src/main.rs devtool/src/coverage/mod.rs devtool/src/coverage/emit.rs
git commit -m "feat(devtool): scaffold in-sandbox dev tool with coverage emit CLI"
```

### Task B2: Implement the in-sandbox failure classifier (pure, tested)

The classifier turns captured `cargo llvm-cov nextest` output into a `CoverageStatus`. It is pure (string → status) so it is unit-testable without running tests. Detection rules (from the #28 evidence):
- **infra** if the output contains `No space left on device`, SQLSTATE `53100`, or `Cannot allocate memory` / `out of memory`.
- **test-failure** if nextest emitted `FAIL [` lines (collect the test names) — but infra wins if both appear (a disk-full run also produces FAILs).
- **tests-ok** otherwise.

**Files:**
- Modify: `devtool/src/coverage/emit.rs`

**Interfaces:**
- Produces: `pub fn classify_nextest_output(output: &str) -> coverage::status::CoverageStatus`.

- [ ] **Step 1: Write the failing tests in `devtool/src/coverage/emit.rs`**

```rust
use coverage::status::{CoverageStatus, StatusCategory};

/// Classify captured `cargo llvm-cov nextest` output into the in-sandbox
/// sentinel. Infra failures (disk/OOM) take precedence over test failures,
/// because a disk-full run ALSO produces spurious test FAILs (#28).
pub fn classify_nextest_output(output: &str) -> CoverageStatus {
    const INFRA_MARKERS: &[&str] = &[
        "No space left on device",
        "53100",
        "Cannot allocate memory",
        "out of memory",
    ];
    if let Some(marker) = INFRA_MARKERS.iter().find(|m| output.contains(**m)) {
        return CoverageStatus {
            category: StatusCategory::Infra,
            failed_tests: Vec::new(),
            infra_detail: Some((*marker).to_string()),
        };
    }
    let failed_tests: Vec<String> = output
        .lines()
        .filter_map(|l| {
            let l = l.trim_start();
            // nextest summary line: "FAIL [   0.71s] <suite> <test path>"
            let rest = l.strip_prefix("FAIL [")?;
            let after_bracket = rest.split(']').nth(1)?.trim();
            after_bracket.split_whitespace().last().map(str::to_string)
        })
        .collect();
    if failed_tests.is_empty() {
        CoverageStatus { category: StatusCategory::TestsOk, failed_tests, infra_detail: None }
    } else {
        CoverageStatus { category: StatusCategory::TestFailure, failed_tests, infra_detail: None }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detects_disk_full_as_infra_even_with_fails() {
        let out = "\
FAIL [ 0.71s] jaunder::web web_posts::case_3
could not extend file \"base/25350/2609_vm\": No space left on device
";
        let s = classify_nextest_output(out);
        assert_eq!(s.category, StatusCategory::Infra);
        assert_eq!(s.infra_detail.as_deref(), Some("No space left on device"));
    }

    #[test]
    fn collects_failed_test_names() {
        let out = "\
FAIL [ 0.71s] jaunder::web web_posts::endpoint_rejects_unauthenticated::case_3
FAIL [ 0.04s] jaunder::web web_posts::get_post_carries_tags::case_2_postgres
";
        let s = classify_nextest_output(out);
        assert_eq!(s.category, StatusCategory::TestFailure);
        assert_eq!(s.failed_tests.len(), 2);
        assert!(s.failed_tests[0].ends_with("case_3"));
    }

    #[test]
    fn clean_output_is_tests_ok() {
        let out = "Summary [ 34s] 1531/1531 tests run: 1531 passed";
        assert_eq!(classify_nextest_output(out).category, StatusCategory::TestsOk);
    }
}
```

(Remove the `run` stub's body collision: keep the `run` fn from Task B1; add the above above it. `run` still `bail!`s for now.)

- [ ] **Step 2: Run tests to verify they pass**

Run: `cargo test --manifest-path devtool/Cargo.toml`
Expected: PASS (3 tests).

- [ ] **Step 3: Lint + format gate**

Run: `cargo fmt --manifest-path devtool/Cargo.toml --check`
Run: `cargo clippy --manifest-path devtool/Cargo.toml --all-targets -- -D warnings`
Expected: clean.

- [ ] **Step 4: Commit**

```bash
git add devtool/src/coverage/emit.rs
git commit -m "feat(devtool): classify nextest output into the coverage sentinel"
```

### Task B3: Implement the emit orchestration

Port `scripts/check-coverage --emit` (check-coverage:99-141) to `run`. It always returns `Ok(())` after producing artifacts; failures are recorded in `status.json`, never via the exit code (so the producer derivation always succeeds — Phase C). It captures combined stdout+stderr of the instrumented run.

**Files:**
- Modify: `devtool/src/coverage/emit.rs`

**Interfaces:**
- Consumes: `coverage::pathnorm::normalize_report_text`, `classify_nextest_output`.
- Produces (in `out` dir): `coverage-report.txt`, `crap-report.json`, `status.json`, and a `diagnostics/` subdir (`nextest.log`, `disk-usage.txt`).
- Emits via `std::process::Command`: `cargo llvm-cov clean --profraw-only`; `bash scripts/with-ephemeral-postgres cargo llvm-cov --no-report nextest --show-progress none`; `cargo llvm-cov report --text`; `cargo llvm-cov report --lcov --output-path <lcov>`; `cargo crap --workspace --lcov <lcov> --exclude '**/tests/**' --format json --output <raw-crap>`.

- [ ] **Step 1: Replace the `run` stub with the orchestration**

```rust
use std::fs;
use std::path::Path;
use std::process::Command;

use anyhow::{Context, Result};

/// Run the instrumented suite and emit reports + status + diagnostics into `out`.
/// Always produces `out/status.json` (and best-effort the rest) so the caller's
/// Nix producer derivation can always realize `$out`. Returns `Err` only if the
/// emit could not run at all (e.g. failing to spawn cargo).
pub fn run(out: &str) -> Result<()> {
    let out = Path::new(out);
    let diag = out.join("diagnostics");
    fs::create_dir_all(&diag).with_context(|| format!("creating {}", diag.display()))?;

    let abs_root = std::env::current_dir()?.to_string_lossy().to_string();

    // 1. Clear stale profraw, keep the instrumented build.
    run_logged(Command::new("cargo").args(["llvm-cov", "clean", "--profraw-only"]))?;

    // 2. Instrumented suite under an ephemeral PostgreSQL. Capture combined
    //    output for classification + the diagnostics bundle. A non-zero exit is
    //    NOT fatal here: a test failure or infra failure is reported via status.
    let nextest = run_capture(Command::new("bash").args([
        "scripts/with-ephemeral-postgres",
        "cargo",
        "llvm-cov",
        "--no-report",
        "nextest",
        "--show-progress",
        "none",
    ]))?;
    fs::write(diag.join("nextest.log"), &nextest)?;
    let status = super::emit::classify_nextest_output(&nextest);
    fs::write(out.join("status.json"), status.to_json())?;

    // 3. Disk-usage snapshot for the diagnostics bundle (#28).
    let df = run_capture(Command::new("df").arg("-h"))?;
    fs::write(diag.join("disk-usage.txt"), df)?;

    // 4. Text + LCOV reports (best-effort; on infra failure they may be partial).
    let report = run_capture(Command::new("cargo").args(["llvm-cov", "report", "--text"]))?;
    let report = coverage::pathnorm::normalize_report_text(&report, &abs_root);
    fs::write(out.join("coverage-report.txt"), &report)?;

    let lcov = out.join("coverage-report.lcov");
    run_logged(Command::new("cargo").args([
        "llvm-cov",
        "report",
        "--lcov",
        "--output-path",
        lcov.to_str().unwrap(),
    ]))?;

    // 5. CRAP report, normalized to repo-relative file paths.
    let raw_crap = out.join("crap-report.raw.json");
    run_logged(Command::new("cargo").args([
        "crap",
        "--workspace",
        "--lcov",
        lcov.to_str().unwrap(),
        "--exclude",
        "**/tests/**",
        "--format",
        "json",
        "--output",
        raw_crap.to_str().unwrap(),
    ]))?;
    let crap = fs::read_to_string(&raw_crap)?;
    fs::write(out.join("crap-report.json"), normalize_crap_paths(&crap, &abs_root)?)?;

    Ok(())
}

/// Strip the absolute sandbox prefix from each CRAP entry's `.file` (ports the
/// `jq` rewrite in `normalize_crap_report`).
fn normalize_crap_paths(raw: &str, abs_root: &str) -> Result<String> {
    let prefix = format!("{abs_root}/");
    let mut v: serde_json::Value = serde_json::from_str(raw)?;
    if let Some(entries) = v.get_mut("entries").and_then(|e| e.as_array_mut()) {
        for e in entries {
            if let Some(f) = e.get("file").and_then(|f| f.as_str()) {
                let rel = f.strip_prefix(&prefix).unwrap_or(f).to_string();
                e["file"] = serde_json::Value::String(rel);
            }
        }
    }
    Ok(format!("{}\n", serde_json::to_string_pretty(&v)?))
}

/// Spawn, inheriting stdio, erroring if the process could not be launched.
fn run_logged(cmd: &mut Command) -> Result<()> {
    let status = cmd.status().with_context(|| format!("spawning {cmd:?}"))?;
    // A non-zero exit is tolerated (recorded elsewhere); a spawn failure is not.
    let _ = status;
    Ok(())
}

/// Spawn, capturing combined stdout+stderr as a String.
fn run_capture(cmd: &mut Command) -> Result<String> {
    let out = cmd.output().with_context(|| format!("spawning {cmd:?}"))?;
    let mut s = String::from_utf8_lossy(&out.stdout).into_owned();
    s.push_str(&String::from_utf8_lossy(&out.stderr));
    Ok(s)
}
```

Add `pub` to `classify_nextest_output` if not already, and ensure the `tests` module from B2 stays.

- [ ] **Step 2: Add a `normalize_crap_paths` unit test**

```rust
    #[test]
    fn normalize_crap_paths_strips_prefix() {
        let raw = r#"{"entries":[{"file":"/build/source/server/src/a.rs","crap":1.0}]}"#;
        let got = super::normalize_crap_paths(raw, "/build/source").unwrap();
        assert!(got.contains("\"server/src/a.rs\""));
        assert!(!got.contains("/build/source"));
    }
```

- [ ] **Step 3: Run tests + lint**

Run: `cargo test --manifest-path devtool/Cargo.toml`
Run: `cargo clippy --manifest-path devtool/Cargo.toml --all-targets -- -D warnings`
Run: `cargo fmt --manifest-path devtool/Cargo.toml --check`
Expected: all pass/clean. (The orchestration itself is exercised end-to-end by the Nix build in Phase C — it cannot run on the host without the sandbox PG toolchain.)

- [ ] **Step 4: Commit**

```bash
git add devtool/src/coverage/emit.rs
git commit -m "feat(devtool): implement coverage emit orchestration"
```

---

## Phase C — Wire the Nix producer/consumer; delete the shell

### Task C1: Make the `coverage` producer build+run `devtool` and always emit

**Files:**
- Modify: `flake.nix` (the `coverage` check, ~895-942)

**Interfaces:**
- Produces: `$out/{coverage-report.txt,crap-report.json,status.json,diagnostics/}` on EVERY run (the build always succeeds).

- [ ] **Step 1: Replace the buildPhase + installPhase of the `coverage` derivation**

Change `buildPhaseCargoCommand` from `bash ./scripts/check-coverage --emit` to build and run `devtool`, writing directly into the build dir, and drop `gawk`/`jq` from `nativeBuildInputs` (keep `cargo-crap`, `cargo-llvm-cov`, `cargo-nextest`, `postgresql_16`):

```nix
    buildPhaseCargoCommand = ''
      export LD_LIBRARY_PATH="${pkgs.lib.makeLibraryPath [ pkgs.openssl ]}:''${LD_LIBRARY_PATH:-}"
      mkdir -p emit-out
      # devtool always exits 0 after producing emit-out/status.json; gating is
      # done by the `coverage-gate` consumer derivation and host xtask.
      cargo run --manifest-path devtool/Cargo.toml -- coverage emit --out emit-out
    '';
    installPhaseCommand = ''
      mkdir -p $out
      cp emit-out/coverage-report.txt $out/coverage-report.txt
      cp emit-out/crap-report.json $out/crap-report.json
      cp emit-out/status.json $out/status.json
      cp -r emit-out/diagnostics $out/diagnostics
    '';
```

(Leave the `src` filter unchanged — it already includes `devtool/` and `coverage/` and excludes `/xtask/`.)

- [ ] **Step 2: Build the producer and verify it always succeeds and emits status.json**

Run: `nix build --accept-flake-config --out-link .xtask/gcroots/coverage .#checks.x86_64-linux.coverage`
Run: `cat .xtask/gcroots/coverage/status.json`
Expected: build succeeds; `status.json` shows `"category": "tests-ok"` on a green tree; `coverage-report.txt`, `crap-report.json`, and `diagnostics/` present.

- [ ] **Step 3: Commit**

```bash
git add flake.nix
git commit -m "feat(flake): coverage producer runs devtool and always emits status"
```

### Task C2: Add the `coverage-gate` consumer derivation

**Files:**
- Modify: `flake.nix` (add a check next to `coverage`)

**Interfaces:**
- Produces: `checks.<system>.coverage-gate` — fails iff `status.json.category != "tests-ok"`. Its name keeps the `jaunder-coverage` substring so the existing `pushFilter` keeps it out of cachix (always re-evaluated).

- [ ] **Step 1: Add the consumer derivation**

```nix
            # Belt-and-suspenders: an independent Nix-level red for in-sandbox
            # failures (test/infra) even if a caller bypasses host xtask. The
            # coverage-regression verdict is host-only (needs committed baselines
            # + git) and lives in xtask, not here. Named `jaunder-coverage-gate`
            # so the cachix pushFilter (jaunder-coverage|jaunder-e2e) excludes it.
            coverage-gate =
              pkgs.runCommand "jaunder-coverage-gate"
                {
                  nativeBuildInputs = [ pkgs.jq ];
                }
                ''
                  cat ${self.checks.${system}.coverage}/status.json
                  cat=$(jq -r .category ${self.checks.${system}.coverage}/status.json)
                  if [ "$cat" != "tests-ok" ]; then
                    echo "coverage gate failed: category=$cat" >&2
                    jq -r '.infra_detail // (.failed_tests | join("\n"))' \
                      ${self.checks.${system}.coverage}/status.json >&2
                    exit 1
                  fi
                  touch $out
                '';
```

- [ ] **Step 2: Verify the consumer passes on a green tree**

Run: `nix build --accept-flake-config .#checks.x86_64-linux.coverage-gate`
Expected: success (category is `tests-ok`).

- [ ] **Step 3: Commit**

```bash
git add flake.nix
git commit -m "feat(flake): add coverage-gate consumer derivation on the sentinel"
```

### Task C3: Delete `scripts/check-coverage` and the vestigial `coverage-update`

The gap baseline is regenerated by `cargo xtask __regen-baseline` (reads the producer's `coverage-report.txt`); the CRAP manifest is healed host-side by `cargo xtask check`. The percent-model `coverage-update` derivation (flake.nix:784-816) is dead.

**Files:**
- Delete: `scripts/check-coverage`
- Modify: `flake.nix` (remove `coverage-update`)

- [ ] **Step 1: Remove the `coverage-update` derivation block** (flake.nix:774-816, including the comment header). Re-baselining is now documented as: `nix build .#checks.x86_64-linux.coverage --out-link .xtask/gcroots/coverage` then `cargo xtask __regen-baseline`.

- [ ] **Step 2: Delete the script**

```bash
git rm scripts/check-coverage
```

- [ ] **Step 3: Verify nothing else references the deleted names**

Run: `cargo run --manifest-path devtool/Cargo.toml -- coverage emit --help` (sanity: devtool builds)
Run: `nix flake check --accept-flake-config --no-build 2>&1 | tail -20` (the flake still evaluates with `coverage-update` gone)
Expected: no reference errors; flake evaluates.

- [ ] **Step 4: Commit**

```bash
git add -A
git commit -m "refactor: delete scripts/check-coverage and vestigial coverage-update"
```

---

## Phase D — Host gate consumes the sentinel + infra backstop

### Task D1: `xtask` depends on the `coverage` crate

**Files:**
- Modify: `xtask/Cargo.toml`

- [ ] **Step 1: Add the dependency**

```toml
coverage = { path = "../coverage" }
```

- [ ] **Step 2: Verify it builds**

Run: `cargo build --manifest-path xtask/Cargo.toml`
Expected: success.

- [ ] **Step 3: Commit**

```bash
git add xtask/Cargo.toml xtask/Cargo.lock
git commit -m "build(xtask): depend on the shared coverage crate"
```

### Task D2: Read the sentinel and surface the failure category

`xtask/src/steps/nix.rs::coverage` currently treats any non-zero `nix build` of the `coverage` check as an opaque build failure. With the producer always succeeding, build the `coverage-gate` check (which fails on a bad sentinel), and when it fails, read `status.json` from the producer's gcroot to report the precise category.

**Files:**
- Modify: `xtask/src/steps/nix.rs`

**Interfaces:**
- Consumes: `coverage::status::{CoverageStatus, StatusCategory}` (read from `.xtask/gcroots/coverage/status.json`).
- Produces: a `StepResult` whose detail names the category (`infra` / `test-failure`) and evidence.

- [ ] **Step 1: Write the failing test** (a pure helper that maps a `CoverageStatus` to a `StepResult` detail string)

Add to `xtask/src/steps/nix.rs`:

```rust
/// Render the in-sandbox sentinel into a human StepResult detail. Pure + tested;
/// the I/O (reading status.json, running nix build) stays in `coverage()`.
fn sentinel_detail(status: &coverage::status::CoverageStatus) -> String {
    use coverage::status::StatusCategory::*;
    match status.category {
        TestsOk => "in-sandbox: tests ok".to_string(),
        Infra => format!(
            "infrastructure failure (not a coverage regression): {}",
            status.infra_detail.as_deref().unwrap_or("unknown")
        ),
        TestFailure => format!(
            "test failure(s) (not a coverage regression): {}",
            status.failed_tests.join(", ")
        ),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use coverage::status::{CoverageStatus, StatusCategory};

    #[test]
    fn infra_detail_is_labeled_as_infrastructure() {
        let s = CoverageStatus {
            category: StatusCategory::Infra,
            failed_tests: vec![],
            infra_detail: Some("No space left on device".into()),
        };
        let d = sentinel_detail(&s);
        assert!(d.contains("infrastructure failure"));
        assert!(d.contains("No space left on device"));
    }

    #[test]
    fn test_failure_lists_tests_and_disclaims_coverage() {
        let s = CoverageStatus {
            category: StatusCategory::TestFailure,
            failed_tests: vec!["web_posts::case_3".into()],
            infra_detail: None,
        };
        let d = sentinel_detail(&s);
        assert!(d.contains("test failure"));
        assert!(d.contains("web_posts::case_3"));
    }
}
```

- [ ] **Step 2: Run the test to verify it fails (function not yet present), then passes after adding it**

Run: `cargo test --manifest-path xtask/Cargo.toml steps::nix`
Expected: PASS after Step 1.

- [ ] **Step 3: Wire `coverage()` to build the gate and read the sentinel on failure**

Replace the body of `pub fn coverage(result: &mut CommandResult, mode: Mode)` so it:
1. builds the producer (`build_check("nix-coverage", "coverage")`) — always ok now;
2. builds the consumer (`build_check("nix-coverage-gate", "coverage-gate")`);
3. if the gate failed, reads `.xtask/gcroots/coverage/status.json` and pushes `StepResult::fail("coverage").detail(sentinel_detail(&status))`, then returns (skip host post-processing — the in-sandbox failure is authoritative);
4. otherwise runs the host gate `coverage::run(...)` exactly as today.

```rust
pub fn coverage(result: &mut CommandResult, mode: Mode) {
    let build = build_check("nix-coverage", "coverage");
    result.push(build);
    let gate = build_check("nix-coverage-gate", "coverage-gate");
    if !gate.ok {
        let status_path = ".xtask/gcroots/coverage/status.json";
        let detail = std::fs::read_to_string(status_path)
            .ok()
            .and_then(|s| coverage::status::CoverageStatus::from_json(&s).ok())
            .map(|s| sentinel_detail(&s))
            .unwrap_or_else(|| "coverage gate failed (no status.json)".to_string());
        result.push(StepResult::fail("coverage").detail(detail));
        return;
    }
    result.push(gate);
    let (step, report) = coverage::run(".xtask/gcroots/coverage", mode);
    result.push(step);
    result.coverage = report;
}
```

Note `build_check("nix-coverage-gate", "coverage-gate")` must pass `--out-link .xtask/gcroots/coverage-gate`; it already derives the out-link from the check name. Ensure the producer is built with `--out-link .xtask/gcroots/coverage` (already the case) so `status.json` is reachable.

- [ ] **Step 4: Run the xtask test suite + lint**

Run: `cargo test --manifest-path xtask/Cargo.toml`
Run: `cargo clippy --manifest-path xtask/Cargo.toml --all-targets -- -D warnings`
Run: `cargo fmt --manifest-path xtask/Cargo.toml --check`
Expected: pass/clean.

- [ ] **Step 5: Commit**

```bash
git add xtask/src/steps/nix.rs
git commit -m "feat(xtask): surface in-sandbox coverage failure category from sentinel"
```

### Task D3: `--keep-failed` backstop + diagnostics copy-out

For catastrophic infra failures where the producer cannot even write `status.json`, retain the failed build dir and copy its diagnostics to a host path.

**Files:**
- Modify: `xtask/src/steps/nix.rs` (the `build_check` helper)

**Interfaces:**
- Produces: on a failed `nix build`, copies `diagnostics/` (if present in the kept build dir) to `.xtask/diagnostics/<check>/`.

- [ ] **Step 1: Add `--keep-failed` to the `nix build` invocation in `build_check`**

In the `Command::new("nix").args([...])` array, add `"--keep-failed"` after `"build"`. (This makes Nix retain `/tmp/nix-build-*.drv-*` on failure; the build log already streams to the CI console.)

- [ ] **Step 2: On failure, best-effort copy diagnostics to a host path**

After detecting `Ok(s)` non-success in `build_check`, before returning the `fail` StepResult, attempt to locate the most recent `/tmp/nix-build-jaunder-<check>*` dir and copy any `emit-out/diagnostics` into `.xtask/diagnostics/<check>/`. Keep it best-effort (ignore errors):

```rust
fn rescue_diagnostics(check: &str) {
    let dest = format!(".xtask/diagnostics/{check}");
    let _ = std::fs::create_dir_all(&dest);
    // Best-effort: copy from any kept failed build dir. Implemented via a shell
    // glob since the exact drv-suffixed dir name is not known ahead of time.
    let _ = std::process::Command::new("bash")
        .arg("-c")
        .arg(format!(
            "cp -r /tmp/nix-build-jaunder-{check}*/emit-out/diagnostics/* {dest}/ 2>/dev/null || true"
        ))
        .status();
}
```

Call `rescue_diagnostics(check)` in the non-success branch of `build_check`.

- [ ] **Step 3: Lint + format gate**

Run: `cargo clippy --manifest-path xtask/Cargo.toml --all-targets -- -D warnings`
Run: `cargo fmt --manifest-path xtask/Cargo.toml --check`
Expected: clean. (Behavior is exercised by a real infra failure; not host-unit-testable.)

- [ ] **Step 4: Commit**

```bash
git add xtask/src/steps/nix.rs
git commit -m "feat(xtask): keep-failed backstop and diagnostics rescue on nix build failure"
```

### Task D4: Lint the new crates from `xtask` static checks

So `cargo xtask validate` (and `check`) also cover `coverage/` and `devtool/`.

**Files:**
- Modify: `xtask/src/steps/static_checks.rs`

- [ ] **Step 1: Append fmt + clippy steps for each standalone crate**

For each of `coverage` and `devtool`, add a `step(...)` running (Check mode) `cargo fmt --manifest-path <crate>/Cargo.toml --check` and `cargo clippy --manifest-path <crate>/Cargo.toml --all-targets -- -D warnings`; (Fix mode) `cargo fmt --manifest-path <crate>/Cargo.toml`. Mirror the existing `step(sh, ...)` pattern in the file.

- [ ] **Step 2: Verify**

Run: `cargo run --manifest-path xtask/Cargo.toml -- check --no-test`
Expected: the new fmt/clippy steps appear and pass.

- [ ] **Step 3: Commit**

```bash
git add xtask/src/steps/static_checks.rs
git commit -m "build(xtask): lint coverage and devtool crates in static checks"
```

---

## Phase E — CI artifact exfiltration

### Task E1: Upload diagnostics as a CI artifact on every run

**Files:**
- Modify: `.github/workflows/ci.yml`

- [ ] **Step 1: Add an upload step after the Validate step**

```yaml
      - name: Upload coverage/e2e diagnostics
        if: always()
        uses: actions/upload-artifact@v4
        with:
          name: validate-diagnostics
          path: |
            .xtask/diagnostics/
            .xtask/gcroots/coverage/status.json
            .xtask/gcroots/coverage/diagnostics/
            .xtask/last-result.json
          if-no-files-found: ignore
          retention-days: 14
```

- [ ] **Step 2: Validate the workflow YAML parses**

Run: `python3 -c "import yaml,sys; yaml.safe_load(open('.github/workflows/ci.yml'))"`
Expected: no error.

- [ ] **Step 3: Commit**

```bash
git add .github/workflows/ci.yml
git commit -m "ci: upload coverage/e2e diagnostics as an artifact on every run"
```

---

## Phase F — Line-map reference frame (#3 + #11)

The host gate currently builds its line map from `git diff --unified=0 HEAD --` (HEAD→working-tree). But the Nix report is built from committed HEAD, and the baseline was healed at a past commit. The correct frame is **baseline-anchor-commit → HEAD**, never the working tree. This dissolves #3 (uncommitted edits stop manufacturing phantom regressions — they're not in the report) and #11 (committed line shifts since the baseline are now spanned).

### Task F1: Compute the baseline anchor commit

**Files:**
- Modify: `xtask/src/coverage/mod.rs`

**Interfaces:**
- Produces: `fn baseline_anchor_commit() -> Result<String>` — `git log -1 --format=%H -- coverage-baseline.json`; on empty output (uncommitted baseline) returns `"HEAD"` (identity map).

- [ ] **Step 1: Add the function**

```rust
/// The commit the committed baseline was last healed at. The Nix report is built
/// from committed HEAD, so the correct line map is anchor..HEAD — NOT a
/// working-tree diff (which manufactured phantom regressions, #3) and NOT just
/// HEAD's own diff (which ignored shifts committed since the baseline, #11).
fn baseline_anchor_commit() -> Result<String> {
    let out = Command::new("git")
        .args(["log", "-1", "--format=%H", "--", BASELINE_PATH])
        .output()
        .context("running git log for baseline anchor")?;
    let sha = String::from_utf8_lossy(&out.stdout).trim().to_string();
    Ok(if sha.is_empty() { "HEAD".to_string() } else { sha })
}
```

- [ ] **Step 2: Build to verify it compiles**

Run: `cargo build --manifest-path xtask/Cargo.toml`
Expected: success.

- [ ] **Step 3: Commit**

```bash
git add xtask/src/coverage/mod.rs
git commit -m "feat(xtask): compute the baseline anchor commit for line mapping"
```

### Task F2: Diff anchor→HEAD instead of HEAD→working-tree

**Files:**
- Modify: `xtask/src/coverage/mod.rs`

- [ ] **Step 1: Write a failing test asserting the diff command spans anchor..HEAD**

Refactor `git_diff_unified0()` to take an explicit range and add a test of the argument construction via a small pure helper:

```rust
/// The `git diff` argv for mapping the baseline anchor to the report's HEAD
/// frame. Pinned prefixes so repo/CI diff config can't change the `+++ b/`
/// prefix the parser keys on.
fn diff_args(anchor: &str) -> Vec<String> {
    vec![
        "diff".into(),
        "--unified=0".into(),
        "--no-color".into(),
        "--src-prefix=a/".into(),
        "--dst-prefix=b/".into(),
        format!("{anchor}..HEAD"),
        "--".into(),
    ]
}

#[cfg(test)]
mod anchor_tests {
    use super::*;
    #[test]
    fn diff_args_span_anchor_to_head_not_worktree() {
        let args = diff_args("abc123");
        assert!(args.contains(&"abc123..HEAD".to_string()));
        assert!(!args.contains(&"HEAD".to_string())); // not a bare-HEAD worktree diff
    }
}
```

- [ ] **Step 2: Run the test to verify it fails, then implement**

Run: `cargo test --manifest-path xtask/Cargo.toml anchor_tests`
Expected: FAIL (function missing) → add `diff_args`, then PASS.

- [ ] **Step 3: Rewrite `git_diff_unified0` to use the anchor range**

```rust
fn git_diff_anchor_to_head(anchor: &str) -> Result<String> {
    let out = Command::new("git")
        .args(diff_args(anchor))
        .output()
        .context("running git diff anchor..HEAD")?;
    Ok(String::from_utf8_lossy(&out.stdout).to_string())
}
```

In `run_inner`, replace:
```rust
let diff = git_diff_unified0()?;
```
with:
```rust
let anchor = baseline_anchor_commit()?;
let diff = git_diff_anchor_to_head(&anchor)?;
```

Replace the untracked-file mapping that uses `git ls-files --others` (working-tree-only) with files **added between anchor and HEAD**:
```rust
for path in git_added_files_since(&anchor)? {
```
where:
```rust
/// Files added (status A) between the anchor and HEAD — the committed analogue
/// of the old untracked-file handling. Their lines have no anchor preimage, so
/// uncovered lines classify as new_uncovered rather than regression.
fn git_added_files_since(anchor: &str) -> Result<Vec<String>> {
    let out = Command::new("git")
        .args(["diff", "--name-only", "--diff-filter=A", &format!("{anchor}..HEAD")])
        .output()
        .context("running git diff --diff-filter=A")?;
    Ok(String::from_utf8_lossy(&out.stdout)
        .lines()
        .map(str::trim)
        .filter(|l| !l.is_empty())
        .map(String::from)
        .collect())
}
```
Delete the now-unused `git_diff_unified0` and `git_untracked_files`.

- [ ] **Step 4: Run the full xtask test suite + lint**

Run: `cargo test --manifest-path xtask/Cargo.toml`
Run: `cargo clippy --manifest-path xtask/Cargo.toml --all-targets -- -D warnings`
Expected: pass/clean. (The existing diffmap unit tests in `diffmap.rs` are unaffected — they test the parser, not the range.)

- [ ] **Step 5: Commit**

```bash
git add xtask/src/coverage/mod.rs
git commit -m "fix(xtask): map baseline anchor->HEAD, not working tree (#3, #11)"
```

---

## Phase G — Confirm #7 (CRAP-heal JSON idempotence)

The Rust heal already writes pretty, key-sorted JSON compared via a normalized form (`mod.rs:193-203`). Lock it with a regression test so the compact-JSON drift cannot return, then close #7.

### Task G1: Heal-idempotence regression test

**Files:**
- Modify: `xtask/src/coverage/mod.rs` (tests)

- [ ] **Step 1: Add the test**

```rust
    #[test]
    fn crap_heal_is_idempotent_and_pretty() {
        // A compact CRAP report normalizes to the same value as its pretty form,
        // so a second heal would not rewrite (no spurious diff churn, #7).
        let compact = r#"{"entries":[{"crate":"c","file":"a.rs","function":"f","line":1,"crap":2.0}]}"#;
        let pretty = pretty_json(compact).unwrap();
        assert!(pretty.contains("\n"), "heal must write multi-line pretty JSON");
        assert_eq!(
            normalize_json(&pretty).unwrap(),
            normalize_json(compact).unwrap(),
            "pretty and compact must normalize equal, so re-heal is a no-op"
        );
    }
```

- [ ] **Step 2: Run the test**

Run: `cargo test --manifest-path xtask/Cargo.toml crap_heal_is_idempotent`
Expected: PASS.

- [ ] **Step 3: Commit**

```bash
git add xtask/src/coverage/mod.rs
git commit -m "test(xtask): lock CRAP-heal pretty-JSON idempotence (#7)"
```

---

## Phase H — Full-system verification

### Task H1: End-to-end validate in the Nix sandbox

- [ ] **Step 1: Run the full host gate**

Run: `nix develop .#ci --accept-flake-config -c cargo xtask validate --no-e2e`
Expected: static checks + coverage producer/consumer + host gate all green; `.xtask/last-result.json` shows `ok: true`.

- [ ] **Step 2: Simulate a test-failure surfaces correctly (manual spot check)**

Temporarily break one test (e.g. add `assert!(false)` to a small unit test), run:
Run: `nix build --accept-flake-config .#checks.x86_64-linux.coverage-gate 2>&1 | tail -20`
Expected: gate fails with `category=test-failure` and the test name; the producer (`.#checks.x86_64-linux.coverage`) still **succeeds** and emits `status.json`. Revert the break afterward.

- [ ] **Step 3: Confirm re-baseline flow works**

Run: `nix build --accept-flake-config --out-link .xtask/gcroots/coverage .#checks.x86_64-linux.coverage`
Run: `cargo run --manifest-path xtask/Cargo.toml -- __regen-baseline`
Expected: `coverage-baseline.json` regenerates from the producer report without error.

- [ ] **Step 4: Final commit (only if any fixups were needed)**

```bash
git add -A
git commit -m "chore: coverage migration end-to-end verification fixups"
```

---

## Self-Review

**Spec coverage:**
- Crate architecture → Phases A, B, D1 (refined: minimal shared lib, gate stays in xtask — see header note). ✓
- Emit pipeline → B3, C1. ✓
- Producer/consumer/host-gate layering → C1, C2, D2. ✓
- Failure attribution (#2 + #28 infra axis) → B2, C2, D2. ✓
- Line-map reference frame (#3 + #11) → F1, F2. ✓
- CRAP-heal #7 → G1. ✓
- Diagnostics preservation & exfiltration → B3 (bundle), D3 (keep-failed), E1 (artifact). ✓
- Nix wiring & deletions (drop gawk/jq, delete check-coverage, delete percent path) → C1, C3. ✓
- Single coverage model → C3 (delete coverage-update). ✓
- Follow-up issues for other scripts → deferred per user (opened after the plan). Not a task here. ✓
- Testing → unit tests throughout; H1 end-to-end. ✓

**Placeholder scan:** No TBD/TODO; every code step shows code; every verify step shows a command + expected result. ✓

**Type consistency:** `CoverageStatus`/`StatusCategory` (kebab-case `tests-ok|test-failure|infra`) consistent across A1, B2, C2, D2. `coverage emit --out` consistent across B1, B3, C1. `coverage-baseline.json`/`crap-manifest.json` paths consistent with the existing constants. ✓

## Notes / risks (carried from the spec)
- **In-sandbox build cost:** `devtool` + `coverage` compile inside the producer derivation; their deps are small (clap/serde/anyhow) and the producer is never cached/pushed, so cost is bounded and acceptable.
- **Reference-frame correctness (F):** sequenced last and isolated; relies on the invariant that the Nix report == committed HEAD. If that invariant ever changes (dirty-tree flake builds), revisit F2.
- **Refinement from spec §2:** the shared lib is intentionally minimal and the host gate stays in `xtask` (better cache behavior); this is the one deliberate divergence from the written spec and is flagged for the reviewer.
