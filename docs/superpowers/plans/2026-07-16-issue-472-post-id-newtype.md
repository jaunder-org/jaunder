# Plan — #472: `PostId` newtype

- Spec:
  [2026-07-16-issue-472-post-id-newtype.md](../specs/2026-07-16-issue-472-post-id-newtype.md)
- Issue: [#472](https://github.com/jaunder-org/jaunder/issues/472)
- Precedent: #471 (`UserId`) — same two-commit shape.

## Shape & commit strategy

Two commits, because of the gated-commit + compile-ripple constraint (identical
to #471):

- **Commit 1 — define the type** (`common::ids::PostId` + unit test). New,
  unused-by-others → the workspace stays green. Independently verifiable.
- **Commit 2 — thread it through everything** (common/feed, storage
  records/traits/impls/backends/helpers, server atompub + feed, web, all tests).
  Flipping `PostRecord.post_id` or a trait signature to `PostId` forces every
  reader across `storage`→`server`/`web` to change **in the same commit** — an
  intermediate per-crate commit would leave the workspace non-compiling, which
  the git-enforced gate rejects. It lands atomically. Moderate size but **purely
  mechanical** (a type substitution + boundary conversion at `.bind`/decode
  sites); the compiler enumerates every rippled site.

No separable-concerns issues to file — the nearby non-post-id ids are already
tracked (`UserId` #471 landed; `TagId` #473, `AudienceId` #475, etc.).

The authoritative site list is the **edit-map Appendix below**. Line numbers are
from the fork point (`issue-472-base`); **verify before editing** (siblings may
have shifted them). Task 2 is complete when the workspace is green **and** every
Appendix item is struck.

---

## Task 1 — Define `common::ids::PostId` + unit test

- [ ] Append to `common/src/ids.rs` (after `UserId`): a doc-commented
      `#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash, IdNewtype)] pub struct PostId(i64);`.
      (`use macros::IdNewtype;` and `pub mod ids;` already present — no lib.rs
      change.)
- [ ] Unit test in `ids.rs` (`#[cfg(test)]`) exercising **all generated code**
      (coverage gate): `From<i64>` construct, `i64::from(..)` round-trip,
      `Display` (`PostId::from(7).to_string() == "7"`), and serde
      **bare-integer** invariance
      (`serde_json::to_string(&PostId::from(42)) == "42"` and `from_str("42")`
      round-trips) — mirror the existing `UserId` tests.
- **Verify:** `cargo test -p common ids` green;
  `cargo check -p common --all-features`.
- **Commit:** `refactor(common): add PostId id newtype (#472)`.

## Task 2 — Thread `PostId` through the workspace

Work in dependency order, keeping edits mechanical; **do not commit until the
whole workspace is green.** Sub-steps (work order within the single commit):

- [ ] **common/feed/metadata.rs** — `FeedItem.id: PostId`; at the `feed_etag`
      hashing site read `i64::from(i.id)` before `.to_le_bytes()` (keep the ETag
      bytes identical); test helper `item(id: PostId, …)` + callers pass
      `PostId::from(n)`.
- [ ] **storage record/input/cursor structs** — `PostRecord.post_id`,
      `PostRevisionRecord.post_id`, `PostTag.post_id`, `PostCursor.post_id`,
      `CollectionCursor.post_id`; `post_service.rs`
      `RenderedPostUpdate.post_id`, `PostUpdate.post_id`, `PostCreation` post-id
      field.
- [ ] **storage `parse_post_cursor`** — the public cursor-parse boundary: param
      `cursor_post_id: Option<PostId>`; its `PostCursor { post_id }` shorthand
      now takes a `PostId` (compiler-forced); update its unit tests (`posts.rs`
      ~:2632/:2641/:2646) to pass `PostId::from(n)`. `cursor_post_id` is
      `PostId` end-to-end (web `#[server]` → this parse), **not** wrapped at
      storage.
- [ ] **storage trait defs** — object traits **and** `Backend`-generic
      dispatch/Dialect traits: `post_id` params on `soft_delete_post`,
      `unpublish_post`, `tag_post`, `untag_post`, `get_tags_for_post`,
      `get_post_by_id`, `update_post`, `get_post_audiences`,
      `replace_post_audiences`, **`apply_post_tag_diff`**; post-id **return
      types**: `post_id_for_idempotency_key`, and the **create-post return
      chain** — `write_post_in_tx`, object-trait `create_post`/`create_posts`,
      their impls, and `post_service::create_rendered_post` all return `PostId`
      (the `RETURNING post_id` wrap at the bottom forces this up the chain;
      call-site locals just become `PostId`, consumed by `get_post_by_id` which
      also takes `PostId` — no `i64::from` needed).
- [ ] **storage impls & sqlx** — `.bind(post_id)` → `.bind(i64::from(post_id))`
      (and `.bind(cursor.post_id)` → `.bind(i64::from(cursor.post_id))`) at
      every post-id bind site; wrap decoded post ids in `build_post_record`
      (`PostId::from` alongside the existing `UserId::from`) and its
      `PostRecordParts` tuple; wrap the `get_tags_for_post` map-closure
      `post_id` and `parse_post_tags_json`'s `post_id: PostId` param; wrap the
      `RETURNING     post_id` `query_scalar::<_, i64>` result with
      `PostId::from`. **Do NOT** touch the `where i64: Encode/Type` bounds
      (still binding `i64`).
- [ ] **storage backend dirs** — `sqlite/posts.rs`, `postgres/posts.rs` dialect
      impls per the edit-map: post-id params, binds, and any post-id decode
      positions.
- [ ] **server** — `atompub/posts.rs` (`post_id` fields/params,
      `Path<(Username, PostId)>` extractors, `soft_delete_post(post.post_id)`,
      cursor `last.post_id`, `post_id_for_idempotency_key`, test literals);
      `feed/regenerate.rs` (`FeedItem { id: p.post_id }`,
      `get_tags_for_post(p.post_id)` — flow automatically once records flip, but
      verify). Route-string literals in `atompub/mod.rs` need no change.
- [ ] **web** — `posts/mod.rs` DTO `.post_id` fields + `#[server]`
      params/returns + `cursor_post_id: Option<PostId>`; `posts/listing.rs`
      (`TimelinePostSummary.post_id`, `TimelinePage.next_cursor_post_id`,
      `cursor_post_id` params); `pages/posts.rs` (route-param parse →
      `.parse::<i64>().ok().map(PostId::from)`; `RwSignal::new(None::<PostId>)`;
      struct-literal `post_id:` sites; hidden `<input value=post_id>` renders
      via `Display`).
- [ ] **tests** — replace i64-literal post-id construction with
      `PostId::from(n)` across in-file `#[cfg(test)]` mods,
      `storage/tests`/`server/tests` helpers, and web test helpers.
      `assert_eq!`/`==` need no edit (derived `PartialEq`).
- **Verify (all must pass):**
  1. `cargo check --all-features --all-targets` green (catches
     `#[cfg(feature=server)]` web code the default check skips — repo gotcha
     #397).
  2. **AC2 completeness** = every Appendix edit-map item struck. A grep over
     touched files (`post_id`, `\.id\b` in feed, `Result<i64`, `-> i64`,
     `(i64,`) is a **supplementary backstop only** — known false-negatives
     (differently named params, rustfmt-wrapped signatures). The struck edit-map
     is the surface, not the grep.
  3. `cargo xtask check` green (static + clippy incl. no new
     `unwrap`/`expect`, + coverage — the new `PostId` code is covered by Task
     1's unit test).
- **Commit:** `refactor: thread PostId through storage/server/web (#472)`.

## Task 3 — Final gate checkpoint (no commit)

_A gate re-run, not a code task._

- [ ] `cargo xtask validate --no-e2e` clean (fmt, clippy, coverage — pre-push
      gate).
- [ ] Confirm **wire invariance** (AC3/AC5): the Task-1 serde test proves
      bare-int encoding; the existing `feed_etag` tests prove the ETag is
      unchanged; spot that no `#[server]`/DTO JSON shape changed (transparent
      bridge guarantees it). e2e is the ship step's job (CI matrix).
- **Verify:** `validate --no-e2e` exit 0; `xtask-done: … ok=true` sentinel.

---

## Coverage of spec ACs

| AC                                            | Task                                |
| --------------------------------------------- | ----------------------------------- |
| AC1 (type exists, derives, no FromStr/no Ord) | Task 1                              |
| AC2 (every post-id site is `PostId`)          | Task 2 verify (edit-map + grep)     |
| AC3 (wire byte-identical)                     | Task 1 serde test + Task 3          |
| AC4 (both backends, no migration)             | Task 2 (backend dirs) + Task 3      |
| AC5 (feed ETag unchanged)                     | Task 2 (metadata) + feed_etag tests |
| AC6 (`validate --no-e2e` clean, e2e pass)     | Task 3 (+ ship e2e)                 |

---

## Appendix — edit-map (AC2 completeness surface)

Line numbers from `issue-472-base`; **re-verify** (siblings shift them).
`.bind(post_id)` → `.bind(i64::from(post_id))`; i64 literals in tests →
`PostId::from(n)`; feed hashing → `i64::from(i.id)`.

### common

- `common/src/ids.rs` — APPEND `PostId` + unit test (`pub mod ids;` already
  declared).
- `common/src/feed/metadata.rs` — `:20` `FeedItem.id: PostId` · `:45`
  `last_id = items.last().map_or(0, |i| i64::from(i.id))` · `:50`
  `.to_le_bytes()` now on the `i64` `last_id` (no change once `:45` converts) ·
  `:66` `item(id: PostId, …)` helper · `:83,87,94,101,102,117,118` call sites
  `item(PostId::from(1), …)`.

### storage — record/input/cursor structs

- `posts.rs:46` `PostRecord.post_id` · `:113` `PostRevisionRecord.post_id`
  (LEAVE `:111` `revision_id`, #474) · `:181` `PostCursor.post_id` · `:191`
  `CollectionCursor.post_id` · `:250` `PostTag.post_id`.
- `post_service.rs:129` `RenderedPostUpdate.post_id` · `:264`
  `PostUpdate.post_id` · `:419` `PostCreation` post-id field.

### storage — trait defs (object + Dialect/dispatch)

- `posts.rs:375` `parse_post_cursor(cursor_post_id: Option<PostId>)` (public
  boundary) + its tests `~:2632/:2641/:2646` · `:543`
  `post_id_for_idempotency_key` return · `:586` `soft_delete_post` · `:589`
  `unpublish_post` · `:654` `tag_post` · `:657` `untag_post` · `:660`
  `get_tags_for_post` · `:744` `get_post_audiences` · `:406`
  `apply_post_tag_diff(post_id: PostId)` (public; called from
  `web/src/posts/mod.rs:454`) · Dialect-trait post-id params `:781`
  `update_post`, `:788` `tag_post`, `:794` `untag_post`, `:1914`
  `replace_post_audiences` · `:899` `get_post_by_id` impl · `:991`
  `update_post`.
- **create-post return chain** (all → `PostId`, compiler-forced by the
  `RETURNING post_id` wrap below): `posts.rs:1847` `write_post_in_tx` return ·
  `:532` object-trait `create_post` · `:538` `create_posts` · `:844,:859` impls
  · `post_service.rs:57` `create_rendered_post` return. Call-site locals become
  `PostId` and flow (consumed by `get_post_by_id`, also `PostId`) —
  `storage/src/test_support.rs:635`, `server/tests/storage/mod.rs`
  (`create_post`/ `create_posts` sites incl. `:6423/:6473`
  `create_rendered_post`), in-file tests.

### storage — impls & sqlx (binds/decodes/helpers)

- `helpers.rs:129-144` `PostRecordParts` tuple (post-id position stays `i64` in
  the tuple; wrap on build) · `:177,:204-206` `build_post_record` — add
  `post_id: PostId::from(post_id)` alongside `user_id: UserId::from(user_id)` ·
  `:158,:166-167,:202` `parse_post_tags_json(post_id: PostId)` +
  `PostTag { post_id }`.
- `posts.rs` binds: `:1007,:1020,:1317,:1897,:1926,:1932` `.bind(post_id)` ·
  cursor binds `:1064,:1136,:1203,:1261,:1389,:1485` `.bind(cursor.post_id)` ·
  `:1322-1330` `get_tags_for_post` map closure
  `PostTag { post_id: PostId::from(post_id) }` · `:1864` `RETURNING post_id`
  `query_scalar::<_,i64>` → wrap `PostId::from` (used at `:1886,:1897,:1903`).

### storage — backend dirs

- `sqlite/posts.rs` — post-id params/binds/decodes (verify; `update_post` editor
  is `UserId` already).
- `postgres/posts.rs` — same.

### server

- `atompub/posts.rs:228,:285` `post_id` fields/params · `:255,:313,:469`
  `Path<(Username, PostId)>` (incl. `member_put` at `:469`) · `:233`
  `get_post_by_id(post_id)` · `:288` `get_tags_for_post(post_id)` · `:292,:295`
  `tag_post`/`untag_post` · `:330` `soft_delete_post(post.post_id)` · `:171`
  cursor `last.post_id` · `:405` `post_id_for_idempotency_key`. Test literals in
  the file.
- `atompub/mod.rs` — route-string literals only (no type change).
- `feed/regenerate.rs:138` `get_tags_for_post(p.post_id)` · `:146`
  `FeedItem { id: p.post_id }` (flows once records/FeedItem flip; verify
  compiles).

### web

- `posts/mod.rs`: DTO `.post_id` — `:51` CreatePostResult · `:63`
  UpdatePostResult · `:154` DraftSummary · `:172` PublishPostResult · `:191`
  PostResponse. `#[server]`/internal params: `:368` `get_post_preview` · `:394`
  update-post fn · `:502` `post_audience_selection` · `:571` `publish_post` ·
  `:629` `delete_post` · `:662` `unpublish_post`. Cursor `:377,:526`
  `cursor_post_id: Option<PostId>`. Assignments `:298,:478,:553,:619` flow.
- `posts/listing.rs:36` `TimelinePostSummary.post_id` · `:57`
  `TimelinePage.next_cursor_post_id: Option<PostId>` · `:79,:224` set from
  `c.post_id` · cursor params `:97,:126,:147,:169,:190,:243,:304,:328`.
- `pages/posts.rs:359,:1114` `RwSignal::new(None::<PostId>)` · `:530,:659`
  route-param `params.get("post_id")…parse()` →
  `.parse::<i64>().ok().map(PostId::from)` · `:575,:1024,:1030,:1054` hidden
  `<input value=post_id>` (Display) · struct literals `:217,:549,:586,:696`
  `post_id: fetched.post_id` · `:1049` fn param `post_id: PostId`.

### tests

- In-file `#[cfg(test)]` literals per §storage/§server/§web above.
- `storage/tests`, `server/tests` post-id helper params/literals; web test
  helpers. Replace i64-literal post ids with `PostId::from(n)`;
  `assert_eq!`/`==` unchanged.
