# Issue #1982 — report an in-progress merge-group failure as `ejected`

## Outcome

When the merge queue removes a PR's current head because a job in its
merge_group run failed, `just pr-watch` / `just pr-land` report `ejected`,
naming the failed job and pointing at the run — whether or not the run as a
whole has finished. `dequeued` is reported only when no current-head merge_group
run has a failed job.

## Approach and tradeoffs

Today the observer's ejection probe reads the newest merge_group workflow run
for the PR (`GET /repos/{o}/{r}/actions/runs?event=merge_group`). The pure state
machine calls it an ejection only when the run's overall `conclusion` is
`failure` and the run is newer than the head commit. GitHub ejects as soon as a
required check fails, but the run's `conclusion` stays `null` (status not
`completed`) until every job, including slow E2E jobs, finishes. In that window
the observer falls through to `dequeued` ("no failed current-head merge-group
run explains it"); after the run finishes the same incident reads `ejected`.

Change:

1. **Observation boundary.** The probe's run record gains the run `id`. When the
   newest merge_group run has a null conclusion, the probe fetches all pages of
   that run's latest-attempt jobs
   (`/actions/runs/{id}/jobs?filter=latest&per_page=100&page=N`) and records the
   name of the first job whose conclusion is `failure`, `timed_out` or
   `startup_failure`. A completed run needs no extra call. The all-pages jobs
   fetch is one helper, extracted from the two copies of that loop already in
   the shared-failure evidence code, which then use it too. The new job parser
   accepts null conclusions (running jobs) and fails closed on a missing `jobs`
   array; a jobs-call error propagates like any other probe error instead of
   reading as "no ejection".
2. **Decision.** A run counts as an ejection when it is newer than the head
   commit and either its conclusion is `failure` or it has a failed job. For an
   in-progress run the detail names the job ("job 'Validation host' in the
   front-of-queue merge_group run failed; the run is still in progress"). The
   pointer is the run URL in both cases, so `--reenqueue` acknowledgment (keyed
   on run URL) is unchanged.

Why "any failed job" rather than only ruleset-named ones: consistency. The
existing rule already counts any failed job, because one failed job makes the
finished run conclude `failure`. The new rule reaches the same verdict earlier,
so the reports before and after the run finishes agree. `cancelled` jobs are not
counted: cancellation follows a removal rather than causing it, and a cancelled
run is not an ejection today either.

Cost: the jobs call happens only in the existing ejection-probe state (open,
green, unqueued, queue present) and only while the newest merge_group run is
still in progress.

## Acceptance

- Decision tests: a current-head in-progress run with a failed job, vanished
  from the queue with same-head queue history, yields `ejected` with the run URL
  and job name — not `dequeued`. An in-progress run with no failed job still
  yields `dequeued`. An older-than-head in-progress run with a failed job is not
  an ejection.
- Watch test: an in-progress failed-job run whose URL is the acknowledged
  (`--reenqueue`) ejection does not end in `ejected`.
- Boundary tests, through an injectable transport: a completed run makes no jobs
  call; an in-progress run fetches jobs and reports a `failure` / `timed_out`
  job but not pending, `success` or `cancelled` ones; a jobs-call error or a
  payload without `jobs` propagates as an error.
- Existing pr-observer tests pass
  (`cargo test --locked --manifest-path tools/Cargo.toml -p pr-observer`).

## Boundaries

- No change to ejection recency rules, `--reenqueue` semantics, or other
  outcomes.
- No new API calls outside the ejection-probe state.
- No live GitHub replay; unit tests on hand-built values and JSON are the proof.
