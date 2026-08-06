# ADR-0065: Typed `#[server]` wire args with client-side pre-validation via the shared newtype

- Status: accepted
- Date: 2026-07-12
- Issue: [#414](https://github.com/jaunder-org/jaunder/issues/414)
- Note: amended 2026-07-30 (#568) — the coverage boundary is re-pointed at
  ADR-0070 (the widgets are wasm-only and never host-compile), and
  `<ValidatedTextarea<T>>` joins `<ValidatedInput<T>>` as a default renderer
- Note: amended 2026-08-05 (#822) — the secret exception is restated around the
  inbound-twin split (a secret **is** a typed wire arg; it was never true that
  its arg "stays `String`"), and the decode-stage consequence now records that
  an arg-decode failure emits boundary telemetry rather than vanishing

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
- **Rendering: component or direct bind.** _Amended by #568._ The
  `<ValidatedInput<T>>` and `<ValidatedTextarea<T>>` components are the default
  renderers for a standard labelled field (and for `ActionForm` name/value
  submission); both share one private `Labelled` chrome. A form with a bespoke
  layout or a programmatic `.dispatch(...)` may bind the same `Field<T>`
  **directly** to its own `<input>` — `prop:value=field.value`, an `on:input`
  that sets `field.error = field.error_for(&v)`, `on:blur` → `field.touch()`,
  and the touched-gated inline error — keeping the single validation source
  without the component's fixed markup. The canonical direct-bind example is now
  the backup destination field (`web/src/backup/component.rs`), which needs a
  placeholder and bespoke classes the shared components cannot yet express
  (#450); the post compose/edit forms, which this bullet previously cited, moved
  onto the shared components in #568.
- **Defense-in-depth.** The typed-arg `Deserialize` still validates server-side;
  because legitimate clients pre-validate, the generic-`ServerFunction`-error
  path is only reachable by a malformed/malicious request.
- **Secret exception — the twin split, not a `String` arg.** _Amended by #822._
  A secret's **domain** type (`Password`) has no serde bridge (ADR-0063), so it
  cannot itself cross the wire — but its **inbound twin** (`ProfferedPassword`;
  ADR-0063's `Proffered`, generalized by ADR-0084) does have one and carries the
  wire role. So a secret **is** a typed wire arg like any other: validated at
  decode through the shared shape rule, and client-pre-validated through that
  same rule. What stays special is the twin split — the domain type is reachable
  only by converting inward from the twin — plus keeping the value out of
  tracing via `skip`. (This bullet previously said a secret's arg "stays
  `String`". That has been untrue since #315 typed all three password-taking
  fns; `server/tests/web/web_password_reset.rs` asserts the wire rejection
  directly.)
- **Coverage boundary (ADR-0070 — web verticals split host/wasm at the file
  level).** _Amended by #568._ This bullet previously cited ADR-0056 and claimed
  `<ValidatedInput<T>>` host-compiles as dead-but-exempt; ADR-0056 is superseded
  by ADR-0070 and the code follows ADR-0070. `field_error<T>` host-compiles and
  is coverage-measured (host-tested). The widgets — `<ValidatedInput<T>>`,
  `<ValidatedTextarea<T>>`, and the shared `Labelled` chrome — live in a
  `#[cfg(target_arch = "wasm32")]`-gated `forms/component.rs` and **never
  host-compile**, so they carry no coverage obligation and need no exemption
  marker. `Field<T>`'s methods are **signal-only** (they build no
  `Effect`/`Resource`), so — like `Invalidator::{new, notify, track}` — they are
  **host-tested under an `Owner`**, coverage-measured, _not_
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
  button gates the browser) now fails _before_ the fn body, surfacing as a
  generic transport/decode error rather than the controlled public message.
  Accepted: that's the defense-in-depth path, not the user path.

  _Amended by #822._ This applies to **every** typed wire arg, secrets included
  — there is no `String`-arg carve-out, so nothing "still parses in the body".
  The decode path also used to skip the body's error-boundary telemetry
  entirely, leaving a malformed request with no trace at all; `web::error`'s
  `FromServerFnError` impl now emits the standard boundary event and error
  metric for arg-decode failures (`Validation`/`Client`, with a `stage = decode`
  context entry). The rejection _metrics_ named below — `metrics::login`,
  `metrics::registration`, `metrics::password_reset` — sit downstream of the
  parse and never covered this path in any version. One consequence remains: at
  decode time the `web.<vertical>.<ident>` span has not been entered, so the
  failing endpoint is identified by the enclosing request span's `uri`, not by
  span name.

- What this rules out: re-implementing a newtype's validation in the client;
  typing a wire arg **without** client pre-validation (which would expose the
  generic-error UX); and treating "localized" as i18n (out of scope).
