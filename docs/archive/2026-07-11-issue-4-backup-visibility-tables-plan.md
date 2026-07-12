# Backup target auto-derivation Implementation Plan

> **For agentic workers:** Execute this plan task-by-task with
> **jaunder-iterate** (delegating an individual task to a subagent via
> **jaunder-dispatch** when useful). Steps use checkbox (`- [ ]`) syntax for
> tracking. Commit per task; run the gate (`cargo xtask check`) before each
> commit (**jaunder-commit**). No `Co-Authored-By` trailer.

**Spec:**
[`docs/superpowers/specs/2026-07-11-issue-4-backup-visibility-tables.md`](../specs/2026-07-11-issue-4-backup-visibility-tables.md)
— the "what/why." This plan is the "how"; it does not restate the spec.

**Goal:** Make the backup subsystem derive its target-table set from the live
schema (minus an explicit denylist) instead of a hand-maintained list, so every
table — including the seven visibility tables, `media`, `user_config`, and
`feed_events` — is backed up and restored automatically, and future tables
cannot silently fall out.

**Architecture:** Postgres FKs become `DEFERRABLE INITIALLY IMMEDIATE` so
restore can `SET CONSTRAINTS ALL DEFERRED` and insert in any order (SQLite
already restores FK-off); with order irrelevant, the export set is "live tables
− denylist, sorted." A golden guardrail test and an FK-deferrability discipline
test keep the two invariants fail-closed. The shared backup fixture is extended
to seed + assert visibility fidelity, closing the reported bug (a restored
non-public post is visible to a non-author viewer).

**Tech Stack:** Rust, `sqlx` (SQLite + Postgres dialects), `cargo nextest`,
`rstest`/`rstest_reuse` dual-backend templates, the Nix `cargo xtask` gate.

## Review layer

**Scope — in:** `storage/src/backup.rs` + both dialect backup files; two `0023`
migrations; the restore emptiness guard (`ensure_restore_target_empty` →
`database_is_empty`); the shared backup fixture and its call sites; new
discipline/guardrail tests; an ADR draft. **Out:** a `--force` restore flag
(restore keeps its hard refusal of a non-empty target); cross-backend restore
semantics beyond version lockstep; `#136`'s contract-test refactor (a follow-up
once this lands); any web/emacs surface.

**Tasks:**

1. **ADR draft** — record the decision (auto-derived backup set; deferred/off-FK
   authoritative-replace restore; the two guard tests).
2. **Deferrable FKs** — `0023` migrations (Postgres `DO`-block alters every FK
   deferrable; SQLite no-op for version lockstep) + a discipline test asserting
   no non-deferrable FK survives.
3. **Auto-derive + authoritative restore** — replace `TABLES_IN_EXPORT_ORDER`
   with `live − denylist` (alphabetical); all-column `ORDER BY`; Postgres
   restore defers, both backends `DELETE`-before-`INSERT`; golden guardrail
   test; fix in-file unit tests.
4. **Fidelity** — extend the backup fixture to seed a Named-audience post +
   `user_config`/`media`/`feed_events`, assert a non-author subscriber sees the
   post (and anonymous does not), and remove the `#4` author-viewer workaround.
5. **Tighten the restore guard** — replace `database_has_users` with
   `database_is_empty` (every table but the three seeded lookups is empty);
   guard test that a fresh `init` is empty and exactly those three are seeded.

**Key risks/decisions:**

- **Restore now replaces, not appends** (`DELETE` before `INSERT`). Required
  because the migration-seeded lookups (`channels`, `subscription_statuses`,
  `target_kinds`) now enter the backup set and would duplicate-key on a
  fresh-init target. Existing same-/cross-backend round-trip tests
  (`backup_interop.rs`, `misc/commands.rs`) restore into pre-seeded targets, so
  they **verify** this — Task 3 fails them if `DELETE` is missing.
- **Denylist = `{_sqlx_migrations, feed_cache}`.** Everything else is backed up.
  The golden guardrail (`manifest.tables` == 20 known names, **and** live table
  count == 22) is bidirectional: a new table trips it whether auto-included or
  denylisted.
