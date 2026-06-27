# Scheduled Publishing (issue #70) Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Let an author schedule a post for a future time (and backdate); the post is publicly invisible until that time, becomes visible at query time instantly, reaches cached feeds within one worker tick (and after a restart that straddles go-live), and the schedule is honored over AtomPub `<published>`.

**Architecture:** Visibility is derived purely from `published_at` (draft `NULL` / scheduled `> now` / live `<= now`). Every public read gains an explicit `now: DateTime<Utc>` parameter and an `AND published_at <= now` clause (mirroring the existing `list_published_in_window` convention). The publish "verb" stops being a bool: storage update takes an explicit optional timestamp; AtomPub reads the wire `<published>`; the web form gains a datetime control. Future-dated go-live reaches cached feeds via a new go-live pass in the feed worker (in-memory `(last_tick, now]` window + a feed-relative startup catch-up); immediate/backdated publishes keep enqueuing their own feed regeneration on the write path.

**Tech Stack:** Rust, `sqlx` (SQLite + Postgres), `axum`, Leptos (`#[server]` fns), `tokio_cron_scheduler`, `chrono`, `rstest` test templates.

## Global Constraints

- **Spec (binding):** `docs/superpowers/specs/2026-06-16-emacs-blogging-frontend-design.md` — section **"Unit A — Scheduled publishing"**. This plan implements that section verbatim, with the two user decisions of 2026-06-26: (a) the epic spec's Unit A is the binding spec (no separate spec file); (b) keep the minimal author "scheduled for &lt;time&gt;" surface in #70.
- **Three states, derived from `published_at` only:** draft = `NULL`; scheduled = `NOT NULL AND > now`; live = `NOT NULL AND <= now`.
- **Backend coverage via one common test, not per-backend duplicates.** Behavior tests use the harness templates in `server/tests/helpers/mod.rs`: `#[apply(backends)] #[tokio::test] async fn name(#[case] backend: Backend)` runs the *same* test against SQLite and Postgres. Do **not** write separate SQLite/Postgres tests — there is a high bar for a backend-specific test, justified only by genuinely divergent behavior (none is expected here). Postgres cases self-skip unless `postgres_testing_enabled()`.
- **Determinism in tests:** never assert via `sleep`/wall-clock races. Inject a fixed `now` through the new `now` parameters and assert both sides of the `<= now` boundary (`now - 1s` present, `now + 1s` absent).
- **Backdating is allowed** and uses the same code path (a past `published_at` = immediately live with that timestamp).
- **Slug freeze stays at schedule time** — the existing `slug = CASE WHEN published_at IS NULL THEN $n ELSE slug END` rule is unchanged. Do not touch it.
- **Storage dialect duplication (ADR-0019):** `update_post` SQL is duplicated verbatim across `storage/src/sqlite/posts.rs` and `storage/src/postgres/posts.rs`; any change there is made in **both** files. Shared read SQL lives once in the generic `PostStore<DB>` impl in `storage/src/posts.rs`.
- **No `Co-Authored-By` trailers** in any commit (overrides the global default).
- **Worktree only / never `main`:** commit on `worktree-issue-70-scheduled-publishing`. Review against the anchor: `git diff wt-base-issue-70..HEAD`.
- **Per-task gate:** `cargo xtask check --no-test` (clippy + fmt) while iterating a task; the commit at the end of each task is taken only after the task's own tests pass. Run `cargo xtask validate --no-e2e` before the final task's commit / before requesting the landing gate. Invoke xtask bare via context-mode, `cd` into the worktree first (context-mode runs against the main repo otherwise).
- **Dialect files keep no in-file `#[cfg(test)]`** (project rule): tests for storage behavior go in `server/tests/storage/storage.rs`.

## Separable concerns surfaced during investigation

None new. The full scheduled-post management UI (a dedicated scheduled list, in-place reschedule, pull-back-to-draft) is already deferred to **#15**. "Unschedule" (clear `published_at` back to draft) falls out for free from Task 4's `PublishUpdate::Unpublish` and needs no separate issue. No new GitHub issues are filed by this plan.

## File map

- **Modify** `storage/migrations/sqlite/00NN_index_posts_published_at.sql` *(new)*, `storage/migrations/postgres/00NN_index_posts_published_at.sql` *(new)* — standalone `published_at` index.
- **Modify** `storage/src/posts.rs` — add `now: DateTime<Utc>` to the 5 public reads + `list_drafts_by_user`; add `AND p.published_at <= $now`; add 2 go-live read methods.
- **Modify** `storage/src/post_service.rs` — `PublishUpdate` enum; `PostUpdate.publish` type change; `perform_post_update` wiring.
- **Modify** `storage/src/sqlite/posts.rs`, `storage/src/postgres/posts.rs` — `update_post` `published_at` CASE + binds.
- **Modify** `server/src/atompub/mapping.rs` — parse `<published>` into `PostFields`.
- **Modify** `server/src/atompub/posts.rs` — create/update honor `<published>` / `PublishUpdate`.
- **Modify** `server/src/feed/worker.rs` — `last_tick` state + `go_live_pass`; **Modify** `server/src/feed/regenerate.rs` only if a shared feed-url builder must be extracted.
- **Modify** `web/src/posts/mod.rs` — `create_post`/`update_post`/`list_drafts` server fns gain scheduling; `DraftSummary` gains scheduled time.
- **Modify** `web/src/pages/posts.rs`, `web/src/pages/ui.rs` — datetime control + "scheduled for" marker.
- **Test** `server/tests/storage/storage.rs`, `server/tests/atompub/atompub_posts.rs`, `server/tests/feed/feed_worker.rs`, `server/tests/web/web_posts.rs`.

