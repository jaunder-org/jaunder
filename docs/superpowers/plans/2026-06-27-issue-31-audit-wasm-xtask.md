# `cargo xtask audit-wasm` Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Replace the Node.js `scripts/audit-wasm-bundle` with a self-documenting `cargo xtask audit-wasm` host subcommand that reports raw/gzip/brotli frontend bundle sizes.

**Architecture:** Per [ADR-0026](../../adr/0026-devtool-vs-xtask-boundary.md), this host-side analysis tool lives in `xtask` (not `devtool`). A new `xtask/src/audit_wasm.rs` module holds the report types, pure size/format helpers (unit-tested), and the `nix build`/fs I/O. The report rides xtask's existing `CommandResult` envelope (mirroring the `coverage` payload) and reuses the global `--json` flag.

**Tech Stack:** Rust, `clap` (derive), `flate2` (gzip), `brotli`, `serde`/`serde_json`.

## Global Constraints

- **Placement:** host-side `xtask`, NOT `devtool` (ADR-0026 litmus: it runs `nix build`).
- **Behavior-preserving:** same artifacts (`pkg/jaunder_bg.wasm`, `pkg/jaunder.js`), same raw/gzip/brotli columns, same `--site-path` override.
- **Compression parity:** gzip at level 9 (`Compression::best()` = Node `Z_BEST_COMPRESSION`); brotli quality 11, window 22 (= the script's `BROTLI_PARAM_QUALITY: 11`, default window).
- **Self-documenting:** `cargo xtask audit-wasm --help` must state the problem it solves, when to use it, and how (examples).
- **Not a gate:** never wired into `check`/`validate`; standalone manual tool.
- **Per-step iteration gate:** `cargo xtask check --no-test` (clippy + fmt + host xtask unit suite). **Commit gate:** `cargo xtask validate --no-e2e` green (xtask is excluded from the coverage Nix source, so its coverage check is a cache hit — cheap to re-run).
- **No `Co-Authored-By` trailers.**

---

## Note on separable concerns

Investigation surfaced **no new issues to file**. The ADR-0026 pre-classification of #32/#33 targets issues that already exist (reconfirmed in their own cycles), and the `docs/README.md` ADR-table restoration (0023–0025) was approved to fold into this branch. The only administrative follow-up is **editing #31's own text** to reflect the xtask placement — handled at ship, not as a new issue.

---

## Task 1: Planning artifacts commit

**Files:**
- Already written (spec phase): `docs/superpowers/specs/2026-06-27-issue-31-audit-wasm-xtask.md`, `docs/adr/0026-devtool-vs-xtask-boundary.md`, `docs/README.md` (ADR table), `docs/superpowers/plans/2026-06-27-issue-31-audit-wasm-xtask.md` (this file).

- [x] **Step 1: Verify the working tree holds only the planning docs**

Run: `git status --porcelain`
Expected: only the four files above are new/modified, nothing else.

- [x] **Step 2: Commit the planning artifacts**

```bash
git add docs/superpowers/specs/2026-06-27-issue-31-audit-wasm-xtask.md \
        docs/adr/0026-devtool-vs-xtask-boundary.md \
        docs/README.md \
        docs/superpowers/plans/2026-06-27-issue-31-audit-wasm-xtask.md
git commit -m "docs(issue-31): spec + ADR-0026 devtool/xtask boundary + plan"
```

(No code gate needed — docs only. The ADR/README belong on the branch from the start so the boundary rationale is committed before the code that depends on it.)

---

## Task 2: Report types + pure helpers (`audit_wasm.rs`)

**Files:**
- Modify: `xtask/Cargo.toml` (add `flate2`, `brotli`)
- Create: `xtask/src/audit_wasm.rs`
- Modify: `xtask/src/lib.rs` (add `mod audit_wasm;` near the other `mod` lines, ~line 3)

**Interfaces:**
- Produces (consumed by Tasks 3–4):
  - `pub struct AuditReport { pub site_path: String, pub artifacts: Vec<ArtifactMetrics> }` (derives `Serialize`)
  - `pub struct ArtifactMetrics { pub path: String, pub raw_bytes: u64, pub gzip_bytes: u64, pub brotli_bytes: u64 }` (derives `Serialize`)
  - `pub fn format_bytes(bytes: u64) -> String`
  - `pub fn parse_store_path(nix_output: &str) -> Option<String>`
  - `pub fn gzip_size(bytes: &[u8]) -> u64`
  - `pub fn brotli_size(bytes: &[u8]) -> u64`
  - `pub fn render_table(report: &AuditReport) -> String`

- [x] **Step 1: Add the compression dependencies**

In `xtask/Cargo.toml`, under `[dependencies]`, add:

```toml
flate2 = "1"
brotli = "7"
```

- [x] **Step 2: Register the module**

In `xtask/src/lib.rs`, add alongside the existing `mod coverage;` / `mod result;` lines:

```rust
mod audit_wasm;
```

- [x] **Step 3: Write the failing tests**

Create `xtask/src/audit_wasm.rs` with the types, `use`s, and a `#[cfg(test)]` block (leave the helper bodies as `todo!()` for now so it compiles-and-fails):

```rust
use serde::Serialize;

#[derive(Serialize)]
pub struct AuditReport {
    pub site_path: String,
    pub artifacts: Vec<ArtifactMetrics>,
}

#[derive(Serialize)]
pub struct ArtifactMetrics {
    pub path: String,
    pub raw_bytes: u64,
    pub gzip_bytes: u64,
    pub brotli_bytes: u64,
}

pub fn format_bytes(_bytes: u64) -> String {
    todo!()
}

pub fn parse_store_path(_nix_output: &str) -> Option<String> {
    todo!()
}

pub fn gzip_size(_bytes: &[u8]) -> u64 {
    todo!()
}

pub fn brotli_size(_bytes: &[u8]) -> u64 {
    todo!()
}

pub fn render_table(_report: &AuditReport) -> String {
    todo!()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn format_bytes_matches_script_rounding() {
        assert_eq!(format_bytes(0), "0 B");
        assert_eq!(format_bytes(512), "512 B");
        assert_eq!(format_bytes(1024), "1.0 KiB");
        assert_eq!(format_bytes(1536), "1.5 KiB");
        // >= 10 in the unit → 0 decimals
        assert_eq!(format_bytes(10 * 1024), "10 KiB");
        assert_eq!(format_bytes(1024 * 1024), "1.0 MiB");
        assert_eq!(format_bytes(1024_u64.pow(3)), "1.0 GiB");
    }

    #[test]
    fn parse_store_path_takes_last_store_line() {
        let out = "warning: ignoring\n/nix/store/aaa-x\n  /nix/store/bbb-jaunder-site  \n";
        assert_eq!(
            parse_store_path(out).as_deref(),
            Some("/nix/store/bbb-jaunder-site")
        );
    }

    #[test]
    fn parse_store_path_none_when_no_store_line() {
        assert_eq!(parse_store_path("no paths here\n"), None);
    }

    #[test]
    fn compression_shrinks_repetitive_input_and_is_deterministic() {
        let bytes = vec![b'a'; 10_000];
        let g = gzip_size(&bytes);
        let b = brotli_size(&bytes);
        assert!(g < bytes.len() as u64, "gzip should shrink: {g}");
        assert!(b < bytes.len() as u64, "brotli should shrink: {b}");
        assert_eq!(g, gzip_size(&bytes), "gzip deterministic");
        assert_eq!(b, brotli_size(&bytes), "brotli deterministic");
    }

    #[test]
    fn render_table_has_header_site_path_and_relative_names() {
        let report = AuditReport {
            site_path: "/nix/store/x-jaunder-site".into(),
            artifacts: vec![ArtifactMetrics {
                path: "/nix/store/x-jaunder-site/pkg/jaunder_bg.wasm".into(),
                raw_bytes: 2 * 1024 * 1024,
                gzip_bytes: 700 * 1024,
                brotli_bytes: 600 * 1024,
            }],
        };
        let t = render_table(&report);
        assert!(t.contains("WASM bundle audit"));
        assert!(t.contains("site output: /nix/store/x-jaunder-site"));
        assert!(t.contains("artifact"));
        // relative path, not the absolute one
        assert!(t.contains("pkg/jaunder_bg.wasm"));
        assert!(!t.contains("/nix/store/x-jaunder-site/pkg/jaunder_bg.wasm"));
    }
}
```

- [x] **Step 4: Run the tests to verify they fail**

Run: `cargo test -p xtask audit_wasm`
Expected: FAIL (panics from `todo!()`).

- [x] **Step 5: Implement the pure helpers**

Replace the `todo!()` bodies in `xtask/src/audit_wasm.rs`:

```rust
use std::io::Write;
use std::path::Path;

pub fn format_bytes(bytes: u64) -> String {
    const UNITS: [&str; 4] = ["B", "KiB", "MiB", "GiB"];
    let mut value = bytes as f64;
    let mut unit = 0usize;
    while value >= 1024.0 && unit < UNITS.len() - 1 {
        value /= 1024.0;
        unit += 1;
    }
    let decimals = if value >= 10.0 || unit == 0 { 0 } else { 1 };
    format!("{value:.decimals$} {}", UNITS[unit])
}

pub fn parse_store_path(nix_output: &str) -> Option<String> {
    nix_output
        .lines()
        .map(str::trim)
        .filter(|l| l.starts_with("/nix/store/"))
        .last()
        .map(str::to_string)
}

pub fn gzip_size(bytes: &[u8]) -> u64 {
    use flate2::{write::GzEncoder, Compression};
    let mut enc = GzEncoder::new(Vec::new(), Compression::best());
    enc.write_all(bytes).expect("gzip write to Vec is infallible");
    enc.finish().expect("gzip finish to Vec is infallible").len() as u64
}

pub fn brotli_size(bytes: &[u8]) -> u64 {
    let mut out = Vec::new();
    {
        // buffer 4096, quality 11, window 22 — matches the Node script's
        // BROTLI_PARAM_QUALITY: 11 with the default window.
        let mut w = brotli::CompressorWriter::new(&mut out, 4096, 11, 22);
        w.write_all(bytes).expect("brotli write to Vec is infallible");
    }
    out.len() as u64
}

pub fn render_table(report: &AuditReport) -> String {
    let mut s = String::new();
    s.push_str("WASM bundle audit\n");
    s.push_str(&format!("site output: {}\n", report.site_path));
    s.push('\n');
    s.push_str("artifact          raw        gzip       brotli\n");
    for row in &report.artifacts {
        let name = Path::new(&row.path)
            .strip_prefix(&report.site_path)
            .map(|p| p.to_string_lossy().into_owned())
            .unwrap_or_else(|_| row.path.clone());
        s.push_str(&format!(
            "{:<16}  {:>9}  {:>9}  {:>9}\n",
            name,
            format_bytes(row.raw_bytes),
            format_bytes(row.gzip_bytes),
            format_bytes(row.brotli_bytes),
        ));
    }
    s
}
```

- [x] **Step 6: Run the tests to verify they pass**

Run: `cargo test -p xtask audit_wasm`
Expected: PASS (5 tests).

- [x] **Step 7: Run the iteration gate, then the commit gate**

Run: `cargo xtask check --no-test`
Expected: static + clippy + host xtask unit suite pass.

Run: `cargo xtask validate --no-e2e`
Expected: PASSED.

- [x] **Step 8: Commit**

```bash
git add xtask/Cargo.toml xtask/Cargo.lock xtask/src/audit_wasm.rs xtask/src/lib.rs
git commit -m "feat(xtask): audit-wasm report types + pure size/format helpers"
```

---

## Task 3: I/O — resolve site path and build the report

**Files:**
- Modify: `xtask/src/audit_wasm.rs`

**Interfaces:**
- Consumes: `parse_store_path`, `gzip_size`, `brotli_size`, `AuditReport`, `ArtifactMetrics` (Task 2).
- Produces (consumed by Task 4):
  - `pub fn resolve_site_path(explicit: Option<&str>) -> anyhow::Result<String>`
  - `pub fn run(site_path: Option<&str>) -> anyhow::Result<AuditReport>`

- [x] **Step 1: Write the failing test (artifact-missing error path — no `nix` needed)**

Add to the `#[cfg(test)] mod tests` in `xtask/src/audit_wasm.rs`:

```rust
#[test]
fn run_errors_when_artifact_missing() {
    // An explicit empty temp dir has no pkg/jaunder_bg.wasm → run() must Err.
    let dir = std::env::temp_dir().join(format!("audit-wasm-test-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let res = run(Some(dir.to_str().unwrap()));
    std::fs::remove_dir_all(&dir).ok();
    let err = res.unwrap_err().to_string();
    assert!(err.contains("jaunder_bg.wasm"), "error names the missing artifact: {err}");
}
```

- [x] **Step 2: Run it to verify it fails**

Run: `cargo test -p xtask audit_wasm::tests::run_errors_when_artifact_missing`
Expected: FAIL ("cannot find function `run`" — not yet implemented).

- [x] **Step 3: Implement the I/O**

Add to `xtask/src/audit_wasm.rs` (top: `use anyhow::{bail, Context, Result};` and `use std::process::Command;`):

```rust
/// Resolve the `.#site` output to audit. Returns `explicit` verbatim when set;
/// otherwise runs the deterministic `nix build .#site` and parses its store path.
pub fn resolve_site_path(explicit: Option<&str>) -> Result<String> {
    if let Some(p) = explicit {
        return Ok(p.to_string());
    }
    let out = Command::new("nix")
        .args(["build", ".#site", "--no-link", "--print-out-paths"])
        .output()
        .context("spawning `nix build .#site`")?;
    if !out.status.success() {
        bail!(
            "`nix build .#site` failed ({}):\n{}",
            out.status,
            String::from_utf8_lossy(&out.stderr)
        );
    }
    let stdout = String::from_utf8_lossy(&out.stdout);
    parse_store_path(&stdout)
        .context("could not parse a /nix/store path from `nix build .#site` output")
}

/// Resolve the site path, then measure the two frontend artifacts.
pub fn run(site_path: Option<&str>) -> Result<AuditReport> {
    let site_path = resolve_site_path(site_path)?;
    let names = ["pkg/jaunder_bg.wasm", "pkg/jaunder.js"];
    let mut artifacts = Vec::new();
    for name in names {
        let path = Path::new(&site_path).join(name);
        let bytes = std::fs::read(&path)
            .with_context(|| format!("reading artifact {}", path.display()))?;
        artifacts.push(ArtifactMetrics {
            path: path.to_string_lossy().into_owned(),
            raw_bytes: bytes.len() as u64,
            gzip_bytes: gzip_size(&bytes),
            brotli_bytes: brotli_size(&bytes),
        });
    }
    Ok(AuditReport { site_path, artifacts })
}
```

- [x] **Step 4: Run the test to verify it passes**

Run: `cargo test -p xtask audit_wasm`
Expected: PASS (6 tests).

- [x] **Step 5: Run the gates**

Run: `cargo xtask check --no-test`
Expected: pass.

Run: `cargo xtask validate --no-e2e`
Expected: PASSED.

- [x] **Step 6: Commit**

```bash
git add xtask/src/audit_wasm.rs
git commit -m "feat(xtask): audit-wasm site-path resolution + report builder"
```

---

## Task 4: Wire the subcommand + envelope + self-documenting help

**Files:**
- Modify: `xtask/src/result.rs` (add `audit` field + `print_human` rendering)
- Modify: `xtask/src/lib.rs` (add `Command::AuditWasm`, `command_name` arm, `run` arm)

**Interfaces:**
- Consumes: `audit_wasm::run`, `AuditReport`, `render_table` (Tasks 2–3).

- [x] **Step 1: Write the failing test (envelope carries + renders the audit report)**

Add to the `#[cfg(test)] mod tests` in `xtask/src/result.rs`:

```rust
#[test]
fn audit_report_serializes_and_renders_in_envelope() {
    let mut r = CommandResult::new("audit-wasm");
    r.push(StepResult::ok("audit-wasm").detail("2 artifact(s)"));
    r.audit = Some(crate::audit_wasm::AuditReport {
        site_path: "/nix/store/x-jaunder-site".into(),
        artifacts: vec![crate::audit_wasm::ArtifactMetrics {
            path: "/nix/store/x-jaunder-site/pkg/jaunder_bg.wasm".into(),
            raw_bytes: 2 * 1024 * 1024,
            gzip_bytes: 700 * 1024,
            brotli_bytes: 600 * 1024,
        }],
    });
    let v: serde_json::Value = serde_json::to_value(&r).unwrap();
    assert_eq!(v["audit"]["site_path"], "/nix/store/x-jaunder-site");
    assert_eq!(v["audit"]["artifacts"][0]["raw_bytes"], 2 * 1024 * 1024);
}
```

- [x] **Step 2: Run it to verify it fails**

Run: `cargo test -p xtask audit_report_serializes_and_renders_in_envelope`
Expected: FAIL (no field `audit` on `CommandResult`).

- [x] **Step 3: Add the `audit` payload to the envelope**

In `xtask/src/result.rs`, add the field to `CommandResult` (next to `coverage`):

```rust
    #[serde(skip_serializing_if = "Option::is_none")]
    pub audit: Option<crate::audit_wasm::AuditReport>,
```

In `CommandResult::new`, initialize it:

```rust
            audit: None,
```

In `print_human`, after the steps loop and before the verdict line, render the table when present:

```rust
        if let Some(audit) = &self.audit {
            print!("{}", crate::audit_wasm::render_table(audit));
        }
```

- [x] **Step 4: Run the test to verify it passes**

Run: `cargo test -p xtask audit_report_serializes_and_renders_in_envelope`
Expected: PASS.

- [x] **Step 5: Add the self-documenting subcommand**

In `xtask/src/lib.rs`, add a variant to `enum Command` (the doc comment IS the `--help` documentation — problem, when, and how):

```rust
    /// Measure the frontend WASM/JS bundle size — raw, gzip, and brotli.
    ///
    /// Reports the download weight of the deterministic `nix build .#site`
    /// output (`pkg/jaunder_bg.wasm`, `pkg/jaunder.js`) so you can catch
    /// bundle-size bloat before it ships and compare a change's effect on
    /// what users download. Run it after a change you expect to move the
    /// bundle (a new dependency, a feature touching the client), or
    /// periodically to watch the trend. This is a manual tool — it is not part
    /// of `check`/`validate`.
    #[command(after_help = "EXAMPLES:\n  \
        cargo xtask audit-wasm\n  \
        cargo xtask audit-wasm --site-path /nix/store/...-jaunder-site\n  \
        cargo xtask --json audit-wasm")]
    AuditWasm {
        /// Audit a prebuilt `.#site` store path instead of running `nix build`.
        #[arg(long)]
        site_path: Option<String>,
    },
```

In `command_name`, add the arm:

```rust
            Command::AuditWasm { .. } => "audit-wasm",
```

In `run`, add the arm (before the closing `}` of the `match cli.command`):

```rust
        Command::AuditWasm { site_path } => {
            let start = std::time::Instant::now();
            let mut result = CommandResult::new("audit-wasm");
            match audit_wasm::run(site_path.as_deref()) {
                Ok(report) => {
                    let n = report.artifacts.len();
                    result.audit = Some(report);
                    result.push(StepResult::ok("audit-wasm").detail(format!("{n} artifact(s)")));
                }
                Err(e) => {
                    result.push(StepResult::fail("audit-wasm").detail(format!("{e:#}")));
                }
            }
            finalize(&mut result, start);
            Ok(result)
        }
