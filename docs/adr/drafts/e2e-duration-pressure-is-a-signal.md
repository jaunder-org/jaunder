# ADR-DRAFT: E2E duration pressure is a reported signal; only a timeout fails

- Status: proposed
- Date: 2026-11-26
- Issue: [#2137](https://github.com/jaunder-org/jaunder/issues/2137)

## Context

Every e2e lane reconciles its Playwright report with its duration-budget
manifest. Until now, any attempt that used 80% or more of its effective timeout
failed the lane, even when the attempt passed. That ejected #2110 from the merge
queue for a passing draft-lifecycle attempt at 102% of its 66 s timeout. The
cause of the slowness was fixed separately (#2113), but the rule kept failing
code that worked.

The hermetic checks run with `JAUNDER_E2E_RETRIES=1`. A test whose first attempt
fails and whose retry passes is reported `flaky` and exits 0. A timed-out first
attempt followed by a passing retry was failed only by the pressure rule,
because a timed-out attempt is always at 100% or more.

[ADR-0238](../0238-just-e2e-workflow-ownership.md) retained retry-duration
pressure as one of the narrow verdicts. This decision narrows that verdict:
pressure is still computed from the same evidence, but no longer refuses a lane.

The human decision (2026-11-26): a test that is close to its timeout should not
fail the lane, but that fact must be surfaced, not silently absorbed.

## Decision

- **The hard limit is a `timedOut` attempt.** The duration verdict reads each
  attempt's Playwright `status`. An attempt reported `timedOut` fails the lane
  at any retry, even when a later retry passes. That keeps the timeout outcome
  the pressure rule used to enforce. Every other outcome is Playwright's own
  verdict. A fast `failed` attempt whose retry passes stays a flaky pass. A
  missing or unknown status fails closed, as does any other malformed or
  mismatched evidence.
- **Pressure is a signal.** Any other attempt at or above 80% of its effective
  timeout passes the lane. That includes a passing attempt over 100%. Each such
  attempt is reported in these places:
  - in the lane producer's `e2e-evidence` step output;
  - in CI, as one `Duration pressure` warning annotation per lane, naming the
    worst attempt and the count;
  - in CI, as every attempt in the job's step summary.

  The per-lane annotation keeps the signal under GitHub's limit of 10
  annotations per step. Pair reconciliation re-checks the same lanes and says
  how many attempts were under pressure, but annotates nothing, so each
  occurrence is announced once.

- **The signal reaches the handoff.** When `dev pr-watch` ends `ready-to-land`
  or `merged`, it reads the `Duration pressure` annotations once from the PR
  head and, when merged, from the merge commit. The merge queue fast-forwards
  `main` to the merge-group commit, and CI's push run skips the e2e lanes when
  it finds that commit's passing merge-queue run. So the merge commit's pressure
  annotations normally come from the merge-group run alone; they come from a
  push run only when the lanes genuinely ran again. It reports each one as a
  `warning` event, and `dev pr-land` inherits them through pr-watch. A warning
  never changes the outcome.
- **Persistent pressure is a filing rule, not a scanner.** Whoever reads a
  pressure warning in a handoff comments on the open issue for that test, or
  files one (`CONTRIBUTING.md`). The tracker dedups across runs.

## Consequences

- A slow but passing test no longer ejects a merge-queue entry. Its headroom
  problem is still visible after the run is green, and it is tracked through an
  issue.
- Nothing aggregates pressure across runs automatically. If the filing rule
  turns out not to be followed, the fallback is a scheduled scan of recent runs'
  annotations, which is deliberately not built now.
- The 80% threshold and the timeout budgets are unchanged. Re-sizing a timeout
  remains a separate, evidence-led change.
- `pr-watch` surfaces one annotation title, `Duration pressure`, which it shares
  with the producer through a neutral annotations module rather than depending
  on the e2e verdicts. Other warning annotations, such as GitHub's own
  deprecation notices, are not surfaced.
