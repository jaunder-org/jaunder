# ADR-0220: Explicit one-shot PR re-enqueue

- Status: accepted
- Date: 2026-10-23
- Issue: [#1817](https://github.com/jaunder-org/jaunder/issues/1817)
- Amended: 2026-11-26 — automatic one-shot re-enqueue after an infrastructure
  ejection ([#2139](https://github.com/jaunder-org/jaunder/issues/2139))

## Context

[ADR-0087](0087-xtask-github-pr-observation.md) separates observational
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

_Amended by #2139._ **One automatic re-enqueue for infrastructure.** Overnight
infrastructure ejections left PRs waiting for hours (#1859's retrospective: 15
of 17 check ejections were not the PR's fault). So `dev pr-land` also
re-enqueues by itself, through the same sequence, when the observer classifies
the ejection as a known infrastructure failure:

- The observer (`pr_watch`, read-only) classifies an `ejected` run and adds an
  `ejection` object to its report. Failed jobs that `need` another failed job of
  the same run (the aggregates and reconciliation) are derived, not evidence.
  Every other failed job must match a signature from a closed, reviewed list in
  source, each written from a real example job. The first is the Nix substitute
  failure (`unable to download …: HTTP error <n>` together with
  `… is required, but there is no substituter that can build it`, job
  135941369307). A mix with any unmatched job, or any failed read, is
  unclassified. Classification never changes the outcome.
- One re-enqueue per invocation, wherever the ejection is seen (preflight or
  after arming), shared with `--reenqueue`. Any later ejection is terminal,
  whatever its cause. The matched signature and run URL are printed before the
  action and recorded in the observer's report and trace.
- Runner loss and collateral ejection from the PR ahead are not signatures: no
  merge-queue ejection in the history showed either. Advisory failures stay
  terminal.

A wrong classification never merges an unchecked head, because the re-enqueued
group re-runs every required check; its cost is one wasted queue cycle, bounded
by the one-attempt rule.

This refines only ADR-0087 decision 4's blanket refusal of re-enqueue. Running
`land` remains merge approval and `watch` remains observational. Existing
observation and reporting, including merged-state handling, remain the
authority.

## Consequences

- Recovery for any cause other than a known infrastructure signature requires
  considered explicit intent for each invocation; it does not certify that the
  prior failure was a flake.
- Adding a signature is a reviewed source change with a real example job.
  Judging only jobs failed at classification time means a real failure still
  running when an infrastructure job fails gets one retry; the second ejection
  ends it.
- Invocation-local acknowledgement is not an authorization/provenance framework,
  persistent registry, cross-invocation cap, or altered check verdict.
- No operator-supplied head/run arguments or new merge-proof machinery are
  needed.
- Existing merge-group attribution limitations remain unchanged. Host
  fake-source regressions prove command behavior, not real GitHub queue
  operation.
