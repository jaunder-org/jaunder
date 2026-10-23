# ADR-DRAFT: Explicit one-shot PR re-enqueue

- Status: proposed
- Date: 2026-10-23
- Issue: [#1817](https://github.com/jaunder-org/jaunder/issues/1817)

## Context

[ADR-0087](../0087-xtask-github-pr-observation.md) separates observational
`pr watch` from approval-bearing `pr land` and refuses re-enqueue after an
ejection. Operators need a supported, explicit recovery invocation when they
judge that another enqueue is appropriate, without an automatic retry loop.

## Decision

`cargo xtask pr land <PR> --reenqueue` permits one existing armer call after an
observed ejection. Without the flag, the ejection refusal is unchanged. The
flagged invocation refuses queued or auto-merge-armed PRs and requires current
required checks and merge readiness. Head-divergence, observation-head,
mutation-head, GitHub queue rules, and fail-closed guards remain authoritative.

After arming, the invocation retains the historical failed run's URL. Only that
run is disregarded as an ejection discriminator; its failed verdict is not
changed. Delayed fresh-run visibility uses the existing observation deadline. A
distinct fresh failure is terminal. Neither uncertainty nor a silent mutation
no-op permits a second armer call or fallback to ordinary rearming. Existing
queue-disappearance and adverse-check outcomes remain intact.

This refines only ADR-0087 decision 4's blanket refusal of re-enqueue. Running
`land` remains merge approval and `watch` remains observational. Existing
observation and reporting, including merged-state handling, remain the
authority.

## Consequences

- Recovery requires considered explicit intent for each invocation; it does not
  certify that the prior failure was a flake.
- Invocation-local acknowledgement is not an authorization/provenance framework,
  persistent registry, cross-invocation cap, or altered check verdict.
- No operator-supplied head/run arguments or new merge-proof machinery are
  needed.
- Existing merge-group attribution limitations remain unchanged. Host
  fake-source regressions prove command behavior, not real GitHub queue
  operation.