- **All-column `ORDER BY`** replaces the per-table key map + `rowid` fallback
  (Postgres has no `rowid`). Deterministic on both backends (`columns()` returns
  schema order); reproduces current dumps for the existing tables (their key is
  the first, unique column) so byte-stability tests still hold.

## Global Constraints

_Every task's requirements implicitly include this section._

- **Backend parity (CONTRIBUTING "backend parity"):** any storage behavior is
  tested on **both** backends via `#[apply(backends)]`, or via the existing
  cross-backend `backup_interop.rs`. A backend-specific test uses
  `#[apply(sqlite_only)]` / `#[apply(postgres_only)]` with a `// reason:` note.
  A bare `#[tokio::test]` that should be dual-backend fails the
  `test-backend-pattern` guard.
- **Coverage policy:** new host-reachable Rust must be covered by the gate.
  Migration `.sql` is not coverage-measured.
- **Denylist:**
  `TABLES_EXCLUDED_FROM_BACKUP = ["_sqlx_migrations", "feed_cache"]`.
- **Migration-seeded lookups** (non-empty in a pristine `init`; excluded from
  the restore emptiness check):
  `MIGRATION_SEEDED_TABLES = ["channels", "subscription_statuses", "target_kinds"]`.
  This is a distinct concept from the backup denylist.
- **Backed-up set (golden, alphabetical, 20):** `audience_members`, `audiences`,
  `channels`, `email_verifications`, `feed_events`, `invites`, `media`,
  `password_resets`, `post_audiences`, `post_revisions`, `post_tags`, `posts`,
  `sessions`, `site_config`, `subscription_statuses`, `subscriptions`, `tags`,
  `target_kinds`, `user_config`, `users`.
- **Schema-version lockstep:** both backends advance to `0023` together (the
  manifest's `schema_version = MAX(version)` is compared by the cross-backend
  interop tests).
- **Gate & commits:** run `cargo xtask check` clean before each commit; one
  clean commit per task; no `Co-Authored-By` trailer.

---

### Task 1: ADR draft — decision record

**Files:**

- Create: `docs/adr/0064-backup-target-auto-derivation.md` (numberless draft;
  `cargo xtask adr promote` numbers it at ship)

**Interfaces:** none (documentation).

Author with the **jaunder-adr** skill. The ADR records: **Context** — the
hand-maintained `TABLES_IN_EXPORT_ORDER` is a silent-data-loss class (issue #4
plus the latent `user_config`/`media` gaps). **Decision** — derive the backup
set from the live schema minus an explicit denylist; make restore
order-independent (Postgres FKs `DEFERRABLE` + `SET CONSTRAINTS ALL DEFERRED`;
SQLite FK-off) and authoritative (`DELETE`-before-`INSERT`); guard both
invariants with a golden guardrail test and an FK-deferrability discipline test.
**Consequences** — new tables are backed up by default; exclusions are the sole
reviewed decision; a schema-discipline burden (future FKs must be deferrable) is
made fail-closed by the discipline test. **Alternatives rejected** — a
topological sort of the FK graph (unneeded once order is irrelevant, and brittle
to cycles/self-refs); `SET session_replication_role = replica` (no commit-time
recheck → silently imports broken data; needs elevated privilege).