---

## Task 1: Standalone `published_at` index

**Files:**
- Create: `storage/migrations/sqlite/00NN_index_posts_published_at.sql` (next number after the highest existing sqlite migration)
- Create: `storage/migrations/postgres/00NN_index_posts_published_at.sql` (same number, postgres dir)
- Test: `server/tests/storage/storage.rs`

**Interfaces:**
- Produces: a `idx_posts_published_at` index supporting `WHERE published_at <= now AND deleted_at IS NULL` range scans used by Tasks 2 and 7.

- [x] **Step 1: Write the failing test** in `server/tests/storage/storage.rs` (asserts the index exists after migrations; one common test, both backends):

```rust
#[apply(backends)]
#[tokio::test]
async fn posts_published_at_index_exists(#[case] backend: Backend) {
    let env = backend.setup().await;
    let names: Vec<String> = match backend {
        Backend::Sqlite => sqlx::query_scalar::<_, String>(
            "SELECT name FROM sqlite_master WHERE type='index' AND name='idx_posts_published_at'",
        )
        .fetch_all(env.sqlite_pool())
        .await
        .unwrap(),
        Backend::Postgres => sqlx::query_scalar::<_, String>(
            "SELECT indexname FROM pg_indexes WHERE indexname='idx_posts_published_at'",
        )
        .fetch_all(env.postgres_pool())
        .await
        .unwrap(),
    };
    assert_eq!(names, vec!["idx_posts_published_at".to_string()]);
}
```

(If `env` exposes pools under different accessors, match the existing helper API in `server/tests/helpers/mod.rs`; this is the one place a backend `match` is legitimate — it asserts a backend-specific catalog, not divergent product behavior.)

- [x] **Step 2: Run it, verify it fails** — `cd <worktree> && cargo nextest run -p jaunder posts_published_at_index_exists` → FAIL (index missing).

- [x] **Step 3: Write the migrations** (`0022_index_posts_published_at.sql`). SQLite (`...sqlite/0022_index_posts_published_at.sql`):

```sql
CREATE INDEX IF NOT EXISTS idx_posts_published_at
    ON posts (published_at)
    WHERE deleted_at IS NULL;
```

Postgres (`...postgres/00NN_index_posts_published_at.sql`):

```sql
CREATE INDEX IF NOT EXISTS idx_posts_published_at
    ON posts (published_at)
    WHERE deleted_at IS NULL;
```

- [x] **Step 4: Run it, verify it passes** — same nextest command → PASS (SQLite locally; Postgres + full coverage via `cargo xtask validate --no-e2e`). NOTE: adding migration 0022 bumped `schema_version_returns_migration_count` in `storage/src/sqlite/backup.rs` (21→22) — update it.

- [x] **Step 5: Commit** — `feat(storage): index posts.published_at for scheduled-publishing reads (#70)`.

---

## Task 2: Unify public-read visibility on `published_at <= now`

**Files:**
- Modify: `storage/src/posts.rs` — trait declarations (lines 308, 339, 349, 385, 395) and generic impls (lines 631, 712, 779, 1000, 1084)
- Modify: all call sites of these 5 methods (web handlers, AtomPub, existing tests — the compiler enumerates them)
- Test: `server/tests/storage/storage.rs`

**Interfaces:**
- Produces — new signatures (add `now: DateTime<Utc>` as the last parameter, matching `list_published_in_window`):
  - `get_post_by_permalink(&self, username, year, month, day, slug, viewer, now: DateTime<Utc>)`
  - `list_published_by_user(&self, username, cursor, limit, viewer, now: DateTime<Utc>)`
  - `list_published(&self, cursor, limit, viewer, now: DateTime<Utc>)`
  - `list_posts_by_tag(&self, tag_slug, cursor, limit, viewer, now: DateTime<Utc>)`
  - `list_user_posts_by_tag(&self, user_id, tag_slug, cursor, limit, viewer, now: DateTime<Utc>)`

Each of these five queries currently contains `AND p.published_at IS NOT NULL` (lines 650, 731/758, 795/820, 1030/1059, 1117/1148). The transformation in every case: keep `IS NOT NULL`, add `AND p.published_at <= $K` (new positional bind for `now`), and add `.bind(now)` in the matching position; renumber later positional binds. **Worked example — `get_post_by_permalink`:** the clause `AND p.published_at IS NOT NULL` becomes `AND p.published_at IS NOT NULL AND p.published_at <= $K`, with `now` bound after `slug`/before/after the viewer binds per that query's existing order. Apply the identical edit to all five.

