# Issue #450 — Shared Field Wiring Primitives Implementation Outline

> Execute with `jaunder-iterate`; delegate individual slices through
> `jaunder-dispatch` only if useful. This outline exists because the approved
> spec changes the shared `forms` interface and amends ADR-0065.

## Scope

In:

- Add shared wasm-only `forms` primitives for validated bare input wiring and
  touched-gated error rendering.
- Migrate the current direct-bind adopters: audiences create/rename, backup
  destination, post slug override, and app-password session label.
- Make `ValidatedInput<T>` reuse the same primitives without changing its public
  props or labelled chrome behavior.
- Amend ADR-0065's direct-bind/consequence text to describe the component
  primitive seam.
- Add/adjust focused browser-flow coverage for one migrated direct-bind path and
  one `ValidatedInput<T>` path.

Out:

- `Field<T>` semantics, optional-field parsing, server wire types, submit-gate
  policy, `ValidatedTextarea<T>`, `Labelled` interface redesign, schedule/tag
  controls, styling redesign, and new validation rules.

## Task outline

- [x] Task 1: introduce shared input/error primitives in `web/src/forms`
  - Contract: `ValidatedInput<T>` keeps its current caller-facing props and
    routes its inner `<input>` through the new bare-input primitive. The error
    primitive takes the erased validity/touched signals needed by ADR-0117 and a
    caller-selected render shape/class.
  - Verification: existing with-chrome call sites still compile; focused browser
    proof later exercises a `ValidatedInput<T>` path with unchanged
    disable-until-valid and touched-gated error behavior.

- [x] Task 2: migrate current direct-bind `Field<T>` input sites
  - Contract: audiences, backup, posts, and sessions keep existing DOM
    contracts: names, ids, placeholders, classes, labels, error placement, and
    button disabling. Only the repeated value/error/touch wiring moves behind
    the shared primitives.
  - Verification: focused browser proof exercises at least one migrated
    direct-bind create path with invalid input, blur, inline error, and disabled
    submit until valid.

- [x] Task 3: update ADR-0065 to the chosen seam
  - Contract: the rendering bullet no longer defines direct-bind as hand-written
    `prop:value`/`on:input`/`on:blur`; the consequences no longer promise a
    macro specifically. Backup is no longer described as blocked by
    `ValidatedInput`'s missing placeholder/classes.
  - Verification: ADR text is coherent with the migrated code and the spec; the
    normal ADR/doc gates run through `cargo xtask check`.

- [x] Task 4: run the implementation gates and commit
  - Contract: each completed slice is committed through `jaunder-commit`; tick
    the relevant outline checkbox before the commit gate.
  - Verification: focused browser-flow proof from Tasks 1/2 plus
    `devtool run -- cargo xtask check` before the final implementation boundary.

## Risk checks

- `ValidatedInput<T>` must preserve `input_type`, `autocomplete`,
  help/`aria-describedby`, field/input classes, and `transform`.
- Direct-bind migrations must not move errors into invalid HTML or change where
  existing errors appear relative to forms/buttons.
- Submit disabling remains caller-owned; the primitives must not dispatch or
  decide validity policy beyond updating/reading `Field<T>`.
- ADR-0117's erased-signal `Labelled` interface stays unchanged.
- No lint suppression may be added without explicit approval.
