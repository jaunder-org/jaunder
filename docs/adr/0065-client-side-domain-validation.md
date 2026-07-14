# ADR-0065: Typed `#[server]` wire args with client-side pre-validation via the shared newtype

- Status: proposed
- Date: 2026-07-12
- Issue: [#414](https://github.com/jaunder-org/jaunder/issues/414)

## Context

ADR-0063 §4 says to parse domain values into newtypes at the **outermost**
boundary and hold them inward. For the web crate the outermost boundary is the
`#[server]` function argument. But typing those args naively degrades error UX:
`WebError` implements `FromServerFnError` (`web/src/error.rs:68-74`), which maps
**any** framework/decode error — including a typed-arg `Deserialize` failure —
to the generic `WebError::ServerFunction` variant, never `WebError::Validation`.
So a malformed username submitted to `login(username: Username, …)` surfaces as
"server function error", not the controlled validation message. #14/#350 flagged
this and led us to keep wire args stringly-typed and parse on entry.

That trade — weak typing for good errors — is unnecessary. The domain newtypes
live in `common`, which is compiled for the wasm target (`web` depends on it
all-target), so the newtype's `FromStr` runs **in the browser**. We can validate
on the client with the same function the server's `Deserialize` routes through,
and only then send the typed value.

A prior local decision went the other way: the tag input re-implemented
`Tag::from_str`'s rule in `web` (`tags::is_valid_tag_slug`) to avoid pulling
`common`'s rule into the wasm bundle. That is a second source of truth that has
already drifted (#416), and the bundle rationale no longer holds (`common` is
already in the bundle).

## Decision

**Type `#[server]` wire args as domain newtypes, and require client-side
pre-validation using the same newtype `FromStr` — never a re-implemented rule.**

- **One validation source.** Client validation calls `input.parse::<T>()` on the
  `common` newtype. Re-implementing a newtype's rule in `web` is prohibited; the
  tag re-implementation is retired (#416).
- **The pattern.** A pure both-target `field_error<T>(input) -> Option<String>`
  (the newtype's `FromStr::Err` `Display` on failure) is the chokepoint; a
  wasm-only `<ValidatedInput<T>>` component drives a parent-owned `Field<T>`
  (its live value + validity), rendering an **inline, client-local** error.
  "Client-local" means shown at the field with no server round-trip — **not**
  i18n/translation.
- **Timing & gating.** Validity is computed on every input; the visible message
  is gated on a `touched` flag (set on blur). Submission is gated
  **disable-until-valid** (`prop:disabled` on the submit button), which keeps
  the pattern working inside the existing leptos `ActionForm`.
- **Optional fields.** A field whose _empty_ state is valid (e.g. an
  auto-generated `slug_override`) uses `Field::optional()` /
  `optional_prefilled(initial)`: `error_for` treats empty input as valid
  (`None`) and validates non-empty input through the newtype's `FromStr` as
  before. The wire arg is `Option<T>` and the form reads
  `field.parsed() -> Option<T>`. Because empty is valid, `is_valid()` leaves
  submit **enabled** for a blank optional field while still gating a non-empty
  invalid entry. First adopter: `slug_override` (#408).
- **Rendering: component or direct bind.** The `<ValidatedInput<T>>` component
  is the default renderer for a standard labelled field (and for `ActionForm`
  name/value submission). A form with a bespoke layout or a programmatic
  `.dispatch(...)` (e.g. the post compose/edit forms) may bind the same
  `Field<T>` **directly** to its own `<input>` — `prop:value=field.value`, an
  `on:input` that sets `field.error = field.error_for(&v)`, `on:blur` →
  `field.touch()`, and the touched-gated inline error — keeping the single
  validation source without the component's fixed markup.
- **Defense-in-depth.** The typed-arg `Deserialize` still validates server-side;
  because legitimate clients pre-validate, the generic-`ServerFunction`-error
  path is only reachable by a malformed/malicious request.
- **Secret exception.** A secret newtype (`Password`) has no serde bridge
  (ADR-0063), so it **cannot** be a typed wire arg; its arg stays `String`
  (parsed on entry) but it still gets client-side pre-validation via its
  `FromStr`.
- **Coverage boundary (ADR-0056, superseding 0055 — no `target_arch` gating).**
  `field_error<T>` host-compiles and is coverage-measured (host-tested).
  `<ValidatedInput<T>>` is a `#[component]`, host-compiling as dead-but-exempt
  (ADR-0050 syntactic exemption). `Field<T>`'s methods are **signal-only** (they
  build no `Effect`/`Resource`), so — like `Invalidator::{new, notify, track}` —
  they are **host-tested under an `Owner`**, coverage-measured, _not_
  `#[client_only]`-exempted; the marker is reserved for genuinely
  `Effect`/`Resource`-building helpers. The component's rendering/interaction is
  exercised via e2e.

## Consequences

- The #404 verticals type their `#[server]` args as newtypes and adopt
  `<ValidatedInput>` in their forms, replacing the String+parse-on-entry
  stopgap.
- The tag re-implementation is deleted (#416); no `web`-side re-statement of a
  newtype rule is permitted going forward.
- `Field::parsed()` exposes the already-parsed value as a seam toward shipping
  request-aggregate domain types across the boundary (#417) — a larger, separate
  bet.
- A boilerplate-reducing macro over `Field`/`ValidatedInput` is a sanctioned
  future ergonomic addition (no redesign required).
- Typing a `#[server]` arg moves that value's validation into arg-**decode**: a
  malformed value (only reachable by a non-browser client, since the disabled
  button gates the browser) now fails _before_ the fn body — surfacing as a
  generic transport/decode error and skipping the body's `boundary!` telemetry
  and rejection metrics. Accepted: that's the defense-in-depth path, not the
  user path. Args that stay `String` (secrets like `password`) still parse in
  the body, so their rejection telemetry is unchanged.
- What this rules out: re-implementing a newtype's validation in the client;
  typing a wire arg **without** client pre-validation (which would expose the
  generic-error UX); and treating "localized" as i18n (out of scope).
