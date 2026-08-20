# #1061 Host Test Comment Implementation Plan

> **For agentic workers:** Execute this plan task-by-task with jaunder-iterate
> (delegating individual tasks to a subagent via jaunder-dispatch when useful).
> Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Correct the stale `host_tests` comments without changing gate
behavior.

**Architecture:** This is a comment-only cleanup in the xtask host gate. The
implementation preserves the two `step(...)` calls exactly and narrows the prose
to ADR-0141's real invariant: auxiliary workspace unit suites are not executed
by root application coverage/Nix test gates.

**Tech Stack:** Rust source comments in `xtask`; verification via
`devtool run -- cargo xtask check --no-test`.

**Scope:** In: `xtask/src/steps/host_tests.rs` comments only. Out: commands,
step names, step order, workspace membership, Nix filters, coverage behavior,
ADRs.

**Task list:**

1. Rewrite the host test comments and verify behavior stayed unchanged.

**Key risks/decisions:**

- Risk: accidentally changing the gate commands while editing adjacent code.
  Guard by diffing the two `result.push(step(...))` blocks before commit.
- Decision: no new tests; the observable contract is unchanged and the spec
  requires the existing gate proof.

## Global Constraints

- Follow `CONTRIBUTING.md`: structured edits, no lint suppressions, commit only
  what was checked.
- Keep the load-bearing `issue-1061` token in this plan path.
- Spec: change only comments in `xtask/src/steps/host_tests.rs`.
- Spec: `xtask-tests` and `tools-test` commands, step names, arguments, and
  order remain behaviorally identical.
- Spec: wording must stay consistent with ADR-0028 and ADR-0141.
- Gate command: `devtool run -- cargo xtask check --no-test`.

---

### Task 1: Correct `host_tests` workspace-boundary comments

**Files:**

- Modify: `xtask/src/steps/host_tests.rs:6-21`
- Reference:
  `docs/superpowers/specs/2026-08-20-issue-1061-host-tests-comment.md`
- Reference: `docs/adr/0028-devtool-vs-xtask-boundary.md`
- Reference: `docs/adr/0141-cargo-workspace-execution-boundaries.md`

**Interfaces:**

- Consumes: existing `pub fn run(sh: &Shell, result: &mut CommandResult)` and
  its two `result.push(step(...))` calls.
- Produces: unchanged `pub fn run(sh: &Shell, result: &mut CommandResult)`
  behavior with corrected explanatory comments.

- [x] **Step 1: Rewrite the comments**

  In `xtask/src/steps/host_tests.rs`, update only the doc comment above `run`
  and the inline comment above `tools-test`.

  Required final meaning:
  - Top-level doc comment says these are fast host-side unit tests for auxiliary
    workspaces whose unit suites are not executed by root application
    coverage/Nix test gates.
  - Top-level doc comment keeps the `--no-test` / `--no-e2e` distinction and the
    `No coverage here` note.
  - Inline `tools-test` comment says `tools/` is an auxiliary virtual workspace
    and `tools-test` executes its unit suite; it must not say the whole
    workspace is excluded from every Nix check.
  - It is acceptable to mention that some tool crates are still built or used by
    static checks, because that is the corrected distinction.

  Do not edit these behavior lines except for incidental line-number shifts from
  comment wrapping:

  ```rust
  result.push(step(
      sh,
      "xtask-tests",
      "cargo",
      &["test", "--manifest-path", "xtask/Cargo.toml"],
  ));
  result.push(step(
      sh,
      "tools-test",
      "cargo",
      &["test", "--manifest-path", "tools/Cargo.toml"],
  ));
  ```

- [x] **Step 2: Inspect the diff for behavior changes**

  Run: `git diff -- xtask/src/steps/host_tests.rs`

  Expected: PASS by inspection — the diff changes only comments. The two
  `result.push(step(...))` calls keep the same step names, commands, arguments,
  and order.

- [x] **Step 3: Run the gate proof**

  Run: `devtool run -- cargo xtask check --no-test`

  Expected: PASS — JSON summary has `ok: true` and `exit_code: 0`.

- [x] **Step 4: Commit**

  Before committing, tick this task checkbox in this plan. The implementation
  change remains scoped to `xtask/src/steps/host_tests.rs`; the spec/plan files
  are lifecycle artifacts carried with the commit so the cycle can be resumed
  and archived.

  Stage exactly:

  ```bash
  git add xtask/src/steps/host_tests.rs docs/superpowers/specs/2026-08-20-issue-1061-host-tests-comment.md docs/superpowers/plans/2026-08-20-issue-1061-host-tests-comment.md
  ```

  Commit:

  ```bash
  git commit -m "docs(xtask): correct host test rationale"
  ```

  No `Co-Authored-By` trailer.
