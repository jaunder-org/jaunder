# Issue #1434 — Integrated performance harness

## Outcome

Jaunder provides a repeatable performance harness that populates representative
large datasets, measures the storage and browser paths most likely to degrade at
scale, and emits reviewable machine-readable results. Maintainers can compare
absolute timings with a versioned baseline over time without making noisy
shared-runner measurements a merge gate.

## Load-bearing decisions

- The public entry point is one host command: `cargo xtask perf <profile>`.
- The command supports backend and browser selection plus storage-only and
  browser-only execution. `xtask` owns orchestration, analysis, reporting, the
  standard result sidecar, and baseline comparison under ADR-0028.
- Code that must execute inside a Nix derivation produces artifacts through
  `devtool`; out-of-process fixture creation remains in the structurally
  test-only `test-support` binary under ADR-0046. No fixture affordance enters
  the production CLI, server API, or raw backend-specific SQL.
- The harness is integrated: one deterministic dataset supports direct storage
  measurements and realistic browser measurements. Storage and browser results
  retain distinct measurement frames and workload identities.
- Three canonical profiles are defined:
  - `small`: 100 Posts, 10 authors, and 500 revisions;
  - `medium`: 5,000 Posts, 100 authors, and 25,000 revisions;
  - `large`: 50,000 Posts, 1,000 authors, and 250,000 revisions.
- `medium` is the only routine comparison profile. `small` is a smoke profile;
  `large` is an intentional stress profile. Count overrides are allowed for
  exploration but mark the result non-baseline.
- Dataset generation is deterministic from a versioned generator schema and
  fixed seed. A result records both so incomparable datasets cannot be presented
  as a regression comparison.
- Every canonical profile uses the same deterministic distributions. Fractional
  buckets use largest-remainder allocation with the stable Post order as the
  tie-breaker:
  - lifecycle: 60 percent live, 15 percent Draft, 15 percent Scheduled, and 10
    percent deleted; one-third of live Posts are backdated;
  - revisions: 70 percent of Posts have 1 revision, 20 percent have 5, and 10
    percent have 33, yielding exactly five revisions per Post;
  - tags: 25 percent have none, 50 percent have 2, and 25 percent have 8;
  - audiences: 50 percent are public-only, 25 percent have 1 private audience,
    and 25 percent have 5 private audiences;
  - media: 75 percent have none, 20 percent have 1 attachment, and 5 percent
    have 5;
  - the cross-product of Markdown, HTML, and plain-text formats with 256-byte,
    4-KiB, and 64-KiB ASCII body classes is allocated as evenly as integer
    counts permit; and
  - each author follows the next `min(20, authors - 1)` authors in a
    deterministic ring.
- Dataset setup is not a measured workload. Provisioning and seeding durations
  are reported separately from application performance.
- Paginated direct storage workloads cover the public timeline, authenticated
  timeline, owner-wide history, and per-Post history at the initial page and a
  deep cursor precomputed at 80 percent of the matching result set. Revision
  detail is a separate exact-ID point workload. They run against both SQLite and
  PostgreSQL in accordance with ADR-0001 and exercise cursor semantics from
  ADR-0004.
- Browser workloads cover `/`, `/app`, `/history`, per-Post history, revision
  detail, and pagination. Their primary metric is a Node monotonic-clock
  action-to-ready duration: timing starts immediately before the initiating
  navigation or Load-more action and ends only when the workload-specific
  semantic-ready condition is true. Timeline and history lists require the
  expected rows with loading cleared; per-Post history additionally requires the
  correct Post heading; revision detail requires the requested revision identity
  and detail content; pagination requires the expected row-count increase with
  loading cleared. Sleeps and network-idle heuristics are not readiness
  conditions.
- Browser spans correlate with existing server and storage spans under ADR-0011.
  Attributes obey the existing bounded-cardinality and PII rules; benchmark
  fixtures never export bodies, secrets, email addresses, or arbitrary audience
  names.
- Canonical browser comparisons use Chromium against SQLite and PostgreSQL.
  Firefox remains supported on demand but has no routine committed baseline.
- Canonical runs use release-mode application artifacts and execute one workload
  at a time without concurrent benchmark jobs on the runner.
- Storage workloads use one cold sample followed by 30 warm samples. The cold
  sample is the first invocation against a freshly provisioned dataset; warm
  repetitions remain separately identified.
- Canonical browser workloads use 20 cold samples, each in a fresh browser
  context without a pre-warm navigation, under ADR-0099. Any future warm-browser
  experiment is a separately named matched arm, never an implicit suite warmup
  or part of the canonical baseline.
