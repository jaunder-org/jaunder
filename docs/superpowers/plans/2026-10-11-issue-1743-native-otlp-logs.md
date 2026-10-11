# Issue #1743 — native OTLP logging implementation outline

> Execute with **jaunder-iterate**; use **jaunder-dispatch** only for bounded
> tasks. Trigger: shared subscriber architecture, privacy admission, process
> lifecycle and a cross-agent prerequisite. The approved spec remains
> authoritative.

## Scope

In: native process logs through the configured Collector, stdout without OTLP,
logger lifetime/failure handling, real-stack message proof and operator
guidance. Out: #1742's filter implementation, historical repair, deployment,
credentials, retention, browser telemetry, unrelated errors, new log-filter
controls.

Order: #1742/rebase + baseline → record projection → sink/lifecycle → real-stack
routing → combined proof/docs. Key risks: privacy admission, explicit-root
context, SDK recursion and bounded shutdown. No independent filter
implementation.

## Task outline

- [ ] 1. Consume the delivered #1742 foundation and capture the before state.
  - Wait for #1742 to land; rebase this checkout, reconciling docs by intent.
    Verify its independent admission, initial/late field retention, ErrorLayer,
    trace-parent and capture contracts before altering subscriber composition.
  - Capture deterministic VictoriaLogs default message presentation while the
    stack still uses journal ingestion; retain query/state/viewport for task 5.
  - Verification: peer merge/contract evidence and source review; baseline
    visual proof. Stop/coordinate if delivered behavior differs from the agreed
    seam.
- [ ] 2. Introduce a reviewed native log-record projection.
  - Contract: explicit message → string body; tracing level → SDK severity;
    admitted fields → typed attributes; target/path use existing stack query
    names. No arbitrary field/Debug or inherited-span-map serialization.
  - Resolve the event's explicit/contextual/root parent from actual admitted
    tracing context. Prevent SDK ambient-context fallback for roots or missing
    parent context; never enter/create a synthetic span to obtain IDs.
  - Verification: in-memory SDK records for exact body/types, contextual and
    differing explicit parents, root/filtered parents, safe field retention and
    exclusion of credentials, request aggregates and `error.span_trace`.
- [ ] 3. Integrate the active log sink, logger ownership and failure isolation.
  - Contract: consume #1742's ordinary-log policy; select stdout only without an
    endpoint, native records only with one, independent of trace admission.
    Logger setup failure must not restore ordinary stdout mirroring.
  - Use the shared endpoint/resource and pinned SDK gRPC capability, creating
    the provider inside the existing Tokio process lifecycle. The process guard
    owns logger shutdown alongside tracer/meter; shutdown drains buffered logs.
  - Verification: real subscriber/exporter admission matrix and target filters;
    log/trace `off` independence, unchanged CLI result output, one-shot flush,
    late-field/SpanTrace preservation without stdout, repeated failures and
    nonfatal setup/export/shutdown. No SDK failure re-enters native logs.
- [ ] 4. Switch stack ingestion and support isolated harness log export.
  - Contract: production Collector logs receiver becomes OTLP; remove journal
    receiver/JSON transformation, not other ingress/storage/authentication.
  - Extend the traces-only E2E Collector with a separate test-only log file
    exporter/pipeline. Keep `otel-traces.jsonl`, diag/panic layout and their
    consumers unchanged; optional log evidence must not mix into trace JSONL.
  - Verification: module assertions and both real stack signal-capture VMs
    (`jaunder-stack-sqlite-bcrypt`, `jaunder-stack-postgresql`): exact `_msg`,
    severity, safe fields, linked trace/span IDs, credential exclusion and
    single ingestion. Keep SQLite persistence and existing ingress assertions.
- [ ] 5. Prove combined behavior and project delivered contracts.
  - Run focused host/HTTP proofs, one real browser/server flow preserving
    trace/boot/panic evidence, then applicable stack/CI boundaries. Review
    findings before expensive final proofs; never weaken gates or resources.
  - Capture the comparable after VictoriaLogs UI/query presentation. Update
    observability/deployment procedures and convert proposed architecture to
    built reality only after implementation; retain the numberless ADR.
  - Verification: spec conformance, before/after judgment, both-backend stack
    proof and authoritative CI. Retire completed spec/outline at ship; execution
    reports and temporary probes remain ignored. Merge requires fresh approval.

## Record-admission and failure contracts

- Start with a closed profile: `error.kind`, `error.class`, `error.public`,
  `error.source`, `error.context`, `error.disposition`, `telemetry.origin`,
  `db.system` and audited stable identifier fields. Preserve safe
  `jaunder.target` and `jaunder.request.uri` context. Freeze actual
  producer/field representations through table tests before wiring; any
  additional retained field requires a reviewed existing producer, not wildcard
  admission. Document query-shape changes.
- Preserve scalar types where supported; unsigned values must not wrap into
  OTLP's signed integer range. Debug rendering is allowed only for reviewed
  carrier/ID projections, never as a fallback for an unknown field or aggregate.
  Audit messages from newly exported CLI/seed processes as well as the server.
- Reject SDK/exporter/self-diagnostic targets before native record emission,
  regardless of user filters. Keep them on the agreed local failure path. Redact
  failure payloads before SDK diagnostics can expose them; preserve a failed
  result/classification rather than report successful export.
- Coalesce new logger diagnostics to at most one fixed redacted stderr line per
  failure kind per process; cap each line at 256 bytes. No endpoint, raw
  exporter text, payload or caller value. Prove bounds under repeated failures.
- Rely on pinned SDK bounded batching defaults, not explicit restatement:
  default queue 2048 records/batch 512; logger shutdown defaults to five
  seconds. Lock behavior with regressions, including saturation and an
  unavailable Collector. Resolve any supported inherited SDK policy at the
  process boundary; callbacks must not reread environment. Never add an
  unbounded flush before the bounded shutdown or change existing tracer/meter
  lifetime budgets.

## Risk checks

- Filters affect admission, not availability of local span-field/capture
  context. Correlation cannot accidentally inherit an unrelated ambient SDK
  context.
- Raw span traces can be explicit event fields, not just inherited stacks.
  Native admission and permissive filters must not expose them or self-failures.
- Log-provider creation/export/shutdown must be valid under the pinned runtime
  and must not panic, block indefinitely or alter the application's outcome.
- Test-only log capture is separate from trace capture; existing metrics
  behavior, backend parity, panic rules, retries, timeouts and VM budgets remain
  intact.
- One writer in this checkout; do not edit the #1742 checkout or implement its
  owned filtering foundation. New uncertainty returns to coordination/review.
