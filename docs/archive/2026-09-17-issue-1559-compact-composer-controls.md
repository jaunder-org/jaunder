# Compact composer controls

Issue: #1559

## Outcome

The Post composer’s secondary controls occupy substantially less space while
remaining discoverable and keyboard-accessible. Tags and Summary remain directly
editable; Media and defaulted settings become compact, in-place disclosures that
show their current state while closed.

## Load-bearing decisions

- Remove the “Post details” and “Publication options” headings.
- Order the always-visible fields as Tags, then Summary.
- Summary starts at one line while empty and unfocused, and expands when focused
  or populated.
- Place Media, Format, Slug, Publish, and Audience in one disclosure grid below
  the always-visible fields.
- Media spans both grid columns. Format/Slug and Publish/Audience form two
  paired rows; very narrow containers use one column.
- Each disclosure renders its control inside its own tile, immediately beneath
  its summary. Opening one disclosure closes the previously open disclosure.
- Every disclosure is initially closed and shows its current value:
  - Media: `None`; one uploaded filename; or `<n> files`.
  - Format: the selected format.
  - Slug: the entered override, otherwise `auto`.
  - Publish: `Now` when unscheduled, otherwise the selected publication time.
  - Audience: the selected base audience.
- Media opens automatically for upload failures. Its open body retains Add,
  thumbnail, filename, Copy URL, and Dismiss controls. Upload progress is
  visible through the Media summary while an upload is active.
- Elide the explanatory Audience guidance; retain the base selector, named
  audience controls, loading/error states, and submission behavior.
- The same organization applies to every composer surface using these shared
  controls, including Home, new-Post, and Post editing.
- Disclosure state is local to the mounted composer and is not persisted across
  navigation or reload.

## Acceptance

- Home shows Tags and a compact empty Summary before the closed Media/default
  controls, with neither former section heading present.
- Closed controls expose current values and require no horizontal scrolling at
  desktop, stacked, phone, or very narrow widths.
- Activating a disclosure places its control directly beneath that disclosure;
  activating another closes the first.
- Keyboard users can toggle every disclosure and assistive technology receives
  expanded/collapsed state.
- Media’s closed summary updates after successful uploads and dismissals; Media
  opens and identifies an upload failure.
- Editing any moved control produces the same create/update request and retains
  existing validation, scheduling, audience, upload, and submit behavior.
- Focused browser coverage proves ordering, defaults, disclosure behavior,
  responsive layout, status updates, and the absence of Audience guidance.

## Boundaries

- Do not change Post persistence, Media Record semantics, upload APIs,
  publication rules, audience policy, or composer action placement.
- Do not persist disclosure preferences.
- Do not redesign the Body editor, Save draft/Publish actions, named audience
  model, or Media row actions.