- [x] **Step 1: Write the draft** at the path above, following jaunder-adr's
      template (Status: proposed; link issue #4 and the spec).
- [x] **Step 2: No commit now.** `docs/adr/drafts/` is gitignored — the draft
      lives out of git until `cargo xtask adr promote` (run by jaunder-ship
      after the final rebase) numbers it, moves it out of `drafts/`, and stages
      it for the ship commit. Nothing to commit at this step.

---

### Task 2: Deferrable FKs + discipline test

**Files:**

- Create: `storage/migrations/postgres/0023_defer_foreign_keys.sql`
- Create: `storage/migrations/sqlite/0023_defer_foreign_keys.sql`
- Test: `server/tests/storage/storage.rs` (new `#[apply(postgres_only)]` test)

**Interfaces:**

- Produces: both backends at `schema_version = 23`; every Postgres FK
  `condeferrable = true`. Task 3's Postgres restore relies on this to
  `SET CONSTRAINTS ALL DEFERRED`.

- [ ] **Step 1: Write the failing discipline test** in
      `server/tests/storage/storage.rs`, near the other raw-SQL harness tests
      (it uses `raw_scalar_i64`, defined at `storage.rs:7423`):

```rust
#[apply(postgres_only)]
// reason: FK deferrability is a Postgres catalog property (pg_constraint.condeferrable);
// SQLite enforces FKs per-connection and has no equivalent, so this is Postgres-only.
#[tokio::test]
async fn every_foreign_key_is_deferrable(#[case] backend: Backend) {
    let env = backend.setup().await;
    let non_deferrable = raw_scalar_i64(
        backend,
        &env,
        "SELECT COUNT(*) FROM pg_constraint \
         WHERE contype = 'f' AND connamespace = 'public'::regnamespace \
           AND NOT condeferrable",
    )
    .await;
    assert_eq!(
        non_deferrable, 0,
        "every foreign key must be DEFERRABLE so restore can SET CONSTRAINTS ALL DEFERRED"
    );
}
```

- [ ] **Step 2: Run it, verify it fails**

Run: `cargo xtask check` (the Nix gate provides Postgres; `postgres_only` tests
do not run under a bare `cargo nextest`). Expected: FAIL — current FKs are
`NOT DEFERRABLE`, so the count is > 0.

- [ ] **Step 3: Write the Postgres migration** `0023_defer_foreign_keys.sql` — a
      `DO` block altering every FK in place (no hand-listed names):

```sql
-- Make every foreign key DEFERRABLE INITIALLY IMMEDIATE so a restore can
-- SET CONSTRAINTS ALL DEFERRED and bulk-load rows in any order, with integrity
-- verified once at COMMIT. INITIALLY IMMEDIATE keeps normal operation unchanged.
DO $$
DECLARE r RECORD;
BEGIN
    FOR r IN
        SELECT conrelid::regclass AS tbl, conname
        FROM pg_constraint
        WHERE contype = 'f' AND connamespace = 'public'::regnamespace
    LOOP
        EXECUTE format(
            'ALTER TABLE %s ALTER CONSTRAINT %I DEFERRABLE INITIALLY IMMEDIATE',
            r.tbl, r.conname
        );
    END LOOP;
END $$;
```

- [ ] **Step 4: Write the SQLite migration** `0023_defer_foreign_keys.sql` — a
      no-op that exists only to keep the two backends' migration versions in
      lockstep:

```sql
-- No-op: parity placeholder so SQLite and Postgres share schema_version 23.
-- SQLite restore disables FK enforcement per-connection (PRAGMA foreign_keys = OFF)
-- and validates once via foreign_key_check, so no schema change is needed here.
-- Altering SQLite FK deferrability would require a full table rebuild for no benefit.
SELECT 1;
```

- [ ] **Step 5: Run the discipline test + a migration-apply check, verify they
      pass**

Run: `cargo xtask check` Expected: PASS — `every_foreign_key_is_deferrable`
passes; both backends' migrations apply (the existing on-migrated-database
harness tests exercise `0023`).

- [ ] **Step 6: Commit**

```bash
git add storage/migrations/postgres/0023_defer_foreign_keys.sql \
        storage/migrations/sqlite/0023_defer_foreign_keys.sql \
        server/tests/storage/storage.rs
git commit -m "feat(storage): make foreign keys deferrable for order-independent restore (#4)"
```

---

### Task 3: Auto-derive the backup set + authoritative restore

**Files:**

- Modify: `storage/src/backup.rs` (drop `TABLES_IN_EXPORT_ORDER`; add
  `TABLES_EXCLUDED_FROM_BACKUP` + `backup_table_set`; change `order_by_clause`;
  fix its unit test)
- Modify: `storage/src/sqlite/backup.rs` (`existing_export_tables`;
  `import_table` `DELETE`-before-`INSERT`; `json_select` call site; imports)
- Modify: `storage/src/postgres/backup.rs` (`existing_export_tables`;
  `restore_database` defers; `import_table` `DELETE`-before-`INSERT`;
  `json_select` call site; imports)
