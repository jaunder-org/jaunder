# CRAP-manifest refresh path (#131) — Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Give a CRAP regression the same first-class recovery the coverage baseline got in #88 — a discoverable `cargo xtask coverage refresh-crap` command (candidate + refuse on a regression, refresh-in-place otherwise) and a gate failure that prints the exact command to run.

**Architecture:** Mirror `reanchor.rs`'s shape inside `crap.rs`: relocate the CRAP-manifest constants/helpers there, add a pure `plan_crap_refresh` + `refusal_report`, and keep a thin I/O wrapper (`refresh_crap` / `refresh_crap_inner`) plus a clap subcommand in `mod.rs`/`lib.rs`. The gate's `failure_report` CRAP branch prints `cargo xtask coverage refresh-crap`.

**Tech Stack:** Rust, `xtask` dev-driver crate, `clap` (derive), `anyhow`, `serde_json`, `cargo nextest`.

## Global Constraints

- **TDD:** every behavior change starts with a failing test (`cargo nextest run --manifest-path xtask/Cargo.toml <filter>`), then minimal code to green.
- **xtask is excluded from coverage instrumentation** (the Nix coverage-src denylist excludes `/xtask/`). This code is therefore **not** coverage-gated and changes **no coverage baseline**; its safety net is the xtask host unit suite. Do **not** touch `coverage-baseline.json` or `crap-manifest.json`.
- **No `Co-Authored-By` trailers** in any commit (jaunder override of the global default).
- **Per-task gate:** end each task with `cargo xtask check --no-test` (clippy + fmt) before committing. The git pre-commit hook independently runs `check --no-test` + `validate --no-e2e --allow-dirty`, so each commit is gate-verified.
- **Never commit on `main`.** Work stays on `worktree-issue-131-crap-refresh`.
- One clean, verified commit per task.
- **Mirror, don't reinvent:** the safety model is #88/ADR-0030's candidate-promotion (refuse + side-path + manual `cp`), never an accept-all flag.

---

### Task 1: Relocate CRAP-manifest constants/helpers into `crap.rs`

