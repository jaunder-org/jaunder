# ADR-0011: Unified Observability Strategy

* Status: accepted
* Deciders: mdorman, Gemini CLI
* Date: 2026-05-15

## Context and Problem Statement

Jaunder is a full-stack application with complex interactions between the backend (SSR, server functions) and the frontend (hydration, client-side routing). Traditional logging is insufficient for diagnosing performance bottlenecks or understanding the full lifecycle of a user request across these boundaries, especially during end-to-end testing.

## Decision Drivers

*   Visibility: End-to-end tracing from the test runner through the browser to the backend.
*   Performance: Ability to identify hydration hotspots and slow database queries.
*   Consistency: Using industry-standard protocols (OpenTelemetry).

## Decision Outcome

Chosen option: "Unified Observability with OpenTelemetry", because it provides a standard way to correlate spans across different environments and languages.

### Implementation Details

*   **Backend**: Uses the `tracing` crate with `tracing-opentelemetry` in the `server` crate.
*   **E2E Test Runner**: Playwright fixtures in `end2end/tests/fixtures.ts` generate spans and inject trace context.
*   **Correlation**: The `JAUNDER_E2E_TRACEPARENT` environment variable is used to propagate trace context from the test runner to the backend.
*   **Layered Tracing**:
    - `e2e.test`: Automatic, captures one span per test with resource and navigation summaries.
    - `e2e.flow`: Manual, captures domain-specific semantic phases (e.g., "login flow").
*   **Artifacts**: Traces are exported as JSONL files (`otel-traces.jsonl`) during CI and VM test runs for offline analysis.
*   **PII discipline**: Span fields and the structured error boundary (`error.source`, `error.context` in `web/src/error.rs`) are operator-only but are still exported to trace backends, so they MUST NOT carry user PII or secrets — email addresses, session/verification tokens, passwords, or post bodies. Record stable, non-sensitive identifiers instead (`user_id`, `db.system`, `error.kind`/`error.class`); usernames are public identifiers and acceptable. The preserved error source chain is built from typed errors (`sqlx::Error`, `io::Error`, parse errors), which carry structural/diagnostic text — not bound parameter values — so the chain is PII-free as long as constructors keep raw user input out of error messages.

## Consequences

*   Good: Deep visibility into hydration timing and backend performance during tests.
*   Good: Correlated traces make it easy to see exactly what happened in the backend during a specific E2E test step.
*   Bad: Adds some complexity to the test runner and backend initialization.
*   Bad: Generates large trace files that require specialized analysis scripts.
