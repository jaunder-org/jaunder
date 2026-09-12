# Integrated Performance Harness Implementation Outline

> Execute with `jaunder-iterate`, delegating bounded work through
> `jaunder-dispatch`. This outline exists because the approved spec introduces a
> shared artifact/baseline contract across storage, browser instrumentation, Nix
> producers, host orchestration, and CI.

## Scope

In:

- Deterministic canonical performance datasets and test-only population tooling.
- Direct storage and Chromium browser workload producers for the approved read
  paths.
- Versioned artifacts, statistics, compatibility checks, advisory comparison,
  and trusted baseline import.
- `cargo xtask perf` orchestration and the manual/weekly/labeled-PR workflow.

Out:

- Query or UI optimization, write/concurrency benchmarks, external result
  storage, hard timing gates, and routine Firefox baselines.
- Production seed surfaces or backend-specific fixture SQL.

The storage and browser pieces remain one issue because they consume the same
dataset identity and result contract; splitting them would create the
incompatible formats the integrated design is intended to prevent.

## Key contracts

- Add the pure `performance` tooling library at `tools/performance`. It owns
  `dataset-manifest-v1.json`, the versioned fragment/run envelopes,
  workload/result identities, raw sample sets, exact statistics, compatibility
  keys, and baseline comparison.
- Producers write immutable fragments under `$out/performance/fragments/`:
  `storage-<backend>-v1.json` or `browser-<backend>-<browser>-v1.json`. Each
  envelope has a `storage | browser` producer discriminator and workloads keyed
  by `(producer, workload, backend, browser?, frame)`. The selected run
  configuration determines the exact required key set, so host aggregation
  rejects missing and duplicate workloads.
- `test-support` alone writes `$out/performance/dataset-manifest-v1.json`, the
  authority for profile counts, deterministic distribution buckets, selected
  workload subjects, and precomputed 80-percent cursors. Storage and browser
  producers consume it; neither reconstructs fixture identities.
- Host `xtask` validates and combines fragments into
  `.xtask/performance/<run-id>/performance-result-v1.json`; the committed
  comparison point is `tools/performance/baseline-v1.json`. Producers never read
  or mutate the baseline.
- Browser fragments keep Node action-to-ready samples, document-frame
  diagnostics, and correlated trace locations distinct. The combined run
  envelope records either explicit local provenance or GitHub repository,
  workflow, job, ref/SHA, run ID, and attempt supplied by the workflow.
- Canonical baseline import is a host-only GitHub operation: given a run ID,
  `xtask` fetches and validates the approved successful `main` artifact before
  changing the committed baseline.

## Task outline

- [x] Task 1: Establish the shared performance data and result contract
  - Contract: introduce `tools/performance`; model the three profiles, fixed
    seed and generator version, exact distributions and rounding, manifest and
    fragment envelopes, aggregation keys, sample/statistic calculations,
    compatibility equality, comparison output, provenance, and serde JSON
    format. Register the library in the relevant workspaces and architecture
    view.
  - Verification: deterministic tests prove exact counts for all profiles,
    stable manifests/cursors, envelope round trips, required-key rejection,
    midpoint median and nearest-rank p95, incompatible-key rejection, and the
    advisory 20-percent boundary.

- [ ] Task 2: Produce deterministic fixtures and the authoritative manifest
  - Contract: extend `test-support` through real storage APIs with an internal
    performance seed command that consumes the shared contract, provisions every
    approved record distribution, and emits `dataset-manifest-v1.json`. It owns
    fixture population only; setup duration remains separate from measurements.
  - Verification: focused dual-backend checks use `#[apply(backends)]` where
    behavior is generic, prove exact distributions and cursor subjects from
    stored observations, and run the small fixture smoke against SQLite and
    PostgreSQL.

- [ ] Task 3: Produce direct storage measurements in `devtool`
  - Contract: add the sandbox-side `devtool` storage producer, consuming the
    Task 2 manifest and real storage APIs to emit the storage fragment. It
    measures initial-page, 80-percent-cursor, and revision-detail workloads with
    one cold and 30 warm samples per backend.
  - Verification: focused producer tests prove workload/sample identities, rows
    returned, cold/warm separation, point-versus-pagination semantics, failure
    propagation, and schema-valid small fragments against SQLite and PostgreSQL.

