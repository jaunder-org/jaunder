# Issue #15 — Scheduled Post management UI

## Outcome

Authors get a dedicated web surface for managing Scheduled Posts: a
scheduled-only list reachable from authenticated navigation, with each row
making the scheduled time and edit path obvious. Existing edit-page controls
remain the place to reschedule a Scheduled Post or pull it back to draft.

## Load-bearing decisions

- A Scheduled Post remains a Post whose current `published_at` is non-null and
  greater than `now`; no new lifecycle state, table, or persisted schedule flag
  is introduced.
- The dedicated surface is separate from `/drafts`. `/drafts` may continue to
  show all unpublished Posts, but scheduled management must not require scanning
  the mixed Drafts list.
- The scheduled management entry point is authenticated-owner UI only. Direct
  unauthenticated requests must follow the existing authenticated-page
  denial/sign-in behavior before any scheduled-row data is listed. Public reads,
  Syndication Feeds, AtomPub Collections, and guest permalink behavior keep the
  ADR-0027 time gate unchanged.
- Row actions link to the existing Post editor rather than duplicating inline
  reschedule/pullback controls in the list. The edit page is already the single
  mutation surface for schedule changes, clear-schedule, and pull-back-to-draft.
- The list shows only currently scheduled Posts; true drafts and already-live
  Posts are excluded. A Post that becomes live by time passing leaves the
  scheduled list on refresh/revalidation through the same derived
  `published_at > now` rule.
- Scheduled rows are ordered by scheduled go-live time, soonest first, then by
  `post_id` ascending. This makes the surface a management queue rather than
  another created-at draft chronology.
- The empty state names the domain precisely: no Scheduled Posts, with a path
  back to composing a Post.
- Slug behavior follows ADR-0130 unchanged: scheduled/live editors hide the slug
  control; after a pullback creates a draft, later draft editing may expose slug
  editing.

## Acceptance

- Authenticated navigation exposes a scheduled-post management destination
  distinct from Drafts.
- Visiting that destination shows scheduled Posts only, with each row displaying
  the Post title or fallback label, the scheduled go-live time, and a link/path
  into the existing editor.
- The scheduled list excludes true drafts, live Posts, Deleted Posts, and Posts
  owned by other users.
- Scheduled Posts are ordered by scheduled go-live time ascending, then by
  `post_id` ascending when two Posts share a timestamp.
- From the scheduled list, an author can open a Scheduled Post, reschedule it in
  the existing editor, save, and see the updated time reflected back on the
  scheduled surface.
- From the scheduled list, an author can open a Scheduled Post, clear its
  schedule or save it as draft in the existing editor, and see it disappear from
  the scheduled surface while remaining available from Drafts.
- Existing scheduled-publishing visibility remains intact: scheduled Posts stay
  hidden from public post pages and Syndication Feeds until due, while remaining
  author-visible.
- The change has focused web/server coverage for the scheduled-only listing
  contract and an end-to-end browser flow covering scheduled list → editor →
  reschedule/pullback → list update.

## Boundaries

- No AtomPub or Emacs client change is part of this issue.
- No new scheduler, feed worker behavior, or public visibility policy is part of
  this issue.
- No inline bulk actions, calendar view, notifications, or recurrence support is
  part of this issue.
- No redesign of the mixed Drafts surface is required beyond preserving its
  existing unpublished-post behavior.