- Test: `server/tests/storage/storage.rs` (new `#[apply(backends)]` guardrail
  test)

**Interfaces:**

- Consumes: `ColumnInfo { name, type_name }` (`backup.rs:96`), the deferrable
  FKs from Task 2.
- Produces:
  - `pub(crate) const TABLES_EXCLUDED_FROM_BACKUP: &[&str] = &["_sqlx_migrations", "feed_cache"];`
  - `pub(crate) fn backup_table_set(live: impl IntoIterator<Item = String>) -> Vec<String>`
    — drops `sqlite_%` internal names and denylisted names, returns the rest
    sorted ascending. Both dialects call it to turn the raw live-table list into
    the export set.
  - `pub(crate) fn order_by_clause(columns: &[ColumnInfo], quote_identifier: fn(&str) -> String) -> String`
    — new signature: returns every column, quoted and comma-joined (schema
    order), for a deterministic `ORDER BY`. Replaces the old `(table, quote)`
    map.

- [ ] **Step 1: Write the failing unit tests** in `storage/src/backup.rs`
      `#[cfg(test)]`. Replace `order_by_clause_uses_stable_table_keys`
      (`backup.rs:621`) and add a `backup_table_set` test:

```rust
#[test]
fn order_by_clause_orders_by_every_column_in_schema_order() {
    let columns = [
        ColumnInfo { name: "post_id".into(), type_name: "integer".into() },
        ColumnInfo { name: "tag_id".into(), type_name: "integer".into() },
    ];
    assert_eq!(
        order_by_clause(&columns, quote_test_identifier),
        "\"post_id\", \"tag_id\""
    );
    let one = [ColumnInfo { name: "user_id".into(), type_name: "integer".into() }];
    assert_eq!(order_by_clause(&one, quote_test_identifier), "\"user_id\"");
}

#[test]
fn backup_table_set_drops_internal_and_denylisted_and_sorts() {
    let live = [
        "posts", "users", "feed_cache", "_sqlx_migrations",
        "sqlite_sequence", "channels",
    ]
    .into_iter()
    .map(str::to_owned);
    assert_eq!(
        backup_table_set(live),
        vec!["channels".to_owned(), "posts".to_owned(), "users".to_owned()]
    );
}
```

- [ ] **Step 2: Run them, verify they fail**

Run:
`cargo nextest run -p storage order_by_clause_orders_by_every_column backup_table_set_drops`
Expected: FAIL — `backup_table_set` undefined; `order_by_clause` still takes
`&str`.

- [ ] **Step 3: Implement the `backup.rs` core.** Delete
      `TABLES_IN_EXPORT_ORDER`; add `TABLES_EXCLUDED_FROM_BACKUP`; add
      `backup_table_set`; rewrite `order_by_clause`. Contract pinned by Step 1's
      tests (filter `sqlite_%` + denylist, sort ascending; join every quoted
      column). Signature:

```rust
pub(crate) fn backup_table_set(live: impl IntoIterator<Item = String>) -> Vec<String> {
    let mut tables: Vec<String> = live
        .into_iter()
        .filter(|t| !t.starts_with("sqlite_") && !TABLES_EXCLUDED_FROM_BACKUP.contains(&t.as_str()))
        .collect();
    tables.sort();
    tables
}
```

- [ ] **Step 4: Rewire both dialects.** In `sqlite/backup.rs` and
      `postgres/backup.rs`:
  - `existing_export_tables`: keep the live-table query
    (`sqlite_master WHERE type='table'` /
    `information_schema.tables … BASE TABLE`), then return
    `backup_table_set(names)` instead of intersecting `TABLES_IN_EXPORT_ORDER`.
  - `json_select`: call `order_by_clause(columns, quote_identifier)` (the
    `columns` slice is already in scope) instead of
    `order_by_clause(table, quote_identifier)`.
  - `import_table`: `DELETE FROM <quoted table>` **before** the empty-rows early
    return, so restore is authoritative even for tables the backup left empty:

```rust
// Authoritative replace: clear the target table, then load the backup's rows.
// Safe inside the FK-off (SQLite) / deferred (Postgres) restore transaction.
sqlx::query(&format!("DELETE FROM {}", quote_identifier(table)))
    .execute(&mut *connection)
    .await?;
let rows = read_table_rows(source_path, table)?;
if rows.is_empty() {
    return Ok(());
}
```

