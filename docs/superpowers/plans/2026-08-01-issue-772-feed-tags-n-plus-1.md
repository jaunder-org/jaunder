# Feed tag N+1 Implementation Plan (issue #772)

> **For agentic workers:** Execute this plan task-by-task with `jaunder-iterate`
> (delegating individual tasks to a subagent via `jaunder-dispatch` when
> useful). Steps use checkbox (`- [ ]`) syntax for tracking.

**Spec:** `docs/superpowers/specs/2026-08-01-issue-772-feed-tags-n-plus-1.md`
(approved). The plan is "how"; the spec is "what/why" — decisions are cited as
**D1**–**D8** and criteria as **AC1**–**AC7**, not restated.

**Goal:** Stop feed regeneration re-reading tags per post; read them off the
records the listing query already returned, and pin tag ordering so that
switching sources cannot silently reorder feed bodies.

**Architecture:** Two independent changes. First pin slug-ordering inside
`DB::TAGS_SUBQUERY` on both dialects (`COLLATE "C"` on Postgres so the two
agree), making slug order a documented property of `PostRecord.tags`. Then
delete the per-post `get_tags_for_post` loop in `build_feed_items` and map
`p.tags`, which drops the function's `PostStorage` parameter, `async`, and
`Result` — making storage unreachable from it by compilation.

**Tech Stack:** Rust, sqlx 0.8 (bundled SQLite 3.46 + Postgres),
rstest/rstest-reuse dual-backend templates, cargo-nextest, `cargo xtask` gate.

## Review header

**Scope — in:** the two `TAGS_SUBQUERY` constants; `build_feed_items`; the
ordering contract's doc comments; four tests (two strengthened, two new).
**Scope — out:** removing `get_tags_for_post` and batching per-tag writes (both
#771, D7 — already recorded as a scope addition on that issue, so no filing task
here); `feed_etag`'s hashed field set.

| #   | Task                                                                | Deliverable                                                                                       |
| --- | ------------------------------------------------------------------- | ------------------------------------------------------------------------------------------------- |
| 1   | Pin slug ordering in both dialect constants + document the contract | `PostRecord.tags` is slug-ordered on both backends, provably and documented (AC3, AC4, AC5, AC6)  |
| 2   | Read tags from `p.tags` in `build_feed_items`                       | Zero tag round-trips per regeneration; signature proves it (AC1, AC2), then the branch gate (AC7) |

**Key risks / decisions:**

- **Task order is load-bearing.** Ordering (Task 1) must land _before_ the
  source switch (Task 2). Reversed, Task 2 leaves an intermediate commit where
  feed tag order is unpinned, and Task 2's own test would go red.
- **`COLLATE "C"` is not decoration** (D3). SQLite sorts BINARY; Postgres uses
  an unpinned cluster locale, and the two measurably disagree on hyphenated
  slugs. Omitting it turns "pin the ordering" into a backend-parity violation.
- **Task 2's test is a characterization test, green before and after — by
  design.** It pins the observable contract across a redundancy-elimination
  refactor. The refactor's real proof is the signature change, which the
  compiler enforces (D6, AC1). Do not manufacture a false red for it.

## Global Constraints

Every task's requirements implicitly include these.

- **No `Co-Authored-By` trailer** on any commit.
- **Backend parity** (`CONTRIBUTING.md` "Backend parity rules"): any change to
  persisted behavior is implemented on both backends in the same change.
- **`test-backend-pattern` guard**: every DB-touching `#[tokio::test]` under
  `server/tests/` and `storage/src/` must carry `#[apply(backends)]` (or a
  documented single-backend template). A pure `#[test]` over constants is not
  DB-touching and is exempt.
- **ADR-0053 §1 "home by what it proves"**: a test proving both dialects agree
  must NOT live in a `sqlite/` or `postgres/` dialect directory.
- **Postgres gets `COLLATE "C"`, SQLite does not** — SQLite's default BINARY
  collation already is byte order.
- Do not switch sqlx off its bundled SQLite; `ORDER BY` inside an aggregate
  needs >= 3.44 and the bundled build supplies 3.46 (D3).
- Per-commit gate:
  `devtool run --cwd /home/mdorman/src/jaunder/.claude/worktrees/issue-772-feed-tags-n-plus-1 -- cargo xtask check`
  must pass before committing (**jaunder-commit**).
