# Spec — #472: `PostId` newtype

- Issue: [#472](https://github.com/jaunder-org/jaunder/issues/472) (sub-issue of
  the umbrella [#457](https://github.com/jaunder-org/jaunder/issues/457))
- Milestone: Domain-value type safety (newtypes)
- Governing ADR: [ADR-0063](../../adr/0063-domain-value-newtype-convention.md)
  (numeric-ID trailer, §"Numeric IDs")
- Sibling precedent:
  [#471 `UserId`](../../archive/2026-07-16-issue-471-user-id-newtype-spec.md)
  (archived on ship) — the first ID newtype; **established the shared home
  (`common::ids`), sqlx-boundary conversion, and pervasive-adoption pattern this
  spec follows.**
- Date: 2026-07-16

## Problem

A post id crosses the codebase as a bare `i64` through `common`, `storage`,
`server`, and `web`. Any `i64` can be passed where a post id is expected, and at
a call site a `post_id` is indistinguishable from a `user_id`, `tag_id`, or any
other integer — the `tag_post(post_id, user_id)`-transposition class. Per
ADR-0063 §1 the value qualifies on the **transposition** axis. No invariant, no
security surface — the sole win is turning ID mix-ups into compile errors.

`PostId` is the second of the umbrella's eight ID newtypes to land, immediately
after `UserId` (#471). It reuses that pattern wholesale; nothing new is
established here.

## Decision

Introduce `PostId` per ADR-0063's numeric trailer and thread it through every
Rust site that carries a post id. Behavior and wire shapes are unchanged; this
is a `refactor:` (type-only) change.

### The type — `common::ids`

```rust
// common/src/ids.rs  (existing module; append PostId next to UserId)
/// A post's row id. Newtyped so it can't be transposed with another `i64` id
/// (user, tag, audience, …) at a call site.
#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash, IdNewtype)]
pub struct PostId(i64);
```

- Lives in **`common/src/ids.rs`** — the existing shared home (created by #471),
  appended next to `UserId`. `common` exposes no `pub use` re-exports —
  consumers use the full path **`common::ids::PostId`**. `common` is namable by
  client wasm, so web DTOs can adopt the type.
- `IdNewtype` (ADR-0062 `macros` crate) supplies `From<i64>`,
  `From<Self> for i64`, `Display`, and a **transparent-i64 serde bridge** (wire
  form stays a bare integer). It derives **no** sqlx traits and **no** `Ord`.
- **No `Ord`** — verified: no post id is a sort/`BTreeMap`/`.max()` key.
  `feed_etag` reads `items.last()` **positionally** (not by max id) and only
  hashes the id's bytes, so `PartialOrd`/`Ord` is not needed. `Hash` is kept
  (IDs plausibly seed `HashSet`/`HashMap`). Add `Ord` only if a future site
  needs it.
- No hand-written `FromStr`, no validation — an id has no value invariant.

### sqlx boundary — convert at the edge

Same as #471. `IdNewtype` derives no sqlx traits, and storage is generic over
`DB: Backend` with `for<'q> i64: Encode<'q, DB> + Type<DB>` bounds. `PostId`
crosses the DB boundary by **conversion, not trait impls**:

- **Writes:** `.bind(i64::from(post_id))` at each bind site (the `i64: Encode`
  bounds are unchanged — still binding an `i64`).
- **Reads:** keep decoding a raw `i64` in `query_scalar::<_, i64>` / `query_as`
  tuples, then **wrap** `PostId::from(raw)` inside `build_post_record` in
  `storage/src/helpers.rs` (the per-record chokepoint) and at `RETURNING id`
  scalar call sites.

SQL column names (`WHERE post_id = $1`, `RETURNING id`) are **unchanged** — the
newtype is a Rust-side concern; the schema and wire bytes are untouched.

### Pervasive adoption

Every Rust site that carries a post id adopts `PostId` — storage
records/traits/impls, `#[server]` args & returns, web DTO fields, and the feed
surfaces — **including `common::feed::metadata::FeedItem.id`** (sourced from
`PostRecord.post_id`; see below). The transparent-i64 serde bridge keeps every
serialized shape identical, so no e2e or wire change.

**`FeedItem.id` is a `PostId`.** It is the ETag `last_post_id` input. #470
covered only the flattened _string_ newtypes on `FeedItem` (`title`,
`content_html`); it does **not** own this `id`, which is in scope here. In
`feed_etag`, `last_id` is derived and hashed as bytes, so the hashing site reads
`i64::from(i.id)` — the ETag bytes are unchanged (still the bare `i64`
little-endian). The `#[cfg(test)]` `item(id, …)` helper's `id` param becomes
`PostId`; callers pass `PostId::from(n)`.

## Scope (layers)

The concrete line-numbered **edit-map is the plan's completeness surface**
(there is no enforcement gate). The layers, per the #457 survey (verify before
editing):

1. **common** — append `PostId` to `ids.rs`; in `feed/metadata.rs`,
   `FeedItem.id: PostId`, the `feed_etag` hashing site (`i64::from(i.id)`), and
   the test `item(id: PostId, …)` helper + its callers.
2. **storage** — `posts.rs` (`PostRecord.post_id`, `PostRevisionRecord.post_id`,
   `PostTag.post_id`, **`PostCursor.post_id`**, **`CollectionCursor.post_id`**,
   `soft_delete_post`, `unpublish_post`, `tag_post`/`untag_post`,
   `get_tags_for_post`, `get_post_by_id`, `update_post`, `get_post_audiences`,
   `replace_post_audiences`, `post_id_for_idempotency_key` return),
   `post_service.rs` (`RenderedPostUpdate.post_id`, `PostUpdate.post_id`,
   `PostCreation`), **`parse_post_cursor(cursor_post_id: Option<PostId>)`** (the
   public cursor-parse boundary — see below), `helpers.rs` (`build_post_record`
   wrap, `parse_post_tags_json` param; the `PostRecordParts` raw-decode tuple
   **stays `i64`** — `IdNewtype` derives no `Decode`, so the `PostId::from` wrap
   happens in `build_post_record`, mirroring today's `UserId::from`); object
   traits **and** the `Backend`-generic dispatch traits; impls (`.bind`/decode);
   both backend dirs (`sqlite/`, `postgres/`). Post-id **return types**
   (`RETURNING post_id` scalars) become `PostId`.
   - **`cursor_post_id` is `PostId` end-to-end, not wrapped at storage.** The
     web listing `#[server]` fns take `cursor_post_id: Option<PostId>` and pass
     it straight to `storage::posts::parse_post_cursor`, whose param therefore
     becomes `Option<PostId>` (its `PostCursor { post_id }` shorthand and unit
     tests update accordingly). Serde-transparency keeps the wire form a bare
     integer.
3. **server** — `atompub/*` (post-id params/returns,
   `Path<(Username, i64→PostId)>` route extractors, test literals),
   `feed/regenerate.rs` (`FeedItem { id: p.post_id }`,
   `get_tags_for_post(p.post_id)`).
4. **web** — `posts/mod.rs` (DTO fields `CreatePostResult`/`UpdatePostResult`/
   `DraftSummary`/`PublishPostResult`/`PostResponse` `.post_id` + `#[server]`
   params/returns: `get_post_preview` / `publish_post` / `delete_post` /
   `unpublish_post` / `post_audience_selection`, and
   `cursor_post_id: Option<PostId>`), `posts/listing.rs`
   (`TimelinePostSummary.post_id`, `TimelinePage.next_cursor_post_id`, the
   `cursor_post_id` params), `pages/posts.rs` (the route-param `post_id` parse
   boundary + `RwSignal`/struct sites). At the route-param boundary the parse
   becomes `.parse::<i64>().ok().map(PostId::from)` (an id has no `FromStr`);
   hidden `<input value=post_id>` and route-string interpolation render fine via
   `Display`.
5. **tests** — construct post ids via `PostId::from(n)` directly (an infallible
   wrap — no `parse_*` helper, matching #471; an unused thin wrapper trips the
   line-coverage gate). `assert_eq!`/`==` need only the derived `PartialEq`.

**Do not over-reach** (nearby `i64`s in touched files that are _not_ post ids):
`user_id`/`editor_user_id`/`author_user_id` (already `UserId`, #471), tag ids
(`TagId`, #473), audience ids (#475), subscription ids (#476). Type only post
ids.

## Acceptance criteria (observable)

- **AC1** `common::ids::PostId` exists (appended to `common/src/ids.rs`; the
  module is already declared `pub mod ids;`), derives
  `Copy, Clone, Debug, PartialEq, Eq, Hash, IdNewtype`, and has no hand-written
  `FromStr` and no `Ord`.
- **AC2** No storage record/input field, storage trait signature (object or
  dispatch), `#[server]` function signature, or web DTO field that holds a post
  id is typed `i64` — each is `PostId`, **including `FeedItem.id` and post-id
  return types** (`post_id_for_idempotency_key`, `tag_post`/create-post
  `RETURNING id` scalars, `get_post_preview`, …). **Completeness surface = the
  plan's edit-map checklist**, not a single grep. Supplementary grep (over
  touched files only) must cover `post_id`/`\.id\b` and
  `Result<i64`/`(i64,`/`-> i64` return/tuple forms; SQL string literals
  excepted.
- **AC3** Wire/serialized shapes are byte-identical — a `PostId` serializes as a
  bare integer; the feed ETag bytes are unchanged; all existing e2e and
  serialization tests pass unchanged.
- **AC4** Both SQLite and Postgres impls compile and pass; no migration is added
  (schema unchanged).
- **AC5** `feed_etag`'s ETag value is unchanged. The id still hashes as the bare
  `i64` LE bytes — byte-identity is guaranteed **by construction**
  (`i64::from(PostId::from(n)) == n`, pinned by the Task-1 round-trip test), and
  the existing `feed_etag` stability tests (`etag_stable_for_identical_input`,
  `etag_changes_when_count_changes`, …) stay green after the `item(id)` helper
  flips to `PostId`. (Those tests prove stability, not a golden value; the
  construction guarantee is what pins no-change, so no brittle golden-hash
  assertion is added.)
- **AC6** `cargo xtask validate --no-e2e` is clean (fmt, clippy incl. no new
  `unwrap`/`expect` in production, coverage gate), and the e2e suite passes.

## Tests

- `common`: unit-test the `PostId` derive in `ids.rs` — `From<i64>`/`i64::from`
  round-trip, `Display`, and a serde round-trip proving the wire form is a bare
  integer (`serde_json::to_string(&PostId::from(42)) == "42"`), mirroring the
  existing `UserId` tests.
- Existing storage/server/web/feed tests: update construction/comparison sites
  to `PostId` (behavior unchanged). The existing `feed_etag` tests
  (`etag_stable_for_identical_input`, …) guard AC3/AC5 after the `item(id)`
  helper flips to `PostId`. No new behavioral tests — this is a type-only
  refactor.

## Non-goals

- The other six ID classes (#473–#478) — `TagId`, `AudienceId`, etc. stay bare
  `i64` here; a signature that mixes `post_id` and (say) `tag_id` types only the
  `post_id`.
- A transparent sqlx bridge for the newtype (#438) — out of scope; use boundary
  conversion.
- `Ord` for `PostId` — not needed by any current site.
- Any schema/wire/behavior change.

## Risks

- **Shared files with sibling sub-issues.** `posts.rs` and `helpers.rs` also
  carry `UserId` (landed, #471) and future `TagId`/`RevisionId`. Touch only the
  `post_id` fields; expect to rebase if a sibling lands close together (the
  issue's own note).
- **`FeedItem.id` ownership overlap with #470.** #470 owns the string fields
  (`title`/`content_html`); this issue owns `id`. Verified above — no conflict.
- **axum route extractors rely on `PostId`'s `Deserialize`.** The atompub
  `Path<(Username, PostId)>` extractors decode the `{post_id}` segment via
  `PostId`'s transparent `Deserialize` (forwards to `i64::deserialize`, which
  axum's path deserializer drives by parsing the percent-decoded string) — same
  as the bare `i64` today, so no behavior change. The existing atompub route
  tests (AC3) guard against a regression.
- **Scale** — moderate (feed + posts storage + atompub + web posts). The
  compiler enumerates every rippled site once `PostRecord.post_id` and the trait
  signatures flip; lean on `cargo check --all-features --all-targets`. A missed
  _tightening_ (a local helper left `i64` that still compiles) is caught by the
  AC2 edit-map, not the compiler.
