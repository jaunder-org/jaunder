# #1742: Independent log/trace filtering implementation outline

> Execute with **jaunder-iterate**; use **jaunder-dispatch** only for authorized
> helper work. Trigger: subscriber/configuration architecture and the shared
> filtering interface with #1743. The
> [approved spec](../specs/2026-10-11-issue-1742-independent-trace-filtering.md)
> is authoritative. Implementation waits for this outline's approval and the
> #1743 owner's confirmation of interface/order.

## Scope

In: shared filter resolution, sink-independent span context, independent stdout
and trace admission, request-parent handling, regression proof and guidance.
Out: native OTLP log export/routing/logger lifetime or journal ingestion
(#1743), new sampling/retention policy, storage/schema changes or production
deployment.

## Task outline

- [x] **0. Lock down the original working-tree middleware regression.**
  - Prerequisite to every behavior change: exercise the current branch's real
    request middleware and original subscriber composition with OTLP enabled and
    quiet log settings. The regression expects no false assignment warning and
    an exported, correctly parented request span; establish its red verdict on
    the existing warning/missing-span behavior. Retain an INFO-filter positive
    control with correct parent correlation and the separate deployed-binary
    control.
  - Verification: an unattended, repeatable red-capable regression at the real
    composition seam, with its pre-change command/revision/result captured in
    ignored evidence. Do not substitute resolver-only tests or wait until the
    post-change end-to-end task to establish this baseline.
- [ ] **1. Resolve and expose the two immutable filter policies.**
  - Depends on 0. Contract: add raw `trace_filter` input to
    `TelemetryRawConfig`, collected as `JAUNDER_TRACE_FILTER` by both executable
    roots. `TelemetryConfig` owns the resolved log and trace policies; expose
    its parsed ordinary-log policy as `ordinary_log_filter(&self) -> EnvFilter`
    for subscriber-layer reuse by #1743, without duplicated environment
    resolution or sink selection.
  - Verification: resolver/default/precedence/fallback coverage, including
    unreadable input, independent `off` and `--verbose`; update all typed config
    constructors and child-environment isolation fixtures. No runtime ambient
    reads below roots. Application statuses and endpoint policy stay unchanged.
- [ ] **2. Compose independent sinks and preserve diagnostic span context.**
  - Depends on 1. Apply log policy only to ordinary stdout admission and trace
    policy only to the OTLP trace layer. Do not retain a shared global filter
    that intersects diagnostics or ErrorLayer. Keep server WARN+ capture and the
    independent panic hook intact.
  - Contract: host telemetry owns an authoritative per-span trace-admission
    marker, attached by a layer sharing the OTLP layer's exact filter/lifecycle;
    expose `span_is_trace_admitted(&Span) -> bool` for the server. This is
    filter admission, not a promise that sampling exports a span. No
    re-evaluation of a cloned dynamic filter and no `span.is_disabled()`
    inference.
  - Contract: keep ErrorLayer span-field storage independent of ordinary-log
    output, with initial and late-field recording. Ensure field updates occur
    once and remain valid for pretty/JSON stdout or an absent stdout sink;
    ErrorLayer alone supplies initial fields but does not handle late records.
  - Verification: actual subscriber/in-memory-exporter matrix with log WARN and
    trace INFO, reverse verbosity, both `off` directions,
    target/context-sensitive filters, and endpoint absence. Prove SpanTrace
    fields with stdout removed, WARN+ capture with both filters `off`, and
    unchanged panic independence.
- [ ] **3. Adopt request parents only for trace-admitted spans.**
  - Depends on 2. Use the host-owned admission contract at actual request-span
    creation; do not obtain/start the span context merely to decide admission.
    Preserve middleware ordering, request IDs, safe fields and W3C extraction.
  - Contract: expected filtering skips assignment without warning. Admitted-span
    layer/lifecycle failures retain bounded-reason local stderr diagnostics,
    without tracing recursion or raw header/source dumps. #1743 must keep that
    local diagnostic path outside its native-log exporter.
  - Verification: real HTTP/router/subscriber proof of trace ID, parent ID and
    child operation correlation; missing/invalid headers; trace-filter rejection
    while console/ErrorLayer keeps the span active; genuine missing-layer and
    already-started failures. Backend-common HTTP/storage assertions cover both
    backends through the existing harness, not hand-built pools.
- [ ] **4. Prove executable/browser compatibility and publish the handoff.**
  - Depends on 1–3. Prove both executable roots use inherited policy, production
    CLI and seed flush traces, and metrics/provider shutdown/capture scope stay
    intact. Retain the deployed-binary control and rerun the original request
    loop against the current working-tree executable.
  - Verification: focused host/HTTP tests, then a real quiet-log browser flow.
    The usual `RUST_LOG=info` e2e lane alone is insufficient: use the preferred
    log override at the child boundary, e.g. `JAUNDER_LOG_FILTER=warn`, with
    trace filter unset, and inspect correlated exported capture. The focused
    lane is `devtool run -- cargo xtask e2e-local auth.spec.ts`; preserve its
    existing budgets, fixture lifecycle and capture/boot consumers.
  - Update operator guidance and project the ADR from proposed-change prose to
    implemented reality. Record the ordinary-log accessor, trace-admission and
    field-retention contracts for #1743. #1743 rebases after #1742 lands and
    owns combined native message/severity/correlation, no-mirroring, diagnostic
    exclusion and logger-shutdown proof; #1742 does not certify an absent sink.

## Risk checks

- Every spec acceptance item maps to tasks 0–4; evidence remains ignored/session
  storage, while maintained tests, minimal fixtures and guidance are committed.
- Dynamic filter admission must reflect the actual OTLP layer, not overall
  subscriber activity. Adding another ordinary-log sink must not change it.
- Field bookkeeping must not depend on stdout admission or double-append fields
  when a formatter is active. Keep slow-span reporting and sampling unchanged.
- SDK setup/export and request-parent diagnostics stay nonrecursive and
  PII-free; changing trace admission does not create a native log pipeline or
  broaden recordable field types.
- Configuration is an immutable snapshot; no unsafe environment mutation,
  heterogeneous dependency bundle or new `mod.rs` implementation body.
- The #1743 owner confirmed the shared filter contract and #1742-first / peer
  rebase order in issue comment 6110903958. Exact helper signatures remain
  subject to this outline's approval. Native relationship readback also confirms
  #1743 is blocked by #1742; that edge does not authorize implementation or
  merge.
