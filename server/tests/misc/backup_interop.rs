#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::too_many_lines,
    clippy::similar_names,
    clippy::items_after_statements,
    clippy::unused_async
)]

use std::path::Path;

use jaunder::cli::StorageArgs;
use jaunder::commands::{cmd_backup, cmd_init, cmd_restore};
use storage::BackupMode;
use tempfile::TempDir;

use crate::backup_fixture::{assert_backup_fixture_restored, populate_backup_fixture};

use rstest::*;
#[allow(clippy::single_component_path_imports)]
use rstest_reuse;
use rstest_reuse::*;

use crate::helpers::{postgres_only, unique_postgres_url, Backend, PostgresDbGuard};

fn sqlite_storage_args(base: &TempDir, name: &str) -> StorageArgs {
    StorageArgs {
        storage_path: base.path().join(format!("{name}-storage")),
        db: format!(
            "sqlite:{}",
            base.path().join(format!("{name}.db")).display()
        )
        .parse()
        .expect("sqlite db"),
    }
}

async fn postgres_storage_args(base: &TempDir, name: &str) -> (StorageArgs, PostgresDbGuard) {
    let (db, guard) = unique_postgres_url().await;
    (
        StorageArgs {
            storage_path: base.path().join(format!("{name}-storage")),
            db,
        },
        guard,
    )
}

#[apply(postgres_only)]
// reason: cross-backend backup interop exercises BOTH engines in one test
// (SQLite source restored into Postgres target), so it needs a live Postgres.
#[tokio::test]
async fn sqlite_backup_restores_into_postgres(#[case] backend: Backend) {
    let _ = backend;

    let base = TempDir::new().expect("temp dir");
    let source_args = sqlite_storage_args(&base, "sqlite-source");
    cmd_init(&source_args, false)
        .await
        .expect("init sqlite source");
    let post_id = populate_backup_fixture(&source_args).await;

    let backup_path = base.path().join("sqlite-backup");
    cmd_backup(
        &source_args,
        BackupMode::Directory,
        Some(backup_path.clone()),
    )
    .await
    .expect("sqlite backup");

    let (target_args, _pg_target) = postgres_storage_args(&base, "postgres-target").await;
    cmd_init(&target_args, false)
        .await
        .expect("init postgres target");
    cmd_restore(&target_args, &backup_path)
        .await
        .expect("restore into postgres");

    assert_backup_fixture_restored(&target_args, post_id).await;
}

#[apply(postgres_only)]
// reason: cross-backend backup interop exercises BOTH engines in one test
// (Postgres source restored into SQLite target), so it needs a live Postgres.
#[tokio::test]
async fn postgres_backup_restores_into_sqlite(#[case] backend: Backend) {
    let _ = backend;

    let base = TempDir::new().expect("temp dir");
    let (source_args, _pg_source) = postgres_storage_args(&base, "postgres-source").await;
    cmd_init(&source_args, false)
        .await
        .expect("init postgres source");
    let post_id = populate_backup_fixture(&source_args).await;

    let backup_path = base.path().join("postgres-backup");
    cmd_backup(
        &source_args,
        BackupMode::Directory,
        Some(backup_path.clone()),
    )
    .await
    .expect("postgres backup");

    let target_args = sqlite_storage_args(&base, "sqlite-target");
    cmd_init(&target_args, false)
        .await
        .expect("init sqlite target");
    cmd_restore(&target_args, &backup_path)
        .await
        .expect("restore into sqlite");

    assert_backup_fixture_restored(&target_args, post_id).await;
}

/// Assert two backup directories are byte-identical over `db/*.ndjson` and
/// `manifest.json` with only its (wall-clock) `timestamp` field excluded.
fn assert_backups_equal(left: &Path, right: &Path) {
    let mut left_tables: Vec<_> = std::fs::read_dir(left.join("db"))
        .expect("read left db")
        .map(|entry| entry.expect("entry").file_name())
        .collect();
    let mut right_tables: Vec<_> = std::fs::read_dir(right.join("db"))
        .expect("read right db")
        .map(|entry| entry.expect("entry").file_name())
        .collect();
    left_tables.sort();
    right_tables.sort();
    assert_eq!(left_tables, right_tables, "db table file sets differ");
    for name in left_tables {
        assert_eq!(
            std::fs::read(left.join("db").join(&name)).expect("read left table"),
            std::fs::read(right.join("db").join(&name)).expect("read right table"),
            "table {} differs between dumps",
            name.display()
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

// #136: an A→B→A→B cross-backend cycle proves value fidelity through four hops and
// byte-stable Postgres dumps (E_B1 == E_B2). See the spec's "A→B→A→B full-cycle
// fidelity" section.
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

    // note (DEC-D, verified here): the SQLite `db/*.ndjson` dumps do NOT survive a
    // Postgres round-trip byte-for-byte, so `E_A1 == E_A2` is intentionally NOT
    // asserted. `created_at`/`updated_at` are app-written with nanosecond precision on
    // SQLite but quantized to microseconds by Postgres `timestamptz`
    // (e.g. `…38.696740562` → `…38.696741`). This is cosmetic: value fidelity is proven
    // by the `assert_backup_fixture_restored` calls above (including `published_at`,
    // seeded at µs precision), and `E_B1 == E_B2` is the byte-level floor.
}
