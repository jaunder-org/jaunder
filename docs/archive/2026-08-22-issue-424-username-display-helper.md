# Issue #424: username display helper for user listing pages

## Outcome

`UserTimelinePage` and `UserTagPage` keep rendering the same canonical username
text in their page chrome while sharing one small expression for converting the
route `Memo<Option<Username>>` into display text. The change removes the
duplicated username-display closures without changing routing, fetching,
projector seeding, or rendered UI behavior.

## Load-bearing decisions

- Preserve the existing route parsing boundary: each page still parses the
  `~username` path segment into `Memo<Option<Username>>` at the source, where
  invalid segments become `None` and downstream consumers handle absence.
- Preserve the heading behavior exactly: valid usernames render their canonical
  parsed/lowercased `Username`; invalid route segments render an empty username
  string because the page already renders the invalid-route state elsewhere.
- Extract only the shared display conversion for `Memo<Option<Username>>`. Keep
  route-specific behavior in each page: `UserTimelinePage` still owns its
  profile timeline resource, feed/RSD discovery, subscribe button, and topbar
  text; `UserTagPage` still owns its tag memo, tag heading, feed discovery, and
  empty text.
- Keep the helper local to the posts component module unless an existing nearby
  helper location is clearly more idiomatic. Do not create a new cross-vertical
  shared component or public API for this cosmetic dedup.
- This is a pure code-quality refactor. No route, request, server function,
  storage, projector seed, cache, CSS, or user-facing text changes.

## Acceptance

- The duplicated `username.get().map(String::from).unwrap_or_default()` closure
  body no longer appears separately in both `UserTimelinePage` and
  `UserTagPage`.
- Both pages call one shared helper for canonical username display text.
- `UserTimelinePage` still renders `Topbar` title text as
  `Posts by {canonical_username}`.
- `UserTagPage` still renders `Topbar` sub text as
  `Posts by ~{canonical_username}` and keeps its tag title behavior unchanged.
- Existing web/projector tests and the project check pass.
- No public API, storage schema, server route, or web client contract changes
  appear in the diff.

## Boundaries

- Do not change `Username` parsing or normalization.
- Do not alter invalid-route behavior beyond preserving the existing empty
  display string.
- Do not refactor the two page resources, load-more callbacks, feed discovery
  blocks, or timeline state wiring.
- Do not introduce a trait or broad extension surface solely for this helper.
