# ADR-0154: Sink-Specific Telemetry Interfaces

- Status: accepted
- Date: 2026-08-25
- Issue: [#1138](https://github.com/jaunder-org/jaunder/issues/1138)

## Context

[ADR-0011](0011-unified-observability.md) admitted server-function parameter
values to tracing on four grounds: intrinsically bounded values, operator
configuration, already-public permalink values, and `Username`. The
`server-fn-tracing` gate stored that decision as `RECORDABLE_TYPES`, reducing
Rust syntax to a final type name and comparing strings. Types therefore owned
neither their admission nor the exact representation sent to tracing.

The wire-argument error gate had the same ownership problem at another sink. It
allowed selected third-party `Display` implementations through a central list of
crate names and versions, even though generic error text does not say whether it
is safe for telemetry, useful only to the submitting user, or safe nowhere. The
Croner parse message is useful operator feedback; email parser detail and raw
submitted input are not telemetry.

The lists were fail-closed, but both inferred sink policy outside the type that
owns the value. A dependency version or a syntactically similar type name was a
proxy for the interface the caller actually needed.

## Decision

Tracing admission is represented by `common::trace_field::TraceField`. Its
associated `Value<'a>: Debug` names the exact projected representation. The
approved primitives project by value, approved domain values by borrow, and only
`Option<T>` and `&T` recurse. There is no blanket implementation for strings,
`Debug`, or `Display`.

`#[macros::server]` always hides original parameters from `tracing::instrument`
with generated `skip_all`. It then emits one field per named, non-skipped
parameter through `TraceField::trace_value`. Trait resolution is the
default-deny assertion: a newly unskipped type without an implementation fails
compilation. Source `skip(name)` and `skip_all` remain explicit author intent;
pattern-bound parameters still require `skip_all`. The `server-fn-tracing`
source gate retains attribute grammar, skip-name, pattern, and
declaration-only-field checks, but performs no type-name classification.

The four admission grounds and exclusions from ADR-0011 remain unchanged.
Secrets, email, arbitrary content, request aggregates, and the existing skipped
parameters gain no tracing interface.
[ADR-0147](0147-decision-path-observability.md) still permits decision-path
fields only as `tracing::field::Empty` declarations recorded later by the span
owner. Rejecting value expressions plus the generated parameter `skip_all`
prevents those declarations from bypassing `TraceField`.

Parse errors expose owned sink methods. Stable user feedback is returned by
`user_message`; bounded telemetry classification is returned by
`telemetry_code`. Third-party detail intentionally retained for a user is stored
in `common::UserFacingMessage`, whose `Debug` is redacted and which has no
`TraceField` or `Display` implementation. Every conversion from external
`Display` requires an immediately preceding, non-empty
`server-fn-wire-arg-error:allow` source marker. The static gate derives that
marker census and rejects unmarked, stale, shared, trailing, bare, and orphan
markers instead of storing dependency/version entries.

Server-function decode telemetry remains fixed and source-free:
`error.public = "invalid request arguments"`, `stage = "decode"`, with no raw
submitted value or user-facing third-party detail.

## Consequences

- Admission changes are ordinary trait implementations with compiler-checked
  representation types, not edits to a name allowlist.
- The procedural macro cannot silently omit a non-implementer; the author must
  implement the reviewed interface or explicitly skip the parameter.
- `InvalidEmail` loses third-party diagnostic detail and exposes stable user and
  telemetry strings. `InvalidBackupSchedule` preserves detailed Croner feedback
  only for the submitting operator and exposes a stable telemetry code.
- External formatter exceptions become narrow, reasoned source doors whose
  liveness is derived from code. Cargo.lock versions no longer encode sink
  safety.
- The source gates continue to enforce syntax and sanitization that trait
  resolution cannot express.
