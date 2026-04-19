# Observability

This project emits OpenTelemetry traces from both the backend and end-to-end
test runner.

## Backend

- Backend spans are produced via `tracing` + OpenTelemetry in the `server`
  crate.
- In e2e VM checks, traces are exported to the in-VM collector and written to:
  - `/var/lib/jaunder/otel-traces.jsonl` (inside VM)
  - `otel-traces-sqlite.jsonl/otel-traces.jsonl` (copied artifact)
  - `otel-traces-postgres.jsonl/otel-traces.jsonl` (copied artifact)

## End-to-End Tracing Layers

E2E tracing currently has two complementary layers:

- `e2e.test` (automatic, from `end2end/tests/fixtures.ts`)
  - one span per test
  - request timing summary
  - navigation lifecycle summary (`e2e.navigation_top_json`)
  - includes `commit -> hydration` timing when hydration is observed
  - resource summary
  - timed action summary (`e2e.action_top_json`)
- `e2e.flow.*` (manual semantic phases, from `end2end/tests/perf.ts`)
  - opt-in for selected scenarios
  - mark-to-mark phase timing for domain-specific flow analysis

Both layers use the same trace context (`JAUNDER_E2E_TRACEPARENT`) so browser
and backend spans are correlated in a single trace.

## Analysis

Use `scripts/analyze-otel-traces` on one or more artifact files, for example:

```bash
scripts/analyze-otel-traces \
  /nix/store/...-vm-test-run-jaunder-e2e-sqlite/otel-traces-sqlite.jsonl/otel-traces.jsonl \
  /nix/store/...-vm-test-run-jaunder-e2e-postgres/otel-traces-postgres.jsonl/otel-traces.jsonl
```

The analyzer reports:

- slowest spans overall
- slowest `e2e.test` spans
- top e2e action hotspots
- top navigation phase hotspots and slow targets
- per-project/browser e2e duration breakdown
- per-trace duration totals

To run both e2e VM checks and immediately analyze the produced traces, use:

```bash
scripts/run-e2e-trace-analysis --top 25
```

Optional filters:

- `--top N` controls how many rows each section prints.
- `--trace TRACE_ID` restricts analysis to one trace id.

## WASM Bundle Audit

Use `scripts/audit-wasm-bundle` to measure frontend bundle size from the
deterministic Nix `site` build output:

```bash
scripts/audit-wasm-bundle
```

This reports raw, gzip, and brotli sizes for:

- `pkg/jaunder_bg.wasm`
- `pkg/jaunder.js`

Useful options:

- `--json` for machine-readable output
- `--site-path /nix/store/...-jaunder-site` to reuse a previously built site output