- **Every `cargo nextest` command below needs a reachable PostgreSQL** — the
  whole integration suite is backend-parametric, so the postgres cases connect
  to `JAUNDER_PG_TEST_URL` and _fail_ if nothing is listening
  (`CONTRIBUTING.md:437-444`). The bare commands below are written for
  readability; run each through an ephemeral cluster:

  ```bash
  cargo run --manifest-path tools/Cargo.toml -p devtool -- pg run -- <the nextest command>
  ```

  `cargo xtask check` / `validate` need no such wrapper — Nix provisions their
  PostgreSQL.

- Package names: the server crate is **`jaunder`** (`server/Cargo.toml:2`), not
  `server`; its integration target is `--test integration`. The storage crate is
  `storage`.

---

### Task 1: Pin slug ordering in both dialect constants

**Files:**

- Modify: `storage/src/sqlite/posts.rs:15` (`TAGS_SUBQUERY`)
- Modify: `storage/src/postgres/posts.rs:15` (`TAGS_SUBQUERY`)
- Modify: `storage/src/posts.rs:68` (`PostRecord.tags` doc), `:809-811` (trait
  const doc)
- Modify: `server/src/atompub/posts.rs:96` (D8 note)
- Test: `storage/src/posts.rs` (`#[cfg(test)] mod tests`),
  `server/tests/storage/mod.rs:5821`, `server/tests/web/web_posts.rs:1871`

**Interfaces:**

- Consumes: nothing (first task).
- Produces: the invariant later work relies on — `PostRecord.tags: Vec<PostTag>`
  is ordered by `tag_slug` ascending, byte-order, identically on both backends.
  `PostDialect::TAGS_SUBQUERY` remains a `&'static str` const; only its text
  changes.

- [x] **Step 1: Strengthen the two accidental-pass ordering tests**

Both currently tag `"Rust"` then `"web"` — insertion order _coincides_ with slug
order, so the assertion cannot distinguish them (spec, test plan items 1–2).
Swap the seed to reverse-slug order; leave the assertions alone.

In `server/tests/storage/mod.rs`, within `post_record_carries_tags`, replace
**lines 5839-5849** — the stale comment plus the `p1` pair of `tag_post` calls.
Note the statement begins at `:5840` (`state` / `.posts`), not at the
`.tag_post` line; replacing from `:5842` would duplicate `state` / `.posts` and
leave a syntax error. Leave `p2` at `:5850-5854` untouched.

```rust
    // p1: two tags, applied in reverse-slug order so the assertion below tests
    // ordering rather than coinciding with insertion order (#772);
    // p2: one tag; p3: none.
    state
        .posts
        .tag_post(p1, &"web".parse::<TagLabel>().unwrap())
        .await
        .unwrap();
    state
        .posts
        .tag_post(p1, &"Rust".parse::<TagLabel>().unwrap())
        .await
        .unwrap();
```

In `server/tests/web/web_posts.rs`, within
`list_user_posts_carries_tags_per_post`, replace the two `tag_post` calls at
`:1891-1900`:

```rust
    // Applied in reverse-slug order so the slug assertion below tests ordering
    // (#772) rather than coinciding with insertion order.
    state
        .posts
        .tag_post(created.post_id, &"web".parse::<TagLabel>().unwrap())
        .await
        .unwrap();
    state
        .posts
        .tag_post(created.post_id, &"Rust".parse::<TagLabel>().unwrap())
        .await
        .unwrap();
```

- [x] **Step 2: Write the failing dialect-constant test**

Add to the `#[cfg(test)] mod tests` in `storage/src/posts.rs`. A pure const test
— no DB, no async — so a plain `#[test]`, exempt from the `test-backend-pattern`
guard, and homed with the trait declaration rather than in a dialect dir
(ADR-0053 §1).

