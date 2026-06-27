# Issue #12 — export telemetry from one-shot CLI commands

* Status: approved
* Date: 2026-06-27
* Issue: [#12](https://github.com/jaunder-org/jaunder/issues/12) (milestone 2 — "Code analysis 2026-06-12"; ex-beads `jaunder-kq8w.25`)
* Related: ADR-0011 (Unified Observability), `docs/superpowers/specs/2026-06-18-otel-metrics-pipeline-design.md`

## Problem

State-changing CLI commands emit operational signals through the `common::metrics`
facade — `cmd_user_create` → `jaunder.auth.registrations{source=cli}` and
`cmd_user_invite` → `jaunder.auth.invites` — and also produce trace spans. Today
those emits are silently dropped.

The original issue framed this as "a one-shot CLI process never sets up a
MeterProvider." Reading the code (`server/src/main.rs:34-36`) shows that premise is
slightly off: `run()` already calls `observability::init_tracing(cli.verbose)` for
**every** non-`serve` command, which **does** install the OTLP `MeterProvider` and
`TracerProvider` when an endpoint is configured. The real defect is downstream:

* `build_otel_meter` / `build_otel_tracer` (`server/src/observability.rs:51-76`)
  move each provider into the global registry and **drop the local handle**. Nothing
  retains a handle, so nothing can later flush.
* The OTLP metric exporter is *periodic* and the span exporter is *batch* — both fire
  on an interval that a short-lived CLI process exits long before. The server gets
  away with this because it runs indefinitely.

So the fix is **retain the provider handles and flush before exit**, not "install a
provider." Both metrics *and* traces have the identical drop-on-exit problem, so the
fix covers both (decided during brainstorming — avoids a near-identical follow-up).

## Design

### `TelemetryGuard` (RAII)

A guard owning the retained providers, returned by `init_tracing` and held for the
working scope of the process. Its `Drop` flushes; binding it once at the `run()`
boundary means **no per-command boilerplate** — command bodies are untouched and a
future emitting one-shot command exports correctly just by being dispatched through
`run()`.

```rust
// server/src/observability.rs
pub struct TelemetryGuard {
    meter: Option<SdkMeterProvider>,
    tracer: Option<SdkTracerProvider>,
}

impl Drop for TelemetryGuard {
    fn drop(&mut self) {
        // shutdown() force-flushes then shuts the provider down.
        if let Some(m) = self.meter.take() {
            if let Err(e) = m.shutdown() {
                // telemetry-export failure must never affect the command; log only.
                eprintln!("telemetry: meter shutdown failed: {e}");
            }
        }
        if let Some(t) = self.tracer.take() {
            if let Err(e) = t.shutdown() {
                eprintln!("telemetry: tracer shutdown failed: {e}");
            }
        }
    }
}
```

(Exact log target/level and `eprintln!`-vs-`tracing` choice mirror the existing
non-fatal treatment at `observability.rs:192-198`; finalized during implementation.)

### Signature changes

* `build_otel_meter(endpoint) -> Result<SdkMeterProvider, String>` — clone the
  provider into `global::set_meter_provider`, **return** the retained clone instead
  of discarding it. `SdkMeterProvider` is `Arc`-backed, so the clone is cheap; the
  implementer verifies clone/`shutdown` semantics against the pinned
  `opentelemetry_sdk` version during TDD.
* `build_otel_tracer(endpoint) -> Result<SdkTracerProvider, String>` — symmetric.
* `init_tracing(verbose) -> TelemetryGuard` and `init_tracing_impl(verbose) ->
  TelemetryGuard` — change from `()`. When no OTLP endpoint is configured, both
  fields are `None` and the guard is an inert no-op. The `static INIT_TRACING: Once`
  idempotency guard stays; init still runs at most once per process.

### Flush site

* `run()` non-serve branch (`server/src/main.rs:34-36`): bind the guard for the whole
  dispatch — `let _telemetry = jaunder::observability::init_tracing(cli.verbose);` —
  so it drops at the end of `run()` no matter how the command exits.
* `cmd_serve` (`server/src/commands.rs:410`): bind it too; held for the process
  lifetime. Flushing at server shutdown is a harmless bonus (serve already exports
  continuously).

The only files that change are `server/src/observability.rs` (the guard + signatures)
and the two binding sites (`server/src/main.rs`, `server/src/commands.rs`).
`server/src/commands.rs` command bodies keep their existing one-line emits unchanged.

## Error handling & edge cases

* **Emit-then-fail.** `Drop` runs on the error-return (`?`) path and on panic unwind,
  so a command that emits and then fails still exports the buffered telemetry — which
  is exactly what you want for diagnosing the failure.
* **No OTLP endpoint** (common dev/test case). Guard holds `None`/`None`; `Drop` is a
  no-op. No flush attempt, no error, no added latency.
* **Flush failure / collector unreachable.** `shutdown()` errors are swallowed and
  logged; they must never change the CLI command's exit code or surface to the user.
* **Latency.** `shutdown()` blocks until flush completes or its internal timeout
  elapses — a small bounded delay added to CLI exit *only when* an endpoint is
  configured. That delay is the feature.

## Testing

Testable unit: **"the guard flushes its providers on drop."** Template:
`common/src/metrics.rs:240-260` (in-memory exporter + flush + assert export).

* **Test seam:** a constructor that builds a `TelemetryGuard` from injected in-memory
  exporters (`InMemoryMetricExporter`, `InMemorySpanExporter`) rather than from an
  OTLP endpoint — keeps tests off the network.
* **Metrics-on-drop:** build a guard wired to `InMemoryMetricExporter`, emit a
  counter, drop the guard, assert the sink received the metric.
* **Traces-on-drop:** same shape with `InMemorySpanExporter` and an emitted span.
* **No-endpoint inert:** `init_tracing` with no OTLP env var yields a guard whose drop
  is a no-op (no panic, nothing exported).

Backend Rust, so the repo coverage policy applies; the new guard logic is covered by
the unit tests above, living in `server/src/observability.rs`.

## ADR

Recorded as an **addendum to ADR-0011** (Unified Observability), not a new ADR:
ADR-0011 already owns the provider lifecycle and its existing addendum explicitly
deferred this work (the "Deferred → CLI export (jaunder-kq8w.25)" bullet). The
addendum records the convention:

> One-shot processes install the same OTLP providers as the server, but their
> periodic/batch exporters never fire before exit. They therefore hold the
> `TelemetryGuard` returned by `init_tracing` for their working scope; the guard's
> `Drop` force-flushes and shuts the providers down on every exit path (success,
> error, panic). Centralizing this at the `run()` dispatch boundary keeps command
> bodies free of telemetry-lifecycle code.

No new `docs/README` ADR-table row is needed (0011 is already listed).

## Scope / non-goals

* **In scope:** retain provider handles; flush metrics + traces before one-shot CLI
  exit; the guard; tests; the ADR-0011 addendum.
* **Out of scope:** saturation gauges via async observables (issue #13); decision-path
  tracing / SpanTrace / tail sampling (issue #16); any change to the instrument
  catalog or to the command bodies' emit logic; any new config knob (the existing
  `JAUNDER_OTEL_EXPORTER_OTLP_ENDPOINT` / `OTEL_EXPORTER_OTLP_ENDPOINT` gate is reused
  unchanged).
