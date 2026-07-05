#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::too_many_lines,
    clippy::similar_names,
    clippy::items_after_statements,
    clippy::unused_async
)]

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
