# Working-tree-robust coverage gate (untracked files) — Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Closes:** #3 (untracked-file classification) **and** #38 (run the xtask unit suite in the gate) — genuinely coupled: #3's new tests gate nowhere until the xtask suite runs in `check`/`validate`.

**Goal:** Make the coverage gate classify an untracked `.rs` file's uncovered lines as `new_uncovered` (not a misleading `regression`), with no staging/committing required — and make the xtask unit suite actually run in the gate so these tests enforce.

**Architecture:** The Nix coverage build instruments untracked, non-gitignored files, so they appear in the parsed report (`current`), but `git diff <anchor>` omits them, so they get no `LineMap` and fall back to the identity map — mislabeling their uncovered lines as `regression`. Fix: after building the diff-derived maps in `coverage::run_inner`, synthesize an all-added `LineMap` for each untracked `.rs` path that appears in `current`. The classifier is unchanged. Git access is a thin, untested wrapper (repo convention); all logic is pure and unit-tested. Separately, `cargo xtask` never ran xtask's own tests; Task 1 adds an always-run host step so the suite (including #3's new tests) gates in both `check` and `validate`.

**Tech Stack:** Rust (the `xtask` host workspace, `xtask/src/coverage/`). Tests via `cargo test --manifest-path xtask/Cargo.toml`.

## Global Constraints

