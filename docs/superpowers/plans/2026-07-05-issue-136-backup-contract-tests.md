# Backup Contract Tests (issue #136) — Implementation Plan

> **For agentic workers:** Execute this plan task-by-task with
> **jaunder-iterate** (delegating an individual task to a subagent via
> **jaunder-dispatch** when useful). Steps use checkbox (`- [ ]`) syntax for
> tracking.

**Goal:** Reconceive backup/restore testing at the contract level — delete the 8
interim `sqlite_only` storage tests, fill the real gaps (archive round-trip,
dual-backend negatives, an A→B→A→B cycle), and make a constraint-violating
restore fail uniformly on both backends.

**Architecture:** Backup is a cross-backend contract tested through the public
CLI/`AppState` interface (`cmd_init`/`cmd_backup`/`cmd_restore` + store
handles), not dialect internals. Tests live in the `misc` server test target
beside the existing `commands.rs` round-trips and `backup_interop.rs` hops. One
small product change makes restore failure uniform (Postgres SQLSTATE-`23` →
`ConstraintViolation`; SQLite validate-before-commit rollback).

**Tech Stack:** Rust, sqlx, rstest/rstest_reuse (`#[apply(backends)]`),
cargo-nextest, `cargo xtask` gate, `devtool`.

**Spec:** `docs/superpowers/specs/2026-07-05-issue-136-backup-contract-tests.md`
— this plan is "how"; the spec is "what/why". Read it for rationale
(DEC-A…DEC-D, coverage bookkeeping, hazards).

## Global Constraints

- **Backend parity** (CONTRIBUTING §"backend parity"): DB `#[tokio::test]`s use
  a template — `#[apply(backends)]` (dual) or `#[apply(postgres_only)]`
  (cross-backend, with a decisive `// reason:`). A bare DB `#[tokio::test]`
  fails the `test-backend-pattern` guard.
- **Coverage gate** (ADR-0050): per-function CRAP; an accepted uncovered branch
  carries an in-source `// cov:ignore` with a reason. The gate runs one
  workspace-wide instrumented pass with Postgres live — placement is
  coverage-neutral.
- **Postgres required:** `#[apply(backends)]`/`postgres_only` cases need a
  reachable PG. Run tests via `devtool pg run -- <cmd>`; the commit gate
  (`cargo xtask check`) provides its own PG.
- **Commit discipline** (jaunder-commit): run `cargo xtask check` clean before
  each commit (the pre-commit hook runs it too). **No `Co-Authored-By`
  trailer.**
- **Test target:** `commands.rs` and `backup_interop.rs` compile into the `misc`
  target (`server/tests/misc/main.rs`). Filter runs with `--test misc`.

---

## Review header

**Scope — in:** delete the 8 interim storage backup tests + `migrated_pool`/
`migrated_conn`; consolidate + strengthen the shared fixture (assert timestamp
value); add dual-backend negatives (dangling-FK, malformed-row, missing-`db/`)
and an archive round-trip; add the A→B→A→B cycle; the DEC-C
uniform-restore-failure product change; cov:ignore bookkeeping; an ADR draft;
expand issue #4.

