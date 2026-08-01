# Feed regeneration reads tags N+1 — spec (issue #772)

Status: approved (2026-08-01) Issue:
[#772](https://github.com/jaunder-org/jaunder/issues/772) Origin: #766 trace
analysis (`docs/superpowers/specs/2026-07-31-issue-766-sqlite-busy-e2e.md`,
Deliverable 5)

## Problem

`build_feed_items` (`server/src/feed/regenerate.rs:132-158`) issues one
`posts.get_tags_for_post(post_id)` round-trip per post record per feed
regeneration. In #766's failing e2e run this produced **15,594**
`storage.posts.get_tags_for_post` spans — the single most frequent span in the
capture. The feed worker regenerates every affected surface after each go-live,
and each regeneration re-reads tags post by post.

The reads are read-only, so they take no write lock and are not a #766
mechanism. They multiply pool round-trips and CPU on the same hot background
path that produced the #766 write storm, and on a starved CI runner that load
contributes to the contention climate.

## Root cause — the reads are redundant, not merely unbatched

The issue proposes fetching tags for the regeneration's post set in one query
(batched `WHERE post_id IN`, or a join in the listing query). **Investigation
found the join already exists**, so neither is needed.

`list_published_in_window` (`storage/src/posts.rs:1779-1796`) — the query that
produces the very `PostRecord`s `build_feed_items` iterates — already projects
`DB::TAGS_SUBQUERY` via `list_published_in_window_rows` (`:2227`, spliced at
`:2254`), and `post_record_from_row` parses it into
`PostRecord.tags: Vec<PostTag>` (`storage/src/helpers.rs:157-181`). The
projections are field-for-field equivalent: both `TAGS_SUBQUERY` and
`get_tags_for_post` select `t.tag_id` / `t.tag_slug` / `pt.tag_display`, with
`post_id` supplied from the row. `build_feed_items` consumes only `tag_display`
(`server/src/feed/regenerate.rs:156`).

So the correct fix takes the extra round-trip count from N to **zero**, not from
N to one. This supersedes the issue's stated ask; the rationale is recorded on
the issue.

## Decisions

**D1 — Read tags from `p.tags`; delete the per-post read.** `build_feed_items`
maps `p.tags` (already populated) instead of calling `get_tags_for_post`. Feed
regeneration performs **no** tag round-trips beyond the listing query it already
runs.

**D2 — Simplify `build_feed_items`' signature.** With the only storage call
gone, it no longer needs the `posts: &dyn PostStorage` parameter, `async`, or
`Result`. It becomes a function of `(base, records)`. This is a direct
consequence of D1, not separate scope.

**D3 — Pin tag ordering in SQL, in both dialect constants, under a collation
that agrees.** `get_tags_for_post` sorts `ORDER BY t.tag_slug`
(`storage/src/posts.rs:1516`); `TAGS_SUBQUERY` has no `ORDER BY`, so D1 would
otherwise leave feed tag order as whatever the join scan yields. Ordering is
therefore pinned at the source:

- sqlite: `json_group_array(json_object(...) ORDER BY t.tag_slug)`
- postgres: `json_agg(json_build_object(...) ORDER BY t.tag_slug COLLATE "C")`

`ORDER BY` inside an aggregate requires SQLite >= 3.44. This tree builds
**bundled** SQLite 3.46 (`Cargo.toml:58` → sqlx 0.8.6 `sqlite` →
`sqlx-sqlite/bundled` → libsqlite3-sys 0.30.1, `SQLITE_VERSION_NUMBER 3046000`;
no `LIBSQLITE3_SYS_USE_PKG_CONFIG` anywhere in `flake.nix`/`nix/`). So both
dialect strings stay structurally parallel — one clause each — rather than
SQLite needing a nested ordered-subquery wrapper.

The `COLLATE "C"` is load-bearing, not decoration. SQLite's `ORDER BY <text>` is
BINARY; Postgres uses the database collation, which here is inherited from
`initdb`'s environment locale (`tools/devtool/src/pg.rs:36-46` passes no
`--locale`; the `CREATE DATABASE` calls in
`storage/src/test_support.rs:515,564,611` pin nothing; the column carries no
`COLLATE` — `storage/migrations/{sqlite,postgres}/0009_create_tags.sql:3`). Tag
slugs are `[a-z0-9][a-z0-9-]*` (`common/src/tag.rs:6`), so hyphens and digits
are in-alphabet and the two disagree. Measured on this box:

```
C / SQLite BINARY   →  a-b  a1  ab  web-dev  web1  webdev
en_US.UTF-8         →  a1  a-b  ab  web1  web-dev  webdev
```

