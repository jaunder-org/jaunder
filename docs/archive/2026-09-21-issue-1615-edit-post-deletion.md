# Delete a Post from its edit form

Issue: [#1615](https://github.com/jaunder-org/jaunder/issues/1615)

## Outcome

An authenticated Post owner can delete the Post directly from
`/posts/:post_id/edit`, without first returning to Drafts, Scheduled Posts,
Home, or the Post permalink. The operation uses Jaunder's existing soft-delete
lifecycle and makes no new erasure promise.

## Load-bearing decisions

- The edit form exposes a visually separated danger action at the bottom of its
  control rail. Delete does not sit beside or resemble the ordinary save action.
- The action is available for Draft, Scheduled, and Published Posts loaded into
  the editor.
- Activating Delete asks for the existing confirmation, “Delete this post?”. No
  typed confirmation or additional warning for unsaved edits is required.
- Confirming dispatches the existing authenticated Post deletion operation. It
  does not introduce a separate deletion policy or endpoint.
- Navigation occurs only after the server confirms deletion. The destination is
  determined by the publication state loaded into the editor, classified at the
  server-provided fetch instant; elapsed time while editing does not reclassify
  a loaded Scheduled Post as Published:
  - a Published Post goes to Home (`/app`);
  - a Draft goes to Drafts (`/drafts`);
  - a Scheduled Post goes to Scheduled Posts (`/scheduled`).
- A failed or commit-indeterminate deletion leaves the user on the edit route,
  preserves the current form contents, and presents the typed failure or
  indeterminate guidance. It must not imply that deletion failed when the commit
  outcome is unknown.
- While deletion is in flight, the edit surface prevents a competing save or
  second deletion dispatch. Ordinary editing can resume after a settled
  non-confirmed outcome.

## Acceptance

- On `/posts/:post_id/edit`, the owner can discover a clearly dangerous Delete
  action distinct from Save and publication controls.
- Canceling the confirmation performs no mutation or navigation and preserves
  all current form values.
- Confirming deletion of a Draft removes it from active Post surfaces and
  navigates within the app to `/drafts`.
- Confirming deletion of a Scheduled Post removes it from active Post surfaces
  and navigates within the app to `/scheduled`.
- Confirming deletion of a Published Post removes it from active Post surfaces
  and navigates within the app to `/app`.
- A rejected deletion displays the server error on the edit page without losing
  the user's current form values.
- A commit-indeterminate deletion displays refresh guidance on the edit page and
  does not navigate.
- While deletion is pending, repeated activation produces exactly one deletion
  dispatch and Save cannot dispatch. After a rejected or commit-indeterminate
  settlement, the controls become available again.
- Automated browser coverage exercises the edit-form deletion flow and proves
  the confirmed destination without adding a second document boot.
- The Post authoring lifecycle flow document distinguishes the existing
  permalink deletion outcome from the edit form's lifecycle-specific CSR
  navigation.
- Comparable before/after screenshots show the authenticated editor control rail
  for the same Published Post and deterministic content at 1440 × 900 using the
  built-in default theme. These are transient review evidence, not a new visual
  snapshot baseline.

## Boundaries

- This work does not change soft deletion, retention, Post Revision, permalink
  reuse, media-reference, authorization, or feed-regeneration policy.
- It does not add restore, purge, bulk deletion, undo, or deletion from any new
  surface other than the existing edit route.
- It does not redesign the existing permalink or Drafts deletion controls.
- It does not add a leave-page warning for unsaved edits.
