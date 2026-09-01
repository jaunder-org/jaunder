# ADR-DRAFT: Allow synthetic browser diagnostic payloads in isolated E2E telemetry

- Status: proposed
- Date: 2026-09-01
- Issue: [#839](https://github.com/jaunder-org/jaunder/issues/839)

## Context

[ADR-0011](../0011-unified-observability.md) requires exported telemetry to be
free of user PII and secrets. That rule is necessary for production traces,
which may reach operator-controlled backends and describe real users.

The disposable Playwright E2E environment has a different diagnostic boundary.
Its browser console warnings/errors and uncaught page errors can contain the
synthetic users, tokens, passwords, and application payloads deliberately seeded
for a test. Redacting those values would make an E2E failure harder to diagnose,
while the Playwright harness already confines this corpus to its isolated test
environment. [ADR-0096](../0096-e2e-trace-capture-vs-attribution.md) requires
page capture to attach at the context-level seam and assigns records before the
test phase to a non-test sink.

## Decision

Production telemetry remains PII- and secret-free without exception. Production
browser code MUST NOT install console/page-error listeners or export their raw
payloads.

The isolated, disposable Playwright E2E harness MAY export raw browser
diagnostic payloads, including synthetic application values, solely to diagnose
that E2E corpus. This exception is limited to console warnings and errors and
uncaught page errors captured while the harness drives its seeded test
environment. It does not permit real-user data, non-test deployments, or
infrastructure credentials in telemetry. Capturing a non-test deployment is
outside the supported contract.

The harness observes diagnostics only: a warning, error, or uncaught page error
MUST NOT itself fail a test. It retains the first 20 test-phase records per
owning span in sequence order and records the exact omitted remainder, so the
trace remains bounded without silently losing the likely root cause.

## Consequences

- E2E failure traces can preserve the synthetic diagnostic text and stacks that
  explain browser-side failures directly.
- The production data boundary in ADR-0011 remains the default and is not
  weakened for real deployments or operational telemetry.
- The harness must keep listener installation and raw-payload export inside its
  isolated Playwright code; a production exporter would violate this decision.
- Traces record observations, not assertions, so future policy changes such as
  allowlists or test failures require a separate decision backed by captured
  evidence.
