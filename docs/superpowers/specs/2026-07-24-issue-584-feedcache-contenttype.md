# Spec — #584: type the `FeedCacheRow` fields (ContentType; ETag decision)

Milestone #13 (Domain-value type safety). Applies ADR-0063 (make invalid states
unrepresentable), the #438 sqlx-newtype bridge, and the existing `#495`
`common::media::ContentType` newtype. No new ADR: this is a mechanical
application of established patterns.

## Problem

`FeedCacheRow` crosses the public `FeedCacheStorage` trait
(`storage/src/feed_cache.rs:15-36`) with a raw `content_type: String` — a value
that is always a valid media type by construction but is erased to an untyped
`String` past the row-mapping boundary, unlike the sibling media records which
already carry `common::media::ContentType`. `etag: String` similarly carries an
unenforced `"…"`-quoted-format invariant.

## Decisions

1. **`content_type: ContentType`.** `FeedCacheRow.content_type` becomes
   `common::media::ContentType`. `ContentType` is a `StrNewtype` with the
   validating sqlx bridge (#438), so the `content_type` column decodes directly
   into the newtype at the query boundary through its `FromStr` — a corrupt
   stored value is rejected as a `ColumnDecode` error before the row mapper
   runs, exactly as `MediaRow` already does. No hand re-parse.

2. **Typed producer via a trusted door.** `FeedFormat::content_type()`
   (`common/src/feed/feed_path.rs:68`) returns `ContentType` instead of
   `&'static str`, making the feed-regenerate → cache → serve path typed
   end-to-end and removing the `.to_string()` at
   `server/src/feed/regenerate.rs:112`. It mints via a new `pub(crate)`
   `ContentType::from_trusted(impl Into<String>)` door from its three fixed,
   known-valid literals — the established pattern for minting a newtype from
   known-valid constants (`RenderedHtml::from_trusted`; the in-module
   `detect_content_type`, which is refactored onto the same door so there is a
   single trusted mint site). **Pinned** by `format_content_types` (each literal
   is a valid media type → an invalid edit fails the build).

   A `FromStr` round-trip (`literal.parse().expect(...)`) — the first-cut idea —
   is **not viable**: the repo denies
   `clippy::expect_used`/`missing_panics_doc`, so it fails the static gate.
   `pub(crate)` keeps the bypass in-crate: outside `common` the only door stays
   the validating `FromStr`.

   The `#398` `rendered-html-from-trusted` static gate reserves the
   `from_trusted` **name** (leaf-matched) for `RenderedHtml`'s XSS-sensitive
   door, so it is **extended** to exempt the `ContentType::` qualifier — a
   distinct, non-HTML door — with tests proving it still bites
   `RenderedHtml::from_trusted`, bare/aliased forms, and any other type.

3. **`etag` stays `String` (documented); repo-wide `ETag` newtype deferred.**
   The quoted-format invariant is upheld by construction at each of the ~5
   independent server producers (`site.rs`, `projector/mod.rs`, `media.rs`,
   `feed::metadata::feed_etag`, `atompub::posts::etag_for`); only the feed_cache
   copy is ever stored. Typing the stored field alone would enforce nothing
   across the untyped producers. A meaningful `ETag` newtype is a cross-cutting
   sweep threading every producer — out of scope for "type the `FeedCacheRow`
   fields." Recorded as a documented comment on the field; a separate milestone
   #13 follow-up issue captures the repo-wide newtype (plan's first task).

## Acceptance criteria

- **AC1** `FeedCacheRow.content_type` has type `common::media::ContentType`
  across the `FeedCacheStorage` trait surface and the generic `FeedCacheStore`
  impl (both backends). No `String` field for content type remains.
- **AC2** The `content_type` column decodes into `ContentType` via the #438
  bridge inside the `query_as` tuple mapping — no hand `parse`/`try_from` in the
  row mapper.
- **AC3** `FeedFormat::content_type()` returns `ContentType`, minted via the
  `pub(crate) ContentType::from_trusted` door on its three static literals;
  `regenerate.rs` no longer stringifies it (`.to_string()` removed); the sole
  non-test caller (`regenerate.rs:112`) compiles against the new return type. A
  unit test pins that each of the three literals is a valid `ContentType`.
  `detect_content_type` mints through the same door. The `#398`
  `rendered-html-from-trusted` gate is extended to exempt the `ContentType::`
  qualifier, with tests proving it still bites `RenderedHtml` and other types.
- **AC4** `etag` remains `String` and carries a code comment recording the
  deferral rationale and **referencing the concrete follow-up issue number**.
  Because the comment must cite a real number, the follow-up issue is filed
  first (plan task 1) — before the comment is written.
- **AC5** Behavior unchanged: `GET /feed.{rss,atom,json}` still serves the
  correct `Content-Type` header and honors `If-None-Match` → `304`. Existing
  feed-cache storage tests and `server/src/feed/handlers.rs` tests pass with the
  test rows built via `common::test_support::parse_content_type(...)` (per the
  newtype test-helper convention) rather than `"…".into()`.
- **AC6** Coverage: the `ContentType`-decode error path on the feed_cache read
  is exercised (a stored non-content-type value yields a `ColumnDecode` error),
  matching how the macros/storage crates cover the other bridge decode paths.
- **AC7** `cargo xtask validate --no-e2e` clean.

## Out of scope

- A repo-wide `ETag` newtype and threading it through the ~5 producers (filed as
  a follow-up; plan task 1).
- `FeedCacheRow.body` stays `String` — an opaque rendered-document payload, no
  domain invariant.
- Any change to feed cache SQL/schema, endpoints, or 304 semantics.

## Testing

- Storage: existing `feed_cache.rs` backend tests, updated to build
  `ContentType` via `parse_content_type`. Add the AC6 decode-error case.
- Server: `feed/handlers.rs` tests updated to `parse_content_type`; they already
  assert the served `Content-Type` header and 304 behavior (AC5).
- `common`: `FeedFormat::content_type()` return-type change covered by existing
  feed_path tests (adjusted to the newtype).
