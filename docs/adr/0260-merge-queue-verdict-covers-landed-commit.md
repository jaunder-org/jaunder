# ADR-0260: A merge-queue verdict covers the commit it lands

- Status: accepted
- Date: 2026-11-22
- Issue: [#1966](https://github.com/jaunder-org/jaunder/issues/1966)

## Context

Under the [merge queue](0077-adopt-github-merge-queue.md), each merged PR ran CI
three times: on `pull_request`, on `merge_group`, and on the `push` to `main`
that followed. The queue fast-forwards `main` to the exact commit it tested, so
the push run repeated a full execution on an identical SHA. In that repeat, the
hermetic and test-check lanes substituted from Cachix, but the host lane,
coverage and every E2E lane ran again. The issue measured about 193
runner-minutes per push run. After later lane changes, push run 45063249071 took
about 147, of which `Validation host` was about 5. The repeat also caused jobs
to queue while runs overlapped.

[ADR-0192](0192-split-ci-non-e2e-validation-lanes.md) says coverage and e2e
verdicts "must remain per-ref".
[ADR-0252](0252-substitutable-static-verdicts.md) adds that "CI executes them
for every ref". Read literally, every push to `main` would need its own
execution, even of a commit that was just tested.

## Decision

A coverage or e2e verdict belongs to the commit that was tested, not to the
branch. "Per-ref" in ADR-0192 and ADR-0252 means per tested commit. A
`merge_group` CI run in which `Validate (no e2e)` and `e2e gate` passed is the
verdict for that commit when it lands on `main`.

This keeps ADR-0077's "never trust a cached green check". The queue run executes
coverage and e2e on the combined state and takes no test result from Cachix.
Nothing here relaxes the `pushFilter` rule in ADR-0252.

For a push to `main`, CI's first job, `Merge-queue verdict`, looks for such a
run of the pushed SHA:

- If one exists, the push run skips every validation and e2e job except
  `Validation host`. That lane stays because it is the only writer of `main`'s
  kache store ([ADR-0256](0256-kache-compile-cache.md)), and merge-queue caches
  cannot be restored by pull requests.
- Otherwise, for example an admin push that bypassed the queue, or a failed
  lookup, the push run validates in full.

Pull-request and merge-queue runs never skip. The result-only aggregates keep
`always()`, so a failed lane still fails its required check rather than skipping
it.

## Consequences

- Each merge saves about 140 runner-minutes (run 45063249071, less
  `Validation host`), and fewer runs overlap.
- Every pull-request and merge-queue run waits for one `ubuntu-slim` job before
  its lanes other than `Validation host` start, and a failure of that job turns
  the run red.
- The push run no longer gives a second sample of flaky or infrastructure
  failures in the slow lanes. Flakiness has to show up in pull-request,
  merge-queue or scheduled runs.
- On a landed SHA, views that show the latest check per name show the push
  suite's skipped `Validate (no e2e)` and `e2e gate`. The `merge_group` check
  suite on the same SHA carries the green verdict. A check of whether `main` is
  green must read the commit's check suites, not CI runs filtered by
  `event=push`.
- Reusing a PR's own results inside the merge queue is a separate decision and
  is not made here.