- [ ] Task 4: Produce semantic browser measurements
  - Contract: add a focused performance Playwright surface that consumes the
    Task 2 manifest, uses existing authentication/seeding and `createPerfProbe`
    conventions, and emits 20 fresh-context Node action-to-ready samples for
    `/`, `/app`, global history, per-Post history, revision detail, and
    pagination. A `devtool` producer validates and collects its browser
    fragment; existing document-frame/trace diagnostics remain separately
    attributed.
  - Verification: a small-profile Chromium smoke against each backend proves
    every semantic-ready boundary, exact workload/sample identities, trace
    correlation, absence of pre-warming, and PII-safe emitted attributes.

- [ ] Task 5: Orchestrate fresh runs, comparisons, and trusted baseline import
  - Contract: add the approved `cargo xtask perf <profile>` surface with
    backend/browser and storage-only/browser-only selectors. It supplies a
    per-invocation freshness nonce included in the Nix derivation identity,
    invokes the selected pinned producers, rejects
    missing/duplicate/incompatible fragments, writes the standard sidecar and
    combined run artifact with explicit local-or-CI provenance, renders
    absolute/advisory comparisons, and imports baselines by verified GitHub
    Actions run ID.
  - Verification: focused command tests cover selector exclusion, non-canonical
    override marking, producer failure propagation, artifact assembly, exact
    compatibility, provenance population, pull-request/local/failed-run
    rejection, deterministic valid import, and timing-regression report-only
    behavior. Two identical small invocations prove distinct producer derivation
    identities and fresh measurement timestamps; each selector mode runs as an
    actual smoke path.

- [ ] Task 6: Integrate the pinned performance workflow and operator
      documentation
  - Contract: add a performance workflow for manual dispatch, weekly schedule,
    and explicitly labeled pull requests; serialize benchmark jobs, pass GitHub
    repository/workflow/job/ref/SHA/run/attempt provenance into the combined
    artifact, run the canonical medium release-mode matrix, upload complete
    artifacts/diagnostics, and never rewrite the baseline. Document invocation,
    result interpretation, baseline import, freshness, and the non-gating policy
    in existing performance/architecture guides.
  - Verification: workflow/static checks prove trigger and label behavior,
    canonical matrix identities, job serialization, provenance-bearing
    successful-artifact upload, and no timing-delta gate. Before PR, run
    `devtool run -- cargo xtask validate --no-e2e` and the feasible small
    integrated performance smoke; PR CI must pass Validate (no e2e), all four
    existing `{sqlite,postgres} × {chromium,firefox}` e2e lanes, and the
    explicitly labeled canonical performance workflow.

## Ordering and parallelism

1. Task 1 fixes the contract every other task consumes.
2. Task 2 supplies the real fixture producer and manifest.
3. Tasks 3 and 4 may proceed in parallel after Task 2; their file ownership is
   storage/devtool versus Playwright/browser instrumentation.
4. Task 5 consumes both producer fragments and therefore follows Tasks 3 and 4.
5. Task 6 follows the stable host command, artifact paths, and
   freshness/provenance inputs from Task 5.

## Risk checks

- Preserve SQLite/PostgreSQL parity and real storage semantics; no raw SQL
  fixture fork.
- Keep `test-support` structurally absent from production artifacts and preserve
  ADR-0028's sandbox-producer/host-analyzer boundary.
- Never mix Node and document timing frames or silently pre-warm browser
  contexts.
- Make profile, cursor, sample, toolchain, runner, database, and browser
  identities comparison-critical rather than descriptive metadata.
- Treat malformed/incomplete workloads and producer failures as failures while
  keeping timing deltas advisory.
- Validate trusted baseline provenance through GitHub, not self-declared JSON
  fields.
- Keep benchmark execution isolated and release-mode; cached Nix output must not
  masquerade as a fresh measurement.
- Preserve telemetry cardinality and PII restrictions.
- Update every workspace manifest, Nix source/input boundary, generated
  architecture view, workflow documentation, and result sidecar consumer
  affected by the new tooling library and command.
