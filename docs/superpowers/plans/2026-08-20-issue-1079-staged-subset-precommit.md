# Staged-Subset-Safe Precommit Gate Implementation Plan

> **For agentic workers:** Execute this plan task-by-task with `jaunder-iterate`
> (delegating individual tasks to a subagent via `jaunder-dispatch` when
> useful). Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Replace the pre-commit hook's stale-index-prone `cargo xtask check`
wrapper with an explicit `cargo xtask precommit` command that runs the fast gate
and safe-stages only provably intended tracked formatter fixes.

**Architecture:** Keep the gate ladder intact: `check` remains the developer
Fix-mode command, `validate --no-e2e` remains the clean-tree pre-push proof, and
only the new `precommit` command owns Git/index mutation. Git status parsing and
auto-staging policy live in `xtask/src/git.rs`; command orchestration in
`xtask/src/lib.rs` runs the existing host gate surface and then applies the Git
policy.

**Tech Stack:** Rust 2024 (`xtask`), Clap subcommands, throwaway Git fixture
repos, Bash hooks, ADR/docs markdown.

## Review header

**Scope — in:** `cargo xtask precommit`; reusable host-gate runner shared by
`check`/`validate`/`precommit`; porcelain status parser and safe-staging policy;
pre-commit hook reroute; ADR-0029 and architecture amendments; unit/fixture
coverage for the stale-index and unsafe-staging cases.

**Scope — out:** result-stamp or checked-tree cache; any Nix/e2e gate change;
pre-push weakening; generic support for delete/rename auto-staging; hook-mode
detection inside `cargo xtask check`; banning staged-subset commits.

**Tasks:**

1. Add Git status parsing plus safe-staging policy/fixture tests.
2. Add `cargo xtask precommit`, reuse the host gate, and reroute the pre-commit
   hook.
3. Amend gate documentation and run the focused contracts plus the per-commit
   gate.

**Key risks / decisions:** Use `git status --porcelain` parsing in one helper,
not ad-hoc shell comparisons. Pre-existing untracked files are tolerated when
still untracked after the gate; newly-created untracked files fail. Any delete
or rename status fails closed. `precommit` may run on a dirty worktree by
design; only `validate --no-e2e` proves the committed tip is clean. If the host
gate itself fails, `precommit` still evaluates staging safety after the run so
safe formatter fixes on already-staged paths are not left as stale-index traps,
but the overall command remains failed.

## Global Constraints

- Implement
  [the approved specification](../specs/2026-08-20-issue-1079-staged-subset-precommit.md),
  especially D1-D8 and AC1-AC12.
- `cargo xtask precommit` runs the same non-Nix test surface as
  `cargo xtask check --no-test`: host static checks, repo-shape gates,
  type-safety gates, and host tests; no `wasm-tests`, `coverage`, `doctests`, or
  e2e.
- `cargo xtask check` and `cargo xtask check --no-test` never stage files.
- `.githooks/pre-push` stays `cargo xtask validate --no-e2e`; `validate` keeps
  the default dirty-tree refusal.
- Never use `git add .` / `git add -A` in the implementation. Auto-staging calls
  `git add -- <path>` for each approved tracked path only.
- Commit each task after `devtool run -- cargo xtask check`; stage the checked
  tree before commit; no `Co-Authored-By` trailer.

---

## File structure

- Modify `xtask/src/git.rs` — add typed porcelain parsing, tracked worktree
  fingerprints, before/after precommit reconciliation, and path-specific
  `git add -- <path>` staging.
- Modify `xtask/src/lib.rs` — add `Command::Precommit`, factor the host gate
  surface out of `check`/`validate`, run reconciliation after the host gate, and
  report it as a named step.
- Modify `.cargo/config.toml` — keep the `xtask` alias locked so Cargo cannot
  rewrite `xtask/Cargo.lock` before `precommit` snapshots Git state.
- Modify `.githooks/pre-commit` — preserve `SKIP_PRE_COMMIT=1`, then
  `exec cargo xtask precommit`.
- Leave `.githooks/pre-push` behavior unchanged; only comments/docs may mention
  it.
- Modify `docs/adr/0029-git-enforced-verify-gate.md` — amend the accepted ADR to
  describe `precommit`, fast hook surface, and safe-staging policy.
