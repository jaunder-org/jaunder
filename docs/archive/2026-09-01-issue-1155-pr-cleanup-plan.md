# #1155 — Explicit post-land cleanup implementation outline

Spec:
[`docs/archive/2026-09-01-issue-1155-pr-cleanup-spec.md`](2026-09-01-issue-1155-pr-cleanup-spec.md)

## Risk trigger

This work adds a public CLI that performs ordered local Git mutations after an
irreversible remote merge. The implementation needs explicit capability,
identity, and partial-failure seams before those mutations are wired.

## Contracts

- `PrCommand::Cleanup { number: Option<u64> }` and command name `pr-cleanup` are
  separate from `PrOperation`; `watch` and `land` retain their existing
  observer/armer state machine unchanged.
- A read-only `CleanupSource` returns typed cleanup subjects containing PR
  number, state, base ref, head ref, and head SHA. Explicit-number lookup reads
  that PR. Omitted-number lookup follows GitHub cursors until `hasNextPage` is
  false, filters the complete merged-PR population for the captured branch and
  head SHA, and accepts exactly one match. Page size is transport detail, never
  uniqueness evidence.
- A narrow injected `CleanupCheckout` owns local facts and operations: current
  branch, HEAD, dirty status, fetch origin, ancestry proof, detach origin/main,
  safe local branch deletion, and root cargo clean. Production uses the existing
  env-scrubbed Git command owner; tests use a fake or temporary repository.
- The production executor first captures immutable branch/HEAD identity,
  resolves the PR, and pushes `pr-cleanup-precheck`. Preconditions require
  merged state, base `main`, exact branch/head identity, and a clean tree. Every
  refusal is `StepResult::fail`, never `skip`.
- After precheck, the executor appends exactly these fail-fast boundaries:
  `fetch-origin`, `verify-origin-main`, `detach-origin-main`,
  `delete-local-branch`, `cargo-clean`. Each receives duration and actionable
  failure detail. Later boundaries are absent after failure.
- Verification uses `git merge-base --is-ancestor <captured-head> origin/main`;
  deletion uses `git branch -d -- <captured-branch>`. No force/stash/reset,
  local-main update, remote deletion, tracker mutation, or cross-checkout
  fallback exists.
- The command's result and exit are ordinary `CommandResult` step semantics:
  complete cleanup exits 0; refused or failed cleanup exits 1; inability to
  construct the command environment before a result exists remains exit 2.
- The new ADR is a tracked proposed draft at
  `docs/adr/0171-explicit-post-land-cleanup-command.md`; architecture cites
  that path for promoter rewriting. `CONTEXT.md` remains unchanged.

## Implementation slices

### 1. Cleanup command and local capability

Own `xtask/src/pr/cleanup.rs`, `xtask/src/pr/mod.rs`, `xtask/src/pr/gh.rs` only
if shared GitHub transport needs a focused helper, `xtask/src/lib.rs`, and the
smallest necessary public helpers in `xtask/src/git.rs`, plus colocated tests.

- Add CLI parsing/dispatch without adding Cleanup to the watch/land operation
  enum or armer capability.
- Implement typed explicit and omitted cleanup resolution. Keep JSON/cursor
  parsing at the GitHub boundary; exhaust every page before rejecting or
  accepting zero/one/multiple exact matches. Test a unique match and ambiguity
  whose decisive entries occur after the first page, plus malformed
  identity/base/state/page evidence.
- Implement `CleanupCheckout` production operations with argument-safe branch
  handling and fail-closed errors. Keep captured identity immutable across fetch
  and checkout mutation.
- Implement the production executor and stable step sequence/details.
- Unit-test CLI, resolution, every precondition, successful ordering, every
  injected failure boundary, and unchanged watch/land dispatch.
- Add temporary-repository tests for fetch behavior, ancestry, detach, safe
  deletion, dirty refusal, and worktree/deletion refusal. Cargo clean remains an
  injected boundary in tests so the suite does not erase its own build.

### 2. Decision and workflow projection

Own `docs/adr/0171-explicit-post-land-cleanup-command.md`,
`docs/ARCHITECTURE.md`, and `CONTRIBUTING.md`.

- Create the proposed numberless ADR from the repository template. Record the
  explicit command, checkout identity proof, local-only capability, fail-closed
  sequence, and separation from merge approval/tracker mutation.
- Project the decision into the architecture with a descriptive draft-path
  citation.
- Replace `CONTRIBUTING.md`'s repository-tracked post-land four-command sequence
  with `cargo xtask pr cleanup [N]`; retain Status Done as a separate step and
  every human halt. Installed `.agents`/`.claude` skill mirrors are excluded
  local configuration, not feature-branch sources.

## Integration order

1. Develop the code and documentation slices concurrently against the contracts
   above; they own disjoint files.
2. Integrate the production command names and rendered details into the docs
   without changing the contract.
3. Format Rust and Markdown once after both slices land.

## Verification

1. Focused xtask tests:
   `devtool run -- cargo xtask test-local -- --manifest-path xtask/Cargo.toml`.
2. Static/compile gate: `devtool run -- cargo xtask check --no-test`.
3. Real CLI surface: `devtool run -- cargo xtask pr cleanup --help`.
4. Review both Standards and Spec axes over the whole branch, fix every finding,
   then commit and run the normal precommit/prepush gates before opening the PR.
5. Open the feature PR. Before merge approval, run the real cleanup command
   against that still-open PR. It must emit a blocking `pr-cleanup-precheck`,
   exit 1, and leave branch, HEAD, index, and worktree unchanged.
6. After the feature PR is confirmed merged, exercise the shipped command on its
   own checkout with that PR number. Success must leave HEAD detached at fetched
   `origin/main`, remove only the local feature branch, and complete root cargo
   clean. Set project Status Done separately.
