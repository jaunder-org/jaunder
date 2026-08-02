# Batch per-tag write loops Implementation Plan (issue #771)

> **For agentic workers:** Execute this plan task-by-task with `jaunder-iterate`
> (delegating individual tasks to a subagent via `jaunder-dispatch` when
> useful). Steps use checkbox (`- [ ]`) syntax for tracking.

**Spec:** `docs/superpowers/specs/2026-08-01-issue-771-batch-tag-writes.md`
(approved). Decisions are cited as **D1**–**D14**, criteria as **AC1**–**AC13**;
the plan is "how", the spec is "what/why".

**Goal:** Replace three per-tag autocommit write loops with one declarative
`set_post_tags` call that does the whole read-diff-write in a single
transaction, then delete the API that made looping possible.

**Architecture:** Additive first — `set_post_tags` lands beside the existing
primitives with its own tests (Task 1), then the four production sites move onto
it and the two apply-helpers die (Task 2). **ADR-0092 compliance is achieved at
Task 2**; Tasks 3–5 are the bounding fix and the removal of now-dead API.
Sequencing this way keeps every intermediate commit green and independently
reviewable.

**Tech Stack:** Rust, sqlx 0.8 (bundled SQLite + Postgres), rstest/rstest-reuse
dual-backend templates, cargo-nextest, `cargo xtask` gate.

## Review header

