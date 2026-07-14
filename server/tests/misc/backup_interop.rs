use std::path::Path;

use jaunder::cli::StorageArgs;
use jaunder::commands::{cmd_backup, cmd_init, cmd_restore};
use storage::BackupMode;
use tempfile::TempDir;

use crate::misc::backup_fixture::{assert_backup_fixture_restored, populate_backup_fixture};

use storage::test_support::{unique_postgres_url, PostgresDbGuard};

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

// guard:low-level-db — cross-backend interop; drives both engines in one body and needs a live Postgres
#[tokio::test]
async fn sqlite_backup_restores_into_postgres() {
    let base = TempDir::new().expect("temp dir");
    let source_args = sqlite_storage_args(&base, "sqlite-source");
    cmd_init(&source_args, false)
        .await
        .expect("init sqlite source");
    let ids = populate_backup_fixture(&source_args).await;

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

    assert_backup_fixture_restored(&target_args, &ids).await;
}

// guard:low-level-db — cross-backend interop; drives both engines in one body and needs a live Postgres
#[tokio::test]
async fn postgres_backup_restores_into_sqlite() {
    let base = TempDir::new().expect("temp dir");
    let (source_args, _pg_source) = postgres_storage_args(&base, "postgres-source").await;
    cmd_init(&source_args, false)
        .await
        .expect("init postgres source");
    let ids = populate_backup_fixture(&source_args).await;

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

    assert_backup_fixture_restored(&target_args, &ids).await;
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

// #136: a Postgres→SQLite→Postgres→SQLite cycle proves value fidelity through four hops
// AND byte-stable dumps on BOTH backends. Seeding from Postgres is deliberate: Postgres
// `timestamptz` stores at microsecond resolution — the coarser of the two backends — so
// every timestamp is pinned at µs from the first store and no later hop quantizes it
// further. SQLite stores restored text verbatim and never truncates, so both same-backend
// dump pairs stay byte-identical. (Seeding from SQLite instead would lose sub-µs precision
// of `created_at`/`updated_at` on the first SQLite→Postgres hop — see ADR-0054 DEC-D — and
// only the Postgres pair would be byte-comparable.)
// guard:low-level-db — cross-backend interop; drives both engines in one body and needs a live Postgres
#[tokio::test]
async fn backup_round_trips_full_cycle_across_backends() {
    let base = TempDir::new().expect("temp dir");

    // P1 (postgres): seed, export E_P1.
    let (p1, _pg_p1) = postgres_storage_args(&base, "p1").await;
    cmd_init(&p1, false).await.expect("init p1");
    let ids = populate_backup_fixture(&p1).await;
    let pg_seed_export = base.path().join("dir-p1");
    cmd_backup(&p1, BackupMode::Directory, Some(pg_seed_export.clone()))
        .await
        .expect("backup p1");

    // S1 (sqlite): restore, assert, export E_S1.
    let s1 = sqlite_storage_args(&base, "s1");
    cmd_init(&s1, false).await.expect("init s1");
    cmd_restore(&s1, &pg_seed_export)
        .await
        .expect("restore into s1");
    assert_backup_fixture_restored(&s1, &ids).await;
    let sqlite_relay_export = base.path().join("dir-s1");
    cmd_backup(
        &s1,
        BackupMode::Directory,
        Some(sqlite_relay_export.clone()),
    )
    .await
    .expect("backup s1");

    // P2 (postgres): restore, assert, export E_P2.
    let (p2, _pg_p2) = postgres_storage_args(&base, "p2").await;
    cmd_init(&p2, false).await.expect("init p2");
    cmd_restore(&p2, &sqlite_relay_export)
        .await
        .expect("restore into p2");
    assert_backup_fixture_restored(&p2, &ids).await;
    let pg_return_export = base.path().join("dir-p2");
    cmd_backup(&p2, BackupMode::Directory, Some(pg_return_export.clone()))
        .await
        .expect("backup p2");

    // S2 (sqlite): restore, assert, export E_S2.
    let s2 = sqlite_storage_args(&base, "s2");
    cmd_init(&s2, false).await.expect("init s2");
    cmd_restore(&s2, &pg_return_export)
        .await
        .expect("restore into s2");
    assert_backup_fixture_restored(&s2, &ids).await;
    let sqlite_final_export = base.path().join("dir-s2");
    cmd_backup(
        &s2,
        BackupMode::Directory,
        Some(sqlite_final_export.clone()),
    )
    .await
    .expect("backup s2");

    // Both same-backend dump pairs are byte-identical — nothing drifts across the cycle.
    assert_backups_equal(&pg_seed_export, &pg_return_export);
    assert_backups_equal(&sqlite_relay_export, &sqlite_final_export);
}
