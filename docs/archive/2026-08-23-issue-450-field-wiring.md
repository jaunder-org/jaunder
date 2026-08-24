# Issue #450 — Shared Field wiring primitives

## Outcome

ADR-0065 direct-bind fields stop copying the same value/error/touch wiring into
each bespoke layout. Callers keep owning their form chrome, buttons, and submit
policy while using one shared form seam for a validated bare input and its
touched-gated inline error.

## Load-bearing decisions

- The seam is wiring primitives, not a second form shape: add shared wasm-only
  `forms` renderers for a bare validated `<input>` and for the touched-gated
  error view.
- Do not use a macro. The current call sites vary in label structure, flex/card
  layout, error placement, and server-action style; normal components keep the
  interface visible to rustc/leptosfmt and avoid macro-only syntax.
- Do not make `ValidatedInput<T>` absorb every direct-bind use case. Its
  labelled chrome remains the default for ordinary fields, but audiences,
  backup, sessions, and post options need caller-owned surrounding markup.
- The bare input primitive owns the repeated `prop:value`, `on:input` validation
  update, and `on:blur` touch behavior for `Field<T>`. It may expose ordinary
  input attributes already needed by adopters (`type`, `name`, `id`, `class`,
  `placeholder`, `autocomplete`) without taking over layout.
- The error primitive owns only the touched gate and error rendering. Its caller
  chooses placement and element/class shape so existing markup stays legal and
  visually unchanged.
- `ValidatedInput<T>` and the new direct-bind adopters use the same wiring path
  internally, preserving ADR-0065's single validation source (`Field<T>` /
  `field_error::<T>`). ADR-0065 must be amended from its old "macro" consequence
  to the chosen component-primitive seam.
- Submit disabling remains owned by each form's existing submit gate or button;
  the extraction must not change disable-until-valid behavior.

## Acceptance

- Current direct-bind `Field<T>` sites in audiences, backup destination, post
  slug override, and app-password session label use the shared wiring primitives
  instead of hand-written value/error/touch blocks.
- `ValidatedInput<T>` reuses the same bare input and error primitives rather
  than keeping a second copy of the wiring, while preserving its existing props
  and behavior: `type`, `autocomplete`, help/`aria-describedby`, wrapper/input
  classes, and username-style `transform`.
- Existing names, ids, placeholders, classes, label structure, error placement,
  and button-disabled behavior are preserved at every migrated direct-bind call
  site.
- Focused browser-flow proof covers at least one migrated direct-bind create
  path and one with-chrome `ValidatedInput` path, demonstrating unchanged
  disable-until-valid and touched-gated inline messages.
- ADR-0065's direct-bind and consequences text records the chosen
  wiring-primitive component seam: direct-bind call sites no longer hand-write
  the value/error/touch block, and backup is no longer described as blocked by
  `ValidatedInput`'s missing placeholder/classes.
- `cargo xtask check` passes locally.

## Boundaries

- No change to `Field<T>` validation semantics, `field_error`, optional-field
  behavior, parsed values, or server wire types.
- No redesign of `Labelled`, no answer to ADR-0117's open question about erased
  validity signals, and no generic component-with-children migration.
- No conversion of textarea widgets, schedule controls, tag inputs, or unrelated
  signal-bound inputs.
- No styling redesign and no new validation rules.
