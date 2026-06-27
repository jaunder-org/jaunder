# Coverage Heal Hardening (#86 + #7) Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Make the coverage gate self-heal benign line-shifts (#86) and stop the CRAP manifest churning when no CRAP score changed (#7), plus a keep-ours merge driver so the two JSON artifacts stop conflicting on overlapping merges.

**Architecture:** All changes are host-side in the `xtask` coverage engine that post-processes the Nix `coverage` check's reports. #86 adds a text-identity re-anchor predicate (`appeared ⊆ structural` per file) that overrides the line-identity classifier's phantom failures. #7 makes the CRAP compare key line-independent (ordinal tie-break) and the manifest rewrite trigger line-independent. A `.gitattributes` keep-ours driver defers authoritative regeneration to the next Fix-mode heal.

**Tech Stack:** Rust (`xtask` crate), `serde_json`, `git`, `cargo nextest` (host xtask unit tests via `cargo xtask check --no-test`).

**Spec:** `docs/superpowers/specs/2026-06-27-issue-86-coverage-heal-text-identity-design.md`

## Global Constraints

- **Worktree:** all work in `/home/mdorman/src/jaunder/.claude/worktrees/issue-86-coverage-heal-text-identity`. Run the gate from there (context-mode otherwise targets the main repo): `cd <worktree> && cargo xtask check --no-test`, or use the Bash tool (already in the worktree).
- **Per-task gate:** `cargo xtask check --no-test` (static + clippy + host xtask unit tests) must pass before each commit.
- **Commits use `--no-verify`.** The installed `.githooks/pre-commit` runs raw `cargo nextest run` against PostgreSQL with no ephemeral cluster and fails with `Connection refused` (the broken raw hook #99 replaces). The authoritative gate is `cargo xtask check --no-test` per task and `cargo xtask validate --no-e2e` at the end, run explicitly.
- **No Co-Authored-By trailers.**
- **TDD:** failing test → run-it-fails → implement → run-it-passes → commit.
- **Coverage policy:** never lower the baseline without approval; auto-heal/re-anchor only when coverage is provably not lowered.

---

### Task 1: File the follow-on issue (merge-driver auto-registration)

Out-of-scope separable concern captured up front so it can be picked up independently.

**Files:** none (GitHub issue only).

- [ ] **Step 1: Create the issue**

```bash
gh issue create -R jaunder-org/jaunder \
  --title "tooling: auto-register the coverage keep-ours merge driver (self-healing installer + optional post-merge re-heal)" \
  --label tooling \
  --milestone "Verify-gate hardening" \
  --body "Follow-on from #86/#7. That cycle adds a keep-ours git merge driver for \`coverage-baseline.json\` and \`crap-manifest.json\` (committed \`.gitattributes\` + a one-shot \`cargo xtask install-merge-driver\`). The driver only takes effect once registered in local git config, which is not version-controlled, so a fresh clone/worktree still hits conflicts until someone runs the one-shot.

Automate it: register \`merge.coverage-keepours.driver=true\` from the self-healing hook/installer (coherent with #99) so every clone and worktree wires itself up. Optionally add a \`post-merge\` hook that eagerly re-heals the artifacts (\`cargo xtask check\`) so the keep-ours result is reconciled to the merged tree without waiting for the next manual gate run.

Relates to #99 (git-enforced gate convergence)."
```

Expected: prints the new issue URL.

- [ ] **Step 2: Record the issue number** in the PR description later (no commit).

---

### Task 2: ADR 0029 — coverage re-anchor by text identity

**Files:**
- Create: `docs/adr/0029-coverage-reanchor-text-identity.md`
- Modify: `docs/README.md` (ADR table — add the 0029 row, matching the existing row format)

- [ ] **Step 1: Write the ADR**

Create `docs/adr/0029-coverage-reanchor-text-identity.md`:

```markdown
# 0029. Coverage re-anchor by text identity

Status: accepted

## Context

The coverage gate classifies each uncovered line by **line number** against the
committed `coverage-baseline.json` (ADR-0019 era engine). A line-shifting change
whose unified diff models an accepted-uncovered gap as deleted-then-reappeared
produces a *phantom* regression/new-uncovered: the line did not change coverage,
it only moved. This blocked the Fix-mode auto-heal and forced manual
regeneration (#51/#52/#53 refactors, #63 sweep).

The naive fix — "current uncovered text multiset ⊆ baseline accepted text
multiset" — is unsound: covering one `}` while a different identical-text `}`
regresses leaves the multiset unchanged and would mask the regression.

## Decision

The heal's safety condition is **text-identity re-anchor**, keyed on what the
diff *removed* vs. what *appeared*:

- `structural_texts(file)` = texts of accepted gaps the diff removed (the
  classifier's `structural` bucket).
- `appeared_texts(file)` = texts of newly-flagged uncovered lines
  (`regressions` ∪ `new_uncovered`).
- **Safe re-anchor iff, per file, `appeared_texts` ⊆ `structural_texts`** as a
  multiset — every newly-flagged uncovered line is explained by an accepted gap
  of identical text that the diff removed (the line genuinely moved).

When safe, `cargo xtask check` (Fix) re-anchors the baseline and passes;
`validate` (Check) passes without mutating. When an appeared text has no removed
counterpart (genuine lowering), the gate still fails.

## Consequences

- Benign line-shifts (including those introduced by concurrently-merged
  branches) self-heal instead of forcing manual regeneration.
- Residual ambiguity: two identical-text lines in one file, where one is removed
  as an accepted gap and an unrelated identical-text line regresses in the same
  change, can be conflated as a safe move. Bounded and accepted; the
  line-identity classifier remains the primary signal — text-identity only
  *excuses* line failures the diff explains as moves.
- The predicate is a single primitive (`reanchor_is_safe`) reused by the gate
  and, later, the explicit reanchor command (#88).
```

