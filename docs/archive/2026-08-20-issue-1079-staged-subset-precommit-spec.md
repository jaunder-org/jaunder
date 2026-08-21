# #1079 — staged-subset-safe precommit gate

Issue: [#1079](https://github.com/jaunder-org/jaunder/issues/1079). Milestone:
Developer tooling & DX. Provenance: the #791 stale-index failure, the OMP
cycle-time analysis, and ADR-0029's git-enforced gate architecture.

## Summary

The pre-commit hook currently runs `cargo xtask check` after authors and agents
have often staged a selected set of paths. `check` runs in Fix mode, so it can
rewrite a file that is already staged. If that rewrite happens after `git add`,
Git commits the old index blob and leaves the fixed file dirty in the worktree.
#791 hit exactly that shape with `end2end/tests/seed.ts`: Prettier reformatted
the staged file after staging, the commit landed the stale blob, and the later
clean-tree `cargo xtask validate` failed immediately.

The fix is not to ban staged-subset commits. Transcript data showed staged
subsets are common in the agent workflow, while literal `git commit -- <paths>`
is rare. The fix is to make the hook entrypoint preserve staged subsets while
restaging only the fixes that are provably part of the commit the user already
asked to make.

## Current gate contract

ADR-0029 and `docs/ARCHITECTURE.md` define a two-rung local gate:

- `cargo xtask check` runs host static checks in Fix mode, then repo-shape
  gates, host tests, and — unless `--no-test` — the Nix `wasm-tests`,
  `coverage`, and `doctests` checks.
- `cargo xtask validate` runs verify-only, adds `wasm-budget`, and — unless
  `--no-e2e` — the e2e aggregate. Its clean-tree precheck is the local proof
  that the committed tip is what was measured.

This issue changes only the pre-commit entrypoint. It does not weaken the
pre-push proof gate.

## Decisions

| ID     | Decision                                                                                                                                                                                                                                                                                                                                                                                                           |
| ------ | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------ |
| **D1** | Add a dedicated `cargo xtask precommit` subcommand. The hook must call that subcommand directly; `cargo xtask check` must not infer hook mode from Git environment or parent processes.                                                                                                                                                                                                                            |
| **D2** | `precommit` runs the fast Fix-mode gate equivalent to `cargo xtask check --no-test`: host static checks, repo-shape gates, type-safety gates, and host tests, but not the Nix `wasm-tests`, `coverage`, or `doctests` checks and not e2e.                                                                                                                                                                          |
| **D3** | Staged-subset commits are supported. The hook may re-stage a tracked path only when the path was staged before the run and had no pre-existing unstaged tracked change before the run.                                                                                                                                                                                                                             |
| **D4** | Unsafe mutations fail closed with exact paths and reasons. Unsafe means: the gate changed an unstaged-only tracked path; changed a path that already had mixed staged and unstaged content; created a new non-gitignored untracked path; or any before/after Git status for a path observed during `precommit` contains a deletion or rename. Delete/rename states are deliberately not auto-staged in this issue. |
| **D5** | `precommit` never stages untracked paths and never runs `git add .` / `git add -A`. Auto-staging is limited to tracked paths that satisfy D3.                                                                                                                                                                                                                                                                      |
| **D6** | `cargo xtask check` remains a developer command and never stages changes. All hook-specific staging logic lives behind `cargo xtask precommit`.                                                                                                                                                                                                                                                                    |
| **D7** | `.githooks/pre-commit` becomes a small wrapper around `cargo xtask precommit`, preserving `SKIP_PRE_COMMIT=1`. `.githooks/pre-push` remains `cargo xtask validate --no-e2e`.                                                                                                                                                                                                                                       |
| **D8** | ADR-0029 and `docs/ARCHITECTURE.md` must be amended to describe the new hook entrypoint and the fast/verify split. A new ADR is not required: this is an amendment to the accepted gate architecture, not a new independent architecture.                                                                                                                                                                          |

## Acceptance criteria

- **AC1 — hook entrypoint exists.** `cargo xtask precommit` is a first-class
  subcommand and is the only command `.githooks/pre-commit` invokes for the
  gate.

- **AC2 — pre-commit is fast.** `precommit` runs the same non-Nix test surface
  as `cargo xtask check --no-test`: host static checks, repo-shape gates,
  type-safety gates, and host tests. It does not build or realize the Nix
  `wasm-tests`, `coverage`, or `doctests` checks, and does not run e2e.

- **AC3 — staged tracked formatter fix is preserved.** Given a tracked file that
  is staged before `precommit`, has no unstaged tracked change before
  `precommit`, and is modified by the gate, `precommit` re-stages that file so
  the commit would include the fixed blob. This is the #791 stale-index trap.

- **AC4 — staged subsets remain subsets.** Given unrelated unstaged tracked work
  that existed before `precommit` and is not modified by the gate, `precommit`
  leaves it unstaged and succeeds. The hook does not require a clean worktree.

- **AC5 — mixed staged/unstaged files fail closed.** Given a path with both
  staged and unstaged tracked changes before `precommit`, if the gate modifies
  that path then `precommit` fails and does not stage it. The diagnostic names
  the path and says the pre-existing mixed state made auto-staging unsafe.

- **AC6 — unstaged-only gate mutations fail closed.** Given a tracked path with
  no staged change before `precommit`, if the gate modifies that path then
  `precommit` fails and does not stage it. The diagnostic names the path and
  says the hook will not add work the user did not stage.

- **AC7 — untracked files are never auto-staged.** Pre-existing untracked,
  non-gitignored files that are not modified by the gate do not fail the hook.
  New untracked, non-gitignored files created during `precommit` fail the hook.
  No untracked file is staged automatically.

- **AC8 — delete/rename states fail closed.** If any before/after Git status for
  a path observed during `precommit` contains a deletion or rename, `precommit`
  fails with a path-specific diagnostic and does not stage that path. This
  includes staged deletes, worktree deletes, staged renames, worktree renames,
  and delete/recreate shapes; this issue deliberately does not distinguish a
  safe subset of them.

- **AC9 — developer `check` stays non-staging.** `cargo xtask check` and
  `cargo xtask check --no-test` do not stage files, even when they modify files
  in Fix mode.

- **AC10 — pre-push proof remains.** `.githooks/pre-push` still runs
  `cargo xtask validate --no-e2e`, and `validate` still refuses dirty trees by
  default.

- **AC11 — documentation is current.** ADR-0029 and `docs/ARCHITECTURE.md`
  explain that pre-commit runs `cargo xtask precommit`, precommit safe-stages
  only already-staged clean tracked files changed by the gate, and pre-push
  remains the clean-tree proof gate.

- **AC12 — regression coverage bites.** Automated tests or executable fixture
  checks cover AC3, AC5, AC6, and AC7. The tests must fail if implementation
  falls back to `git add .`, if staged formatter fixes are not re-staged, or if
  mixed staged/unstaged files are auto-staged.

## Risks and constraints

- **Git status parsing can be subtle.** Prefer porcelain v1/v2 parsing in one
  helper with unit tests over ad-hoc string checks inside the command body.
- **The hook must not hide intent.** Auto-staging is acceptable only where the
  user already staged that tracked path and the hook can prove there were no
  pre-existing unstaged hunks in it.
- **The fast gate is not the final proof.** `precommit` may run on a dirty
  worktree by design. `validate --no-e2e` remains the clean-tree proof before
  push.
- **No result-stamp system.** Nix remains the cache/witness for Nix-backed
  gates; this issue is only about commit-time Git/index safety and moving the
  heavy Nix checks out of pre-commit.
