# CLI Telemetry Export (issue #12) Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Make one-shot CLI commands export their OTel metrics and trace spans before the process exits, instead of silently dropping them.

**Architecture:** `init_tracing` returns a `TelemetryGuard` that owns the OTLP `SdkMeterProvider` and `SdkTracerProvider`. The guard is bound for the working scope of the process; its `Drop` calls `shutdown()` (force-flush + shutdown) on each provider, so buffered telemetry exports on every exit path (success, `?` error, panic). One binding at the `run()` dispatch boundary covers every one-shot command — command bodies are untouched.

**Tech Stack:** Rust, `opentelemetry` / `opentelemetry_sdk` / `opentelemetry-otlp` 0.30.0, `tracing-opentelemetry`, clap.

## Global Constraints

- **TDD:** every behavior change is introduced test-first (failing test → minimal code → green).
- **Coverage policy:** `server/src` is testable logic — cover new lines with real tests, do **not** lower the coverage baseline to absorb them. Both `Drop` error branches are covered deterministically via double-`shutdown()` (see Task 1).
- **No `#[cfg(test)]` in dialect files:** N/A (no dialect files touched).
- **No Co-Authored-By trailers** in any commit.
- **Per-task gate:** `cargo xtask check --no-test` (clippy + fmt) during iteration; final gate is `cargo xtask validate --no-e2e`. Invoke bare via context-mode or the worktree shell; never append `; echo`/`| tee`.
- **OTel version:** `opentelemetry_sdk` 0.30.0. `SdkMeterProvider` and `SdkTracerProvider` are `Clone` (Arc-backed) and expose `shutdown(&self) -> opentelemetry_sdk::error::OTelSdkResult`; a second `shutdown()` returns `Err(OTelSdkError::AlreadyShutdown)`. Confirm by compiling — do not hand-edit around a signature mismatch without re-reading the crate docs.
- **Files touched:** `server/src/observability.rs` (guard + signatures + tests), `server/src/main.rs` (bind guard in `run()`), `server/src/commands.rs` (bind guard in `cmd_serve`). Command bodies in `commands.rs` are **not** modified.

---

### Task 1: `TelemetryGuard` — retain providers and flush on exit

**Files:**
- Modify: `server/src/observability.rs` (signatures `build_otel_tracer`, `build_otel_meter`, `init_tracing_impl`, `init_tracing`; remove `Once`; add `TelemetryGuard` + `Drop`; add tests)
- Modify: `server/src/main.rs:33-36` (bind guard across `run()` dispatch)
- Modify: `server/src/commands.rs:410` (bind guard for server lifetime)
- Test: `server/src/observability.rs` `#[cfg(test)] mod tests`

**Interfaces:**
- Produces:
  - `pub struct TelemetryGuard { meter: Option<opentelemetry_sdk::metrics::SdkMeterProvider>, tracer: Option<opentelemetry_sdk::trace::SdkTracerProvider> }` (fields private; the in-module test builds it via struct literal).
  - `pub fn init_tracing(verbose: bool) -> TelemetryGuard`
  - `fn init_tracing_impl(verbose: bool) -> TelemetryGuard`
  - `fn build_otel_tracer(endpoint: &str) -> Result<opentelemetry_sdk::trace::SdkTracerProvider, String>`
  - `fn build_otel_meter(endpoint: &str) -> Result<opentelemetry_sdk::metrics::SdkMeterProvider, String>`
- Consumes: existing `otel_exporter_otlp_endpoint()`, `resolved_filter`, `use_json_format`, `SlowSpanLayer`, and the `use opentelemetry::trace::TracerProvider as _;` import (kept — `init_tracing_impl` now calls `provider.tracer("jaunder")`).

- [ ] **Step 1: Write the failing flush-on-drop tests**

Add to the `#[cfg(test)] mod tests` block in `server/src/observability.rs`. Add the import line `use opentelemetry_sdk::metrics::{InMemoryMetricExporter, PeriodicReader, SdkMeterProvider};` and `use opentelemetry_sdk::trace::{InMemorySpanExporter, SdkTracerProvider};` at the top of the test module (next to the existing `use super::*;`). (Mirror the working metrics template at `common/src/metrics.rs:240-262`.)

