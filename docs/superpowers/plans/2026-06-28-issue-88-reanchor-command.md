# Surface + harden the baseline-reanchor command (#88) — Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Replace the hidden, accept-all `cargo xtask __regen-baseline` with a discoverable, safe-by-construction `cargo xtask coverage reanchor` that re-anchors benign line-shift drift, refuses genuine coverage lowering (writing a candidate to a side path), and have a failing coverage gate print the exact recovery command.

**Architecture:** Add a nested `coverage` clap subcommand group (first in xtask) with `reanchor`. Factor the gate's anchor-load + classify + safety block into a shared `classify_against_anchor` helper used by both the gate and the new command. The reanchor decision (`plan_reanchor`) is pure and lives next to `reanchor_is_safe` in `coverage/reanchor.rs`; the I/O orchestration lives in `coverage/mod.rs`. No accept-all path exists — accepting an approved lowering is a manual `cp` of the candidate.

**Tech Stack:** Rust, `clap` (derive, nested subcommands), `anyhow`, `cargo nextest`, `cargo xtask`.

## Global Constraints

- **Verify ladder (git-enforced — ADR-0029):** per-task iteration gate is `cargo xtask check --no-test` (fmt + clippy, fast). Each commit is gated automatically by the pre-commit hook (full `cargo xtask check`, fail-and-restage). `validate --no-e2e` is the hook-enforced pre-push gate; CI runs full `validate`. Run gates from the worktree (Bash tool, or `cd <worktree> &&`) — context-mode runs against the main repo.
- **xtask is its own workspace** (empty `[workspace]` in `xtask/Cargo.toml`) — test it with `cargo nextest run --manifest-path xtask/Cargo.toml ...`, not `-p xtask` from the root.
- **No Co-Authored-By trailers** in any commit.
- **Soundness:** the no-lowering check reuses the diff-based `reanchor::reanchor_is_safe` (ADR-0030); never a text-set-only comparison (#112 rejected it as unsound).
- **Candidate path** `.xtask/coverage-baseline.candidate.json` is under the gitignored `/.xtask/` (`.gitignore:39`) — it must never become a tracked/untracked-non-ignored file (which would dirty the tree and be instrumented).
- **Scope:** coverage baseline only. Do not touch `crap-manifest.json` or the line-identity classifier / `reanchor_is_safe` predicate.
- **No `#[allow(...)]`/`#[expect(...)]`** to pass clippy without explicit user approval.

---

### Task 1: File the CRAP-refresh follow-on issue

The CRAP-regression recovery path is prose-only (no command/mechanism) — the symmetric gap to the one this issue closes for the baseline. Capture it up front so it isn't blocked behind #88. **This task produces no commit** (a GitHub issue only).

**Files:** none.

- [x] **Step 1: Create the issue via `jaunder-issues`** — filed as #131 (Developer Experience project)

Use the `jaunder-issues` skill to open an issue in `jaunder-org/jaunder`. Content:

- **Title:** `coverage: surface + harden the CRAP-manifest refresh path (recovery command + candidate)`
- **Labels:** `coverage`, `dx`
- **Milestone:** `Verify-gate hardening` (milestone 1)
- **Body:**
  > Follow-on from #88. #88 gives a coverage-baseline *lowering* a first-class, safe recovery (`cargo xtask coverage reanchor` + a candidate side-path). The symmetric **CRAP-regression** path has no equivalent: the gate's `failure_report` (`xtask/src/coverage/mod.rs:294-298`) prints only prose — "reduce complexity / improve coverage; refresh `crap-manifest.json` (with approval) only if stale drift" — with **no command, no pointer to the fresh `crap-report.json`, and no candidate-promotion analog**. The manual refresh is also fiddly: `check` Fix-mode only regenerates the manifest when there are **no** CRAP regressions (`mod.rs:202-212`), so an approved regression must be hand-overwritten from the fresh report and re-run.
  >
  > **Change:** give CRAP regressions a recovery as clear as the baseline's — e.g. a `cargo xtask coverage refresh-crap` that writes a candidate manifest to a `.xtask/` side path and/or the gate printing the exact refresh steps. Mirror #88's candidate-promotion model (approved drift is a deliberate, reviewable `cp`).
  >
  > Relates to #87 (gate failure output), #88 (baseline reanchor), #7 (line-independent CRAP compare).

- [x] **Step 2: Record the issue number** — #131

Note the assigned `#NNN` in your working notes — it's referenced in the Task 6 ADR/docs as the CRAP follow-on. (No file change.)

---

### Task 2: Extract the shared `classify_against_anchor` helper

A pure refactor of the gate so the new command can reuse the exact verdict+safety computation. No behavior change.

**Files:**
- Modify: `xtask/src/coverage/mod.rs` (extract lines ~169-184 of `run_inner` into a helper; call it from `run_inner`)
- Commit also includes the planning docs: `docs/superpowers/specs/2026-06-28-issue-88-reanchor-command-design.md`, `docs/superpowers/plans/2026-06-28-issue-88-reanchor-command.md`

**Interfaces:**
- Consumes (existing private fns in `mod.rs`): `baseline_anchor_commit()`, `git_diff_anchor_to_worktree(&str)`, `diffmap::parse_unified_diff(&str)`, `synthesize_untracked_maps(&mut _, &[FileCoverage], &[_])`, `untracked_rs_files()`, `load_baseline_at_anchor(&str)`, `classify::classify(&[FileCoverage], &Baseline, &maps)`, `reanchor::reanchor_is_safe(&verdict, &current, &baseline)`.
- Produces: `fn classify_against_anchor(current: &[FileCoverage]) -> Result<(Baseline, CoverageVerdict, reanchor::ReanchorSafety)>`.

- [x] **Step 1: Add the helper**

In `xtask/src/coverage/mod.rs`, add immediately above `fn run_inner` (~line 142):

```rust
/// Classify the current coverage against the **anchor-commit** baseline and compute
/// text-identity re-anchor safety — the shared core of the gate verdict and the
/// `coverage reanchor` command. Loading the baseline at the anchor (not the working
/// tree) keeps its frame aligned with the diff's "from" frame (#110).
fn classify_against_anchor(
    current: &[FileCoverage],
) -> Result<(Baseline, CoverageVerdict, reanchor::ReanchorSafety)> {
    let anchor = baseline_anchor_commit()?;
    let diff = git_diff_anchor_to_worktree(&anchor)?;
    let mut maps = diffmap::parse_unified_diff(&diff);
    synthesize_untracked_maps(&mut maps, current, &untracked_rs_files()?);
    let baseline = load_baseline_at_anchor(&anchor)?;
    let verdict = classify::classify(current, &baseline, &maps);
    let safety = reanchor::reanchor_is_safe(&verdict, current, &baseline);
    Ok((baseline, verdict, safety))
}
```

- [x] **Step 2: Call it from `run_inner`**

In `run_inner`, replace the block currently at lines ~169-184 (from `let anchor = baseline_anchor_commit()?;` through `let safety = reanchor::reanchor_is_safe(&verdict, &current, &baseline);`) with:

```rust
    let (baseline, verdict, safety) = classify_against_anchor(&current)?;
```

(The surrounding code — `let repo_root` / `let current` above, and `old_crap_manifest` / `heal_baseline(&safety, …, &current, &baseline, mode)` / `CoverageReport { … }` below — is unchanged; `baseline`, `verdict`, and `safety` keep the same names and types.)

- [x] **Step 3: Verify the gate still builds and its tests pass**

Run: `cargo nextest run --manifest-path xtask/Cargo.toml coverage`
Expected: PASS — every existing `coverage::mod` and `coverage::reanchor` test (the heal/gate/failure tests) still green; the extraction changed no behavior.

- [x] **Step 4: Per-task gate**

Run: `cargo xtask check --no-test`
Expected: exit 0, no clippy warnings.

- [x] **Step 5: Commit**

```bash
git add xtask/src/coverage/mod.rs docs/superpowers/specs/2026-06-28-issue-88-reanchor-command-design.md docs/superpowers/plans/2026-06-28-issue-88-reanchor-command.md
git commit -m "refactor(xtask): extract classify_against_anchor from the coverage gate (#88)

Shared anchor-load + classify + re-anchor-safety core, so the upcoming
coverage reanchor command reuses the gate's exact verdict. No behavior
change. Includes the #88 spec and plan."
```

---

### Task 3: Add the `coverage reanchor` command

**Files:**
- Modify: `xtask/src/coverage/reanchor.rs` (pure decision: `ReanchorPlan`, `plan_reanchor`, `refusal_report`, `CANDIDATE_PATH` + tests)
- Modify: `xtask/src/coverage/mod.rs` (`reanchor` / `reanchor_inner` orchestration)
- Modify: `xtask/src/lib.rs` (nested `Coverage(CoverageCommand)` clap group, `command_name` arm, `run` arm, parse tests)

**Interfaces:**
- Consumes: `classify_against_anchor` (Task 2); `Baseline::from_files(&[FileCoverage]) -> Baseline`, `Baseline::save(&self, &str)`; `report::parse_text_report(&str, &str) -> Vec<FileCoverage>`; `git_repo_root() -> Result<String>`; `BASELINE_PATH`; `reanchor::{ReanchorSafety, LineText}`.
- Produces: `reanchor::ReanchorPlan`; `reanchor::plan_reanchor(ReanchorSafety, Baseline) -> ReanchorPlan`; `reanchor::refusal_report(&[LineText]) -> String`; `reanchor::CANDIDATE_PATH: &str`; `coverage::reanchor(out_dir: &str) -> StepResult`; clap `CoverageCommand::Reanchor { gcroot: String }`.

- [x] **Step 1: Write the failing pure-decision tests**

In `xtask/src/coverage/reanchor.rs`, inside its existing `#[cfg(test)] mod` (reuse the `fl` fixture, `reanchor.rs:148`), add:

```rust
    #[test]
    fn plan_reanchors_when_safe() {
        let safety = ReanchorSafety { safe: true, lowering: vec![] };
        let candidate = Baseline::from_files(&[]);
        match plan_reanchor(safety, candidate) {
            ReanchorPlan::Reanchor { .. } => {}
            ReanchorPlan::Refuse { .. } => panic!("safe must re-anchor"),
        }
    }

    #[test]
    fn plan_refuses_and_carries_lowering_when_unsafe() {
        let safety = ReanchorSafety { safe: false, lowering: vec![fl("a.rs", 5, "}")] };
        let candidate = Baseline::from_files(&[]);
        match plan_reanchor(safety, candidate) {
            ReanchorPlan::Refuse { lowering, .. } => {
                assert_eq!(lowering.len(), 1);
                assert_eq!(lowering[0].line, 5);
            }
            ReanchorPlan::Reanchor { .. } => panic!("a lowering must refuse"),
        }
    }

    #[test]
    fn refusal_report_lists_lines_and_promotion_recipe() {
        let report = refusal_report(&[fl("src/x.rs", 12, "    Ok(())")]);
        assert!(report.contains("src/x.rs:12: Ok(())"));
        assert!(report.contains(CANDIDATE_PATH));
        assert!(report.contains("cp "));
        assert!(report.contains("git diff --no-index"));
    }
```

- [x] **Step 2: Run them to verify they fail**

Run: `cargo nextest run --manifest-path xtask/Cargo.toml reanchor`
Expected: FAIL — `cannot find ... plan_reanchor / ReanchorPlan / refusal_report / CANDIDATE_PATH`.

- [x] **Step 3: Implement the pure decision in `reanchor.rs`**

Add (after the existing `reanchor_is_safe` definition, before the test module). `Baseline` and `LineText` are already in scope in this file:

```rust
/// Where a refused re-anchor writes its candidate baseline. Under the gitignored
/// `/.xtask/`, so it never dirties the tree or gets instrumented.
pub const CANDIDATE_PATH: &str = ".xtask/coverage-baseline.candidate.json";

/// The action a `coverage reanchor` run should take, decided purely from the safety
/// verdict. The candidate content is identical in both arms
/// (`Baseline::from_files(current)`); only the destination and the exit status differ.
pub enum ReanchorPlan {
    /// Safe re-anchor — persist `baseline` to the committed `coverage-baseline.json`.
    Reanchor { baseline: Baseline },
    /// Genuine lowering — persist `candidate` to [`CANDIDATE_PATH`] and refuse,
    /// surfacing the offending lines.
    Refuse { candidate: Baseline, lowering: Vec<LineText> },
}

/// Decide the re-anchor action. Pure (no I/O) so it is unit-testable; the caller
/// performs the write and sets the exit status.
pub fn plan_reanchor(safety: ReanchorSafety, candidate: Baseline) -> ReanchorPlan {
    if safety.safe {
        ReanchorPlan::Reanchor { baseline: candidate }
    } else {
        ReanchorPlan::Refuse { candidate, lowering: safety.lowering }
    }
}

/// Operator-facing message for a refused re-anchor: the offending `file:line: text`
/// plus how to inspect and (only if genuinely approved) promote the candidate. There
/// is deliberately no flag that promotes automatically — approval is a visible diff.
pub fn refusal_report(lowering: &[LineText]) -> String {
    use std::fmt::Write as _;
    const MAX: usize = 25;
    let mut s = format!(
        "refused: {} genuinely-uncovered line(s) would lower coverage:",
        lowering.len()
    );
    for l in lowering.iter().take(MAX) {
        let _ = write!(s, "\n    {}:{}: {}", l.file, l.line, l.text.trim());
    }
    if lowering.len() > MAX {
        let _ = write!(s, "\n    … and {} more", lowering.len() - MAX);
    }
    let _ = write!(
        s,
        "\n  wrote candidate to {CANDIDATE_PATH} (NOT the committed baseline).\
         \n  inspect:  git diff --no-index coverage-baseline.json {CANDIDATE_PATH}\
         \n  if genuinely approved (coverage-baseline policy), promote:\
         \n    cp {CANDIDATE_PATH} coverage-baseline.json && git add coverage-baseline.json\
         \n  otherwise add a test — never promote a real loss."
    );
    s
}
```

- [x] **Step 4: Run the pure tests to verify they pass**

Run: `cargo nextest run --manifest-path xtask/Cargo.toml reanchor`
Expected: PASS — the three new tests plus all existing `reanchor_is_safe` tests.

- [x] **Step 5: Implement the orchestration in `mod.rs`**

In `xtask/src/coverage/mod.rs`, add (near `run`/`run_inner`):

```rust
/// `cargo xtask coverage reanchor`: re-anchor `coverage-baseline.json` from an
/// existing coverage report when the drift is a safe line-shift; on a genuine
/// lowering, write a candidate to the side path and FAIL (non-zero) with the
/// offending lines. Consumes an existing report — it does not rebuild coverage.
pub fn reanchor(out_dir: &str) -> StepResult {
    match reanchor_inner(out_dir) {
        Ok(step) => step,
        Err(e) => StepResult::fail("coverage-reanchor").detail(format!("{e:#}")),
    }
}

fn reanchor_inner(out_dir: &str) -> Result<StepResult> {
    let report_path = format!("{out_dir}/coverage-report.txt");
    let report = std::fs::read_to_string(&report_path).map_err(|_| {
        anyhow::anyhow!(
            "no coverage report at {report_path} — run `cargo xtask check` or \
             `cargo xtask validate` first to build one"
        )
    })?;
    let repo_root = git_repo_root()?;
    let current = report::parse_text_report(&report, &repo_root);
    let (_baseline, _verdict, safety) = classify_against_anchor(&current)?;
    let candidate = Baseline::from_files(&current);
    match reanchor::plan_reanchor(safety, candidate) {
        reanchor::ReanchorPlan::Reanchor { baseline } => {
            baseline.save(BASELINE_PATH)?;
            let n = current
                .iter()
                .filter(|f| f.lines.iter().any(|l| !l.covered))
                .count();
            Ok(StepResult::ok("coverage-reanchor")
                .detail(format!("re-anchored {BASELINE_PATH} ({n} file(s) with gaps)")))
        }
        reanchor::ReanchorPlan::Refuse { candidate, lowering } => {
            candidate.save(reanchor::CANDIDATE_PATH)?;
            Ok(StepResult::fail("coverage-reanchor").detail(reanchor::refusal_report(&lowering)))
        }
    }
}
```

- [x] **Step 6: Wire the nested clap command in `lib.rs`**

Add the new variant to the `Command` enum (after `AuditWasm { … }`, keeping `RegenBaseline` for now — it's removed in Task 4):

```rust
    /// Coverage-baseline maintenance.
    #[command(subcommand)]
    Coverage(CoverageCommand),
```

Add the nested enum just below the `Command` enum:

```rust
/// `coverage` subcommands.
#[derive(Subcommand)]
pub enum CoverageCommand {
    /// Re-anchor `coverage-baseline.json` to the current coverage report when the
    /// drift is a safe line-shift (ADR-0030); refuse and write a candidate to
    /// `.xtask/coverage-baseline.candidate.json` on a genuine coverage lowering.
    /// Consumes an existing report (run `check`/`validate` first); never rebuilds.
    #[command(after_help = "EXAMPLES:\n  \
        cargo xtask coverage reanchor\n  \
        cargo xtask coverage reanchor --gcroot .xtask/gcroots/coverage")]
    Reanchor {
        /// GC-root / out-link directory holding `coverage-report.txt`.
        #[arg(long, default_value = ".xtask/gcroots/coverage")]
        gcroot: String,
    },
}
```

Add the `command_name` arm (in `impl Cli::command_name`, after the `AuditWasm` arm):

```rust
            Command::Coverage(CoverageCommand::Reanchor { .. }) => "coverage-reanchor",
```

Add the `run` dispatch arm (after the `AuditWasm` arm):

```rust
        Command::Coverage(CoverageCommand::Reanchor { gcroot }) => {
            let start = std::time::Instant::now();
            let mut result = CommandResult::new("coverage-reanchor");
            result.push(coverage::reanchor(&gcroot));
            finalize(&mut result, start);
            Ok(result)
        }
```

- [x] **Step 7: Add the clap parse tests in `lib.rs`**

In the `lib.rs` `#[cfg(test)] mod cli_tests` (`lib.rs:~330`), after the `validate_*_parses` tests, add (note `CoverageCommand` is imported via the module's `use super::*;`):

```rust
    #[test]
    fn coverage_reanchor_parses_with_default_gcroot() {
        let cli = Cli::try_parse_from(["xtask", "coverage", "reanchor"]).unwrap();
        match cli.command {
            Command::Coverage(CoverageCommand::Reanchor { gcroot }) => {
                assert_eq!(gcroot, ".xtask/gcroots/coverage");
            }
            _ => panic!("expected coverage reanchor"),
        }
    }

    #[test]
    fn coverage_reanchor_accepts_gcroot() {
        let cli =
            Cli::try_parse_from(["xtask", "coverage", "reanchor", "--gcroot", "/tmp/x"]).unwrap();
        match cli.command {
            Command::Coverage(CoverageCommand::Reanchor { gcroot }) => assert_eq!(gcroot, "/tmp/x"),
            _ => panic!("expected coverage reanchor"),
        }
    }
```

(`cli_tests` uses `use super::*;`; if it instead imports specific names, add `CoverageCommand` to that import.)

- [x] **Step 8: Build, parse-check, and test**

Run: `cargo nextest run --manifest-path xtask/Cargo.toml`
Expected: PASS — including the new `cli_tests` and `reanchor` tests.

Run: `cargo run --manifest-path xtask/Cargo.toml -- coverage reanchor --help`
Expected: prints the reanchor help with the EXAMPLES `after_help` (confirms it's discoverable and un-hidden).

- [x] **Step 9: Per-task gate**

Run: `cargo xtask check --no-test`
Expected: exit 0, no clippy warnings.

- [x] **Step 10: Commit**

```bash
git add xtask/src/coverage/reanchor.rs xtask/src/coverage/mod.rs xtask/src/lib.rs
git commit -m "feat(xtask): add safe \`coverage reanchor\` command (#88)

Re-anchors coverage-baseline.json on a safe line-shift, refuses a genuine
lowering and writes a candidate to .xtask/coverage-baseline.candidate.json
(non-zero exit) with the offending file:line: text and a promotion recipe.
Reuses the sound diff-based reanchor_is_safe (ADR-0030). No accept-all path."
```

---

### Task 4: Remove the hidden `__regen-baseline` command

Now redundant — `coverage reanchor` is the discoverable, safe replacement.

**Files:**
- Modify: `xtask/src/lib.rs` (delete `RegenBaseline` variant, `command_name` arm, `run` arm, `regen_baseline` + `regen_baseline_inner`)

**Interfaces:** removes `Command::RegenBaseline` and the `regen_baseline*` fns; no new API.

- [ ] **Step 1: Delete the `RegenBaseline` enum variant**

In `xtask/src/lib.rs`, remove (the doc comment + `#[command(name = "__regen-baseline", hide = true)]` + `RegenBaseline { gcroot: String }`, currently ~lines 47-54).

- [ ] **Step 2: Delete the `command_name` arm**

Remove `Command::RegenBaseline { .. } => "__regen-baseline",` (~line 79).

- [ ] **Step 3: Delete the `run` dispatch arm**

Remove the `Command::RegenBaseline { gcroot } => { … }` block (~lines 124-131).

- [ ] **Step 4: Delete the `regen_baseline` / `regen_baseline_inner` fns**

Remove both functions (~lines 168-198). If a helper they used (e.g. a local with-gaps count) is now unreferenced, remove it too — let the compiler/clippy guide you.

- [ ] **Step 5: Verify build, clippy, and tests are clean**

Run: `cargo xtask check --no-test`
Expected: exit 0. The `command_name` and `run` matches over `Command` are exhaustive — the compiler confirms every `RegenBaseline` reference is gone; no `unused function` warnings (nothing else called `regen_baseline*`).

Run: `cargo nextest run --manifest-path xtask/Cargo.toml`
Expected: PASS — no test referenced `__regen-baseline`.

- [ ] **Step 6: Confirm no stray references**

Run: `git grep -n -- '__regen-baseline\|regen_baseline' -- ':!docs/archive' ':!docs/superpowers'`
Expected: no matches (the only historical references live in archived docs).

- [ ] **Step 7: Commit**

```bash
git add xtask/src/lib.rs
git commit -m "refactor(xtask): drop the hidden accept-all __regen-baseline (#88)

Replaced by the safe, discoverable \`coverage reanchor\`."
```

---

### Task 5: Gate failure message prints the recovery command

**Files:**
- Modify: `xtask/src/coverage/mod.rs` (`failure_report`, ~lines 287-293; its tests, ~686-735)

**Interfaces:** no signature change; `failure_report(&[reanchor::LineText], &[CrapRegression]) -> String` gains one line in the lowering branch.

- [ ] **Step 1: Update the failure-message tests first**

In `xtask/src/coverage/mod.rs` tests, update `failure_report_lists_lines_crap_and_recovery` (~686) to assert the command line appears when there is a lowering, and `failure_report_guidance_is_category_conditional` (~707) to assert it is **absent** for a CRAP-only failure. Add these assertions (adapt to the tests' existing local variable names for the rendered strings):

```rust
        // A coverage lowering points the operator at the exact recovery command.
        assert!(with_lowering.contains("cargo xtask coverage reanchor"));
        // A CRAP-only failure must NOT suggest reanchor (it doesn't touch CRAP).
        assert!(!crap_only.contains("cargo xtask coverage reanchor"));
```

- [ ] **Step 2: Run the tests to verify they fail**

Run: `cargo nextest run --manifest-path xtask/Cargo.toml failure_report`
Expected: FAIL — the `cargo xtask coverage reanchor` line is not emitted yet.

- [ ] **Step 3: Add the recovery command to `failure_report`**

In `failure_report`, extend the lowering-branch guidance (currently `mod.rs:287-293`) to end with the command. Replace that `if !lowering.is_empty() { s.push_str("…") }` block's string with:

```rust
        s.push_str(
            "\n  → these lines are a real coverage loss unless the baseline is stale\
             \n    (a shift the anchor diff couldn't see, e.g. after a rebase): add a\
             \n    test, or re-anchor only after confirming they are not a genuine loss.\
             \n    run:  cargo xtask coverage reanchor",
        );
```

(Leave the CRAP-branch guidance at `mod.rs:294-298` unchanged — it must not mention reanchor.)

- [ ] **Step 4: Run the tests to verify they pass**

Run: `cargo nextest run --manifest-path xtask/Cargo.toml failure_report`
Expected: PASS.

- [ ] **Step 5: Per-task gate**

Run: `cargo xtask check --no-test`
Expected: exit 0.

- [ ] **Step 6: Commit**

```bash
git add xtask/src/coverage/mod.rs
git commit -m "feat(xtask): coverage gate prints the reanchor recovery command (#88)

A coverage lowering now ends with \`run: cargo xtask coverage reanchor\`;
CRAP-only failures keep their manifest-refresh guidance (reanchor is
baseline-only)."
```

---

### Task 6: Docs — CONTRIBUTING + ADR-0030 supplement

**Files:**
- Modify: `CONTRIBUTING.md` (coverage section — document `coverage reanchor` + candidate promotion)
- Modify: `docs/adr/0030-coverage-reanchor-text-identity.md` (append `## Supplement (#88)`)

**Interfaces:** none.

- [ ] **Step 1: Document the command in CONTRIBUTING**

In `CONTRIBUTING.md`, in the coverage-gate paragraph that explains the heal / re-anchor (around the "uncovered-text identity" / "re-anchor" discussion, ~lines 188-194), add a sentence documenting the command and the candidate-promotion flow:

```markdown
When a coverage gate fails on a genuine lowering, run `cargo xtask coverage reanchor` (it consumes the report the gate just built under `.xtask/gcroots/coverage`). It re-anchors a safe line-shift in place, or — if the lowering is real — refuses, writes the would-be baseline to `.xtask/coverage-baseline.candidate.json`, and prints the offending `file:line: text`. Accepting a genuinely-approved lowering is then a deliberate, reviewable step: inspect with `git diff --no-index coverage-baseline.json .xtask/coverage-baseline.candidate.json` and, if approved per the coverage-baseline policy, `cp` the candidate over the committed baseline and commit it. There is no flag that lowers the baseline automatically.
```

(Place it adjacent to the existing re-anchor discussion; do not duplicate the merge-driver paragraph.)

- [ ] **Step 2: Append the ADR-0030 supplement**

At the end of `docs/adr/0030-coverage-reanchor-text-identity.md`, append:

```markdown

## Supplement (#88) — the explicit reanchor command

ADR-0030 anticipated "the explicit reanchor command (#88)". It lands as
`cargo xtask coverage reanchor`, and it does **not** introduce a second, weaker
safety notion: it reuses this ADR's `reanchor_is_safe` predicate (diff-based,
`appeared ⊆ structural`), so it refuses a genuine lowering exactly when the gate
does. The issue's original "uncovered-text-set unchanged/shrank" wording — a naive
multiset check — is **not** what shipped; that is the text-primary approach the #112
supplement above rejected as unsound.

The command is **candidate-promotion only**: a safe move re-anchors
`coverage-baseline.json` in place; a genuine lowering is refused (non-zero exit) and
the would-be baseline is written to `.xtask/coverage-baseline.candidate.json` (under
the gitignored `/.xtask/`), never the committed file. There is deliberately **no
accept-all path** — the removed `__regen-baseline` was exactly that footgun.
Accepting an approved lowering is a manual `cp` of the candidate, so it always lands
as a reviewable diff under the coverage-baseline policy. The failing coverage gate
prints `cargo xtask coverage reanchor` as the recovery for a lowering; the symmetric
CRAP-manifest refresh path is tracked separately (the #88 CRAP follow-on).
```

- [ ] **Step 3: Sanity-check**

Run: `git grep -n 'coverage reanchor' CONTRIBUTING.md docs/adr/0030-coverage-reanchor-text-identity.md`
Expected: the new references are present.

- [ ] **Step 4: Per-task gate**

Run: `cargo xtask check --no-test`
Expected: exit 0 (prettier/markdown formatting clean).

- [ ] **Step 5: Commit**

```bash
git add CONTRIBUTING.md docs/adr/0030-coverage-reanchor-text-identity.md
git commit -m "docs(#88): document coverage reanchor + ADR-0030 supplement

CONTRIBUTING documents the command and candidate-promotion flow; ADR-0030
records the reuse of reanchor_is_safe and the deliberate no-accept-all model."
```

---

## Final gate (before ship)

- [ ] Run the full CI-faithful gate from the worktree: `cargo xtask validate` (or `validate --no-e2e` per the autonomous-gate policy — the diff is xtask + docs only, no web/server/e2e surface). Expected: exit 0, `xtask-done: ... ok=true`.
- [ ] Review the branch diff against the fork point: `git diff wt-base-issue-88..HEAD`.
- [ ] Confirm the CRAP follow-on issue (Task 1) is filed and its number is captured.

## Self-review (plan vs. spec)

- **Spec coverage:** reuse sound `reanchor_is_safe` → Task 2 (shared helper) + Task 3 (command); candidate-promotion, no accept-all → Task 3 (`plan_reanchor`) + Task 4 (remove `__regen-baseline`); nested `coverage reanchor` → Task 3; gate recovery command → Task 5; remove `__regen-baseline` → Task 4; docs + ADR-0030 supplement → Task 6; CRAP separable concern → Task 1. All spec sections map to a task.
- **Placeholder scan:** none — every code/step block is concrete. (Task 1's issue `#NNN` is an output to capture, not a code placeholder.)
- **Type consistency:** `classify_against_anchor(&[FileCoverage]) -> Result<(Baseline, CoverageVerdict, ReanchorSafety)>`, `plan_reanchor(ReanchorSafety, Baseline) -> ReanchorPlan`, `refusal_report(&[LineText]) -> String`, `reanchor(&str) -> StepResult`, `CoverageCommand::Reanchor { gcroot: String }`, `CANDIDATE_PATH: &str` are used identically across their definitions (Tasks 2-3), the orchestration (Task 3), the tests (Task 3), and the docs (Task 6).
