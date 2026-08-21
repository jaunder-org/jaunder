# #769 — trace every host process that writes the e2e database

Issue: [#769](https://github.com/jaunder-org/jaunder/issues/769). Milestone:
Observability & diagnostics. Provenance: #766 diagnosis and ADR-0011's one-shot
telemetry guard.

## Summary

The e2e trace currently explains the server side of a failure but not every
process that mutates the same database. The `test-support` binary seeds users,
sessions, and Posts by calling the real `storage` paths from a separate process,
but it never initializes the OpenTelemetry tracer/exporter. Its `storage.*`
spans are therefore absent from `capture/otel-traces.jsonl`, even though the e2e
harness already provides `JAUNDER_OTEL_EXPORTER_OTLP_ENDPOINT` to the VM.

The production `jaunder` CLI already initializes telemetry for every command at
`server/src/main.rs::run` and flushes both traces and metrics through
`TelemetryGuard` on exit (ADR-0011's 2026-06-27 addendum). This cycle must keep
that behavior and prove it while adding the same process-scope telemetry
lifecycle to `test-support`.

## Evidence

- `test-support/src/main.rs` calls `storage::open_existing_database` in
  `seed-posts`, `create-user`, `seed-user`, and `create-session`, but its
  `main()` dispatch owns no telemetry guard.
- The storage layer already emits spans such as `storage.open_existing_database`
  and `storage.posts.create_batch`.
- `flake.nix` sets `JAUNDER_OTEL_EXPORTER_OTLP_ENDPOINT=http://127.0.0.1:4317`
  for the e2e VM service environment and runs e2e seeding through the
  `test-support` binary.
- `server/src/main.rs::run` already binds
  `let _telemetry = jaunder::observability::init_tracing(cli.verbose);` across
  every production CLI command, including one-shot storage writers such as
  `site-config set`.

## Decisions

- **D1 — Both `test-support` and the production CLI are in scope.**
  `test-support` is the observed gap; the CLI is named by the issue and remains
  a regression surface even though the current tree already implements its
  lifecycle.

- **D2 — Split OTLP process telemetry from server-scoped diagnostics.**
  `server::observability` is currently doing three jobs: OTLP process telemetry,
  HTTP router observability, and server-only diagnostic capture. `test-support`
  must not depend on the heavy `server` crate merely to export storage spans,
  but it also must not become a writer for `diag.log`. Move only the shared OTLP
  process interface — `init_tracing(verbose) -> TelemetryGuard`, endpoint/filter
  resolution, OTLP provider construction, the JSON/pretty fmt layer, slow-span
  logging, and provider shutdown — behind a small `host` interface. Keep the
  scoped diag layer and panic hook in `server`, and install them only for the
  live server process. Keep the Axum request layer and traceparent request
  extraction in `server`.

- **D3 — One guard per process, held across the command dispatch.** The caller
  binds `TelemetryGuard` once after clap has selected a runnable subcommand and
  before any storage work starts. Dropping the guard remains the only flush
  path; command bodies do not own telemetry lifecycle code.

- **D4 — No endpoint means no-op, not failure.** The interface keeps ADR-0011's
  behavior: when neither `JAUNDER_OTEL_EXPORTER_OTLP_ENDPOINT` nor
  `OTEL_EXPORTER_OTLP_ENDPOINT` is set, telemetry setup is inert and commands
  run unchanged. Invalid endpoints, exporter failures, subscriber-install
  failure, and flush failure report fallback diagnostics but never change a
  command's exit status.

- **D5 — Existing e2e trace context is enough.** This issue does not create a
  separate test-support trace protocol. The harness already exports the same
  OTLP endpoint to the process environment; the accepted observable contract is
  that storage spans from `test-support` appear in the collector output.
  Parentage to a specific `e2e.test` span is out of scope unless the existing
  environment already provides it without new protocol.

- **D6 — Document the new home as an ADR-0011/ADR-0057 projection.** Moving OTLP
  exporter setup from `server::observability` to `host` changes the current
  architecture but not the observability policy. Server-scoped diagnostics
  remain server-owned per ADR-0057. Update ADR-0011 with a dated addendum,
  project the result into `docs/ARCHITECTURE.md`, and update
  `docs/observability.md`; no new ADR is needed unless implementation uncovers a
  policy change beyond this seam split.

## Acceptance criteria

- **AC1.** `test-support` initializes process telemetry once for every runnable
  subcommand and holds the returned guard across the subcommand's whole
  dispatch.

- **AC2.** A focused test or smoke harness proves a `test-support` DB-writing
  subcommand exports at least one storage span to an OTLP receiver when
  `JAUNDER_OTEL_EXPORTER_OTLP_ENDPOINT` is set.

- **AC3.** The same proof covers shutdown flushing: the span must be observed
  after a short-lived `test-support` process exits, without relying on a
  periodic exporter interval.

- **AC4.** `test-support` remains a no-op telemetry caller when no OTLP endpoint
  is set; existing command behavior and exit status are unchanged.

- **AC5.** Production CLI telemetry remains process-scoped: `jaunder` still
  binds one `TelemetryGuard` across each runnable command, including
  storage-writing subcommands such as `site-config set`.

- **AC6.** A focused test or smoke harness proves a short-lived `jaunder`
  storage-writing CLI command exports at least one storage span to an OTLP
  receiver when `JAUNDER_OTEL_EXPORTER_OTLP_ENDPOINT` is set.

- **AC7.** Telemetry transport/setup/shutdown failures do not fail either
  binary; they emit fallback diagnostics only, preserving ADR-0011's operational
  rule.

- **AC8.** `server` remains the home of HTTP request observability and scoped
  server diagnostics: `with_http_observability`, inbound `traceparent`
  extraction, the diag `tracing` layer, and the diagnostic panic hook stay in
  the server layer. `test-support` and one-shot CLI commands must not write
  `diag.log` merely because `JAUNDER_CAPTURE_DIR` is set.

- **AC9.** Documentation reflects the shipped design: ADR-0011, the architecture
  view, and `docs/observability.md` name `host` as the OTLP process telemetry
  home and state that `server`, production CLI commands, and `test-support` use
  the same guard; ADR-0057/server-diagnostics prose continues to name the server
  as the scoped diagnostic writer.

- **AC10.** `cargo xtask validate` passes before shipping.

## Out of scope

- Adding new e2e traceparent propagation from Playwright to `test-support`.
- Changing storage span names, DB queries, seed behavior, or CLI user-visible
  output.
- Adding metrics instruments beyond whatever existing storage/server metrics are
  flushed by the shared guard.
- Re-analyzing historical #766 captures; this issue fixes future capture
  completeness.
