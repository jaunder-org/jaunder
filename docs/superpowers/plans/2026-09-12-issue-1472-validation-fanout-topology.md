# Validation Fan-Out Topology Implementation Outline

> Execute with `jaunder-iterate`, using `jaunder-dispatch` for isolated work.
> This outline exists because the experiment changes a required-check
> architecture and adds a CI-only xtask interface.

## Scope

In:

- One evolving PR containing baseline, treatment, and final evidence.
- A two-value `cargo xtask ci-validate <core|coverage>` interface backed by the
  existing validation orchestration.
- Two independent full-VM validation jobs and one stable result-only aggregate
  job during the treatment.
- Repeated source-changing, narrow-change, and warmed Actions observations.
- A threshold-based final state: retain and document the topology, or revert it
  and record the measured rejection.

Out:

- Coverage backend partitioning (#1474).
- Cachix support-output eligibility changes (#1473).
- E2E matrix changes, host-gate step parallelism, serial preparation, and
  inter-lane artifact transfer.

## Task outline

- [x] Task 1: Establish the comparable baseline
  - Contract:
    `docs/superpowers/research/2026-09-12-issue-1472-validation-fanout.md` owns
    run selection, timings, exclusions, and the baseline/treatment comparison
    table. Temporary source and documentation markers are committed only long
    enough to obtain immutable Actions runs.
  - Verification: at least two successful baseline observations for each
    compared class; exact run/head/attempt identities; workflow, validation,
    setup, probe, e2e-critical-path, aggregation, and runner-time-proxy fields
    reconcile.

- [x] Task 2: Add the two-lane validation module
  - Contract: `CiValidateLane::{Core,Coverage}` is the complete public choice;
    `cargo xtask ci-validate core|coverage` is verify-only and host-only.
    Private orchestration shared with full `validate --no-e2e` makes the two
    lane catalogs duplicate-free and exhaustive without changing local
    validation behavior.
  - Verification: focused xtask tests prove CLI parsing, lane membership,
    full-validation equivalence, ordered host checks, fail-closed results, and
    lane-specific diagnostic ownership; `devtool run -- cargo xtask check`
    passes before commit.

- [x] Task 3: Fan out the Actions validation jobs
  - Contract: internal `Validation core` and `Validation coverage` jobs run
    independently on pinned `ubuntu-24.04`; result-only `Validate (no e2e)` on
    `ubuntu-slim` depends on both with `if: always()`. Existing
    branch-protection and `cargo xtask pr` check names remain unchanged. E2E
    jobs and `e2e gate` are byte-for-byte unchanged.
  - Verification: workflow structure tests pin triggers, job dependencies,
    runner classes, commands, probes, diagnostics, and the stable aggregate
    result; both CI lanes execute successfully on the treatment head.

- [x] Task 4: Measure treatment and decide
  - Contract: use the report's fixed protocol and temporary marker shapes;
    collect at least two successful treatment observations per compared class
    and identify warm reruns explicitly. Compare medians and individual runs;
    report runner consumption separately.
  - Verification: every cited run is completed and green; timings reconcile to
    GitHub job/step timestamps and available xtask phase sidecars;
    failed/cancelled/infrastructure-invalid runs are excluded from threshold
    arithmetic.

- [x] Task 5: Materialize the measured decision
  - Contract: if the repeated comparison clears 10% or three minutes without
    moving another required check onto the critical path, retain the lanes, add
    an ADR narrowly superseding ADR-0034's one-job description, and project it
    into `docs/ARCHITECTURE.md` plus CI operations documentation. Otherwise
    remove the workflow and xtask experiment and land only the report's measured
    rejection. Remove every temporary marker in either case.
  - Verification: the final diff contains no experiment-only marker or stale
    command; required surfaces and per-ref coverage/e2e semantics are accounted
    for; `devtool run -- cargo xtask check` passes before the final commit and
    PR CI proves the final topology.

## Ordering dependencies

1. Baseline runs precede treatment implementation so the baseline cannot benefit
   from treatment topology or treatment-populated final outputs.
2. The xtask lane interface precedes workflow fan-out because both Actions jobs
   consume it.
3. Treatment runs precede the retain/revert decision and any accepting ADR.
4. Final cleanup and documentation precede the landing gate; historical Actions
   URLs survive rewritten experiment commits.

## Risk checks

- The lane union equals the full non-e2e validation inventory; no duplicate or
  omitted surface.
- `Validate (no e2e)` remains the sole stable required validation context and
  fails unless both lanes pass.
- Coverage producer, Nix gate, host verdict, source-drift probe, status, and
  diagnostics remain in one owning lane.
- Core preserves host-gate ordering, doctest and Emacs producer/consumer pairs,
  and the Nix source-closure probe.
- Final Rust coverage and all four e2e verdict derivations remain per-ref and
  Cachix-ineligible.
- `pull_request`, `push main`, and `merge_group` run the same required graph.
- The experiment reports duplicated checkout/setup/Nix costs and runner
  consumption; it does not relabel billing reduction as wall-clock improvement.
- A retained topology explicitly reconciles ADR-0034, ADR-0077, and ADR-0178; a
  rejected topology adds no architectural decision.
