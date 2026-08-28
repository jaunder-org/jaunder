# Issue #1243: E2E collector receiver readiness

## Outcome

Every NixOS E2E VM waits until the OpenTelemetry collector's OTLP/gRPC and
OTLP/HTTP receivers accept connections before starting Jaunder or one-shot seed
exporters. Cold VM startup cannot lose the seed trace merely because systemd
marked the collector process active before its listeners were ready.

## Load-bearing decisions

- `otel-collector.service` being active proves process spawn, not receiver
  readiness; it is never sufficient by itself.
- Readiness probes the actual fixed loopback receivers: port 4317 for OTLP/gRPC
  and port 4318 for OTLP/HTTP.
- One shared VM test-script fragment owns receiver readiness for both SQLite and
  PostgreSQL and every browser combination.
- The fragment runs after each collector start: initial VM boot and the restart
  performed after seed-span verification.
- Jaunder initialization, seed commands, and Playwright begin only after both
  receiver probes pass.
- Readiness is fail-closed and bounded. No sleep, exporter retry, or weakened
  seed assertion hides an unavailable collector.

## Acceptance

- SQLite and PostgreSQL E2E test scripts wait for both collector ports after the
  collector unit becomes active and before starting Jaunder or seeding.
- The collector restart inside seed-span verification waits for both ports
  before Jaunder is restarted.
- The existing seed command exports both `e2e.seed.jaunder` and
  `e2e.seed.test-support` storage spans without `tcp connect error`.
- `assert_seed_storage_spans` remains strict and still fails a missing or
  malformed trace.
- All four backend/browser E2E checks use the same readiness policy.

## Boundaries

- No collector port, protocol, trace schema, or production service change.
- No fixed delay or retry around a failed seed assertion.
- No Playwright behavior, backend setup, or capture-retention change.
