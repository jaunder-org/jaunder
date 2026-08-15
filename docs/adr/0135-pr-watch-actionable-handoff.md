# ADR-0135: PR observation stops at the next caller-actionable state

- Status: accepted
- Date: 2026-08-14
- Issue: [#1044](https://github.com/jaunder-org/jaunder/issues/1044)

## Context

ADR-0087 separated read-only PR observation from merge approval: `pr watch`
cannot mutate, while running `pr land` is the approval that arms auto-merge. The
original watcher nevertheless waited only for a terminal GitHub state.

That stopping rule deadlocks the ordinary pre-approval workflow. After required
checks turn green, an unarmed and unqueued PR cannot progress until `pr land`
runs, but the agent waiting inside `pr watch` cannot reach the approval halt
that permits `pr land`. The watcher can diagnose the state, but `devtool run`
parks its stream until the child exits, so the diagnosis does not reach the
caller before the 90-minute timeout.

There is still a legitimate passive-observation case: another actor may approve
and arm a PR while an observer follows it. Making that exceptional case the
default imposes its blocking behavior on every ordinary shipping cycle.

## Decision

Default `cargo xtask pr watch` observes until the next state requiring caller
action. After existing adverse verdicts take precedence, an open, green,
unarmed, unqueued PR whose non-empty gate and GitHub merge state establish that
it can land returns the successful `ready-to-land` outcome. A same-head queue
entry that vanishes without a matching failed current-head merge-group run
returns the distinct `dequeued` outcome instead of warning and continuing; a
head change resets that queue history.

GitHub states that explicitly prevent merge return `blocked`; indeterminate
mergeability continues as `awaiting-mergeability`. An empty required-check set
fails closed as `watcher-error` because it cannot establish readiness. `pr land`
applies the same empty-gate refusal before invoking its arming capability.

An already armed or queued PR still continues through merge-queue processing
because no caller action is needed. `cargo xtask pr watch --until merged`
explicitly requests passive observation across `ready-to-land` while another
actor may arm the PR; adverse or action-requiring outcomes still end that watch.

The capability seam from ADR-0087 remains unchanged: every `pr watch` mode is
read-only, and `pr land` alone acquires the arming capability. Running `pr land`
remains the merge approval.

## Consequences

- The ordinary flow becomes finite at the structural approval gate: watch to
  `ready-to-land`, obtain approval, then run `pr land`.
- `ready-to-land` is exit 0 because the observer successfully reached its
  intended handoff. `pr land` separately retains exit 0 only for `merged`;
  result success is command-specific rather than a global synonym for merging.
- Passive waiting remains available but must be requested explicitly with
  `--until merged`, making its potentially full-timeout behavior visible at the
  call site.
- The result vocabulary gains `ready-to-land`, `dequeued`, and `blocked`; these
  preserve the distinctions among a successful approval handoff, an observed
  unexplained same-head dequeue, a proven failed merge-group ejection, and a
  GitHub state that explicitly prevents merge.
- ADR-0087 remains accepted: xtask ownership, `gh` transport, ruleset-derived
  gate shape, and the observer/armer split are unchanged. This decision refines
  only the observer's stopping rule and result success semantics.
