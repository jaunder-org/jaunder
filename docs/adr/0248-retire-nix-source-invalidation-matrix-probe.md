# ADR-0248: Retire the Nix source-invalidation matrix probe

- Status: accepted
- Date: 2026-11-15
- Issue: [#1946](https://github.com/jaunder-org/jaunder/issues/1946)

## Context

`just source-diagnostics nix-probe-source` evaluated ten derivation identities
before and after each of 30 one-file perturbations and compared the changed set
with a hand-written expectation table.
[ADR-0227](0227-on-demand-source-invalidation-probes.md) removed it from CI but
kept it as an on-demand diagnostic.

Unrun through #1898's intentional boundary changes, it failed on `main`
(e0382b474). A full run found four mismatch kinds across 19 arms, each a stale
expectation rather than a source-filter defect. What only this probe detected
was over-inclusion (needless rebuilds) and changed derivation wiring: cost
diagnostics that intentional refactors trip. A missing compile/embed input
already fails inside the Nix sandbox, filters carry inline membership asserts,
and whether tests run is assured by the real test, coverage and reconciliation
gates.

## Decision

Retire `nix-probe-source`, its expectation matrix and its private evaluation
helper, with no replacement. This narrowly supersedes only ADR-0227's
requirement to keep that manual command and its implementation available. Its
removal of automatic probe execution from CI, and the on-demand
`coverage-check probe-source` filter probe, remain as ADR-0227 decided.
`source-diagnostics cache-safety-probe` is unaffected. _Amended by #2103:_ that
probe has since been retired too
([CI honours non-substitutable verdicts](drafts/ci-honours-non-substitutable-verdicts.md)).

## Consequences

No tool watches Nix derivation boundaries for over-inclusion or wiring drift;
such cost regressions surface only through observed rebuilds. Boundary changes
no longer require maintaining a perturbation table. Source-filter membership
remains guarded by the filters' inline asserts and, for the coverage filter, its
probe.