- [ ] **Step 2: Add the ADR table row in `docs/README.md`**

Find the ADR table (rows like `| 0028 | ... | accepted |`) and add, in number order:

```markdown
| 0029 | Coverage re-anchor by text identity | accepted |
```

Match the exact column layout of the surrounding rows (read the 0028 row first and mirror it).

- [ ] **Step 3: Commit**

```bash
git add docs/adr/0029-coverage-reanchor-text-identity.md docs/README.md
git commit --no-verify -m "docs(adr): 0029 coverage re-anchor by text identity (#86)"
```

---

### Task 3: Re-anchor safety predicate (`reanchor.rs`)

The #86 primitive. Pure function over the classifier verdict + current report + baseline; no Nix, fully unit-testable.

**Files:**
- Create: `xtask/src/coverage/reanchor.rs`
- Modify: `xtask/src/coverage/mod.rs` (add `pub mod reanchor;` next to the other `pub mod` lines, ~line 15-19)

**Interfaces:**
- Consumes: `CoverageVerdict`, `FileCoverage` (from `crate::coverage`), `Baseline` (from `crate::coverage::baseline`).
- Produces:
  - `pub struct LineText { pub file: String, pub line: u32, pub text: String }`
  - `pub struct ReanchorSafety { pub safe: bool, pub lowering: Vec<LineText> }` (derives `Clone, Debug, Default, PartialEq`)
  - `pub fn reanchor_is_safe(verdict: &CoverageVerdict, current: &[FileCoverage], baseline: &Baseline) -> ReanchorSafety`

- [ ] **Step 1: Register the module**

In `xtask/src/coverage/mod.rs`, add to the module list (with the existing `pub mod baseline; pub mod classify; ...`):

```rust
pub mod reanchor;
```

- [ ] **Step 2: Write failing tests**

Create `xtask/src/coverage/reanchor.rs` with only the tests first (the types/fn won't exist yet → compile fail is the "failure"):

```rust
//! Text-identity re-anchor safety for the coverage heal.
//!
//! The line-identity classifier flags an uncovered line as a
//! `regression`/`new_uncovered` purely by line number. A line-shifting change
//! whose diff models an accepted gap as deleted-then-reappeared produces a
//! *phantom* failure even though coverage is unchanged. This module decides
//! whether such a line-dirty verdict is in fact a safe re-anchor: per file, the
//! multiset of *appeared* texts (regressions ∪ new_uncovered) must be contained
//! in the multiset of *structural* texts (accepted gaps the diff removed). A new
//! uncovered text with no removed-gap counterpart is a genuine lowering.

use std::collections::HashMap;

use crate::coverage::baseline::Baseline;
use crate::coverage::{CoverageVerdict, FileCoverage};

/// One genuinely-lowered uncovered line (an appeared text with no matching
/// removed-gap text), for the gate's failure report.
#[derive(Clone, Debug, PartialEq)]
pub struct LineText {
    pub file: String,
    pub line: u32,
    pub text: String,
}

/// The re-anchor safety verdict: `safe` overall, plus the genuine lowerings.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct ReanchorSafety {
    pub safe: bool,
    pub lowering: Vec<LineText>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::coverage::baseline::{Baseline, Gap};
    use crate::coverage::{FileCoverage, FileLines, LineCov};

    fn fc(path: &str, lines: &[(u32, bool, &str)]) -> FileCoverage {
        FileCoverage {
            path: path.into(),
            lines: lines
                .iter()
                .map(|(l, c, t)| LineCov {
                    line: *l,
                    covered: *c,
                    text: (*t).into(),
                })
                .collect(),
        }
    }

    fn baseline_with(path: &str, gaps: Vec<Gap>) -> Baseline {
        let mut b = Baseline::default();
        b.set_gaps(path, gaps);
        b
    }

    fn fl(file: &str, lines: &[u32]) -> FileLines {
        FileLines { file: file.into(), lines: lines.to_vec() }
    }

    #[test]
    fn pure_move_is_safe() {
        // Accepted gap "let x = 1;" was at line 2 (now removed by the diff →
        // structural) and reappears uncovered at line 9 (→ new_uncovered),
        // identical text. appeared ⊆ structural → safe.
        let baseline = baseline_with("a.rs", vec![Gap { line: 2, text: "let x = 1;".into() }]);
        let current = vec![fc("a.rs", &[(9, false, "let x = 1;")])];
        let verdict = CoverageVerdict {
            structural: vec![fl("a.rs", &[2])],
            new_uncovered: vec![fl("a.rs", &[9])],
            ..Default::default()
        };
        let s = reanchor_is_safe(&verdict, &current, &baseline);
        assert!(s.safe, "identical-text move must be a safe re-anchor");
        assert!(s.lowering.is_empty());
    }

    #[test]
    fn net_zero_swap_is_not_safe() {
        // An accepted "}" gap was COVERED (improvement, not structural) and a
        // different "}" regressed. structural has no "}" → appeared "}" is a
        // genuine lowering.
        let baseline = baseline_with("a.rs", vec![Gap { line: 2, text: "}".into() }]);
        let current = vec![fc("a.rs", &[(2, true, "}"), (7, false, "}")])];
        let verdict = CoverageVerdict {
            improvements: vec![fl("a.rs", &[2])],
            regressions: vec![fl("a.rs", &[7])],
            ..Default::default()
        };
        let s = reanchor_is_safe(&verdict, &current, &baseline);
        assert!(!s.safe, "covering one `}}` then regressing another must fail");
        assert_eq!(s.lowering, vec![LineText { file: "a.rs".into(), line: 7, text: "}".into() }]);
    }

    #[test]
    fn genuine_new_uncovered_is_not_safe() {
        // Brand-new uncovered text, nothing removed → not contained.
        let baseline = Baseline::default();
        let current = vec![fc("a.rs", &[(5, false, "todo!()")])];
        let verdict = CoverageVerdict {
            new_uncovered: vec![fl("a.rs", &[5])],
            ..Default::default()
        };
        let s = reanchor_is_safe(&verdict, &current, &baseline);
        assert!(!s.safe);
        assert_eq!(s.lowering.len(), 1);
    }

    #[test]
    fn line_clean_verdict_is_safe() {
        // No appeared failures → trivially safe.
        let s = reanchor_is_safe(&CoverageVerdict::default(), &[], &Baseline::default());
        assert!(s.safe);
        assert!(s.lowering.is_empty());
    }

    #[test]
    fn duplicate_text_counts_are_multiset() {
        // Two accepted "}" removed; two "}" reappear → contained (safe).
        let baseline = baseline_with(
            "a.rs",
            vec![Gap { line: 2, text: "}".into() }, Gap { line: 4, text: "}".into() }],
        );
        let current = vec![fc("a.rs", &[(8, false, "}"), (9, false, "}")])];
        let verdict = CoverageVerdict {
            structural: vec![fl("a.rs", &[2, 4])],
            new_uncovered: vec![fl("a.rs", &[8, 9])],
            ..Default::default()
        };
        assert!(reanchor_is_safe(&verdict, &current, &baseline).safe);
    }

    #[test]
    fn more_appeared_than_removed_is_not_safe() {
        // One "}" removed but two "}" appear → one is a genuine lowering.
        let baseline = baseline_with("a.rs", vec![Gap { line: 2, text: "}".into() }]);
        let current = vec![fc("a.rs", &[(8, false, "}"), (9, false, "}")])];
        let verdict = CoverageVerdict {
            structural: vec![fl("a.rs", &[2])],
            new_uncovered: vec![fl("a.rs", &[8, 9])],
            ..Default::default()
        };
        let s = reanchor_is_safe(&verdict, &current, &baseline);
        assert!(!s.safe);
        assert_eq!(s.lowering.len(), 1);
    }
}
```