- Fix `use` lines: drop `TABLES_IN_EXPORT_ORDER`, add
  `TABLES_EXCLUDED_FROM_BACKUP` if referenced (only
  `backup_table_set`/`order_by_clause` are needed at the call sites).

- [ ] **Step 5: Defer constraints in Postgres restore.** In
      `postgres/backup.rs::restore_database`, after `BEGIN` and before the
      import loop:

```rust
sqlx::query("SET CONSTRAINTS ALL DEFERRED")
    .execute(&mut *connection)
    .await?;
```

(SQLite restore already runs `PRAGMA foreign_keys = OFF`; no change.)

- [ ] **Step 6: Write the guardrail test** in `server/tests/storage/storage.rs`:

```rust
#[apply(backends)]
#[tokio::test]
async fn backup_covers_every_table_or_deliberately_excludes_it(#[case] backend: Backend) {
    // The set backup actually captures, proven by a real export of a fresh DB.
    let base = tempfile::TempDir::new().expect("tempdir");
    let args = storage_args_for(backend, &base).await; // per-backend StorageArgs helper
    cmd_init(&args, false).await.expect("init");
    let dir = base.path().join("backup");
    cmd_backup(&args, BackupMode::Directory, Some(dir.clone())).await.expect("backup");
    let manifest: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(dir.join("manifest.json")).unwrap()).unwrap();
    let mut tables: Vec<String> = manifest["tables"]
        .as_array().unwrap().iter().map(|v| v.as_str().unwrap().to_owned()).collect();
    tables.sort();

    let expected: Vec<String> = [
        "audience_members","audiences","channels","email_verifications","feed_events",
        "invites","media","password_resets","post_audiences","post_revisions","post_tags",
        "posts","sessions","site_config","subscription_statuses","subscriptions","tags",
        "target_kinds","user_config","users",
    ].iter().map(|s| s.to_string()).collect();
    assert_eq!(tables, expected, "backup set drifted — add the new table here or to TABLES_EXCLUDED_FROM_BACKUP");

    // Bidirectional: every live table is either backed up or an intentional exclusion.
    // 20 backed up + feed_cache + _sqlx_migrations = 22 tables in the schema.
    let count_sql = match backend {
        Backend::Sqlite => "SELECT COUNT(*) FROM sqlite_master \
                            WHERE type='table' AND name NOT LIKE 'sqlite_%'",
        Backend::Postgres => "SELECT COUNT(*) FROM information_schema.tables \
                             WHERE table_schema='public' AND table_type='BASE TABLE'",
    };
    let env = backend.setup().await;
    assert_eq!(
        raw_scalar_i64(backend, &env, count_sql).await, 22,
        "a table was added or removed — update the golden set and denylist deliberately"
    );
}
```

If no per-backend `StorageArgs` helper exists in this test module, add a small
`storage_args_for(backend, &base)` mirroring `backup_interop.rs`'s
`sqlite_storage_args` / `postgres_storage_args` (the Postgres arm returns and
holds a `PostgresDbGuard`). Keep it beside the test.

- [ ] **Step 7: Run the full gate, verify green**

Run: `cargo xtask check` Expected: PASS — unit tests, the new guardrail (both
backends), the discipline test, and the **existing** `backup_interop.rs` /
`misc/commands.rs` round-trips (which restore into pre-seeded targets,
exercising `DELETE`-before-`INSERT` and the deferred/off-FK path).

- [ ] **Step 8: Commit**

```bash
git add storage/src/backup.rs storage/src/sqlite/backup.rs \
        storage/src/postgres/backup.rs server/tests/storage/storage.rs
git commit -m "feat(storage): derive backup table set from live schema; restore replaces (#4)"
```

---

### Task 4: Fidelity — seed + assert visibility, media, config, feed events

**Files:**

- Modify: `server/tests/misc/backup_fixture.rs` (extend
  `populate_backup_fixture` + `assert_backup_fixture_restored`; return a small
  ids struct; drop the `#4` workaround)
