# Incomplete PR classification evidence

## Outcome

`cargo xtask pr watch` treats a graph-derived required workflow target that has
not materialized yet as incomplete evidence while its owning GitHub Actions
workflow is still running. Observation waits with an explicit diagnostic rather
than exhausting the watcher strike budget, while settled malformed evidence
continues to fail closed.

## Load-bearing decisions

- The immutable workflow graph and branch ruleset remain the authorities for
  required-target identity and dependency ancestry. The lifecycle refinement is
  recorded in `docs/adr/drafts/active-workflow-incomplete-pr-evidence.md`.
- The owning Actions workflow run's status is the sole authority for whether an
  absent graph-derived required target may still materialize. Job-list shape is
  not used as a settledness heuristic.
- When the owning workflow is not `completed`, an absent required target is
  incomplete evidence, not malformed evidence and not an optional check.
- Default observation emits a bounded diagnostic naming the absent target and
  owning workflow, consumes no malformed-response strike, and continues within
  the existing watch budget.
- One-shot observation returns `pending` with the same precise diagnostic.
- `pr land` returns `pending` from its pre-arm classification and does not
  invoke the merge armer while classification evidence is incomplete.
- When the owning workflow is `completed`, a still-absent required target
  remains malformed evidence and follows the existing poll-error and
  `watcher-error` policy.
- The rule applies generically to every graph-derived required target. No job,
  aggregate, workflow, matrix, backend, or browser name becomes an allowlist.
- Existing directly required, transitively required, optional, rerun,
  current-head, timeout, and merge-approval semantics remain unchanged.

## Acceptance

- A failed current-head constituent observed before a required aggregate job
  materializes remains in a non-terminal waiting phase while the owning workflow
  is running, without consuming the watcher strike budget.
- The waiting diagnostic identifies both the absent required target and the
  owning workflow and is bounded by the watcher's existing change-only/heartbeat
  event discipline.
- `pr watch --once` reports `pending`, rather than `watcher-error`, for that
  same incomplete state.
- `pr land` reports `pending` and the armer is observably never called in that
  state.
- Once the missing aggregate materializes, the same watch classifies the failed
  constituent through the existing direct, transitive, or optional graph rules.
- A workflow that reaches `completed` without its graph-derived required target
  produces malformed evidence and ultimately `watcher-error` under the existing
  strike policy.
- Tests cover running-workflow incompleteness, one-shot behavior, pre-arm
  refusal, later materialization, and settled malformed evidence through
  production evidence parsing and virtual-clock watcher behavior.

## Boundaries

- Do not weaken required-check or workflow-graph validation.
- Do not infer dependency ancestry or settledness from display names or the
  currently visible job population.
- Do not add workflow mutation, cancellation, rerun, or merge capabilities to
  observation.
- Do not change CI workflow topology, branch protection, aggregate jobs, or
  merge queue policy.