- [ ] **Step 3: Run tests to verify they fail**

Run: `cargo test -p xtask --lib coverage::reanchor`
Expected: FAIL — `reanchor_is_safe` not found.

- [ ] **Step 4: Implement the predicate**

Insert above the `#[cfg(test)]` module in `reanchor.rs`:

```rust
/// Decide whether a (possibly line-dirty) verdict is a safe re-anchor.
///
/// `current` supplies the text of appeared (currently-uncovered) lines;
/// `baseline` supplies the text of structural (removed accepted-gap) lines.
/// Owned `String` keys keep the multiset bookkeeping free of cross-borrow
/// lifetime entanglement between `current` and `baseline`.
pub fn reanchor_is_safe(
    verdict: &CoverageVerdict,
    current: &[FileCoverage],
    baseline: &Baseline,
) -> ReanchorSafety {
    // (file, line) -> current source text.
    let mut cur_text: HashMap<(String, u32), String> = HashMap::new();
    for f in current {
        for l in &f.lines {
            cur_text.insert((f.path.clone(), l.line), l.text.clone());
        }
    }

    // Files that have any appeared (regression / new_uncovered) failure.
    let mut appeared_files: Vec<&str> = verdict
        .regressions
        .iter()
        .chain(&verdict.new_uncovered)
        .map(|fl| fl.file.as_str())
        .collect();
    appeared_files.sort_unstable();
    appeared_files.dedup();

    let mut lowering = Vec::new();
    for file in appeared_files {
        // Multiset of structural (removed accepted-gap) texts for this file.
        let structural_lines: std::collections::HashSet<u32> = verdict
            .structural
            .iter()
            .filter(|fl| fl.file == file)
            .flat_map(|fl| fl.lines.iter().copied())
            .collect();
        let mut counts: HashMap<String, i64> = HashMap::new();
        for g in baseline.gaps(file) {
            if structural_lines.contains(&g.line) {
                *counts.entry(g.text.clone()).or_default() += 1;
            }
        }

        // Each appeared line consumes one matching structural text; an
        // unmatched appeared text is a genuine lowering.
        let mut appeared: Vec<u32> = verdict
            .regressions
            .iter()
            .chain(&verdict.new_uncovered)
            .filter(|fl| fl.file == file)
            .flat_map(|fl| fl.lines.iter().copied())
            .collect();
        appeared.sort_unstable();
        for line in appeared {
            let text = cur_text
                .get(&(file.to_string(), line))
                .cloned()
                .unwrap_or_default();
            let slot = counts.entry(text.clone()).or_default();
            if *slot > 0 {
                *slot -= 1;
            } else {
                lowering.push(LineText { file: file.to_string(), line, text });
            }
        }
    }

    ReanchorSafety { safe: lowering.is_empty(), lowering }
}
```