- Modify `docs/ARCHITECTURE.md` — update the verify-gate projection to match the
  new command split.

## Interfaces and contracts

```rust
// xtask/src/git.rs
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GitStatusSnapshot {
    pub paths: std::collections::BTreeMap<String, GitPathStatus>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GitPathStatus {
    pub staged: bool,
    pub unstaged: bool,
    pub untracked: bool,
    pub delete_or_rename: bool,
    pub worktree_fingerprint: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PrecommitStagePlan {
    pub stage_paths: Vec<String>,
    pub failures: Vec<String>,
}

pub fn status_snapshot(dir: &std::path::Path) -> anyhow::Result<GitStatusSnapshot>;
pub fn parse_status_snapshot(porcelain: &str) -> GitStatusSnapshot;
pub fn precommit_stage_plan(
    before: &GitStatusSnapshot,
    after: &GitStatusSnapshot,
) -> PrecommitStagePlan;
pub fn apply_precommit_stage_plan(
    dir: &std::path::Path,
    plan: &PrecommitStagePlan,
) -> crate::StepResult;
```

Status parsing contract:

- `"M  src/a.rs"` => `staged=true`, `unstaged=false`, `untracked=false`.
- `" M src/a.rs"` => `staged=false`, `unstaged=true`, `untracked=false`.
- `"MM src/a.rs"` => `staged=true`, `unstaged=true`, `untracked=false`.
- `"?? scratch.rs"` => `untracked=true`, not staged/unstaged.
- Any `D` or `R` in either status column sets `delete_or_rename=true` for that
  path.
- Rename display paths may stay as Git's raw `old -> new` payload for
  diagnostics; they must not be auto-staged.
- Repeated records for the same path are merged by OR-ing status booleans and
  preserving `delete_or_rename=true`; a delete/recreate shape such as `D  a.rs`
  plus `?? a.rs` must remain unsafe, not get overwritten by the later untracked
  record.
- `parse_status_snapshot` sets `worktree_fingerprint=None`; `status_snapshot`
  fills it for every tracked, non-delete path by hashing the current worktree
  file content. This is load-bearing: status text alone cannot detect a gate
  rewrite that leaves a file in the same ` M` or `MM` class.

Precommit reconciliation contract:

- A tracked path counts as changed by the gate when either its status flags or
  its `worktree_fingerprint` differ between `before` and `after`, including the
  case where it existed in `before` and disappears from `after`.
- A path becomes a `stage_paths` entry only when it is tracked, staged in
  `before`, not unstaged in `before`, changed by the gate, and not a
  delete/rename status in either snapshot.
- A tracked path that changed by the gate and was not staged in `before` becomes
  a failure, including the clean-before case where the path is absent from the
  before snapshot. Message contains the path and
  `will not add work the user did not stage`.
- A path becomes a failure when it changed by the gate and was both staged and
  unstaged in `before`: message contains the path and `pre-existing mixed`.
- A path becomes a failure when it is untracked in `after` and absent from
  `before`: message contains the path and `new untracked`.
- A path becomes a failure when either snapshot has `delete_or_rename=true`:
  message contains the path and `delete/rename`.
- A path that is untracked in both snapshots is ignored.
- A path with unrelated unstaged tracked work in both snapshots is ignored if
  its `worktree_fingerprint` did not change.

`apply_precommit_stage_plan` contract:

- Run `git add -- <path>` once per `stage_paths` entry before returning any
  recorded failure. This keeps safe formatter/check fixes staged even when an
  unrelated unsafe path makes the hook fail.
- If any staging command errors, append that error to the failure list.
- If the final failure list is non-empty, return
  `StepResult::fail("precommit-staging")` with newline-joined failure strings.
- If `stage_paths` is empty, return `StepResult::ok("precommit-staging")` with
  detail `no staged fixes`.
- Otherwise return `StepResult::ok("precommit-staging")` with detail
  `staged: a, b`.

### Task 1: Git status parsing and safe-staging policy

**Files:**

- Modify: `xtask/src/git.rs:1-240`
- Test: `xtask/src/git.rs:242-411`

**Interfaces:**

- Produces the `GitStatusSnapshot`, `GitPathStatus`, `PrecommitStagePlan`,
  `status_snapshot`, `parse_status_snapshot`, `precommit_stage_plan`, and
  `apply_precommit_stage_plan` interfaces listed above.