```rust
#[tokio::test]
async fn guard_drop_flushes_meter_provider() {
    use opentelemetry::metrics::MeterProvider as _;
    let exporter = InMemoryMetricExporter::default();
    let reader = PeriodicReader::builder(exporter.clone()).build();
    let provider = SdkMeterProvider::builder().with_reader(reader).build();

    // Emit a counter on this provider, then let the guard's Drop flush it.
    let counter = provider.meter("test").u64_counter("test.counter").build();
    counter.add(1, &[]);
    drop(TelemetryGuard {
        meter: Some(provider),
        tracer: None,
    });

    let metrics = exporter.get_finished_metrics().expect("metrics");
    let found = metrics
        .iter()
        .flat_map(opentelemetry_sdk::metrics::data::ResourceMetrics::scope_metrics)
        .flat_map(opentelemetry_sdk::metrics::data::ScopeMetrics::metrics)
        .any(|metric| metric.name() == "test.counter");
    assert!(found, "metric not exported on guard drop");
}

#[tokio::test]
async fn guard_drop_flushes_tracer_provider() {
    use opentelemetry::trace::{Tracer as _, TracerProvider as _};
    let exporter = InMemorySpanExporter::default();
    let provider = SdkTracerProvider::builder()
        .with_batch_exporter(exporter.clone())
        .build();

    provider.tracer("test").in_span("test-span", |_cx| {});
    drop(TelemetryGuard {
        meter: None,
        tracer: Some(provider),
    });

    let spans = exporter.get_finished_spans().expect("spans");
    assert!(
        spans.iter().any(|span| span.name == "test-span"),
        "span not exported on guard drop"
    );
}

#[test]
fn guard_drop_is_noop_when_inert() {
    // No OTLP endpoint configured → both providers None → Drop does nothing and
    // must not panic.
    drop(TelemetryGuard {
        meter: None,
        tracer: None,
    });
}

#[tokio::test]
async fn guard_drop_swallows_shutdown_errors() {
    // A second shutdown() returns AlreadyShutdown; Drop must log, not panic or
    // propagate. Covers both Err branches in Drop.
    let metric_exporter = InMemoryMetricExporter::default();
    let meter = SdkMeterProvider::builder()
        .with_reader(PeriodicReader::builder(metric_exporter).build())
        .build();
    let span_exporter = InMemorySpanExporter::default();
    let tracer = SdkTracerProvider::builder()
        .with_batch_exporter(span_exporter)
        .build();

    // First guard shuts both down cleanly.
    drop(TelemetryGuard {
        meter: Some(meter.clone()),
        tracer: Some(tracer.clone()),
    });
    // Second guard's shutdown() now errors (already shut down); Drop swallows it.
    drop(TelemetryGuard {
        meter: Some(meter),
        tracer: Some(tracer),
    });
}
```

- [ ] **Step 2: Run the tests to verify they fail**

Run: `cargo nextest run -p jaunder observability::tests::guard_`
Expected: FAIL — `cannot find struct TelemetryGuard` (not yet defined) / `init_tracing_impl` field types. Compilation error is the expected failure here.

- [ ] **Step 3: Add `TelemetryGuard` + `Drop`**

In `server/src/observability.rs`, after `init_tracing` (replacing nothing yet), add:

```rust
/// Owns the OTLP providers installed by [`init_tracing`] so a short-lived
/// process flushes buffered telemetry before exit. The periodic metric reader
/// and batch span processor only export on their interval — which a one-shot
/// CLI command exits long before — so without this the CLI's metric and span
/// emits are silently dropped. Holding the guard for the command's scope and
/// letting `Drop` run `shutdown()` (force-flush + shutdown) exports them on
/// every exit path: success, `?` error-return, and panic unwind.
///
/// Both fields are `None` when no OTLP endpoint is configured, making the guard
/// an inert no-op (the common dev/test case).
pub struct TelemetryGuard {
    meter: Option<opentelemetry_sdk::metrics::SdkMeterProvider>,
    tracer: Option<opentelemetry_sdk::trace::SdkTracerProvider>,
}

impl Drop for TelemetryGuard {
    fn drop(&mut self) {
        // A telemetry-export failure (e.g. the collector is unreachable) must
        // never change the command's outcome, so errors are logged, not
        // propagated — mirroring the non-fatal exporter-setup handling in
        // `init_tracing_impl`.
        if let Some(meter) = self.meter.take() {
            if let Err(error) = meter.shutdown() {
                eprintln!("OTel meter provider shutdown failed during flush: {error}");
            }
        }
        if let Some(tracer) = self.tracer.take() {
            if let Err(error) = tracer.shutdown() {
                eprintln!("OTel tracer provider shutdown failed during flush: {error}");
            }
        }
    }
}
```

- [ ] **Step 4: Change `build_otel_tracer` / `build_otel_meter` to return their providers**

Replace `build_otel_tracer` (currently `observability.rs:51-63`) with:

