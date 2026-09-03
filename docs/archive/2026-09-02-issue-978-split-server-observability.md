# Issue #978 — split server observability by concern

## Outcome

`server::observability` becomes a directory whose leaves separately own scoped
server diagnostics, server tracing initialization, and HTTP observability.
Existing public paths, telemetry output, diagnostics behavior, and request
middleware behavior remain unchanged.

## Load-bearing decisions

- `server/src/observability/mod.rs` is assembly only: it declares the three
  leaves and explicitly re-exports the existing public functions.
- `diagnostics.rs` owns the WARN+ JSONL layer, fixed diagnostic fallbacks, panic
  record schema, and the independent append-only panic hook. The per-layer WARN
  filter, one-record-per-line shape, previous-hook chaining, and bypass of
  `tracing` remain intact under ADR-0049.
- `initialization.rs` owns `init_server_tracing` and composes the host telemetry
  lifecycle with the optional server diagnostic layer and panic hook.
- `http.rs` owns W3C trace-context extraction, request-ID set/propagation,
  request-span construction, and `with_http_observability`.
- `host::telemetry` remains unchanged. It continues to own unified trace/meter
  lifecycle and slow-span detection under ADR-0011.
- `jaunder::observability::{init_server_tracing, with_http_observability}`
  retain their signatures and paths through explicit re-exports; the router's
  internal `crate::observability::with_http_observability` call remains
  unchanged.
- Unit tests move beside the implementation or contract they prove. Tests that
  mutate process-global panic-hook, subscriber, or meter state retain one shared
  synchronization lock across leaves. Test-only subprocess selectors may follow
  their moved test; no production or supported test API changes.
- The architecture view is corrected where it names obsolete observability
  ownership or source paths; this refactor introduces no new architectural
  decision.

## Acceptance

- Every observability leaf has one named responsibility, and `mod.rs` satisfies
  ADR-0128 without implementation or inline tests.
- Existing diagnostics tests preserve fallback text, WARN filtering, JSONL panic
  records, append and hook-chaining behavior, and failure handling.
- Existing HTTP tests preserve traceparent adoption and request-span behavior;
  router construction retains request-ID and response tracing layers.
- The architecture view points HTTP middleware to `observability/http.rs` and
  assigns tracer/meter setup and slow-span detection to `host::telemetry`.
- Moved process-global tests remain serialized by one shared lock.
- All existing production callers compile unchanged against the two re-exported
  functions, and the repository gate passes.

## Boundaries

- No change to emitted span names, fields, levels, parent relationships,
  diagnostic schema, fallback messages, capture enablement, or panic behavior.
- No decomposition or behavior change in `host::telemetry`.
- No new observability feature, dependency, public API, ADR, or gate.
