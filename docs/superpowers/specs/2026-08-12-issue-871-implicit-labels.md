# Issue #871: implicit labels for compose options

## Context

`ComposeOptions` renders the unpublished Post's slug and optional publish-time
controls for both the full composer and editor. Each currently associates its
visible label through an explicit `for`/`id` pair. This is the deferred cleanup
from issue #863's D3: Jaunder's form chrome instead nests each control inside
its label so association is structural and cannot drift.

This change applies that existing convention to these two hand-built fields. It
does not move them into the forms crate: the slug retains bespoke validation
markup and the schedule control is not a validated `Field<T>`.

## Decisions

1. Each field's existing `<label>` becomes the wrapper around its input and
   visible label text. The explicit `for` and matching input `id` disappear.
2. The slug's wrapping label takes the existing `.j-field-row` class and inline
   two-column grid style. Its visible text moves into a `.j-field-label` span,
   leaving the span and input as sibling grid items inside the label. This
   preserves the existing side-by-side presentation while making association
   implicit. The schedule field keeps its existing outer layout wrapper and
   nests its input in the `.j-field-label` label.
3. Existing layout styles, input classes and types, values, handlers,
   placeholder, validation message, conditional rendering, and field order stay
   unchanged except for the class relocation and text span required above.
4. The slug keeps its existing `name="slug_override"`. The schedule input gains
   `name="publish_at"`, providing its stable semantic selector after its id is
   removed.
5. The two scheduling end-to-end tests select the schedule input by
   `input[name="publish_at"]`; their comments describe that contract rather than
   the removed id.
6. Playwright assertions cover each implicit label/control relationship, the
   control names, and the absence of the retired ids in both the composer and
   draft editor surfaces served by `ComposeOptions`.
7. A `<label>` permits phrasing content, not a `<p>`. Render touched-gated
   validation messages nested in implicit labels as `<span class="error">`: the
   span is valid label content. Give the existing shared `.error` class
   `display:block`, preserving the paragraph's block presentation without
   repeating an inline style across renderers. Apply this both to the new slug
   wrapper and to the shared `forms::Labelled` renderer whose existing paragraph
   has the same invalid nesting. Standalone action/page errors remain
   paragraphs.
8. Migrate the existing `.j-composer-field p.error` Playwright selector to
   `.j-composer-field span.error`; its unchanged message assertion becomes the
   direct proof that shared `forms::Labelled` validation uses the valid element.
9. Update `ComposeOptions`'s rustdoc: the shared component no longer has an id
   prefix to unify.
10. No domain glossary or ADR update is needed: this applies an established UI
    convention and changes no Post behavior or architectural boundary.

## Acceptance criteria

- **AC1 — Implicit slug association.** In `ComposeOptions`, the visible `Slug`
  text span and slug input are sibling grid items inside one wrapping `<label>`
  carrying the existing `.j-field-row` class and two-column style; neither
  `for="options-slug"` nor `id="options-slug"` is emitted.
- **AC2 — Implicit schedule association.** In `ComposeOptions`, the visible
  `Publish at (optional)` text and datetime-local input share one wrapping
  `<label>`, and neither `for="options-publish-at"` nor
  `id="options-publish-at"` is emitted.
- **AC3 — Stable names.** The slug input remains named `slug_override`; the
  schedule input is named `publish_at`.
- **AC4 — Behavior preserved.** Slug input and blur still update and touch the
  same `Field<Slug>` and render its touched-gated error. Schedule input still
  updates `ComposeState::publish_at`. Published Posts still render neither
  control.
- **AC5 — Presentation preserved.** The slug retains its side-by-side grid
  layout; the schedule retains its existing layout. Existing input classes,
  input types, slug placeholder, field order, and visible label text are
  unchanged.
- **AC6 — Consumer selectors migrated.** No executable source under `web/src` or
  end-to-end selector/comment refers to either removed `options-*` id; both
  scheduling scenarios fill `input[name="publish_at"]` and retain their existing
  behavioral assertions. Historical archived documentation is exempt.
- **AC7 — DOM contract covered.** Focused Playwright assertions on both
  `/posts/new` and a draft Post's edit page prove that each named control is
  nested in the label containing its expected visible text and that neither
  retired id exists.
- **AC8 — Valid label content.** The slug's touched-gated validation error and
  `forms::Labelled`'s shared validation error render as block-displayed
  `<span class="error">` elements inside their labels. No `<p>` is nested in an
  implicit label; standalone action/page error paragraphs are unchanged. The
  existing composer validation scenario selects `.j-composer-field span.error`
  and retains its message assertion while asserting computed `display: block`.
- **AC9 — Layout contract covered.** The focused Playwright assertions prove the
  slug wrapper retains `.j-field-row` and its `grid-template-columns:auto 1fr`
  override. The schedule wrapper retains its existing top-margin layout.
- **AC10 — Documentation current.** `ComposeOptions`'s rustdoc describes the
  shared aside and field order without claiming the removed id prefix.

## Verification

- Run the focused end-to-end Post suite while iterating; its assertions directly
  exercise AC1–AC3, AC6–AC9 on both `ComposeOptions` surfaces. Existing users of
  `forms::Labelled` exercise the shared renderer's AC8 markup; the composer
  validation scenario explicitly asserts its `span.error` element and message.
- Run the repository's full local validation gate before pushing.

## Out of scope

- Replacing either hand-built field with a forms-crate component.
- Changing slug or scheduling semantics, validation, field order, or styling.
- Refactoring other explicit label/control associations.
- Changing standalone action/page error paragraphs.
