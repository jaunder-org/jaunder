# Issue #1625: Default Audience and bulk Post operations

## Outcome

Authors can choose a safe default audience for newly created Posts and repair
existing Posts efficiently from a compact Manage Posts screen. Operators set the
site fallback, each User may override it, and bulk Change Audience and Delete
operations apply predictably across an explicitly confirmed Post selection.

## Load-bearing decisions

- Jaunder distinguishes **Site Default Audience**, **User Default Audience**,
  and **Effective Default Audience** as defined in `CONTEXT.md` and
  `docs/adr/drafts/hierarchical-default-audience.md`.
- The operator controls the Site Default Audience from Site Configuration. A
  User controls only their own optional User Default Audience from Profile.
  Operators do not set another User's override.
- A User Default Audience overrides the Site Default Audience. “Use site
  default” is the initial per-user state and displays the currently inherited
  value.
- Both stored defaults are closed to Public, Subscribers, and Private. Named
  Audiences are excluded because their independent lifecycle could silently
  invalidate or change a stored default.
- An unset or malformed Site Default Audience remains Private, so upgrades never
  publish silently. A malformed User Default Audience is an error rather than
  inheritance of a potentially broader site value.
- The Effective Default Audience applies only when a new Post has no explicit
  audience. It applies consistently to web and AtomPub creation. Changing either
  default never changes an existing Post.
- Authors repair existing Posts through a dedicated, compact Manage Posts screen
  at `/posts/manage`. It contains active Posts in three disjoint publication
  states: Draft (`published_at` absent), Scheduled (`published_at > now`), and
  Published (`published_at <= now`). Deleted Posts are not listed and restore or
  purge is not introduced.
- Manage Posts offers one publication-state filter, one audience-target filter,
  and title/slug text search. Publication state accepts All, Draft, Scheduled,
  or Published. Audience accepts All, Public, Subscribers, Private, or one of
  the User's Named Audiences; Public, Subscribers, or Named matches target-set
  membership, while Private matches only the empty target set. Text search is a
  case-insensitive substring over the authored title and slug after Unicode
  whitespace normalization.
- Rows expose the minimum information needed to identify a Post and judge a bulk
  change: the existing compact title-or-summary fallback label (and the slug
  fallback for textless Posts), publication state, complete Audience Selection,
  and updated time.
- Selection persists across pages. The User may select visible rows or
  explicitly select every Post matching the current filters. The interface
  always reports the exact selected count.
- Opening confirmation resolves the current selection to an immutable snapshot
  of exact Post IDs plus a concurrency token for each Post. “Select all
  matching” resolves the filtered query at that moment. A newly matching or
  newly created Post is not added and does not invalidate the snapshot; a
  non-selected Post leaving the query is also irrelevant. Any mutation,
  deletion, disappearance, or authorization change to a selected Post aborts the
  complete operation without mutation and requires a refreshed selection.
- The first bulk operations are Change Audience and Delete. The framework is an
  internal reusable contract for Jaunder-owned operations, not a plugin or
  user-defined action API.
- Change Audience uses the existing audience picker and replaces the complete
  Audience Selection, including any Named Audiences, for every selected Post.
- Delete applies the existing Deleted Post semantics to every selected Post.
  Confirmation states the exact count and scope. Selections of ten or more also
  require entering the count; smaller selections require ordinary explicit
  confirmation.
- Each bulk operation is one synchronous, all-or-nothing transaction. Any stale,
  missing, unauthorized, or otherwise invalid target aborts the entire
  operation.
- Bulk orchestration preserves ordinary per-Post semantics: each meaningful
  mutation creates the same Post Revision and causes the same Syndication Feed,
  WebSub, and other externally observable consequences as an equivalent
  individual operation. An already-equal Audience Selection is a semantic no-op
  with no Revision, timestamp change, or publication side effect. Results
  distinguish the selected count from the materially changed count. Bulk work is
  not an administrative bypass.
- No arbitrary selection-size cap is introduced. The initial operations are
  expected to remain bounded enough for a synchronous transaction, and the UI
  prevents duplicate submission while one is pending.

## Acceptance

- An operator can read and change the Site Default Audience in the web UI among
  Public, Subscribers, and Private; unauthorized Users cannot mutate it.
- A User can choose Public, Subscribers, Private, or “Use site default” in
  Profile and can see the inherited Site Default Audience.
- Web and AtomPub creation without an explicit audience use the User Default
  Audience when present and otherwise the Site Default Audience.
- Explicit audience input wins over either default, and changing a default does
  not mutate existing Posts.
- Unset or malformed site configuration resolves to Private without hiding a
  storage failure; malformed user configuration rejects resolution and Post
  creation rather than inheriting the site value.
- Manage Posts presents the current User's active Draft, Scheduled, and
  Published Posts and excludes other Users' and Deleted Posts. Titleless Posts
  use the existing compact fallback-label projection and textless Posts use the
  slug fallback.
- Publication-state filters implement the three `published_at` predicates;
  audience filters implement target membership with Private as the empty target
  set; title/slug search implements normalized case-insensitive substring
  matching. Pagination does not discard explicit selections.
- “Select all matching” snapshots the complete filtered result across pages and
  displays its exact count before an operation can proceed. Later matching
  insertions do not join or invalidate it, while any concurrent change to a
  selected Post rejects the whole operation.
- A stale selected Post, authorization failure, or injected storage failure
  leaves every selected Post unchanged and reports an actionable conflict or
  failure.
- Bulk Change Audience replaces the complete Audience Selection for every
  selected Post. Material changes record ordinary Post Revisions and publication
  side effects; already-equal Posts remain semantic no-ops.
- Bulk Delete applies Deleted Post semantics to every selected Post, records
  ordinary Post Revisions and publication side effects, and enforces the
  count-entry safeguard for selections of ten or more.
- Successful operations report both selected and materially changed counts,
  refresh the compact result set, and cannot be submitted twice while pending.
- Dual-backend storage and HTTP integration tests prove default precedence,
  exact selection, authorization, atomic success, rollback, and revision/side
  effect parity.
- End-to-end tests cover operator and User settings, compact filtered selection
  across pages, Change Audience, Delete confirmation, pending state, success,
  and stale-selection failure.
- Visual proof uses the same seeded Posts at 1440×900 and 390×844: Before shows
  `/app` and `/drafts`; After shows `/posts/manage`. Manage Posts keeps its
  label, state, Audience Selection, selection control, and updated time
  available without horizontal page overflow. Confirmations show the operation,
  exact count, and complete target Audience Selection or Deleted Post scope.

## Boundaries

- This work does not change an existing Post merely because a default changes.
- It does not permit Named Audiences as Site or User defaults.
- It does not expose Deleted Posts, restore, permanent purge, bulk publication,
  bulk editing, a plugin API, or user-defined operations.
- It does not introduce durable background jobs, progress recovery after leaving
  the page, or an arbitrary operation-size cap.
- If measured production behavior later requires a cap, a follow-up must first
  preserve ergonomic batching by providing inverse/result-aware filters that
  make already-processed Posts easy to exclude.
- Friendly Named Audience discovery for external Protocol Clients remains
  outside this issue.
