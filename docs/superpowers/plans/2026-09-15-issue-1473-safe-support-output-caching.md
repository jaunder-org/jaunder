# Issue 1473 Safe Support-Output Caching Implementation Outline

> Execute with `jaunder-iterate`, delegating bounded work with
> `jaunder-dispatch`. This outline exists because cache eligibility changes the
> CI/Nix correctness boundary for per-ref coverage and end-to-end verdicts.

## Scope

In:

- Machine-readable coverage/e2e output classification and closure inventory.
- A fail-closed safety probe over actual Nix derivation and output closures.
- Source-invalidation proof for each newly eligible support-output class.
- A narrowly changed Cachix policy only when the safety probe passes.
- Matched GitHub Actions measurements and a checked-in decision report.
- A draft ADR plus architecture projection only if the cache boundary changes.

Out:

- Changes to required CI contexts, gate populations, backend/browser coverage,
  or merge-queue behavior.
- New preparation jobs, cache services, application optimization, or unrelated
  Nix source-closure cleanup.
- Reuse of any result-bearing coverage/e2e output or lifted equivalent.

## Task outline

- [x] Task 1: Establish the cache-safety model and executable inventory
  - Contract: `nix/cache-policy.json` is the versioned machine-readable catalog
    consumed by later tasks. It classifies every coverage/e2e output identity as
    support or final/result-bearing, including aggregates and lifted
    equivalents; an inventory-derived check rejects unknown, duplicate, missing,
    or unreachable classifications when reconciled with the Nix definitions.
  - Contract: `cargo xtask cache-safety probe` queries actual derivation/output
    closures without realizing a final verdict merely to inspect it, and proves
    that every eligible support closure excludes every final output and
    equivalent result-bearing artifact.
  - Contract: retain the broad Cachix filter throughout this task; do not rely
    on output names, `allowSubstitutes`, or `preferLocalBuild` without a probe
    that demonstrates the required behavior.
  - Verification: focused unit/fixture tests include admitted support,
    deliberate transitive leakage, incomplete inventory, and malformed policy
    arms; an exhaustive assertion derived from `nix/cache-policy.json` rejects
    every classified final output, aggregate, and lifted equivalent.

- [ ] Task 2: Apply and guard the candidate cache boundary
  - Contract: policy generation or validation consumes the exact
    `nix/cache-policy.json` schema and successful
    `cargo xtask cache-safety probe` verdict from Task 1; no second
    classification source is introduced.
  - Contract: narrow the Cachix filter only for support outputs admitted by that
    structural proof; independently force final outputs to execute locally when
    validated Nix semantics provide that defense without replacing the closure
    proof.
  - Contract: each admitted support-output class has paired source probes
    proving relevant inputs alter its derivation identity and unrelated inputs
    preserve it.
  - Contract: the checked-in machine inventory and concise human explanation are
    generated or cross-checked from one authoritative output catalog so policy
    and Nix definitions cannot drift independently.
  - Verification: the focused cache-safety and source probes pass, then
    `devtool run -- cargo xtask check --no-test` exercises the integrated static
    surface without accepting a cached final verdict.

- [ ] Task 3: Measure the candidate and select the durable outcome
  - Contract: gather matched baseline/treatment observations for all four
    source-changing/narrow × cold/warmed combinations, with two pairs per cell
    and the spec's numeric rule deciding whether a third pair is required.
  - Contract: evidence records run/ref identity, cache state, actual
    substitution of every designated support output, fresh final-derivation
    execution, required-check wall-clock, aggregate runner time, transfer/setup
    costs, exclusions, and comparability limits. A run lacking substitution or
    final-execution evidence is excluded; an incomplete measurement cell
    restores the broad filter.
  - Contract: keep the narrower policy only if one cell improves by at least 10%
    on either axis and no cell regresses by at least 10% on either axis.
    Otherwise restore the broad filter while retaining the inventory, probes,
    and measured rejection report.
  - Verification: fresh pull-request and merge-group runs show every final
    coverage/e2e verdict executing for the tested ref; the checked-in report's
    calculations reproduce from its cited observations.

- [ ] Task 4: Record the accepted cache boundary
  - Contract: if Task 3 retains a narrower policy, write a numberless draft ADR
    and project it into `docs/ARCHITECTURE.md`; if the broad filter remains,
    record the rejection only in the issue report because no architecture
    decision changed.
  - Verification: documentation links, ADR projection checks when applicable,
    and the final Standards/Spec review all pass.

## Risk checks

- No support closure can upload, transport, or substitute a final verdict.
- Final Rust coverage and every backend/browser e2e result execute per tested
  pull-request, push, and merge-group ref.
- Coverage status, e2e traces, diagnostics, server-function evidence, and
  zero-panic results cannot be mistaken for reusable preparation.
- Inventory population is derived and fail-closed when Nix adds or renames an
  output.
- Source probes cover both under-invalidation and unnecessary invalidation.
- Measurements include cache transfer and duplicated-runner costs and never pool
  unlike change/cache classes.
- A failed safety proof or sub-threshold treatment restores the current broad
  exclusion rather than weakening the acceptance rule.