`COLLATE "C"` (always available in Postgres) makes Postgres byte-order the
ASCII-only slugs, which is exactly SQLite's BINARY — so the dialects agree by
construction rather than by luck of the cluster's locale. Per-post tag count is
bounded at 25 (`MAX_TAGS_PER_POST`, `common/src/tag.rs:99`), so the sort is over
<= 25 rows and raises no index concern.

**D4 — Slug-ordering is a documented contract of `PostRecord.tags`.**
`TAGS_SUBQUERY` feeds every post read site (11 splice sites plus the two
`update_post` impls), so D3 makes slug-ordering a property of all of them, not
just feeds. It is documented so the guarantee is discoverable instead of buried
in two SQL constants. `PostRecord.tags` (`storage/src/posts.rs:68`) is currently
the only field on the struct with no doc comment at all, so this fills a real
gap.

**D5 — `get_tags_for_post` is unchanged.** It keeps its own
`ORDER BY t.tag_slug` — the same contract D3 gives the aggregate. It runs in the
same database as `TAGS_SUBQUERY`, so the two always agree _within_ a backend.
Its four remaining callers are all single-post reads; after D1 it has **zero
callers on any list path**, so the N+1 _class_ is eliminated regardless.

**D6 — No anti-regrowth guard for the loop.** A mock-based guard
(`MockPostStorage` with no `expect_get_tags_for_post`, so a reintroduced
per-post read panics) was considered and **deliberately not added**:
`get_tags_for_post` is expected to be removed outright by #771, and guarding
against calls to a method that will not exist is dead weight. D2's signature
change is the real proof — with no `PostStorage` parameter, `build_feed_items`
_cannot_ reach storage, which the compiler enforces.

**D7 — No follow-up issue; the removal belongs to #771.** The four surviving
`get_tags_for_post` reads are all superfluous and removable, but that work sits
on #771's exact three production sites, on adjacent lines in the same hunks —
and hoisting the read out of `apply_post_tag_diff` is what lets #771 open its
batched write with `BEGIN IMMEDIATE` and nothing before it. Filing it separately
would have created a collision. Recorded as a scope addition on #771 instead.

**D8 — The AtomPub per-post ETag becomes deterministic; a one-time shift is
accepted.** `server/src/atompub/posts.rs:96` folds
`post.tags.iter().map(|t| &t.tag_display)` into a sha256 **in iteration order**.
Today that order is unspecified, so an entry's ETag is nondeterministic across
query plans and backends; D3 fixes that (an unclaimed benefit). It also means
existing posts' AtomPub entry ETags can shift once at deploy. That is benign —
an ETag change causes a re-fetch, never staleness — and is recorded rather than
guarded.

## Acceptance criteria

Each is stated so conformance can be told delivered from not.

- **AC1 — Feed regeneration performs zero tag round-trips.** `build_feed_items`
  takes no `PostStorage` parameter, is not `async`, and does not return
  `Result`. Storage access is therefore unreachable from it _by compilation_,
  not by convention — this subsumes the weaker "no `get_tags_for_post` call
  remains under `server/src/feed/`".
- **AC2 — Feed bodies still carry tags, slug-ordered.** For a post whose tags
  were written in reverse-slug order, a regenerated **JSON Feed** body's
  `items[0].tags` equals the slug-ordered label vector exactly. JSON is named
  deliberately: **RSS renders no tags at all** (`common/src/feed/rss.rs` never
  reads `item.tags`), and while Atom does emit them as `<category term=…>`
  (`common/src/feed/atom.rs:44-52`), only JSON (`common/src/feed/json.rs:25-26`)
  yields an array that takes an exact vector assertion rather than
  substring-index comparison. Real-DB test, both backends.
- **AC3 — `PostRecord.tags` is slug-ordered on both backends.** A post tagged in
  reverse-slug order reads back slug-ordered, through `TAGS_SUBQUERY`, on both
  dialects.
- **AC4 — The contract is documented** at three sites: the `PostRecord.tags`
  field (`storage/src/posts.rs:68`), the `TAGS_SUBQUERY` trait declaration
  (`storage/src/posts.rs:809-811`), and the two impl constants
  (`storage/src/{sqlite,postgres}/posts.rs:15`) — the latter noting that the
  `ORDER BY` is what provides the ordering, that the two must stay in sync, and
  why Postgres needs `COLLATE "C"` to match SQLite.
- **AC5 — Both dialect constants carry the `ORDER BY`,** asserted by a test
  rather than by reading (see test plan item 4).
- **AC6 — Backend parity: no user-visible behavior differs between backends.**
  With `COLLATE "C"` this is achieved rather than documented-around; AC3's
  dual-backend test is what demonstrates it.
- **AC7 — The gate is green.** `cargo xtask validate` passes (`--no-e2e` while
  iterating; full `validate` before merge).

## Test plan

