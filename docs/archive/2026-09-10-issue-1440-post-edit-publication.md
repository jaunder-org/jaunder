# Edit publication state and time for existing Posts

Issue: #1440

## Outcome

An author editing an existing Scheduled or live Post can change its publication
instant or pull it back to Draft without leaving the edit form or silently
losing other edits.

## Load-bearing decisions

- `/posts/edit` uses one publication-control contract for Scheduled and live
  Posts. Both show the persisted publication instant as browser-local date and
  time.
- An untouched displayed value round-trips the exact persisted UTC instant,
  including seconds, sub-seconds, and the selected instant in an ambiguous DST
  interval. Editing the value replaces that instant with the strictly parsed
  browser-local wall time.
- **Save** atomically persists all current Post fields and the selected
  publication instant. ADR-0027 remains authoritative: a due or past instant is
  live; a future instant is Scheduled.
- An edited publication value must be present and identify a real local instant.
  Empty, malformed, and nonexistent local times disable **Save**, dispatch no
  Save update, and show an inline error. **Unpublish** does not parse or
  validate the publication-time field, but remains gated by every other
  validation and loading condition for the fields it persists.
- **Unpublish** atomically persists the current editable body, summary, tag,
  audience, and format fields with `published_at = NULL`. It preserves the
  existing slug during that update; after the editor adopts the confirmed result
  as a Draft, a later update may change the slug under ADR-0130.
- Unpublish is reversible and uses the repository's existing immediate Unpublish
  convention: no confirmation prompt. A confirmed result stays at the editor
  URL, adopts the authoritative Draft lifecycle without another read, exposes
  the Draft controls (including the now-editable slug), and uses the existing
  **Draft saved.** summary. The retained fields are the exact values sent in the
  confirmed atomic update.
- A confirmed **Save** redirects to the returned permalink whether the resulting
  state is live or Scheduled. A failed or commit-indeterminate Save or Unpublish
  remains on the editor, retains the last confirmed lifecycle controls, and uses
  the existing error or refresh-guidance outcome.
- Scheduled Posts use the same explicit Unpublish action as live Posts. The
  scheduled-only **Clear schedule** path is removed.
- Draft editing remains unchanged: **Save draft**, immediate **Publish**, and
  optional scheduling retain their current behavior.

## Acceptance

- Editing a live Post displays its current publication instant in browser-local
  time.
- Changing that value to a valid past time and saving keeps the Post live at the
  selected instant; changing it to a valid future time makes the Post Scheduled.
- Editing a Scheduled Post displays and permits changing its existing
  publication instant through the same controls.
- Leaving the displayed value untouched preserves the exact stored UTC instant.
- Clearing or entering an invalid/nonexistent local time blocks **Save** with an
  inline error and dispatches no Save update. It does not block **Unpublish**
  when all other current form fields are valid and loaded.
- Clicking **Unpublish** on either a live or Scheduled Post sends one update
  containing the current editable fields and Draft publication intent. A
  confirmed response remains at `/posts/edit`, reports **Draft saved.**, and
  adopts the Draft controls with the slug editable.
- Browser regression coverage exercises live timestamp editing and atomic
  Unpublish through the real `/posts/edit` form. Scheduled-state coverage proves
  the shared control and transition contract.

## Boundaries

- No storage schema, publication-state derivation, scheduled worker, public
  time-gate, Syndication Feed, AtomPub, or listing changes.
- No redesign of the new-Post scheduling disclosure or compact composer.
- No timezone selector or persisted author timezone. Conversion stays at the
  existing browser-local boundary.
- No dedicated Scheduled Post management changes beyond the editor controls.
