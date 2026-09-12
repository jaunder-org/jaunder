# Issue #1438: Post composer form consistency

## Outcome

Post creation and editing present one coherent form: every field has a visible,
accessible name in a predictable position, and the Audience and Format choices
explain the decisions they control. The compact, full, and edit composers retain
their current publishing behavior while sharing the same field language.

## Load-bearing decisions

- Scope is the post composer only: compact creation, full creation, and editing.
  Other application forms are not part of this issue.
- Composer field labels appear above their controls. The existing mixed
  left-label/stacked-label geometry is removed from this surface.
- Body, Slug, Publish at, Summary, Tags, Audience, and Format have visible
  labels wherever each control appears. Placeholder text remains a hint, never
  the control's only identification.
- Tags keeps its current chip-and-input interaction. Its input retains “Add
  tag…” as an action hint beneath a visible “Tags” label.
- Audience is one semantic group. It presents the base Public, Subscribers, or
  Private choice first, followed by additive Named audience choices under “Also
  share with” when those choices exist.
- Audience help explains the relationship between the base choice and Named
  audiences. Named-audience loading, failure, populated, and empty states remain
  inside the Audience group rather than appearing as unrelated text.
- Private retains its current behavior of making Named audience choices
  unavailable. This issue clarifies that behavior; it does not change Audience
  authorization or persistence semantics.
- Format remains a compact Markdown/Org segmented choice. It gains a visible
  “Format” group label, help explaining that Format controls how Jaunder
  interprets the Body, and programmatically exposed selected state.
- Audience and Format are keyboard-operable semantic groups, not merely styled
  collections of controls. Visible state and accessible state agree.
- Existing validation, touched-error, submission gating, scheduling, Media,
  defaults, and persisted values remain authoritative and unchanged.
- This is a composer form-consistency correction, not a new domain concept,
  public Style Contract change, or architectural decision. It requires no
  CONTEXT.md or ADR change.

## Acceptance

- In compact creation, full creation, and editing, every rendered composer field
  has a visible label above its control and an accessible name matching that
  label.
- The Tags input is discoverable by the visible name “Tags”; “Add tag…” remains
  only its input hint.
- The Audience group visibly and accessibly contains the base Audience control,
  its explanation, and the Named-audience loading, failure, populated, or empty
  state.
- Selecting Private makes Named audience choices unavailable without separating
  them visually or semantically from the Audience group.
- The Format group visibly explains its effect on the Body, exposes Markdown and
  Org as one choice set, and exposes the current selection to assistive
  technology.
- Keyboard interaction can reach and change Audience and Format without a
  pointer, with visible and accessible selection state staying synchronized.
- A browser accessibility scan of the full composer reports no WCAG 2.2 A/AA
  violations.
- Existing post creation and editing scenarios still publish and save the same
  values, including schedule, Summary, Tags, Audience, Format, and Media state.
- The composer remains usable at the narrow viewport represented by the issue
  report; labels do not revert to a mixed geometry or collide with controls.

## Boundaries

- No application-wide form redesign or migration of auth, settings, profile,
  backup, or audience-management forms.
- No changes to Audience membership rules, Default Audience, Default Post
  Format, content parsing, validation rules, or database representation.
- No new post fields, Media workflow, theme API, or generalized design-system
  abstraction unless an existing shared primitive already fits the composer.
- No copywriting expansion beyond the Audience and Format explanations needed to
  make those controls understandable.