- Consumes existing `git::output`, `git::run`,
  `crate::test_support::{commit, git_ok, write}`, and `crate::StepResult`.

- [x] **Step 1: Write parser unit tests**

Add these tests under `xtask/src/git.rs`'s existing `#[cfg(test)] mod tests`:

```rust
#[test]
fn precommit_status_parser_classifies_index_worktree_and_untracked() {
    let snap = parse_status_snapshot("M  src/a.rs\n M src/b.rs\nMM src/c.rs\n?? scratch.rs\n");
    assert_eq!(snap.paths["src/a.rs"].staged, true);
    assert_eq!(snap.paths["src/a.rs"].unstaged, false);
    assert_eq!(snap.paths["src/b.rs"].staged, false);
    assert_eq!(snap.paths["src/b.rs"].unstaged, true);
    assert_eq!(snap.paths["src/c.rs"].staged, true);
    assert_eq!(snap.paths["src/c.rs"].unstaged, true);
    assert_eq!(snap.paths["scratch.rs"].untracked, true);
}

#[test]
fn precommit_status_parser_marks_delete_and_rename_unsafe() {
    let snap = parse_status_snapshot("D  gone.rs\n D missing.rs\nR  old.rs -> new.rs\n");
    assert!(snap.paths["gone.rs"].delete_or_rename);
    assert!(snap.paths["missing.rs"].delete_or_rename);
    assert!(snap.paths["old.rs -> new.rs"].delete_or_rename);
}
```

- [x] **Step 2: Run parser tests and verify they fail**

Run:

```bash
cargo nextest run --manifest-path xtask/Cargo.toml precommit_status_parser
```

Expected: FAIL because the new parser/types do not exist.

- [x] **Step 3: Implement parser/types**

Add the four structs and `parse_status_snapshot`/`status_snapshot` exactly as in
`Interfaces and contracts`. Implementation detail that tests cannot fully pin:
parse porcelain v1 using `let bytes = line.as_bytes()` with length checks for
the two status columns, then a safe `line.get(3..).unwrap_or("")` path slice
before trimming; ignore blank lines. Treat `?`/`?` as untracked. Treat any
status-column `D` or `R` as `delete_or_rename`. `status_snapshot` calls
`git status --porcelain --untracked-files=all --find-renames`, parses it, then
fills `worktree_fingerprint` for each tracked, non-delete/rename path with
`git hash-object -- <path>`.

- [x] **Step 4: Run parser tests and verify they pass**

Run:

```bash
cargo nextest run --manifest-path xtask/Cargo.toml precommit_status_parser
```

Expected: PASS.

- [x] **Step 5: Write reconciliation unit tests**

Add these pure-plan tests. Use a small local helper in the test module to set
`worktree_fingerprint` values without invoking Git:

