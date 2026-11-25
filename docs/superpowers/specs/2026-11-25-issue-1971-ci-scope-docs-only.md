# Issue #1971 — CI scope for documentation-only changes

## Outcome

A pull request or merge-queue entry whose changes are all Markdown runs only the
documentation checks (Prettier and the repository-docs checks such as doc
links), not the product lanes, and still passes both required checks
(`Validate (no e2e)` and `e2e gate`). A new commit pushed to a pull request
cancels that pull request's superseded CI run. Decisions: human, 2026-11-24, on
the issue.

## Approach and tradeoffs

**One rule, one implementation.** A change is _Markdown-only_ when it is
non-empty and every record of its raw Git diff is an add, modify or delete of a
`.md` path whose mode on each existing side is a regular file. That rule is one
Python function in `tools/dev/dev/index.py`, over one raw-diff record parser
returning `(old_mode, new_mode, status, path)`. Both callers use it unchanged:

- **Precommit** (`classify`) applies it to the staged diff
  (`git diff --cached --raw`). Its other conditions are about the working tree
  matching the commit, which only a hook has: no uncertain status evidence, no
  untracked or unstaged path. Those stay hook-only.
- **CI** (`dev ci-scope --base <sha>`) fetches the base at depth 1 and applies
  it to `git diff --raw -z --no-abbrev --no-renames <base> HEAD` (a tree diff,
  so a shallow clone suffices), printing `docs_only=true|false` for
  `$GITHUB_OUTPUT`. A missing or unfetchable base, git error or malformed record
  answers `false` (run everything) and exits 0; only infrastructure failures
  fail the job.

**Behaviour change for the precommit: Markdown deletes take the narrow route.**
Today ADR-0029 routes any delete or rename broad, as "conservative routing"; no
check in the narrow route needs the deleted file (Prettier has nothing to
format, and doc-links catches links the delete breaks). With `--no-renames` a
rename is a delete plus an add, so a Markdown rename also qualifies — which is
what lets ADR-promoter PRs qualify in CI. The narrow route's `static-docs`
admission, which today requires every path to be a staged regular file on disk,
accepts staged deletes. ADR-0029's narrow-class definition is amended by the new
ADR draft.

ADR-promoter PRs get no identity check. They qualify through the rule; one that
also rewrites a citation in a `.ts` comment (as #2090 did) runs everything.

**No Markdown feeds a product lane.** Nothing uses `include_str!` on Markdown,
and the Nix product checks filter `.md` out of their sources. The one exception
today: the host-proof cache-safety probe and the coverage policy probe
(`tools/host-proof/src/cache_safety.rs`,
`tools/coverage/src/host/policy/ probe.rs`) dirty the root `README.md` in a
probe worktree. They move to a tracked non-Markdown file, so the rule needs no
exception.

**Base per event.** `pull_request`: `github.event.pull_request.base.sha`;
`merge_group`: `github.event.merge_group.base_sha`; `push`: no base, always
full. A stale pull-request base only adds paths to the diff, so it can only move
towards running everything. (`HEAD^1` of the test merge would be exact for pull
requests, but one payload-based mechanism for both events is simpler and does
not depend on the merge method.)

**Workflow shape** (`.github/workflows/ci.yml`). The `e2e-catalog` job becomes
`ci-scope` (display `CI scope`): it runs `ci-scope`, then reads the catalog as
today, so e2e lanes start no later than now.

| Job                                                  | Docs-only run                                                        | Other runs                                        |
| ---------------------------------------------------- | -------------------------------------------------------------------- | ------------------------------------------------- |
| `ci-scope`                                           | runs                                                                 | runs; skipped on a queue-tested `push` (as today) |
| new `validate-docs` (`dev ci-validate-docs`, `.#ci`) | runs                                                                 | skipped                                           |
| `validate-host`                                      | skipped                                                              | runs, also when `ci-scope` is skipped             |
| hermetic, test-checks, coverage, e2e lanes           | skipped                                                              | as today                                          |
| `Validate (no e2e)`                                  | requires scope + docs success                                        | requires scope + four lanes success               |
| `e2e gate`                                           | requires scope success; lanes and reconcile skipped = not applicable | as today                                          |

`validate-host` is `main`'s kache writer and today never waits. It now takes
`needs: [ci-scope]` with
`if: !cancelled() && needs.ci-scope.outputs.docs_only != 'true'`, so it still
runs when `ci-scope` is skipped on a queue-tested push. Accepted cost: the four
validation lanes, host included, start about 45 s later (measured catalog job;
waiting is not billed), so a full run's wall clock grows about 45 s on its ~13.5
min coverage critical path. Running the rule in JavaScript in the `ubuntu-slim`
verdict job would avoid the wait, but the precommit would then call `node`, and
GitHub's compare API reports no file modes and truncates at 300 files (human
decision, 2026-11-25: keep the rule in Python).

`.#ci` because `ci-nix` has no Prettier. `dev ci-validate-docs` runs
`clean_tree`, the Markdown Prettier group in verify mode and the repository-docs
checks — the groups the precommit's Markdown-only route runs in fix mode, shared
as one group function.

**Cancellation.** Workflow-level
`concurrency: { group: ci-${{ github.event_name == 'pull_request' && github.event.pull_request.number || github.run_id }}, cancel-in-progress: true }`.
Non-PR groups are unique per run, so merge-queue and push runs are never
cancelled or queued. Re-running an old PR run cancels a newer in-progress one;
accepted. `always()` on `e2e-reconcile` and the two aggregates becomes
`!cancelled()`, so a cancelled run does not start the reconcile matrix; a failed
lane still fails its aggregate.

**Recorded decision.** An ADR draft (`jaunder-adr`) records path-scoped CI lanes
and the cancellation rule; `CONTRIBUTING.md`, `docs/ci-merge-queue.md` and the
matching `docs/ARCHITECTURE.md` prose describe the new topology.

## Acceptance

- `tools/dev` tests of the one rule, through both callers against real
  repositories: Markdown-only add/modify/delete/rename → Markdown-only; any
  non-`.md` path, a `.md` symlink (added or deleted), typechange and empty diff
  → not. `ci-scope` with an unfetchable or missing base → `false`. Precommit: a
  staged Markdown delete now routes `class=staged-markdown-only` and passes the
  `static-docs` admission; untracked/unstaged routing unchanged.
- The two probes no longer touch `README.md`, and their existing tests pass.
- Workflow structural tests (`tools/host-proof/tests/workflow_callers.rs`,
  `e2e_topology.rs`) updated to pin: the aggregates' docs-only and full
  requirements, `e2e gate` accepting skipped lanes only when `docs_only`, the
  host lane's condition, `!cancelled()`, and the concurrency block.
- This PR's own CI takes the full path and passes.
- A throwaway draft PR targeting this branch with one salted `.md` commit takes
  the docs-only path: only scope, docs and aggregate jobs run, and both required
  checks pass. A second salt commit cancels the first run.
- Before/after runner-minutes recorded on #1971. Before: promoter PR run
  45503763934, 158.6 job-minutes (28 jobs). After: the throwaway PR's run.

## Boundaries

- No change to `push` runs or the merge-queue-verdict skip.
- No other lane subsetting; only Markdown-only versus everything.
- Other workflows unchanged.
- Merge-queue docs-only behaviour is proven by the shared code path and the
  pull-request probe, not a separate queue experiment.