- [ ] **Step 5: Run tests to verify they pass**

Run: `cargo test -p xtask --lib coverage::reanchor`
Expected: PASS (6 tests).

- [ ] **Step 6: Gate + commit**

```bash
cargo xtask check --no-test
git add xtask/src/coverage/reanchor.rs xtask/src/coverage/mod.rs
git commit --no-verify -m "feat(xtask): text-identity re-anchor safety predicate (#86)"
```

---

### Task 4: Wire re-anchor into the gate and heal

Replace the line-identity `verdict.is_clean()` gate/heal condition with the text-identity `ReanchorSafety`.

**Files:**
- Modify: `xtask/src/coverage/mod.rs` (`heal_baseline` signature + `run_inner` gate logic + the `#[cfg(test)]` calls to `heal_baseline`)

**Interfaces:**
- Consumes: `reanchor::{reanchor_is_safe, ReanchorSafety}` (Task 3).
- Produces: `heal_baseline(safety: &ReanchorSafety, crap_regs: &[CrapRegression], current: &[FileCoverage], loaded: &Baseline, mode: Mode) -> (Option<Baseline>, bool)`.

- [ ] **Step 1: Change `heal_baseline` to take `ReanchorSafety`**

In `mod.rs`, replace the `heal_baseline` signature and the `clean` line:

```rust
fn heal_baseline(
    safety: &reanchor::ReanchorSafety,
    crap_regs: &[CrapRegression],
    current: &[FileCoverage],
    loaded: &Baseline,
    mode: Mode,
) -> (Option<Baseline>, bool) {
    let clean = safety.safe && crap_regs.is_empty();
    if !matches!(mode, Mode::Fix) || !clean {
        return (None, false);
    }
    let healed = Baseline::from_files(current);
    if healed.to_json() != loaded.to_json() {
        (Some(healed), true)
    } else {
        (None, false)
    }
}
```

(Update the doc comment above it: heal happens when the run is a *safe re-anchor* — `safety.safe` — with no CRAP regressions, generalising the old line-clean condition.)

- [ ] **Step 2: Wire it into `run_inner`**

In `run_inner`, after `let verdict = classify::classify(&current, &baseline, &maps);` add:

```rust
    let safety = reanchor::reanchor_is_safe(&verdict, &current, &baseline);
```

Replace `let gate_fails = !verdict.is_clean() || !crap_regs.is_empty();` with:

```rust
    let gate_fails = !safety.safe || !crap_regs.is_empty();
```

Replace the `heal_baseline(&verdict, ...)` call with `heal_baseline(&safety, ...)`.

In the CRAP rewrite-trigger `if` (still in `run_inner`), replace `verdict.is_clean()` with `safety.safe`:

```rust
    if matches!(mode, Mode::Fix) && safety.safe && crap_regs.is_empty() {
```

Replace the `gate_fails` detail/`else` detail blocks with:

```rust
    let step = if gate_fails {
        let detail = format!(
            "{} coverage lowering(s), {} CRAP regression(s)",
            safety.lowering.len(),
            crap_regs.len(),
        );
        StepResult::fail("coverage").detail(detail)
    } else {
        let reanchored = count_lines(&verdict.regressions) + count_lines(&verdict.new_uncovered);
        let detail = format!(
            "clean — {reanchored} re-anchored, {} structural, {} improvement(s){}",
            count_lines(&verdict.structural),
            count_lines(&verdict.improvements),
            if healed { "; baselines healed" } else { "" },
        );
        StepResult::ok("coverage").detail(detail)
    };
```

- [ ] **Step 3: Update existing `heal_baseline` unit tests**

The tests `heals_a_shrunk_baseline_when_clean_in_fix_mode`, `does_not_heal_when_a_regression_is_present_even_in_fix_mode`, `does_not_heal_when_crap_regressions_present`, and `never_heals_in_check_mode` call `heal_baseline(&verdict, ...)`. They now pass a `ReanchorSafety`. Update each call site:

- "clean/improvement heals": pass `&reanchor::ReanchorSafety { safe: true, lowering: vec![] }`.
- "a regression present must not heal": pass `&reanchor::ReanchorSafety { safe: false, lowering: vec![reanchor::LineText { file: "a.rs".into(), line: 5, text: String::new() }] }`.
- "crap regressions present": pass `&reanchor::ReanchorSafety { safe: true, lowering: vec![] }` (CRAP blocks the heal independently).
- "never heals in check mode": pass `&reanchor::ReanchorSafety { safe: true, lowering: vec![] }`, `Mode::Check`.

Add `use crate::coverage::reanchor;` to the test module if not already in scope. Delete the now-unused `verdict_with_improvement()` helper if it is no longer referenced (or keep it only if a remaining test uses it — verify with the compiler).

- [ ] **Step 4: Add a gate-level test for the re-anchor pass**

In `mod.rs` tests, add:

```rust
    #[test]
    fn safe_reanchor_heals_in_fix_and_not_in_check() {
        let safety = reanchor::ReanchorSafety { safe: true, lowering: vec![] };
        let mut loaded = Baseline::default();
        loaded.set_gaps("a.rs", vec![baseline::Gap { line: 2, text: "x".into() }]);
        // current re-anchors the gap to line 9 (same text), baseline regenerated
        // from current drops the old numbering.
        let current = vec![FileCoverage {
            path: "a.rs".into(),
            lines: vec![LineCov { line: 9, covered: false, text: "x".into() }],
        }];

        let (fix, healed) = heal_baseline(&safety, &[], &current, &loaded, Mode::Fix);
        assert!(healed && fix.is_some(), "Fix re-anchors a safe drift");

        let (chk, healed) = heal_baseline(&safety, &[], &current, &loaded, Mode::Check);
        assert!(!healed && chk.is_none(), "Check never mutates");
    }
```

