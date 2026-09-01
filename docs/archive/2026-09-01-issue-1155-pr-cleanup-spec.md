# #1155 — Explicit post-land checkout cleanup

Issue: [#1155](https://github.com/jaunder-org/jaunder/issues/1155). Milestone:
Developer tooling & DX.

## Outcome

`cargo xtask pr cleanup [N]` replaces the four mechanical local commands that
follow a confirmed `pr land`: fetch `origin`, detach at the merged
`origin/main`, delete the local PR branch with Git's safe `-d`, and run root
`cargo clean`. The command proves the merged PR and checkout identity before
mutating anything. `pr land` remains unchanged and never invokes cleanup
automatically.

## Load-bearing decisions

- Cleanup is an explicit command separate from `pr land`. Running `pr land`
  remains the merge approval and retains its existing `merged` outcome and exit
  semantics even when local cleanup has not run or later fails.
- The CLI is `cargo xtask pr cleanup [N]`. With an explicit number, GitHub
  identifies that PR. With no number, cleanup exhaustively paginates every
  merged PR candidate for the current branch, then requires exactly one whose
  head ref and head SHA equal the captured checkout. It does not reuse the
  open-PR resolver used by `watch` and `land`, and a bounded first page is never
  treated as uniqueness evidence.
- Cleanup mutates the current checkout only when all preconditions hold:
  - GitHub reports the PR as merged;
  - the observed PR base ref is exactly `main`;
  - a branch is currently checked out;
  - the current branch exactly equals the PR head ref;
  - local HEAD exactly equals the PR head SHA; and
  - the working tree has no staged, unstaged, or untracked changes.
- A detached checkout, another current branch, divergent local HEAD, dirty tree,
  unresolved or ambiguous PR, or non-merged PR performs no cleanup. Because the
  explicit cleanup request remains incomplete, it reports the exact reason and
  exits nonzero; it is not a successful no-op.
- After precheck, cleanup runs these ordered boundaries and stops at the first
  failure:
  1. plain `git fetch origin`;
  2. verify the captured PR head SHA is an ancestor of `origin/main`;
  3. `git switch --detach origin/main`;
  4. `git branch -d -- <captured-pr-branch>`; and
  5. root `cargo clean`.
- The ancestry proof is separate from GitHub's merged state. It prevents branch
  deletion when the local remote-tracking ref has not incorporated the merged
  head, while preserving the linked-worktree rule that local `main` never needs
  to move.
- Cleanup never stashes, resets, force-deletes, checks out local `main`, updates
  `main:main`, deletes a remote branch, closes an issue, changes project status,
  selects the next issue, or bypasses a worktree/branch-deletion refusal.
- Each boundary is an ordinary ordered `StepResult` with stable names and
  actionable detail: `pr-cleanup-precheck`, `fetch-origin`,
  `verify-origin-main`, `detach-origin-main`, `delete-local-branch`, and
  `cargo-clean`. An incomplete safety refusal or failed operation uses a
  blocking `StepResult::fail`, so the cleanup command exits nonzero and later
  steps are absent. It does not use `StepResult::skip`, whose repository-wide
  contract is successful/nonblocking.
- A cleanup failure cannot rewrite the already-observed remote merge as failed:
  `pr land` has already completed independently. The cleanup command reports its
  own local failure and remediation.
- Local operations are injected behind a narrow cleanup interface for tests,
  following ADR-0016. GitHub observation reuses the read-only PR source
  boundary; cleanup acquires no `PrArmer` and does not weaken ADR-0087's
  observer/approval split.
- A new tracked ADR draft,
  `docs/adr/drafts/explicit-post-land-cleanup-command.md`, records the third,
  local-only PR command and its capability boundary. `docs/ARCHITECTURE.md`
  projects that draft and `CONTRIBUTING.md` owns the repository-tracked shipping
  workflow. The installed `.agents`/`.claude` skill mirrors are intentionally
  excluded from this repository and remain outside this change. `CONTEXT.md` is
  unchanged because this introduces no domain vocabulary.

## Acceptance

- CLI parsing and command naming cover `pr cleanup`, `pr cleanup 1155`, and
  reject invalid numbers consistently with `watch`/`land`.
- Omitted-number resolution exhausts all result pages and selects a merged PR
  only when branch and exact head SHA match; zero or multiple matches fail
  without local mutation. Tests place both a unique match and an ambiguity
  beyond the first page.
- Explicit-number cleanup still requires the current branch and HEAD to match
  that PR. Running it from another branch or detached HEAD fails before fetch.
- A merged PR whose observed base ref is not exactly `main` fails precheck
  without local mutation.
- Open/closed-unmerged PRs, dirty trees, divergent heads, missing origin, and
  ambiguous/unparseable GitHub evidence fail before mutation with actionable
  `pr-cleanup-precheck` detail.
- A successful fake execution records exactly the six production boundaries in
  order and passes the captured branch/SHA unchanged to verification and
  deletion.
- Injected failure at each operational boundary stops every later boundary and
  leaves the failed step visible.
- Temp-repository tests prove the real local implementation:
  - fetch advances `origin/main` without moving local `main`;
  - ancestry verification accepts a merged head and rejects an unincorporated
    head;
  - detach targets `origin/main`;
  - safe `-d` deletes only the captured merged local branch;
  - dirty state blocks all mutation; and
  - branch/worktree deletion refusal is surfaced rather than forced.
- The cargo-clean runner is invoked only after successful branch deletion; its
  failure is reported without changing the prior merged PR result.
- Existing `pr watch` and `pr land` subject resolution, capabilities, reports,
  tests, and command-specific exits remain unchanged.
- Contributor, architecture, and ADR documentation names the new command and
  leaves project Status Done as the separate claim-release step.
- Focused xtask tests, `cargo xtask check --no-test`, and the real
  `cargo xtask pr cleanup --help` CLI surface pass.

## Boundaries

- No automatic cleanup inside `pr land` and no change to merge approval, arming,
  watching, queue handling, or GitHub outcome semantics.
- No issue/project mutation, remote branch deletion, next-issue automation,
  cross-checkout cleanup, stash/reset/force behavior, or generic PR automation
  framework.
- No support for cleaning a checkout that is not currently on the exact merged
  PR head branch.
- No cleanup of non-`main` PRs in this issue; both the observed base ref and the
  fetched ancestry proof fail closed before detach/delete.
