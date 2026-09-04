# Drafts Pagination

Issue: #799

## Outcome

An author can reach every unpublished post presented by `/drafts` without
leaving the page. The surface initially shows the newest default-sized page and
offers an incremental Load more control whenever another cursor page exists.

## Load-bearing decisions

- `/drafts` consumes the existing page envelope; it does not request an
  unbounded draft list or reconstruct cursors from rows.
- Each page preserves the endpoint's existing membership and ordering: true
  drafts and future-scheduled posts, newest first, with the storage-provided
  opaque cursor selecting the next page.
- Load more appends the next page to the currently visible rows. Existing rows
  and their Edit, Publish, Delete, and public-link affordances remain intact.
- Only one next-page request may be in flight. The control is disabled and
  reports loading while that request is pending.
- The control is present only while the server reports another page and is
  removed after the final page is appended.
- A next-page failure preserves every already-visible draft, displays the real
  error inline, and leaves the same cursor retryable. The error clears when a
  retry starts.
- An initial-page failure retains the current inline error behavior rather than
  presenting an empty draft list.
- A successful Publish or Delete mutation resets the surface through its
  existing first-page revalidation. Pagination state is not retained across a
  mutation because membership and ordering may have changed. The reset
  invalidates any in-flight next-page request; its later completion is ignored.
- Every page request uses the existing authenticated, author-scoped endpoint;
  pagination introduces no viewer, cross-author, or browser-local state.
- Reactive transition logic remains host-testable; browser-only code owns only
  rendering and asynchronous dispatch, consistent with ADR-0083.

## Acceptance

- Given more than the default page size of drafts, `/drafts` initially renders
  exactly the newest default-sized page and a visible Load more control.
- Activating Load more appends the next drafts in server order without removing
  or duplicating the first page.
- While loading, a second activation cannot dispatch another next-page request.
- After the final page arrives, every seeded draft is visible and the Load more
  control is absent.
- A failed next-page request leaves the current drafts visible, shows its error,
  and permits a retry that uses the same cursor.
- Publishing or deleting from an expanded list produces a fresh first page and
  does not retain stale appended rows.
- If Publish or Delete completes while Load more is in flight, that stale
  next-page completion cannot append rows after the fresh first page.
- An end-to-end case crosses the default page boundary and proves the second
  page is reachable through the UI.
- Host tests cover loading, append, terminal-page, failure, retry, mutation
  reset, and rejection of stale post-reset completions.

## Boundaries

- No storage query, cursor, page-envelope, ordering, or server-function wire
  changes.
- No infinite scroll, automatic prefetch, page-number navigation, or persisted
  client pagination position.
- No redesign of draft-row actions or the broader posts navigation.
- The separate Scheduled page and public timelines remain unchanged.