- [x] **Step 1: Write the failing boundary tests** in `server/tests/storage/storage.rs`. One common test per surface; each seeds a *scheduled* post (`published_at = now + 1h`) and a *live* post (`published_at = now - 1h`) and asserts the boundary. Example for the permalink read (repeat the shape for `list_published_by_user`, `list_published`, `list_posts_by_tag`, `list_user_posts_by_tag`):

```rust
#[apply(backends)]
#[tokio::test]
async fn permalink_hides_scheduled_until_due(#[case] backend: Backend) {
    let env = backend.setup().await;
    let now = Utc.with_ymd_and_hms(2026, 6, 26, 12, 0, 0).unwrap();
    // live post (published_at = now - 1h)
    let live = seed_post_published_at(&env, "alice", "live-one", now - Duration::hours(1)).await;
    // scheduled post (published_at = now + 1h)
    let sched = seed_post_published_at(&env, "alice", "sched-one", now + Duration::hours(1)).await;

    // At `now`: live is visible, scheduled is not.
    let got_live = env.storage().get_post_by_permalink(
        &uname("alice"), 2026, 6, 26, &slug("live-one"), &ViewerIdentity::Anonymous, now,
    ).await.unwrap();
    assert!(got_live.is_some(), "live post must be visible at now");

    let got_sched = env.storage().get_post_by_permalink(
        &uname("alice"), 2026, 6, 26, &slug("sched-one"), &ViewerIdentity::Anonymous, now,
    ).await.unwrap();
    assert!(got_sched.is_none(), "scheduled post must be hidden before its time");

    // One second past the scheduled time: now visible (locks the boundary).
    let after = (now + Duration::hours(1)) + Duration::seconds(1);
    let got_sched_after = env.storage().get_post_by_permalink(
        &uname("alice"), 2026, 6, 26, &slug("sched-one"), &ViewerIdentity::Anonymous, after,
    ).await.unwrap();
    assert!(got_sched_after.is_some(), "scheduled post must appear once now >= published_at");
    let _ = (live, sched);
}
```

Add a small helper `seed_post_published_at(env, username, slug, published_at)` next to the existing `make_published_create_post_input` (storage.rs:1766) that creates a post with an explicit `published_at` via `perform_post_creation` (which already takes `published_at: Option<DateTime<Utc>>`). Reuse `uname`/`slug` helpers if present; otherwise construct `Username`/`Slug` as neighboring tests do.

- [x] **Step 2: Run them, verify they fail** — `cd <worktree> && cargo nextest run -p jaunder hides_scheduled` (and the other test names). Expected: FAIL to compile first (new `now` arg) — that compile failure is the red state — and, once you stub the signature, FAIL on the assertion (scheduled post leaks). (Red state guaranteed by the signature change: the tests call the 5 reads with a `now` arg that did not exist before the impl.)

- [x] **Step 3: Implement.** In `storage/src/posts.rs`: add `now: DateTime<Utc>` to the five trait declarations and the five impls; in each query string add `AND p.published_at <= $K` next to the existing `IS NOT NULL`; add the matching `.bind(now)` and renumber subsequent positional binds. Then update every call site to pass `Utc::now()` (web read handlers, any AtomPub reads, and existing tests). Let the compiler list them.