- [ ] **Step 5: Run tests**

Run: `cargo test -p xtask --lib coverage::`
Expected: PASS.

- [ ] **Step 6: Gate + commit**

```bash
cargo xtask check --no-test
git add xtask/src/coverage/mod.rs
git commit --no-verify -m "feat(xtask): gate + heal on text-identity re-anchor, not line identity (#86)"
```

---

### Task 5: Line-independent CRAP compare key (ordinal tie-break)

#7, part 1. The compare must survive line drift, so its key drops the absolute `line` in favour of an ordinal within each `(crate, file, function)` group.

**Files:**
- Modify: `xtask/src/coverage/crap.rs`

**Interfaces:**
- Produces: `pub fn compare(new_report: &str, old_manifest: &str) -> Result<Vec<CrapRegression>>` (unchanged signature; internal key changes).

- [ ] **Step 1: Add failing tests**

Add to the `#[cfg(test)]` module in `crap.rs`:

```rust
    const OLD_LINE1: &str =
        r#"{"entries":[{"crate":"c","file":"a.rs","function":"f","line":1,"crap":2.0}]}"#;
    // Same function, shifted to line 99, CRAP worsened.
    const NEW_SHIFTED_WORSE: &str =
        r#"{"entries":[{"crate":"c","file":"a.rs","function":"f","line":99,"crap":5.0}]}"#;
    // Same function, shifted, CRAP unchanged.
    const NEW_SHIFTED_SAME: &str =
        r#"{"entries":[{"crate":"c","file":"a.rs","function":"f","line":99,"crap":2.0}]}"#;

    #[test]
    fn detects_regression_across_a_line_shift() {
        let r = compare(NEW_SHIFTED_WORSE, OLD_LINE1).unwrap();
        assert_eq!(r.len(), 1, "line shift must not hide a real CRAP regression");
        assert_eq!(r[0].function, "f");
    }

    #[test]
    fn line_shift_alone_is_not_a_regression() {
        assert!(compare(NEW_SHIFTED_SAME, OLD_LINE1).unwrap().is_empty());
    }

    #[test]
    fn same_name_functions_in_one_file_are_disambiguated_by_ordinal() {
        // Two `from` impls in one file; the second worsened, the first held.
        let old = r#"{"entries":[
            {"crate":"c","file":"a.rs","function":"from","line":10,"crap":2.0},
            {"crate":"c","file":"a.rs","function":"from","line":20,"crap":2.0}]}"#;
        let new = r#"{"entries":[
            {"crate":"c","file":"a.rs","function":"from","line":10,"crap":2.0},
            {"crate":"c","file":"a.rs","function":"from","line":20,"crap":9.0}]}"#;
        let r = compare(new, old).unwrap();
        assert_eq!(r.len(), 1, "only the second `from` regressed");
        assert_eq!((r[0].old, r[0].new), (2.0, 9.0));
    }
```

- [ ] **Step 2: Run to verify failure**

Run: `cargo test -p xtask --lib coverage::crap`
Expected: FAIL — `detects_regression_across_a_line_shift` fails (current line-keyed compare misses the shift).

- [ ] **Step 3: Replace the key + `compare`**

Change the `Key` type and rewrite the keying. Replace:

```rust
type Key = (String, String, String, i64);

fn key(e: &Entry) -> Key {
    (
        e.crate_field.clone(),
        e.file.clone(),
        e.function.clone(),
        e.line,
    )
}
```

with:

```rust
/// (crate, file, function, ordinal). The ordinal is the entry's index among
/// those sharing (crate, file, function), ordered by line — a shift-stable
/// disambiguator for same-named functions in one file, replacing the
/// churn-prone absolute `line` in the compare key (#7).
type Key = (String, String, String, usize);

/// Map every entry to its line-independent key → CRAP score.
fn keyed(entries: &[Entry]) -> HashMap<Key, f64> {
    let mut groups: HashMap<(String, String, String), Vec<(i64, f64)>> = HashMap::new();
    for e in entries {
        groups
            .entry((e.crate_field.clone(), e.file.clone(), e.function.clone()))
            .or_default()
            .push((e.line, e.crap));
    }
    let mut out = HashMap::new();
    for ((c, f, fun), mut v) in groups {
        v.sort_by_key(|(line, _)| *line);
        for (i, (_, crap)) in v.into_iter().enumerate() {
            out.insert((c.clone(), f.clone(), fun.clone(), i), crap);
        }
    }
    out
}
```

Replace the body of `compare`:

```rust
pub fn compare(new_report: &str, old_manifest: &str) -> Result<Vec<CrapRegression>> {
    let new: Report = serde_json::from_str(new_report)?;
    let old: Report = serde_json::from_str(old_manifest)?;
    let old_by_key = keyed(&old.entries);

    // Re-derive the new side's ordinals alongside the entry so a regression can
    // report the offending file/function.
    let mut groups: HashMap<(String, String, String), Vec<&Entry>> = HashMap::new();
    for e in &new.entries {
        groups
            .entry((e.crate_field.clone(), e.file.clone(), e.function.clone()))
            .or_default()
            .push(e);
    }
    let mut regressions = Vec::new();
    for ((c, f, fun), mut v) in groups {
        v.sort_by_key(|e| e.line);
        for (i, e) in v.into_iter().enumerate() {
            let k = (c.clone(), f.clone(), fun.clone(), i);
            if let Some(&old_crap) = old_by_key.get(&k) {
                if e.crap > old_crap + EPSILON {
                    regressions.push(CrapRegression {
                        file: e.file.clone(),
                        function: e.function.clone(),
                        old: old_crap,
                        new: e.crap,
                    });
                }
            }
        }
    }
    Ok(regressions)
}
```

