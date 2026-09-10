# Audience management layout and actions

Issues: #1447, #1448

## Outcome

The Audiences screen uses the established card-body spacing, clears a
successfully submitted audience name, and presents row actions as an edit
disclosure instead of an always-visible form and destructive button. Subscriber
refresh remains available but explains itself on hover.

## Load-bearing decisions

- Treat #1447 and #1448 as one audience-management UI change; the delivered PR
  closes both issues.
- Use the existing generic card-body spacing for the create form and every
  audience-list state. Do not introduce audience-specific padding values or
  another card-content convention.
- Clear the create-audience field only after a confirmed creation. Preserve the
  entered name after validation failure, server failure, or a
  commit-indeterminate result.
- An audience row rests in browse mode: its name and a compact `Edit` button are
  visible; rename and delete controls are not.
- `Edit` opens an inline disclosure containing the current name, `Save`,
  `Cancel`, and `Delete` controls.
- `Cancel` discards the draft name and returns the row to browse mode without a
  mutation.
- A confirmed rename returns the row to browse mode with the new name. A failed
  or commit-indeterminate rename keeps the disclosure and entered name available
  with its existing feedback.
- `Delete` remains destructive, is available only inside the edit disclosure,
  and requires the repository-standard confirmation before submission.
- The subscriber refresh remains the icon button required by #347. Keep its
  accessible name and add a native hover tooltip that names the action; do not
  replace manual refresh with polling, focus refresh, or server push.
- Preserve the existing keyed-list, sticky-roster, error-surfacing, and
  per-audience membership-refetch behavior.

## Acceptance

- The create form controls are inset from the card edges and aligned with the
  card heading through the shared card-body spacing.
- The audience list, loading message, empty message, roster/list errors, and
  populated rows use the same inset instead of touching the card border.
- The layout remains usable at the existing narrow-page breakpoint: controls
  wrap or shrink without horizontal overflow.
- After a confirmed create, the new audience appears and the create field is
  empty and pristine.
- After a create error or commit-indeterminate result, the entered audience name
  remains present and the existing feedback remains visible.
- A populated row initially shows its audience name and `Edit`, with no rename
  input or `Delete` control visible.
- Activating `Edit` reveals the current name plus `Save`, `Cancel`, and
  `Delete`; `Cancel` restores browse mode without changing the audience.
- A confirmed rename updates the visible name and closes the disclosure. Rename
  failure or indeterminate completion leaves the disclosure and draft name
  intact.
- Activating `Delete` opens a confirmation. Cancelling preserves the audience.
  Accepting and receiving a confirmed result removes the row; a failed or
  commit-indeterminate result keeps the disclosure open with its existing
  feedback visible.
- The refresh icon has accessible name and tooltip text `Refresh subscribers`;
  clicking it still refreshes the shared subscriber roster without a page
  reload.
- A failed subscriber refresh keeps the last resolved roster and checklists
  mounted, shows the existing page-level roster error, and does not replace them
  with an empty or loading state.
- Existing audience CRUD and membership flows continue to work on SQLite and
  PostgreSQL.

## Boundaries

- No changes to audience membership, authorization, storage, wire APIs, or
  domain vocabulary.
- No automatic roster refresh, polling, focus listeners, server push, or new
  loading animation.
- No redesign of post-editor audience selection or controls outside the
  Audiences screen.
- No new shared UI primitive or audience-specific spacing system.
