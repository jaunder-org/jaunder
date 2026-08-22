# Issue #16 — Decision-path tracing

## Outcome

Jaunder's operator telemetry should answer why a request reached a failing line
without replaying every log line. Server failures will carry the active span
stack and its recorded determinant fields, and representative control-flow
decisions will be recorded as queryable span fields rather than encoded as extra
span names.

## Load-bearing decisions

- Decision-path observability is an extension of ADR-0011's OpenTelemetry model:
  each meaningful unit of work remains its own span, and branch determinants are
  fields on the narrowest span that owns that decision rather than new
  branch-specific span names.
- Determinant fields are declared per instrumentation site, not globally. A span
  declares the fields it may later record, then records each value when that
  value becomes known. This keeps field names visible at the owning span and
  avoids whole-struct dumps.
- Determinants may include bounded decisions, stable internal identifiers, and
  non-secret public routing identifiers. They must not include passwords,
  tokens, raw emails, free-form request bodies, or arbitrary source text.
- `InternalError` captures a `tracing_error::SpanTrace` at construction time,
  while the active spans and their fields still exist. Boundary logging emits
  that span trace with the existing `error.kind`, `error.class`, `error.public`,
  `error.source`, and `error.context` fields.
- SpanTrace is operator-only. It never crosses the `InternalError -> WebError`
  projection and must not alter public responses.
- Determinant fields are ordinary span attributes when a trace is exported;
  `SpanTrace` is the error-time snapshot of the active span stack, not the only
  place those values exist.
- Continued-after-error paths are part of decision-path observability. Boundary
  `InternalError` failures must capture `SpanTrace`; swallowed/continued error
  reporting must either capture equivalent active span context in this issue or
  explicitly document why it remains a follow-up.
- Tail decisioning reuses the existing host telemetry close hook shape: the
  implementation may emit extra diagnostics for errored or slow spans, but
  normal successful requests must not gain an always-on branch-log stream.
- The deleted `boundary!` macro is not revived. Any server-function-wide
  automation belongs in the current `#[macros::server]` expansion or in ordinary
  helpers the macro can call.
- Durable guidance is required: update the observability
  architecture/contributor guidance and record the new convention as an
  ADR-0011-extending decision if the implementation adds SpanTrace or changes
  server-fn instrumentation policy.

## Acceptance

- An `InternalError` constructed inside nested instrumented spans includes an
  operator-only span trace showing the active span stack and fields recorded
  before the error was created.
- A boundary failure log includes the span trace together with the existing
  structured error fields, and the outward `WebError` projection remains
  unchanged.
- Continued-after-error reporting is either given equivalent active-span context
  or documented as a deliberately deferred follow-up with rationale.
- At least one existing branch encoded as a server-side tracing span name is
  migrated to determinant fields on the enclosing unit-of-work span, with tests
  proving the emitted fields are present on success/failure as applicable.
- Slow/error-only diagnostic retention is either implemented through the
  existing slow-span hook or explicitly documented as collector-side tail
  sampling; successful non-slow spans do not emit extra buffered branch logs.
- The contributor/architecture documentation states the determinant-field
  convention, cardinality/PII limits, and the preference for fewer wider spans
  over branch-name span proliferation.
- `cargo xtask check` passes.

## Boundaries

- This issue does not build a new telemetry backend, collector deployment,
  dashboard, or sampling policy outside Jaunder's process.
- This issue does not rename the macro-derived `web.<vertical>.<ident>`
  server-fn spans except where replacing a branch-specific child span with
  determinant fields is part of the accepted example.
- This issue does not make `SpanTrace` public, serializable to clients, or part
  of any API response.
- This issue does not attempt a repository-wide conversion of every conditional
  branch; one representative migrated path plus durable convention is
  sufficient.
- This issue does not relax existing recordable-type, PII, or secret-redaction
  rules.