Per `CONTRIBUTING.md:831-858`, the `test-backend-pattern` guard requires every
DB-touching test under `server/tests/` and `storage/src/` to be
backend-explicit, so any real-DB assertion here is dual-backend by construction.
A mock test cannot exercise `TAGS_SUBQUERY` at all, so mock and real-DB coverage
are complementary, not alternatives.

1. **Strengthen `post_record_carries_tags`**
   (`server/tests/storage/mod.rs:5821`, already `#[apply(backends)]`). It
   asserts `p1_slugs == ["rust","web"]` but tags `p1` with `"Rust"` then `"web"`
   (`:5842-5847`) — insertion order _equals_ slug order, so the assertion cannot
   distinguish them and passes by accident. Tag in reverse-slug order. Model the
   shape on `get_tags_for_post`'s own ordering test
   (`server/tests/storage/mod.rs:3662-3708`), which already seeds
   `zebra`/`apple`/`mango` out of order. → AC3, AC6.
2. **Fix the same defect at the web surface:**
   `list_user_posts_carries_tags_per_post`
   (`server/tests/web/web_posts.rs:1871`) has the identical flaw — tags `"Rust"`
   then `"web"` at `:1893-1900`, asserts `["rust","web"]` at `:1909`.
   Strengthening only the storage test would leave a second vacuous ordering
   assertion behind. → AC3.
3. **Feed body tag rendering** in `server/tests/feed/feed_regenerate.rs` (all
   five existing tests are `#[apply(backends)] #[tokio::test]` real-DB): seed
   via `SeedRawPost::tags(["web","Rust"])`
   (`storage/src/test_support.rs:1001-1011`, applied in call order at `:1069`),
   regenerate `/~{user}/feed.json`, parse the body and
   `assert_eq!(v["items"][0]["tags"], json!(["Rust","web"]))`. The expected
   values are tag **displays** ordered by **slug** — `build_feed_items` emits
   `t.tag_display` (`server/src/feed/regenerate.rs:156`) while D3 orders by
   `tag_slug`, so `rust < web` yields `["Rust","web"]`. Note the existing tests
   in this file all use `.rss`, which renders no tags — the new test must use
   `.json`. → AC2.
4. **Dialect-constant guard** (unit test, no DB): assert both
   `<Sqlite as PostDialect>::TAGS_SUBQUERY` and
   `<Postgres as PostDialect>::TAGS_SUBQUERY` contain `ORDER BY t.tag_slug`.
   Cheap, needs no backend, and unlike the guard declined in D6 it operates on
   the dialect constants — which #771 does not touch, so it does not go stale. →
   AC5.
5. **Existing mock-based tests** in `server/src/feed/regenerate.rs` continue to
   pass with the simplified `build_feed_items`. → AC1.

### Order-assertion sweep — completed, zero breaks

Adding `ORDER BY` changes tag order for every post read site, so every existing
assertion over tag order was enumerated. **Nothing breaks.**

_Consumers of `PostRecord.tags`:_ `server/src/atompub/mapping.rs:177-185` (Atom
`<category>` order, user-visible); `server/src/atompub/posts.rs:96` (post ETag,
order-sensitive — see D8); `web/src/posts/render.rs:99,118` and
`web/src/posts/component.rs:177,988,1155` (tag-list HTML, editor chip prefill);
`web/src/posts/api.rs:314,476,510,538` (collected into `BTreeSet<Tag>`,
order-irrelevant); `storage/src/posts.rs:1837` (`GoLivePost.tag_slugs` fan-out,
order-irrelevant).

_Order assertions:_ the two accidental-pass tests above (items 1-2, both still
pass after the change); `server/tests/storage/mod.rs:3662-3708` (already
correct, unaffected by D5); single/empty tag sets at
`server/tests/storage/mod.rs:5875-5885`,
`server/tests/web/web_posts.rs:1965-1967`,
`server/tests/misc/backup_fixture.rs:204-206`. E2E
(`end2end/tests/posts.spec.ts:770-772,781,805,851-854`, `feeds.spec.ts:124`)
uses `toContainText` throughout — order-insensitive. Renderer tests in
`common/src/feed/{json,atom,rss}.rs` carry 0 or 1 tag each. **No golden/snapshot
files exist.** Backup dumps `post_tags ORDER BY "post_id","tag_id"`
(`storage/src/backup.rs:639`) — unaffected.

## Out of scope

- Removing `get_tags_for_post` and its four callers → #771 (D7).
- Batching the per-tag _write_ loops → #771.
- Any change to `feed_etag`'s hashed field set
  (`common/src/feed/metadata.rs:44-58`). Tags are not hashed there today; that
  is the reason ordering must be pinned at the source (an unpinned reorder would
  be invisible to consumers via ETag), not a proposed change. The separate
  _AtomPub post_ ETag does move — see D8.
