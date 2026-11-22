# Issue #1966 — skip the redundant push run for merge-queue-tested commits

## Outcome

When the merge queue lands a commit on `main`, the CI `push` run for that same
commit no longer repeats coverage, the Nix test-check and hermetic lanes, or the
E2E matrix. It runs only `Validation host`, which also writes `main`'s kache
compile cache. A commit that reaches `main` without a green merge-queue run (an
admin bypass) is still validated in full.

## Approach and tradeoffs

The human decided this on the issue (2026-11-21): a CI verdict belongs to the
commit that was tested, not to the branch. A green `merge_group` run on a commit
is that commit's coverage and E2E verdict when it lands on `main`. This fits
ADR-0077's "never trust a cached green check", because the queue run executed
everything and took no test result from Cachix.

Changes in `.github/workflows/ci.yml`:

1. A new first job, `Merge-queue verdict`, on `ubuntu-slim`, with job-level
   `permissions: { actions: read }`. Its lookup step runs only when
   `if: github.event_name == 'push'`. It uses a version-pinned
   `actions/github-script`, which runs on the runner's bundled Node, so nothing
   has to be installed on the slim image. The step looks for a CI `merge_group`
   run for the pushed SHA whose `Validate (no e2e)` and `e2e gate` jobs both
   succeeded. When it finds one, it links that run in the step summary.
   - The job output is
     `${{ github.event_name == 'push' && steps.lookup.outputs.tested == 'true' }}`.
     A pull-request or merge-queue run therefore cannot skip anything, by
     construction.
   - The script wraps everything in try/catch and always sets `tested`. When
     there is no matching run, or an API call fails, it sets `tested=false` (an
     API failure also raises a warning), so the run validates in full.
   - If the job itself fails (runner fault, action download), every lane is
     skipped and the aggregates fail. That turns the run red. It never turns
     into a silent skip.
   - It checks the two required jobs rather than the whole run's conclusion. The
     queue merges as soon as those checks pass, so the push can arrive before
     GitHub marks the run completed. On the latest pair the gap was 29 s.
   - The SHA is the same in both runs: with `merge_method: MERGE`, the queue
     fast-forwards `main` to the exact commit it tested. A batch of PRs gets one
     push of the group's tip, and that tip has its own `merge_group` run.
2. These jobs add `merge-queue-verdict` to `needs`: `validate-hermetic`,
   `validate-test-checks`, `validate-coverage`, `e2e-catalog`, `e2e-lane`,
   `e2e-reconcile`, `Validate (no e2e)` and `e2e gate`.
   - The lanes use `if: needs.merge-queue-verdict.outputs.tested != 'true'`. The
     implicit `success()` stays in their conditions.
   - The three aggregates **must keep `always()`**:
     `if: always() && needs.merge-queue-verdict.outputs.tested != 'true'`.
     Without it, a failed lane would mark a required check _skipped_, which
     satisfies branch protection, and the queue would merge a broken commit.
   - On a skipped push the aggregates show as skipped on the push suite. The
     commit's green verdict is the `merge_group` check suite on the same SHA.
3. `validate-host` stays independent and runs on every push. Human decision
   (this cycle): it is `main`'s only kache writer. Merge-queue runs save no
   kache, and they could not usefully do so, because their caches would be
   scoped to `gh-readonly-queue/…` refs that PRs cannot restore. This costs
   about 5 runner-minutes per merge, against about 150 saved.
4. The `ci.yml` comments that reserve `ubuntu-slim` for result-only gates are
   reworded to cover the verdict job. That job depends on no preinstalled image
   software either.

Alternatives rejected: skipping `validate-host` too would leave `main`'s kache
entry stale forever. A separate job that only warms the cache is a second job
definition that saves only the host tests. A workflow-level skip is impossible,
because `on:` cannot query the API.

Accepted cost: the push run no longer samples flakiness or infrastructure
failures a second time in the slow lanes (human decision on the issue).

Recorded: an ADR draft,
`docs/adr/drafts/merge-queue-verdict-covers-landed-commit.md`, clarifies that
"per-ref" in ADR-0192 (line 25) and ADR-0252 (lines 45-47) means per tested
commit. It also covers ADR-0077 (line 70) and states the skip rule and its
admin-push fallback. It is projected into `docs/ARCHITECTURE.md`'s merge-queue
paragraph. `docs/ci-merge-queue.md` gets a short note: views that show the
latest check per name for a landed SHA will show `skipped` from the push suite,
while the `merge_group` suite carries the verdict.

## Acceptance

- Gate: `dev check` passes (workflow lint, docs, ADR draft format and links).
- Post-merge proof, recorded on the issue. The push run for this PR's own
  landing commit shows `Merge-queue verdict` reporting `tested=true` with a link
  to the queue run. `Validation host` ran and saved kache. Every other
  validation and E2E job was skipped. The skipped matrix jobs show unexpanded
  names, which is expected. `GET /commits/{sha}/check-suites` still shows the
  `merge_group` suite as success.
- The PR's own `pull_request` and `merge_group` runs execute every lane, and the
  aggregates report from the real lane results. Code review confirms that
  `always()` is kept on all three aggregates.
- The fallback is checked by reading the script: when no matching run exists, or
  the API call fails, the output is `tested=false`. No admin push is
  manufactured to prove it.
- Runner-minutes and queue wait before and after are recorded on the issue.
  Before is push run 45063249071; after is this PR's landing push run.

## Boundaries

- No change to the `pull_request` or `merge_group` behaviour, the ADR promoter,
  `performance.yml`, `mutants.yml` or branch protection.
- No reuse of PR results inside the merge queue (the separate idea in the
  issue's first comment).
- No new local tooling. No repository tool reads `event=push` CI runs.