```rust
fn fp(mut snap: GitStatusSnapshot, path: &str, value: &str) -> GitStatusSnapshot {
    snap.paths
        .get_mut(path)
        .unwrap()
        .worktree_fingerprint = Some(value.to_string());
    snap
}

#[test]
fn precommit_stage_plan_stages_only_clean_previously_staged_tracked_paths() {
    let before = fp(
        fp(parse_status_snapshot("M  a.rs\n M b.rs\n?? scratch.rs\n"), "a.rs", "old-a"),
        "b.rs",
        "old-b",
    );
    let after = fp(
        fp(parse_status_snapshot("MM a.rs\n M b.rs\n?? scratch.rs\n"), "a.rs", "new-a"),
        "b.rs",
        "old-b",
    );
    let plan = precommit_stage_plan(&before, &after);
    assert_eq!(plan.stage_paths, vec!["a.rs".to_string()]);
    assert!(plan.failures.is_empty());
}

#[test]
fn precommit_stage_plan_rejects_mixed_and_unstaged_only_mutations() {
    let before = fp(
        fp(parse_status_snapshot("MM mixed.rs\n M unstaged.rs\n"), "mixed.rs", "old-mixed"),
        "unstaged.rs",
        "old-unstaged",
    );
    let after = fp(
        fp(parse_status_snapshot("MM mixed.rs\n M unstaged.rs\n"), "mixed.rs", "new-mixed"),
        "unstaged.rs",
        "new-unstaged",
    );
    let plan = precommit_stage_plan(&before, &after);
    assert!(plan.stage_paths.is_empty());
    assert!(plan.failures.iter().any(|f| f.contains("mixed.rs") && f.contains("pre-existing mixed")));
    assert!(plan.failures.iter().any(|f| f.contains("unstaged.rs") && f.contains("will not add work the user did not stage")));
}

#[test]
fn precommit_stage_plan_rejects_clean_before_tracked_mutation() {
    let before = parse_status_snapshot("");
    let after = fp(parse_status_snapshot(" M clean.rs\n"), "clean.rs", "new-clean");
    let plan = precommit_stage_plan(&before, &after);
    assert!(plan.stage_paths.is_empty());
    assert!(plan.failures.iter().any(|f| f.contains("clean.rs") && f.contains("will not add work the user did not stage")));
}

#[test]
fn precommit_stage_plan_tolerates_old_untracked_and_rejects_new_untracked() {
    let before = parse_status_snapshot("?? old.tmp\n");
    let after = parse_status_snapshot("?? old.tmp\n?? new.tmp\n");
    let plan = precommit_stage_plan(&before, &after);
    assert!(plan.stage_paths.is_empty());
    assert_eq!(plan.failures.len(), 1);
    assert!(plan.failures[0].contains("new.tmp"));
    assert!(plan.failures[0].contains("new untracked"));
}

#[test]
fn precommit_stage_plan_rejects_delete_or_rename_states() {
    let before = parse_status_snapshot("M  keep.rs\n");
    let after = parse_status_snapshot("D  keep.rs\nR  old.rs -> new.rs\n");
    let plan = precommit_stage_plan(&before, &after);
    assert!(plan.stage_paths.is_empty());
    assert!(plan.failures.iter().any(|f| f.contains("keep.rs") && f.contains("delete/rename")));
    assert!(plan.failures.iter().any(|f| f.contains("old.rs -> new.rs") && f.contains("delete/rename")));
}

#[test]
fn precommit_stage_plan_preserves_delete_recreate_as_delete_rename() {
    let after = parse_status_snapshot("D  a.rs\n?? a.rs\n");
    assert!(after.paths["a.rs"].delete_or_rename);
    let plan = precommit_stage_plan(&parse_status_snapshot(""), &after);
    assert!(plan.stage_paths.is_empty());
    assert!(plan.failures.iter().any(|f| f.contains("a.rs") && f.contains("delete/rename")));
}
```

- [x] **Step 6: Run reconciliation tests and verify they fail**

Run:

```bash
cargo nextest run --manifest-path xtask/Cargo.toml precommit_stage_plan
```

Expected: FAIL because `precommit_stage_plan` is not implemented.

- [x] **Step 7: Implement reconciliation**

Implement `precommit_stage_plan` exactly to the contract above. Compare the
`before` and `after` snapshots by path and by `worktree_fingerprint`, not just
by status class. Sort `stage_paths` and `failures` by path for deterministic
output.

- [x] **Step 8: Run reconciliation tests and verify they pass**

Run:

```bash
cargo nextest run --manifest-path xtask/Cargo.toml precommit_stage_plan
```

Expected: PASS.

- [x] **Step 9: Write executable Git fixture tests**

Add fixture tests that call `status_snapshot`, `precommit_stage_plan`, and
`apply_precommit_stage_plan` against real temp repos:

