# Issue #839 implementation outline

> Approved specification:
> `docs/archive/2026-09-01-issue-839-browser-console-capture-spec.md`

## Risk trigger

An outline is required because this change adds an OTLP attribute schema,
changes the accepted telemetry privacy boundary through a proposed ADR, and
coordinates default-page and secondary-context capture contracts.

## Tasks

- [ ] Task 1: Deepen the phase-aware browser diagnostic sink
  - Replace the warning-only record/list with one discriminated
    console/pageerror record union.
  - Attach `console` and `pageerror` listeners only through
    `attachTraceCapture`; normalize console location and page-error
    name/message/optional stack synchronously.
  - Retain only console `warning` and `error` levels while leaving the full
    ordered sink uncapped in memory.
  - Migrate the client-telemetry warning-order assertion to the new sink.
  - Add a focused browser test proving warning, error, pageerror, excluded log
    level, record fields, ordering, and test-phase attribution.

- [ ] Task 2: Export bounded diagnostics on their owning browser spans
  - Add a pure telemetry projection that keeps the first 20 records, preserves
    sequence order, serializes them for `e2e.console_json`, and computes
    `e2e.console_dropped` exactly.
  - Add both attributes to the default `e2e.test` span and each
    secondary-context `e2e.page` span without changing span identity, timing,
    parentage, or existing payloads.
  - Add deterministic transform/schema tests for empty, boundary, and over-cap
    inputs and for both span surfaces.
  - Leave pretest records in the pretest sink and exclude them from test/page
    attributes; switch captures to a sinkless teardown phase before settlement.

- [ ] Task 3: Record the test-only telemetry boundary
  - Add a numberless proposed ADR draft amending ADR-0011: production telemetry
    remains PII/secret-free; isolated Playwright E2E telemetry may preserve
    synthetic application values required for diagnosis; real-user deployments
    and infrastructure credentials remain excluded.
  - Project the amendment and browser-diagnostic schema into
    `docs/ARCHITECTURE.md`.
  - Update `docs/observability.md` with the record schema, default/secondary
    span ownership, warning/error filter, first-20 cap, dropped-count attribute,
    no-fail policy, and synthetic-data boundary.
  - Do not edit accepted ADR-0011 or ADR-0096.

- [ ] Task 4: Verify the delivered contract
  - Run the focused Playwright browser-diagnostic test with
    `cargo xtask e2e-local <file-or-line>`.
  - Run focused TypeScript/trace schema tests through the repository xtask lane.
  - Run the applicable static check once, then stage the intended tree and
    commit through the precommit gate.
  - Review the fixed-point diff on Standards and Spec axes before shipping.

## Invariants

- Production browser code gains no listener or diagnostic exporter.
- Every listener is installed once per context through `attachTraceCapture` and
  covers seeded and future pages.
- Phase is selected at event delivery; pretest noise never enters test
  attributes, and diagnostics delivered after the test body are sinkless.
- Raw diagnostic records remain exact; only list length is truncated at OTLP
  serialization.
- Every truncated list reports its exact dropped count.
- Browser diagnostics remain observational and never change Playwright test
  status.
- Existing trace analyzers continue to ignore the new attributes safely.
