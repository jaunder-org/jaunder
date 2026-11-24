# Rewrite PR observation as a native `dev pr-watch` (Python)

## Outcome

`dev pr-watch` observes a PR natively inside the Python `dev` runner instead of
forwarding to the Rust `tools/pr-observer` binary, which is deleted. Its CLI,
outcomes, exit codes, JSON report and stderr progress keep their contract, so
`scripts/pr-land.sh` works unchanged, and docs change only where a feature below
is deliberately dropped. Production code shrinks from about 5,300 lines of Rust
to an expected few hundred lines of Python.

## Feature audit result

Kept (ADR-0087, 0135, 0220, 0240 invariants):

- Flags `[N]`, `--interval`, `--timeout`, `--once`, `--until merged`,
  `--expected-head`, `--acknowledged-ejection`, `--observed-queued-head`, with
  their validation; refusals exit 2 with empty stdout.
- Outcome precedence: merged, closed-unmerged, conflicted, checks-failed, stale
  (strict + BEHIND), empty ruleset → watcher-error, ejected, then
  queued/armed/awaiting-checks phases, dequeued, blocked, awaiting-mergeability,
  ready-to-land. `--until merged` continues past readiness; `--once` returns
  `pending` with `phase`.
- Ruleset read every run: required contexts, strict policy, merge queue present.
- Check states: CheckRun passes on SUCCESS/NEUTRAL/SKIPPED, is pending until
  COMPLETED, otherwise fails; StatusContext passes on SUCCESS, fails on
  FAILURE/ERROR. Same-named checks: a pending re-run outranks an earlier
  failure, else the latest completion wins. Only checks on the current head
  count. Readiness requires every ruleset context to have succeeded.
- Ejection probe (merge-group run by `gh-readonly-queue/main/pr-<N>-` prefix,
  created after the head commit, failed, or in progress with a failed job) and
  `--acknowledged-ejection` disregarding only that run URL.
- Expected-head binding (`stale` on drift, even when merged); dequeue from
  same-head queue history, reset on a new head, confirmed by one immediate
  re-poll; `--observed-queued-head` seeds that history.
- Strike budget: 5 consecutive transient errors; a non-transient error, or any
  error in `--once`, is terminal `watcher-error`; deadline → `timed-out` if any
  read succeeded, else `watcher-error`.
- `gh` result classification: missing, unauthenticated, not-found, GraphQL
  `errors` even on exit 0, rate-limited, transport, malformed; transient =
  rate-limited, transport, malformed.
- Change-only stderr progress, 10-minute heartbeat, `events` in the report.

Replaced: the job-graph classifier (matrix expansion, reusable-workflow
inlining, `jq` catalog re-execution, ADR-0198 incomplete evidence and the
`awaiting-classification-evidence` phase) → the membership rule below.

Dropped: shared-failure enrichment and the `shared_failure` field (no consumer,
never changes a verdict); rate-limit reset lookup (a rate limit is a transient
strike); the separate pre-loop establishment snapshot (folded into the first
poll); unused commit-checks query, test-only helpers, hand-written RFC3339, the
10-second deadline transport, and the `ci.yml` `fail-fast` lint test.

## Approach and tradeoffs

- **Location.** A module of the `tools/dev` package (`dev/pr_watch.py`, plus a
  small `gh` module if it earns one) implements the `pr-watch` subcommand
  in-process: no separate package, Nix build step or binary. ruff,
  `mypy --strict` and 100% branch coverage apply. The module issues only
  read-only `gh` calls; `scripts/pr-land.sh` stays the only armer.