```rust
#[test]
fn precommit_apply_restages_only_the_previously_staged_file() {
    let dir = temp_repo("precommit-restage");
    commit(&dir, "a.rs", "fn a(){}\n");
    commit(&dir, "b.rs", "fn b(){}\n");

    write(&dir, "a.rs", "fn a() { }\n");
    git_ok(&dir, &["add", "a.rs"]);
    write(&dir, "b.rs", "fn b() { }\n");
    let before = status_snapshot(&dir).unwrap();

    write(&dir, "a.rs", "fn a() { }\n// formatted\n");
    let after = status_snapshot(&dir).unwrap();
    let plan = precommit_stage_plan(&before, &after);
    let step = apply_precommit_stage_plan(&dir, &plan);

    assert!(step.ok, "{step:?}");
    assert_eq!(output(&dir, &["diff", "--cached", "--name-only"]).unwrap(), "a.rs");
    assert_eq!(output(&dir, &["diff", "--name-only"]).unwrap(), "b.rs");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn precommit_apply_refuses_mixed_staged_unstaged_file() {
    let dir = temp_repo("precommit-mixed");
    commit(&dir, "a.rs", "one\n");
    write(&dir, "a.rs", "two\n");
    git_ok(&dir, &["add", "a.rs"]);
    write(&dir, "a.rs", "three\n");
    let before = status_snapshot(&dir).unwrap();

    write(&dir, "a.rs", "four\n");
    let after = status_snapshot(&dir).unwrap();
    let plan = precommit_stage_plan(&before, &after);
    let step = apply_precommit_stage_plan(&dir, &plan);

    assert!(!step.ok);
    assert!(step.detail.as_deref().unwrap().contains("pre-existing mixed"));
    assert_eq!(output(&dir, &["diff", "--cached", "--name-only"]).unwrap(), "a.rs");
    assert_eq!(output(&dir, &["diff", "--name-only"]).unwrap(), "a.rs");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn precommit_apply_refuses_unstaged_only_file() {
    let dir = temp_repo("precommit-unstaged");
    commit(&dir, "a.rs", "one\n");
    let before = status_snapshot(&dir).unwrap();

    write(&dir, "a.rs", "two\n");
    let after = status_snapshot(&dir).unwrap();
    let plan = precommit_stage_plan(&before, &after);
    let step = apply_precommit_stage_plan(&dir, &plan);

    assert!(!step.ok);
    assert!(step.detail.as_deref().unwrap().contains("will not add work the user did not stage"));
    assert!(output(&dir, &["diff", "--cached", "--name-only"]).unwrap().is_empty());
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn precommit_apply_refuses_new_untracked_file_without_staging_old_untracked() {
    let dir = temp_repo("precommit-untracked");
    commit(&dir, "tracked.rs", "one\n");
    write(&dir, "old.tmp", "old\n");
    let before = status_snapshot(&dir).unwrap();

    write(&dir, "new.tmp", "new\n");
    let after = status_snapshot(&dir).unwrap();
    let plan = precommit_stage_plan(&before, &after);
    let step = apply_precommit_stage_plan(&dir, &plan);

    assert!(!step.ok);
    assert!(step.detail.as_deref().unwrap().contains("new untracked"));
    assert!(output(&dir, &["diff", "--cached", "--name-only"]).unwrap().is_empty());
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn precommit_apply_refuses_new_file_inside_preexisting_untracked_dir() {
    let dir = temp_repo("precommit-untracked-dir");
    commit(&dir, "tracked.rs", "one\n");
    write(&dir, "scratch/old.tmp", "old\n");
    let before = status_snapshot(&dir).unwrap();

    write(&dir, "scratch/new.tmp", "new\n");
    let after = status_snapshot(&dir).unwrap();
    let plan = precommit_stage_plan(&before, &after);
    let step = apply_precommit_stage_plan(&dir, &plan);

    assert!(!step.ok);
    assert!(step.detail.as_deref().unwrap().contains("scratch/new.tmp"));
    assert!(step.detail.as_deref().unwrap().contains("new untracked"));
    assert!(output(&dir, &["diff", "--cached", "--name-only"]).unwrap().is_empty());
    let _ = std::fs::remove_dir_all(&dir);
}
```

- [x] **Step 10: Run fixture tests and verify they fail**

Run:

```bash
cargo nextest run --manifest-path xtask/Cargo.toml precommit_apply
```

Expected: FAIL because `apply_precommit_stage_plan` is not implemented.

- [x] **Step 11: Implement `apply_precommit_stage_plan`**

Implement the exact `StepResult` contract. Use
`git::run(dir, &["add", "--", path.as_str()])` or an equivalent helper that
includes `--`. Never call `git add .` or `git add -A`.

- [x] **Step 12: Run all Git policy tests and verify they pass**

Run:

```bash
cargo nextest run --manifest-path xtask/Cargo.toml precommit
```

Expected: PASS.

- [x] **Step 13: Run the per-commit gate and commit**

Run:

```bash
devtool run -- cargo xtask check
```

Expected: PASS. Stage only `xtask/src/git.rs` plus any gate-applied mechanical
fixes to that file, then commit:

```bash
git add xtask/src/git.rs
git commit -m "fix(xtask): model safe precommit staging"
```

### Task 2: `cargo xtask precommit` command and hook routing

**Files:**

- Modify: `xtask/src/lib.rs:104-128,467-556,853-874,877-983`
- Modify: `.cargo/config.toml:4-5`
- Modify: `.githooks/pre-commit:1-29`
- Test: `xtask/src/lib.rs:877-983`

**Interfaces:**

- Consumes Task 1's `status_snapshot`, `precommit_stage_plan`, and
  `apply_precommit_stage_plan`.
- The `cargo xtask` alias includes Cargo's `--locked` flag before the binary
  argument separator, so no Cargo-controlled lockfile write can happen before
  `run_precommit_with_host_gate` takes its `before` snapshot.
- Produces `Command::Precommit` with no flags.
- Produces
  `fn run_host_gate(sh: &xshell::Shell, mode: Mode, result: &mut CommandResult)`
  inside `xtask/src/lib.rs`, used by `check` and `precommit`; `validate` reuses
  the non-test host surface and then preserves its pre-existing
  `wasm-budget`-before-`host-tests` order.

- [x] **Step 1: Write CLI and host-surface tests**

Add tests under `cli_tests`:

```rust
#[test]
fn precommit_parses_as_first_class_subcommand() {
    let cli = Cli::try_parse_from(["xtask", "precommit"]).unwrap();
    match cli.command {
        Command::Precommit => {}
        _ => panic!("expected precommit"),
    }
}

#[test]
fn precommit_does_not_replace_check_no_test_parse() {
    let cli = Cli::try_parse_from(["xtask", "check", "--no-test"]).unwrap();
    match cli.command {
        Command::Check { no_test } => assert!(no_test),
        _ => panic!("expected check"),
    }
}
```

Add a host-surface regression by extracting the repeated host steps into a pure
helper if needed:

```rust
#[test]
fn precommit_host_surface_is_check_no_test_surface() {
    let check = host_gate_step_names_for_test(Mode::Fix);
    let precommit = precommit_host_step_names_for_test();
    assert_eq!(precommit, check);
    assert!(!precommit.contains(&"nix-wasm-tests"));
    assert!(!precommit.contains(&"nix-coverage"));
    assert!(!precommit.contains(&"nix-doctests"));
}
```

The test-only helpers may be `#[cfg(test)]` and should return
`Vec<&'static str>` from the same ordered source that `run_host_gate` executes;
do not duplicate an expected hand-written list that can drift.

Add an orchestration seam that proves `Command::Precommit` uses the Task 1
snapshot/reconcile/apply path, not just the same host-step list:

```rust
#[test]
fn precommit_orchestration_restages_safe_fixture() {
    let dir = crate::test_support::temp_repo("precommit", "orchestration-safe");
    crate::test_support::commit(&dir, "a.rs", "fn a(){}\\n");
    crate::test_support::commit(&dir, "b.rs", "fn b(){}\\n");
    crate::test_support::write(&dir, "a.rs", "fn a() { }\\n");
    crate::test_support::git_ok(&dir, &["add", "a.rs"]);
    crate::test_support::write(&dir, "b.rs", "fn b() { }\\n");

    let result = run_precommit_with_host_gate(&dir, |result| {
        crate::test_support::write(&dir, "a.rs", "fn a() { }\\n// formatted\\n");
        result.push(StepResult::ok("fake-host-gate"));
    })
    .unwrap();

    assert!(result.ok);
    assert_eq!(git::output(&dir, &["diff", "--cached", "--name-only"]).unwrap(), "a.rs");
    assert_eq!(git::output(&dir, &["diff", "--name-only"]).unwrap(), "b.rs");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn precommit_orchestration_fails_clean_before_tracked_mutation() {
    let dir = crate::test_support::temp_repo("precommit", "orchestration-unsafe");
    crate::test_support::commit(&dir, "clean.rs", "one\\n");

    let result = run_precommit_with_host_gate(&dir, |result| {
        crate::test_support::write(&dir, "clean.rs", "two\\n");
        result.push(StepResult::ok("fake-host-gate"));
    })
    .unwrap();

    assert!(!result.ok);
    let staging = result.steps.iter().find(|s| s.name == "precommit-staging").unwrap();
    assert!(staging.detail.as_deref().unwrap().contains("will not add work the user did not stage"));
    assert!(git::output(&dir, &["diff", "--cached", "--name-only"]).unwrap().is_empty());
    let _ = std::fs::remove_dir_all(&dir);
}
```

