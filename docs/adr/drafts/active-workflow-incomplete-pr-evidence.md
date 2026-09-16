# ADR-DRAFT: Active-workflow missing PR targets are incomplete evidence

- Status: proposed
- Date: 2026-09-16
- Issue: [#1543](https://github.com/jaunder-org/jaunder/issues/1543)

## Context

[ADR-0195](../0195-dynamic-pr-check-classification.md) requires the PR observer
to correlate every graph-derived required target with exactly one
current-attempt runtime job before using workflow ancestry. Missing correlation
entered the observer's poll-error and strike policy because uncertainty must
never classify a failed constituent as optional.

GitHub Actions does not necessarily materialize a `needs`-dependent aggregate
job while its prerequisites are still running. A failed constituent can
therefore require ancestry classification during a legitimate interval in which
the immutable workflow graph names the required aggregate but the current
runtime-job population does not contain it. Treating that bounded lifecycle
state as malformed evidence repeatedly interrupts autonomous observation even
though the owning workflow can still supply the missing evidence.

The observer must distinguish temporary incompleteness from a settled
contradiction without inferring from display names or the currently visible job
population.

## Decision

The exact owning Actions workflow run status determines whether an absent
graph-derived required target is incomplete or malformed.

While that workflow run is not `completed`, the absence is typed incomplete
evidence. Default observation reports the state through its bounded
change-only/heartbeat event discipline, spends no malformed-response strike, and
continues within the existing watch budget. One-shot observation reports
`pending`. The approval-bearing `pr land` pre-arm check also reports `pending`
and does not invoke the armer.

When the exact workflow run is `completed`, the same absence is malformed
evidence and follows the existing poll-error and strike policy. Unknown workflow
status values fail closed during evidence parsing. The policy is generic across
graph-derived required targets and introduces no name allowlist.

This refines ADR-0195's missing-correlation rule only for the lifecycle interval
in which the exact owning workflow remains active. Ruleset authority, immutable
graph authority, current-head and current-attempt binding, exact correlation,
and the prohibition on guessing optional classification remain unchanged.

## Consequences

- Early constituent failures remain caller-actionable as soon as their required
  aggregate materializes and exact ancestry can be established.
- A normal watch remains autonomous across GitHub's late-job materialization
  interval instead of turning expected incompleteness into repeated terminal
  watcher failures.
- `pending`, `checks-failed`, and `watcher-error` preserve distinct meanings:
  evidence may still arrive, a required failure is proven, or settled evidence
  is untrustworthy.
- Evidence acquisition must parse workflow run status fail-closed and bind it to
  the same run ID, attempt, workflow path, and head as the runtime jobs.
- `pr land` gains no new mutation capability; incomplete evidence is an explicit
  pre-arm refusal.