- Modify: `server/tests/misc/backup_interop.rs` and
  `server/tests/misc/commands.rs` (update the fixture call sites to the new
  return type)

**Interfaces (storage APIs consumed — from `AppState`):**

- `state.users.create_user(&Username, &Password, Some(&str), is_operator: bool) -> i64`
- `state.subscriptions.local_channel_id() -> sqlx::Result<i64>`
- `state.subscriptions.subscribe(author_user_id: i64, channel_id: i64, subscriber_ref: &str) -> sqlx::Result<i64>`
  (Active; `subscriber_ref` = viewer's `user_id.to_string()`)
- `state.audiences.create_audience(author_user_id: i64, name: &str) -> Result<i64, AudienceError>`
- `state.audiences.add_member(author_user_id: i64, audience_id: i64, subscription_id: i64) -> Result<(), AudienceError>`
- `common::visibility::AudienceTarget::{Public, Named(i64)}`;
  `CreatePostInput.audiences: Vec<AudienceTarget>`
- `common::visibility::ViewerIdentity::{local(user_id, channel_id), Anonymous}`
- `state.posts.get_post_by_id(post_id: i64, &ViewerIdentity) -> sqlx::Result<Option<PostRecord>>`
- `state.user_config.set(user_id, key, value)` /
  `.get(user_id, key) -> Option<String>`
- `state.media.create_media(&MediaRecord) -> Result<(), CreateMediaError>` /
  `.get_media(user_id, sha256, filename, &MediaSource) -> Option<MediaRecord>`;
  `MediaRecord { user_id, sha256, filename, source: MediaSource::Upload, content_type, size_bytes, source_url, created_at }`
- `state.feed_events.enqueue(feed_url: &str) -> Result<i64, FeedEventError>`

**Produces (fixture contract for its call sites):**

```rust
pub struct BackupFixtureIds {
    pub author_id: i64,
    pub viewer_id: i64,      // a non-author subscriber
    pub public_post_id: i64,
    pub named_post_id: i64,  // targeted at a Named audience the viewer belongs to
}
```

- [ ] **Step 1: Extend `populate_backup_fixture`** to also seed the visibility
      stack and the three previously-unbacked tables, and return
      `BackupFixtureIds`. Append after the existing public post (keep the media
      _file_ write):

```rust
// Non-author subscriber who will be admitted to a Named-audience post.
let viewer_name: Username = "viewer".parse().expect("valid username");
let viewer_id = state.users
    .create_user(&viewer_name, &password, Some("Viewer"), false).await.expect("create viewer");
let local = state.subscriptions.local_channel_id().await.expect("local channel");
let sub_id = state.subscriptions
    .subscribe(user_id, local, &viewer_id.to_string()).await.expect("subscribe");
let audience_id = state.audiences.create_audience(user_id, "friends").await.expect("audience");
state.audiences.add_member(user_id, audience_id, sub_id).await.expect("add member");
let named_post_id = state.posts.create_post(&CreatePostInput {
    user_id,
    title: Some("Friends Only".to_owned()),
    slug: "friends-only".parse().expect("valid slug"),
    body: "secret body".to_owned(),
    format: PostFormat::Markdown,
    rendered_html: "<p>secret body</p>".to_owned(),
    published_at: Some(fixture_published_at()),
    summary: None,
    audiences: vec![AudienceTarget::Named(audience_id)],
}).await.expect("create named post");

state.user_config.set(user_id, "editor.theme", "dark").await.expect("set config");
state.media.create_media(&MediaRecord {
    user_id,
    sha256: "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef".to_owned(), // any stable value; media file mirror is separate
    filename: "photo.jpg".to_owned(),
    source: MediaSource::Upload,
    content_type: "image/jpeg".to_owned(),
    size_bytes: 4,
    source_url: None,
    created_at: fixture_published_at(),
}).await.expect("create media row");
state.feed_events.enqueue("https://example.com/feed.xml").await.expect("enqueue feed event");

BackupFixtureIds { author_id: user_id, viewer_id, public_post_id: post_id, named_post_id }
```

Update the signature to `-> BackupFixtureIds` and the imports
(`common::visibility::AudienceTarget`, `common::username::Username`,
`storage::{MediaRecord, MediaSource}` — confirm exact paths at the modules named
in Interfaces).

- [ ] **Step 2: Extend `assert_backup_fixture_restored`** to take
      `&BackupFixtureIds` and assert the restored fidelity. Replace the `#4`
      author-viewer workaround comment (`backup_fixture.rs:66-69`) with real
      non-author visibility:

```rust
pub async fn assert_backup_fixture_restored(args: &StorageArgs, ids: &BackupFixtureIds) {
    let state = open_existing_database(&args.db).await.expect("open restored database");
    let local = state.subscriptions.local_channel_id().await.expect("local channel");

    // Public post still resolves (author view retained).
    let author_view = ViewerIdentity::local(ids.author_id, local);
    let public = state.posts.get_post_by_id(ids.public_post_id, &author_view)
        .await.expect("get public").expect("public post restored");
    assert_eq!(public.slug.as_str(), "restored-post");

    // #4 closed: the Named-audience post is visible to the non-author subscriber…
    let viewer = ViewerIdentity::local(ids.viewer_id, local);
    assert!(
        state.posts.get_post_by_id(ids.named_post_id, &viewer).await.expect("get named").is_some(),
        "restored post_audiences/subscriptions/audience_members must admit the subscriber"
    );
    // …and correctly invisible to anonymous.
    assert!(
        state.posts.get_post_by_id(ids.named_post_id, &ViewerIdentity::Anonymous)
            .await.expect("get named anon").is_none(),
        "a Named-audience post must not be public"
    );

    // The previously-unbacked tables survive.
    assert_eq!(
        state.user_config.get(ids.author_id, "editor.theme").await.expect("get config").as_deref(),
        Some("dark")
    );
    assert!(
        state.media.get_media(ids.author_id, "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef", "photo.jpg", &MediaSource::Upload)
            .await.expect("get media").is_some(),
        "media table row must be restored"
    );

    // media file mirror unchanged.
    assert_eq!(
        std::fs::read_to_string(args.storage_path.join("media").join("avatar.txt")).expect("media"),
        "media"
    );
    // (Existing user/tag assertions retained.)
}
```

Keep the existing operator-user and tag assertions (re-derive `user`/`tags` as
before). `feed_events` fidelity is covered structurally by the guardrail (it is
now in the backup set) and the byte-stable round-trip; no per-row assert is
required, but a `feed_events` count check may be added if desired.

- [ ] **Step 3: Update the fixture call sites** in `backup_interop.rs` (3 tests,
      incl. the 4-hop cycle that asserts the same ids at S1/P2/S2) and
      `commands.rs` (the M6.6.1 and `#136` archive-mode tests). Mechanical:
      `let ids = populate_backup_fixture(&src).await;` then
      `assert_backup_fixture_restored(&target, &ids).await;`. The
      rejected-restore test keeps `assert_target_unmodified` and just ignores
      the returned ids (`let _ = …`).

- [ ] **Step 4: Run the gate, verify green**

Run: `cargo xtask check` Expected: PASS — the fidelity assertions hold on both
backends and across the cross-backend cycle (non-author visibility now survives
restore); byte-stability pairs still match (fixture values are seeded once and
carried verbatim).

- [ ] **Step 5: Commit**

```bash
git add server/tests/misc/backup_fixture.rs server/tests/misc/backup_interop.rs \
        server/tests/misc/commands.rs
git commit -m "test(storage): assert visibility/config/media fidelity through backup restore (#4)"
```

---

### Task 5: Tighten the restore emptiness guard

**Files:**

- Modify: `storage/src/db.rs` (add `MIGRATION_SEEDED_TABLES` +
  `database_is_empty`; drop `database_has_users` + its routing test)
- Modify: `storage/src/sqlite/mod.rs`, `storage/src/postgres/mod.rs`
  (per-backend `database_is_empty`; drop the per-backend `database_has_users`)
- Modify: `server/src/commands.rs` (`ensure_restore_target_empty` calls
  `database_is_empty`)
- Test: `server/tests/storage/storage.rs` (dual-backend guard test)

**Interfaces:**

- Consumes: the live-table enumeration each dialect already performs for backup
  (`sqlite_master` / `information_schema`).
- Produces:
  - `pub(crate) const MIGRATION_SEEDED_TABLES: &[&str] = &["channels", "subscription_statuses", "target_kinds"];`
  - `pub async fn database_is_empty(options: &DbConnectOptions) -> sqlx::Result<bool>`
    — true iff every table except `MIGRATION_SEEDED_TABLES` (and
    `_sqlx_migrations` / `sqlite_%` internals) is empty. Replaces
    `database_has_users`.

- [ ] **Step 1: Write the failing guard test** in
      `server/tests/storage/storage.rs`. It pins the invariant the check depends
      on: a fresh `init` reads as empty, and the only seeded tables are the
      three lookups.

```rust
#[apply(backends)]
#[tokio::test]
async fn fresh_init_is_empty_except_seeded_lookups(#[case] backend: Backend) {
    let base = tempfile::TempDir::new().expect("tempdir");
    let args = storage_args_for(backend, &base).await;
    cmd_init(&args, false).await.expect("init");

    // The restore guard must treat a pristine database as empty.
    assert!(
        storage::database_is_empty(&args.db).await.expect("is_empty"),
        "a freshly-initialized database must count as empty"
    );

    // …and the only populated tables are the migration-seeded lookups. If a future
    // migration seeds a new table, database_is_empty above turns false and this fails.
    let env = backend.setup().await;
    for table in ["channels", "subscription_statuses", "target_kinds"] {
        assert!(
            raw_scalar_i64(backend, &env, &format!("SELECT COUNT(*) FROM {table}")).await > 0,
            "{table} must be seeded by migrations"
        );
    }
}
```

- [ ] **Step 2: Run it, verify it fails**

Run: `cargo nextest run -p jaunder fresh_init_is_empty_except_seeded_lookups`
(SQLite arm; the Postgres arm runs under `cargo xtask check`). Expected: FAIL —
`storage::database_is_empty` does not exist yet.

- [ ] **Step 3: Implement `database_is_empty`.** In `storage/src/db.rs` add
      `MIGRATION_SEEDED_TABLES` and the cross-backend router (mirroring the
      existing `database_has_users` routing), then per-backend impls in
      `sqlite/mod.rs` / `postgres/mod.rs`:

```rust
// per backend: connect, list base tables (sqlite_master type='table' NOT LIKE 'sqlite_%'
// and <> '_sqlx_migrations'  /  information_schema BASE TABLE in 'public'),
// skip MIGRATION_SEEDED_TABLES, and return false on the first table that has a row.
for table in tables {
    if MIGRATION_SEEDED_TABLES.contains(&table.as_str()) { continue; }
    let has_row: Option<i64> = sqlx::query_scalar(
        &format!("SELECT 1 FROM {} LIMIT 1", quote_identifier(&table)))
        .fetch_optional(&mut *conn).await?;
    if has_row.is_some() { return Ok(false); }
}
Ok(true)
```

      Remove `database_has_users` (both per-backend impls, the router, and its
      `..._routes_to_postgres_backend` test in `db.rs`) — it has no other caller.

- [ ] **Step 4: Point the guard at it.** In `server/src/commands.rs`
      `ensure_restore_target_empty`, change
      `storage::database_has_users(&storage.db)` to
      `storage::database_is_empty(&storage.db).await?` inverted:

```rust
if !storage::database_is_empty(&storage.db).await? {
    return Err(anyhow::anyhow!("refusing to restore into a non-empty database"));
}
```

- [ ] **Step 5: Run the gate, verify green**

Run: `cargo xtask check` Expected: PASS — the guard test passes on both
backends; existing restore tests still pass (fresh-init targets read as empty);
the populated-target rejection test still refuses (a user row makes
`database_is_empty` false).

- [ ] **Step 6: Commit**

```bash
git add storage/src/db.rs storage/src/sqlite/mod.rs storage/src/postgres/mod.rs \
        server/src/commands.rs server/tests/storage/storage.rs
git commit -m "feat(storage): refuse restore unless every non-seeded table is empty (#4)"
```
