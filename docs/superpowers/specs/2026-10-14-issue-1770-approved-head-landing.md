# Bind PR landing to the checked and approved head (#1770)

## Outcome

`cargo xtask pr land` must never arm a PR head other than the exact head checked
and qualified by that invocation's approval, including its retry. A remote head
change requires a fresh qualified landing invocation; it must not inherit the
old head's approval.

## Load-bearing decisions

- Establish one expected PR head SHA for the landing invocation and retain it
  through the local divergence guard, pre-arm reads, mutation, arm verification,
  and any re-arm. Subsequent reads cannot replace that approval identity.
- Refuse observed head drift before initial arming or re-arming, with a
  non-success typed result and a diagnostic identifying expected and observed
  heads. Unreadable evidence must not authorize mutation.
- Every arming attempt must supply the same expected SHA through GitHub's
  supported expected-head condition (`gh pr merge --match-head-commit SHA`). A
  prior matching read alone is insufficient to close the read/write race.
- Retain actual GitHub auto-merge/queue readback rather than treating command
  success as proof of arming; a different head's state cannot verify this arm.
- Preserve the existing observer/armer capability separation, per-PR human
  approval gate, required-check classification, ejection refusal, bounded re-arm
  behavior, and command-specific success semantics.
- `pr watch` remains observation-only and may observe changing remote heads; it
  does not gain approval, mutation, or automatic re-approval capability.

## Acceptance

- Deterministic tests drive the real landing execution with fake PR input,
  arming, and time boundaries. A checked head A followed by pre-arm head B
  results in no arming call and an explicit non-success refusal.
- A head change after a no-op first arm prevents the retry from arming B.
  Recorded arming requests never change their expected SHA from A.
- A mutation-boundary head mismatch is rejected using the expected-head
  condition; a test verifies the production command carries that condition.
- Different-head arm readback cannot establish successful arming for A or
  authorize a retry on B. Unreadable readback remains fail-closed.
- Unchanged-head initial arm, direct enqueue, and permitted one-time re-arm
  still verify through readback and can reach `merged` in deterministic tests.
- Existing observation-only and adverse-result regressions remain intact.
  Focused host xtask tests provide proof; outputs remain ignored evidence, not
  committed delivery reports.

## Boundaries

No live merge/requeue test, live GitHub mutation campaign, CI/ruleset change,
backend/browser/coverage weakening, approval revocation for unrelated PRs,
external skill edits, or runtime/Herdsman ownership framework. This addresses
current source correctness, not a demonstrated historical wrong-head merge.