- [x] **Step 2: Run command tests and verify they fail**

Run:

```bash
cargo nextest run --manifest-path xtask/Cargo.toml precommit
```

Expected: FAIL because `Command::Precommit` and host-surface helpers do not
exist.

- [x] **Step 3: Add `Command::Precommit` and factor host gate**

Add a `Precommit` variant immediately after `Check` in the `Command` enum:

```rust
/// Git pre-commit gate: fast Fix-mode host surface from `check --no-test`, then
/// safe-stage only already-staged clean tracked paths changed by the gate.
Precommit,
```

Move lines currently shared by `Check` and `Validate` — static checks,
repo-shape/type-safety checks, `doctest_fences`, HTML gates, e2e scaffold,
`xlang_literal_check`, and `host_tests` — into:

```rust
fn run_host_gate(sh: &xshell::Shell, mode: Mode, result: &mut CommandResult) {
    steps::static_checks::run(sh, mode, result);
    steps::sequence_check::run(result);
    steps::adr_check::run(result);
    steps::doc_links::run(result);
    steps::error_swallowing_inventory_check::run(result);
    steps::test_pattern_check::run(result);
    steps::server_fn_registrar_check::run(result);
    steps::server_fn_tracing_check::run(result);
    steps::server_fn_coverage_check::run(result);
    steps::traced_context_check::run(result);
    steps::proffered_secret_check::run(result);
    steps::proffered_filename_check::run(result);
    steps::no_full_reload_check::run(result);
    steps::e2e_goto_wrapper_check::run(result);
    steps::target_arch_placement_check::run(result);
    steps::lint_suppression_check::run(result);
    steps::thin_components::run(result);
    steps::sqlx_newtype_bind_check::run(result);
    steps::sqlx_newtype_decode_check::run(result);
    steps::doctest_fences::run(result);
    steps::rendered_html_from_trusted_check::run(result);
    steps::raw_html_door_check::run(result);
    steps::html_sink_check::run(result);
    steps::e2e_scaffold_check::run(result);
    steps::xlang_literal_check::run(result);
    steps::host_tests::run(sh, result);
}
```

Then make `Check` call `run_host_gate(..., Mode::Fix, ...)` followed by
`steps::nix::test_checks(&mut result, no_test)`. Make `Validate` call
`run_host_gate(..., Mode::Check, ...)` after the clean-tree precheck, then keep
`wasm_budget`, `nix::test_checks(false)`, and optional `nix::e2e` unchanged.

- [x] **Step 4: Implement `Precommit` command body**

Factor the command body through this testable helper:

```rust
fn run_precommit_with_host_gate(
    dir: &Path,
    run_gate: impl FnOnce(&mut CommandResult),
) -> anyhow::Result<CommandResult> {
    let start = std::time::Instant::now();
    let mut result = CommandResult::new("precommit");
    let before = git::status_snapshot(dir)?;
    run_gate(&mut result);
    let after = git::status_snapshot(dir)?;
    let plan = git::precommit_stage_plan(&before, &after);
    result.push(git::apply_precommit_stage_plan(dir, &plan));
    finalize(&mut result, start);
    Ok(result)
}
```

Then the `match cli.command` arm is only:

```rust
Command::Precommit => {
    let sh = xshell::Shell::new()?;
    run_precommit_with_host_gate(Path::new("."), |result| {
        run_host_gate(&sh, Mode::Fix, result);
    })
}
```

Do not skip staging reconciliation when `run_host_gate` already failed; the
overall result stays failed through `CommandResult::ok`, but the stale-index fix
is still applied when safe.

- [x] **Step 5: Update `.githooks/pre-commit`**

Replace the hook body after the `SKIP_PRE_COMMIT` block with:

```bash
echo "--- pre-commit: running cargo xtask precommit ---"
exec cargo xtask precommit
```

Keep `set -euo pipefail` and the existing `SKIP_PRE_COMMIT=1` escape. Remove the
old shell-level `pre=$(git status --porcelain)` / `post=...` comparison; Git
policy now lives in Rust.