```rust
    /// Both dialects pin tag ordering, and pin it to the *same* order. The JSON
    /// aggregate has no inherent order, and `PostRecord.tags` promises slug order
    /// (#772). Postgres additionally needs `COLLATE "C"`: its default is the
    /// cluster's locale, which disagrees with SQLite's BINARY on the hyphens and
    /// digits that are in the slug alphabet.
    #[test]
    fn tags_subquery_pins_slug_ordering_on_both_dialects() {
        let sqlite = <sqlx::Sqlite as PostDialect>::TAGS_SUBQUERY;
        let postgres = <sqlx::Postgres as PostDialect>::TAGS_SUBQUERY;
        assert!(
            sqlite.contains("ORDER BY t.tag_slug"),
            "sqlite TAGS_SUBQUERY must order by slug: {sqlite}"
        );
        assert!(
            postgres.contains("ORDER BY t.tag_slug COLLATE \"C\""),
            "postgres TAGS_SUBQUERY must order by slug under C collation: {postgres}"
        );
    }
```

- [x] **Step 3: Run the tests, verify they fail**

```
devtool run --cwd /home/mdorman/src/jaunder/.claude/worktrees/issue-772-feed-tags-n-plus-1 -- cargo nextest run -p storage tags_subquery_pins_slug_ordering_on_both_dialects
devtool run --cwd /home/mdorman/src/jaunder/.claude/worktrees/issue-772-feed-tags-n-plus-1 -- cargo nextest run -p jaunder --test integration post_record_carries_tags
devtool run --cwd /home/mdorman/src/jaunder/.claude/worktrees/issue-772-feed-tags-n-plus-1 -- cargo nextest run -p jaunder --test integration list_user_posts_carries_tags_per_post
```

Expected: **FAIL** on all three.

- The const test fails both asserts — neither constant carries `ORDER BY` yet.
- The two seeded tests fail with `["web", "rust"] != ["rust", "web"]`: without
  `ORDER BY` the aggregate yields the join's scan order — on SQLite,
  `sqlite_autoindex_post_tags_1` over `(post_id, tag_id)`, i.e. `tag_id` order,
  which coincides with insertion order here because both tags are newly created.
  (Verified against real SQLite 3.51 and PostgreSQL 16: both currently return
  `web,rust` and both return `rust,web` after Step 4.)

**If a seeded test passes instead of failing, stop and investigate** — it means
the aggregate is already ordering by something other than insertion, and the
premise that these assertions were vacuous needs rechecking before proceeding.

- [x] **Step 4: Add `ORDER BY` to both constants**

Both constants also get the doc comment AC4 requires at this site (neither
carries one today). `ORDER BY` goes inside the aggregate, after
`json_object(...)` / `json_build_object(...)`.

`storage/src/sqlite/posts.rs:15`:

```rust
    /// `ORDER BY t.tag_slug` is what makes [`PostRecord::tags`] slug-ordered
    /// (#772). SQLite's default text collation is BINARY, so the bare clause is
    /// already byte order — the Postgres twin needs an explicit `COLLATE "C"` to
    /// match. Keep the two in sync; asserted by
    /// `tags_subquery_pins_slug_ordering_on_both_dialects`.
    const TAGS_SUBQUERY: &'static str = "COALESCE((SELECT json_group_array(json_object('tag_id', t.tag_id, 'tag_slug', t.tag_slug, 'tag_display', pt.tag_display) ORDER BY t.tag_slug) FROM post_tags pt JOIN tags t ON pt.tag_id = t.tag_id WHERE pt.post_id = p.post_id), '[]')";
```

`storage/src/postgres/posts.rs:15` — same clause plus `COLLATE "C"` (escaped for
the Rust literal):

```rust
    /// `ORDER BY t.tag_slug COLLATE "C"` is what makes [`PostRecord::tags`]
    /// slug-ordered (#772). The `COLLATE` is load-bearing: Postgres would
    /// otherwise sort under the cluster locale, which disagrees with SQLite's
    /// BINARY on the hyphens and digits in the slug alphabet. Keep in sync with
    /// the SQLite twin; asserted by
    /// `tags_subquery_pins_slug_ordering_on_both_dialects`.
    const TAGS_SUBQUERY: &'static str = "COALESCE((SELECT json_agg(json_build_object('tag_id', t.tag_id, 'tag_slug', t.tag_slug, 'tag_display', pt.tag_display) ORDER BY t.tag_slug COLLATE \"C\") FROM post_tags pt JOIN tags t ON pt.tag_id = t.tag_id WHERE pt.post_id = p.post_id), '[]'::json)::text";
```

