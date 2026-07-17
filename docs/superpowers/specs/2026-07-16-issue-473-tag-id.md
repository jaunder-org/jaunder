# Spec — #473: `TagId` newtype

- Issue: [#473](https://github.com/jaunder-org/jaunder/issues/473) (sub-issue of
  the umbrella [#457](https://github.com/jaunder-org/jaunder/issues/457))
- Milestone: Domain-value type safety (newtypes)
- Governing ADR: [ADR-0063](../../adr/0063-domain-value-newtype-convention.md)
  (numeric-ID trailer)
- Date: 2026-07-16

## Problem

A tag's row id (`tags.tag_id`) crosses as a bare `i64` through `storage`,
transposable with any other id. Applies the umbrella's `IdNewtype` pattern
(#471/…/#477) to the tag **row id**. Per ADR-0068/#468: `TagLabel`/`Tag` (the
string slug/display) are already newtypes; `TagId` is the distinct numeric row
id. Tag ids are **storage-internal** — the web renders tags by
`tag_slug`/`tag_display`, never by id — so this is a storage-scoped change with
no wire surface.

## Decision

Introduce `TagId` per the shared convention; thread it through every Rust site
carrying a tag row id. Type-only; behavior and wire shapes unchanged.

### The type

```rust
// common/src/ids.rs
/// A tag's row id (distinct from the string `Tag` slug / `TagLabel` display).
#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash, IdNewtype)]
pub struct TagId(i64);
```

`common::ids::TagId`; From/Into/Display/FromStr + transparent serde. No `Ord`.

### sqlx / serde boundary

- `PostTagJson.tag_id` (the `#[derive(Deserialize)]` struct mirroring the SQL
  `json_object('tag_id', …)` aggregation, `storage/src/helpers.rs`) becomes
  `TagId` — the transparent-i64 serde bridge deserializes the JSON integer
  directly, so `parse_post_tags_json`'s `tag_id: r.tag_id` stays a **straight
  move** (no wrap).
- The row-tuple decode chokepoints in `posts.rs` wrap `TagId::from(raw)` at
  record construction: the `(post_id, tag_id, tag_slug, tag_display)` map →
  `PostTag` (~:1342), and the `SELECT tag_id, tag_slug FROM tags` map →
  `TagRecord` (~:1588).
- Generic `i64: Encode/Type` bounds untouched; no migration; SQL column names
  unchanged.

## Scope

1. **common** — define `TagId` in `ids.rs`.
2. **storage `posts.rs`** — `TagRecord.tag_id: TagId` (:243);
   `PostTag.tag_id: TagId` (:251, alongside the already-`PostId` `post_id`); the
   two decode chokepoints wrap `TagId::from` (~:1342, ~:1588); test literal
   `tag_id: TagId::from(0)` (~:2245).
3. **storage `helpers.rs`** — `PostTagJson.tag_id: TagId` (:154);
   `parse_post_tags_json` map unchanged (straight move); tests: the
   `{"tag_id": 1, …}` JSON string literals stay (they deserialize into `TagId`),
   the `assert_eq!(record.tags[0].tag_id, …)` becomes `TagId::from(1)` (~:684).
4. **server/web tests** — `server/atompub/posts.rs` `mk_tag` helper (:557): keep
   the `tag_id: i64` param and wrap `TagId::from(tag_id)` at the `PostTag {…}`
   construct (mirrors its `post_id: i64` param + `PostId::from` construct, so
   bare-literal callers need no change); `mapping.rs` synthetic
   `tag_id: TagId::from(i64::try_from(i)? + 1)` (:542); `web/src/feed_events.rs`
   test literals `tag_id: TagId::from(1|2)` (:48/:54/:60). (`server/tests` has
   no actionable site — only a JSON string literal.)

**Do not over-reach / deliberate leaves:**

- The **dialect fetch-to-bind transients** in
  `storage/src/{sqlite,postgres}/posts.rs`
  (`let tag_id: i64 = query_scalar::<_, i64>("SELECT tag_id FROM tags WHERE tag_slug = $1")…`
  then `.bind(tag_id)` into the `post_tags` INSERT) **stay `i64`**: a raw value
  fetched and immediately re-bound, crossing no `TagId`-typed API boundary —
  typing it would be a pure `i64 → TagId → i64` round-trip. (Same class as the
  raw-decode-before-wrap pattern.)
- `tag_slug: Tag` / `tag_display: TagLabel` are already string newtypes —
  untouched.
- `post_id` (already `PostId` #472). The `backup.rs` `"tag_id"` occurrences are
  SQL column-name **strings** — unchanged. SQL `JOIN … pt.tag_id = t.tag_id`
  strings unchanged.

## Acceptance criteria

- **AC1** `common::ids::TagId` exists, derived per convention.
- **AC2** No `tag_id` **field/record/return** is bare `i64`
  (`TagRecord`/`PostTag`/ `PostTagJson`); the plan edit-map is the completeness
  surface. Dialect fetch-to-bind transients + SQL strings excepted.
- **AC3** Wire/serialized shapes byte-identical — `PostTag`/`TagRecord` are
  internal (not `Serialize`); the internal SQL-JSON deserialization is unchanged
  (`TagId` reads the bare integer). Existing tests pass.
- **AC4** Both backends compile and pass; no migration.
- **AC5** `cargo xtask validate --no-e2e` clean; e2e green in CI.

## Tests

Construct via `TagId::from(n)`. The JSON string fixtures (`{"tag_id": 1, …}`)
stay verbatim. No new behavioral tests.

## Risks

- Low; storage-scoped, no wire, no reactive-store. Builds on merged #472
  (`PostTag.post_id` is `PostId`). The one judgement call is the dialect
  fetch-to-bind `i64` leave (documented).