- **Edit only the `xtask` workspace + `CONTRIBUTING.md`.** Do not touch `flake.nix` (the source-filter purity smell is tracked separately as #37) or the app/`tools` workspaces.
- **No new dependencies.** `xtask/Cargo.toml` deps stay as-is (no `tempfile`, no git-harness crate).
- **Repo convention:** thin git-shelling wrappers are left untested; pure functions get unit tests. Mirror the existing `diff_args` (tested) vs `git_diff_anchor_to_worktree` (untested) split.
- **xtask is its own workspace**, excluded from all Nix derivations and the CI coverage/app gates. Its tests run with `cargo test --manifest-path xtask/Cargo.toml`; its clippy with `cargo clippy --manifest-path xtask/Cargo.toml --all-targets -- -D warnings`.
- **No Co-Authored-By trailers** in commits.
- **Commit cadence:** one clean, verified commit per task. After Task 1, the per-task gate is `cargo xtask check --no-test` (now runs the xtask unit suite + host static/clippy) — use it before each commit. Run the full `cargo xtask validate` once at ship time (handled by jaunder-ship). The standalone `cargo test --manifest-path xtask/Cargo.toml` / xtask clippy commands below remain the fast TDD inner loop within a task.

---

### Task 1: Run the xtask unit suite in the gate (#38)

`cargo xtask` never runs xtask's own `#[cfg(test)]` tests, so the coverage module's suite gates nowhere. Add an always-run host step that runs them in both `check` and `validate`. Doing this first means Tasks 2-4 are immediately enforced by `cargo xtask check --no-test`.

**Files:**
- Create: `xtask/src/steps/host_tests.rs`
- Modify: `xtask/src/lib.rs` (register the module in the `steps` block; call it in both `Check` and `Validate` arms of `run`)

**Interfaces:**
- Produces: `pub fn steps::host_tests::run(sh: &xshell::Shell, result: &mut CommandResult)` — pushes a `"xtask-tests"` step that runs `cargo test --manifest-path xtask/Cargo.toml`.

- [x] **Step 1: Create the host-tests step**

Create `xtask/src/steps/host_tests.rs`:

```rust
use xshell::Shell;

use crate::result::CommandResult;
use crate::sh::step;

/// Run the host-only workspace unit tests that no Nix derivation covers. xtask is
/// its own workspace, excluded from every Nix check, so without this its tests
/// gate nowhere. Fast host suite — runs in every mode (it is NOT the heavy Nix
/// instrumented suite that `--no-test` / `--no-e2e` skip). No coverage here.
pub fn run(sh: &Shell, result: &mut CommandResult) {
    result.push(step(
        sh,
        "xtask-tests",
        "cargo",
        &["test", "--manifest-path", "xtask/Cargo.toml"],
    ));
}
```

- [x] **Step 2: Register and call the module**

In `xtask/src/lib.rs`, add to the `steps` module block:

```rust
mod steps {
    pub mod host_tests;
    pub mod nix;
    pub mod static_checks;
}
```

In `run`, call it (always, before the Nix coverage step) in both arms:

```rust
        Command::Check { no_test } => {
            let sh = xshell::Shell::new()?;
            let start = std::time::Instant::now();
            let mut result = CommandResult::new("check");
            steps::static_checks::run(&sh, Mode::Fix, &mut result);
            steps::host_tests::run(&sh, &mut result);
            if !no_test {
                steps::nix::coverage(&mut result, Mode::Fix);
            }
            finalize(&mut result, start);
            Ok(result)
        }
        Command::Validate { no_e2e } => {
            let sh = xshell::Shell::new()?;
            let start = std::time::Instant::now();
            let mut result = CommandResult::new("validate");
            steps::static_checks::run(&sh, Mode::Check, &mut result);
            steps::host_tests::run(&sh, &mut result);
            steps::nix::coverage(&mut result, Mode::Check);
            if !no_e2e {
                steps::nix::e2e(&mut result);
            }
            finalize(&mut result, start);
            Ok(result)
        }
```

- [x] **Step 3: Verify the step runs and passes**

Run: `cargo xtask check --no-test`
Expected: completes green; the JSON sidecar lists an `xtask-tests` step with `ok: true`. Confirm with:
Run: `cargo run --manifest-path xtask/Cargo.toml -- --json check --no-test` and check the `steps[]` array contains `{"name":"xtask-tests","ok":true,...}` (or inspect `.xtask/last-result.json`).

- [x] **Step 4: Verify it actually gates (negative check)**

Temporarily add a failing test to `xtask/src/coverage/diffmap.rs` tests (e.g. `#[test] fn _gate_probe() { assert_eq!(1, 2); }`), run `cargo xtask check --no-test`, and confirm it now FAILS with the `xtask-tests` step red. Then remove the probe and confirm green again. (Do not commit the probe.)

- [x] **Step 5: Lint**

Run: `cargo clippy --manifest-path xtask/Cargo.toml --all-targets -- -D warnings`
Expected: no warnings.

- [x] **Step 6: Commit**

```bash
git add xtask/src/steps/host_tests.rs xtask/src/lib.rs
git commit -m "feat(xtask): run the xtask unit suite in check/validate (#38)"
```

---

### Task 2: `LineMap::all_added` constructor

Add a real (non-test) constructor for a file with no committed preimage. Today only the test-only `set_added_for_test` exists; the synthesis in Task 2 needs a public way to build an all-added map.

**Files:**
- Modify: `xtask/src/coverage/diffmap.rs` (add associated fn in `impl LineMap`, ~after line 41; add a test in the `tests` module)

**Interfaces:**
- Produces: `LineMap::all_added(lines: impl IntoIterator<Item = u32>) -> LineMap` — a `LineMap` whose `added` set is exactly `lines` (its `map`/`offset_after` stay empty; `map()` is never consulted for untracked files because they have no baseline gaps).

- [x] **Step 1: Write the failing test**

In the `tests` module of `xtask/src/coverage/diffmap.rs` (e.g. after `empty_map_has_no_added_lines`):

```rust
#[test]
fn all_added_marks_every_given_line() {
    let m = LineMap::all_added([3, 7, 10]);
    let mut added: Vec<u32> = m.added_lines().into_iter().collect();
    added.sort();
    assert_eq!(added, vec![3, 7, 10]);
    // An empty input yields no added lines.
    assert!(LineMap::all_added(std::iter::empty()).added_lines().is_empty());
}
```

- [x] **Step 2: Run test to verify it fails**

Run: `cargo test --manifest-path xtask/Cargo.toml all_added_marks_every_given_line`
Expected: FAIL — `no function or associated item named `all_added` found for struct `LineMap``.

- [x] **Step 3: Write minimal implementation**

In `impl LineMap` (after the `map`/`added_lines` methods, before the `#[cfg(test)]` helpers):

```rust
/// Build a map for a file with no committed preimage (e.g. an untracked
/// file): every given new-side line number is "added". The classifier uses
/// `added_lines()` to bucket an untracked file's uncovered lines as
/// `new_uncovered` rather than `regression`. `map()` is irrelevant here —
/// an untracked file has no baseline gaps, so it is never called.
pub fn all_added(lines: impl IntoIterator<Item = u32>) -> LineMap {
    LineMap {
        added: lines.into_iter().collect(),
        ..LineMap::default()
    }
}
```

- [x] **Step 4: Run test to verify it passes**

Run: `cargo test --manifest-path xtask/Cargo.toml all_added_marks_every_given_line`
Expected: PASS.

- [x] **Step 5: Lint**

Run: `cargo clippy --manifest-path xtask/Cargo.toml --all-targets -- -D warnings`
Expected: no warnings.

- [x] **Step 6: Commit**

```bash
git add xtask/src/coverage/diffmap.rs
git commit -m "feat(xtask): add LineMap::all_added for files with no committed preimage"
```

---

### Task 3: Synthesize maps for untracked files and wire into the gate

Enumerate untracked `.rs` files and, for each that appears in the coverage report, install an all-added map so its uncovered lines classify as `new_uncovered`.

**Files:**
- Modify: `xtask/src/coverage/mod.rs`
  - Add `use std::collections::{HashMap, HashSet};` and `use crate::coverage::diffmap::LineMap;` to the existing imports (top of file, alongside `use std::process::Command;`).
  - Add three functions: `parse_untracked_list`, `untracked_rs_files`, `synthesize_untracked_maps` (place them near `git_diff_anchor_to_worktree`/`diff_args`, ~after line 295).
  - In `run_inner`: change `let maps = diffmap::parse_unified_diff(&diff);` (line 158) to `let mut maps = …` and add the synthesis call on the next line.
  - Add tests to the `tests` module.

**Interfaces:**
- Consumes: `LineMap::all_added` (Task 2); existing `FileCoverage { path: String, lines: Vec<LineCov> }` and `LineCov { line: u32, .. }`.
- Produces (all private to the module):
  - `fn parse_untracked_list(stdout: &str) -> Vec<String>` — split NUL-delimited git output, dropping empties.
  - `fn untracked_rs_files() -> anyhow::Result<Vec<String>>` — thin git wrapper (untested).
  - `fn synthesize_untracked_maps(maps: &mut HashMap<String, LineMap>, current: &[FileCoverage], untracked: &[String])` — for each `current` file whose path is in `untracked` and has no existing map, insert `LineMap::all_added` over that file's reported line numbers.

- [x] **Step 1: Write the failing tests**

In the `tests` module of `xtask/src/coverage/mod.rs` (it already has the `fc` helper and `use crate::coverage::baseline::Baseline;`). Add at the top of the test module body: `use crate::coverage::diffmap::{self, LineMap};` and `use std::collections::HashMap;` if not already in scope via `use super::*;`.

```rust
#[test]
fn parse_untracked_list_splits_nul_and_drops_empties() {
    assert_eq!(
        parse_untracked_list("a.rs\0server/src/b.rs\0"),
        vec!["a.rs".to_string(), "server/src/b.rs".to_string()]
    );
    assert!(parse_untracked_list("").is_empty());
}

#[test]
fn synthesizes_all_added_map_for_untracked_file_in_report() {
    let current = vec![fc("server/src/new.rs", &[(1, true), (2, false)])];
    let mut maps: HashMap<String, LineMap> = HashMap::new();
    synthesize_untracked_maps(&mut maps, &current, &["server/src/new.rs".to_string()]);
    let m = maps.get("server/src/new.rs").expect("synthesized map");
    let mut added: Vec<u32> = m.added_lines().into_iter().collect();
    added.sort();
    assert_eq!(added, vec![1, 2]);
}

#[test]
fn does_not_synthesize_for_a_file_not_in_the_untracked_list() {
    let current = vec![fc("tracked.rs", &[(1, false)])];
    let mut maps: HashMap<String, LineMap> = HashMap::new();
    synthesize_untracked_maps(&mut maps, &current, &[]);
    assert!(maps.get("tracked.rs").is_none(), "tracked file gets no synthesized map");
}

#[test]
fn does_not_overwrite_an_existing_diff_map() {
    let current = vec![fc("u.rs", &[(5, false)])];
    let mut maps: HashMap<String, LineMap> = HashMap::new();
    maps.insert("u.rs".to_string(), diffmap::empty_map()); // already mapped by the anchor diff
    synthesize_untracked_maps(&mut maps, &current, &["u.rs".to_string()]);
    assert!(
        maps.get("u.rs").unwrap().added_lines().is_empty(),
        "an existing diff map must be preserved, not replaced by all-added"
    );
}

#[test]
fn untracked_uncovered_line_classifies_as_new_uncovered_not_regression() {
    // The end-to-end logic (minus the git shell): an untracked file's uncovered
    // line must be new_uncovered, never a phantom regression.
    let current = vec![fc("new.rs", &[(1, true), (2, false)])];
    let mut maps: HashMap<String, LineMap> = HashMap::new();
    synthesize_untracked_maps(&mut maps, &current, &["new.rs".to_string()]);
    let verdict = classify::classify(&current, &Baseline::default(), &maps);
    assert_eq!(
        verdict.new_uncovered,
        vec![FileLines { file: "new.rs".into(), lines: vec![2] }]
    );
    assert!(verdict.regressions.is_empty(), "untracked new code must not be a regression");
}
```

- [x] **Step 2: Run tests to verify they fail**

Run: `cargo test --manifest-path xtask/Cargo.toml --lib coverage::tests`
Expected: FAIL — `cannot find function `parse_untracked_list`` / `synthesize_untracked_maps` in this scope.

- [x] **Step 3: Add the functions**

Add imports at the top of `xtask/src/coverage/mod.rs` (with the existing `use std::process::Command;`):

```rust
use std::collections::{HashMap, HashSet};

use crate::coverage::diffmap::LineMap;
```

Add the three functions near `git_diff_anchor_to_worktree` (~after line 295):

```rust
/// Parse `git ls-files --others --exclude-standard -z` output: NUL-delimited
/// repo-root-relative paths. Empty entries (e.g. the trailing NUL) are dropped.
fn parse_untracked_list(stdout: &str) -> Vec<String> {
    stdout
        .split('\0')
        .filter(|s| !s.is_empty())
        .map(str::to_string)
        .collect()
}

/// Untracked, non-gitignored `.rs` files in the working tree (repo-root-relative).
/// The Nix coverage build instruments these (its source includes untracked,
/// non-gitignored files), but `git diff <anchor>` omits them — so they need a
/// synthesized all-added map. Thin git wrapper; the logic is in
/// `parse_untracked_list` / `synthesize_untracked_maps`.
fn untracked_rs_files() -> Result<Vec<String>> {
    let out = Command::new("git")
        .args([
            "ls-files",
            "--others",
            "--exclude-standard",
            "-z",
            "--",
            "*.rs",
        ])
        .output()
        .context("running git ls-files --others (untracked .rs files)")?;
    Ok(parse_untracked_list(&String::from_utf8_lossy(&out.stdout)))
}

/// For each untracked file that actually appears in the coverage report, install
/// an all-added `LineMap` over its reported line numbers, so its uncovered lines
/// classify as `new_uncovered` instead of a phantom `regression`. Files already
/// carrying a diff-derived map (they appeared in the anchor diff) are left alone;
/// untracked files absent from the report (not compiled) are ignored.
fn synthesize_untracked_maps(
    maps: &mut HashMap<String, LineMap>,
    current: &[FileCoverage],
    untracked: &[String],
) {
    let untracked: HashSet<&str> = untracked.iter().map(String::as_str).collect();
    for f in current {
        if untracked.contains(f.path.as_str()) && !maps.contains_key(&f.path) {
            let lines = f.lines.iter().map(|l| l.line);
            maps.insert(f.path.clone(), LineMap::all_added(lines));
        }
    }
}
```

- [x] **Step 4: Wire synthesis into `run_inner`**

In `run_inner`, change line 158 and add the synthesis call:

```rust
    let mut maps = diffmap::parse_unified_diff(&diff);
    synthesize_untracked_maps(&mut maps, &current, &untracked_rs_files()?);
```

(`classify::classify(&current, &baseline, &maps)` on the next line is unchanged.)

- [x] **Step 5: Run tests to verify they pass**

Run: `cargo test --manifest-path xtask/Cargo.toml --lib coverage::`
Expected: PASS (the four new tests plus all existing coverage tests).

- [x] **Step 6: Lint**

Run: `cargo clippy --manifest-path xtask/Cargo.toml --all-targets -- -D warnings`
Expected: no warnings.

- [x] **Step 7: Commit**

```bash
git add xtask/src/coverage/mod.rs
git commit -m "fix(xtask): classify untracked-file uncovered lines as new, not regression (#3)"
```

---

### Task 4: Document the working-tree contract

CONTRIBUTING.md does not currently describe how the gate treats working-tree state. Add a short contract to the coverage section.

**Files:**
- Modify: `CONTRIBUTING.md` (the "Coverage and dependency policy" section — after the line-identity ratchet description, ~line 186)

- [x] **Step 1: Read the section**

Read `CONTRIBUTING.md` around the "Coverage and dependency policy" heading (~lines 181-198) to confirm the exact insertion point (immediately after the sentence describing the line-identity ratchet / Fix-vs-Check behavior).

- [x] **Step 2: Insert the working-tree contract**

Add this paragraph after the ratchet/Fix-vs-Check description:

```markdown
**Working-tree contract.** The gate reflects your *working tree*, not just
committed state: the Nix coverage build instruments dirty tracked content **and**
untracked, non-gitignored files. So you do **not** need to commit or stage first —
line-shifting edits to tracked files are mapped from the baseline anchor to the
working tree, and a new untracked `.rs` file is measured and its uncovered lines
are reported as new uncovered code (not as a regression). (The source filter that
pulls untracked files into the build is a known purity rough edge, tracked in
issue #37.)
```

- [x] **Step 3: Verify no stale "commit/stage first" guidance contradicts it**

Run: `rg -n -i "commit.*first|stage.*first|before running the (gate|coverage)" CONTRIBUTING.md`
Expected: no surviving instruction telling contributors they must commit/stage before the gate to avoid phantom regressions. If any is found, reconcile it with the new contract (remove or correct it).

- [x] **Step 4: Commit**

```bash
git add CONTRIBUTING.md
git commit -m "docs: document the coverage gate's working-tree contract (#3)"
```

---

## Final gate (ship time)

After all tasks, before the PR (jaunder-ship):

- [x] Run the full gate on the live repo: `cargo xtask validate`. This is the only step that exercises `untracked_rs_files()` against real git state. Expected: green. **Done — PASSED in 719s (e2e VMs + coverage clean + xtask-tests green).**

## Self-Review

- **Spec coverage:** ① untracked → `new_uncovered` (robust approach) → Task 3. ② `LineMap::all_added` → Task 2. ③ `untracked_rs_files` enumeration + `run_inner` wiring → Task 3. ④ heal/baseline reproducibility "no change by construction" → no code needed; the `untracked_uncovered_line_classifies_as_new_uncovered_not_regression` test confirms uncovered untracked → not clean → (existing) no-heal path holds. ⑤ unit tests (classify all-added case, diffmap constructor) → Tasks 2-3; the git-boundary fallback (pure `parse_untracked_list`/`synthesize_untracked_maps` + live `validate`) is the spec-sanctioned alternative to a temp-git test. ⑥ CONTRIBUTING contract → Task 4. ⑦ flake purity (#37) and #7/#10 out of scope — untouched. ⑧ #38 (run xtask suite in the gate; the precondition that makes ⑤ enforce) → Task 1, with a negative-control step proving it actually gates.
- **Placeholder scan:** none — every code/test step shows complete code; commands have expected output.
- **Type consistency:** `all_added(impl IntoIterator<Item = u32>)`, `synthesize_untracked_maps(&mut HashMap<String, LineMap>, &[FileCoverage], &[String])`, `parse_untracked_list(&str) -> Vec<String>`, `untracked_rs_files() -> Result<Vec<String>>` are used identically in their tests and call site. `FileCoverage.path`/`.lines`, `LineCov.line`, `FileLines { file, lines }`, `classify::classify`, `Baseline::default()` match the existing module.
