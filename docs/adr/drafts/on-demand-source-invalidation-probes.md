# ADR-DRAFT: Run source-invalidation probes only on demand

- Status: proposed
- Date: 2026-10-31

## Context

Routine CI runs source-invalidation probes in addition to real validation.
Controlled source perturbations are diagnostic work, not product test or
coverage verdicts; the owner approved removing their routine execution.

## Decision

Remove automatic `cargo xtask nix probe-source` and
`cargo xtask coverage probe-source` execution from CI, including Nix-probe-only
result preservation and artifact references. Keep both manual commands and their
implementations available for explicitly requested diagnostics.

This narrowly supersedes only
[ADR-0203](../0203-distribute-ci-validation-and-firefox-e2e-lanes.md)'s
requirement that `test-checks` run the source-closure probe under `always()` and
fail on its separate verdict. Its four validation lanes, real test and coverage
verdicts, producer/consumer ownership, backend parity, E2E reconciliation and
stable aggregate contexts remain unchanged.

## Consequences

Routine CI avoids source-perturbation cost and no longer automatically detects
source-filter drift through these probes. Real validation remains mandatory;
manual diagnostic commands are not replacement validation gates.
