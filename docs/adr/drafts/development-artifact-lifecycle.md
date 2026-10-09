# ADR-DRAFT: Keep development history out of the current source tree

- Status: proposed
- Date: 2026-10-09
- Issue: [#1712](https://github.com/jaunder-org/jaunder/issues/1712)

## Context

[ADR-0000](../0000-documentation-strategy.md) calls for deleting completed
transient documents while preserving history. The shipping workflow instead
accumulated archived plans, research reports and qualification results. A gate
also checked the delivery ledger of a completed error-handling audit rather than
current source behavior. These records burden discovery without preventing
regressions.

[ADR-0185](../0185-reason-bearing-coverage-exceptions.md) already separates
point-in-time audit evidence from source-local reasons and executable policy.
The same distinction applies to development artifacts generally.

## Decision

Commit approved specs and necessary outlines before implementation, maintain
them through conformance review, and delete them in a final cleanup commit.
Jaunder retains those commits through non-squash merges. Preserve enduring
contracts and decisions in maintained documentation and ADRs; preserve
unresolved work in issues. Historical citations use commit-pinned links rather
than requiring historical files in the tip.

Keep execution evidence in ignored run/session storage or external review
artifacts. Qualification reports use `.xtask/production-baseline-reports/`,
separate from the restricted raw workspaces. Publication still validates schema,
Markdown parity, sanitization and collision refusal. Explicit release/compliance
retention uses a designated external store, owner and retention period.

Keep regression tests, minimal maintained fixtures, intentionally compared
baselines, compatibility corpora and current procedures. A gate checking only an
old delivery ledger does not turn that ledger into a behavioral protection;
retire such bookkeeping without removing the actual behavior tests or policy.

## Consequences

Proof requirements remain unchanged; evidence collection does not imply
permanent source retention. Completed working documents are recovered from Git
when needed, while local execution evidence may expire after review.
Contributors inspect new documents/data for an ongoing consumer at
specification, commit and review boundaries. No replacement evidence inventory
or archive is maintained.

This applies ADR-0000's transient-document rule and extends the ADR-0185
evidence boundary; it does not retire the architectural decision log.