- [ ] **Step 4: Run tests**

Run: `cargo test -p xtask --lib coverage::crap`
Expected: PASS (existing + 3 new tests).

- [ ] **Step 5: Gate + commit**

```bash
cargo xtask check --no-test
git add xtask/src/coverage/crap.rs
git commit --no-verify -m "fix(xtask): key CRAP compare on ordinal, not absolute line (#7)"
```

---

### Task 6: Line-independent CRAP manifest rewrite trigger

#7, part 2. Stop rewriting `crap-manifest.json` when only line attribution changed. `line` stays in the file as a labelled hint.

**Files:**
- Modify: `xtask/src/coverage/mod.rs` (replace `normalize_json` / `normalize_json_or_empty` with `normalize_crap_without_line`; update the trigger; update the `crap_heal_is_idempotent_and_pretty` test; add a line-shift no-op test)

- [ ] **Step 1: Update the idempotence test + add a line-shift test**

In `mod.rs` tests, replace `crap_heal_is_idempotent_and_pretty` with:

```rust
    #[test]
    fn crap_normalize_ignores_line_and_formatting() {
        // Same scores, different line attribution + formatting → equal canonical
        // form, so the heal does not rewrite the manifest (#7).
        let a = r#"{"entries":[{"crate":"c","file":"a.rs","function":"f","line":1,"crap":2.0}]}"#;
        let b = r#"{ "entries": [ {"crap":2.0,"function":"f","file":"a.rs","crate":"c","line":888} ] }"#;
        assert_eq!(
            normalize_crap_without_line(a).unwrap(),
            normalize_crap_without_line(b).unwrap(),
            "line + key order + whitespace must not affect the canonical form"
        );
    }

    #[test]
    fn crap_normalize_detects_a_score_change() {
        let a = r#"{"entries":[{"crate":"c","file":"a.rs","function":"f","line":1,"crap":2.0}]}"#;
        let c = r#"{"entries":[{"crate":"c","file":"a.rs","function":"f","line":1,"crap":9.0}]}"#;
        assert_ne!(
            normalize_crap_without_line(a).unwrap(),
            normalize_crap_without_line(c).unwrap(),
            "a real CRAP change must change the canonical form"
        );
    }

    #[test]
    fn crap_pretty_json_is_multiline() {
        let compact =
            r#"{"entries":[{"crate":"c","file":"a.rs","function":"f","line":1,"crap":2.0}]}"#;
        assert!(pretty_json(compact).unwrap().contains('\n'));
    }
```

- [ ] **Step 2: Run to verify failure**

Run: `cargo test -p xtask --lib coverage::`
Expected: FAIL — `normalize_crap_without_line` not found.

- [ ] **Step 3: Replace the normalizers**

In `mod.rs`, delete `normalize_json` and `normalize_json_or_empty`. Keep `pretty_json`. Add:

```rust
/// Canonical, line- and order-independent form of a CRAP report: each entry
/// minus its `line`, with key-sorted JSON (serde_json `Value` is a `BTreeMap`),
/// and the entry set itself sorted. Two reports that differ only in line
/// attribution (a pure shift) normalize equal, so the Fix-mode heal does not
/// rewrite `crap-manifest.json` unless a CRAP-relevant field changed (#7). The
/// `line` field is retained in the written manifest as a non-authoritative
/// jump-to hint that refreshes wholesale on the next real CRAP change.
fn normalize_crap_without_line(s: &str) -> Result<String> {
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
```

- [ ] **Step 4: Update the rewrite trigger**

Replace the trigger body (the `if normalize_json(...) != normalize_json_or_empty(...)` block) with:

```rust
        // Compare line-independently so a pure line-shift is a no-op; write the
        // full pretty manifest (WITH line, the labelled hint) when a
        // CRAP-relevant field actually changed.
        let new_canon = normalize_crap_without_line(&crap_report_str)?;
        let old_canon = normalize_crap_without_line(&old_crap_manifest).unwrap_or_default();
        if new_canon != old_canon {
            std::fs::write(CRAP_MANIFEST_PATH, pretty_json(&crap_report_str)?)
                .with_context(|| format!("writing {CRAP_MANIFEST_PATH}"))?;
            healed = true;
        }
```

(The enclosing `if matches!(mode, Mode::Fix) && safety.safe && crap_regs.is_empty()` from Task 4 stays.)

- [ ] **Step 5: Run tests**

Run: `cargo test -p xtask --lib coverage::`
Expected: PASS.

- [ ] **Step 6: Gate + commit**

```bash
cargo xtask check --no-test
git add xtask/src/coverage/mod.rs
git commit --no-verify -m "fix(xtask): rewrite crap-manifest only on real CRAP change, line is a hint (#7)"
```

---

### Task 7: Keep-ours merge driver for the coverage artifacts

`.gitattributes` + a one-shot `cargo xtask install-merge-driver` + an integration test of the keep-ours behaviour.

