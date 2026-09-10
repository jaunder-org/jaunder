# Issue #1432 — Show Display Names in Post headers

## Outcome

Every rendered Post header identifies its author with the User's current Display
Name followed by the canonical `@username`. When the User has no Display Name,
the header visibly and accessibly shows only `@username` rather than repeating
the Username.

## Load-bearing decisions

- Display Name remains an optional, current User presentation value. It is not a
  second identity and is not copied into Post state.
- Existing Posts reflect profile Display Name changes on their next fetch or
  refetch. Publication does not snapshot the label.
- The behavior applies to every surface that renders the shared Post header: the
  authenticated `/app` personalized Home Feed; site, User, and tag listings;
  published permalinks; and author-visible Draft and Scheduled Post views.
- The canonical Username remains present as `@username` on every Post. It
  continues to own URLs, identity comparison, authorization, and protocol
  credentials.
- Style Contract version 1 remains unchanged. Every Post header retains exactly
  one `author-name` hook and its existing `author-handle` hook.
- When no Display Name exists, the `author-name` hook is empty and hidden from
  visual and accessibility presentation; the handle is the only presented author
  text.
- The server projector and CSR client consume the same Post presentation data
  and shared renderer. Display Name must not arrive through a later per-Post
  client fetch that could create a paint mismatch or an N+1 request pattern.
- This is an ordinary extension of the existing Display Name domain value and
  Post presentation DTO. It does not introduce a new architectural decision.

## Acceptance

- A Post by Username `alice` with Display Name `Ada Lovelace` renders an
  `author-name` of `Ada Lovelace` immediately followed by an author handle of
  `@alice`.
- A Post by `alice` without a Display Name visibly and accessibly presents only
  `@alice`; it does not render a second visible `alice` label.
- The no-Display-Name case still contains one hidden, empty `author-name` Style
  Contract hook and one visible `author-handle` hook.
- The display behavior is the same on the authenticated `/app` personalized Home
  Feed, public listings, permalinks, and author-visible unpublished Posts.
- The initial server-projected Post content and the mounted CSR Post content
  remain byte-identical for both presence and absence of a Display Name.
- After a User changes their Display Name, reloading an existing Post shows the
  new label without modifying that Post.
- Both SQLite and PostgreSQL Post reads carry the current optional Display Name
  without a second author lookup.
- Existing tests and an end-to-end browser scenario prove the visible
  Display-Name-plus-handle and handle-only cases.

## Boundaries

- Do not change masthead or topbar titles, profile headings, avatars, owner
  action labels, permalinks, or routing.
- Do not change Syndication Feed or AtomPub author serialization.
- Do not add author links or new profile-editing behavior.
- Do not version or broaden the Style Contract.
- Do not add Post columns, migrations, historical Display Name snapshots, or a
  client-side author lookup.
