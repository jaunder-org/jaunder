# Parallel Rust coverage execution implementation outline

> Execute with `jaunder-iterate`, using `jaunder-dispatch` for independent
> slices. This outline exists because the work changes a cache boundary,
> concurrent worker aggregation, and CI-to-Nix contracts.

## Scope

In:

- Two-worker baseline, slice, hash, and measurement-only backend-oriented
  coverage experiments.
- Exact worker evidence aggregation into the existing coverage status contract.
- A cacheable instrumented-build support boundary with machine-checked verdict
  isolation.
- Separate-runner CI measurement and conditional production adoption.
- Closure of #1473 only if the support boundary ships.

Out:

- Test annotations or production backend classification by generated test names.
- More than two workers or runtime-history scheduling.
- Doctest, e2e, browser-coverage, Elisp-coverage, or general validation fan-out
  changes.
- Closure of #1472.

## Task outline

- [x] Task 1: Define exhaustive two-worker evidence and aggregation
  - Contract: `tools/coverage` owns a versioned worker record containing
    partition identity, terminal outcome, JUnit population, profile artifacts,
    duration, and diagnostics; one aggregate retains the existing eight-stage
    `CoverageStatus` contract.
  - Contract: aggregation rejects missing or duplicate identities/artifacts,
    waits for every started worker, and preserves distinct simultaneous
    failures.
  - Verification: focused `tools/coverage` cases prove successful union
    reconciliation plus every single- and dual-worker failure required by the
    spec.

- [x] Task 2: Add an experiment-capable hermetic producer
  - Contract: `devtool coverage emit` remains the producer entry point; an
    explicit experiment mode runs the unpartitioned baseline, slice, hash, or
    identity-derived backend comparator with distinct worker JUnit/profile/log
    paths and one post-merge report path.
  - Contract: local two-worker mode supports both independent nextest defaults
    and a fixed aggregate concurrency budget without changing the default
    production mode before measurements qualify a treatment.
  - Verification: focused command-construction and orchestration cases prove
    unfiltered census authority, exact partition arguments, path isolation,
    wait-all behavior, and one report/LCOV/CRAP generation after profile union.

- [x] Task 3: Split and prove the instrumented support boundary
  - Contract: Nix separates source-complete instrumented test binaries and
    matching coverage metadata from all worker execution, profiles, reports, and
    verdict outputs; every consumed source/config/toolchain input contributes to
    the support identity.
  - Contract: the Cachix eligibility mechanism is derived from an enumerated
    output set plus closure checks, never names alone; broad exclusion remains
    until the proof succeeds.
  - Verification: a repository gate inventories support/final outputs, rejects
    every final coverage/e2e execution or verdict output from upload and
    cache-only substitution, proves support closures exclude them, and proves
    relevant source/configuration mutations invalidate support identity.

- [ ] Task 4: Measure local strategies under controlled concurrency
  - Contract: the measurement manifest records revision, hardware, declared
    cache state, alternating order, partition assignments, concurrency policy,
    compilation, orchestration, execution, merge/report durations, population,
    line/CRAP verdict, and aggregate resource consumption.
  - Verification: after explicit user confirmation that the system is unloaded,
    collect at least two comparable baseline and treatment observations for
    slice/hash under both concurrency policies and the measurement-only backend
    comparator; reject observations without the declared quiescent window.

- [ ] Task 5: Measure separate-runner CI fan-out
  - Contract: CI prepares or substitutes the support output once, runs two
    uncached per-ref workers on separate runners, and aggregates both worker
    artifacts into one uncached per-ref coverage verdict while retaining the
    required-check contract.
  - Verification: every CI observation uses Task 4's manifest schema and records
    cold and warm states separately. Collect at least two comparable baseline
    and two treatment observations for every CI surface used to justify
    adoption, plus enough repeated observations on the non-winning surface to
    evaluate the 10% regression limit. Runs use the same revision, equivalent
    runner class, declared build/cache state, and alternating order; final
    coverage/e2e outputs demonstrably execute for each tested ref.

- [ ] Task 6: Apply the conditional production decision
  - Contract: select a slice or hash treatment only when every correctness,
    isolation, concurrency, cache, timing, and regression condition in the spec
    passes; backend identity parsing cannot be selected.
  - Contract: if no treatment qualifies, remove fan-out wiring and the new
    support-boundary/cache-eligibility production wiring, restore the original
    producer derivation shape, CI topology, and broad Cachix filter, and retain
    only checked-in evidence and non-production probe/report material.
  - Verification: the checked-in comparison derives its decision from recorded
    observations and demonstrates exact merged coverage semantics. #1473 closes
    only if the cacheable support boundary and a qualifying slice/hash fan-out
    both ship with the required closure inventory, safety probe, and cache
    measurements; otherwise it remains open.

- [ ] Task 7: Certify the selected or rejected result
  - Contract: update `CONTRIBUTING.md` and applicable architecture/ADR material
    only for behavior that actually ships; archive the approved spec and outline
    at ship.
  - Verification: focused coverage/tool tests, the actual coverage producer
    scenario, `cargo xtask check`, `cargo xtask validate --no-e2e`, pull-request
    CI, and merge-group CI all pass on the delivered production state.

## Risk checks

- No worker or cache hit can make a missing test, profile, report, or final
  per-ref execution green.
- Support-output reuse cannot import a prior coverage/e2e execution or verdict
  through direct eligibility, runtime closure, or cache-only substitution.
- Instrumented objects and raw profiles share an exact compiler, coverage
  metadata, source, and path identity before merge.
- Worker concurrency does not double the intended local CPU budget unless that
  explicit treatment is being measured.
- Baseline/treatment cache state and ordering cannot systematically favor the
  treatment.
- CI fan-out preserves Nix hermeticity and the stable required coverage result
  consumed by `validate --no-e2e`.
- No quiescence-sensitive local benchmark starts until the user confirms the
  system is unloaded.
