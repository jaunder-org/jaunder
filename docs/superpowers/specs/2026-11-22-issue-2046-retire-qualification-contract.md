# Issue #2046 — retire the unused system-artifact qualification contract

## Outcome

`nix/system-artifact-qualification-contract.nix` is deleted. No gate, document
or tool references it, and the system-artifact qualification wiring stays proven
by the styling qualification harness that actually builds and runs it.

## Approach and tradeoffs

The contract is an evaluation-time file that string-matches the build scripts of
the qualification packages (`buildCommand` containing
`--system-artifact-fixture <name>`). Nothing evaluates it, so it rotted: the CSR
bundle became a `buildNpmPackage` (`buildPhase`, not `buildCommand`) and the
flag became `--fixture`, and evaluation now fails at line 34.

We delete it rather than repair it. The real consumer of
`nix/system-artifact-qualification.nix` is `qualification-host`
(`tools/qualification`, run by `dev production-baseline-qualify-styling`). For
each fixture `a`, `b-app` and `b-theme` it builds the qualification package and
its CSR bundle, loads the bundle's `system-artifacts` inventory, refuses an
inventory outside the bundle, checks that the CSR shell references exactly the
inventory's application stylesheet, and then proves each fixture's computed
presentation in a browser on SQLite and PostgreSQL. A missing
`--features qualification` makes `--fixture` an unknown argument and the bundle
build fails. A missing fixture flag yields the ordinary inventory for every
fixture, which the harness rejects before any VM starts: it requires fixture A
to equal the ordinary compiler inventory, `b-app`/`b-theme` to differ from A in
exactly the application/theme inventory, and the three products to be distinct
outputs. Unknown fixtures are refused three times: by the typed Rust
`QualificationFixture`, by the `throw` in
`mkSystemArtifactQualificationPackage`, and by `system-artifacts`' own fixture
parser.

This harness is opt-in (`dev production-baseline-qualify-styling`), not a CI
gate. After deletion no gate checks this wiring; the same was already true,
since nothing evaluated the contract.

Simplest alternative: repair the string matches and add the file to
`nix/checks.nix`. Rejected: it duplicates the harness with brittle matching on
shell text (the drift that broke it would recur on the next build-script edit)
and adds three qualification package graphs to every check evaluation.

Accepted limitation: two properties the contract asserted are no longer
mechanically checked outside the harness — that qualification bundles reuse the
ordinary `csrWasm` (a build-cost property, visible in `mkCsrWasmBundle`'s
default) and that ordinary `system-artifacts` is built without the
`qualification` feature (visible at `systemArtifactsBin`; the harness still
proves the qualification build has it).

## Acceptance

- The file is gone; `rg system-artifact-qualification-contract` finds nothing
  outside this spec.
- The pre-commit gate passes on the deletion.
- The PR body names the opt-in styling qualification harness as the covering
  proof and says it is not a CI gate.

## Boundaries

- No change to `nix/system-artifact-qualification.nix`, `nix/packages.nix`,
  `tools/qualification` or the harness's checks.
- No new Nix check, and no qualification run is required (the deletion changes
  no derivation).