- **Transport.** Each `gh` call is a traced step that captures exit code, stdout
  and stderr without failing `dev`; a pure `classify(code, stdout, stderr)`
  turns that into data or a typed error (ADR-0087's run/classify split).
  `steps.output` is not used because it aborts on nonzero exit. Decision rules
  are pure functions over parsed snapshots with an injected clock and sleeper.
  An internal defect (uncaught exception) exits 2, never 1, so `pr-land.sh`
  can't mistake it for a verdict.
- **Early failure: membership rule.** A failed check named by the ruleset is
  `checks-failed`, as before. A failed non-Actions check not in the ruleset is
  optional. A failed GitHub Actions check not in the ruleset is _required_ when
  its workflow file defines a job whose literal `name` (or key, when unnamed)
  equals a ruleset context. The watch then ends `checks-failed` with that
  check's URL as `pointer`. Otherwise it is optional: one `warning` event per
  run attempt, and observation continues. The workflow path and run attempt come
  from the existing rollup query (`workflowRun { runAttempt file { path } }`).
  The file is read at the PR head commit with `gh api contents`, cached per
  (path, commit), and parsed with PyYAML. If the file is unreadable or
  unparseable, or its path is not under `.github/workflows/`, the failure is
  optional, not a poll error. Misjudging a check as optional only delays
  `checks-failed` until the required aggregate itself fails, and readiness still
  needs every aggregate green. No check, job or workflow name is hard-coded.
  - Alternative: wait for the `always()` aggregates. Rejected: it gives up the
    early stop on a failed e2e lane that ADR-0195 bought.
  - Accepted costs:
    - A failed optional job placed inside a workflow that also defines a
      required context ends the watch (errs toward blocking).
    - A required context named by an expression, a matrix or a reusable workflow
      never matches, so its siblings only delay `checks-failed`. Don't
      reintroduce graph code to "fix" this.
    - `pull_request` runs actually use the merge ref's workflow file, so reading
      it at the head commit is an approximation that affects only latency.
    - PyYAML (`ps.pyyaml` in `devPythonDeps`, `pyyaml` in `pyproject`
      dependencies, `types-PyYAML` for the mypy check) joins the `dev` closure.
- **Cutover.** One PR removes:
  - `tools/pr-observer` and its Cargo workspace member and lock entries;
  - the `nix/packages.nix` package and its `nix/platform-contract.nix` entry;
  - `commands.PR_OBSERVER` and the forwarding call;
  - the `tools/doctests/src/harness.rs` testdata comment.

  Only one observer exists at any commit. In `test_pr_frontdoors.py`, land tests
  put a fake `dev` first on the PATH of the `pr-land.sh` child that replays
  observer reports. This keeps land tests independent of observer internals.

- **Decision record.** A new ADR draft,
  `docs/adr/drafts/pr-watch-native-python-observer.md`, does three things:
  - It supersedes ADR-0195's and ADR-0198's classification mechanism, restating
    the rules from them that still hold (the check-state rules in the Kept list
    above).
  - It amends ADR-0240: the observer is a read-only `dev` module, not a `tools/`
    executable. Approving this spec is the explicit human approval that ADR-0240
    requires for changing 0195/0198 guarantees.
  - It amends ADR-0258: `pr-observer` leaves the Nix-built host tool list.

  The draft is projected into `docs/ARCHITECTURE.md` (crate list :78 and the
  PR-observation sections). CONTRIBUTING's PR-watching section is updated to
  match: the binary at :1578, incomplete-evidence `pending` at :1612, "never
  guessed optional" at :1598, the `Progress`/`WatchContinuation` type names, the
  `shared_failure` field and the transitive-graph prose. The README content
  moves there or into the module docstring. The jaunder-ship skill's
  `shared_failure` mention lives outside this repo and becomes a follow-up note.

## Acceptance

- `dev pr-watch` runs without building any Nix package. `rg pr-observer` over
  `tools/ nix/ scripts/` finds nothing.
- Pure tests over the decision code, and behaviour tests that drive
  `dev pr-watch` with a fake `gh` (the `test_issue_frontdoors.py` pattern),
  cover every Kept item. That includes:
  - every outcome and its exit code; argument refusals; internal defect → exit
    2;
  - strike exhaustion, transient recovery, rate limit as a strike, timeout vs.
    never-read;
  - `--once` pending and terminal; `--until merged` from readiness to merged;
  - ejection with and without acknowledgement, including in progress with a
    failed job;
  - dequeue, its confirmation re-poll and the head reset; expected-head `stale`
    even when merged;
  - `--observed-queued-head` mismatch refusal and dequeue before the first poll;
  - change-only progress and heartbeat;
  - ruleset-required, membership-required and optional failures; an unreadable
    workflow treated as optional; an empty ruleset failing closed.
- `test_pr_frontdoors.py` still proves `pr-land.sh` against the report shape
  (`pr` number, `head_sha`, `outcome`, `pointer`, `detail`). One behaviour test
  runs `pr-land.sh` → real `dev pr-watch` end to end once.
- The PR description lists all 190 Rust tests by name, each either "ported →
  <pytest name>" or "dropped → <reason from this spec>". Drops are limited to
  the Replaced/Dropped features above (workflow.rs 10, evidence.rs 15,
  shared_failure.rs 9, plus the related tests in other files).
- `dev check` green; pre-PR `dev validate-no-e2e` green.

## Boundaries

- No change to `pr-land.sh`'s logic, `pr-cleanup`, merge/arming behaviour,
  ruleset or `ci.yml`.
- No new outcomes, flags or report fields; no renamed outcome strings.
- No generic GitHub client or reusable polling framework beyond what `pr-watch`
  needs.