```rust
fn build_otel_tracer(endpoint: &str) -> Result<opentelemetry_sdk::trace::SdkTracerProvider, String> {
    let exporter = opentelemetry_otlp::SpanExporter::builder()
        .with_tonic()
        .with_endpoint(endpoint)
        .build()
        .map_err(|error| format!("failed to build OTLP span exporter: {error}"))?;
    let provider = opentelemetry_sdk::trace::SdkTracerProvider::builder()
        .with_batch_exporter(exporter)
        .build();
    // Clone into the global registry; keep the original handle for flush-on-exit.
    opentelemetry::global::set_tracer_provider(provider.clone());
    Ok(provider)
}
```

Replace `build_otel_meter` (currently `observability.rs:65-76`) with:

```rust
fn build_otel_meter(endpoint: &str) -> Result<opentelemetry_sdk::metrics::SdkMeterProvider, String> {
    let exporter = opentelemetry_otlp::MetricExporter::builder()
        .with_tonic()
        .with_endpoint(endpoint)
        .build()
        .map_err(|error| format!("failed to build OTLP metric exporter: {error}"))?;
    let provider = opentelemetry_sdk::metrics::SdkMeterProvider::builder()
        .with_periodic_exporter(exporter)
        .build();
    opentelemetry::global::set_meter_provider(provider.clone());
    Ok(provider)
}
```

- [ ] **Step 5: Make `init_tracing_impl` build the guard, and `init_tracing` return it; remove `Once`**

In `init_tracing_impl` (currently `observability.rs:155`), change the signature to `-> TelemetryGuard` and replace the OTel-layer + metrics section (currently lines 180-198) and the trailing `try_init` block so it reads:

```rust
fn init_tracing_impl(verbose: bool) -> TelemetryGuard {
    // ... unchanged: LogTracer::init, set_text_map_propagator, env_filter,
    //     slow_span_layer, fmt_layer ...

    // Resolve the endpoint once; traces and metrics share it.
    let endpoint = otel_exporter_otlp_endpoint();

    // Build the tracer provider (if configured), derive the layer from it, and
    // retain the provider so the guard can flush it on exit.
    let tracer = endpoint
        .as_deref()
        .and_then(|endpoint| match build_otel_tracer(endpoint) {
            Ok(provider) => Some(provider),
            Err(error) => {
                eprintln!(
                    "OTel disabled because exporter setup failed (endpoint {endpoint}): {error}"
                );
                None
            }
        });
    let otel_layer = tracer
        .as_ref()
        .map(|provider| tracing_opentelemetry::layer().with_tracer(provider.tracer("jaunder")));

    // Metrics share the OTLP endpoint with traces; setup failure is non-fatal.
    let meter = endpoint
        .as_deref()
        .and_then(|endpoint| match build_otel_meter(endpoint) {
            Ok(provider) => Some(provider),
            Err(error) => {
                eprintln!(
                    "OTel metrics disabled because exporter setup failed (endpoint {endpoint}): {error}"
                );
                None
            }
        });

    if let Err(error) = tracing_subscriber::registry()
        .with(env_filter)
        .with(slow_span_layer)
        .with(fmt_layer)
        .with(otel_layer)
        .try_init()
    {
        eprintln!("tracing subscriber init failed (continuing without it): {error}");
    }

    TelemetryGuard { meter, tracer }
}
```

Replace `init_tracing` (currently `observability.rs:215-217`) and delete the `Once`:

```rust
pub fn init_tracing(verbose: bool) -> TelemetryGuard {
    init_tracing_impl(verbose)
}
```

Delete `use std::sync::Once;` (line 1) and `static INIT_TRACING: Once = Once::new();` (line 18). Idempotency was previously enforced by `Once`; it is now moot because `init_tracing` is called exactly once per process (production), and `try_init` / `LogTracer::init` already report repeat installs non-fatally (exercised by `init_tracing_impl_reports_failure_when_already_initialized`, and by the `run()` tests that dispatch twice in one process with no endpoint → inert guards).

- [ ] **Step 6: Bind the guard at the two call sites**

In `server/src/main.rs`, replace the `if !matches!(...)` block (lines 34-36) — hoist the guard to function scope so it outlives the dispatch:

```rust
pub async fn run(cli: Cli) -> anyhow::Result<()> {
    // Hold the telemetry guard for the whole command so its Drop flushes the
    // OTLP exporters before this one-shot process exits. `serve` initializes
    // and flushes telemetry itself, so it is skipped here. Binding at function
    // scope — not inside an `if` block — is load-bearing: the guard must outlive
    // the command dispatch below, not drop early.
    let _telemetry = (!matches!(cli.command, Some(Commands::Serve { .. })))
        .then(|| jaunder::observability::init_tracing(cli.verbose));
    let command = match cli.command {
        // ... unchanged ...
```

In `server/src/commands.rs:410`, replace `crate::observability::init_tracing(verbose);` with a binding held for the server's lifetime:

