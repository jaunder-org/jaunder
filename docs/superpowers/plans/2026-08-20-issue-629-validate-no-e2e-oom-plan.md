# Issue #629 Validate (no e2e) Null-Step Runner Loss Plan

## For agentic workers

Execute this plan task-by-task with `jaunder-iterate`. This is a no-code
disposition: do not change workflow, source, lockfile, or runner sizing.

## Goal

Close #629 as stale/unreproduced using the approved evidence, without pretending
a code fix happened.

## Approved spec

`docs/superpowers/specs/2026-08-20-issue-629-validate-no-e2e-oom-spec.md`

## Global constraints

- No production, workflow, or lockfile changes.
- No cargo/nextest/Nix concurrency cap.
- No larger runner.
- No synthetic repeated CI dispatch.
- The issue comment must preserve the exact recurrence criterion: reopen or file
  a fresh issue only if the null-step + `log not found` signature recurs in
  production CI.

## Task 1: Publish evidence comment

1. Add a GitHub issue comment to #629 summarizing:
   - original job `89231127491` / run `30014588401` still has `steps: []`;
   - `gh run view --job 89231127491 --log` returns `log not found: 89231127491`;
   - latest-30 CI sample found no completed workflow failures;
   - inspected newer `Validate (no e2e)` failure `31842589601` / `94902457387`
     had normal logs, `xtask-done`, and Rust compile errors (`DbConnectOptions`
     not in scope), so it is not the #629 signature;
   - current `Validate (no e2e)` runner is pinned to `ubuntu-24.04`; GitHub
     lists public `ubuntu-24.04` Linux as 4 CPU / 16 GB RAM; the original job
     reported `ubuntu-latest`;
   - after the documentation PR lands, #629 should be closed as
     stale/unreproduced unless the null-step + `log not found` signature recurs
     before then.
2. Verify by reading #629 back: the evidence comment is present.

Expected: #629 carries the disposition evidence while the project Status remains
`In Progress` until the PR merges.

## Task 2: Commit planning artifacts

1. Stage only the #629 spec and this plan.
2. Run `devtool run -- cargo xtask check`.
3. Commit with a docs/superpowers message referencing #629.

Expected: planning artifacts are committed; working tree is clean.

## Task 3: Archive and ship no-code branch

1. Move the #629 spec and plan to `docs/archive/`.
2. Run `devtool run -- cargo xtask check`.
3. Commit the archive move.
4. Rebase on `origin/main`.
5. Run `devtool run -- cargo xtask validate --no-e2e` because the branch is
   docs/issue-tracker only and cannot affect web/server/e2e behavior.
6. Push, open a PR referencing #629, and monitor with `cargo xtask pr watch`.
7. Stop for merge approval.
8. After approved merge succeeds, close #629 with stale/unreproduced rationale
   (`state_reason: not_planned` if the API accepts it; otherwise use the closest
   supported non-completed reason and keep the comment explicit).
9. Set the Jaunder Backlog project Status for #629 to `Done`.
10. Verify by reading #629 back: closed, closing evidence/comment present,
    linked project status is `Done`.

Expected: no-code disposition branch lands, then #629 is closed and released.