**Scope — in:** `set_post_tags` on both dialects; the four production call
sites; deletion of `apply_post_tag_diff`, `apply_categories`, `tag_post`,
`untag_post`, `get_tags_for_post` and two `TaggingError` variants; AtomPub
cap/dedupe + a `TagValidationError → HandlerError` 4xx bridge; conversion of
every test write loop and ~56 test read sites. **Scope — out:** the value of
`MAX_TAGS_PER_POST` (→ #784); the read-only tag listing APIs; #770.

| #   | Task                                                                    | Deliverable                                                                                                   |
| --- | ----------------------------------------------------------------------- | ------------------------------------------------------------------------------------------------------------- |
| 1   | Add `set_post_tags` on both dialects                                    | The batched call exists and is proven: casing, no-op, clear, idempotence, missing/soft-deleted post (AC3–AC7) |
| 2   | Move the four production sites; delete both apply-helpers               | **ADR-0092 compliance** — one write-lock acquisition per mutation (AC1, AC2 partial, AC9)                     |
| 3   | Bound AtomPub: cap + dedupe + 4xx bridge                                | An over-cap entry is rejected instead of driving an unbounded batch (AC8)                                     |
| 4   | Delete the write primitives and dead variants; convert test write loops | Nothing left to loop; docs and the decode gate corrected (AC10, AC11)                                         |
| 5   | Delete `get_tags_for_post`; rewrite ~56 read sites; branch gate         | AC2 complete, AC13                                                                                            |

**Key risks / decisions:**

- **AC4 needs physical row identity, not column values.** `ctid` (Postgres) /
  `rowid` (SQLite). A DELETE+INSERT reproduces `tag_id` and `tag_display`
  exactly, so a column-value assertion would pass the truncate-and-recreate D2
  forbids. **SQLite trap:** no `AUTOINCREMENT` on `post_tags`, so rowid is
  `max(rowid)+1` — the fixture must seed a _second_ post whose tags hold higher
  rowids, or a delete-and-reinsert hands back identical rowids and the test
  still passes wrongly.
- **The decode gate breaks the moment `tag_post` goes**
  (`xtask/src/steps/sqlx_newtype_decode_check.rs:597-614`, stale entries are a
  hard failure at `:1557`). The two entries are handled asymmetrically: SQLite
  keeps a bool COUNT decode, Postgres's `FOR UPDATE` probe has no bool decode at
  all, so its entry is deleted outright.
- **Task 4 and Task 5 are large mechanical test diffs** (~145 write-loop sites,
  ~56 read sites). They are separate tasks precisely so a reviewer can accept
  the production change without wading through them.
- **Task order is forced:** the primitives cannot be deleted until both
  production (Task 2) and test (Task 4) callers are gone.

## Global Constraints

- **No `Co-Authored-By` trailer** on any commit.
- **Backend parity** (`CONTRIBUTING.md`): any persisted-behaviour change is
  implemented on both backends in the same change; every DB-touching
  `#[tokio::test]` carries `#[apply(backends)]`.
- **ADR-0019**: dialect-specific SQL lives in
  `storage/src/{sqlite,postgres}/posts.rs`; shared SQL and pure logic in
  `storage/src/posts.rs`.
- **ADR-0053 §1**: a dual-backend test must NOT live in a dialect directory.
- **ADR-0021**: no deferred read-then-write upgrades — SQLite opens
  `BEGIN IMMEDIATE`, Postgres locks with `FOR UPDATE`.
- Package names: server crate is **`jaunder`** (`server/Cargo.toml:2`),
  integration target `--test integration`; storage crate is `storage`.
- **Every `cargo nextest` command needs a reachable PostgreSQL**
  (`CONTRIBUTING.md:437-444`). Wrap each:
  `cargo run --manifest-path tools/Cargo.toml -p devtool -- pg run -- <nextest command>`.
  `cargo xtask check`/`validate` need no wrapper.
- Per-commit gate:
  `devtool run --cwd /home/mdorman/src/jaunder/.claude/worktrees/issue-771-batch-tag-writes -- cargo xtask check`
  (**jaunder-commit**). Run `check --no-test` first for fast lint feedback.

---

### Task 1: Add `set_post_tags` on both dialects

**Files:**

- Modify: `storage/src/posts.rs` — shared SQL const + row mapper,
  `PostStorage` + `PostDialect` declarations, generic delegation
- Modify: `storage/src/sqlite/posts.rs`, `storage/src/postgres/posts.rs` — the
  two impls
- Modify: `xtask/src/steps/sqlx_newtype_decode_check.rs` — new ALLOWLIST entry
  for SQLite's exists-check
- Test: `storage/src/posts.rs` (`#[cfg(test)] mod tests`)

**Interfaces:**

- Consumes: existing `post_tag_diff` (`storage/src/posts.rs:314`), `PostTag`,
  `TaggingError`.
- Produces:
  `async fn set_post_tags(&self, post_id: PostId, desired: &[TagLabel]) -> Result<(), TaggingError>`
  on `PostStorage`; the same signature (taking `pool: &Pool<Self>`) on
  `PostDialect`; `pub(crate) const SELECT_POST_TAGS` and
  `pub(crate) fn post_tags_from_rows` in `storage/src/posts.rs`.

- [x] **Step 1: Add the shared read SQL and row mapper**

Both dialects need to read existing tags _on the transaction's connection_, not
the pool, so `get_tags_for_post` (which uses `&self.pool`) cannot be reused. The
SELECT is identical for both dialects, so it is shared per ADR-0019. Add to
`storage/src/posts.rs`:

```rust
/// The post's existing tags, read inside `set_post_tags`' transaction. Identical
/// SQL on both dialects, so it is shared here rather than duplicated per ADR-0019.
/// `ORDER BY` is not needed for the diff (which is set-based) but keeps the read
/// deterministic, matching `PostRecord::tags` (#772).
pub(crate) const SELECT_POST_TAGS: &str = "SELECT pt.post_id, pt.tag_id, t.tag_slug, pt.tag_display
     FROM post_tags pt
     JOIN tags t ON pt.tag_id = t.tag_id
     WHERE pt.post_id = $1
     ORDER BY t.tag_slug";

/// Maps [`SELECT_POST_TAGS`] rows to [`PostTag`]. The row tuple's first two
/// positions are `post_id` and `tag_id` — adjacent ids of the same width — so
/// typing them is what stops a swapped destructuring compiling (ADR-0063 §2).
pub(crate) fn post_tags_from_rows(rows: Vec<(PostId, TagId, Tag, TagLabel)>) -> Vec<PostTag> {
    rows.into_iter()
        .map(|(post_id, tag_id, tag_slug, tag_display)| PostTag {
            post_id,
            tag_id,
            tag_slug,
            tag_display,
        })
        .collect()
}
```

- [x] **Step 2: Declare `set_post_tags` on both traits**

In `storage/src/posts.rs`, add to the `PostStorage` trait (beside `tag_post`,
which stays until Task 4):

```rust
    /// Makes the post's tags equal `desired`, in one transaction (#771, ADR-0092).
    ///
    /// The read, the diff and the writes all happen under a single write-lock
    /// acquisition, so a fan-out of N tags costs one acquisition rather than N.
    /// Tags already present with the same slug are left physically untouched, so
    /// the stored `tag_display` casing is preserved; an unchanged set writes
    /// nothing at all.
    ///
    /// An empty `desired` **clears** the post's tags — it is not a no-op (D11).
    ///
    /// # Errors
    ///
    /// [`TaggingError::PostNotFound`] if the post does not exist. Soft-deleted
    /// posts are tagged normally, matching the previous behaviour (D13).
    async fn set_post_tags(
        &self,
        post_id: PostId,
        desired: &[TagLabel],
    ) -> Result<(), TaggingError>;
```

Add the matching `PostDialect` declaration (beside `tag_post`), taking
`pool: &Pool<Self>`, and the generic `PostStore` delegation with the tracing
instrument:

```rust
    #[tracing::instrument(
        name = "storage.posts.set_post_tags",
        skip(self, desired),
        fields(db.system = DB::DB_SYSTEM, tag_count = desired.len())
    )]
    async fn set_post_tags(
        &self,
        post_id: PostId,
        desired: &[TagLabel],
    ) -> Result<(), TaggingError> {
        DB::set_post_tags(&self.pool, post_id, desired).await
    }
```

- [x] **Step 3: Write the failing behaviour tests**

Add to `storage/src/posts.rs`'s `#[cfg(test)] mod tests`. All
`#[apply(backends)]` (dual-backend; the module already uses this template).

```rust
    #[apply(backends)]
    #[tokio::test]
    async fn set_post_tags_adds_removes_and_clears(#[case] backend: Backend) {
        let env = backend.setup().await;
        let user = SeedUser::new().seed(&env.state).await.user_id;
        let post = SeedRawPost::new(user).seed(&env.state).await.post_id;
        let posts = &*env.state.posts;

        posts
            .set_post_tags(post, &[parse_tag_label("rust"), parse_tag_label("web")])
            .await
            .expect("set initial tags");
        assert_eq!(slugs_of(posts, post).await, vec!["rust", "web"]);

        // Reconcile: "web" drops, "nix" arrives, "rust" stays.
        posts
            .set_post_tags(post, &[parse_tag_label("rust"), parse_tag_label("nix")])
            .await
            .expect("reconcile tags");
        assert_eq!(slugs_of(posts, post).await, vec!["nix", "rust"]);

        // D11: an empty desired set clears, it does not no-op.
        posts.set_post_tags(post, &[]).await.expect("clear tags");
        assert!(slugs_of(posts, post).await.is_empty());
    }

    #[apply(backends)]
    #[tokio::test]
    async fn set_post_tags_preserves_existing_display_casing(#[case] backend: Backend) {
        let env = backend.setup().await;
        let user = SeedUser::new().seed(&env.state).await.user_id;
        let post = SeedRawPost::new(user).seed(&env.state).await.post_id;
        let posts = &*env.state.posts;

        posts
            .set_post_tags(post, &[parse_tag_label("Rust")])
            .await
            .expect("initial casing");
        // Same slug, different casing: the stored row wins (D2).
        posts
            .set_post_tags(post, &[parse_tag_label("rUsT")])
            .await
            .expect("re-apply with new casing");

        let record = posts
            .get_post_by_id(post, &ViewerIdentity::Anonymous)
            .await
            .expect("read post")
            .expect("post exists");
        assert_eq!(record.tags.len(), 1);
        assert_eq!(record.tags[0].tag_display, "Rust");
    }

    #[apply(backends)]
    #[tokio::test]
    async fn set_post_tags_is_idempotent_and_dedupes_input(#[case] backend: Backend) {
        let env = backend.setup().await;
        let user = SeedUser::new().seed(&env.state).await.user_id;
        let post = SeedRawPost::new(user).seed(&env.state).await.post_id;
        let posts = &*env.state.posts;

        let desired = [parse_tag_label("rust"), parse_tag_label("web")];
        posts.set_post_tags(post, &desired).await.expect("first");
        posts.set_post_tags(post, &desired).await.expect("second");
        assert_eq!(slugs_of(posts, post).await, vec!["rust", "web"]);

        // D4: two labels with one slug yield one row, first casing winning —
        // post_tag_diff does not dedupe, the conflict-tolerant insert absorbs it.
        posts
            .set_post_tags(post, &[parse_tag_label("Nix"), parse_tag_label("nix")])
            .await
            .expect("duplicate slug in desired");
        let record = posts
            .get_post_by_id(post, &ViewerIdentity::Anonymous)
            .await
            .expect("read post")
            .expect("post exists");
        assert_eq!(record.tags.len(), 1);
        assert_eq!(record.tags[0].tag_display, "Nix");
    }

    #[apply(backends)]
    #[tokio::test]
    async fn set_post_tags_rejects_missing_post_but_allows_soft_deleted(#[case] backend: Backend) {
        let env = backend.setup().await;
        let posts = &*env.state.posts;

        let err = posts
            .set_post_tags(PostId::from(999_999), &[parse_tag_label("rust")])
            .await
            .expect_err("missing post must be rejected");
        assert!(matches!(err, TaggingError::PostNotFound));

        // D13: soft-deleted posts are still taggable, exactly as before.
        let user = SeedUser::new().seed(&env.state).await.user_id;
        let post = SeedRawPost::new(user).seed(&env.state).await.post_id;
        posts.soft_delete_post(post).await.expect("soft delete");
        posts
            .set_post_tags(post, &[parse_tag_label("rust")])
            .await
            .expect("tagging a soft-deleted post still succeeds");
    }
```

Plus the AC4 test, which needs **physical row identity** — the whole point is to
catch a truncate-and-recreate, which reproduces every column value. Note the
decoy post: SQLite's `post_tags` has no `AUTOINCREMENT`, so rowid is
`max(rowid)+1`; without rows above ours a delete-and-reinsert returns the same
rowids and the test passes wrongly.

```rust
    #[apply(backends)]
    #[tokio::test]
    async fn set_post_tags_with_unchanged_set_writes_nothing(#[case] backend: Backend) {
        let env = backend.setup().await;
        let user = SeedUser::new().seed(&env.state).await.user_id;
        let post = SeedRawPost::new(user).seed(&env.state).await.post_id;
        let posts = &*env.state.posts;

        let desired = [parse_tag_label("rust"), parse_tag_label("web")];
        posts.set_post_tags(post, &desired).await.expect("seed tags");

        // Decoy: its post_tags rows occupy HIGHER sqlite rowids, so a
        // delete-and-reinsert of ours could not coincidentally reuse them.
        let decoy = SeedRawPost::new(user).seed(&env.state).await.post_id;
        posts
            .set_post_tags(decoy, &[parse_tag_label("decoy-a"), parse_tag_label("decoy-b")])
            .await
            .expect("seed decoy");

        let before = physical_row_ids(&env, post).await;
        posts
            .set_post_tags(post, &desired)
            .await
            .expect("re-apply the identical set");
        let after = physical_row_ids(&env, post).await;

        assert_eq!(
            before, after,
            "rows were rewritten; set_post_tags must leave unchanged tags physically untouched (D2/AC4)"
        );
    }
```

The `physical_row_ids` helper is backend-specific (`ctid` vs `rowid`).
**`TestEnv` has no `backend()` method** (`storage/src/test_support.rs:153-156`
is just `state` + `base`), and `TestBase::pool()` (`:203`) returns
`&CloseablePool` — an _enum_, not an sqlx `Pool`, so it cannot be handed to
`fetch_all` directly. Match on the enum instead; `CloseablePool` is already
imported in this test module (`storage/src/posts.rs:2538`):

```rust
    /// Physical row identity for the post's `post_tags` rows: `ctid` on Postgres,
    /// `rowid` on SQLite. Column values cannot serve — a DELETE+INSERT reproduces
    /// them exactly, which is precisely what AC4 must detect.
    async fn physical_row_ids(env: &TestEnv, post_id: PostId) -> Vec<String> {
        match env.base.pool() {
            CloseablePool::Postgres(pool) => sqlx::query_scalar::<_, String>(
                "SELECT ctid::text FROM post_tags WHERE post_id = $1 ORDER BY tag_id",
            )
            .bind(post_id)
            .fetch_all(pool)
            .await,
            CloseablePool::Sqlite(pool) => sqlx::query_scalar::<_, String>(
                "SELECT CAST(rowid AS TEXT) FROM post_tags WHERE post_id = $1 ORDER BY tag_id",
            )
            .bind(post_id)
            .fetch_all(pool)
            .await,
        }
        .expect("read physical row ids")
    }

    /// The post's tag slugs, slug-ordered, read through the normal post read path.
    async fn slugs_of(posts: &dyn PostStorage, post_id: PostId) -> Vec<String> {
        posts
            .get_post_by_id(post_id, &ViewerIdentity::Anonymous)
            .await
            .expect("read post")
            .expect("post exists")
            .tags
            .iter()
            .map(|t| t.tag_slug.to_string())
            .collect()
    }
```

Everything else in this step is verified to compile: `parse_tag_label`,
`SeedUser`, `SeedRawPost`, `Backend` and `CloseablePool` are imported at
`storage/src/posts.rs:2535-2543`; `ViewerIdentity` arrives via `use super::*`;
`env.state.posts` is `Arc<dyn PostStorage>` (`storage/src/app_state.rs:44`) so
`&*…` gives `&dyn PostStorage`; `soft_delete_post` is on the trait
(`posts.rs:616`); `SeedRawPost::new` seeds a **published, Public** post
(`test_support.rs:932-946`), so the anonymous `get_post_by_id` read-backs
resolve; and `TagLabel: PartialEq<&str>` (`macros/src/str_newtype.rs:283-292`)
makes `assert_eq!(…tag_display, "Rust")` compile.

- [x] **Step 4: Run the tests, verify they fail**

```
cargo run --manifest-path tools/Cargo.toml -p devtool -- pg run -- cargo nextest run -p storage set_post_tags
```

Expected: **FAIL to compile** — `set_post_tags` has no implementation yet.

- [x] **Step 5: Implement the SQLite dialect**

`storage/src/sqlite/posts.rs`. Mirrors `tag_post`'s manual-transaction shape
(`:152-158`) because sqlx's `Transaction` issues a deferred `BEGIN`.

**First add the imports** — the file's `use` list (`:5-8`) has none of these, so
the body below will not compile without it. Use the explicit path, not the
`pub use posts::*` glob in `storage/src/lib.rs:69` (whose re-export of
`pub(crate)` items is subtle):

```rust
use crate::posts::{post_tag_diff, post_tags_from_rows, SELECT_POST_TAGS};
```

```rust
    async fn set_post_tags(
        pool: &Pool<Sqlite>,
        post_id: PostId,
        desired: &[TagLabel],
    ) -> Result<(), TaggingError> {
        // ADR-0021: BEGIN IMMEDIATE takes the write lock up front, so the read
        // below is not a shared->reserved upgrade — and the whole read-diff-write
        // is serialized, closing the TOCTOU the old separate read left open
        // (#771 D3). sqlx's Transaction issues a deferred BEGIN, so drive the
        // transaction manually, mirroring update_post / sqlite/backup.rs.
        let mut conn = pool.acquire().await?;
        sqlx::query("BEGIN IMMEDIATE").execute(&mut *conn).await?;

        let result: Result<(), TaggingError> = async {
            let post_exists: bool =
                sqlx::query_scalar("SELECT COUNT(*) > 0 FROM posts WHERE post_id = $1")
                    .bind(post_id)
                    .fetch_one(&mut *conn)
                    .await?;
            if !post_exists {
                return Err(TaggingError::PostNotFound);
            }

            let rows = sqlx::query_as::<_, (PostId, TagId, Tag, TagLabel)>(SELECT_POST_TAGS)
                .bind(post_id)
                .fetch_all(&mut *conn)
                .await?;
            let existing = post_tags_from_rows(rows);
            let diff = post_tag_diff(&existing, desired);

            for label in diff.to_add {
                let slug = label.slug();
                sqlx::query("INSERT OR IGNORE INTO tags (tag_slug) VALUES ($1)")
                    .bind(&slug)
                    .execute(&mut *conn)
                    .await?;
                let tag_id =
                    sqlx::query_scalar::<_, TagId>("SELECT tag_id FROM tags WHERE tag_slug = $1")
                        .bind(&slug)
                        .fetch_one(&mut *conn)
                        .await?;
                // OR IGNORE, not a plain INSERT: `desired` may carry two labels
                // sharing a slug (post_tag_diff does not dedupe), and first
                // casing must win (D4).
                sqlx::query(
                    "INSERT OR IGNORE INTO post_tags (post_id, tag_id, tag_display) VALUES ($1, $2, $3)",
                )
                .bind(post_id)
                .bind(tag_id)
                .bind(label)
                .execute(&mut *conn)
                .await?;
            }

            for slug in diff.to_remove {
                // rows_affected is not checked: the slug came from `existing`,
                // read in this same transaction, so "no row" is not an error (D4).
                sqlx::query(
                    "DELETE FROM post_tags
                     WHERE post_id = $1 AND tag_id = (SELECT tag_id FROM tags WHERE tag_slug = $2)",
                )
                .bind(post_id)
                .bind(slug)
                .execute(&mut *conn)
                .await?;
            }
            Ok(())
        }
        .await;

        match result {
            Ok(()) => {
                sqlx::query("COMMIT").execute(&mut *conn).await?;
                Ok(())
            }
            Err(error) => {
                let _ = sqlx::query("ROLLBACK").execute(&mut *conn).await;
                Err(error)
            }
        }
    }
```

- [x] **Step 6: Implement the Postgres dialect**

`storage/src/postgres/posts.rs`. Uses `FOR UPDATE` as both lock and existence
check, mirroring `update_post` (`:47-56`). Note this leaves **no bool decode** —
relevant to Step 7. Add the same import here (`:5-8` is likewise missing them):

```rust
use crate::posts::{post_tag_diff, post_tags_from_rows, SELECT_POST_TAGS};
```

```rust
    async fn set_post_tags(
        pool: &Pool<Postgres>,
        post_id: PostId,
        desired: &[TagLabel],
    ) -> Result<(), TaggingError> {
        let mut tx = pool.begin().await?;

        // FOR UPDATE locks the post row for the whole read-diff-write, so a
        // concurrent set_post_tags cannot interleave (ADR-0021; mirrors
        // update_post). It doubles as the existence check. No deleted_at filter:
        // soft-deleted posts stay taggable, as before (D13).
        let exists = sqlx::query_scalar::<_, PostId>(
            "SELECT post_id FROM posts WHERE post_id = $1 FOR UPDATE",
        )
        .bind(post_id)
        .fetch_optional(&mut *tx)
        .await?;
        if exists.is_none() {
            tx.rollback().await.ok();
            return Err(TaggingError::PostNotFound);
        }

        let rows = sqlx::query_as::<_, (PostId, TagId, Tag, TagLabel)>(SELECT_POST_TAGS)
            .bind(post_id)
            .fetch_all(&mut *tx)
            .await?;
        let existing = post_tags_from_rows(rows);
        let diff = post_tag_diff(&existing, desired);

        for label in diff.to_add {
            let slug = label.slug();
            sqlx::query("INSERT INTO tags (tag_slug) VALUES ($1) ON CONFLICT DO NOTHING")
                .bind(&slug)
                .execute(&mut *tx)
                .await?;
            let tag_id =
                sqlx::query_scalar::<_, TagId>("SELECT tag_id FROM tags WHERE tag_slug = $1")
                    .bind(&slug)
                    .fetch_one(&mut *tx)
                    .await?;
            // ON CONFLICT DO NOTHING, not a bare INSERT: `desired` may carry two
            // labels sharing a slug, and first casing must win (D4).
            sqlx::query(
                "INSERT INTO post_tags (post_id, tag_id, tag_display) VALUES ($1, $2, $3)
                 ON CONFLICT DO NOTHING",
            )
            .bind(post_id)
            .bind(tag_id)
            .bind(label)
            .execute(&mut *tx)
            .await?;
        }

        for slug in diff.to_remove {
            sqlx::query(
                "DELETE FROM post_tags
                 WHERE post_id = $1 AND tag_id = (SELECT tag_id FROM tags WHERE tag_slug = $2)",
            )
            .bind(post_id)
            .bind(slug)
            .execute(&mut *tx)
            .await?;
        }

        tx.commit().await?;
        Ok(())
    }
```

- [x] **Step 7: Add the two decode-gate ALLOWLIST entries**

`xtask/src/steps/sqlx_newtype_decode_check.rs`. Two new decodes need entries;
without them Step 9's `cargo xtask check` fails. Insert beside the existing
`tag_post` entries (which stay until Task 4 — the gate keys on
`(file, function, target, what)`, so these are distinct keys and raise no
duplicate fault).

First, SQLite's exists-check — a new `bool` COUNT decode. Postgres's
`FOR UPDATE` probe decodes `PostId`, not `bool`, so it needs no entry:

```rust
    Allowed {
        file: "sqlite/posts.rs",
        function: "set_post_tags",
        target: "bool",
        what: "\"SELECTCOUNT(*)>0FROMpostsWHEREpost_id=$1\"",
        count: 1,
        category: Category::CountOrExists,
        reason: "post-exists check before the batched tag write; Postgres uses a FOR UPDATE probe instead",
    },
```

Second — easy to miss — **the AC4 test helper itself is policed.**
`POLICED_ROOT` is `"storage/src"` (`:155`), which covers test modules (hence the
existing `test_support.rs::string_triples`/`scalar_i64` entries at `:859-868`,
`:970-978`), and `String` is not an approved leaf. So `physical_row_ids` needs
its own entry:

```rust
    Allowed {
        file: "posts.rs",
        function: "physical_row_ids",
        target: "String",
        what: <the whitespace-stripped first argument, exactly as the gate renders it>,
        count: 2,
        category: Category::TestScaffolding,
        reason: "AC4 physical row-identity probe (ctid/rowid); column values cannot detect a rewrite",
    },
```

`count: 2` matches the two literal arms written in Step 3. **Run the gate and
use the exact `what`/`count` it reports** rather than hand-deriving the stripped
string — a mismatch reads as a stale entry. Confirm `Category::TestScaffolding`
exists; if the enum spells it differently, use the variant the neighbouring
`test_support.rs` entries use.

- [x] **Step 8: Run the tests, verify they pass**

```
cargo run --manifest-path tools/Cargo.toml -p devtool -- pg run -- cargo nextest run -p storage set_post_tags
```

Expected: **PASS**, all five tests × both backends.

**If `set_post_tags_with_unchanged_set_writes_nothing` fails, do not weaken it**
— it is the criterion that distinguishes D2's diff from a truncate-and-recreate.
A failure means the implementation is rewriting rows.

- [x] **Step 9: Gate and commit**

```
devtool run --cwd /home/mdorman/src/jaunder/.claude/worktrees/issue-771-batch-tag-writes -- cargo xtask check --no-test
devtool run --cwd /home/mdorman/src/jaunder/.claude/worktrees/issue-771-batch-tag-writes -- cargo xtask check
```

```bash
git add storage/src/posts.rs storage/src/sqlite/posts.rs storage/src/postgres/posts.rs xtask/src/steps/sqlx_newtype_decode_check.rs docs/superpowers
git commit -m "feat(storage): add set_post_tags, one transaction per tag mutation (#771)"
```

---

### Task 2: Move the four production sites onto it; delete both apply-helpers

This is where **ADR-0092 compliance lands**.

**Files:**

- Modify: `web/src/posts/api.rs:221-230` (create), `:351-360` (update)
- Modify: `server/src/atompub/posts.rs:292-307` (delete `apply_categories`),
  `:428`, `:530`
- Modify: `storage/src/posts.rs:415-439` (delete `apply_post_tag_diff`), `:314`
  (`post_tag_diff` → `pub(crate)`)
- Test: `server/tests/web/web_posts.rs`, `server/tests/atompub/atompub_posts.rs`
- Regenerate: server-fn coverage evidence

**Interfaces:**

- Consumes: Task 1's `set_post_tags`.
- Produces: no new API. `post_tag_diff` becomes `pub(crate)`;
  `apply_post_tag_diff` and `apply_categories` cease to exist.

- [x] **Step 1: Write the mock-counted call tests (AC1)**

**These go in `web/src/posts/api.rs`'s own `#[cfg(test)] mod tests` (`:780-828`)
— not in `server/tests/web/web_posts.rs`.** That integration file contains
**zero** `MockPostStorage`: its tests (including
`update_post_with_tags_unset_leaves_existing_tags_alone` at `:2255-2306`) are
real-backend `#[apply(backends)]` tests driving the server fn against a live
`AppState`, where a mock cannot be substituted. The `enqueue_many`
`times(1)`/`times(0)` guards this plan cites as the model live at
`web/src/feed_events.rs:53-57`.

The in-crate test module is the only seam where a mock is injectable — it
already does `provide_context(Arc::new(posts) as Arc<dyn PostStorage>)`.
`perform_post_creation` and `perform_post_update` each bottom out at exactly one
storage call (`storage/src/post_service.rs:60`, `:182`), so the mock needs
`create_post`/`update_post`, `get_post_by_id`, `set_post_tags`, plus a
`MockFeedEventStorage`.

```rust
// One set_post_tags call per mutation — the ADR-0092 acquisition-count
// property, pinned the way web/src/feed_events.rs pins enqueue_many.
posts.expect_set_post_tags().times(1).returning(|_, _| Ok(()));

// ...and, in the tags-unset update test, none at all: api.rs:351 only writes
// when `new_tags` is Some.
posts.expect_set_post_tags().times(0);
```

- [x] **Step 2: Run them, verify they fail**

```
cargo nextest run -p web --features server posts::api::server_tests
```

Expected: **FAIL** — production still calls `tag_post`, so `set_post_tags` is
never called (`times(1)` unsatisfied). No PostgreSQL wrapper needed: these are
mock-backed in-crate tests, not DB-touching. **`--features server` is
required**: `server_tests` is `#[cfg(all(test, feature = "server"))]` and
`web`'s default feature set is empty, so a bare `-p web` silently skips the
whole module. Observed RED:
`MockPostStorage::tag_post(…): No matching expectation found` (create) and
`MockPostStorage::get_tags_for_post(…)` (both updates).

- [x] **Step 3: Move the web create site**

`web/src/posts/api.rs:221-230`. The loop and the read-back both go: the slugs
the feed-event fan-out needs are already determined, since `tag_post` stored
exactly `TagLabel::slug()` (D10).

```rust
    posts.set_post_tags(created.post_id, &validated_tags).await?;

    let feed_events = expect_context::<Arc<dyn FeedEventStorage>>();
    // Slugs are known without a read-back: set_post_tags stores exactly
    // TagLabel::slug() for each desired label (#771 D10).
    let tag_slugs: BTreeSet<Tag> = validated_tags.iter().map(TagLabel::slug).collect();
    enqueue_feed_events(feed_events.as_ref(), &auth.username, &tag_slugs)
        .await
        .map_err(InternalError::storage)?;
```

- [x] **Step 4: Move the web update site**

`web/src/posts/api.rs:351-360`. `old_tag_slugs` is already in hand (bound at
`:312` from the `get_post_by_id` at `:309`), so the union needs no second read.

```rust
    let mut all_tag_slugs: BTreeSet<Tag> = old_tag_slugs;
    if let Some(new_tags) = new_tags {
        posts.set_post_tags(post_id, &new_tags).await?;
        // Union old with new so both the vacated and the newly-occupied tag
        // surfaces get regenerated.
        all_tag_slugs.extend(new_tags.iter().map(TagLabel::slug));
    }
```

- [x] **Step 5: Move both AtomPub sites and delete `apply_categories`**

Delete `apply_categories` entirely (`server/src/atompub/posts.rs:292-307`) and
replace both call sites (`:428`, `:530`) with:

```rust
    posts.set_post_tags(post_id, &fields.categories).await?;
```

(at `:428` the id is `created.post_id`). `TaggingError` already converts to
`HandlerError` (`server/src/atompub/mod.rs:226`), so `?` still works.

- [x] **Step 6: Delete `apply_post_tag_diff`; demote `post_tag_diff` and
      `PostTagDiff`**

Delete `storage/src/posts.rs:414-439` (the range starts at the leading doc
comment). Change `pub fn post_tag_diff` (`:314`) to
`pub(crate) fn post_tag_diff` — the dialect impls call it, so it must stay
crate-visible. Demote `pub struct PostTagDiff` (`:295`) to `pub(crate)` as well:
it is re-exported by the `pub use posts::*` glob (`storage/src/lib.rs:69`), and
leaving a public type whose only constructor is crate-private is dead public
surface.

Update `PostTagDiff`'s doc (`:293-294`), which currently claims _"callers
perform the actual `tag_post`/`untag_post` writes with their own error mapping"_
— no longer true:

```rust
/// Borrows from both inputs. Applied by `set_post_tags` inside its transaction;
/// no caller performs the writes itself (#771).
```

Also drop `apply_post_tag_diff` from `web/src/posts/api.rs:52`'s import list.

Three references the plan did not list go dangling the moment those two helpers
die, so they are fixed here rather than left broken for a commit:
`apply_post_tag_diff_adds_then_removes_tags` (`storage/src/posts.rs:3792`) is
**deleted** — its only caller is gone, and Task 1's
`set_post_tags_adds_removes_and_clears` already covers the behaviour (Task 4
Step 2 had scheduled the same deletion); `server/src/atompub/mod.rs:143` names
`apply_categories` in `HandlerError`'s doc; and `common/src/test_support.rs:441`
names `apply_post_tag_diff` (its `tag_post` mention stays for Task 4).

- [x] **Step 7: Run the tests, verify they pass**

```
cargo run --manifest-path tools/Cargo.toml -p devtool -- pg run -- cargo nextest run -p jaunder -p storage
```

Expected: **PASS** — including the feed-event fan-out tests (AC9), which must
show the same surfaces enqueued as before.

- [x] **Step 8: Gate and commit**

**No server-fn coverage regeneration is needed** — an earlier draft of this plan
(and the spec's test-plan item 8) claimed otherwise, wrongly. Those artifacts
are keyed on `#[macros::server]` **fn names** plus e2e **trace/test titles**,
not on body text (`xtask/src/server_fn_coverage/io.rs:33-52`, `:56-66`). This
task adds and removes no server fn and renames no test, so
`docs/coverage/server-fns.json` and its evidence file do not change. (Commit
`cba25194` regenerated because a _test title_ changed — a different trigger.)
Regenerating would require a full `cargo xtask e2e sqlite chromium` capture, and
`cargo xtask check` would not have prompted for it anyway.

```
devtool run --cwd /home/mdorman/src/jaunder/.claude/worktrees/issue-771-batch-tag-writes -- cargo xtask check
```

```bash
git add web/src/posts/api.rs server/src/atompub/posts.rs storage/src/posts.rs
git commit -m "perf(tags): one batched write per tag mutation on all four paths (#771)"
```

---

### Task 3: Bound AtomPub — cap, dedupe, and a 4xx bridge

**Files:**

- Modify: `server/src/atompub/mod.rs` — new
  `From<TagValidationError> for HandlerError`
- Modify: `server/src/atompub/posts.rs` — validate right after
  `entry_to_post_fields`: `:368` (create), `:500` (update)
- Modify: `server/src/atompub/mapping.rs:102` — stale comment naming
  `post_tag_diff`
- Test: `server/tests/atompub/atompub_posts.rs`

**Interfaces:**

- Consumes: `common::tag::parse_and_validate_tags`, `TagValidationError`.
- Produces: `impl From<TagValidationError> for HandlerError` yielding a 4xx.

- [ ] **Step 1: Write the failing bounding tests (AC8)**

`server/tests/atompub/atompub_posts.rs`, `#[apply(backends)]`:

```rust
// Over-cap entry -> 4xx, not an unbounded write (#771 D9, ADR-0092).
// MAX_TAGS_PER_POST + 1 distinct categories.
// -> assert status is 4xx (400), and that the post was NOT created/tagged.

// Duplicate categories are deduped by canonical slug, first casing preserved:
// <category term="Rust"/> + <category term="rust"/> -> one tag, display "Rust".

// A MALFORMED term is still skipped leniently, not rejected (R5 unchanged):
// a valid + an invalid term -> 2xx, one tag.
```

Write these as three real tests with concrete Atom entry bodies, following the
existing entry-construction helpers in that file.

- [ ] **Step 2: Run them, verify they fail**

```
cargo run --manifest-path tools/Cargo.toml -p devtool -- pg run -- cargo nextest run -p jaunder --test integration atompub_posts
```

Expected: **FAIL** — over-cap currently succeeds (no cap), duplicates currently
produce one row only by accident of D4's conflict-tolerance rather than by
dedupe.

- [ ] **Step 3: Add the error bridge**

`TagValidationError` today only bridges to `host::error::InternalError`
(`host/src/error.rs:394`), which AtomPub does not use — so nothing currently
produces AC8's 4xx. Add to `server/src/atompub/mod.rs`, beside the other `From`
impls:

`HandlerError::BadRequest` is a **unit** variant
(`server/src/atompub/mod.rs:152-169`) — it takes no payload, so the error text
is dropped:

```rust
impl From<common::tag::TagValidationError> for HandlerError {
    /// An over-cap or otherwise invalid category set is the client's error, not
    /// an internal one — unlike `TaggingError`, which is always an internal
    /// inconsistency. Bounding this is what keeps the batched tag write capped by
    /// construction (#771, ADR-0092).
    fn from(_: common::tag::TagValidationError) -> Self {
        HandlerError::BadRequest
    }
}
```

No blanket `From` impl exists on `HandlerError` (`mod.rs:195-285`), so this
conflicts with nothing.

- [ ] **Step 4: Validate at the two handlers — _before_ any storage mutation**

`entry_to_post_fields` is deliberately **infallible** (`mapping.rs:83-125`,
contract at `:98-104`), so do **not** make it fallible — validate the categories
it returns.

**Validate immediately after `entry_to_post_fields`, not next to the
`set_post_tags` call.** The `set_post_tags` sites (`:428` create, `:530` update)
sit _after_ `perform_post_creation` (`:387-402`) and `perform_post_update`
(`:505-528`), so validating there would create or mutate the post and _then_
return 400 — contradicting AC8 ("rather than written") and Step 1's own
assertion that the post was not created. The correct points are
`server/src/atompub/posts.rs:368` (create) and `:500` (update):

```rust
    let fields = entry_to_post_fields(&entry, default_format);
    // Bound the tag set before anything is written: an over-cap entry must be
    // rejected, not created-then-rejected (#771 D9/D12, ADR-0092).
    let categories = common::tag::parse_and_validate_tags(fields.categories)?;
```

`fields.categories` moves here; `fields.body`/`fields.summary` are moved later
(`:391`/`:397`, `:510`/`:524`) and partial moves of distinct fields are fine.
The `set_post_tags` call then takes `&categories`.

Also fix the now-stale comment at `mapping.rs:102` referring to
`post_tag_diff`'s filtering.

- [ ] **Step 5: Run the tests, verify they pass**

```
cargo run --manifest-path tools/Cargo.toml -p devtool -- pg run -- cargo nextest run -p jaunder --test integration atompub
```

Expected: **PASS**. Existing AtomPub tests must stay green — none posts more
than three `<category>` elements, so none is over-cap.

- [ ] **Step 6: Gate and commit**

```
devtool run --cwd /home/mdorman/src/jaunder/.claude/worktrees/issue-771-batch-tag-writes -- cargo xtask check
```

```bash
git add server/src/atompub server/tests/atompub
git commit -m "fix(atompub): cap and dedupe ingested categories (#771)"
```

---

### Task 4: Delete the write primitives and dead variants; convert test write loops

**Files:**

- Modify: `storage/src/posts.rs` (trait decls, `PostDialect` decls + doc,
  `TaggingError`, generic delegations, tests)
- Modify: `storage/src/sqlite/posts.rs`, `storage/src/postgres/posts.rs` (delete
  both impls)
- Modify: `xtask/src/steps/sqlx_newtype_decode_check.rs:597-614`
- Modify: `storage/src/test_support.rs:1062-1072`,
  `server/tests/web/web_tags.rs:108,134`, `server/tests/storage/mod.rs` (**~90
  `tag_post` + ~8 `untag_post` sites**),
  `server/tests/atompub/atompub_posts.rs:623`,
  `server/tests/atompub/atompub_service.rs:26`
- Modify: `server/src/atompub/mod.rs:412-426` (trim one assert),
  `docs/adr/0068-tag-identity-label-split.md:50-51`,
  `common/src/test_support.rs:441`

**Interfaces:**

- Consumes: `set_post_tags` (all remaining callers move to it).
- Produces: `tag_post`, `untag_post`, `TaggingError::AlreadyTagged`,
  `TaggingError::TagNotFound` no longer exist.

- [ ] **Step 1: Convert every `tag_post`/`untag_post` call site — loops and
      singles alike**

**Scope warning:** `server/tests/storage/mod.rs` holds roughly **90** `tag_post`
sites and **8** `untag_post` sites, and _most are not loops_ (e.g. `:3122`,
`:3189`, `:3310-3322`, `:3803`, `:3912`, `:4740-4766`, `:5844-5854`). Step 4
deletes the trait methods, so every one of these must move first or the tree
goes red. Convert singles as one-element sets — `set_post_tags(post, &[label])`
— remembering that it is _declarative_: where a test adds a tag to a post that
already has others, the call must list **all** desired tags, not just the new
one. Also convert `server/tests/atompub/atompub_posts.rs:623` and
`server/tests/atompub/atompub_service.rs:26`.

The named loop sites:

- `storage/src/test_support.rs:1062-1072` — `SeedRawPost::create`'s per-tag loop
  becomes one call:

```rust
        let post_id = state.posts.create_post(&input).await?;
        if !tags.is_empty() {
            state
                .posts
                .set_post_tags(post_id, &tags)
                .await
                .expect("seed set_post_tags should succeed");
        }
```

The `is_empty` guard is safe **here** and only here: the post was just created,
so clearing and no-op coincide. Do not generalise it — `set_post_tags(id, &[])`
on an existing post means _clear_ (D11).

- `server/tests/web/web_tags.rs:108-114` and `:134-140` — build the label vec,
  then one call. These deliberately exceed `MAX_TAGS_PER_POST` to exercise
  `list_tags` clamping; they bypass the front-end door and storage does not cap
  (D12), so they convert unchanged:

```rust
    let labels: Vec<TagLabel> = (0..60)
        .map(|n| format!("tag{n:02}").parse().expect("valid tag label"))
        .collect();
    state.posts.set_post_tags(post, &labels).await.unwrap();
```

- `server/tests/storage/mod.rs` — the per-row seed loops (`:3232`, `:3512`,
  `:4311`, `:4357`, `:5758` per the issue; re-locate them, line numbers have
  drifted).

- [ ] **Step 2: Rewrite the three orphaned storage tests**

Re-express against `set_post_tags`: `tag_post_insert_error_returns_internal`
(`storage/src/posts.rs:3330`) — closed-pool error path;
`apply_post_tag_diff_adds_then_removes_tags` (`:3561`) — now redundant with Task
1's add/remove/clear test, so **delete it** rather than duplicate;
`tag_post_round_trips_slug_and_label` (`:3605`) — slug/label round-trip.

- [ ] **Step 3: Delete the tests pinning removed behaviour**

These pinned the strictness D4 removes. Delete each, naming in the commit
message the behaviour that no longer exists and that idempotence (Task 1) is its
replacement: `retag_same_post_with_same_tag_fails`
(`server/tests/storage/mod.rs:3394`), `duplicate_tag_error` (`:3940`),
`tag_post_multiple_attempts` (`:4433`), `untag_nonexistent_post` (`:3428`),
`untag_nonexistent_tag_error` (`:4142`), `get_tags_nonexistent_post` (`:3439`);
and the `TaggingError` Display/Debug unit tests at `storage/src/posts.rs:2726`,
`:2732`, `:2738` (trim to the surviving variants rather than deleting
wholesale).

**Trim, do not delete, the AtomPub status test.**
`server/src/atompub/mod.rs:412-426` is a single test —
`storage_and_document_errors_map_to_status` — whose three assertions cover
`sqlx::Error`, `AtomError` and `TaggingError::AlreadyTagged`. Only the third
(`:422-425`) is orphaned; deleting the whole test would drop two unrelated,
still-valid assertions.

- [ ] **Step 4: Delete the primitives and the dead variants**

Remove `tag_post`/`untag_post` from `PostStorage` (`storage/src/posts.rs:684`,
`:687`), `PostDialect` (`:853`, `:861`), both dialect impls
(`sqlite/posts.rs:144-232`, `postgres/posts.rs:134-208`), and the generic
delegations (`:1507`, `:1516`). Remove
`TaggingError::{TagNotFound, AlreadyTagged}` (`:339-344`, including their
leading doc comments). The mockall mock regenerates from the trait — and the
only other `PostStorage` implementor is `PostStore<DB>` (`posts.rs:909`), so
nothing else needs touching.

- [ ] **Step 5: Fix the decode-gate entries**

`xtask/src/steps/sqlx_newtype_decode_check.rs:597-614`. A stale entry is a hard
failure (`:1557`), so both `tag_post` entries must go: **delete** the
`postgres/posts.rs` one outright (its `FOR UPDATE` successor decodes `PostId`,
not `bool`) and **delete** the `sqlite/posts.rs` one too — Task 1 already added
its `set_post_tags` replacement.

- [ ] **Step 6: Fix the docs that name the deleted items (AC11)**

These include **rustdoc intra-doc links**, which the `doc-links` gate checks:

- `storage/src/posts.rs:806-812` — the `PostDialect` rationale links
  `[tag_post][PostDialect::tag_post]` and
  `[untag_post][PostDialect::untag_post]`. This is a **rewrite, not a rename**:
  the `INSERT OR IGNORE` vs `ON CONFLICT` divergence carries over to
  `set_post_tags`, but the `rows_affected`-has-no-generic-form rationale is gone
  (D4 stopped checking it). State that `set_post_tags` is monomorphised for the
  transaction shape (`BEGIN IMMEDIATE` vs `FOR UPDATE`) and the upsert dialect.
- `storage/src/posts.rs:895-896` — "the transaction-bearing and `rows_affected`
  mutations delegate to `PostDialect`".
- `common/src/test_support.rs:441`.
- `docs/adr/0068-tag-identity-label-split.md:50-51` — a live enumeration of
  label-carrying sites naming `tag_post`; add a consequence note pointing at
  `set_post_tags`. ADR-0021 and ADR-0063 mention it only as historical narrative
  — **leave those alone**.

- [ ] **Step 7: Run everything, verify it passes**

```
cargo run --manifest-path tools/Cargo.toml -p devtool -- pg run -- cargo nextest run -p storage -p jaunder
```

Expected: **PASS**. Confirm the names are gone — AC2 says "anywhere in the tree,
tests included", so the search must cover `server/tests/` and `common/` too, not
just the three `src` roots (each must print nothing):

```
rg -n '\btag_post\b|\buntag_post\b' web/src server/src storage/src common/src server/tests || true
rg -n 'AlreadyTagged|TaggingError::TagNotFound' . --glob '!docs/archive/**' --glob '!docs/superpowers/**' --glob '!docs/adr/0021*' --glob '!docs/adr/0063*' || true
```

The two ADR globs are excluded deliberately: D14 leaves ADR-0021 and ADR-0063
alone, since they name the primitives only as historical narrative.

- [ ] **Step 8: Gate and commit**

```
devtool run --cwd /home/mdorman/src/jaunder/.claude/worktrees/issue-771-batch-tag-writes -- cargo xtask check
```

```bash
git add -A
git commit -m "refactor(storage): delete tag_post/untag_post; set_post_tags is the only tag write (#771)"
```

---

### Task 5: Delete `get_tags_for_post`; rewrite the read sites; branch gate

**Files:**

- Modify: `storage/src/posts.rs` (trait decl `:690`, generic impl `:1520-1553`,
  tests `:3580,3594,3622`)
- Modify: `server/tests/storage/mod.rs` (~56 occurrences, ~42 fns),
  `server/tests/web/web_posts.rs:2004,2113,2301`,
  `server/tests/misc/backup_fixture.rs:201`,
  `server/tests/feed/feed_regenerate.rs:237` (a doc comment naming it —
  otherwise Step 3's search still prints)

_(The AtomPub files are **not** in this task: `atompub_posts.rs` and
`atompub_service.rs` contain no `get_tags_for_post` at all — their `tag_post`
writes at `:623` and `:26` belong to Task 4.)_

**Interfaces:**

- Consumes: `get_post_by_id`, which already carries `tags` (slug-ordered since
  #772).
- Produces: `get_tags_for_post` no longer exists.

- [ ] **Step 1: Rewrite every read site**

Mechanical, one shape throughout:

```rust
// before
let tags = posts.get_tags_for_post(post_id).await.expect("get_tags_for_post failed");
// after
let tags = posts
    .get_post_by_id(post_id, &ViewerIdentity::Anonymous)
    .await
    .expect("get_post_by_id failed")
    .expect("post exists")
    .tags;
```

Where the test asserts on a _missing_ post's tags, `get_post_by_id` returns
`None` rather than an empty vec — assert that instead, and note the changed
shape in the commit message. Where a viewer other than anonymous is needed for
visibility, pass it.

- [ ] **Step 2: Delete the method**

Remove from `PostStorage` (`storage/src/posts.rs:690`) and the generic
`PostStore` impl (`:1520-1553`). It is not a `PostDialect` method, so no dialect
files change. The mock regenerates.

- [ ] **Step 3: Run everything, verify it passes**

```
cargo run --manifest-path tools/Cargo.toml -p devtool -- pg run -- cargo nextest run -p storage -p jaunder
rg -n 'get_tags_for_post' . --glob '!docs/archive/**' --glob '!docs/superpowers/**' || true
```

Expected: **PASS**, and the search prints nothing (AC2 complete).

- [ ] **Step 4: Gate and commit**

```
devtool run --cwd /home/mdorman/src/jaunder/.claude/worktrees/issue-771-batch-tag-writes -- cargo xtask check
```

```bash
git add -A
git commit -m "refactor(storage): drop get_tags_for_post; read tags off the post record (#771)"
```

- [ ] **Step 5: Run the branch gate (AC13)**

```
devtool run --cwd /home/mdorman/src/jaunder/.claude/worktrees/issue-771-batch-tag-writes -- cargo xtask validate --no-e2e
```

Expected: **PASS** (`ok: true`). On failure read `.xtask/last-result.json`'s
`steps[]` rather than scraping stdout. Full `validate` with e2e is
**jaunder-ship**'s pre-merge gate.

---

## Spec coverage

| Spec item                                               | Task                                                            |
| ------------------------------------------------------- | --------------------------------------------------------------- |
| D1 `set_post_tags`                                      | 1 (Steps 2, 5, 6)                                               |
| D2 internal diff                                        | 1 (Steps 5–6), pinned by AC4 test (Step 3)                      |
| D3 per-backend serialization                            | 1 (Steps 5–6)                                                   |
| D4 idempotent writes                                    | 1 (Steps 5–6), pinned Step 3                                    |
| D5 delete primitives                                    | 4 (Step 4)                                                      |
| D6 delete dead variants                                 | 4 (Steps 3–4)                                                   |
| D7 delete apply-helpers; `post_tag_diff` → `pub(crate)` | 2 (Step 6)                                                      |
| D8 delete `get_tags_for_post`                           | 5                                                               |
| D9 AtomPub validation at handlers + 4xx bridge          | 3 (Steps 3–4)                                                   |
| D10 web derives slugs                                   | 2 (Steps 3–4)                                                   |
| D11 empty = clear                                       | 1 (Step 3 test), 4 (Step 1 caveat)                              |
| D12 bound by call-graph construction                    | 3 (Step 4 closes the last unbounded door)                       |
| D13 soft-deleted unchanged                              | 1 (Steps 3, 6)                                                  |
| D14 no new ADR                                          | — (ADR-0068 consequence note in 4, Step 6)                      |
| AC1 one acquisition                                     | 2 (Steps 1–2)                                                   |
| AC2 all sites + names gone                              | 2, 4, **5 (Step 3 completes it)**                               |
| AC3 casing                                              | 1 (Step 3)                                                      |
| AC4 no physical writes                                  | 1 (Step 3)                                                      |
| AC5 add/remove/clear                                    | 1 (Step 3)                                                      |
| AC6 idempotent + dup input                              | 1 (Step 3)                                                      |
| AC7 missing / soft-deleted                              | 1 (Step 3)                                                      |
| AC8 AtomPub bounded                                     | 3 (Steps 1, 5)                                                  |
| AC9 feed events                                         | 2 (Step 7)                                                      |
| AC10 decode gate                                        | 1 (Step 7 adds two), 4 (Step 5 removes both `tag_post` entries) |
| AC11 docs + intra-doc links                             | 2 (Step 6), **3 (Step 4, `mapping.rs:102`)**, 4 (Step 6)        |
| AC12 backend parity                                     | 1 (both impls; all tests `#[apply(backends)]`)                  |
| AC13 gate green                                         | 5 (Step 5)                                                      |
