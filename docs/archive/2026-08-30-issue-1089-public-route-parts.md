# Typed public feed and permalink route parts

## Outcome

Public Syndication Feed extensions and Post permalink parts become typed at
their outermost route boundaries without changing public routing behavior. Valid
URLs resolve exactly as before; malformed feed URLs return 404, while malformed
permalink URLs fall through to the projector shell rather than producing an Axum
pre-handler 400.

## Load-bearing decisions

- Implement both typed boundaries identified by #697 and issue #1089; the
  current primitive route pieces are not retained as intentional exceptions.
- `FeedFormat` owns the complete, case-sensitive public extension grammar:
  `rss`, `atom`, and `json`.
- `FeedFormat` implements string parsing, and both server route extraction and
  `FeedPath` parsing reuse that implementation. There is one extension-to-format
  mapping.
- Feed handlers receive a softly extracted `FeedFormat`, not a raw extension
  string. A parse miss remains handler-visible so the handler can return 404.
- `FeedFormat` uses ADR-0091's `#[text_enum]` convention through a `no_serde`
  option that suppresses only the macro's Serde bridge. Its existing derived
  Serde and wire representation remain unchanged.
- Common code owns one typed `PermalinkRoute` value containing `Username`,
  `PermalinkDate`, and `Slug`.
- `PermalinkRoute` owns all-or-nothing semantic parsing of five decoded segment
  strings: username without the route's `~` marker, year, month, day, and slug.
  Calendar validity is enforced jointly through `PermalinkDate`; year, month,
  and day do not become independent domain types.
- Server and client route handling use private capture adapters to normalize
  their router-specific inputs and call that shared parser. The client adapter
  removes its captured `~`; the server capture already excludes it.
- The projector's Axum boundary softly converts a successfully decoded,
  five-capture permalink match into an optional `PermalinkRoute`. No raw numeric
  date parts or partially parsed permalink value cross into the handler.
- For those matched and decoded captures, any invalid username, numeric token,
  calendar date, or slug becomes a soft miss that preserves projector-shell
  fallback. Nonmatching path shapes and path-decoding failures retain their
  existing router/extractor behavior.
- Permalink numeric grammar remains exactly Rust `i32::from_str` semantics for
  year and `u32::from_str` semantics for month and day, followed by
  `PermalinkDate::from_ymd`; accepted signs, leading zeros, overflow rejection,
  and non-digit rejection therefore do not change. Feed extensions remain
  lowercase-only.
- These ownership choices apply ADR-0063 section 4: parse domain values at the
  outermost repository-owned boundary and hold the typed value inward.

## Acceptance

- Every valid public feed route continues returning the same feed representation
  and content type for `rss`, `atom`, and `json`.
- Invalid feed extensions continue returning 404 across site, tag, user, and
  user-tag feed routes; none become Axum 400 responses.
- `FeedFormat` is the sole repository-owned parser for public feed extension
  tokens; server and `FeedPath` code contain no duplicate token match table.
- Valid Post permalinks continue resolving through both cold projector requests
  and client-side routing.
- For successfully decoded five-capture matches, malformed permalink usernames,
  date tokens, impossible calendar dates, and slugs continue serving the
  projector shell; none become Axum 400 responses. Other path shapes and
  decoding failures behave exactly as before.
- Server handlers and inward calls receive `PermalinkRoute` or its typed
  components, never raw permalink date primitives.
- Client and server permalink parsing share the common route value and semantic
  parser.
- Focused tests cover valid and malformed feed extensions at the observable
  route boundary. Shared permalink-parser tests pin representative zero-padded
  and signed numeric forms, overflow and non-digit rejection, and impossible
  dates; observable route tests prove representative valid paths still resolve
  and semantic parse misses still serve the shell.
- The implementation references #697 and ADR-0063 so the audit origin and
  governing boundary rule remain discoverable.

## Boundaries

- No public URL shape, redirect, canonicalization policy, status code, feed
  payload, or projector-shell policy changes.
- No change to AtomPub route extraction; ADR-0063's projector-versus-AtomPub
  split remains intact.
- No new domain types for individual year, month, day, extension text, username,
  or slug values.
- No storage schema, database query, API protocol, or serialization-format
  change.
- No general rewrite of `SoftPath`; it remains available for other deliberately
  soft route boundaries.
- No new ADR or glossary term is required: this applies existing
  `Syndication Feed`, Post permalink, and ADR-0063 concepts rather than
  introducing a durable architectural exception.
