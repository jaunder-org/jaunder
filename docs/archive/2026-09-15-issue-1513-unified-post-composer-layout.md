# Unified Post composer layout

## Outcome

Every web surface for creating or editing a Post presents the same
space-efficient composer structure. The body remains the primary writing surface
while a single controls rail uses horizontal space when available and moves
below the body when the composer itself becomes narrow.

## Load-bearing decisions

- `/app` and `/posts/new` expose the same complete field set, grouping, and
  proportions. The Post editing surface shares that presentation while
  preserving controls that depend on the loaded Post's publication state.
- The routes retain their surrounding page context and state-appropriate action
  labels; those are not separate composer layouts.
- The body side contains the Body field followed by the action row. Save draft
  and the primary publication action fill that row equally.
- The controls rail contains, when applicable, Media, Summary, Tags, Format,
  Slug, optional publication time, and Audience.
- Scheduled and live Posts keep Slug hidden. A Draft exposes Slug, as required
  by the existing current-publication-state slug-freeze policy.
- Format choices fill their row equally and retain their segmented-choice
  treatment.
- Above 800 CSS pixels of available composer-container width, the controls rail
  is 320 pixels wide and the body receives the remaining width, never less than
  480 pixels. At 800 pixels or below, the complete rail stacks beneath the body
  side. This transition depends on container width, not viewport width.
- Source and keyboard order remain Body, actions, then the controls rail; the
  responsive layout does not create a different visual and focus order.
- When the existing Media Upload Capability permits uploads, Media is presented
  as a field headed `Media`, with a compact `+` upload control immediately to
  the heading's right. When uploads are disabled, upload discovery—including
  this field and control—remains hidden.
- Each Media uploaded during the current composer session appears as a compact
  row with a thumbnail, filename, copy-URL action, and `−` dismiss action.
- Dismissing an uploaded Media row removes it only from the composer's temporary
  list. It does not delete the user's persistent Media Record.
- The uploaded URL is not presented as though it were an editable form field.
- A committed publication time is shown in one compact row. For a new Post or
  Draft, balanced edit and clear icons change or remove the optional time;
  clearing leaves the Post a Draft and restores the ordinary Publish action.
- A Scheduled or live Post exposes the edit-time icon but no clear-time icon.
  Its existing Unpublish action remains the only transition back to Draft.
- Every icon-only control has a tooltip and an accessible name.
- Existing publication semantics remain authoritative: the primary action may
  read Publish, Schedule, Save, or Unpublish as appropriate to the Post state.

## Acceptance

- The complete field set and grouping are visibly identical on `/app` and
  `/posts/new`; editing uses the same structure with loaded values and the
  state-conditioned Slug, publication-time, Save, and Unpublish controls above.
- At a 960-pixel composer-container fixture, Body and its filled action row
  occupy a 640-pixel primary column and the controls occupy a 320-pixel rail.
- At an 800-pixel fixture and a narrower mobile fixture, the complete controls
  rail appears below Body and its actions without clipping, overlap, or
  visual/focus-order drift.
- Summary, Tags, Media (when enabled), Format, applicable Slug and publication
  time, and Audience remain usable in wide and stacked layouts.
- Multiple successful uploads can be represented in the current composer
  session; each row exposes its thumbnail/name, copy action, and non-destructive
  dismiss action, while the heading-level upload control remains available.
- With Media Upload Capability disabled, no upload field or affordance appears
  on any composer surface.
- New/Draft clear-time and Scheduled/live Unpublish behaviors are independently
  exercised and retain their distinct state transitions.
- Filled tags, scheduled publication, non-default Audience, and uploaded Media
  remain visually coherent without pushing labels or neighboring fields out of
  alignment.
- Desktop and narrow browser coverage proves the shared creation layout, the
  responsive boundary, action sizing, representative populated controls, and the
  Post editing presentation.
- Existing accessibility checks continue to report no violations on the affected
  composer states.

## Boundaries

- This work does not add a Post-to-Media attachment relationship or change Media
  Record ownership, deletion, storage, quota, or upload policy.
- Dismissing Media from the composer is not a Media deletion operation.
- This work does not change Post persistence, publication scheduling, Audience,
  slug, tag, or authoring-format semantics.
- The dedicated `/posts/new` route remains a focused writing workspace even
  though `/app` exposes the same creation capabilities.
- No server endpoint, storage schema, protocol surface, custom-theme contract,
  or rendering architecture changes are in scope.