The `[`PostRecord::tags`]` intra-doc links resolve — `PostRecord` is in scope in
both dialect files (`storage/src/{sqlite,postgres}/posts.rs:5-6`). The repo sets
no `deny(rustdoc::broken_intra_doc_links)`, so a bad link would warn rather than
fail the gate, but these are correct.

- [x] **Step 5: Document the ordering contract (AC4)**

`storage/src/posts.rs:68` — `PostRecord.tags` is currently the only field on the
struct with no doc comment. Replace the bare field with:

```rust
    /// The post's tags, ordered by `tag_slug` ascending (byte order).
    ///
    /// Populated by the same query that loaded the rest of the row — every post
    /// SELECT projects [`PostDialect::TAGS_SUBQUERY`] — so reading tags off a
    /// `PostRecord` costs no extra round-trip. The ordering is pinned in that
    /// subquery on both backends (#772); do not rely on insertion order.
    pub tags: Vec<PostTag>,
```

`storage/src/posts.rs:809-811` — extend the trait const's doc:

```rust
    /// Correlated JSON tag-aggregation subquery (on `p.post_id`) spelled in
    /// this backend's JSON dialect, yielding a `text` column.
    ///
    /// Both dialects order the aggregate by `t.tag_slug`, which is what makes
    /// [`PostRecord::tags`] slug-ordered (#772). Postgres spells it
    /// `ORDER BY t.tag_slug COLLATE "C"`: its default collation comes from the
    /// cluster locale and disagrees with SQLite's BINARY on the hyphens and
    /// digits in the slug alphabet, so the `COLLATE` is what makes the two
    /// backends agree. Keep the two constants in sync — asserted by
    /// `tags_subquery_pins_slug_ordering_on_both_dialects`.
    const TAGS_SUBQUERY: &'static str;
```

`server/src/atompub/posts.rs:96` — record D8 at the ETag fold:

```rust
    // Tags are folded in iteration order, which `TAGS_SUBQUERY`'s `ORDER BY` now
    // makes deterministic across query plans and backends (#772). Entry ETags for
    // posts tagged before that change shift once; an ETag change costs a re-fetch,
    // never staleness.
```

- [x] **Step 6: Run the tests, verify they pass**

Re-run the three commands from Step 3. Expected: **PASS** on all three.

Then run the wider suites that read tags, to confirm the sweep's "zero breaks"
conclusion (spec, "Order-assertion sweep"):

```
devtool run --cwd /home/mdorman/src/jaunder/.claude/worktrees/issue-772-feed-tags-n-plus-1 -- cargo nextest run -p storage
devtool run --cwd /home/mdorman/src/jaunder/.claude/worktrees/issue-772-feed-tags-n-plus-1 -- cargo nextest run -p jaunder --test integration
```

Expected: **PASS**. Any tag-order failure here is a sweep miss — add it to the
spec's enumeration and fix it in this task.

- [x] **Step 7: Commit**

Run the gate first (**jaunder-commit**):

```
devtool run --cwd /home/mdorman/src/jaunder/.claude/worktrees/issue-772-feed-tags-n-plus-1 -- cargo xtask check
```

```bash
git add storage/src/sqlite/posts.rs storage/src/postgres/posts.rs storage/src/posts.rs server/src/atompub/posts.rs server/tests/storage/mod.rs server/tests/web/web_posts.rs
git commit -m "refactor(storage): pin PostRecord.tags to slug order in both dialects (#772)"
```

---

### Task 2: Read tags from `p.tags` in `build_feed_items`

**Files:**

- Modify: `server/src/feed/regenerate.rs:75` (call site), `:126-160`
  (`build_feed_items`)
- Test: `server/tests/feed/feed_regenerate.rs`

**Interfaces:**

- Consumes: Task 1's invariant — `PostRecord.tags` is slug-ordered — and the
  pre-existing fact that `list_published_in_window` returns it populated.
- Produces:
  `fn build_feed_items(base: &AbsoluteUrl, records: &[PostRecord]) -> Vec<FeedItem>`
  — no `PostStorage` parameter, not `async`, no `Result`.

- [x] **Step 1: Write the feed-body test**

Add to `server/tests/feed/feed_regenerate.rs`. **This test passes before the
change as well as after — deliberately.** It pins the observable contract that
the refactor must preserve; the refactor's own proof is the signature (Step 3).
Do not try to make it red first.

