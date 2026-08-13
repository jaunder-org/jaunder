# Issue #872: shared full-compose layout classes

## Context

The full Post composer and Post editor render the same two-column shell: a body
column followed by an options aside. The shells currently use two names for each
property-identical layout rule:

- `.j-compose-grid` and `.j-edit-form-grid`
- `.j-compose-aside` and `.j-edit-form-aside`

The duplicate names let the two surfaces diverge despite sharing the same
layout. Issue #863 already consolidated their options markup into
`ComposeOptions`; this issue removes the remaining duplicate wrapper names.

## Decisions

1. `j-compose-grid` and `j-compose-aside` are the canonical shared names. This
   matches the existing shared `ComposeState`, `ComposeOptions`, and
   `ComposerFields` vocabulary and avoids introducing another class family.
2. `FullComposer` keeps its existing class names. `EditPostForm` changes only
   its outer grid and aside classes to the canonical names.
3. The canonical CSS rules remain in the compose section of
   `server/assets/jaunder.css`. The section comment states that the grid and
   aside are shared with the Post editor. The duplicate edit-form rules are
   deleted; all remaining rules in the edit section stay in place and unchanged.
4. The two children of each grid remain ordered body first, aside second. Every
   declaration in the canonical grid and aside rules remains unchanged.
5. CSS class tokens remain implementation details. No Playwright assertion is
   added for the names; existing Post scenarios provide behavioral browser
   coverage, while source inspection proves the clean cutover.
6. Historical documents under `docs/archive/` remain unchanged. No domain
   glossary or ADR update is needed: this consolidates existing presentation
   names without changing Post behavior, domain language, or architecture.

## Acceptance criteria

- **AC1 — One grid name.** `FullComposer` and `EditPostForm` both render their
  outer shell with `class="j-compose-grid"`. No executable source or live
  stylesheet references `j-edit-form-grid`.
- **AC2 — One aside name.** Both surfaces render their options/media/action
  column as `<aside class="j-compose-aside">`. No executable source or live
  stylesheet references `j-edit-form-aside`.
- **AC3 — One rule per shared element.** The live stylesheet contains exactly
  one `.j-compose-grid` rule and one `.j-compose-aside` rule. The duplicate
  edit-form grid and aside rules are absent.
- **AC4 — Grid contract preserved.** The canonical grid rule remains `flex: 1`,
  `display: grid`, `grid-template-columns: 1fr 320px`, `overflow: hidden`, and
  `min-height: 0`. In both renderers the body remains the first child and the
  aside remains the second.
- **AC5 — Aside contract preserved.** The canonical aside rule retains its left
  border, `24px 20px` padding, background, column flex layout, `18px` gap, and
  scrolling. Each surface's existing aside children and action placement remain
  unchanged.
- **AC6 — Surface-specific styling preserved.** `j-compose-body`,
  `j-edit-form-body`, and all edit-form field, textarea, and action classes and
  declarations remain unchanged.
- **AC7 — CSS organization current.** The compose-section comment identifies the
  grid and aside as shared with the Post editor; the retired grid and aside
  rules are absent from the edit section, whose other rules remain in place.
- **AC8 — Behavior unchanged.** Existing full-composer and Post-editor browser
  scenarios continue to pass without selector migration or new class-name
  assertions.

## Verification

- Search executable source and the live stylesheet for the four old/current
  class names, proving both call sites use the canonical names, the retired
  names are absent, and each canonical rule is defined once.
- Run the focused Post end-to-end suite to exercise both full-page surfaces in a
  browser.
- Run the repository gate required for the resulting commit and the full local
  validation gate before pushing.

## Out of scope

- Consolidating the distinct body, field, textarea, or action classes.
- Extracting another Leptos component or changing either renderer's structure.
- Changing columns, spacing, overflow, responsive behavior, child order, or any
  Post creation/editing behavior.
- Editing archived historical documents.
- Adding e2e assertions coupled to CSS class names.