**Scope — out:** `post_audiences`/visibility-table fidelity (issue #4);
canonical cross-backend serialization (DEC-D non-goal); any other backup product
change.

**Tasks:**

1. File the issue #4 expansion (missing tables + anti-regression guard).
2. Consolidate + strengthen the shared backup fixture (value interop: assert
   `published_at`).
3. DEC-C uniform restore-failure product change + dual-backend dangling-FK
   negative.
4. Dual-backend malformed-row rollback negative.
5. Dual-backend missing-`db/`-directory negative.
6. Dual-backend archive round-trip.
7. A→B→A→B cross-backend cycle (postgres_only) + dump-equality helper.
8. Delete the 8 interim tests + helpers; cov:ignore the SQLite export arm.
9. ADR draft recording DEC-A/DEC-B/DEC-C.

**Key risks/decisions:**

- **DEC-C SQLite rewrite** is the only runtime-behavior change:
  `foreign_key_check` must run inside the transaction with `foreign_keys = OFF`
  (valid — it scans, not enforces) before `COMMIT`, rolling back on violation.
  Task 3's negative pins it.
- **Ordering keeps every commit green:** all coverage-restoring tests (Tasks
  3–6) land before the deletions (Task 8), so no CRAP dip at any commit.
- **`E_A₁==E_A₂` is empirical** (Task 7): assert if it holds; else downgrade to
  a `// note:` — `E_B₁==E_B₂` is the guaranteed floor.

---

## File Structure

- **Create** `server/tests/misc/backup_fixture.rs` — the single home for
  `populate_backup_fixture` / `assert_backup_fixture_restored` /
  `fixture_published_at` (currently duplicated). Declared `mod backup_fixture;`
  in `server/tests/misc/main.rs`.
- **Modify** `server/tests/misc/commands.rs` — drop the local fixture copy; use
  `crate::backup_fixture::*`; add the negatives (Tasks 3–5) and archive
  round-trip (Task 6).
- **Modify** `server/tests/misc/backup_interop.rs` — drop the local fixture
  copy; add the cycle test + dump-equality helper (Task 7).
- **Modify** `storage/src/postgres/backup.rs` — map SQLSTATE-`23` →
  `ConstraintViolation` on import; remove the now-covered restore-`Err`
  `cov:ignore` (Task 3).
- **Modify** `storage/src/sqlite/backup.rs` — validate-before-commit rollback
  (Task 3); delete the 5 interim tests + `migrated_pool`/`migrated_conn`;
  cov:ignore the export arm (Task 8).
- **Modify** `storage/src/backup.rs` — delete the 3 interim orchestration tests
  (Task 8).
- **Create** `docs/adr/drafts/backup-test-homing-and-uniform-restore-failure.md`
  (Task 9).

---

## Task 1: File the issue #4 expansion

Separable concern (spec §"Separable concern"). No code.

- [x] **Step 1: Add a scope comment to issue #4** via **jaunder-issues**. Post a
      comment on `jaunder-org/jaunder#4` ("backup: restore drops the visibility
      tables") stating the expanded scope: **(a)** add the visibility tables
      (`channels`, `subscription_statuses`, `target_kinds`, `subscriptions`,
      `audiences`, `audience_members`, `post_audiences`) to
      `TABLES_IN_EXPORT_ORDER` and the export/restore path; **(b)** an
      anti-regression guard — a test enumerating the live schema's tables
      (SQLite `sqlite_master` / Postgres `information_schema.tables`) that
      asserts each is either in `TABLES_IN_EXPORT_ORDER` or on an explicit
      exclusion list, so a new migration adding a table fails until its backup
      coverage is decided. Note it was surfaced during #136 (contract-level
      backup testing).

- [x] **Step 2: Verify** the comment is visible on issue #4. No commit
      (tracker-only).

---

## Task 2: Consolidate + strengthen the shared backup fixture

**Files:**

- Create: `server/tests/misc/backup_fixture.rs`
- Modify: `server/tests/misc/main.rs` (add `mod backup_fixture;`)
- Modify: `server/tests/misc/commands.rs:59-144` (remove local fixture; import
  shared)
- Modify: `server/tests/misc/backup_interop.rs:45-130` (remove local fixture;
  import shared)

**Interfaces:**

- Produces:
  - `pub fn fixture_published_at() -> chrono::DateTime<chrono::Utc>`
  - `pub async fn populate_backup_fixture(args: &jaunder::cli::StorageArgs) -> i64`
  - `pub async fn assert_backup_fixture_restored(args: &jaunder::cli::StorageArgs, post_id: i64)`

- [x] **Step 1: Create the shared module**
      `server/tests/misc/backup_fixture.rs`:

```rust
#![allow(clippy::expect_used, clippy::unwrap_used, clippy::similar_names)]

use chrono::{DateTime, Utc};
use common::password::Password;
use common::username::Username;
use common::visibility::{AudienceTarget, ViewerIdentity};
use jaunder::cli::StorageArgs;
use storage::{open_existing_database, CreatePostInput, PostFormat};

/// Fixed microsecond-precision publish time: deterministic and safe from
/// Postgres's µs quantization, so a restored value can be asserted exactly (DEC-D).
pub fn fixture_published_at() -> DateTime<Utc> {
    "2026-04-29T12:34:56.789012Z"
        .parse()
        .expect("valid fixture timestamp")
}

pub async fn populate_backup_fixture(args: &StorageArgs) -> i64 {
    let state = open_existing_database(&args.db)
        .await
        .expect("open database");
    let username: Username = "backupuser".parse().expect("valid username");
    let password: Password = "password123".parse().expect("valid password");
    let user_id = state
        .users
        .create_user(&username, &password, Some("Backup User"), true)
        .await
        .expect("create user");
    let post_id = state
        .posts
        .create_post(&CreatePostInput {
            user_id,
            title: Some("Restored Post".to_owned()),
            slug: "restored-post".parse().expect("valid slug"),
            body: "body text".to_owned(),
            format: PostFormat::Markdown,
            rendered_html: "<p>body text</p>".to_owned(),
            published_at: Some(fixture_published_at()),
            summary: None,
            audiences: vec![AudienceTarget::Public],
        })
        .await
        .expect("create post");
    state
        .posts
        .tag_post(post_id, "Backup-Test")
        .await
        .expect("tag post");
    std::fs::write(args.storage_path.join("media").join("avatar.txt"), "media")
        .expect("write media");
    post_id
}

pub async fn assert_backup_fixture_restored(args: &StorageArgs, post_id: i64) {
    let state = open_existing_database(&args.db)
        .await
        .expect("open restored database");
    let username: Username = "backupuser".parse().expect("valid username");
    let user = state
        .users
        .get_user_by_username(&username)
        .await
        .expect("get user")
        .expect("restored user");
    assert!(user.is_operator);
    assert_eq!(user.display_name.as_deref(), Some("Backup User"));

    // View as the restored post's author. Backup/restore does not yet carry the
    // `post_audiences` rows (issue #4), so an Anonymous viewer would be filtered
    // out by the resolution predicate; the owner is always admitted via the author
    // branch, which is the correct viewer here.
    let local = state
        .subscriptions
        .local_channel_id()
        .await
        .expect("local channel id");
    let post = state
        .posts
        .get_post_by_id(post_id, &ViewerIdentity::local(user.user_id, local))
        .await
        .expect("get post")
        .expect("restored post");
    assert_eq!(post.title.as_deref(), Some("Restored Post"));
    assert_eq!(post.slug.as_str(), "restored-post");
    // Value interop (DEC-D): the timestamp survives with its value.
    assert_eq!(post.published_at, Some(fixture_published_at()));

    let tags = state
        .posts
        .get_tags_for_post(post_id)
        .await
        .expect("get tags");
    assert_eq!(tags.len(), 1);
    assert_eq!(tags[0].tag_slug.as_str(), "backup-test");
    assert_eq!(tags[0].tag_display, "Backup-Test");

    assert_eq!(
        std::fs::read_to_string(args.storage_path.join("media").join("avatar.txt"))
            .expect("read restored media"),
        "media"
    );
}
```

- [x] **Step 2: Wire the module.** In `server/tests/misc/main.rs`, add
      `mod backup_fixture;` (after `mod helpers;`).

- [x] **Step 3: Point both files at the shared fixture.** In `commands.rs`,
      delete the local
      `populate_backup_fixture`/`assert_backup_fixture_restored` (lines ~59–144)
      and add
      `use crate::backup_fixture::{assert_backup_fixture_restored, populate_backup_fixture};`.
      In `backup_interop.rs`, delete its local copies (lines ~45–130) and add
      the same `use`. Remove imports that become unused after the deletion
      (clippy/`-D warnings` will name them — in `commands.rs` expect
      `chrono::Utc` (line 17), `CreatePostInput`, `PostFormat`,
      `AudienceTarget`; `open_existing_database`/`Username` **stay** (used by
      other tests). In `backup_interop.rs` expect `Utc`, `Username`, `Password`,
      `open_existing_database`, `CreatePostInput`, `PostFormat`,
      `AudienceTarget`, and the `common::{password,username, visibility}` lines
      — keeping `storage::BackupMode`).

- [x] **Step 4: Run the affected round-trip tests on both backends** (this is
      where value interop is empirically confirmed — the strengthened assert now
      checks `published_at`, including cross-backend in the interop tests):

Run:
`devtool pg run -- cargo nextest run -p jaunder --test misc restore_restores_directory_backup backup_restores_into`
Expected: PASS — the same-backend round-trip and both cross-backend hops
preserve the fixed `published_at`. (If a cross-backend hop fails on the
timestamp, value interop has a real precision issue — stop and report; the
spec's DEC-D assumption would be wrong.)

- [x] **Step 5: Commit.**

```bash
git add server/tests/misc/backup_fixture.rs server/tests/misc/main.rs server/tests/misc/commands.rs server/tests/misc/backup_interop.rs
git commit -m "test(backup): share and strengthen the backup fixture (assert timestamp value)"
```

Run `cargo xtask check` first (jaunder-commit).

---

## Task 3: DEC-C uniform restore-failure + dangling-FK negative

**Files:**

- Modify: `storage/src/sqlite/backup.rs:62-107` (`restore_database` — validate
  before commit)
- Modify: `storage/src/postgres/backup.rs:117-152` (`import_table` — map
  SQLSTATE `23`) and `:89-93` (remove restore-`Err` `cov:ignore`)
- Test: `server/tests/misc/commands.rs`

**Interfaces:**

- Consumes: `populate_backup_fixture` (Task 2), `storage_args`
  (`commands.rs:41`), `cmd_init`/`cmd_backup`/`cmd_restore`,
  `open_existing_database`, `Backend`.
- Behavior after this task: `restore_backup` on **either** backend rejects a
  constraint-violating backup with `BackupError::ConstraintViolation` and leaves
  the target unmodified.

- [x] **Step 1: Write the failing test** in `commands.rs` (near the other
      `cmd_restore_*` tests):

```rust
// #136: a backup with a dangling foreign key is rejected uniformly (DEC-C) —
// ConstraintViolation + target unmodified, on both backends.
#[apply(backends)]
#[tokio::test]
async fn cmd_restore_rejects_dangling_foreign_key(#[case] backend: Backend) {
    let base = TempDir::new().expect("temp dir");
    let source_args = storage_args(backend, &base).await;
    cmd_init(&source_args, false).await.expect("init source");
    let post_id = populate_backup_fixture(&source_args).await;

    let backup_path = base.path().join("backup");
    cmd_backup(&source_args, BackupMode::Directory, Some(backup_path.clone()))
        .await
        .expect("backup");

    // Append a post_tags row referencing a nonexistent tag_id → dangling FK. The row
    // MUST carry every column of the real exported row (post_id, tag_id, and the
    // NOT NULL tag_display) — import_table derives its column set from the first row
    // and rejects a row missing a column with InvalidBackup *before* inserting, which
    // would mask the FK violation.
    let post_tags = backup_path.join("db").join("post_tags.ndjson");
    let mut contents = std::fs::read_to_string(&post_tags).expect("read post_tags");
    contents.push_str(&format!(
        "{{\"post_id\":{post_id},\"tag_id\":999999,\"tag_display\":\"Dangling\"}}\n"
    ));
    std::fs::write(&post_tags, contents).expect("write tampered post_tags");

    let target_base = TempDir::new().expect("target temp dir");
    let target_args = storage_args(backend, &target_base).await;
    cmd_init(&target_args, false).await.expect("init target");

    let err = cmd_restore(&target_args, &backup_path)
        .await
        .expect_err("restore rejects dangling FK");
    assert!(
        err.to_string().contains("failed constraint validation"),
        "expected ConstraintViolation, got: {err}"
    );

    // Rollback: nothing from the backup landed in the target.
    let state = open_existing_database(&target_args.db)
        .await
        .expect("open target");
    let username: Username = "backupuser".parse().expect("valid username");
    assert!(
        state
            .users
            .get_user_by_username(&username)
            .await
            .expect("get user")
            .is_none(),
        "target must be unmodified after a rejected restore"
    );
}
```

- [x] **Step 2: Run it, verify it FAILS on both backends**

Run:
`devtool pg run -- cargo nextest run -p jaunder --test misc cmd_restore_rejects_dangling_foreign_key`
Expected: FAIL — SQLite commits before checking (target IS modified → the
`is_none()` assertion fails); Postgres returns `Sqlx` not `ConstraintViolation`
(the message assertion fails).

- [x] **Step 3: Implement the SQLite change** — in
      `storage/src/sqlite/backup.rs`, `restore_database`, move
      `validate_foreign_keys` inside the transaction (before `COMMIT`) so a
      violation rolls back. Replace the body from
      `let result = async { … }.await;` through the `match`:

```rust
    let result = async {
        for table in &manifest.tables {
            let columns = columns(&mut connection, table).await?;
            import_table(&mut connection, source_path, table, &columns).await?;
        }
        // Validate FKs before committing so a violation rolls the whole restore
        // back. `foreign_key_check` works with `foreign_keys = OFF` — it scans for
        // violations rather than enforcing on write.
        validate_foreign_keys(&mut connection).await
    }
    .await;

    match result {
        Ok(()) => {
            sqlx::query("COMMIT").execute(&mut *connection).await?;
            sqlx::query("PRAGMA foreign_keys = ON")
                .execute(&mut *connection)
                .await?;
            Ok(())
        }
        Err(error) => {
            let _ = sqlx::query("ROLLBACK").execute(&mut *connection).await;
            let _ = sqlx::query("PRAGMA foreign_keys = ON")
                .execute(&mut *connection)
                .await;
            Err(error)
        }
    }
```

- [x] **Step 4: Implement the Postgres change** — in
      `storage/src/postgres/backup.rs`: add `use sqlx::error::DatabaseError;` to
      the imports, add a mapping helper, apply it at the insert site, and remove
      the now-covered restore-`Err` `cov:ignore`.

Add the helper (module scope):

```rust
/// Map a Postgres integrity-constraint violation (SQLSTATE class `23`, e.g. `23503`
/// foreign_key_violation) to `ConstraintViolation`, so restore fails uniformly with
/// SQLite (DEC-C). Other sqlx errors pass through unchanged.
fn map_restore_error(error: sqlx::Error) -> BackupError {
    if let sqlx::Error::Database(db_error) = &error {
        if db_error.code().is_some_and(|code| code.starts_with("23")) {
            return BackupError::ConstraintViolation(db_error.message().to_owned());
        }
    }
    BackupError::Sqlx(error)
}
```

In `import_table`, change the insert execution (currently
`query.execute(&mut *connection).await?;`) to:

```rust
        query
            .execute(&mut *connection)
            .await
            .map_err(map_restore_error)?;
```

In `restore_database`, delete the two `// cov:ignore-start` /
`// cov:ignore-stop` comment lines around the `Err(error)` arm (lines ~89 and
~93) — the dangling-FK negative now reaches it.

- [x] **Step 5: Run the test, verify it PASSES on both backends**

Run:
`devtool pg run -- cargo nextest run -p jaunder --test misc cmd_restore_rejects_dangling_foreign_key`
Expected: PASS — both backends return `ConstraintViolation` and leave the target
empty.

- [x] **Step 6: Confirm no regression** in the existing SQLite dialect tests
      (still present until Task 8):

Run: `cargo nextest run -p storage backup` Expected: PASS.

- [x] **Step 7: Commit.**

```bash
git add storage/src/sqlite/backup.rs storage/src/postgres/backup.rs server/tests/misc/commands.rs
git commit -m "fix(backup): reject constraint-violating restores uniformly across backends"
```

Run `cargo xtask check` first.

---

## Task 4: Dual-backend malformed-row rollback negative

Coverage/characterization test — the contract-level, dual-backend replacement
for the deleted `restore_database_triggers_rollback_on_import_failure`. Behavior
already exists; this test pins it on both backends.

**Files:** Test: `server/tests/misc/commands.rs`

- [x] **Step 1: Write the test:**

```rust
// #136: a backup with a malformed row is rejected and rolls back cleanly on both backends.
#[apply(backends)]
#[tokio::test]
async fn cmd_restore_rolls_back_on_malformed_row(#[case] backend: Backend) {
    let base = TempDir::new().expect("temp dir");
    let source_args = storage_args(backend, &base).await;
    cmd_init(&source_args, false).await.expect("init source");
    populate_backup_fixture(&source_args).await;

    let backup_path = base.path().join("backup");
    cmd_backup(&source_args, BackupMode::Directory, Some(backup_path.clone()))
        .await
        .expect("backup");

    // Corrupt a NON-first table (posts, export index 6) with a non-object row, so an
    // earlier table (users, index 1) is inserted before the read fails — proving the
    // transaction rolls the earlier inserts back.
    let posts = backup_path.join("db").join("posts.ndjson");
    let mut contents = std::fs::read_to_string(&posts).expect("read posts");
    contents.push_str("[1, 2, 3]\n");
    std::fs::write(&posts, contents).expect("write tampered posts");

    let target_base = TempDir::new().expect("target temp dir");
    let target_args = storage_args(backend, &target_base).await;
    cmd_init(&target_args, false).await.expect("init target");

    let err = cmd_restore(&target_args, &backup_path)
        .await
        .expect_err("restore rejects malformed row");
    assert!(
        err.to_string().contains("non-object row"),
        "expected InvalidBackup, got: {err}"
    );

    let state = open_existing_database(&target_args.db)
        .await
        .expect("open target");
    let username: Username = "backupuser".parse().expect("valid username");
    assert!(
        state
            .users
            .get_user_by_username(&username)
            .await
            .expect("get user")
            .is_none(),
        "target must be unmodified after a rejected restore"
    );
}
```

- [x] **Step 2: Run it, verify PASS on both backends**

Run:
`devtool pg run -- cargo nextest run -p jaunder --test misc cmd_restore_rolls_back_on_malformed_row`
Expected: PASS — `read_table_rows` rejects the non-object row (`InvalidBackup`)
and the transaction rolls back, so `users` is absent from the target.

- [x] **Step 3: Commit.**

```bash
git add server/tests/misc/commands.rs
git commit -m "test(backup): dual-backend malformed-row restore rollback"
```

Run `cargo xtask check` first.

---

## Task 5: Dual-backend missing-`db/`-directory negative

The contract-level replacement for the deleted
`restore_backup_rejects_missing_db_directory` — covers
`storage/src/backup.rs:204-209`.

**Files:** Test: `server/tests/misc/commands.rs`

- [x] **Step 1: Write the test:**

```rust
// #136: a backup missing its db/ directory is rejected (InvalidBackup) on both backends.
#[apply(backends)]
#[tokio::test]
async fn cmd_restore_rejects_missing_db_directory(#[case] backend: Backend) {
    let base = TempDir::new().expect("temp dir");
    let source_args = storage_args(backend, &base).await;
    cmd_init(&source_args, false).await.expect("init source");
    populate_backup_fixture(&source_args).await;

    let backup_path = base.path().join("backup");
    cmd_backup(&source_args, BackupMode::Directory, Some(backup_path.clone()))
        .await
        .expect("backup");

    std::fs::remove_dir_all(backup_path.join("db")).expect("remove db dir");

    let target_base = TempDir::new().expect("target temp dir");
    let target_args = storage_args(backend, &target_base).await;
    cmd_init(&target_args, false).await.expect("init target");

    let err = cmd_restore(&target_args, &backup_path)
        .await
        .expect_err("restore rejects missing db dir");
    assert!(
        err.to_string().contains("missing db directory"),
        "expected InvalidBackup, got: {err}"
    );
}
```

- [x] **Step 2: Run it, verify PASS on both backends**

Run:
`devtool pg run -- cargo nextest run -p jaunder --test misc cmd_restore_rejects_missing_db_directory`
Expected: PASS — manifest reads OK, then `restore_directory_backup` finds no
`db/` and returns `InvalidBackup("missing db directory: …")`.

- [x] **Step 3: Commit.**

```bash
git add server/tests/misc/commands.rs
git commit -m "test(backup): dual-backend missing-db-directory rejection"
```

Run `cargo xtask check` first.

---

## Task 6: Dual-backend archive round-trip

Closes the archive-mode gap (deleted
`archive_backup_round_trips_database_and_media`; `commands.rs` is
Directory-only). Covers archive export + extraction on restore.

**Files:** Test: `server/tests/misc/commands.rs`

- [x] **Step 1: Write the test:**

```rust
// #136: backup/restore round-trips in Archive mode on both backends.
#[apply(backends)]
#[tokio::test]
async fn cmd_restore_restores_archive_backup(#[case] backend: Backend) {
    let base = TempDir::new().expect("temp dir");
    let source_args = storage_args(backend, &base).await;
    cmd_init(&source_args, false).await.expect("init source");
    let post_id = populate_backup_fixture(&source_args).await;

    let archive_path = base.path().join("backup.tar.gz");
    cmd_backup(&source_args, BackupMode::Archive, Some(archive_path.clone()))
        .await
        .expect("backup");
    assert!(archive_path.is_file(), "archive backup is a single file");

    let target_base = TempDir::new().expect("target temp dir");
    let target_args = storage_args(backend, &target_base).await;
    cmd_init(&target_args, false).await.expect("init target");
    cmd_restore(&target_args, &archive_path)
        .await
        .expect("restore");

    assert_backup_fixture_restored(&target_args, post_id).await;
}
```

- [x] **Step 2: Run it, verify PASS on both backends**

Run:
`devtool pg run -- cargo nextest run -p jaunder --test misc cmd_restore_restores_archive_backup`
Expected: PASS.

- [x] **Step 3: Commit.**

```bash
git add server/tests/misc/commands.rs
git commit -m "test(backup): dual-backend archive-mode round-trip"
```

Run `cargo xtask check` first.

---

## Task 7: A→B→A→B cross-backend cycle

**Files:** Modify: `server/tests/misc/backup_interop.rs` (add
`use std::path::Path;`, the cycle test, and two dump helpers)

**Interfaces:**

- Consumes: `sqlite_storage_args`/`postgres_storage_args` (`backup_interop.rs`),
  `populate_backup_fixture`/`assert_backup_fixture_restored` (Task 2),
  `postgres_testing_enabled`, `Backend`, `serde_json`.

- [x] **Step 1: Add the dump-equality helpers** (module scope in
      `backup_interop.rs`):

```rust
/// Assert two backup directories are byte-identical over `db/*.ndjson` and
/// `manifest.json` with only its (wall-clock) `timestamp` excluded.
fn assert_backups_equal(left: &Path, right: &Path) {
    let mut left_tables: Vec<_> = std::fs::read_dir(left.join("db"))
        .expect("read left db")
        .map(|e| e.expect("entry").file_name())
        .collect();
    let mut right_tables: Vec<_> = std::fs::read_dir(right.join("db"))
        .expect("read right db")
        .map(|e| e.expect("entry").file_name())
        .collect();
    left_tables.sort();
    right_tables.sort();
    assert_eq!(left_tables, right_tables, "db table file sets differ");
    for name in left_tables {
        assert_eq!(
            std::fs::read(left.join("db").join(&name)).expect("read left table"),
            std::fs::read(right.join("db").join(&name)).expect("read right table"),
            "table {name:?} differs between dumps"
        );
    }
    assert_eq!(
        manifest_without_timestamp(left),
        manifest_without_timestamp(right),
        "manifest differs (excluding timestamp)"
    );
}

fn manifest_without_timestamp(dir: &Path) -> serde_json::Value {
    let text = std::fs::read_to_string(dir.join("manifest.json")).expect("read manifest");
    let mut value: serde_json::Value = serde_json::from_str(&text).expect("parse manifest");
    value
        .as_object_mut()
        .expect("manifest is a JSON object")
        .remove("timestamp");
    value
}
```

- [x] **Step 2: Add the cycle test:**

```rust
#[apply(postgres_only)]
// reason: the A→B→A→B cycle exercises BOTH engines in one test, so it needs a live Postgres.
#[tokio::test]
async fn backup_round_trips_full_cycle_across_backends(#[case] backend: Backend) {
    let _ = backend;
    if !postgres_testing_enabled() {
        return;
    }

    let base = TempDir::new().expect("temp dir");

    // A (sqlite): seed, export E_A1.
    let a1 = sqlite_storage_args(&base, "a1");
    cmd_init(&a1, false).await.expect("init a1");
    let post_id = populate_backup_fixture(&a1).await;
    let dir_a1 = base.path().join("dir-a1");
    cmd_backup(&a1, BackupMode::Directory, Some(dir_a1.clone()))
        .await
        .expect("backup a1");

    // B (postgres): restore, assert, export E_B1.
    let b1 = postgres_storage_args(&base, "b1").await;
    cmd_init(&b1, false).await.expect("init b1");
    cmd_restore(&b1, &dir_a1).await.expect("restore into b1");
    assert_backup_fixture_restored(&b1, post_id).await;
    let dir_b1 = base.path().join("dir-b1");
    cmd_backup(&b1, BackupMode::Directory, Some(dir_b1.clone()))
        .await
        .expect("backup b1");

    // A2 (sqlite): restore, assert, export E_A2.
    let a2 = sqlite_storage_args(&base, "a2");
    cmd_init(&a2, false).await.expect("init a2");
    cmd_restore(&a2, &dir_b1).await.expect("restore into a2");
    assert_backup_fixture_restored(&a2, post_id).await;
    let dir_a2 = base.path().join("dir-a2");
    cmd_backup(&a2, BackupMode::Directory, Some(dir_a2.clone()))
        .await
        .expect("backup a2");

    // B2 (postgres): restore, assert, export E_B2.
    let b2 = postgres_storage_args(&base, "b2").await;
    cmd_init(&b2, false).await.expect("init b2");
    cmd_restore(&b2, &dir_a2).await.expect("restore into b2");
    assert_backup_fixture_restored(&b2, post_id).await;
    let dir_b2 = base.path().join("dir-b2");
    cmd_backup(&b2, BackupMode::Directory, Some(dir_b2.clone()))
        .await
        .expect("backup b2");

    // Sound floor: same-backend Postgres dumps are byte-identical.
    assert_backups_equal(&dir_b1, &dir_b2);

    // Empirical (DEC-D): same-backend SQLite dumps across the round-trip. See Step 4.
    assert_backups_equal(&dir_a1, &dir_a2);
}
```

- [x] **Step 3: Run it**

Run:
`devtool pg run -- cargo nextest run -p jaunder --test misc backup_round_trips_full_cycle_across_backends`
Expected: the four functional asserts and `E_B₁==E_B₂` PASS.

- [x] **Step 4: Resolve the empirical `E_A₁==E_A₂` assertion.** If Step 3 passed
      entirely, keep the `assert_backups_equal(&dir_a1, &dir_a2)` line. If
      **only** that line failed (SQLite dumps differ across the Postgres
      round-trip — a timestamp reserialization difference, DEC-D), replace it
      with:

```rust
    // note: SQLite's app-written timestamp text and its post-Postgres-round-trip text
    // differ byte-for-byte (cosmetic re-serialization). Value fidelity is proven by the
    // assert_backup_fixture_restored calls above; E_B1==E_B2 is the byte-level floor.
```

Re-run Step 3 and confirm PASS.

- [x] **Step 5: Commit.**

```bash
git add server/tests/misc/backup_interop.rs
git commit -m "test(backup): A→B→A→B cross-backend cycle proves value + dump fidelity"
```

Run `cargo xtask check` first.

---

## Task 8: Delete the interim tests; cov:ignore the SQLite export arm

Now that every coverage source is restored (Tasks 3–6) plus transitive coverage
of `schema_version`/`schema_checksum` from round-trips, remove the interim
tests.

**Files:**

- Modify: `storage/src/backup.rs` (delete 3 tests + unused test imports)
- Modify: `storage/src/sqlite/backup.rs` (delete 5 tests +
  `migrated_pool`/`migrated_conn`
  - unused test imports; cov:ignore the export `Err` arm)

- [ ] **Step 1: Delete the 3 orchestration tests** in `storage/src/backup.rs` —
      `restore_backup_rejects_missing_db_directory`,
      `export_backup_writes_ndjson_media_and_manifest`,
      `archive_backup_round_trips_database_and_media` (the
      `#[apply(sqlite_only)]` block, lines ~930–1082). Keep every pure `#[test]`
      helper test. In the `mod tests` `use` block, remove now-unused imports
      (`sqlite_only`, `Backend`, the `rstest`/`rstest_reuse` globs,
      `std::str::FromStr` — whatever clippy flags after deletion).

- [ ] **Step 2: Delete the 5 dialect tests + helpers** in
      `storage/src/sqlite/backup.rs` —
      `export_database_triggers_rollback_on_write_failure`,
      `restore_database_triggers_rollback_on_import_failure`,
      `validate_foreign_keys_reports_violations`,
      `schema_version_returns_migration_count`,
      `schema_checksum_returns_nonempty_hex_string`, **and** the `migrated_pool`
      / `migrated_conn` helpers. Keep the 3 pure `#[test]` tests
      (`json_select_marks_boolean_values_as_json_booleans`,
      `insert_sql_uses_numbered_placeholders`,
      `bind_json_value_accepts_all_json_shapes`). Remove now-unused `use`
      imports (`sqlite_only`, `Backend`, `rstest`/`rstest_reuse`,
      `sqlx::Connection`, `TempDir` — whatever clippy flags).

- [ ] **Step 3: cov:ignore the SQLite export rollback arm** in
      `storage/src/sqlite/backup.rs`, `export_database` (the `Err(error) =>`
      arm, lines ~55–58), matching the existing Postgres marker:

```rust
        // cov:ignore-start — export rollback is unreachable through public export_backup
        // (export_directory_backup always creates the db/ subdir before the dialect
        // writes); no host-side trigger. Parity with postgres/backup.rs.
        Err(error) => {
            let _ = sqlx::query("ROLLBACK").execute(&mut *connection).await;
            Err(error)
            // cov:ignore-stop
        }
```

- [ ] **Step 4: Run the storage tests + the full gate** (coverage must not
      regress):

Run: `cargo nextest run -p storage backup` Expected: PASS (only the pure helper
tests remain).

Run: `devtool run -- cargo xtask check` Expected: `ok:true` — coverage green, no
CRAP regression, `test-backend-pattern` guard passes (no bare DB
`#[tokio::test]`).

- [ ] **Step 5: Commit.**

```bash
git add storage/src/backup.rs storage/src/sqlite/backup.rs
git commit -m "test(backup): drop interim sqlite_only tests; cov:ignore export rollback arm"
```

---

## Task 9: ADR draft — backup test homing + uniform restore failure

**Files:** Create:
`docs/adr/drafts/backup-test-homing-and-uniform-restore-failure.md`

- [ ] **Step 1: Author the ADR draft** via **jaunder-adr** (numberless draft in
      `docs/adr/drafts/`; `cargo xtask adr promote` numbers it at ship). Record:
  - **Homing (DEC-A/DEC-B):** backup contract tests live at the CLI/server-test
    level (`commands.rs` + `backup_interop.rs`), not rebuilt in the storage
    crate — the dual-backend round-trip already lives there and the CLI is a
    thin wrapper over `export_backup`/`restore_backup`; placement is
    coverage-neutral (ADR-0053). Keep both single-hop cross-backend tests
    **and** the A→B→A→B cycle.
  - **Uniform restore-failure contract (DEC-C):** a constraint-violating restore
    fails as `ConstraintViolation` with the target unmodified on both backends —
    Postgres maps SQLSTATE-`23`; SQLite validates before `COMMIT` and rolls back
    (fixing the prior commit-then-check wart). Reference the interop
    value-fidelity result (DEC-D): interop is value-level; cross-backend dump
    bytes may differ (canonicalization is a non-goal).
  - Amends/extends ADR-0053's backup carve-out. Link issue #136 and issue #4.

- [ ] **Step 2: Format + commit** (prettier the Markdown before staging —
      jaunder pre-commit restages prose):

```bash
git add docs/adr/drafts/backup-test-homing-and-uniform-restore-failure.md
git commit -m "docs(adr): draft backup test-homing + uniform restore-failure contract"
```

Run `cargo xtask check` first.

---

## Done criteria

All of spec AC1–AC13 hold; `devtool run -- cargo xtask validate --no-e2e` is
green. Hand off to **jaunder-ship** for final review, plan/spec archiving, PR,
and merge (which releases issue #136 to Done and promotes the ADR draft).
