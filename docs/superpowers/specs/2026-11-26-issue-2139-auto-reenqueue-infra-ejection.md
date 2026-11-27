# Issue #2139: re-enqueue once automatically after an infrastructure ejection

## Outcome

When the merge queue ejects a PR that `dev pr-land` is landing, `pr-land`
classifies the failed merge-group run. If every primary failed job matches a
known infrastructure signature, it re-enqueues the same approved head once by
itself and keeps observing until it merges. Any other ejection, and any second
ejection in the same invocation, stays terminal as today. The matched signature
and the failed run's URL appear in the output and the trace.

## Approach and tradeoffs

**What a wrong verdict costs.** A re-enqueued group re-runs every required
check, so a misclassification never merges an unchecked head. Its worst case is
one automatic retry of a real failure: a wasted queue cycle that delays the PRs
behind, or an intermittent real failure that passes on retry. The one-attempt
rule bounds it. In exchange, a known class of infrastructure ejections no longer
waits for a human.

**`pr-land` becomes Python.** `scripts/pr-land.sh` (73 lines) moves to
`tools/dev/dev/pr_land.py` and retires. The one-attempt rule makes landing a
state machine, which
[ADR-0261](../../adr/0261-tool-language-by-maintainability.md) names Python's
ground, and an in-process call keeps one classifier inside the observer. The CLI
(`dev pr-land <PR> <SHA> [--reenqueue]`), the guards, the single
`gh pr merge --auto --merge --match-head-commit` action per attempt, the exit
codes and the final observer report on stdout are unchanged. `pr-land` calls the
observer's watch function with its traced `gh` runner instead of spawning
`dev pr-watch` and re-checking its JSON with `jq`. Simplest alternative: keep
bash with a retry function and an attempt flag; rejected because it would carry
the attempt state and the ejection field through `jq` across two programs.
Accepted cost: the land tests script the observer's `gh` calls instead of
replaying `dev pr-watch` reports.

**The observer classifies; it still only reads.** When `pr-watch` reaches
`ejected` (its existing rule picks the run), it adds an `ejection` object to the
report. Every `pr-watch` invocation reports it; only `pr-land` acts on it.

1. Take the run's jobs (`filter=latest`) concluded `failure`, `timed_out` or
   `startup_failure`. A run still in progress is judged on the jobs failed so
   far; the observer does not wait for it.
2. Drop **derived** jobs: a failed job whose definition lists in `needs:` a job
   that also failed in this run. Definitions come from the workflow file the
   observer already reads at the PR head; a definition is found by its literal
   `name` (or key), with each `${{ … }}` expression in the name matching any
   text. A failed job matching no definition, or several, is unclassified. Today
   this drops `Validate (no e2e)`, `e2e gate` and `e2e reconcile (…)` after a
   lane failure. This is a **scoped exception to
   [ADR-0264](../../adr/0264-pr-watch-native-python-observer.md)'s retired graph
   evaluation**: one `needs:` hop, for classifying an ejection, not for early
   failure; no matrix expansion and no job names in source. The jobs API does
   not expose `needs`, and the aggregates and reconcile jobs fail whenever a
   lane does. Accepted cost: `needs: [e2e-lane]` names the whole matrix, so a
   real reconcile failure in one pair is hidden by an infrastructure lane
   failure in another; the bounded retry covers it.
3. Every remaining (primary) job must match a signature, and at least one must
   exist. "A mix of infrastructure and real failures is not retried" thus holds
   among the jobs failed at classification time.

**Signatures** are a closed list in `pr_watch.py`, each with a real example:

| Signature        | Evidence                                                                                                                                     | Example job                           |
| ---------------- | -------------------------------------------------------------------------------------------------------------------------------------------- | ------------------------------------- |
| `nix-substitute` | job log (ANSI stripped) has `is required, but there is no substituter that can build it` and `error: unable to download '…': HTTP error <n>` | 135941369307 (#2016, run 44632169176) |

The log is read with `gh api --allow-escape-sequences …/jobs/<id>/logs`;
annotations never carry the Nix text. `HTTP error <n>` rather than `416` because
#2072 failed with `206`. Never matched: cancellations of any kind,
execution-time limits, and advisories (terminal until #2134).

**Dropped from the issue's starting list, for lack of a real example:**

- _Runner loss._ No merge-group run in the history lost its runner. The only
  runner failure found (push job 133630906946) was `No space left on device`,
  which a PR that grows the build can cause itself.
- _Collateral from the PR ahead._ In six failed runs built on a failed
  PR-ahead's group commit, GitHub rebuilt the group and never ejected the PR
  behind.

Each is added later as a reviewed source change with its real example job.

**Classification never changes the outcome.** It catches its own read failures
(log, annotation or workflow file; transient or not), spends no poll strikes and
reports the job unclassified with the reason. `ejected` stays `ejected`, under
`--once` too.

**Report.**
`ejection = {run, signature, jobs: [{id, name, derived, signature}], unclassified}`:
`signature` is set only when the run qualifies; otherwise `unclassified` says
why. The observer logs a `classify` event (stderr and report) and records
`classify-ejection` as a trace phase with the signature and run URL as
attributes.

**`pr-land` flow** (one attempt per invocation, wherever the ejection is seen):

| Seen                           | Attempt unused, `signature` set | Otherwise                                |
| ------------------------------ | ------------------------------- | ---------------------------------------- |
| preflight `ejected`            | re-enqueue (ordinary mode too)  | `--reenqueue` re-enqueues; else terminal |
| post-arm observation `ejected` | re-enqueue                      | terminal, report printed, exit 1         |

`--reenqueue` keeps its meaning (explicit recovery for any cause) and spends the
attempt. A re-enqueue is today's recovery sequence: not queued or armed,
one-snapshot `ready-to-land` with that run acknowledged, one native action,
readback, then observation with the run acknowledged. `pr-land` prints
`re-enqueueing once: <signature> (<run URL>)` to stderr before acting.

**Decision records.** This narrows the rule that diagnosis never authorizes
recovery: inside an already approved `pr-land`, a classified infrastructure
ejection does, once. Amended in place:
[ADR-0087](../../adr/0087-xtask-github-pr-observation.md) (recovery needs
explicit authorization),
[ADR-0220](../../adr/0220-explicit-one-shot-pr-reenqueue.md) (explicit-only
re-enqueue), ADR-0264 (the `needs:` exception, `pr-land.sh`) and ADR-0261
(`pr-land.sh` leaves the conforming-shell list). ARCHITECTURE.md and
CONTRIBUTING.md (land section, including "Diagnosis alone does not authorize
recovery") change in the same commit.

## Acceptance

Proof to execute:

- pytest over `pr_watch` fixtures: the signature matches its example (both 416
  and 206 forms); derived jobs are dropped; a mixed failure (a Nix job and a
  real test failure), an unknown failure, an unmatched job name, and a failed
  log read (also under `--once`, still `ejected`) are each unclassified.
- pytest through `dev pr-land` with scripted `gh`/`git`: an infrastructure
  ejection after arming re-enqueues once and merges; the same at preflight in
  ordinary mode; a second ejection is terminal whatever its cause; `--reenqueue`
  then an infrastructure ejection is terminal; an unclassified ejection is
  terminal without a second native action. Existing land guard tests keep
  passing.
- Replay: one land test drives the real observer and `pr-land` over GitHub
  responses recorded from #2016's ejection (run 44632169176, its jobs, a log
  excerpt), followed by scripted re-enqueue and merge. Its output is posted on
  #2139.
- `dev validate-no-e2e` green; CI green.

After merge (not blocking): the first organic infrastructure re-enqueue that
merges is recorded on #2139 with its observer report.

## Boundaries

- No retry loop, cross-invocation cap or registry; `pr-watch`'s outcomes and
  exit codes are unchanged (`ejection` is an added field).
- Only the one signature; each addition is a reviewed change with a real example
  job.
- `pr-cleanup.sh`, the ADR promoter's dequeue recovery and the ruleset are
  untouched.
- The out-of-repo `jaunder-ship` skill's "ejected is not automatically requeued"
  guidance is updated once this lands.