```rust
/// #772: the feed reads tags off the records `list_published_in_window` already
/// returned instead of issuing one `get_tags_for_post` per post. This pins the
/// observable contract that must survive that switch — tags still reach the body,
/// slug-ordered even when written in the opposite order.
///
/// JSON is deliberate. RSS renders no tags at all (`common/src/feed/rss.rs` never
/// reads `item.tags`), and Atom's `<category term=…>` would force a substring-index
/// comparison; JSON Feed's `tags` is an array that takes an exact vector assertion.
#[apply(backends)]
#[tokio::test]
async fn regenerated_json_feed_carries_slug_ordered_tags(#[case] backend: Backend) {
    let TestEnv { state, base: _base } = setup_with_base_url(backend).await;

    let user = SeedUser::new().seed(&state).await;
    // Applied in reverse-slug order: an unordered read would surface "web" first.
    SeedRawPost::new(user.user_id)
        .tags(["web", "Rust"])
        .seed(&state)
        .await;

    let row = regenerate_feed(
        state.site_config.as_ref(),
        state.posts.as_ref(),
        state.feed_cache.as_ref(),
        &fp(&format!("/~{}/feed.json", user.username)),
    )
    .await
    .expect("regenerate json feed");

    let v: serde_json::Value = serde_json::from_str(&row.body).expect("feed body is JSON");
    assert_eq!(
        v["items"].as_array().map(Vec::len),
        Some(1),
        "one published post in the feed: {}",
        row.body
    );
    // Ordered by slug (rust < web); the *display* casing the author supplied is
    // what the feed emits.
    assert_eq!(
        v["items"][0]["tags"],
        serde_json::json!(["Rust", "web"]),
        "tags slug-ordered in the JSON feed body: {}",
        row.body
    );
}
```

- [x] **Step 2: Run the test, verify it passes**

```
devtool run --cwd /home/mdorman/src/jaunder/.claude/worktrees/issue-772-feed-tags-n-plus-1 -- cargo nextest run -p jaunder --test integration regenerated_json_feed_carries_slug_ordered_tags
```

Expected: **PASS** (via the current `get_tags_for_post` path, which sorts by
slug itself). If it fails, the fixture is wrong — fix it here, before touching
`build_feed_items`, so the test is known-meaningful when it guards the refactor.

- [x] **Step 3: Rewrite `build_feed_items` and its call site**

`server/src/feed/regenerate.rs:75` — drop the `posts` argument and the `await?`:

```rust
    let items = build_feed_items(base, &published);
```

`server/src/feed/regenerate.rs:126-160` — replace the whole function. The loop
existed only to issue the per-post read; with tags already on the record it
collapses to a map, and the signature loses everything the storage call
required:

```rust
/// Builds the feed's items from the records the listing query already returned.
///
/// Tags come from [`PostRecord::tags`], which `list_published_in_window` populates
/// from the same query that loaded the rest of the row, slug-ordered (#772) — so
/// this performs **no** storage access at all. That is why it takes no
/// `PostStorage`, is not `async`, and cannot fail: a per-post tag read cannot be
/// reintroduced here without changing the signature.
fn build_feed_items(base: &AbsoluteUrl, records: &[PostRecord]) -> Vec<FeedItem> {
    records
        .iter()
        .map(|p| {
            // list_published_in_window guarantees published_at IS NOT NULL,
            // but we fall back to created_at rather than panic if the
            // invariant is ever violated (matches PostRecord::permalink).
            let published_at = p.published_at.unwrap_or(p.created_at);
            FeedItem {
                id: p.post_id,
                // FeedItem carries the post's PostTitle unflattened (#470); renderers
                // read it out via Deref/Display at the external-crate boundary.
                title: p.title.clone(),
                // Compose the root-relative permalink to an absolute per-item feed URL
                // (atom Entry.id/link, RSS link/guid, JSON item url) — no relative atom:id
                // (#560, D1). `base` is the required site origin.
                permalink: compose(base, &p.permalink()),
                summary: p.summary.clone(),
                // FeedItem carries the post's RenderedHtml unflattened (#470); the value
                // is already rendered — no from_trusted rebuild, just propagate it.
                content_html: p.rendered_html.clone(),
                published_at,
                updated_at: p.updated_at,
                tags: p.tags.iter().map(|t| t.tag_display.clone()).collect(),
            }
        })
        .collect()
}
```