```

Add `StepResult` to the `use` in `lib.rs` if not already imported (it re-exports via `pub use result::{CommandResult, Mode, StepResult};` — already present).

- [x] **Step 6: Verify the help renders and the command runs**

Run: `cargo run -p xtask -- audit-wasm --help`
Expected: shows the problem/when description and the EXAMPLES block.

Run: `cargo run -p xtask -- audit-wasm --site-path /tmp/definitely-missing`
Expected: `[FAIL] audit-wasm — reading artifact /tmp/definitely-missing/pkg/jaunder_bg.wasm ...`, exit 1.

- [x] **Step 7: Run the gates**

Run: `cargo xtask check --no-test`
Expected: pass.

Run: `cargo xtask validate --no-e2e`
Expected: PASSED.

- [x] **Step 8: Commit**

```bash
git add xtask/src/result.rs xtask/src/lib.rs
git commit -m "feat(xtask): wire self-documenting audit-wasm subcommand + envelope payload"
```

---

## Task 5: Delete the script and update the docs

**Files:**
- Delete: `scripts/audit-wasm-bundle`
- Modify: `CONTRIBUTING.md:136-140`
- Modify: `docs/ARCHITECTURE.md:103`
- Modify: `docs/observability.md:111-116`

- [x] **Step 1: Delete the Node.js script**

```bash
git rm scripts/audit-wasm-bundle
```

- [x] **Step 2: Update `CONTRIBUTING.md`**

Replace the "WASM Audit" bullet (lines 136-140) with:

```markdown
- **WASM Audit**: Use `cargo xtask audit-wasm` to measure the size of the frontend WASM and JS bundles from the deterministic Nix build.
  ```bash
  cargo xtask audit-wasm
  ```
  Useful options: `--json` (global flag: `cargo xtask --json audit-wasm`) for machine-readable output, or `--site-path` to reuse a build. See `cargo xtask audit-wasm --help`.
