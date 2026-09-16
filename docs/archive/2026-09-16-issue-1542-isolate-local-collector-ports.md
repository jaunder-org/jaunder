# Isolate local collector ports

## Outcome

`cargo xtask e2e-local` can run concurrently from independent Jaunder checkouts
without either collector contending for a fixed host port. Each invocation
continues to own and clean up only its own collector process.

## Load-bearing decisions

- Disable the local collector's unused self-telemetry metrics endpoint instead
  of allocating a third ephemeral port.
- Apply that policy as a host-only collector invocation override. Keep the
  shared collector configuration and its VM behavior unchanged.
- Keep both OTLP receiver endpoints invocation-owned and ephemeral; they remain
  the only host listeners permitted for the local collector.
- Preserve readiness checks against the actual invocation-selected OTLP gRPC and
  HTTP endpoints before the server, seeding, or Playwright work begins.
- Preserve bounded, invocation-scoped collector shutdown and cleanup on success,
  startup failure, test failure, and interruption.
- Treat any future host-bound endpoint added to the local collector as unsafe by
  default unless it is disabled or explicitly allocated and owned per
  invocation.
- This is an operational correction within the accepted host E2E lifecycle and
  observability architecture; it introduces no new domain term or architectural
  decision.

## Acceptance

- A process already listening on `127.0.0.1:8888` does not prevent
  `cargo xtask e2e-local` from starting its collector.
- Two independent focused `e2e-local` runs can overlap without sharing or
  colliding on collector listeners.
- Collector startup still proves both selected OTLP receiver endpoints ready
  before exporters use them.
- Failure and shutdown paths terminate only the collector process started by
  that invocation and retain the existing diagnostic behavior.
- A runtime regression occupies `127.0.0.1:8888`, starts the local collector,
  and proves that both invocation-selected OTLP endpoints become ready.
- A default-deny configuration check permits only the OTLP gRPC and HTTP
  receivers with invocation-supplied endpoints, the batch processor, the file
  exporter, and their traces pipeline. Adding a receiver, extension, pipeline,
  or listener fails until its ownership policy and coverage are explicit.
- Automated checks also fail if the host invocation stops disabling collector
  self-telemetry metrics.

## Boundaries

- Do not change the NixOS, production, or VM collector configuration.
- Do not change application metrics/traces, capture contents, trace attribution,
  Playwright behavior, or endpoint protocols.
- Do not kill, discover, reuse, or coordinate with ambient collector processes.