**Files:**
- Create: `.gitattributes`
- Modify: `xtask/src/lib.rs` (add `InstallMergeDriver` command, `command_name`, `run` arm, `install_merge_driver`/`register_keepours` fns + a test)

**Interfaces:**
- Produces: `Command::InstallMergeDriver`; `fn register_keepours(repo_dir: &std::path::Path) -> anyhow::Result<()>`.

- [ ] **Step 1: Create `.gitattributes`**

Create `.gitattributes` at the repo root:

```gitattributes
# Generated coverage artifacts. Keep-ours on merge: a merge resolves to our side
# with no conflict markers, and the coverage gate's Fix-mode heal restores the
# authoritative content on the next `cargo xtask check`. Register the driver once
# per clone/worktree with: cargo xtask install-merge-driver
coverage-baseline.json merge=coverage-keepours
crap-manifest.json merge=coverage-keepours
```

- [ ] **Step 2: Add the command variant**

In `xtask/src/lib.rs`, add to the `Command` enum (after `AuditWasm { .. }`):

```rust
    /// Register the keep-ours git merge driver for the generated coverage
    /// artifacts. `.gitattributes` maps `coverage-baseline.json` and
    /// `crap-manifest.json` to `merge=coverage-keepours`; git config is not
    /// version-controlled, so this one-shot wires the driver into the local
    /// clone (run once per clone/worktree).
    #[command(name = "install-merge-driver")]
    InstallMergeDriver,
```

Add to `command_name`:

```rust
            Command::InstallMergeDriver => "install-merge-driver",
```

Add the `run` arm (in the `match cli.command`):

```rust
        Command::InstallMergeDriver => {
            let start = std::time::Instant::now();
            let mut result = CommandResult::new("install-merge-driver");
            result.push(install_merge_driver());
            finalize(&mut result, start);
            Ok(result)
        }
```

- [ ] **Step 3: Add the implementation**

Add these functions to `xtask/src/lib.rs` (near `regen_baseline`):

```rust
/// Register the keep-ours merge driver in `repo_dir`'s local git config. The
/// driver command is `true`: it exits 0 without touching `%A` (ours), so a merge
/// of the generated coverage artifacts resolves to our side with no conflict
/// markers. The next `cargo xtask check` re-heals to the merged-tree state.
fn register_keepours(repo_dir: &std::path::Path) -> anyhow::Result<()> {
    use anyhow::ensure;
    let cfg = |args: &[&str]| -> anyhow::Result<()> {
        let status = std::process::Command::new("git")
            .arg("-C")
            .arg(repo_dir)
            .args(args)
            .status()?;
        ensure!(status.success(), "git {:?} failed", args);
        Ok(())
    };
    cfg(&[
        "config",
        "merge.coverage-keepours.name",
        "keep ours for generated coverage artifacts",
    ])?;
    cfg(&["config", "merge.coverage-keepours.driver", "true"])?;
    Ok(())
}

fn install_merge_driver() -> StepResult {
    match register_keepours(std::path::Path::new(".")) {
        Ok(()) => StepResult::ok("install-merge-driver")
            .detail("registered merge.coverage-keepours (keep-ours)"),
        Err(e) => StepResult::fail("install-merge-driver").detail(e.to_string()),
    }
}
```

- [ ] **Step 4: Add a keep-ours integration test**

Add a test module (or extend the existing one) in `xtask/src/lib.rs`:

```rust
#[cfg(test)]
mod merge_driver_tests {
    use super::register_keepours;
    use std::process::Command;

    fn git(dir: &std::path::Path, args: &[&str]) {
        let ok = Command::new("git")
            .arg("-C")
            .arg(dir)
            .args(args)
            .status()
            .unwrap()
            .success();
        assert!(ok, "git {args:?} failed");
    }

    #[test]
    fn keepours_driver_resolves_merge_to_ours_without_markers() {
        let tmp = std::env::temp_dir().join(format!("jaunder-mergetest-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&tmp);
        std::fs::create_dir_all(&tmp).unwrap();

        git(&tmp, &["init", "-q"]);
        git(&tmp, &["config", "user.email", "t@t"]);
        git(&tmp, &["config", "user.name", "t"]);
        register_keepours(&tmp).unwrap();
        std::fs::write(
            tmp.join(".gitattributes"),
            "crap-manifest.json merge=coverage-keepours\n",
        )
        .unwrap();
        std::fs::write(tmp.join("crap-manifest.json"), "base\n").unwrap();
        git(&tmp, &["add", "."]);
        git(&tmp, &["commit", "-q", "-m", "base"]);

        git(&tmp, &["checkout", "-q", "-b", "feature"]);
        std::fs::write(tmp.join("crap-manifest.json"), "theirs\n").unwrap();
        git(&tmp, &["commit", "-qam", "theirs"]);

        git(&tmp, &["checkout", "-q", "master"]);
        // Some git defaults to `main`; tolerate either by checking out the first commit's branch.
        std::fs::write(tmp.join("crap-manifest.json"), "ours\n").unwrap();
        git(&tmp, &["commit", "-qam", "ours"]);

        // Merge must succeed (exit 0) and keep "ours" with no conflict markers.
        let merged = Command::new("git")
            .arg("-C")
            .arg(&tmp)
            .args(["merge", "-q", "--no-edit", "feature"])
            .status()
            .unwrap();
        assert!(merged.success(), "keep-ours merge must not conflict");
        let content = std::fs::read_to_string(tmp.join("crap-manifest.json")).unwrap();
        assert_eq!(content, "ours\n", "keep-ours must retain our side");
        assert!(!content.contains("<<<<<<<"), "no conflict markers");

        let _ = std::fs::remove_dir_all(&tmp);
    }
}
```

