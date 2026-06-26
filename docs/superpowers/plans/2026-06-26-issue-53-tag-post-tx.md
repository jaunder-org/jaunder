# Issue #53 — `tag_post` SQLite transaction discipline: Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Eliminate the `SQLITE_BUSY`-on-upgrade failure mode in `PostDialect for Sqlite::tag_post` by taking the write lock up front with `BEGIN IMMEDIATE`, with no change to observable behavior or backend parity.

**Architecture:** Replace the SQLite implementation's deferred sqlx `Transaction` with a raw pooled connection driven by manual `BEGIN IMMEDIATE`/`COMMIT`/`ROLLBACK` (the merged `create_user_with_invite` #51 / `update_post` #52 / `backup.rs` precedent). No SQL changes. Postgres is unchanged (MVCC-safe).

**Tech Stack:** Rust, `sqlx` (SQLite + Postgres), `tokio`, `rstest`, `cargo xtask` gate.

## Global Constraints

- **Spec:** `docs/superpowers/specs/2026-06-26-issue-53-tag-post-tx.md` (authoritative).
- **SQLite-only code change.** Do **not** touch `storage/src/postgres/posts.rs`. Parity is by behavior, not by line (ADR-0019).
- **Behavior-preserving.** SQL statements, bind order, and the result paths (`PostNotFound`, `AlreadyTagged`, success, `Internal`) are unchanged. Existing tests must stay green without modification.
- **No new ADR** — ADR-0021 (on `main`) governs. No hash / expensive work in the transaction, so ADR-0022 does not apply.
- **No `#[ignore]` tests inside instrumented dialect files** — the new in-file test is a plain `#[tokio::test]` (the file already has such tests; they run in the SQLite coverage pass).
- **Manual-transaction precedent:** `storage/src/sqlite/posts.rs::update_post` (merged #52), `storage/src/sqlite/mod.rs::create_user_with_invite` (merged #51), `storage/src/sqlite/backup.rs:17-56`. Follow exactly.
- **Gate:** per-task `cargo xtask check --no-test`; pre-commit `cargo xtask validate --no-e2e`; full `cargo xtask validate` (sqlite + postgres e2e) at ship. Invoke bare via context-mode with the `cd <worktree> &&` prefix; pass/fail is the exit code.
- **Local test runs:** the `#[apply(backends)]` tests' `case_2_postgres` cases fail locally with `Connection refused` (no local Postgres) — filter to sqlite (`& test(sqlite)`); the Nix coverage/e2e gate exercises Postgres.
- **No commits without explicit user approval; no Co-Authored-By trailers.**

---

### Task 1: Reshape `tag_post` (SQLite) to `BEGIN IMMEDIATE` + add an Internal-arm test

**Files:**
- Modify: `storage/src/sqlite/posts.rs:115-170` (the `tag_post` body, from `let mut tx = …` onward; the leading `let tag: Tag = …` parse stays before the transaction)
- Modify: `storage/src/sqlite/posts.rs` (the in-file `#[cfg(test)] mod tests`, ~line 195) — add `tag_post_insert_error_returns_internal`
- Modify: `crap-manifest.json` (accept the complexity-only CRAP bump via heal)
- Test (existing, unchanged — behavioral guard): `server/tests/storage/storage.rs` `tag_post_nonexistent_post_error` (`PostNotFound`), `retag_same_post_with_same_tag_fails` (`AlreadyTagged`), and the many `#[apply(backends)]` tagging success cases

**Interfaces:**
- Consumes: `Pool<Sqlite>::acquire`, `sqlx::query`/`query_scalar`, `TaggingError` (already `From<sqlx::Error>`), `Tag::parse`.
- Produces: unchanged trait method
  `async fn tag_post(pool: &Pool<Sqlite>, post_id: i64, tag_display: &str) -> Result<(), TaggingError>`.

- [ ] **Step 1: Green baseline.** Confirm the existing tag guards pass (sqlite cases).

Run: `cargo nextest run -E '(test(tag_post) | test(retag_same_post_with_same_tag_fails) | test(simple_tag_lifecycle)) & test(sqlite)'`
Expected: PASS (sqlite cases).

- [ ] **Step 2: Replace the function body.** Edit `storage/src/sqlite/posts.rs` so `tag_post` reads exactly as below (the `let tag: Tag = …` parse is unchanged; replace from `let mut tx = pool.begin().await?;` through the closing brace). Statements/binds identical; only transaction control changes.

```rust
    async fn tag_post(
        pool: &Pool<Sqlite>,
        post_id: i64,
        tag_display: &str,
    ) -> Result<(), TaggingError> {
        let tag: Tag = tag_display.parse().map_err(|_| {
            TaggingError::Internal(sqlx::Error::Decode("invalid tag format".into()))
        })?;

        // ADR-0021: take the write lock up front with BEGIN IMMEDIATE rather than a
        // deferred BEGIN, so the SELECT->INSERT step performs no shared->reserved lock
        // upgrade (the SQLITE_BUSY-on-upgrade failure mode). sqlx's Transaction issues
        // its own deferred BEGIN, so drive the transaction manually on a raw connection,
        // mirroring update_post / create_user_with_invite / sqlite/backup.rs.
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

            sqlx::query("INSERT OR IGNORE INTO tags (tag_slug) VALUES ($1)")
                .bind(tag.as_str())
                .execute(&mut *conn)
                .await?;

            let tag_id: i64 =
                sqlx::query_scalar::<_, i64>("SELECT tag_id FROM tags WHERE tag_slug = $1")
                    .bind(tag.as_str())
                    .fetch_one(&mut *conn)
                    .await?;

            match sqlx::query(
                "INSERT INTO post_tags (post_id, tag_id, tag_display) VALUES ($1, $2, $3)",
            )
            .bind(post_id)
            .bind(tag_id)
            .bind(tag_display)
            .execute(&mut *conn)
            .await
            {
                Ok(_) => Ok(()),
                Err(sqlx::Error::Database(e)) if e.is_unique_violation() => {
                    Err(TaggingError::AlreadyTagged)
                }
                Err(e) => Err(TaggingError::Internal(e)),
            }
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

- [ ] **Step 3: Add the Internal-arm test.** In the in-file `#[cfg(test)] mod tests` of `storage/src/sqlite/posts.rs`, add this test (after the existing `*_with_closed_pool_returns_error` tests). It forces a non-unique DB error on the `post_tags` insert (the existence check and tag insert still succeed), covering the catch-all `Internal` arm and the `BEGIN IMMEDIATE` rollback path.

```rust
    #[tokio::test]
    async fn tag_post_insert_error_returns_internal() {
        let pool = sqlite_pool().await;
        sqlx::query(
            "INSERT INTO users (username, password_hash, created_at, is_operator) VALUES (?, ?, ?, ?)",
        )
        .bind("tagger")
        .bind("hash")
        .bind(Utc::now())
        .bind(false)
        .execute(&pool)
        .await
        .unwrap();
        let storage = SqlitePostStorage::new(pool.clone());
        let post_id = storage
            .create_post(&crate::CreatePostInput {
                user_id: 1,
                title: Some("Post".to_string()),
                slug: "post".parse().unwrap(),
                body: "body".to_string(),
                format: crate::PostFormat::Markdown,
                rendered_html: "<p>body</p>".to_string(),
                published_at: None,
                summary: None,
                audiences: vec![common::visibility::AudienceTarget::Public],
            })
            .await
            .unwrap();

        // Break the post_tags INSERT (but not the existence check or tag insert) so it
        // returns a non-unique Database error: exercises the catch-all Internal arm and
        // the BEGIN IMMEDIATE rollback path on an unexpected failure.
        sqlx::query("ALTER TABLE post_tags RENAME COLUMN tag_display TO tag_display_x")
            .execute(&pool)
            .await
            .unwrap();

        let result = storage.tag_post(post_id, "rust").await;
        assert!(matches!(result, Err(TaggingError::Internal(_))));
    }
```

If `TaggingError` is not in scope via `use super::*`, add `use crate::TaggingError;` to the test module.

- [ ] **Step 4: Run the updated + existing tests (sqlite).**

Run: `cargo nextest run -E '(test(tag_post) | test(retag_same_post_with_same_tag_fails) | test(simple_tag_lifecycle)) & test(sqlite)'`
Expected: all PASS, including `tag_post_insert_error_returns_internal`.

- [ ] **Step 5: Static gate.**

Run: `cargo xtask check --no-test`
Expected: exit 0 (clippy + fmt clean).

- [ ] **Step 6: Coverage gate + accept CRAP baseline.** The manual-transaction restructure adds branches, so `Sqlite::tag_post`'s CRAP rises (complexity-only). Clear it as in #51/#52: read the computed value (`jq '.coverage.crap.regressions' .xtask/last-result.json`), set that function's `crap`/`cyclomatic` baseline in `crap-manifest.json` to it, run `cargo xtask check` (heals the manifest reproducibly from Nix coverage), then re-run `cargo xtask validate --no-e2e` to confirm clean.

Run (verify): `cargo xtask validate --no-e2e`
Expected: exit 0 after the baseline is accepted. With the Internal-arm test added, expect **0 new-uncovered**; if a new-uncovered line still appears, add a focused test for it rather than baseline it.

- [ ] **Step 7: Commit** (hold for user approval at the review gate).

```bash
git add storage/src/sqlite/posts.rs crap-manifest.json
git commit -m "fix(storage/sqlite): take write lock up front in tag_post (#53)

Replace the deferred read-then-write transaction with BEGIN IMMEDIATE on a raw
connection (manual COMMIT/ROLLBACK), eliminating the SQLITE_BUSY-on-upgrade
failure mode per ADR-0021. Behavior unchanged; no SQL changes. Add an in-file
test for the post_tags INSERT-error (Internal) path and accept tag_post's
complexity-only CRAP bump. Postgres untouched."
```

---

### Task 2: Ship gate

- [ ] **Step 1: Full pre-PR gate.**

Run: `cargo xtask validate --no-e2e` (full `cargo xtask validate` with e2e runs at `jaunder-ship`).
Expected: exit 0 (static + clippy + coverage clean).

No follow-up issues or ADRs emerged from this cycle (clean mechanical change; the last of the three ADR-0021 SQLite follow-ups).

---

## Self-Review

**Spec coverage:**
- `BEGIN IMMEDIATE` reshape, no SQL changes, inline final-insert match folded into the block, explicit ROLLBACK → Task 1 Step 2. ✓
- No new ADR / no ADR-0022 → Global Constraints. ✓
- Backend parity (Postgres untouched; dual-backend tag tests guard) → Global Constraints + Task 1 Steps 1,4. ✓
- CRAP bump accepted → Task 1 Step 6. ✓
- Internal-arm new-uncovered handled proactively (in-file SQLite test) → Task 1 Step 3. ✓

**Placeholder scan:** none — full function body and full test code inline.

**Type consistency:** the function signature, `TaggingError` variants (`PostNotFound`/`AlreadyTagged`/`Internal`), `Tag`, `SqlitePostStorage`, `create_post`/`CreatePostInput`, and `sqlite_pool` match `storage/src/sqlite/posts.rs` and its in-file test module; the `async {}.await` block is annotated `Result<(), TaggingError>`.
