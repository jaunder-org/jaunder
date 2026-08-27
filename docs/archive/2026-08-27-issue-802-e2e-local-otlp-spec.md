# Issue #802: Local E2E OTLP capture

## Outcome

`cargo xtask e2e-local` produces an OpenTelemetry JSONL capture for every
existing local E2E lifecycle, containing correlated Playwright and Jaunder
server spans in the VM gate's collector output format. The command reports the
retained artifact path so a developer can pass it to
`cargo xtask traces analyze` without running a Nix VM check.

## Load-bearing decisions

- `e2e-local` supervises the pinned `otelcol-contrib` binary supplied by the
  development shell; it does not implement a browser-only OTLP sink.
- Local capture uses the same collector pipeline and
  `JAUNDER_CAPTURE_DIR/otel-traces.jsonl` contract as the VM gate, preserving
  ADR-0011 and ADR-0057 rather than creating a second trace format or filename.
- A lifecycle is one existing `run_lifecycle` interval: one temporary SQLite
  database, Jaunder server, collector, and capture directory. Multiple
  Playwright invocations or projects inside that interval share one artifact;
  every new interval receives a distinct artifact.
- Each lifecycle allocates distinct ephemeral loopback ports for the collector's
  OTLP/gRPC and OTLP/HTTP receivers. Concurrent checkouts and runs must not
  contend for fixed ports 4317 or 4318.
- The Jaunder server receives its per-run OTLP/gRPC endpoint, and Playwright
  receives its per-run OTLP/HTTP endpoint and trace context. The resulting file
  includes both browser `e2e.*` spans and correlated server `request` spans.
- The collector starts before Jaunder and E2E seeding. It remains alive through
  Playwright fixture teardown, when browser spans are exported, and is then
  stopped cleanly so buffered spans reach the file before it is inspected or
  retained.
- Collector startup, premature exit, shutdown, or missing trace output is a
  local E2E infrastructure failure. The command must not report success after
  silently losing the capture it promises.
- Every lifecycle receives a unique retained run directory under the existing
  `.xtask` runtime/artifact area. The complete capture directory is retained on
  both success and failure, after capture has stopped, following ADR-0037's
  capture-before-assert discipline.
- When `otel-traces.jsonl` exists, the command prints its exact retained path.
  When capture fails before the file exists, it retains and reports the run
  directory plus a diagnostic naming the absent expected trace file.
- The command does not automatically run trace analysis or turn current analysis
  reports into new E2E gates.

## Acceptance

- Running `cargo xtask e2e-local feeds.spec.ts` completes the existing local E2E
  behavior and prints one retained trace path for each `run_lifecycle` interval
  it runs. Multiple Playwright projects or invocations within one interval share
  that path; snapshot-update mode's separate intervals receive separate paths.
- Each printed trace path exists after the command exits and names
  `otel-traces.jsonl` inside a retained capture directory.
- `cargo xtask traces analyze <printed-path>` parses the artifact successfully.
- The artifact contains a Playwright `e2e.test` span and a Jaunder server
  `request` span caused by that test. They share a trace ID, and the request's
  parent span ID is the causing `e2e.test` span ID.
- Two concurrent local E2E lifecycles can start collectors without an OTLP port
  collision.
- A Playwright failure still leaves and reports the partial trace capture before
  `e2e-local` returns failure.
- A collector that cannot become ready, exits early, fails to shut down cleanly,
  or fails to produce the trace artifact makes `e2e-local` fail with a
  diagnostic identifying the collector failure.
- A collector shutdown failure preserves any partial capture and does not mask
  an earlier Playwright failure; diagnostics report both before the command
  returns failure.
- The authoritative SQLite and PostgreSQL VM E2E captures continue to use the
  same collector pipeline and trace-file contract.

## Boundaries

- No PostgreSQL backend option or other backend change is added to `e2e-local`;
  its existing SQLite behavior remains the fast local loop.
- `devtool pg run`, Rust unit/integration test environments, and persistent
  development-service workflows are unchanged.
- No new trace schema, span, trace-analysis policy, server-function coverage
  gate, or automatic analysis step is introduced.
- VM parity covers the collector pipeline, JSONL contract, and browser/server
  signals required here; this issue does not add local assertions for the VM's
  seed-process span population.
- The VM E2E gate remains authoritative; local capture is an iteration aid, not
  a replacement gate.
