# ADR-0165: Playwright report defines trace-gate population

- Status: accepted
- Date: 2026-08-31
- Issue: [#831](https://github.com/jaunder-org/jaunder/issues/831)

## Context

A gate over E2E trace evidence must detect both incomplete records and an
entirely missing Playwright project. The trace cannot prove the absence of a
project when that project's capture is blacked out. Trusting trace rows
therefore makes the producer define its own gate population, contrary to the
fail-closed structural-membership rule in
[ADR-0110](0110-gate-population-membership-is-structural.md).

The expected population could be duplicated from Playwright configuration,
inferred from the trace, or derived from the Playwright JSON report already
produced by the same E2E combination. Duplicating configuration creates a second
project catalog. Trace inference cannot detect total omission. The report
records the projects that actually executed and is lifted with the capture
before the Playwright result is propagated.

## Decision

For host-side gates that reconcile Playwright execution with E2E trace evidence,
the lifted Playwright JSON report is the structural authority for the executed
project population. The consumer requires exact project-set reconciliation
between the report and trace-derived evidence; a missing or unexpected project
is an evidence-integrity failure.

Artifacts remain produced inside the Nix E2E derivation and analyzed by host
`xtask`, preserving [ADR-0028](0028-devtool-vs-xtask-boundary.md). Each
backend×browser combination is reconciled independently under
[ADR-0034](0034-ci-e2e-matrix-distribution.md). Artifacts are still lifted
unconditionally before a failed Playwright result is propagated under
[ADR-0037](0037-e2e-failure-diagnostics-capture.md); trace-derived gates run
only after the combination itself succeeds, so they cannot mask the primary
failure.

## Consequences

A project-wide trace blackout fails rather than disappearing from the
denominator, and stale or mixed project populations cannot inflate a gate
silently. Consumers must parse both the Playwright report and trace capture and
diagnose set differences explicitly.

The Playwright configuration remains the sole declaration of projects; the
report is runtime evidence of which declarations executed, not a second
configuration. A future gate needing finer membership than project identity must
add a similarly independent structural authority rather than trusting the
evidence stream it judges.

This decision does not change browser capture or trace attribution under
[ADR-0096](0096-e2e-trace-capture-vs-attribution.md), and it introduces no
ubiquitous-language change in `CONTEXT.md`.
