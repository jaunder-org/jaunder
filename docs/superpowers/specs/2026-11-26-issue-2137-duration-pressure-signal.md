# Issue #2137 — duration pressure becomes a signal

## Outcome

An e2e lane no longer fails just because a passing test attempt used 80% or more
of its timeout. Such an attempt passes the lane and is reported where people and
agents look. It shows up as a GitHub warning annotation on the job, as a line in
the job's step summary, in the `e2e-evidence` step output, and as a `warning`
event in `dev pr-watch` / `dev pr-land` handoffs. A timed-out attempt still
fails the lane, even when its retry passes.

## Approach and tradeoffs

The duration verdict (`tools/dev/dev/e2e_evidence.py`) still reconciles the
duration manifest against the Playwright report exactly. Missing, malformed or
mismatched evidence still fails closed. It changes only what it does with
utilization, and adds one input: each attempt's Playwright `status`. The
accepted statuses are `passed`, `failed`, `skipped`, `interrupted` and
`timedOut`. A missing or other status fails closed.

| Attempt status                         | Lane                  | Reported                            |
| -------------------------------------- | --------------------- | ----------------------------------- |
| any but `timedOut`, utilization < 80%  | as Playwright decides | maximum pressure, as today          |
| any but `timedOut`, utilization >= 80% | as Playwright decides | listed as an over-threshold attempt |
| `timedOut`, at any retry               | **fails**             | the failure names the attempt       |

"As Playwright decides" means the lane's result is Playwright's own exit status,
which the producer already enforces. For example, an `interrupted` run already
exits non-zero.

These cases keep today's results. A fast `failed` attempt whose retry passes
stays a flaky pass under `JAUNDER_E2E_RETRIES=1`. A `timedOut` attempt whose
retry passes still fails the lane, as the pressure gate failed it before. The
rule now comes from the attempt's status, not its utilization, because a passing
attempt can exceed 100% (#2110's 102%) and must only warn. The #1859 agent
confirmed this was the issue's intent.

Surfacing:

The verdict returns the over-threshold attempts as data, so the step detail, the
annotation and the summary all come from one source.

- **Producer (`dev e2e-producer`).** The `e2e-evidence` step's detail lists
  every attempt at or above 80%. Under GitHub Actions it also prints one
  `::warning title=Duration pressure::<lane>: <n> attempt(s); worst <test>, retry, %`
  annotation per lane. That is one annotation per lane, not one per attempt,
  because GitHub silently drops a job's annotations past 10 per step. It appends
  the full list to `$GITHUB_STEP_SUMMARY`.
- **Reconciliation (`dev e2e-reconcile`)** re-checks the same lanes. Its detail
  mentions pressure but emits no annotations, so each occurrence is announced
  once.
- **`dev pr-watch`.** When the watch ends `ready-to-land` or `merged`, it makes
  one GraphQL read per commit of that commit's check runs and their
  `Duration pressure` warning annotations. The commits are the PR head and, for
  `merged`, the newest merge-group run's commit. That second commit is where
  #2110's 102% occurred, and pr-watch's per-poll query never sees it. Each
  annotation becomes one `warning` event (check name plus message) in the
  report, so it stays visible after the run is green. The per-poll query is
  unchanged, so the read happens once, not on every poll. `dev pr-land` observes
  through pr-watch and inherits the events.

Persistent pressure is handled by a rule, not a scanner (user decision).
CONTRIBUTING tells whoever sees a pressure warning in a handoff to search the
tracker for an open issue naming that test, then comment the run on it or file a
new one. The tracker dedups across runs. The cost we accept: an agent has to
act, and nothing aggregates on its own. The alternative was a scheduled workflow
scanning recent runs, and that is new machinery to maintain.

The decision gets a numberless ADR draft (`jaunder-adr`): duration pressure is a
reported signal and the hard limit is a `timedOut` attempt. No existing ADR owns
the 80% threshold. CONTRIBUTING, `docs/observability.md` and
`docs/ARCHITECTURE.md` stop calling it a gate.

**Port race.** The cited flaky test,
`steps::e2e_local::tests::local_collectors_overlap_while_default_metrics_port_is_occupied`,
was deleted along with the Rust `e2e-local` in #2066. The last cited failure was
on 2026-11-23, before that merged. The Python replacement binds ephemeral
loopback ports, retries on a collision and turns off the collector's fixed
metrics port. No code change is needed; the PR records the supersession and the
50-run acceptance is moot (user decision).

## Acceptance

Tests in `tools/dev/tests`:

- passing attempts at 85% and at 102% each give a passing verdict that records a
  warning naming the attempt;
- a `timedOut` attempt fails the verdict even when its retry passes;
- a fast `failed` attempt followed by a passing retry passes, with no failure;
- a `skipped` attempt is accepted;
- malformed or mismatched duration evidence still fails, including a missing or
  unknown attempt status;
- under a simulated GitHub Actions environment, the producer emits one
  annotation per lane plus the step-summary lines, and reconciliation emits
  neither;
- pr-watch turns `Duration pressure` annotations, on the PR head and on the
  merge-group commit, into `warning` events at `ready-to-land` / `merged`, and
  ignores other annotations.

Proof to execute: one real CI run on a throwaway draft PR shows a pressure
warning in the job annotation and step summary, and in the `dev pr-watch`
`ready-to-land` report for that PR. Getting there may need a threshold
temporarily lowered on that branch only. The merge-group path is proved by the
unit test and by this PR's own `dev pr-land` handoff, not by a forced
merge-group run.

Docs to commit: CONTRIBUTING (the signal and the filing rule),
`docs/observability.md`, `docs/ARCHITECTURE.md` and the ADR draft.

## Boundaries

- Timeouts are not re-sized and the 80% threshold is not changed.
- No scheduled scanner, no new trace-analysis report, no persistent history.
- The census, population, boot-trace and phase verdicts are unchanged.
- Other GitHub warning annotations are not surfaced by pr-watch.
