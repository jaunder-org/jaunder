# Plan — #473: `TagId` newtype

- Spec:
  [2026-07-16-issue-473-tag-id.md](../specs/2026-07-16-issue-473-tag-id.md)
- Issue: [#473](https://github.com/jaunder-org/jaunder/issues/473)

## Commit strategy (two commits, per precedent)

- **Commit 1 — define** `common::ids::TagId` + unit test. Unused → green.
- **Commit 2 — thread** through storage + tests. Ripples across crates → atomic.
  Lean on `cargo check --all-features --all-targets` **and full
  `cargo xtask check`** (both-backend tests; wasm-clippy is a no-op here — no
  wasm sites — but run the full gate anyway).

## Task 1 — Define `TagId`

- [ ] Append `pub struct TagId(i64)` (doc-commented, derives) to
      `common/src/ids.rs`.
- [ ] Unit test (mirror `ChannelId`).
- **Verify:** `cargo test -p common ids::`; commit
  `refactor(common): add TagId id newtype (#473)`.

## Task 2 — Thread `TagId`

Edit-map (lines from `wt-base-issue-473`; verify before editing):

- [ ] **storage/helpers.rs** — `PostTagJson.tag_id: TagId` (:154) — the
      `#[derive(Deserialize)]` struct; the transparent bridge reads the JSON
      integer, so `parse_post_tags_json`'s `tag_id: r.tag_id` (:169) is a
      **straight move, no change**. Tests:
      `assert_eq!(     record.tags[0].tag_id, TagId::from(1))` (:684); the
      `{"tag_id": 1, …}` JSON string fixtures (:665/:716) STAY verbatim
      (deserialize into `TagId`).
- [ ] **storage/posts.rs** — `TagRecord.tag_id: TagId` (:243);
      `PostTag.tag_id: TagId` (:251); the
      `(post_id, tag_id, tag_slug, tag_display)` decode map wraps
      `TagId::from(tag_id)` at the `PostTag {…}` construct (~:1333-1342); the
      `SELECT tag_id, tag_slug FROM tags` decode map wraps at the
      `TagRecord {…}` construct (~:1584-1588); test literal
      `tag_id: TagId::from(0)` (~:2245). SQL `JOIN …tag_id…` strings unchanged.
- [ ] **storage/{sqlite,postgres}/posts.rs** — **LEAVE**
      `let tag_id: i64 = query_scalar::<_,     i64>("SELECT tag_id …")` +
      `.bind(tag_id)` (fetch-to-bind transient, per spec). No change.
- [ ] **server/atompub** — `posts.rs:557` `mk_tag` test helper: **keep
      `tag_id: i64` param**, wrap `TagId::from(tag_id)` at the `PostTag {…}`
      construct (:560) — mirrors the same helper's `post_id: i64` param +
      `PostId::from(post_id)` construct, so the six bare-literal callers
      (:585/:625/:626/:646) need **no change**. `mapping.rs:542` synthetic
      `tag_id: TagId::from(i64::try_from(i).expect(...) + 1)` (test).
- [ ] **web/src/feed_events.rs** — test literals `tag_id: TagId::from(1|2)`
      (:48/:54/:60).
- [ ] **server/tests** — verified: **no actionable Rust site** (the only
      `server/tests` hit is a JSON **string** literal in `misc/commands.rs`, not
      a `PostTag`/`TagRecord` construction).
- **Verify:**
  1. `cargo check --all-features --all-targets` green.
  2. AC2 edit-map struck; grep (`tag_id: i64`, `pub tag_id:`) over touched files
     — each remaining hit is a dialect fetch-to-bind transient / SQL string /
     backup column name.
  3. **`cargo xtask check` green** (clippy, coverage, both-backend tests).
- **Commit:** `refactor: thread TagId through storage (#473)`.

## Task 3 — Ship

- [ ] `cargo xtask validate --no-e2e`; cold-blind pre-merge review; rebase; PR
      (Closes #473); green CI; **HALT for merge approval**.

## AC coverage

| AC  | Task                            |
| --- | ------------------------------- |
| AC1 | Task 1                          |
| AC2 | Task 2 (edit-map)               |
| AC3 | Task 1 serde test + Task 3      |
| AC4 | Task 2 (both backends) + Task 3 |
| AC5 | Task 3                          |
