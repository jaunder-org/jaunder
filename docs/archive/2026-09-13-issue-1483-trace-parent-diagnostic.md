# Issue #1483: Trace-parent diagnostic policy

## Outcome

A server without an active OTLP tracing layer starts and serves requests without
printing
`server.observability.trace_parent: request trace parent assignment failed`.
When OTLP tracing initialization succeeds, a genuine parent-assignment failure
retains the existing fixed, data-free stderr diagnostic.

## Load-bearing decisions

- Trace-parent adoption follows the initialized telemetry capability. Jaunder
  does not infer runtime capability from endpoint presence or reread process
  environment variables in request middleware.
- Without an installed OTLP tracing layer, inbound W3C trace-context extraction
  and parent assignment are disabled. This includes a missing endpoint and
  tracer-exporter or subscriber-layer setup failure after its one startup
  diagnostic. Request tracing remains local, matching ADR-0011's
  no-op-without-a-provider contract.
- With an OTLP tracing layer installed, inbound trace context is extracted and
  the request span attempts to adopt it exactly as today.
- An active parent-assignment failure remains an extreme observability
  diagnostic and uses the existing fixed stderr fallback. It does not route
  through tracing and cannot recurse through the failing observability path.
- Request IDs, request/response spans, log filtering, diagnostic capture, and
  exporter setup/failure policy remain unchanged.
- Issue #1484 is operationally adjacent because its payload also appears during
  `cargo xtask` startup, but it has a separate source and contract: the sandbox
  seeder emits a machine-readable manifest which xtask currently inherits. This
  issue does not change that command output.
- This specializes existing ADR-0011 behavior rather than introducing a new
  architecture or domain concept; no ADR or CONTEXT.md change is required.

## Acceptance

- Serving a request without an installed OTLP tracing layer emits no
  trace-parent fallback, including when the request carries a valid
  `traceparent` header.
- Serving with an installed OTLP tracing layer still extracts inbound trace
  context and attempts to parent the request span.
- If active parent assignment fails, stderr contains exactly the existing fixed
  diagnostic and no request or header data.
- Request ID propagation and ordinary request-span logging behave unchanged in
  both configurations.
- Existing e2e trace attribution remains intact when the e2e OTLP endpoint is
  configured.
- HTTP-boundary regression coverage uses an injected counting or failing
  propagator: the disabled policy performs zero extraction calls, while the
  enabled policy proves extraction and parent-assignment behavior. It does not
  assert source text or reread process environment.

## Boundaries

- Do not silence exporter setup, diagnostic-log, panic, or other observability
  failures.
- Do not weaken W3C trace-context propagation when an OTLP tracing layer is
  installed.
- Do not install a no-op OpenTelemetry span layer merely to absorb assignment;
  disabled tracing should avoid per-request extraction and OTel work.
- Do not change sandbox seed output or otherwise implement issue #1484 here.
