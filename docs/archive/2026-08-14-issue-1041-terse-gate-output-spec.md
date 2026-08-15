# Terse successful e2e-goto-wrapper output

- Issue: [#1041](https://github.com/jaunder-org/jaunder/issues/1041)

## Problem

A passing `e2e-goto-wrapper` gate renders its seven-site exemption census. These
non-actionable rows resemble diagnostics and make routine `cargo xtask check`
output require gate-specific interpretation.

## Decision

Keep deriving the exemption census from the source scan. Render it with failure
diagnostics, where it supplies investigation context; omit it from successful
step output.

## Acceptance criteria

- A successful `e2e-goto-wrapper` step has no detail payload and renders only
  `[ ok ] e2e-goto-wrapper`.
- A failing step retains its actionable problems, recovery guidance, and derived
  exemption census.
- Automated coverage prevents success detail from returning.
- ADR-0094 states that clean gates stay terse and derived censuses accompany
  failures.