Leave `regenerate_feed`'s own `posts: &dyn PostStorage` parameter alone — it
still drives `list_published_in_window`.

- [x] **Step 4: Run the tests, verify they pass**

```
devtool run --cwd /home/mdorman/src/jaunder/.claude/worktrees/issue-772-feed-tags-n-plus-1 -- cargo nextest run -p jaunder --test integration regenerate
devtool run --cwd /home/mdorman/src/jaunder/.claude/worktrees/issue-772-feed-tags-n-plus-1 -- cargo nextest run -p jaunder feed
```

Expected: **PASS** — the new test plus the five pre-existing `feed_regenerate`
tests and the in-file mock-store tests in `server/src/feed/regenerate.rs` (AC1,
test plan item 5).

Confirm AC1 directly — this must print nothing (`|| true` because `rg` exits 1
on no-match, which is the success case here):

```
rg -n 'get_tags_for_post' server/src/feed/ || true
```

- [x] **Step 5: Commit**

Run the gate first (**jaunder-commit**):

```
devtool run --cwd /home/mdorman/src/jaunder/.claude/worktrees/issue-772-feed-tags-n-plus-1 -- cargo xtask check
```

```bash
git add server/src/feed/regenerate.rs server/tests/feed/feed_regenerate.rs
git commit -m "perf(feed): read tags off the listing query instead of N+1 per post (#772)"
```

- [ ] **Step 6: Run the branch gate (AC7)**

`cargo xtask check` is the per-commit gate; AC7 additionally requires
`validate`. Run the iterating form now, over both tasks' work:

```
devtool run --cwd /home/mdorman/src/jaunder/.claude/worktrees/issue-772-feed-tags-n-plus-1 -- cargo xtask validate --no-e2e
```

Expected: **PASS** (`ok: true`, exit 0). On failure, read
`.xtask/last-result.json`'s `steps[]` rather than scraping stdout.

The full `cargo xtask validate` (including e2e) is **jaunder-ship**'s pre-merge
gate, not this plan's — see the spec's AC7.

---

## Spec coverage

| Spec item                                    | Task                                                                                                                                                                                    |
| -------------------------------------------- | --------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| D1 read `p.tags`, delete the loop            | 2 (Step 3)                                                                                                                                                                              |
| D2 simplify signature                        | 2 (Step 3)                                                                                                                                                                              |
| D3 `ORDER BY` both dialects + `COLLATE "C"`  | 1 (Step 4)                                                                                                                                                                              |
| D4 documented contract                       | 1 (Step 5)                                                                                                                                                                              |
| D5 `get_tags_for_post` unchanged             | — (no task modifies it; Task 2 Step 4's `rg` confirms only that no _call site_ remains under `server/src/feed/`, which is the weaker check but sufficient — D5 is a no-change decision) |
| D6 no anti-regrowth guard                    | — (deliberate absence; AC1 met by the Task 2 signature)                                                                                                                                 |
| D7 removal belongs to #771                   | — (already recorded on #771; no filing task)                                                                                                                                            |
| D8 AtomPub ETag note                         | 1 (Step 5)                                                                                                                                                                              |
| AC1 zero round-trips                         | 2 (Steps 3–4)                                                                                                                                                                           |
| AC2 JSON feed body, slug-ordered             | 2 (test written at Step 1; **demonstrated at Step 4**, after the switch — Steps 1–2 only establish it is meaningful beforehand)                                                         |
| AC3 `PostRecord.tags` ordered, both backends | 1 (Steps 1, 6)                                                                                                                                                                          |
| AC4 documented at three sites                | 1 (**Steps 4–5** — the two impl constants in Step 4; the `PostRecord.tags` field and the trait const doc in Step 5)                                                                     |
| AC5 both constants carry `ORDER BY`          | 1 (Step 2)                                                                                                                                                                              |
| AC6 backend parity                           | 1 (Step 4, `COLLATE "C"`)                                                                                                                                                               |
| AC7 gate green                               | 2 (Step 6, `validate --no-e2e`); full `validate` is jaunder-ship's pre-merge gate                                                                                                       |