Note: if `git init` produces a `main` default branch, replace `"master"` with the branch reported by `git -C <tmp> branch --show-current` after the base commit. To be robust, capture it: `let base = String::from_utf8(Command::new("git").arg("-C").arg(&tmp).args(["branch","--show-current"]).output().unwrap().stdout).unwrap().trim().to_string();` right after the base commit, and `git(&tmp, &["checkout","-q",&base]);` instead of the hardcoded `"master"`.

- [ ] **Step 5: Run the test**

Run: `cargo test -p xtask --lib merge_driver_tests`
Expected: PASS.

- [ ] **Step 6: Gate + commit**

```bash
cargo xtask check --no-test
git add .gitattributes xtask/src/lib.rs
git commit --no-verify -m "feat(xtask): keep-ours merge driver for coverage artifacts (#7, #86)"
```

---

### Task 8: Document the new semantics in CONTRIBUTING

**Files:**
- Modify: `CONTRIBUTING.md` (the "Coverage and dependency policy" section, around line 186-194)

- [ ] **Step 1: Extend the coverage policy prose**

After the existing paragraph that begins "The gate does **line-identity** classification…" (line 186) and the auto-heal paragraph (line 188), add:

```markdown
The heal is keyed on **uncovered-text identity**, not line number: a line-shifting change that moves an accepted-uncovered gap (the diff removes it at the old line and it reappears at a new line with identical source text) is recognised as a safe **re-anchor** — `cargo xtask check` re-anchors the baseline and passes, `cargo xtask validate` passes without mutating. Only a *new* uncovered text with no removed-gap counterpart (a genuine lowering) fails the gate. Residual ambiguity: two identical-text lines in one file, where one is removed as an accepted gap while an unrelated identical-text line regresses in the same change, can be conflated as a safe move — bounded, and the line-identity classifier remains the primary signal.

`crap-manifest.json` retains a per-function `line` field as a **non-authoritative jump-to hint**: it can lag the true line until the next change that actually moves a CRAP score (which refreshes every entry's line wholesale). The CRAP regression check and the manifest's rewrite trigger both ignore `line`, so a pure line-shift neither hides a regression nor churns the committed manifest.

The two committed artifacts (`coverage-baseline.json`, `crap-manifest.json`) use a **keep-ours git merge driver** (`.gitattributes`) so overlapping branches do not produce conflict markers; the Fix-mode heal restores authoritative content on the next `cargo xtask check`. Register the driver once per clone/worktree with `cargo xtask install-merge-driver`.
```

- [ ] **Step 2: Commit**

```bash
git add CONTRIBUTING.md
git commit --no-verify -m "docs(contributing): coverage re-anchor + crap line-hint + merge driver (#86, #7)"
```

---

### Task 9: Final gate, artifact reconciliation, and review

**Files:** possibly `coverage-baseline.json` / `crap-manifest.json` (only if the new logic heals them).

- [ ] **Step 1: Run the inner Fix gate to heal any artifact drift**

```bash
cargo xtask check
```

Expected: exit 0. This runs the Nix coverage check and exercises the new engine against the real reports. If it re-anchors the baseline or rewrites the manifest, those files become dirty.

- [ ] **Step 2: Inspect any artifact changes**

```bash
git status --porcelain
git diff --stat coverage-baseline.json crap-manifest.json
```

If either file changed, confirm the diff is a legitimate re-anchor / real-CRAP refresh (not a coverage lowering). If a lowering appears, STOP — that is a real regression to investigate, not to commit.

- [ ] **Step 3: Commit any healed artifacts (if changed)**

```bash
git add coverage-baseline.json crap-manifest.json
git commit --no-verify -m "chore(coverage): re-anchor baseline/manifest under new heal (#86, #7)"
```

- [ ] **Step 4: Run the authoritative gate**

```bash
cargo xtask validate --no-e2e
```

Expected: exit 0 (static + clippy + host xtask tests + Nix coverage, Check mode). Confirm via the sidecar:

```bash
git diff --quiet && echo "tree clean (validate is verify-only)"
```

- [ ] **Step 5: Self-review against the spec**

Confirm each spec deliverable maps to a landed task: #86 predicate (Task 3) + gate/heal (Task 4); #7 ordinal compare (Task 5) + rewrite trigger (Task 6); merge driver (Task 7); ADR (Task 2); docs (Task 8); follow-on issue (Task 1). Request code review (`superpowers:requesting-code-review`) before shipping.

---

## Self-Review (plan author)

- **Spec coverage:** #86 predicate → Task 3; gate/heal/Fix-vs-Check → Task 4; #7 ordinal key → Task 5; line-stripped rewrite trigger + line-as-hint → Task 6; keep-ours `.gitattributes` + one-shot xtask → Task 7; out-of-scope auto-registration → Task 1 (filed); ADR → Task 2; CONTRIBUTING + line-hint note → Task 8. All covered.
- **Placeholder scan:** every code step shows complete code; no TBD/TODO.
- **Type consistency:** `ReanchorSafety { safe, lowering }`, `LineText { file, line, text }`, `reanchor_is_safe(verdict, current, baseline)`, `heal_baseline(safety, …)`, `keyed() -> HashMap<Key,f64>`, `normalize_crap_without_line`, `register_keepours(&Path)` — used consistently across Tasks 3–8.
