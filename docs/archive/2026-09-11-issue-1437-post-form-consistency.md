# Issue #1437: Post form consistency

## Outcome

The inline composer, new-post page, and existing-post editor look and behave
like members of one post-editor family. Each surface keeps the depth appropriate
to its context while sharing the same framed core flow and action vocabulary.

## Load-bearing decisions

- The shared editor family covers `/app`, `/posts/new`, and the existing-post
  edit page.
- Every surface presents the core authoring flow in this order: body, media,
  summary, tags, then format and post actions.
- The core flow uses the same bordered, padded panel and toolbar treatment on
  all three surfaces.
- `/app` remains the compact composer. It does not gain slug, scheduling, or
  audience controls.
- The author avatar remains visible only in `/app`, where it supplies timeline
  context; it is not part of the shared core panel identity.
- New-post and edit surfaces retain their advanced controls in an adjacent
  Options panel on wide layouts.
- The advanced controls keep their current capabilities and lifecycle-specific
  contents. This work changes presentation, not publication semantics.
- At narrow widths, the Options panel stacks below the complete core editor.
  Controls remain visible; there is no new disclosure or hidden state.
- Media attachment uses the existing image/media glyph, and secondary draft-save
  actions use a conventional floppy-disk save glyph. These controls are
  icon-only, with an accessible name and a tooltip available on hover and
  keyboard focus.
- Scheduled and live editors retain a visibly primary, text-labelled Save
  action. Unpublish remains text-labelled and non-primary.
- Other publication-changing actions—including Publish and Schedule—keep visible
  text labels. The format choices also remain text-labelled.
- Each form retains one visually primary action appropriate to its current
  publication state.
- Typed validation, draft/publish scheduling behavior, and current result
  handling remain intact.
- Route-specific success feedback remains route-specific: convergence applies to
  the editor, not the workflow after a successful submission.

## Acceptance

- At desktop width, `/app`, `/posts/new`, and existing-post edit visibly share
  the same core panel, field sequence, spacing, typography, and action toolbar.
- `/app` still offers its current compact capabilities and no advanced fields.
- `/posts/new` still offers slug, publication time, and audience selection in
  the Options panel, and retains its current creation actions in the shared core
  toolbar.
- Existing-post edit still offers the controls appropriate to the post's current
  draft, scheduled, or live state.
- Media and secondary draft-save controls use the chosen glyphs consistently
  across all applicable surfaces; their accessible names and tooltips
  communicate the action without relying on the graphic.
- Primary Save, Publish, Schedule, Unpublish, Markdown, and Org remain readable
  without a tooltip.
- At and below the repository's established narrow breakpoint, the full editor
  becomes one non-overflowing column with the Options section after the core
  editor in visual, source, and keyboard-focus order.
- Keyboard users can reach every editor control, identify icon-only controls,
  submit each form, and observe validation and result feedback.
- Creating a draft, publishing immediately, scheduling, editing, saving, and
  unpublishing retain their existing externally observable outcomes.
- Visual verification exercises all three editor surfaces at desktop and narrow
  widths. Behavioral verification exercises representative create and edit
  submissions rather than asserting markup structure.

## Boundaries

- No new post-authoring capability is added to `/app`.
- No server-function, storage, publication-state, routing, or authorization
  contract changes.
- The broader label, help-text, audience explanation, and format explanation
  work reported separately in #1438 is not absorbed here.
- This work does not redesign post lists, draft-row actions, result messages, or
  the surrounding Home and post-page shells.
- No icon-only treatment for format or publication-changing actions.