Behavior-preserving refactor so `crap.rs` owns all CRAP-manifest logic (the home for Task 2's pure planner). Also lands the planning docs as this branch's first commit.

**Files:**
- Modify: `xtask/src/coverage/crap.rs` (gain `CRAP_MANIFEST_PATH`, `normalize_without_line`, `pretty_manifest` + their moved tests)
- Modify: `xtask/src/coverage/mod.rs` (remove the three items; update references)
- Add (to first commit): `docs/superpowers/specs/2026-06-28-issue-131-crap-refresh-design.md`, `docs/superpowers/plans/2026-06-28-issue-131-crap-refresh.md`

**Interfaces:**
- Produces: `crap::CRAP_MANIFEST_PATH: &str`, `crap::normalize_without_line(&str) -> Result<String>`, `crap::pretty_manifest(&str) -> Result<String>`.
- Consumes: nothing new.

- [ ] **Step 1: Add the three items to `crap.rs`**

At the top of `crap.rs`, below the existing `const EPSILON` (line 16), add:

```rust
/// The committed CRAP baseline. An ordinary (non-dotted) tracked file.
pub const CRAP_MANIFEST_PATH: &str = "crap-manifest.json";
```

Above the `#[cfg(test)]` module (after `compare`, ~line 114), add the two helpers (moved verbatim from `mod.rs`, renamed):

```rust
/// Canonical, line- and order-independent form of a CRAP report: each entry
/// minus its `line`, with key-sorted JSON (serde_json `Value` is a `BTreeMap`),
/// and the entry set itself sorted. Two reports that differ only in line
/// attribution (a pure shift) normalize equal, so a refresh does not rewrite
/// `crap-manifest.json` unless some non-`line` field changed — the `crap` score
/// or its `coverage`/`cyclomatic` inputs, or the set of functions (#7). The
/// `line` field is retained in the written manifest as a non-authoritative
/// jump-to hint that refreshes wholesale on the next such change.
pub fn normalize_without_line(s: &str) -> Result<String> {
    let v: serde_json::Value = serde_json::from_str(s)?;
    let mut rows: Vec<String> = v
        .get("entries")
        .and_then(|e| e.as_array())
        .map(|arr| {
            arr.iter()
                .map(|e| {
                    let mut e = e.clone();
                    if let Some(o) = e.as_object_mut() {
                        o.remove("line");
                    }
                    e.to_string()
                })
                .collect()
        })
        .unwrap_or_default();
    rows.sort();
    Ok(rows.join("\n"))
}

/// Canonical (key-sorted, via `serde_json::Value`'s `BTreeMap`) but
/// pretty-printed with a trailing newline — the on-disk form of the committed
/// manifest, so coverage diffs stay readable.
pub fn pretty_manifest(s: &str) -> Result<String> {
    let v: serde_json::Value = serde_json::from_str(s)?;
    Ok(format!("{}\n", serde_json::to_string_pretty(&v)?))
}
```

Move the three tests `crap_normalize_ignores_line_and_formatting`, `crap_normalize_detects_a_score_change`, `crap_pretty_json_is_multiline` from `mod.rs` (lines 704–733) into `crap.rs`'s `#[cfg(test)] mod tests`, replacing `normalize_crap_without_line` → `normalize_without_line` and `pretty_json` → `pretty_manifest` in their bodies. (They reference only `super::*`, already in scope.)

- [ ] **Step 2: Remove the originals from `mod.rs` and update references**

In `mod.rs`: delete `const CRAP_MANIFEST_PATH` (line 89), delete `fn normalize_crap_without_line` (362–381) and `fn pretty_json` (386–389), and delete the three moved tests (704–733). Update the live references in `run_inner`:

- line 235 `std::fs::read_to_string(CRAP_MANIFEST_PATH)` → `crap::CRAP_MANIFEST_PATH`
- lines 256–257 `normalize_crap_without_line(...)` → `crap::normalize_without_line(...)`
- lines 259–260 `std::fs::write(CRAP_MANIFEST_PATH, pretty_json(&crap_report_str)?)` → `std::fs::write(crap::CRAP_MANIFEST_PATH, crap::pretty_manifest(&crap_report_str)?)` and the `format!("writing {CRAP_MANIFEST_PATH}")` → `format!("writing {}", crap::CRAP_MANIFEST_PATH)`

- [ ] **Step 3: Verify the refactor changed no behavior**

Run: `cargo nextest run --manifest-path xtask/Cargo.toml coverage`
Expected: PASS — every existing `coverage::mod` and `coverage::crap` test stays green (relocation only).

- [ ] **Step 4: Lint/format gate**

Run: `cargo xtask check --no-test`
Expected: `ok` (no clippy warnings, formatting clean).

- [ ] **Step 5: Commit**

```bash
git add xtask/src/coverage/crap.rs xtask/src/coverage/mod.rs \
  docs/superpowers/specs/2026-06-28-issue-131-crap-refresh-design.md \
  docs/superpowers/plans/2026-06-28-issue-131-crap-refresh.md
git commit -m "refactor(xtask): move CRAP-manifest helpers into crap.rs (#131)

crap.rs becomes the owner of CRAP-manifest constants/canonicalization
(CRAP_MANIFEST_PATH, normalize_without_line, pretty_manifest), so the
upcoming refresh planner can be pure and self-contained. No behavior change."
```

---

### Task 2: Pure `plan_crap_refresh` + `refusal_report` in `crap.rs`

The decision and the operator-facing refusal text — pure, no I/O — paralleling `reanchor::{plan_reanchor, refusal_report, CANDIDATE_PATH}`.

**Files:**
- Modify: `xtask/src/coverage/crap.rs`

**Interfaces:**
- Consumes: `compare` (Task 0/existing), `normalize_without_line`, `pretty_manifest`, `CRAP_MANIFEST_PATH` (Task 1).
- Produces:
  - `crap::CRAP_CANDIDATE_PATH: &str` = `".xtask/crap-manifest.candidate.json"`
  - `enum crap::CrapRefreshPlan { Refresh { manifest: Option<String> }, Refuse { candidate: String, regressions: Vec<CrapRegression> } }` (derives `Debug, PartialEq`)
  - `crap::plan_crap_refresh(fresh_report: &str, old_manifest: &str) -> Result<CrapRefreshPlan>`
  - `crap::refusal_report(regressions: &[CrapRegression]) -> String`

- [ ] **Step 1: Write the failing tests**

In `crap.rs`'s `#[cfg(test)] mod tests`, add (reuses the existing `OLD`/`NEW_WORSE` consts at lines 120–123):

```rust
#[test]
fn refresh_writes_when_crap_relevant_field_changed() {
    // No regression key match (different function name) but a real CRAP-relevant
    // change → Refresh carrying the pretty manifest to write.
    let old = r#"{"entries":[{"crate":"c","file":"a.rs","function":"f","line":1,"crap":2.0}]}"#;
    let fresh = r#"{"entries":[{"crate":"c","file":"a.rs","function":"g","line":1,"crap":2.0}]}"#;
    match plan_crap_refresh(fresh, old).unwrap() {
        CrapRefreshPlan::Refresh { manifest: Some(m) } => assert!(m.contains("\"g\"")),
        other => panic!("expected Refresh(Some), got {other:?}"),
    }
}

#[test]
fn refresh_is_noop_on_pure_line_shift() {
    // Same scores, only line attribution differs → already current (no write).
    let old = r#"{"entries":[{"crate":"c","file":"a.rs","function":"f","line":1,"crap":2.0}]}"#;
    let fresh = r#"{"entries":[{"crate":"c","file":"a.rs","function":"f","line":99,"crap":2.0}]}"#;
    assert_eq!(
        plan_crap_refresh(fresh, old).unwrap(),
        CrapRefreshPlan::Refresh { manifest: None }
    );
}

#[test]
fn refresh_refuses_and_carries_candidate_on_regression() {
    match plan_crap_refresh(NEW_WORSE, OLD).unwrap() {
        CrapRefreshPlan::Refuse { candidate, regressions } => {
            assert_eq!(regressions.len(), 1);
            assert_eq!(regressions[0].function, "f");
            assert!(candidate.contains("\"crap\""), "candidate is the pretty fresh report");
        }
        other => panic!("expected Refuse, got {other:?}"),
    }
}

#[test]
fn first_run_empty_manifest_writes_initial() {
    // Empty committed manifest (first run) → no regressions, write the fresh one.
    match plan_crap_refresh(OLD, "").unwrap() {
        CrapRefreshPlan::Refresh { manifest: Some(_) } => {}
        other => panic!("expected Refresh(Some), got {other:?}"),
    }
}

#[test]
fn refusal_report_lists_functions_and_promotion_recipe() {
    let report = refusal_report(&[CrapRegression {
        file: "b.rs".into(),
        function: "f".into(),
        old: 9.0,
        new: 11.0,
    }]);
    assert!(report.contains("b.rs::f  9.00 → 11.00"), "{report}");
    assert!(report.contains(CRAP_CANDIDATE_PATH));
    assert!(report.contains("git diff --no-index"));
    assert!(report.contains("cp "));
    assert!(report.contains(CRAP_MANIFEST_PATH));
}
```

- [ ] **Step 2: Run the tests to verify they fail**

Run: `cargo nextest run --manifest-path xtask/Cargo.toml coverage::crap`
Expected: FAIL — `cannot find ... plan_crap_refresh / CrapRefreshPlan / refusal_report / CRAP_CANDIDATE_PATH`.

- [ ] **Step 3: Implement the planner and report**

In `crap.rs`, above the `#[cfg(test)]` module, add:

```rust
/// Where a refused refresh writes its candidate manifest. Under the gitignored
/// `/.xtask/`, so it never dirties the tree or gets instrumented (mirrors the
/// baseline candidate in `reanchor`).
pub const CRAP_CANDIDATE_PATH: &str = ".xtask/crap-manifest.candidate.json";

/// The action a `coverage refresh-crap` run should take, decided purely from the
/// fresh report vs. the committed manifest. No I/O — the caller writes the bytes
/// and sets the exit status.
#[derive(Debug, PartialEq)]
pub enum CrapRefreshPlan {
    /// No regressions. `manifest` is the pretty bytes to write to the committed
    /// manifest, or `None` when there is no CRAP-relevant drift (already current).
    Refresh { manifest: Option<String> },
    /// A regression would raise the bar: write `candidate` to the side path and
    /// refuse (non-zero). Promotion stays a manual `cp`.
    Refuse {
        candidate: String,
        regressions: Vec<CrapRegression>,
    },
}

/// Decide the refresh action. With no regressions, refresh in place only when a
/// CRAP-relevant field actually changed (a pure line-shift / no change is a
/// no-op, mirroring the Fix-mode heal's churn-avoidance). With regressions, the
/// fresh report becomes a candidate and the run refuses.
pub fn plan_crap_refresh(fresh_report: &str, old_manifest: &str) -> Result<CrapRefreshPlan> {
    let regressions = if old_manifest.trim().is_empty() {
        Vec::new()
    } else {
        compare(fresh_report, old_manifest)?
    };
    if regressions.is_empty() {
        let new_canon = normalize_without_line(fresh_report)?;
        let old_canon = normalize_without_line(old_manifest).unwrap_or_default();
        let manifest = if new_canon != old_canon {
            Some(pretty_manifest(fresh_report)?)
        } else {
            None
        };
        Ok(CrapRefreshPlan::Refresh { manifest })
    } else {
        Ok(CrapRefreshPlan::Refuse {
            candidate: pretty_manifest(fresh_report)?,
            regressions,
        })
    }
}

/// Operator-facing message for a refused refresh: the offending `file::fn old → new`
/// plus how to inspect and (only if genuinely approved) promote the candidate.
/// There is deliberately no flag that promotes automatically — approval is a
/// visible diff (mirrors `reanchor::refusal_report`).
pub fn refusal_report(regressions: &[CrapRegression]) -> String {
    use std::fmt::Write as _;
    const MAX: usize = 25;
    let mut s = format!(
        "refused: {} CRAP regression(s) would raise the complexity-risk bar:",
        regressions.len()
    );
    for r in regressions.iter().take(MAX) {
        let _ = write!(s, "\n    {}::{}  {:.2} → {:.2}", r.file, r.function, r.old, r.new);
    }
    if regressions.len() > MAX {
        let _ = write!(s, "\n    … and {} more", regressions.len() - MAX);
    }
    let _ = write!(
        s,
        "\n  wrote candidate to {CRAP_CANDIDATE_PATH} (NOT the committed manifest).\
         \n  inspect:  git diff --no-index {CRAP_MANIFEST_PATH} {CRAP_CANDIDATE_PATH}\
         \n  if genuinely approved (stale drift, not a real regression), promote:\
         \n    cp {CRAP_CANDIDATE_PATH} {CRAP_MANIFEST_PATH} && git add {CRAP_MANIFEST_PATH}\
         \n  otherwise reduce complexity or improve coverage — never promote a real regression."
    );
    s
}
```

- [ ] **Step 4: Run the tests to verify they pass**

Run: `cargo nextest run --manifest-path xtask/Cargo.toml coverage::crap`
Expected: PASS — the five new tests plus all existing `crap` tests.

- [ ] **Step 5: Lint/format gate**

Run: `cargo xtask check --no-test`
Expected: `ok`.

- [ ] **Step 6: Commit**

```bash
git add xtask/src/coverage/crap.rs
git commit -m "feat(xtask): pure CRAP-manifest refresh planner (#131)

plan_crap_refresh mirrors reanchor's candidate-promotion model: no
regressions refresh in place (no-op on a pure line-shift), a regression
refuses to .xtask/crap-manifest.candidate.json. No accept-all path."
```

---

### Task 3: `refresh_crap` I/O wrapper + clap subcommand

The thin I/O orchestration (mirror of `reanchor` / `reanchor_inner`) and the CLI wiring.

**Files:**
- Modify: `xtask/src/coverage/mod.rs` (add `refresh_crap` / `refresh_crap_inner`)
- Modify: `xtask/src/lib.rs` (`CoverageCommand::RefreshCrap`, `command_name`, dispatch, parse tests)

**Interfaces:**
- Consumes: `crap::{plan_crap_refresh, CrapRefreshPlan, refusal_report, CRAP_MANIFEST_PATH, CRAP_CANDIDATE_PATH}` (Tasks 1–2); `StepResult`; `anyhow::Context` (already imported in `mod.rs`).
- Produces: `coverage::refresh_crap(out_dir: &str) -> StepResult`; clap `CoverageCommand::RefreshCrap { gcroot: String }`.

- [ ] **Step 1: Write the failing CLI parse tests**

In `lib.rs`'s `#[cfg(test)] mod cli_tests`, add:

```rust
#[test]
fn coverage_refresh_crap_parses_with_default_gcroot() {
    let cli = Cli::try_parse_from(["xtask", "coverage", "refresh-crap"]).unwrap();
    match cli.command {
        Command::Coverage(CoverageCommand::RefreshCrap { gcroot }) => {
            assert_eq!(gcroot, ".xtask/gcroots/coverage");
        }
        _ => panic!("expected coverage refresh-crap"),
    }
}

#[test]
fn coverage_refresh_crap_accepts_gcroot() {
    let cli =
        Cli::try_parse_from(["xtask", "coverage", "refresh-crap", "--gcroot", "/tmp/x"]).unwrap();
    match cli.command {
        Command::Coverage(CoverageCommand::RefreshCrap { gcroot }) => assert_eq!(gcroot, "/tmp/x"),
        _ => panic!("expected coverage refresh-crap"),
    }
}
```

- [ ] **Step 2: Run the tests to verify they fail**

Run: `cargo nextest run --manifest-path xtask/Cargo.toml coverage_refresh_crap`
Expected: FAIL — `no variant ... RefreshCrap`.

- [ ] **Step 3: Add the `RefreshCrap` clap variant**

In `lib.rs`'s `enum CoverageCommand` (after the `Reanchor` variant, ~line 83), add:

```rust
    /// Refresh `crap-manifest.json` from the current CRAP report. With no
    /// regressions it rewrites the committed manifest in place (a no-op when no
    /// CRAP-relevant field changed); on a regression it refuses and writes a
    /// candidate to `.xtask/crap-manifest.candidate.json` for a deliberate `cp`.
    /// Consumes an existing report (run `check`/`validate` first); never rebuilds.
    #[command(after_help = "EXAMPLES:\n  \
        cargo xtask coverage refresh-crap\n  \
        cargo xtask coverage refresh-crap --gcroot .xtask/gcroots/coverage")]
    RefreshCrap {
        /// GC-root / out-link directory holding `crap-report.json`.
        #[arg(long, default_value = ".xtask/gcroots/coverage")]
        gcroot: String,
    },
```

- [ ] **Step 4: Wire `command_name` and dispatch**

In `command_name` (after the `Reanchor` arm, line 92), add:

```rust
            Command::Coverage(CoverageCommand::RefreshCrap { .. }) => "coverage-refresh-crap",
```

In `run`'s match (after the `Reanchor` arm, lines 152–158), add:

```rust
        Command::Coverage(CoverageCommand::RefreshCrap { gcroot }) => {
            let start = std::time::Instant::now();
            let mut result = CommandResult::new("coverage-refresh-crap");
            result.push(coverage::refresh_crap(&gcroot));
            finalize(&mut result, start);
            Ok(result)
        }
```

- [ ] **Step 5: Implement `refresh_crap` in `mod.rs`**

In `mod.rs`, after `reanchor_inner` (line 184), add:

```rust
/// `cargo xtask coverage refresh-crap`: refresh `crap-manifest.json` from an existing
/// CRAP report. No regressions → rewrite the committed manifest in place (a no-op when
/// nothing CRAP-relevant changed). Regressions → write a candidate to the side path and
/// FAIL (non-zero), printing the offending functions and the promotion recipe. Consumes
/// an existing report — it does not rebuild coverage.
pub fn refresh_crap(out_dir: &str) -> StepResult {
    match refresh_crap_inner(out_dir) {
        Ok(step) => step,
        Err(e) => StepResult::fail("coverage-refresh-crap").detail(format!("{e:#}")),
    }
}

fn refresh_crap_inner(out_dir: &str) -> Result<StepResult> {
    let crap_path = format!("{out_dir}/crap-report.json");
    let fresh = std::fs::read_to_string(&crap_path).map_err(|_| {
        anyhow::anyhow!(
            "no CRAP report at {crap_path} — run `cargo xtask check` or \
             `cargo xtask validate` first to build one"
        )
    })?;
    let old_manifest = std::fs::read_to_string(crap::CRAP_MANIFEST_PATH).unwrap_or_default();
    match crap::plan_crap_refresh(&fresh, &old_manifest)? {
        crap::CrapRefreshPlan::Refresh {
            manifest: Some(bytes),
        } => {
            std::fs::write(crap::CRAP_MANIFEST_PATH, &bytes)
                .with_context(|| format!("writing {}", crap::CRAP_MANIFEST_PATH))?;
            Ok(StepResult::ok("coverage-refresh-crap")
                .detail(format!("refreshed {}", crap::CRAP_MANIFEST_PATH)))
        }
        crap::CrapRefreshPlan::Refresh { manifest: None } => Ok(StepResult::ok(
            "coverage-refresh-crap",
        )
        .detail(format!(
            "{} already current — no CRAP-relevant drift",
            crap::CRAP_MANIFEST_PATH
        ))),
        crap::CrapRefreshPlan::Refuse {
            candidate,
            regressions,
        } => {
            if let Some(parent) = std::path::Path::new(crap::CRAP_CANDIDATE_PATH).parent() {
                std::fs::create_dir_all(parent)
                    .with_context(|| format!("creating {}", parent.display()))?;
            }
            std::fs::write(crap::CRAP_CANDIDATE_PATH, &candidate)
                .with_context(|| format!("writing {}", crap::CRAP_CANDIDATE_PATH))?;
            Ok(StepResult::fail("coverage-refresh-crap").detail(crap::refusal_report(&regressions)))
        }
    }
}
```

- [ ] **Step 6: Run the CLI tests + smoke-check `--help`**

Run: `cargo nextest run --manifest-path xtask/Cargo.toml coverage_refresh_crap`
Expected: PASS.

Run: `cargo run --manifest-path xtask/Cargo.toml -- coverage refresh-crap --help`
Expected: prints the help with the EXAMPLES `after_help` (confirms it is discoverable).

- [ ] **Step 7: Lint/format gate**

Run: `cargo xtask check --no-test`
Expected: `ok`.

- [ ] **Step 8: Commit**

```bash
git add xtask/src/coverage/mod.rs xtask/src/lib.rs
git commit -m "feat(xtask): add \`coverage refresh-crap\` command (#131)

A discoverable recovery for CRAP drift: refreshes crap-manifest.json in
place when safe, refuses to a candidate on a regression. Mirrors
\`coverage reanchor\`; consumes an existing report (no rebuild)."
```

---

### Task 4: Gate failure prints the CRAP recovery command

`failure_report`'s CRAP branch changes from prose-only to the actionable command, mirroring the coverage-lowering branch's `run: cargo xtask coverage reanchor`.

**Files:**
- Modify: `xtask/src/coverage/mod.rs` (`failure_report` CRAP branch + its tests)

**Interfaces:**
- No signature change; `failure_report(&[reanchor::LineText], &[CrapRegression]) -> String` gains the command line in its CRAP branch.

- [ ] **Step 1: Update the failing tests**

In `mod.rs`'s test module, edit `failure_report_lists_lines_crap_and_recovery` (line 757) to also assert the command, replacing the `assert!(r.contains("CRAP: reduce"), ...)` line with:

```rust
        assert!(r.contains("CRAP: reduce"), "crap guidance: {r}");
        assert!(
            r.contains("cargo xtask coverage refresh-crap"),
            "crap recovery command: {r}"
        );
```

In `failure_report_guidance_is_category_conditional` (line 760), append — after the existing `assert!(r.contains("CRAP: reduce"), "{r}");` (line 778) — a lowering-only check confirming the two recovery commands stay category-split:

```rust
        // A lowering-only failure must NOT suggest refresh-crap (CRAP-only tool).
        let lowering_only = failure_report(
            &[reanchor::LineText {
                file: "a.rs".into(),
                line: 5,
                text: "x".into(),
            }],
            &[],
        );
        assert!(
            !lowering_only.contains("refresh-crap"),
            "refresh-crap is CRAP-only — not for a lowering: {lowering_only}"
        );
        assert!(lowering_only.contains("cargo xtask coverage reanchor"), "{lowering_only}");
```

- [ ] **Step 2: Run the tests to verify they fail**

Run: `cargo nextest run --manifest-path xtask/Cargo.toml failure_report`
Expected: FAIL — the `cargo xtask coverage refresh-crap` line is not emitted yet.

- [ ] **Step 3: Update the CRAP branch in `failure_report`**

In `mod.rs`, replace the CRAP guidance block (lines 344–349):

```rust
    if !crap_regs.is_empty() {
        s.push_str(
            "\n  → CRAP: reduce the function's complexity or improve its coverage;\
             \n    refresh crap-manifest.json (with approval) only if it is stale drift.",
        );
    }
```

with:

```rust
    if !crap_regs.is_empty() {
        s.push_str(
            "\n  → CRAP: reduce the function's complexity or improve its coverage; if this\
             \n    is approved drift (stale, not a real regression), refresh for review:\
             \n    run:  cargo xtask coverage refresh-crap",
        );
    }
```

(The string contains neither `cargo xtask check` nor `cargo xtask coverage reanchor`, so the existing category-conditional assertions at lines 773/775 stay satisfied.)

- [ ] **Step 4: Run the tests to verify they pass**

Run: `cargo nextest run --manifest-path xtask/Cargo.toml failure_report`
Expected: PASS — all four `failure_report` tests.

- [ ] **Step 5: Lint/format gate**

Run: `cargo xtask check --no-test`
Expected: `ok`.

- [ ] **Step 6: Commit**

```bash
git add xtask/src/coverage/mod.rs
git commit -m "feat(xtask): coverage gate prints the refresh-crap recovery command (#131)

A CRAP regression now ends with \`run: cargo xtask coverage refresh-crap\`
instead of prose; the lowering branch keeps \`reanchor\`. The two recovery
commands stay category-split."
```

---

### Task 5: Docs — CONTRIBUTING.md + ADR-0030 supplement

**Files:**
- Modify: `CONTRIBUTING.md` (coverage section)
- Modify: `docs/adr/0030-coverage-reanchor-text-identity.md` (append `## Supplement (#131)`)

**Interfaces:** none (docs only).

- [ ] **Step 1: Document `refresh-crap` in CONTRIBUTING.md**

In `CONTRIBUTING.md`, immediately after the `crap-manifest.json` `line`-field paragraph (line 192), insert a new paragraph:

```markdown
When a coverage gate fails on a **CRAP regression** (a function whose complexity-risk score worsened), run `cargo xtask coverage refresh-crap` (it consumes the same report under `.xtask/gcroots/coverage`). With no regression it refreshes `crap-manifest.json` in place — a no-op when nothing CRAP-relevant changed. With a regression it refuses, writes the would-be manifest to `.xtask/crap-manifest.candidate.json`, and prints each offending `file::fn old → new`. Accepting genuinely-stale drift is then a deliberate, reviewable step: inspect with `git diff --no-index crap-manifest.json .xtask/crap-manifest.candidate.json` and, if approved, `cp` the candidate over the committed manifest and commit it. As with the baseline, there is no flag that accepts a regression automatically — the symmetric recovery to `cargo xtask coverage reanchor` (#131).
```

- [ ] **Step 2: Append the ADR-0030 supplement**

At the end of `docs/adr/0030-coverage-reanchor-text-identity.md` (after line 98), append:

```markdown

## Supplement (#131) — the symmetric CRAP refresh path

The #88 supplement noted "the symmetric CRAP-manifest refresh path is tracked
separately (#131)". It lands as `cargo xtask coverage refresh-crap`, mirroring the
baseline `reanchor` model exactly: a no-regression refresh rewrites
`crap-manifest.json` in place (a no-op on a pure line-shift, keyed on the same
line-independent canonical form the Fix-mode heal uses); a CRAP **regression** is
refused (non-zero exit) with the would-be manifest written to
`.xtask/crap-manifest.candidate.json` — never the committed file. There is **no
accept-all path**; promoting approved drift is a manual `cp` of the candidate, so it
always lands as a reviewable diff. The failing coverage gate now prints
`cargo xtask coverage refresh-crap` as the CRAP recovery, the category-split companion
to the lowering branch's `cargo xtask coverage reanchor`.
```

- [ ] **Step 3: Verify the doc references are consistent**

Run: `rg -n 'refresh-crap' CONTRIBUTING.md docs/adr/0030-coverage-reanchor-text-identity.md`
Expected: both files mention `cargo xtask coverage refresh-crap`.

- [ ] **Step 4: Commit**

```bash
git add CONTRIBUTING.md docs/adr/0030-coverage-reanchor-text-identity.md
git commit -m "docs(#131): document refresh-crap + ADR-0030 supplement

Records the symmetric CRAP refresh path: same candidate-promotion model,
no accept-all, gate prints the recovery command."
```

---

## Final gate (before ship)

- [ ] Run the full pre-push-style gate: `cargo xtask validate --no-e2e`
  Expected: `ok` — static + clippy + xtask host tests + Nix coverage all green on the clean committed tree.
- [ ] Confirm the tree is clean (`git status --porcelain` empty) and the worktree is on `worktree-issue-131-crap-refresh`.

## Self-Review (plan vs. spec)

- **Spec coverage:** new `refresh-crap` command → Task 3; safe-writes-direct / candidate-on-regression semantics → Task 2 (`plan_crap_refresh`) + Task 3 (I/O); `crap.rs` owns CRAP-manifest logic + helper relocation → Task 1; gate prints the command → Task 4; CLI wiring → Task 3; tests (planner, refusal_report, failure_report, CLI parse) → Tasks 2–4; docs + ADR-0030 supplement → Task 5; no coverage-baseline impact (xtask excluded) → Global Constraints. All spec sections map to a task.
- **Type consistency:** `CRAP_MANIFEST_PATH`, `CRAP_CANDIDATE_PATH`, `normalize_without_line(&str)->Result<String>`, `pretty_manifest(&str)->Result<String>`, `CrapRefreshPlan::{Refresh{manifest:Option<String>}, Refuse{candidate:String, regressions:Vec<CrapRegression>}}`, `plan_crap_refresh(&str,&str)->Result<CrapRefreshPlan>`, `refusal_report(&[CrapRegression])->String`, `refresh_crap(&str)->StepResult`, `CoverageCommand::RefreshCrap{gcroot:String}` are used identically across Tasks 1–5.
- **Placeholder scan:** none — every code/step block is concrete.