```rust
    let _telemetry = crate::observability::init_tracing(verbose);
```

(Footgun: a bare `init_tracing(verbose);` statement — or `let _ = ...` — would drop the guard immediately and shut telemetry down right after install. Use the named binding `_telemetry`.)

- [ ] **Step 7: Run the new tests and verify they pass**

Run: `cargo nextest run -p jaunder observability::tests::guard_`
Expected: PASS (4 tests: flush meter, flush tracer, inert no-op, swallow shutdown errors).

- [ ] **Step 8: Run the full observability + run/serve suites to confirm no regression**

Run: `cargo nextest run -p jaunder observability:: run_ cmd_serve`
Expected: PASS — existing `init_tracing_impl_*`, `run_*`, and serve tests still green (each now drops a guard; with no/refused OTLP endpoint, `shutdown()` returns promptly).

- [ ] **Step 9: Gate**

Run: `cargo xtask check --no-test`
Expected: clippy + fmt clean (exit 0). Fix any lint before committing.

- [ ] **Step 10: Commit**

```bash
git add server/src/observability.rs server/src/main.rs server/src/commands.rs
git commit -m "feat(observability): flush OTel telemetry before one-shot CLI exit (#12)"
```

---

### Task 2: Record the convention as an ADR-0011 addendum

**Files:**
- Modify: `docs/adr/0011-unified-observability.md` (append a dated addendum)

**Interfaces:** none (docs only).

- [ ] **Step 1: Append the addendum**

At the end of `docs/adr/0011-unified-observability.md`, add:

```markdown
## Addendum (2026-06-27): Flushing telemetry from one-shot processes (issue #12)

The metrics addendum above installs an OTLP `MeterProvider` (and the tracer
installs a `TracerProvider`) gated on the OTLP endpoint. Both exporters are
deferred — the metric reader is periodic and the span processor batches — so
they only export on an interval the long-running server easily reaches but a
one-shot CLI command exits long before. The CLI's metric and span emits were
therefore installed correctly yet silently dropped.

Convention: `init_tracing` returns a `TelemetryGuard` owning the installed
providers. A process holds the guard for its working scope; the guard's `Drop`
calls `shutdown()` (force-flush + shutdown) on each provider, exporting buffered
telemetry on every exit path — success, `?` error-return, and panic unwind. A
single binding at the `run()` dispatch boundary covers every current and future
one-shot command, so command bodies carry no telemetry-lifecycle code. The
server binds the same guard for its process lifetime. Export failures (e.g. an
unreachable collector) are logged, never propagated — a telemetry failure must
not change a command's exit status. This closes the "CLI export" item the
metrics addendum deferred.
```

- [ ] **Step 2: Confirm no `docs/README` ADR-table change is needed**

ADR-0011 is already listed in the `docs/README.md` table; an addendum adds no new row. (If a reviewer prefers a standalone ADR-0026 instead, that is a scoped redo of this task — flagged at the spec gate.)

- [ ] **Step 3: Commit**

```bash
git add docs/adr/0011-unified-observability.md
git commit -m "docs(adr-0011): record one-shot telemetry-flush convention (#12)"
```

---

## Self-Review

**Spec coverage:**
- Retain provider handles → Task 1 Steps 3-4 (builders return providers; guard owns them). ✓
- Flush metrics + traces before exit → Task 1 Step 5 (`init_tracing_impl` builds both into the guard), Step 3 (`Drop` shuts both down). ✓
- `run()`-boundary flush site, no per-command boilerplate → Task 1 Step 6. ✓
- RAII guard, all exit paths → Task 1 Step 3 (`Drop`) + tests Step 1. ✓
- Error handling: emit-then-fail, no-endpoint inert, swallowed flush failure → tests `guard_drop_*`, and the no-endpoint path makes the guard inert. ✓
- Testing: in-memory metric + span exporters, drop ⇒ flush, inert no-op → Task 1 Step 1. ✓
- ADR-0011 addendum → Task 2. ✓
- Non-goals (gauges #13, tracing #16, command-body emits unchanged) → not touched; `commands.rs` edit is only the `cmd_serve` binding. ✓

**Placeholder scan:** none — all steps carry concrete code/commands. The single deferred detail (exact log wording) is fully specified in the code blocks.

**Type consistency:** `TelemetryGuard { meter: Option<SdkMeterProvider>, tracer: Option<SdkTracerProvider> }` used identically in the struct def (Step 3), `init_tracing_impl` return (Step 5), and all four tests (Step 1). `init_tracing(verbose) -> TelemetryGuard` consumed as `Option<TelemetryGuard>` via `.then(...)` in `run()` and as `TelemetryGuard` in `cmd_serve` — both valid. Builder return types match their callers in `init_tracing_impl`.
