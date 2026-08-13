# ADR-0129: Cohesive request aggregates at server-function boundaries

- Status: accepted
- Date: 2026-08-12
- Issue: [#417](https://github.com/jaunder-org/jaunder/issues/417)

## Context

ADR-0065 validates individual `#[server]` arguments with domain newtypes on the
client and wire. A cohesive form submission can nevertheless remain a positional
list of same-primitive arguments, and `ActionForm` turns already validated
client fields back into browser strings before Leptos decodes them again. That
leaves request-level transposition possible and retains a generic decode-failure
path in the normal browser flow.

Leptos already serializes a nested aggregate as one generated server-function
argument. In Jaunder's pure-CSR architecture (ADR-0040), `ActionForm`'s
progressive-enhancement benefit does not apply, so retaining string harvesting
has no compensating user-facing capability.

## Decision

A server function whose caller-supplied values form one cohesive request takes
exactly one typed request aggregate. The aggregate contains all deliberately
supplied values and excludes ambient context such as authentication, headers,
cookies, and injected services. Names describe meaning: operation-specific
requests use `*Request`; operations may share a domain-shaped aggregate only
when both fields and meaning coincide.

The wasm form constructs the generated server-action input from parsed typed
fields and dispatches it through `ServerAction`. It remains a native `<form>`
with submit handling, default prevention, Enter-key behavior, validation, and
pending state. Inbound secrets are parsed directly into their proffered wire
types.

The `proffered-secret` guard therefore admits an inbound-secret field only on a
`*Request` type actually named by a `#[macros::server]` parameter, and admits
temporary staging only as `Field<Proffered*>`, its validated input renderer, or
an explicit `parse::<Proffered*>()` in wasm-only `web/src/*/component.rs`. It
continues to reject response fields, return positions, server-side helpers, and
every other occurrence. This amends ADR-0063's narrower direct-parameter-only
enforcement while preserving its inbound-only guarantee.

This is a semantic rule, not a blanket arity rule. Single-value commands and
genuinely independent arguments remain direct parameters. No static gate tries
to infer cohesion.

## Consequences

- The aggregate makes request-level transposition a compile error and makes an
  invalid browser dispatch unconstructable after field parsing.
- The server-function wire gains nested parameter names. This is acceptable:
  `/api/*` is the private CSR protocol under ADR-0082, and endpoint paths do not
  change.
- Components pay explicit submit-assembly code and stop using `ActionForm` for
  cohesive requests. Native form behavior must be retained and tested.
- Login is the tracer bullet. Its nested transport and UI behavior are proved
  across backend integration tests, targeted Playwright, and `cargo xtask check`
  before the convention is applied to the other current cohesive forms.
- Arity alone remains insufficient evidence for an aggregate; consistency does
  not justify one around a single value or unrelated arguments.
