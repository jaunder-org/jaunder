# #1742 revision: Collector-owned retention outline

> Execute with **jaunder-iterate**; use **jaunder-dispatch** only for authorized
> helper work. Trigger: SDK sampling/subscriber architecture and the changed
> #1743 sink contract. The
> [approved spec](../specs/2026-10-12-issue-1742-collector-owned-retention.md)
> governs; this outline awaits approval before behavior changes.

## Scope

In: remove unlanded OTLP filters/admission bookkeeping, always record Jaunder
spans, simplify HTTP adoption, preserve diagnostic context, update shared/local
interfaces and current guidance. Out: native logger implementation, Collector
configuration/deployment, reliable delivery, #1755 receiver repair,
storage/schema or UI changes.

## Task outline

- [x] **0. Establish revised regressions and settle the local-only interface.**
  - Prerequisite: current production composition/exporter fails the desired
    DEBUG/TRACE span/event export under local WARN/off and valid
    unsampled-parent recording/correlation; retain those reds and prior controls
    before changes.
  - Contract: `TelemetryConfig::local_log_filter(&self) -> EnvFilter` replaces
    the unlanded active-sink accessor. #1743's native OTLP logger is unfiltered
    and must not use it. Obtain owner acknowledgment and preserve landing/rebase
    order before changing the coordinated interface.
- [x] **1. Remove filtering and install AlwaysOn recording.**
  - Depends on 0. Remove raw trace input, resolver/fallback/accessor, per-span
    marker/query and obsolete policy fixtures/tests; update both roots and typed
    constructors. Local formatter alone consumes the local filter. The SDK trace
    layer has no severity/target/context filter; production provider uses
    `Sampler::AlwaysOn`. Add no substitute filter or metadata machinery.
  - Verification: revised real SDK/composition matrix covers local WARN/off,
    verbosity and target/context directives, DEBUG/TRACE spans and span events,
    endpoint absence and sampled/unsampled parents. Prove the actual production
    provider policy, not a test-only independently configured sampler.
- [x] **2. Simplify parent adoption and preserve diagnostics.**
  - Depends on 1. Use the installed tracer/subscriber capability already owned
    by `TelemetryGuard` to enable HTTP extraction/adoption, not per-span
    inference or SDK-context lookup. Retain IDs, ordering, safe fields and
    bounded genuine failure diagnostics. Keep distinct ErrorLayer initial/late
    cache, local pretty/JSON/absent-output behavior, slow spans, metrics and
    panic capture.
  - Verification: actual HTTP/router correlation for sampled/unsampled and
    missing/invalid headers, no-tracer silence and real lifecycle failure;
    existing field/formatter/diagnostic/panic/metric controls remain green.
- [ ] **3. Re-prove delivery boundaries and publish the handoff.**
  - Depends on 1–2. First prove the approved bounded patch-free SDK isolation
    candidate: fixed dynamic OTLP-layer SDK-context guard, public
    suppression-preserving activation, and exporter-only runtime with ordinary
    trace/metric builders. Use the accepted fixed five-second runtime shutdown
    budget after both providers; characterize possible residual blocking work
    with an owned-release proof. Document the repeated same-span out-of-order
    external-context/suppression limitation without adding API restrictions;
    prove distinguishable-context LIFO restoration and compare the pinned
    bridge's ordinary latest-ID behavior. Focused tests do not certify transport
    delivery or Collector feedback; keep this task incomplete until its full
    proof succeeds. Update operator guidance and project the approved numberless
    decision from proposal to implemented reality. Confirm #1743's unfiltered
    native-log/local-only contract; its
    all-severity/no-mirroring/logger-lifetime proofs remain with its owner.
  - Verification: isolated inherited-input roots, owned healthy-Collector
    production CLI/test-support flush and server metrics/shutdown; original
    quiet request loop and
    `devtool run -- env JAUNDER_LOG_FILTER=warn cargo xtask e2e-local auth.spec.ts`
    with no trace filter. Check correlation and existing capture/boot consumers;
    disclose #1755 rather than certify absent local seed traces.
  - Final boundary: cumulative independent Standards and full revised
    acceptance/outline conformance; authoritative full coverage and required CI
    on the corrected tree, then retire active planning. PR stays draft until
    complete and merge still requires approval.

## Risk checks

- AlwaysOn overrides only the caller's recording hint, not its trace/parent IDs;
  no promise of recovering upstream data or guaranteed delivery.
- Newly visible DEBUG/TRACE spans/events must obey privacy/field admission;
  audit and test synthetic-secret exclusion, not hide it with a severity filter.
- No shared global filter may clamp OTLP, fields or independent diagnostics. Do
  not remove the separate field-retention fix while deleting admission
  machinery.
- Preserve endpoint precedence, bounded local SDK/fallback isolation and
  provider lifetime. Native OTLP logs remain absent until #1743; do not certify
  them here.
- No deep ambient reads, in-process environment mutation, hand-built DB pools or
  weakened gates. Any storage-backed assertion uses the shared dual-backend
  harness. Keep evidence ignored and ADR numbering/promotion CI-owned.