```

- [x] **Step 3: Update `docs/ARCHITECTURE.md`**

Replace line 103 with:

```markdown
-   **`cargo xtask audit-wasm`**: Measures deterministic WASM bundle sizes (raw, gzip, brotli) from Nix build outputs.
```

- [x] **Step 4: Update `docs/observability.md`**

Replace the body of the "## WASM Bundle Audit" section (lines 111-116) so the prose and example use `cargo xtask audit-wasm`:

```markdown
Use `cargo xtask audit-wasm` to measure frontend bundle size from the
deterministic Nix `site` build output:

```bash
cargo xtask audit-wasm
```
```

- [x] **Step 5: Confirm no stale references remain**

Run: `git grep -n "scripts/audit-wasm-bundle"`
Expected: no output (all references updated; the archived design doc under `docs/archive/` is a frozen historical record and is intentionally left untouched — confirm any remaining hits are only under `docs/archive/`).

- [x] **Step 6: Run the gates**

Run: `cargo xtask check --no-test`
Expected: pass.

Run: `cargo xtask validate --no-e2e`
Expected: PASSED.

- [x] **Step 7: Commit**

```bash
git add scripts/audit-wasm-bundle CONTRIBUTING.md docs/ARCHITECTURE.md docs/observability.md
git commit -m "docs(issue-31): point WASM-audit docs at cargo xtask audit-wasm; remove script"
```

---

## Self-Review

- **Spec coverage:** subcommand + flags (Task 4) ✓; self-documenting help (Task 4 Step 5) ✓; module/types/pure helpers (Task 2) ✓; I/O resolve+run (Task 3) ✓; envelope `audit` payload + render (Task 4) ✓; deps (Task 2 Step 1) ✓; script deletion + 3 doc updates (Task 5) ✓; ADR + ADR-table + spec (Task 1) ✓; testing per spec ✓; not-a-gate (never added to check/validate) ✓.
- **Type consistency:** `AuditReport`/`ArtifactMetrics` field names (`site_path`, `artifacts`, `path`, `raw_bytes`, `gzip_bytes`, `brotli_bytes`) used identically across Tasks 2–4; `run(Option<&str>)`, `resolve_site_path(Option<&str>)`, `parse_store_path(&str)` signatures stable.
- **Placeholders:** none (the `todo!()` bodies in Task 2 Step 3 are deliberate red-test scaffolding, replaced in Step 5).
- **Behavior parity note:** gzip/brotli byte counts may differ trivially from Node's zlib (header bytes) — accepted non-goal; columns and semantics are identical.
