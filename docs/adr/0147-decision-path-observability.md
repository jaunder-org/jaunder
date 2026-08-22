# ADR-0147: Decision-Path Observability

- Status: accepted
- Date: 2026-08-21
- Issue: [#16](https://github.com/jaunder-org/jaunder/issues/16)

## Context

ADR-0011 made OpenTelemetry the shared tracing substrate and later addenda made
`#[macros::server]` derive stable `web.<vertical>.<ident>` server-function span
names. That gives every request an operation identity, but it does not by itself
answer the operator's harder question: why did this request reach this failing
line?

Encoding each branch as a different span name makes the trace visually obvious
but collapses query dimensions into prose. For example,
`web.registration.register.create_user_invite` says the path, but it makes
"invite-only registrations grouped by error kind" a string-matching exercise and
multiplies span names as decisions combine. The alternative is the wide-event
model ADR-0011 already chose: the span name identifies the operation, and fields
carry bounded facts about that operation.

`tracing_error::SpanTrace` adds the missing boundary-failure view. It captures
the active span stack when the error carrier is constructed, so an operator
reading a single failure event can see the path-to-here with span fields
attached. The trace exporter still emits ordinary spans when configured;
`SpanTrace` is the operator-side error snapshot, not a replacement backend.

## Decision

Jaunder records branch determinants as span fields on the narrowest span that
owns the decision, not as branch-specific span-name suffixes. A meaningful
called operation still gets its own child span when it is worth timing, reused
by multiple parents, or has determinants/failures of its own.

Server-function determinant fields are allowed through `#[macros::server]` only
as empty declarations (`field = tracing::field::Empty`). The body records
bounded values once they are known. The macro still rejects value expressions in
`fields(...)`, so `skip(email)` plus `fields(who = %email)` cannot bypass the
recordable-type gate.

`host::error::InternalError` captures a `SpanTrace` when constructed. Boundary
failures emit that trace as `error.span_trace` alongside the existing structured
operator fields and keep the `InternalError -> WebError` public projection
unchanged. Native swallowed-error reporting also emits an active span trace.
Client-swallowed reporting stays bounded-client-data-only: the accepted server
intake no longer has the browser's span stack and must not infer one.

Jaunder emits determinant fields and SpanTrace data in-process; retention policy
is collector-side. Operators that want failure-biased retention should configure
OTel tail sampling to retain errored and slow traces. This issue deliberately
does not add an in-process buffered branch-log stream.

## Consequences

Span names remain operation identities. Queries use fields such as
`registration.policy`, `registration.invite_present`, `registration.outcome`,
`db.system`, and `error.kind` instead of parsing branch names.

Span fields carry the same PII/cardinality constraints as ADR-0011: bounded
decisions and stable internal identifiers are acceptable; passwords, tokens, raw
emails, invite codes, request bodies, arbitrary source text, and whole-struct
dumps are not.

`tracing_error::ErrorLayer` is part of normal host telemetry setup, and host
telemetry maintains formatted span fields for SpanTrace capture when values are
recorded after span creation.

The first migrated branch path is registration: `web.registration.register`
records registration determinants, while the invite-backed atomic create keeps a
non-branch-named child operation span for timing.
