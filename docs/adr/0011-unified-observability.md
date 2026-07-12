# ADR-0011: Unified Observability Strategy

- Status: accepted
- Deciders: mdorman, Gemini CLI
- Date: 2026-05-15

## Context and Problem Statement

Jaunder is a full-stack application with complex interactions between the
backend (SSR, server functions) and the frontend (hydration, client-side
routing). Traditional logging is insufficient for diagnosing performance
bottlenecks or understanding the full lifecycle of a user request across these
boundaries, especially during end-to-end testing.

## Decision Drivers

- Visibility: End-to-end tracing from the test runner through the browser to the
  backend.
- Performance: Ability to identify hydration hotspots and slow database queries.
- Consistency: Using industry-standard protocols (OpenTelemetry).

## Decision Outcome

Chosen option: "Unified Observability with OpenTelemetry", because it provides a
standard way to correlate spans across different environments and languages.

### Implementation Details

- **Backend**: Uses the `tracing` crate with `tracing-opentelemetry` in the
  `server` crate.
- **E2E Test Runner**: Playwright fixtures in `end2end/tests/fixtures.ts`
  generate spans and inject trace context.
- **Correlation**: The `JAUNDER_E2E_TRACEPARENT` environment variable is used to
  propagate trace context from the test runner to the backend.
