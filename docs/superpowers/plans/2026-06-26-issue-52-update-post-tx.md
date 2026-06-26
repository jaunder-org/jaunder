# Issue #52 — `update_post` SQLite transaction discipline: Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Eliminate the `SQLITE_BUSY`-on-upgrade failure mode in `PostDialect for Sqlite::update_post` by taking the write lock up front with `BEGIN IMMEDIATE`, with no change to observable behavior or backend parity.

**Architecture:** Replace the SQLite implementation's deferred sqlx `Transaction` with a raw pooled connection driven by manual `BEGIN IMMEDIATE`/`COMMIT`/`ROLLBACK` (the now-merged `create_user_with_invite` / `backup.rs` precedent). `replace_post_audiences` already takes `&mut DB::Connection`, so it needs no change. Postgres is unchanged (MVCC-safe).

**Tech Stack:** Rust, `sqlx` (SQLite + Postgres), `tokio`, `rstest`, `cargo xtask` gate.

## Global Constraints

- **Spec:** `docs/superpowers/specs/2026-06-26-issue-52-update-post-tx.md` (authoritative).
- **SQLite-only code change.** Do **not** touch `storage/src/postgres/posts.rs` or the shared `storage/src/posts.rs::replace_post_audiences`. Parity is by behavior, not by line (ADR-0019).
- **Behavior-preserving.** SQL statements, bind order, and the result paths (`NotFound`, `Unauthorized`, success, `Internal`) are unchanged. Existing tests must stay green without modification.
- **No new ADR** — ADR-0021 (on `main`) governs this. No hash / expensive work in the transaction, so ADR-0022 does not apply.
- **No `#[ignore]` tests inside instrumented dialect files** (`storage/src/sqlite/*.rs`).
- **Manual-transaction precedent:** `storage/src/sqlite/mod.rs::create_user_with_invite` (merged #51) and `storage/src/sqlite/backup.rs:17-56`. Follow exactly.
- **Gate:** per-task `cargo xtask check --no-test`; pre-commit `cargo xtask validate --no-e2e`; full `cargo xtask validate` (sqlite + postgres e2e) at ship. Invoke bare via context-mode with the `cd <worktree> &&` prefix (context-mode runs in the MAIN repo otherwise); pass/fail is the exit code.
- **No commits without explicit user approval; no Co-Authored-By trailers.**

---

### Task 1: Reshape `update_post` (SQLite) to `BEGIN IMMEDIATE` + add a wrong-owner auth test

**Files:**
- Modify: `storage/src/sqlite/posts.rs:25-95` (the `update_post` body)
- Modify: `crap-manifest.json` (accept the complexity-only CRAP bump via heal)
- Test (add): `server/tests/storage/storage.rs` — a dual-backend `post_update_by_non_owner_returns_unauthorized`
- Test (existing, unchanged — behavioral guard): `server/tests/storage/storage.rs` `post_update_writes_revision_and_updates_record` (1841), `post_update_not_found_returns_error` (1879), `update_soft_deleted_post` (4954), all `#[apply(backends)]`

**Interfaces:**
- Consumes: `Pool<Sqlite>::acquire`, `sqlx::query`/`query_as`, `crate::posts::replace_post_audiences::<Sqlite>(&mut DB::Connection, …)`, `post_record_from_row`, `UpdatePostError` (already `From<sqlx::Error>`).
- Produces: unchanged trait method
  `async fn update_post(pool: &Pool<Sqlite>, post_id: i64, editor_user_id: i64, input: &UpdatePostInput) -> Result<PostRecord, UpdatePostError>`.

Behavior-preserving refactor: cycle is **green baseline → refactor → still green**, plus one new test that asserts the previously-unasserted wrong-owner `Unauthorized` boundary (and covers that branch).

- [ ] **Step 1: Green baseline.** Confirm the existing update guards pass before changing anything.

Run: `cargo nextest run -E 'test(post_update_writes_revision_and_updates_record) + test(post_update_not_found_returns_error) + test(update_soft_deleted_post)'`
Expected: all PASS (sqlite cases; postgres cases require `JAUNDER_PG_TEST_URL`).

- [ ] **Step 2: Replace the function body.** Edit `storage/src/sqlite/posts.rs` so `update_post` reads exactly as below. The statements/binds are identical to the original; only the transaction control changes (deferred `tx` → raw connection + `BEGIN IMMEDIATE` + explicit `COMMIT`/`ROLLBACK`, body in an `async {}.await` block). Note `replace_post_audiences::<Sqlite>(&mut *conn, …)`.

```rust
    async fn update_post(
        pool: &Pool<Sqlite>,
        post_id: i64,
        editor_user_id: i64,
        input: &UpdatePostInput,
    ) -> Result<PostRecord, UpdatePostError> {
        // ADR-0021: take the write lock up front with BEGIN IMMEDIATE rather than a
        // deferred BEGIN, so the SELECT->INSERT step performs no shared->reserved lock
        // upgrade (the SQLITE_BUSY-on-upgrade failure mode). sqlx's Transaction issues
        // its own deferred BEGIN, so drive the transaction manually on a raw connection,
        // mirroring create_user_with_invite / sqlite/backup.rs.
        let mut conn = pool.acquire().await?;
        sqlx::query("BEGIN IMMEDIATE").execute(&mut *conn).await?;
        let now = Utc::now();

        let result: Result<PostRow, UpdatePostError> = async {
            let existing = sqlx::query_as::<_, (i64, Option<DateTime<Utc>>)>(
                "SELECT user_id, deleted_at FROM posts WHERE post_id = $1",
            )
            .bind(post_id)
            .fetch_optional(&mut *conn)
            .await?;

            match existing {
                None => return Err(UpdatePostError::NotFound),
                Some((owner_id, deleted_at))
                    if owner_id != editor_user_id || deleted_at.is_some() =>
                {
                    return Err(UpdatePostError::Unauthorized);
                }
                Some(_) => {}
            }

            sqlx::query(
                "INSERT INTO post_revisions (post_id, user_id, title, slug, body, format, rendered_html, edited_at)
                 SELECT post_id, user_id, title, slug, body, format, rendered_html, $1
                 FROM posts WHERE post_id = $2",
            )
            .bind(now)
            .bind(post_id)
            .execute(&mut *conn)
            .await?;

            let row = sqlx::query_as::<_, PostRow>(
                "UPDATE posts
                 SET title = $1,
                     slug = CASE WHEN published_at IS NULL THEN $2 ELSE slug END,
                     body = $3,
                     format = $4,
                     rendered_html = $5,
                     published_at = CASE WHEN $6 THEN COALESCE(published_at, $7) ELSE NULL END,
                     updated_at = $8
                 WHERE post_id = $9
                 RETURNING post_id, user_id,
                           (SELECT username FROM users WHERE user_id = posts.user_id) AS username,
                           title, slug, body, format, rendered_html,
                           created_at, updated_at, published_at, deleted_at, summary,
                           COALESCE((SELECT json_group_array(json_object('tag_id', t.tag_id, 'tag_slug', t.tag_slug, 'tag_display', pt.tag_display)) FROM post_tags pt JOIN tags t ON pt.tag_id = t.tag_id WHERE pt.post_id = posts.post_id), '[]') AS tags",
            )
            .bind(&input.title)
            .bind(input.slug.as_str())
            .bind(&input.body)
            .bind(input.format.to_string())
            .bind(&input.rendered_html)
            .bind(input.publish)
            .bind(now)
            .bind(now)
            .bind(post_id)
            .fetch_one(&mut *conn)
            .await?;

            crate::posts::replace_post_audiences::<Sqlite>(&mut *conn, post_id, &input.audiences)
                .await?;

            Ok(row)
        }
        .await;

        match result {
            Ok(row) => {
                sqlx::query("COMMIT").execute(&mut *conn).await?;
                post_record_from_row(row).map_err(UpdatePostError::Internal)
            }
            Err(error) => {
                let _ = sqlx::query("ROLLBACK").execute(&mut *conn).await;
                Err(error)
            }
        }
    }
```

- [ ] **Step 3: Add the wrong-owner auth test.** In `server/tests/storage/storage.rs`, immediately after `post_update_not_found_returns_error` (ends ~line 1903), insert the test below. It asserts the `Unauthorized` boundary that `update_soft_deleted_post` only exercises incidentally, and covers the `owner_id != editor_user_id` branch.

```rust
#[apply(backends)]
#[tokio::test]
async fn post_update_by_non_owner_returns_unauthorized(#[case] backend: Backend) {
    let env = backend.setup().await;
    let state = &env.state;
    let owner = state
        .users
        .create_user(&username("post_owner"), &password("password"), None, false)
        .await
        .expect("owner creation failed");
    let other = state
        .users
        .create_user(&username("other_user"), &password("password"), None, false)
        .await
        .expect("other creation failed");

    let post_id = state
        .posts
        .create_post(&CreatePostInput {
            user_id: owner,
            title: Some("Owned".to_string()),
            slug: "owned".parse().unwrap(),
            body: "Content".to_string(),
            format: PostFormat::Markdown,
            rendered_html: "<p>Content</p>".to_string(),
            published_at: None,
            summary: None,
            audiences: vec![AudienceTarget::Public],
        })
        .await
        .expect("post creation failed");

    let err = state
        .posts
        .update_post(
            post_id,
            other,
            &UpdatePostInput {
                title: Some("Hijacked".to_string()),
                slug: "hijacked".parse().unwrap(),
                body: "Nope".to_string(),
                format: PostFormat::Markdown,
                rendered_html: "<p>Nope</p>".to_string(),
                publish: false,
                summary: None,
                audiences: vec![AudienceTarget::Public],
            },
        )
        .await
        .expect_err("non-owner update must fail");

    assert!(matches!(err, UpdatePostError::Unauthorized));
}
```

If `UpdatePostError` is not already imported in the test module, add it to the `use storage::{…}` import group (alongside `RegisterWithInviteError` etc.).

- [ ] **Step 4: Run the updated + existing tests.** They must pass; the new test proves the explicit `ROLLBACK` leaves no open transaction on the auth-fail path, and the success/NotFound tests prove `COMMIT`/`ROLLBACK` work.

Run: `cargo nextest run -E 'test(post_update) + test(update_soft_deleted_post)'`
Expected: all `post_update_*` (incl. the new `post_update_by_non_owner_returns_unauthorized`) and `update_soft_deleted_post` PASS.

- [ ] **Step 5: Static gate.**

Run: `cargo xtask check --no-test`
Expected: exit 0 (clippy + fmt clean).

- [ ] **Step 6: Coverage gate + accept CRAP baseline.** The manual-transaction restructure adds branches, so `Sqlite::update_post`'s CRAP rises (complexity-only). Clear it the same way as #51: set the baseline entry in `crap-manifest.json` to the computed value to remove the regression, then let `cargo xtask check` regenerate/heal the full manifest reproducibly from Nix coverage.

Run (verify): `cargo xtask validate --no-e2e`
Expected: exit 0. If it reports a CRAP regression on `Sqlite::update_post`, read the new value (`jq '.coverage.crap.regressions' .xtask/last-result.json`), set that function's `crap`/`cyclomatic` baseline in `crap-manifest.json` to it, run `cargo xtask check` (heals the manifest), then re-run `cargo xtask validate --no-e2e` to confirm clean. If a `new-uncovered` line appears, add a focused dual-backend test for that arm rather than baseline it.

- [ ] **Step 7: Commit** (hold for user approval at the review gate).

```bash
git add storage/src/sqlite/posts.rs server/tests/storage/storage.rs crap-manifest.json
git commit -m "fix(storage/sqlite): take write lock up front in update_post (#52)

Replace the deferred read-then-write transaction with BEGIN IMMEDIATE on a raw
connection (manual COMMIT/ROLLBACK), eliminating the SQLITE_BUSY-on-upgrade
failure mode per ADR-0021. Behavior unchanged; replace_post_audiences is reused
as-is (it already takes &mut DB::Connection). Add a dual-backend wrong-owner
Unauthorized test and accept update_post's complexity-only CRAP bump. Postgres
untouched."
```

---

### Task 2: Ship gate

- [ ] **Step 1: Full pre-PR gate.**

Run: `cargo xtask validate --no-e2e` (full `cargo xtask validate` with e2e runs at `jaunder-ship`).
Expected: exit 0 (static + clippy + coverage clean).

No follow-up issues or ADRs emerged from this cycle (the `replace_post_audiences` connection signature made it a clean mechanical change).

---

## Self-Review

**Spec coverage:**
- `BEGIN IMMEDIATE` reshape, `replace_post_audiences` unchanged, auth-fail → `return Err` + explicit ROLLBACK, row returned then converted post-COMMIT → Task 1 Step 2. ✓
- No new ADR / no ADR-0022 → Global Constraints. ✓
- Backend parity (Postgres untouched; dual-backend `post_update_*` guards) → Global Constraints + Task 1 Steps 1,4. ✓
- CRAP bump accepted; new-uncovered → add test → Task 1 Step 6. ✓
- New wrong-owner `Unauthorized` test → Task 1 Step 3. ✓

**Placeholder scan:** none — full function body and full test code inline. (Step 6's contingency names the exact `jq`/manifest mechanism, not a vague "handle it".)

**Type consistency:** the function signature, `UpdatePostError` variants (`NotFound`/`Unauthorized`/`Internal`), `PostRow`, `post_record_from_row`, and `replace_post_audiences::<Sqlite>(&mut *conn, …)` match `storage/src/sqlite/posts.rs` and `storage/src/posts.rs:1387`; the `async {}.await` block is annotated `Result<PostRow, UpdatePostError>`.
