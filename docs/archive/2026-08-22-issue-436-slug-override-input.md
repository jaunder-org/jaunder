# Issue #436 — Shared slug-override input

## Outcome

The full compose page and edit page keep the same slug-override user behavior
while routing the draft-only slug input through one shared renderer. The visible
field, its validation timing, and submit gating remain unchanged; only the
duplicated Leptos binding shape gets one owner.

## Load-bearing decisions

- The shared renderer is a slug-override-specific shared form component, not a
  broad `Field<T>` abstraction or a replacement for `ValidatedInput<T>`.
- The component owns only the input binding and touched-gated inline validation
  message for `Field<Slug>`, putting the post forms on a shared ADR-0065
  renderer instead of page-local bespoke binding.
- Draft-only visibility remains owned by the publication/options branch, because
  scheduled and live Posts must not render a slug override control.
- Submit-button disabling remains owned by each form's existing submit gate; the
  buttons differ and are not part of the extraction.
- ADR-0065 stays intact: client-side validation continues to call the shared
  `Field<Slug>` rule, with validation recomputed on input and errors shown after
  blur/touch.
- The field contract remains `name="slug_override"`, existing ids/classes/layout
  are preserved, and e2e selectors targeting `input[name="slug_override"]` keep
  resolving.

## Acceptance

- The full composer and editor reach the draft slug-override control through the
  shared helper rather than carrying their own input/error binding block.
- The `on:input`, `on:blur`, `prop:value`, and touched-gated error markup for
  `Field<Slug>` exists in one renderer only.
- Drafts still show the slug-override input; scheduled and live Posts still omit
  it.
- Invalid non-empty slug overrides still disable save/publish actions and show
  the validation error only after blur.
- `cargo xtask check` passes locally, and the PR's e2e/validate gates cover the
  unchanged browser flow before merge.

## Boundaries

- No change to `Field`, `field_error`, `ValidatedInput`, or the ADR-0065
  validation mechanics.
- No change to server wire types, `PostInputs`, slug parsing, schedule controls,
  tag input, or publication-state behavior.
- No styling redesign; keep the existing options-aside layout and classes.