- Paginated workloads use the product-default page size of 50. Workload identity
  records that page size, the 80-percent cursor target, and the resolved cursor
  rank so different query positions cannot compare as one workload.
- Browser action-to-ready metrics stay wholly in the Node frame. Document-frame
  boot decompositions follow ADR-0100 and remain diagnostics; bridge and
  frame-skew values are reported separately and never substituted into the
  primary duration.
- Every workload retains raw samples and reports sample count, minimum, maximum,
  arithmetic mean, midpoint median, and nearest-rank p95. Results also record
  rows returned.
- The result compatibility key is exact equality over result-schema version,
  generator version and seed, profile, workload, backend, browser where
  applicable, build mode, cold/warm frame, sample count, page size, cursor
  target and resolved rank, Nix system and derivation identities, runner image
  and architecture, CPU model, database version, and browser version.
- A committed JSON baseline is the durable comparison point. Git history records
  reviewed baseline changes; CI artifacts retain complete run evidence.
- Baseline import accepts a GitHub Actions run ID and fetches the artifact
  itself. It verifies the Jaunder repository, approved performance workflow and
  job identities, successful conclusions, `main` ref and head SHA, workflow run
  and attempt, artifact identity, and the complete compatibility key before
  producing a reviewable diff. Pull-request or local artifacts may be compared
  but cannot become canonical. CI never edits or commits the baseline.
- Comparison reports always show absolute values and sample spread. They
  highlight a median or p95 regression of at least 20 percent as advisory
  evidence; the harness does not fail because of timing deltas.
- The performance workflow runs manually, on a weekly schedule, and for a pull
  request carrying an explicit performance label. It does not run for every pull
  request.
- Performance workflow failures caused by invalid fixtures, missing samples,
  malformed artifacts, inconsistent identities, or unsuccessful workloads remain
  real failures. Only timing regressions are report-only.

## Acceptance

- `cargo xtask perf small` completes an isolated smoke run and writes the
  standard xtask completion sentinel, sidecar, and a dedicated machine-readable
  performance artifact.
- Backend selectors can run the same direct storage workload set against SQLite
  and PostgreSQL without backend-specific fixture behavior.
- Backend and browser selectors and storage-only and browser-only modes run
  exactly the selected workload set, omit unselected workloads, and record every
  selection in result identity; this includes on-demand Firefox execution.
- Repeated generation with the same profile, generator version, and seed
  produces the same counts, relationships, lifecycle distribution, and workload
  cursors.
- Every canonical dataset demonstrably contains the exact aggregate counts and
  deterministic distribution buckets declared above.
- Storage output contains successful initial-page and 80-percent-cursor
  measurements for every paginated workload, plus a successful point measurement
  for revision detail, with cold and warm samples reported separately.
- Browser output contains 20 successful cold action-to-ready samples for every
  named route and pagination path against both canonical backend configurations,
  each ending at its declared semantic-ready condition.
- Browser artifacts preserve the existing trace correlation and clock-frame
  rules; conformance review can distinguish document-frame, Node-frame, and
  storage measurements.
- The machine-readable result contains raw samples, statistics, rows returned,
  workload and dataset identities, the complete compatibility key, and GitHub
  Actions provenance.
- A non-canonical override run is visibly marked and cannot be imported as the
  canonical baseline.
- Importing a successful compatible artifact by approved GitHub Actions run ID
  updates the committed baseline deterministically; importing a local,
  pull-request, failed, malformed, provenance-invalid, or
  compatibility-mismatched artifact is rejected without changing it.
- Comparing with the baseline reports absolute values and deltas and highlights
  qualifying median or p95 regressions without returning failure solely for
  those regressions.
- Manual, weekly, and explicitly labeled pull-request workflow triggers exercise
  the canonical medium configuration and upload the result and diagnostic
  artifacts.
- Existing static, host-test, hermetic validation, and end-to-end gates continue
  to pass on both storage backends.

## Boundaries

- This issue measures read and rendering behavior; it does not establish
  write-throughput, concurrency, saturation, or destructive-load benchmarks.
- It does not optimize queries or UI paths found to be slow. Findings become
  separately scoped issues with the captured workload as reproduction evidence.
- It does not add a hard timing gate, automatically rewrite baselines, or
  publish results to an external metrics service.
- It does not make Firefox part of the routine baseline matrix.
- It does not model unrelated records such as email delivery, authentication
  tokens, backup state, or federation delivery queues.
- It does not replace existing functional tests, timeout-pressure checks,
  operational telemetry, or WASM size budgets.
