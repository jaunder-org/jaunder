# Remove routine CI source-invalidation probes

## Outcome

Routine CI no longer runs the Nix or coverage source-invalidation probes. The
accepted frozen-184 measurement corpus remains immutable; subsequent source
commits have identities distinct from that measured baseline.

## Approach and tradeoffs

Remove the two automatic probe steps from `.github/workflows/ci.yml`, plus
Nix-probe-only result preservation and artifact references. Retain the existing
manual CLI commands and implementations, dormant unless explicitly requested.
This removes routine perturbation cost, not real validation or coverage. Add one
numberless proposed ADR draft narrowly superseding only ADR-0203's
automatic-probe clause, with a minimal `docs/ARCHITECTURE.md` projection and
corrections to false automatic-execution claims in `CONTRIBUTING.md`. Preserve
ADR-0203's remaining contracts and immutable Decision/status; do not
number/promote the draft or edit the generated `docs/README.md` index.

## Acceptance

Use the approved focused CI-shape regression in `xtask/src/gate.rs`: first
require both probe commands absent across all workflow jobs and preserved
validation lanes, demonstrate RED against the old workflow, then GREEN after the
smallest workflow change. Preserve all four validation lanes, real coverage,
tests, security, both-backend E2E producers, reconciliation and aggregate
verdicts. Use pinned tools through `devtool run`, retaining exact command exits
and parked evidence.

## Boundaries

No probe execution/replay, remote CI, benchmarks, full matrix, replacement
validator/framework, CLI cleanup, dependency or snapshot changes, push or merge.
No outline is needed. Commit the approved spec only after fresh soundness review
and its normal hook; leave implementation uncommitted for Lead diff review.
