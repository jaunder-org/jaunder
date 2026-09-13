# ADR-DRAFT: Split CI non-e2e validation into core and coverage lanes

- Status: proposed
- Date: 2026-09-13
- Issue: [#1472](https://github.com/jaunder-org/jaunder/issues/1472)

## Context

[ADR-0034](../0034-ci-e2e-matrix-distribution.md) distributed the e2e matrix but
kept all non-e2e validation in one `cargo xtask validate --no-e2e` CI job. That
job remained the workflow critical path on both narrow and source-changing
measurements. Its Rust coverage surface was independent of the other host and
Nix validation surfaces, so serial execution added latency without sharing a
required artifact or verdict.

Issue #1472 measured a two-lane treatment against repeated completed GitHub
Actions runs. The cache-state-matched narrow comparison reduced median workflow
wall-clock by 322 seconds (21.8%) with the runner-time proxy effectively flat,
clearing both numeric retention thresholds. The source-changing observations
showed a directional 304-second (16.1%) reduction, but their successful baseline
mixed a first attempt and a same-head warmed rerun, so they were not used to
qualify the decision. Their runner-time proxy increased by 291 seconds (4.4%).

Branch protection and the merge queue require the stable `Validate (no e2e)`
context. Final Rust coverage and e2e verdicts must remain per-ref and cannot be
accepted from Cachix.

## Decision

CI runs non-e2e validation as two independent full-VM jobs:

- `Validation core` runs the verify-only host/static surface, wasm budget, Nix
  static proof, wasm tests, Nix doctests, and Elisp coverage producer/consumer.
- `Validation coverage` runs Rust coverage and its gate.

Both jobs execute through `cargo xtask ci-validate <core|coverage>`. The lane
commands and local `cargo xtask validate --no-e2e` select from one ordered
validation-surface catalog so the distributed and local definitions cannot drift
independently. Each lane retains the clean-tree precondition and normal command
lifecycle. The workflow runs the Nix source-closure probe after core and the
coverage source-drift probe after coverage.

A result-only `Validate (no e2e)` job depends on both lanes and succeeds only
when both succeeded. Its name remains the branch-protection contract. No
preparation job or artifact-transfer edge connects the lanes, and neither lane
reuses a final coverage verdict from Cachix.

The `{backend}×{browser}` e2e matrix and `e2e gate` are unchanged. Local
`cargo xtask validate --no-e2e` and full `cargo xtask validate` remain
single-command gates whose non-e2e catalog executes serially; full validation
retains ADR-0034's Nix-concurrent e2e combinations.

This narrowly supersedes [ADR-0034](../0034-ci-e2e-matrix-distribution.md)
decision 2 only where it specifies one non-e2e CI job. ADR-0034's distributed
e2e matrix, stable aggregate contexts, and local full-gate decisions remain
current.

## Consequences

- Good: the cache-state-matched narrow comparison reduces median workflow
  critical-path time by more than five minutes.
- Good: coverage is independently visible and no longer serializes unrelated
  validation surfaces.
- Good: branch protection keeps the stable `Validate (no e2e)` context; matrix
  or lane implementation names remain internal.
- Neutral: workflow success is the conjunction of two lane verdicts rather than
  one `validate --no-e2e` process. The shared catalog preserves surface parity.
- Cost: source-changing observations consumed 4.4% more aggregate runner time in
  the mixed-cache-state comparison; narrow observations were flat.
- Cost: two jobs repeat runner setup and the clean-tree precheck.
- Constraint: a future lane split must identify a genuinely independent surface
  and preserve all producer/consumer and per-ref verdict boundaries. Fan-out
  without measured critical-path benefit is not justified.