- **Layered Tracing**:
  - `e2e.test`: Automatic, captures one span per test with resource and
    navigation summaries.
  - `e2e.flow`: Manual, captures domain-specific semantic phases (e.g., "login
    flow").
- **Artifacts**: Traces are exported as JSONL files (`otel-traces.jsonl`) during
  CI and VM test runs for offline analysis.
- **PII discipline**: Span fields and the structured error boundary
  (`error.source`, `error.context` in `web/src/error.rs`) are operator-only but
  are still exported to trace backends, so they MUST NOT carry user PII or
  secrets — email addresses, session/verification tokens, passwords, or post
  bodies. Record stable, non-sensitive identifiers instead (`user_id`,
  `db.system`, `error.kind`/`error.class`); usernames are public identifiers and
  acceptable. The preserved error source chain is built from typed errors
  (`sqlx::Error`, `io::Error`, parse errors), which carry structural/diagnostic
  text — not bound parameter values — so the chain is PII-free as long as
  constructors keep raw user input out of error messages.

## Consequences

- Good: Deep visibility into hydration timing and backend performance during
  tests.
- Good: Correlated traces make it easy to see exactly what happened in the
  backend during a specific E2E test step.
- Bad: Adds some complexity to the test runner and backend initialization.
- Bad: Generates large trace files that require specialized analysis scripts.

## Addendum (2026-06-18): Event metrics pipeline (jaunder-kq8w.21, pre-GitHub bead tracker)

Traces answer "what happened in this one request"; they do not answer "how
often, and is the rate abnormal". This addendum adds an OpenTelemetry
**metrics** pipeline alongside the existing tracer for operational signals —
auth abuse, silent email/WebSub failures, backup health, upload pressure, and an
overall error rate. The full instrument catalog lives in the design spec
(`docs/archive/2026-06-18-otel-metrics-pipeline-design.md`); this records the
conventions and architecture.

### Pipeline

- The OTLP `MeterProvider` is installed in `server::observability` next to the
  tracer (`build_otel_meter`), behind the **same** OTLP-endpoint gate
  (`JAUNDER_OTEL_EXPORTER_OTLP_ENDPOINT` / `OTEL_EXPORTER_OTLP_ENDPOINT`). One
  endpoint feeds both traces and metrics; setup failure is non-fatal.
- When no endpoint is set (or in any non-server process — wasm, the CLI), no
  provider is installed and every instrument is a no-op, exactly like traces.

### Emit facade: `host::metrics` (originally `common::metrics`)

> **Superseded (2026-07-09, #345):** the facade now lives in
> **`host::metrics`**, declared unconditionally. `host` is native-only
> (ADR-0058), so `opentelemetry` is kept out of the wasm bundle by crate
> structure rather than the feature gate described below. The text that follows
> is the original, correct-for-its-time rationale; see the 2026-07-09 addendum
> for the move.

- A single facade in `common`, behind an optional **`metrics` feature** (enabled
  by `server` and `web/ssr`, never by the wasm/hydrate build). This is a feature
  gate, not a `target_arch` gate, so `common` stays free of `target_arch` cfgs
  (jaunder-kq8w.10) and the wasm build never pulls `opentelemetry`. `common` is
  the only crate reachable by every emitter (`server → web → common`), including
  the CLI.
- **Cardinality is enforced by the type system**: every helper takes bounded
  Rust enums (e.g. `metrics::login(LoginOutcome::InvalidCredentials)`), each
  mapping to a `&'static str` attribute. A call site cannot pass an unbounded
  label (username, id, URL). Instruments are built once from
  `global::meter("jaunder")` via a `LazyLock`.

### PII discipline

The PII rules above apply unchanged: metric attributes are bounded enums or
fixed operation names, never user input, so metrics carry no PII or secrets.

### Refinements over the spec catalog

- `jaunder.auth.registrations` gained a `closed` policy value (the site can have
  a `Closed` registration policy) and a `cli_bypass` policy for CLI-created
  users.
- `jaunder.media.uploads` gained an `error` outcome for unexpected (non-
  `MediaError`) upload failures.
- `jaunder.posts{event}` counts **web-UI** post lifecycle actions; posts created
  or mutated over `AtomPub` are reflected in `jaunder.atompub.requests` instead,
  so the two never double-count.
- `jaunder.atompub.requests{op,result}` is emitted from a single router-level
  middleware: `op` comes from the matched route template + method (a bounded
  set), `result` from the response status class.
- Email metrics are emitted at the `web` send call sites (where the message
  `kind` is known), not inside the generic mailer.

### Testing

Facade and mapping helpers are unit-tested against an in-memory metric reader
(`opentelemetry_sdk` `testing` feature): install a `MeterProvider` backed by an
`InMemoryMetricExporter`, call a helper, `force_flush`, and assert the named
instrument was exported. Branch-mapping helpers (error kind/class, session
outcome, upload/serve outcome, AtomPub op/result) are additionally covered by
exhaustive table tests so every attribute mapping is exercised regardless of
which request paths a given integration test happens to hit.

### Deferred

- Saturation **gauges** via async observable callbacks (queue depth, pool
  utilization, storage bytes, time-since-last-backup) — jaunder-kq8w.24.
- Making one-shot **CLI** emits actually export (provider init + `force_flush`)
  — jaunder-kq8w.25. CLI emit sites exist but are no-ops without a provider.

## Addendum (2026-06-27): Flushing telemetry from one-shot processes (issue #12)

The metrics addendum above installs an OTLP `MeterProvider` (and the tracer
installs a `TracerProvider`) gated on the OTLP endpoint. Diagnosis of the CLI
case (the kq8w.25 item deferred above) found the providers _were_ already
installed for one-shot commands — `run()` calls `init_tracing` for every
non-`serve` command — but both exporters are deferred: the metric reader is
periodic and the span processor batches, so they only export on an interval the
long-running server easily reaches and a one-shot CLI command exits long before.
The CLI's metric and span emits were therefore installed correctly yet silently
dropped. The fix is to flush before exit, not to install a provider.

Convention: `init_tracing` returns a `#[must_use]` `TelemetryGuard` owning the
installed providers. A process holds the guard for its working scope; the
guard's `Drop` calls `shutdown()` (force-flush + shutdown) on each provider,
exporting buffered telemetry on every exit path — success, `?` error-return, and
panic unwind. A single binding at the `run()` dispatch boundary owns telemetry
for _every_ command — `serve` included — so command bodies (including
`cmd_serve`) carry no telemetry-lifecycle code; for `serve` the guard is simply
held for the process lifetime and flushes at shutdown. Export failures (e.g. an
unreachable collector) are logged, never propagated — a telemetry failure must
not change a command's exit status. This closes the "CLI export" item the
metrics addendum deferred (metrics **and** traces, since both shared the
drop-on-exit defect).

## Addendum (2026-07-09): Metrics facade relocated to `host::metrics` (issue #345)

The 2026-06-18 addendum placed the emit facade in `common` behind an optional
`metrics` feature because, at the time, `common` was the lowest crate reachable
by every emitter (`server → web → common`) and a feature gate kept
`opentelemetry` out of the dual-target crate's wasm build.

ADR-0058 has since introduced `host` — the native-only sibling of `common` — and
the emitter set has grown to include `host` and `storage`. Every crate that
emits metrics now depends on `host` (`storage → host`, `web → host` under its
`server` feature, `server → host`, and `host` itself), and `host` is never in
the wasm dependency closure. The facade therefore moves to **`host::metrics`**,
declared **unconditionally**: `opentelemetry` is excluded from wasm by crate
structure, so the `metrics` feature on `common` — and the `common/metrics` /
`features = ["metrics"]` opt-ins in `host`, `server`, and `web` — are deleted.
This also removes `storage`'s prior reliance on Cargo feature unification to see
the metrics `SessionOutcome` enum (it now references `host::metrics` on a direct
dependency).

No behavior change: the instrument catalog, bounded-enum cardinality discipline,
PII rules, and no-op-without-a-provider semantics are unchanged; only the
facade's crate home moves. Exporter setup remains in `server::observability`.
This is an application of ADR-0058's charter ("any strictly-host-focused shared
code... including production machinery pushed down out of `web`"), not a new
observability decision — hence an amendment here rather than a new ADR.