- [x] **Step 6: Run command tests and verify they pass**

Run:

```bash
cargo nextest run --manifest-path xtask/Cargo.toml precommit
```

Expected: PASS.

- [x] **Step 7: Smoke the hook bypass path**

Run:

```bash
SKIP_PRE_COMMIT=1 .githooks/pre-commit
```

Expected stdout contains `SKIP_PRE_COMMIT=1, skipping` and exit status 0.

- [x] **Step 8: Run focused xtask tests and commit**

Run:

```bash
cargo nextest run --manifest-path xtask/Cargo.toml precommit
```

Expected: PASS.

Run:

```bash
devtool run -- cargo xtask check
```

Expected: PASS. Stage `xtask/src/lib.rs`, `.githooks/pre-commit`, and any
mechanical fixes to those files, then commit:

```bash
git add xtask/src/lib.rs .githooks/pre-commit
git commit -m "fix(xtask): add precommit gate entrypoint"
```

### Task 3: Gate documentation and final verification

**Files:**

- Modify: `docs/adr/0029-git-enforced-verify-gate.md:15-62`
- Modify: `docs/ARCHITECTURE.md:2129-2152`
- Test: existing docs gates through `cargo xtask check`

**Interfaces:**

- Consumes Task 2's `cargo xtask precommit` and hook behavior.
- Produces current human-facing gate docs; no new ADR number.

- [ ] **Step 1: Amend ADR-0029**

Update ADR-0029's Decision bullets:

- Pre-commit hook now runs `cargo xtask precommit`, not `cargo xtask check`.
- `precommit` runs the fast Fix-mode host surface equivalent to
  `cargo xtask check --no-test`.
- Safe-staging is limited to already-staged tracked paths with no pre-existing
  unstaged change; unsafe paths fail with diagnostics; untracked paths are never
  staged; delete/rename states fail closed.
- Pre-push remains `cargo xtask validate --no-e2e` and keeps the clean-tree
  proof.

Update Consequences to remove the old claim that every commit runs the full
coverage build. Replace it with: pre-commit is shorter because it skips Nix
coverage/doctest/wasm checks; pre-push/CI remain the Nix-backed proof; safe
restaging prevents the #791 stale-index trap without sweeping unrelated dirty
work into commits.

- [ ] **Step 2: Amend `docs/ARCHITECTURE.md` gate projection**

Update the gate ladder section so it states:

- `cargo xtask check` is still the developer Fix-mode ladder and `--no-test`
  skips only Nix `wasm-tests`, `coverage`, and `doctests`.
- `cargo xtask precommit` is the hook entrypoint; it runs the `check --no-test`
  host surface, then applies safe-staging policy.
- `.githooks/pre-commit` calls `cargo xtask precommit`.
- `.githooks/pre-push` calls `cargo xtask validate --no-e2e`; `validate` remains
  the clean-tree proof.

- [ ] **Step 3: Run documentation-sensitive focused tests**

Run:

```bash
cargo nextest run --manifest-path xtask/Cargo.toml doc_links
```

Expected: PASS.

Run:

```bash
cargo nextest run --manifest-path xtask/Cargo.toml adr
```

Expected: PASS.

- [ ] **Step 4: Run full focused command surface**

Run:

```bash
cargo nextest run --manifest-path xtask/Cargo.toml precommit
```

Expected: PASS.

Run:

```bash
SKIP_PRE_COMMIT=1 .githooks/pre-commit
```

Expected: PASS with the skip message.

- [ ] **Step 5: Run the per-commit gate and commit docs**

Run:

```bash
devtool run -- cargo xtask check
```

Expected: PASS. Stage the docs and any mechanical fixes to those docs, then
commit:

```bash
git add docs/adr/0029-git-enforced-verify-gate.md docs/ARCHITECTURE.md
git commit -m "docs: describe staged-subset precommit gate"
```

- [ ] **Step 6: Final branch verification before ship/PR**

Run:

```bash
devtool run -- cargo xtask validate --no-e2e
```

Expected: PASS. If it fails only because the worktree is dirty from expected
doc/plan checkbox updates, stage and commit those updates first, then rerun on a
clean tree. Do not use `--allow-dirty` for the final proof.
