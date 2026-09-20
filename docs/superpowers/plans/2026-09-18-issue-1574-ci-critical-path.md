# CI Critical-Path Reduction Implementation Outline

> Execute with `jaunder-iterate`, delegating bounded tasks through
> `jaunder-dispatch` when useful. This outline exists because the issue changes
> the distributed validation architecture, the CI/local authority contract, and
> multi-lane e2e evidence reconciliation.
>
> Authoritative spec:
> [issue #1574](https://github.com/jaunder-org/jaunder/issues/1574).

## Scope

In:

- Distribute non-e2e core validation without changing its complete surface.
- Run the Nix source-closure probe on the Nix-heavy test-checks runner while
  preserving its independent result and failure signal.
- Prototype and retain qualifying Firefox e2e partitioning with isolated VMs.
- Reconcile test ownership, reports, retries, duration evidence, traces, and
  diagnostics across e2e lanes.
- Measure cache-matched workflow latency and runner cost before retaining either
  treatment.
- Record the accepted architecture as a numberless ADR draft and update
  `docs/ARCHITECTURE.md`.

Out:

- Weakening any validation/e2e verdict or caching a per-ref final verdict.
- Shared mutable e2e databases or storage, routine WebKit CI, or recombining
  `static-docs` and `static-code`.
- Unrelated individual-test performance work.

## Task outline

- [x] Task 1: Define and distribute the non-e2e validation lanes.
  - Contract: replace the two-value CI lane selection with one exhaustive,
    duplicate-free partition of the existing `NON_E2E_VALIDATION_SURFACES`: host
    (`HostGateWithoutTests`, `HostTests`), hermetic (`NixStaticChecks`,
    `WasmBudget`), test checks (`WasmTests`, `Doctests`, `ElispCoverage`), and
    the existing independent Rust coverage lane. Local `validate --no-e2e`
    continues to execute the complete ordered catalog serially.
  - Contract: keep doctest producer/gate and Elisp producer/consumer within
    their owning test-check lane; every lane retains the clean-tree precheck and
    ordinary command lifecycle.
  - Contract: `.github/workflows/ci.yml` runs the source-closure probe under
    `always()` after preserving the test-check result, keeps both diagnostics,
    and retains `Validate (no e2e)` as the sole stable result context over host,
    hermetic, combined test-check/probe, and coverage jobs.
  - Verification: catalog tests prove complete/duplicate-free lane membership
    and local-order stability; workflow contract tests prove every lane command
    and aggregate dependency; each new `cargo xtask ci-validate <lane>` command
    reaches only its declared surface.

- [x] Task 2: Measure and retain the validation fan-out.
  - Contract: collect at least three successful cache-state-matched control and
    treatment runs at immutable heads, separating setup, command, Nix
    realization/substitution, source-probe, aggregate runner-time, and workflow
    critical-path durations.
  - Contract: record raw run links and normalized results in durable research
    documentation; retain the split only if it preserves every verdict and
    clears issue #1574's latency threshold, with any runner-time increase called
    out explicitly.
  - Verification: the retained workflow's `Validate (no e2e)` succeeds only
    after all four required jobs succeed; aggregate injection covers each job,
    and workflow tests require the source probe to run under `always()` in the
    combined test-check/probe job.

- [x] Task 3: Introduce a first-class e2e lane identity and ownership contract.
  - Contract: one shared catalog defines each gate lane by backend, browser,
    partition, and optional shard index/count. Nix derivation names, trace
    identities, Playwright report/manifest names, capture paths, xtask
    diagnostics directories, uploaded artifact names, sidecars, and phase
    records all include that identity.
  - Contract: Firefox partitions are `ordinary 1/2`, `ordinary 2/2`, and
    `serial-special`; visual tests have one declared owner. Each lane has a
    fresh VM, database, storage root, collector, and browser lifecycle.
  - Contract: lane-specific Playwright selection preserves project ordering
    inside a lane but does not pull another lane's dependency projects. The
    ordinary shards own only ordinary tests; the serial-special lane orders the
    global-configuration project before invite.
  - Contract: an expected-test census derived before execution reconciles
    against the union of lane reports: every expected
    `{backend,browser,project,test-id}` appears exactly once, with no unexpected
    identity. Missing, malformed, duplicate, or mismatched lane evidence fails
    closed.
  - Verification: focused config/catalog tests cover reciprocal ownership,
    dependency isolation, visual single ownership, lane naming, and census
    failure cases before any CI matrix expansion.

- [x] Task 4: Reconcile distributed e2e verdicts and diagnostics.
  - Contract: extend existing flaky, duration-pressure, zero-panic, and
    boot-decomposition consumers to accept lane-qualified inputs and produce one
    backend/browser aggregate without discarding per-lane attempts or traces.
  - Contract: a failed lane still lifts every available lane-qualified
    diagnostic; retries remain visible in `.xtask/last-result.json` and
    `$GITHUB_STEP_SUMMARY`; `fail-fast: false` remains in CI.
  - Contract: local full `cargo xtask validate` builds the same retained lane
    derivations that CI distributes, while an unsplit Firefox control remains
    available only for the measurement comparison.
  - Verification: fixture reports/manifests/traces prove complete aggregation
    and explicit rejection of collisions, omissions, duplicate attempts, wrong
    lane identity, partial trace coverage, and a panic in any lane.

- [ ] Task 5: Run the Firefox experiment and choose the retained topology.
  - Contract: compare the existing workers=2 unsplit control, two ordinary
    shards plus one serial-special lane, and a workers=4/larger-VM control for
    both SQLite and PostgreSQL Firefox. Use at least three successful
    cache-state-matched repetitions per arm and keep Chromium unchanged unless
    it becomes the measured critical path.
  - Contract: retain Firefox fan-out only when its exact census and evidence
    reconciliation pass, no new resource/state race appears, p95 Firefox
    wall-clock improves by at least 20%, and runner time/artifact volume stay
    within issue #1574's limit. Otherwise remove the experimental gate topology
    while retaining reusable measurement evidence.
  - Verification: durable results include job/phase timings, Nix realization
    classification, test and retry counts, peak resource/failure evidence,
    runner-time proxy, artifact volume, and resulting workflow critical path.

- [ ] Task 6: Record the accepted CI architecture.
  - Contract: add a numberless ADR draft that supersedes only the affected
    clauses of ADR-0034, ADR-0039, and ADR-0192; preserve ADR-0178's
    documentation/code static boundary and all unaffected local-gate decisions.
  - Contract: project the accepted validation and e2e lane topology into
    `docs/ARCHITECTURE.md`, and update `CONTRIBUTING.md` only where
    contributor-visible gate commands or authority changed.
  - Verification: ADR bundle, architecture projection parity, documentation
    links, and flow-document parity pass through the normal commit gate.

## Ordering and execution boundaries

1. Tasks 1–2 form the validation treatment and measurement decision.
2. Tasks 3–4 establish the e2e interfaces before the workflow gains more lanes;
   they may be delegated separately only after the lane identity and census
   schemas are fixed.
3. Task 5 owns the empirical retain/revert decision; no speculative e2e topology
   becomes authoritative before its measurements reconcile.
4. Task 6 records only the retained result, but its ADR draft and architecture
   projection ship in the same feature branch.
5. Each task keeps focused proof and then commits through `jaunder-commit`; no
   commit receives a `Co-Authored-By` trailer.

## Risk checks

- The distributed non-e2e union equals the local serial catalog exactly and
  retains clean-tree authority.
- Doctest and Elisp producer/consumer edges never cross jobs without a validated
  artifact contract.
- Final Rust coverage and e2e verdicts remain per-ref and are never accepted
  from Cachix.
- E2E lane filters cannot silently omit or duplicate visual, ordinary,
  global-configuration, invite, retry, or browser/backend coverage.
- Every e2e lane has isolated mutable state; per-test identities are not treated
  as isolation for site-wide configuration.
- Trace IDs, output paths, uploads, and diagnostics are collision-free across
  backend, browser, partition, and shard.
- Duration-pressure and boot-decomposition gates inspect every reported
  attempt/lane rather than a merged happy-path subset.
- GitHub matrix expansion preserves stable required contexts and merge-queue
  behavior.
- Added parallelism is judged on workflow wall-clock and aggregate runner cost
  under matched cache conditions, not one warm rerun.
- Any retained decision contradicting existing ADR clauses is explicit in the
  new ADR draft and architecture projection.