- [x] **Step 4: Run them, verify they pass** — the same nextest filters → PASS on both backends (SQLite verified locally: 5/5 passed; Postgres via the controller's Nix gate). Then `cd <worktree> && cargo xtask check --no-test` → clean.

- [x] **Step 5: Commit** — `feat(storage): gate public reads on published_at <= now for scheduled posts (#70)`.

---

## Task 3: Author "scheduled" surface (minimal)

**Files:**
- Modify: `storage/src/posts.rs` — `list_drafts_by_user` (trait 357, impl 841)
- Modify: `web/src/posts/mod.rs` — `DraftSummary` (144), `list_drafts` server fn (493)
- Modify: `web/src/pages/posts.rs` — `DraftsPage` (818)
- Test: `server/tests/storage/storage.rs`, `server/tests/web/web_posts.rs`

**Interfaces:**
- Consumes: nothing from Task 2 (independent), but follows the same `now` convention.
- Produces: `list_drafts_by_user(&self, user_id, cursor, limit, now: DateTime<Utc>)` now returns drafts **and** scheduled posts (it is the author's "not-yet-live" surface); `DraftSummary` gains `scheduled_at: Option<String>` (RFC3339 UTC; `None` for true drafts).

Rationale: a scheduled post (`published_at NOT NULL AND > now`) currently falls out of `list_drafts_by_user` (gates `IS NULL`) *and*, after Task 2, out of every public list — it would be invisible to its own author until go-live. Broadening the drafts query to `published_at IS NULL OR published_at > now` gives scheduled posts a home with a marker, satisfying the spec's minimal author surface. Full management UI stays in #15.

- [ ] **Step 1: Write the failing storage test** in `server/tests/storage/storage.rs`:

```rust
#[apply(backends)]
#[tokio::test]
async fn drafts_list_includes_scheduled_excludes_live(#[case] backend: Backend) {
    let env = backend.setup().await;
    let now = Utc.with_ymd_and_hms(2026, 6, 26, 12, 0, 0).unwrap();
    let alice = seed_user(&env, "alice").await; // existing helper / seed_alice equivalent
    let _draft = create_draft(&env, alice, "a-draft").await;                       // published_at NULL
    let _sched = seed_post_published_at(&env, "alice", "a-sched", now + Duration::hours(2)).await;
    let _live  = seed_post_published_at(&env, "alice", "a-live",  now - Duration::hours(2)).await;

    let rows = env.storage()
        .list_drafts_by_user(alice, None, 50, now)
        .await
        .unwrap();
    let slugs: Vec<_> = rows.iter().map(|p| p.slug.as_str().to_string()).collect();
    assert!(slugs.contains(&"a-draft".to_string()), "drafts must include true drafts");
    assert!(slugs.contains(&"a-sched".to_string()), "drafts must include scheduled posts");
    assert!(!slugs.contains(&"a-live".to_string()), "drafts must exclude live posts");
}
```

- [ ] **Step 2: Run it, verify it fails** — `cd <worktree> && cargo nextest run -p jaunder drafts_list_includes_scheduled` → FAIL (compile, then assertion).

- [ ] **Step 3: Implement the query.** In `storage/src/posts.rs` add `now: DateTime<Utc>` to `list_drafts_by_user` (trait + impl) and change the gate from `AND p.published_at IS NULL` to `AND (p.published_at IS NULL OR p.published_at > $K)` in both the cursor and no-cursor branches, binding `now`.

- [ ] **Step 4: Surface the marker through the web layer.** In `web/src/posts/mod.rs`: add `scheduled_at: Option<String>` to `DraftSummary` (144); in `list_drafts` (493) pass `Utc::now()` and set `scheduled_at = post.published_at.map(|t| t.to_rfc3339())` (only populated when `published_at` is in the future — true drafts stay `None`). In `web/src/pages/posts.rs` `DraftsPage` (818), when `scheduled_at` is `Some`, render a "Scheduled for {local time}" badge instead of the draft label (format the RFC3339 string to the viewer's locale with the existing date-rendering helper used elsewhere in that page).

- [ ] **Step 5: Write + run the web server-fn test** in `server/tests/web/web_posts.rs`: create a post scheduled in the future, call `list_drafts`, assert the returned `DraftSummary` has `scheduled_at: Some(_)` and that a live post does not appear. Run: `cd <worktree> && cargo nextest run -p jaunder -E 'test(list_drafts)'` → PASS. Then `cargo xtask check --no-test` → clean.

- [ ] **Step 6: Commit** — `feat(web): show scheduled posts in the author drafts surface with a marker (#70)`.

---

## Task 4: Storage update takes an explicit publish timestamp

**Files:**
- Modify: `storage/src/post_service.rs` — `PublishUpdate` enum (new), `PostUpdate` (121-141), `perform_post_update` (152)
- Modify: `storage/src/sqlite/posts.rs` (`update_post`, ~25), `storage/src/postgres/posts.rs` (`update_post`, ~26)
- Modify: callers — `server/src/atompub/posts.rs:381`, `web/src/posts/mod.rs` (`update_post` 345, `publish_post` 531)
- Test: `server/tests/storage/storage.rs`

**Interfaces:**
- Produces:

```rust
/// What an update does to a post's publication state.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PublishUpdate {
    /// Clear `published_at` back to NULL (draft / unschedule).
    Unpublish,
    /// Publish. `at = Some(t)` sets `published_at = t` (future = scheduled,
    /// past = backdated-live). `at = None` keeps an existing timestamp or
    /// stamps `now` for a previously-unpublished post.
    Publish { at: Option<DateTime<Utc>> },
}
```

  `PostUpdate.publish` changes from `bool` to `PublishUpdate`. `perform_post_update` derives the three SQL inputs and passes them to `update_post`.

- [ ] **Step 1: Write the failing storage tests** in `server/tests/storage/storage.rs` (one common test, both backends):

```rust
#[apply(backends)]
#[tokio::test]
async fn update_publish_timestamp_semantics(#[case] backend: Backend) {
    let env = backend.setup().await;
    let now = Utc.with_ymd_and_hms(2026, 6, 26, 12, 0, 0).unwrap();
    let alice = seed_user(&env, "alice").await;
    let draft = create_draft(&env, alice, "p").await; // published_at NULL

    // Publish { at: Some(future) } on a draft => scheduled.
    let future = now + Duration::days(1);
    let rec = perform_post_update(env.storage(), update_input(&draft, alice,
        PublishUpdate::Publish { at: Some(future) })).await.unwrap();
    assert_eq!(rec.published_at, Some(future), "explicit future timestamp is stored");

    // Publish { at: None } on an already-published post keeps the existing timestamp.
    let rec2 = perform_post_update(env.storage(), update_input(&draft, alice,
        PublishUpdate::Publish { at: None })).await.unwrap();
    assert_eq!(rec2.published_at, Some(future), "publish-without-timestamp keeps existing");

    // Unpublish clears it.
    let rec3 = perform_post_update(env.storage(), update_input(&draft, alice,
        PublishUpdate::Unpublish)).await.unwrap();
    assert_eq!(rec3.published_at, None, "unpublish clears published_at");

    // Publish { at: None } on a never-published draft stamps ~now.
    let draft2 = create_draft(&env, alice, "q").await;
    let rec4 = perform_post_update(env.storage(), update_input(&draft2, alice,
        PublishUpdate::Publish { at: None })).await.unwrap();
    assert!(rec4.published_at.is_some(), "publish-now stamps a timestamp");
}
```

`update_input(...)` is a small local helper building a `PostUpdate` with the given `publish` and otherwise-unchanged fields (mirror `make_*` helpers).

- [ ] **Step 2: Run them, verify they fail** — `cd <worktree> && cargo nextest run -p jaunder update_publish_timestamp_semantics` → FAIL (compile: `PublishUpdate` undefined / `publish` is `bool`).

- [ ] **Step 3: Implement the type + service.** Add `PublishUpdate` to `storage/src/post_service.rs`; change `PostUpdate.publish` to `PublishUpdate`; in `perform_post_update` derive `(unpublish: bool, explicit_at: Option<DateTime<Utc>>)` from it and pass both (plus `now`) into `UpdatePostInput`.

- [ ] **Step 4: Implement the dialect SQL** in **both** `storage/src/sqlite/posts.rs` and `storage/src/postgres/posts.rs`. Replace the `published_at` CASE:

```sql
published_at = CASE
    WHEN $U THEN NULL
    WHEN $E IS NOT NULL THEN $E
    ELSE COALESCE(published_at, $N)
END
```

where `$U` = `unpublish` (bool), `$E` = `explicit_at` (`Option<DateTime<Utc>>`), `$N` = `now`. Read the current bind list in each file, replace the single `publish` bind with the `unpublish` + `explicit_at` binds, keep the `now`/`updated_at` binds, and renumber every positional placeholder consistently. The slug-freeze CASE and the revision insert are unchanged.

- [ ] **Step 5: Update callers (keep the build green).** `server/src/atompub/posts.rs:381` `publish: !fields.is_draft` → `PublishUpdate` (Task 5 refines this; for now `if fields.is_draft { Unpublish } else { Publish { at: None } }`). `web/src/posts/mod.rs` `update_post` (345): `publish: if publish { Publish { at: None } } else { Unpublish }`. `publish_post` (531): `Publish { at: None }`.

- [ ] **Step 6: Run tests + gate** — `cargo nextest run -p jaunder update_publish_timestamp_semantics` → PASS both backends; `cargo xtask check --no-test` → clean.

- [ ] **Step 7: Commit** — `feat(storage): replace update publish bool with explicit publish timestamp (#70)`.

---

## Task 5: AtomPub honors the entry `<published>`

**Files:**
- Modify: `server/src/atompub/mapping.rs` — `PostFields` (13), `entry_to_post_fields` (35)
- Modify: `server/src/atompub/posts.rs` — `collection_post` (260, stamp 274-278), `member_put` (339, publish 381)
- Test: `server/tests/atompub/atompub_posts.rs`

**Interfaces:**
- Consumes: `PublishUpdate` (Task 4).
- Produces: `PostFields` gains `published: Option<DateTime<Utc>>` parsed from the entry's `<published>` element.

Semantics: **create** — draft entry → `published_at = None`; non-draft with `<published>` → that timestamp (future = scheduled, past = backdated); non-draft without `<published>` → `Utc::now()`. **update** — draft entry → `PublishUpdate::Unpublish`; non-draft → `PublishUpdate::Publish { at: fields.published }`.

- [ ] **Step 1: Write the failing tests** in `server/tests/atompub/atompub_posts.rs` (one common test, both backends; use the existing `entry_xml` builder (487) and `make_app`/`basic_header`/`seed_alice` helpers). The `entry_xml` builder may need an optional `<published>` argument — add it if absent:

```rust
#[apply(backends)]
#[tokio::test]
async fn create_with_future_published_is_scheduled(#[case] backend: Backend) {
    let env = backend.setup().await;
    let (app, storage) = make_app_for(&env).await; // existing make_app(state, storage) pattern
    seed_alice(&storage).await;
    let future = "2099-01-01T00:00:00Z";
    let xml = entry_xml_with_published("Future post", "body", /*draft*/ false, Some(future));
    let resp = post_collection(&app, &basic_header("alice"), xml).await;
    assert_eq!(resp.status(), StatusCode::CREATED);
    // The created post carries the future timestamp...
    let loc = location_post_id(&resp);
    let rec = storage.get_post_by_id(loc).await.unwrap().unwrap();
    assert_eq!(rec.published_at.unwrap().to_rfc3339(), "2099-01-01T00:00:00+00:00");
    // ...and is invisible on the public permalink at "now".
    let now = Utc::now();
    let public = storage.get_post_by_permalink(
        &uname("alice"), rec_year(&rec), rec_month(&rec), rec_day(&rec),
        &rec.slug, &ViewerIdentity::Anonymous, now,
    ).await.unwrap();
    assert!(public.is_none(), "future-published AtomPub post must be hidden until due");
}

#[apply(backends)]
#[tokio::test]
async fn create_with_past_published_is_live_backdated(#[case] backend: Backend) {
    let env = backend.setup().await;
    let (app, storage) = make_app_for(&env).await;
    seed_alice(&storage).await;
    let past = "2000-01-01T00:00:00Z";
    let xml = entry_xml_with_published("Old post", "body", false, Some(past));
    let resp = post_collection(&app, &basic_header("alice"), xml).await;
    assert_eq!(resp.status(), StatusCode::CREATED);
    let rec = storage.get_post_by_id(location_post_id(&resp)).await.unwrap().unwrap();
    assert_eq!(rec.published_at.unwrap().to_rfc3339(), "2000-01-01T00:00:00+00:00");
}
```

Keep the existing `create_draft_entry_is_unpublished` (atompub_posts.rs:978) passing unchanged.

- [ ] **Step 2: Run them, verify they fail** — `cd <worktree> && cargo nextest run -p jaunder -E 'test(create_with_future_published) + test(create_with_past_published)'` → FAIL.

- [ ] **Step 3: Implement parsing.** In `server/src/atompub/mapping.rs`: add `published: Option<DateTime<Utc>>` to `PostFields`; in `entry_to_post_fields` read the entry's `<published>` (the `atom_syndication`/`feed-rs` entry type already used; map `published` to `DateTime<Utc>`), set `None` when absent.

- [ ] **Step 4: Implement create.** In `collection_post` replace the `Utc::now()` stamp (274-278):

```rust
let published_at = if fields.is_draft {
    None
} else {
    Some(fields.published.unwrap_or_else(chrono::Utc::now))
};
```

- [ ] **Step 5: Implement update.** In `member_put` (381) build the `PublishUpdate`:

```rust
publish: if fields.is_draft {
    storage::PublishUpdate::Unpublish
} else {
    storage::PublishUpdate::Publish { at: fields.published }
},
```

- [ ] **Step 6: Run tests + gate** — the new tests + `create_draft_entry_is_unpublished` → PASS both backends; `cargo xtask check --no-test` → clean.

- [ ] **Step 7: Commit** — `feat(atompub): honor entry <published> on create and update (#70)`.

---

## Task 6: Web compose datetime control

**Files:**
- Modify: `web/src/posts/mod.rs` — `create_post` (189), `update_post` (345)
- Modify: `web/src/pages/ui.rs` (editor form), `web/src/pages/posts.rs` (`CreatePostPage` 29, `DraftPreviewPage` 458)
- Test: `server/tests/web/web_posts.rs`

**Interfaces:**
- Consumes: `PublishUpdate` (Task 4); storage creation already accepts `published_at: Option<DateTime<Utc>>`.
- Produces: `create_post`/`update_post` gain `publish_at: Option<DateTime<Utc>>`; when publishing, `published_at = publish_at.or(Some(now))` (a supplied future time schedules; a supplied past time backdates; absent = now).

- [ ] **Step 1: Write the failing server-fn test** in `server/tests/web/web_posts.rs`:

```rust
#[apply(backends)]
#[tokio::test]
async fn create_post_with_future_publish_at_is_scheduled(#[case] backend: Backend) {
    let env = backend.setup().await;
    let ctx = web_ctx(&env, "alice").await; // existing server-fn test context
    let future = Utc::now() + Duration::days(3);
    let res = create_post_call(&ctx, CreatePostArgs {
        body: "hello".into(), format: "markdown".into(), slug_override: None,
        publish: true, publish_at: Some(future), tags: None, summary: None, audience: None,
    }).await.unwrap();
    let rec = ctx.storage().get_post_by_id(res.post_id).await.unwrap().unwrap();
    assert_eq!(rec.published_at, Some(future));
    // Not visible publicly now.
    let public = ctx.storage().list_published(None, 50, &ViewerIdentity::Anonymous, Utc::now())
        .await.unwrap();
    assert!(!public.iter().any(|p| p.post_id == res.post_id));
}
```

(Adapt to the actual server-fn test harness shape in `web_posts.rs`.)

- [ ] **Step 2: Run it, verify it fails** — `cd <worktree> && cargo nextest run -p jaunder create_post_with_future_publish_at` → FAIL (no `publish_at` param).

- [ ] **Step 3: Implement the server fns.** Add `publish_at: Option<DateTime<Utc>>` to `create_post` (189) and `update_post` (345). `create_post`: `let published_at = publish.then(|| publish_at.unwrap_or_else(Utc::now));`. `update_post`: `publish: if publish { PublishUpdate::Publish { at: publish_at } } else { PublishUpdate::Unpublish }`. Read handlers that call the Task-2 reads pass `Utc::now()`.

- [ ] **Step 4: Implement the UI control.** In the editor form (`web/src/pages/ui.rs`) add an optional `datetime-local` input ("Publish at (optional)"). On submit, interpret the local value as the author's local time, convert to UTC, and pass as `publish_at`; leaving it empty sends `None` (publish-now on Publish). Display existing scheduled times in local time when editing. Keep "Save draft" = `publish: false`.

- [ ] **Step 5: Run test + gate** — `cargo nextest run -p jaunder create_post_with_future_publish_at` → PASS; `cargo xtask check --no-test` → clean.

- [ ] **Step 6: Commit** — `feat(web): add a publish-at datetime control to the compose form (#70)`.

---

## Task 7: Restart-durable go-live in the feed worker

The on-demand pages already flip at query time (Task 2). Only cached feeds need a nudge when a *future-dated* post crosses into "live" with no accompanying write. This task adds that nudge: a steady-state `(last_tick, now]` window pass and a feed-relative startup catch-up. Immediate/backdated publishes continue to enqueue their own regeneration on the write path (`web/src/feed_events.rs::enqueue_feed_events`) — the tick never reasons about backdating.

### Task 7a: Storage — posts that went live in a window

**Files:** Modify `storage/src/posts.rs` (trait + generic impl); Test `server/tests/storage/storage.rs`.

**Interfaces — Produces:**

```rust
/// A post that crossed into "live" within a time window, with the data
/// needed to compute its affected feed URLs.
pub struct GoLivePost {
    pub username: Username,
    pub tag_slugs: Vec<Tag>,
}

async fn list_posts_gone_live_between(
    &self,
    after: DateTime<Utc>,   // exclusive
    upto: DateTime<Utc>,    // inclusive
) -> sqlx::Result<Vec<GoLivePost>>;
```

SQL: posts where `published_at > $after AND published_at <= $upto AND deleted_at IS NULL`, joined to the author's username and the post's tag slugs (reuse the `TAGS_SUBQUERY` dialect const or a join + aggregate). One row per post; `tag_slugs` empty when untagged.

- [ ] **Step 1: Failing test** — `#[apply(backends)]` test: seed a post `published_at = T_after + 30min` (inside the window) and another at `T_upto + 1h` (outside); call `list_posts_gone_live_between(T_after, T_upto)`; assert only the in-window post is returned, with the right username and tags.
- [ ] **Step 2: Run → FAIL.**
- [ ] **Step 3: Implement** the trait method + generic impl in `storage/src/posts.rs`.
- [ ] **Step 4: Run → PASS** (both backends); `cargo xtask check --no-test` → clean.
- [ ] **Step 5: Commit** — `feat(storage): query posts gone live within a window (#70)`.

### Task 7b: Storage — feeds needing startup catch-up

**Files:** Modify `storage/src/posts.rs` and/or `storage/src/feed_cache.rs`; Test `server/tests/storage/storage.rs`.

**Interfaces — Produces:**

```rust
/// Feed URLs whose surface has a live post newer than the feed's own
/// `generated_at` — i.e. cached feeds that missed a go-live while the
/// worker was down.
async fn feed_urls_needing_catchup(
    &self,
    now: DateTime<Utc>,
) -> sqlx::Result<Vec<String>>;
```

Implementation: iterate cached feeds (`feed_cache` rows carry `feed_url` + `generated_at`, confirmed present — no migration needed); for each, derive its `FeedSurface` (reuse the existing `feed_url` → surface parsing used by `regenerate_feed`) and check whether `max(published_at)` for that surface (`published_at <= now`, not deleted) exceeds the row's `generated_at`. Return the URLs that do. (A single set-based SQL is preferable if the surface↔post join is expressible; otherwise a per-row check over `feed_cache` is acceptable given feed count is small.)

- [ ] **Step 1: Failing test** — seed a cached feed row with `generated_at = T0` and a live post for that surface with `published_at = T1 > T0`; assert the feed URL is returned. Add a control: a feed whose `generated_at` is newer than its newest post is **not** returned.
- [ ] **Step 2: Run → FAIL.**
- [ ] **Step 3: Implement.**
- [ ] **Step 4: Run → PASS** (both backends); `cargo xtask check --no-test` → clean.
- [ ] **Step 5: Commit** — `feat(storage): identify cached feeds needing go-live catch-up (#70)`.

### Task 7c: Worker — go-live pass (window + startup catch-up)

**Files:** Modify `server/src/feed/worker.rs` (`FeedWorker` 23-29, `tick` 54); possibly `server/src/feed/regenerate.rs` (extract a shared surface→`feed_url` builder if one is not already reusable); Test `server/tests/feed/feed_worker.rs`.

**Interfaces:**
- Consumes: `list_posts_gone_live_between` (7a), `feed_urls_needing_catchup` (7b), `FeedEventStorage::enqueue` (`storage/src/feed_events.rs:43`).
- Produces: `FeedWorker` gains `last_tick: tokio::sync::Mutex<Option<DateTime<Utc>>>` (init `None`) and:

```rust
/// Enqueue feed-regeneration for posts that crossed into "live" since the
/// last pass. On the first call (last_tick == None) runs the feed-relative
/// startup catch-up; thereafter runs the (last_tick, now] window pass.
/// Seeds last_tick = now in both branches.
pub async fn go_live_pass(&self, now: DateTime<Utc>) -> anyhow::Result<()>;
```

`tick()` calls `self.go_live_pass(Utc::now()).await` **before** draining the queue, so the same tick regenerates what it just enqueued.

- [ ] **Step 1: Write the restart-straddle test (the centerpiece)** in `server/tests/feed/feed_worker.rs`:

```rust
#[apply(backends)]
#[tokio::test]
async fn startup_catchup_regenerates_feed_for_go_live_while_down(#[case] backend: Backend) {
    let env = backend.setup().await;
    let worker = make_feed_worker(&env).await; // fresh worker => last_tick == None
    let t0 = Utc.with_ymd_and_hms(2026, 6, 26, 10, 0, 0).unwrap();
    // A cached feed generated at t0 (stale).
    seed_cached_feed(&env, "/feed/site.atom", /*generated_at*/ t0).await;
    // A post that went live at t1 > t0 while the worker was "down".
    let t1 = t0 + Duration::hours(1);
    seed_post_published_at(&env, "alice", "went-live", t1).await;

    // Restart: first go-live pass at now = t2.
    let t2 = t1 + Duration::hours(1);
    worker.go_live_pass(t2).await.unwrap();

    // The site feed was enqueued for regeneration.
    let pending = env.storage().feed_events_pending_urls().await.unwrap(); // test helper / claim_pending_batch
    assert!(pending.iter().any(|u| u == "/feed/site.atom"),
        "startup catch-up must enqueue the feed that missed a go-live");
}

#[apply(backends)]
#[tokio::test]
async fn steady_state_window_enqueues_newly_live_posts(#[case] backend: Backend) {
    let env = backend.setup().await;
    let worker = make_feed_worker(&env).await;
    let t0 = Utc.with_ymd_and_hms(2026, 6, 26, 10, 0, 0).unwrap();
    worker.go_live_pass(t0).await.unwrap(); // seeds last_tick = t0 (startup branch, nothing live)

    // A post scheduled to go live between t0 and t1.
    let go_live = t0 + Duration::minutes(30);
    seed_post_published_at(&env, "alice", "soon", go_live).await;

    let t1 = t0 + Duration::hours(1);
    worker.go_live_pass(t1).await.unwrap(); // window (t0, t1] catches it

    let pending = env.storage().feed_events_pending_urls().await.unwrap();
    assert!(pending.iter().any(|u| u.contains("alice")),
        "the author's feeds must be enqueued when their scheduled post goes live");
}
```

Add small test helpers as needed (`make_feed_worker`, `seed_cached_feed`, `feed_events_pending_urls` — the last can wrap `claim_pending_batch` or a direct `SELECT feed_url FROM feed_events WHERE status='pending'`).

- [ ] **Step 2: Run them, verify they fail** — `cd <worktree> && cargo nextest run -p jaunder -E 'test(startup_catchup_regenerates_feed) + test(steady_state_window_enqueues)'` → FAIL (no `go_live_pass`).

- [ ] **Step 3: Implement.** Add `last_tick: tokio::sync::Mutex<Option<DateTime<Utc>>>` to `FeedWorker` (and its constructor). Implement `go_live_pass`:
  - lock `last_tick`;
  - if `None`: `for url in storage.feed_urls_needing_catchup(now) { feed_events.enqueue(&url).await?; }`;
  - else `Some(last)`: `for p in storage.list_posts_gone_live_between(last, now) { for url in feed_urls_for(&p.username, &p.tag_slugs) { feed_events.enqueue(&url).await?; } }`;
  - set `*last_tick = Some(now)`.
  - `feed_urls_for` reuses the existing surface→URL construction (mirror `web/src/feed_events.rs::enqueue_feed_events`'s Site + User + per-tag × {Rss,Atom,Json} fan-out); if that logic is only in the web crate, extract a shared helper into the server/common crate and call it from both places (no behavior change to the web path).
  - Wire `self.go_live_pass(Utc::now()).await` into `tick()` before `claim_pending_batch`.

- [ ] **Step 4: Run them, verify they pass** — same filters → PASS both backends; `cd <worktree> && cargo xtask check --no-test` → clean.

- [ ] **Step 5: Final gate** — `cd <worktree> && cargo xtask validate --no-e2e` → green (static + clippy + coverage).

- [ ] **Step 6: Commit** — `feat(server): restart-durable go-live pass in the feed worker (#70)`.

---

## Self-review (spec coverage)

- Spec change 1 (unify visibility + index) → Tasks 1, 2.
- Spec change 2 (author scheduled surface, minimal) → Task 3.
- Spec change 3 (go-live: window + startup catch-up; writes self-enqueue) → Task 7 (existing `enqueue_feed_events` covers the write path).
- Spec change 4 (compose datetime control; backdating) → Task 6.
- Spec change 5 (slug freeze unchanged) → enforced as a global constraint (no task touches it).
- Spec change 6 (AtomPub honors `<published>`; update takes optional timestamp) → Tasks 4, 5.
- Edge cases (scheduled absent from every public surface; author sees all three states; backdated live immediately; feed reflects go-live within a tick; **restart straddling go-live healed by catch-up**; both backends) → boundary tests in Tasks 2/3, AtomPub tests in Task 5, worker tests in Task 7c, all via `#[apply(backends)]`.
- Out of scope (full management UI; reschedule; pull-back) → #15, untouched here.
