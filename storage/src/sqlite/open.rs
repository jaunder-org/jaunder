use std::{sync::Arc, time::Duration};

use log::LevelFilter;
use sqlx::{
    ConnectOptions, SqlitePool,
    sqlite::{SqliteConnectOptions, SqliteJournalMode},
};

use crate::backup::CatalogTableName;
use crate::db::StorageRuntimeConfig;
use crate::posts::media;
use crate::sql::Exists;
use crate::{AppState, instance_identity, make_app_state};

/// Resolves application `SQLite` options from the runtime connection snapshot.
#[must_use]
pub(crate) fn resolved_sqlite_options(
    options: &SqliteConnectOptions,
    runtime: &StorageRuntimeConfig,
) -> SqliteConnectOptions {
    options
        .clone()
        .log_slow_statements(LevelFilter::Warn, runtime.sql_slow_query_threshold())
}

#[tracing::instrument(
    name = "storage.sqlite.open_database",
    skip(options, runtime),
    fields(create_if_missing)
)]
pub(crate) async fn open_sqlite_database_with_pool(
    options: &SqliteConnectOptions,
    create_if_missing: bool,
    runtime: &StorageRuntimeConfig,
) -> sqlx::Result<(Arc<AppState>, SqlitePool, crate::InstanceId)> {
    let mut options = resolved_sqlite_options(options, runtime);
    if create_if_missing {
        options = options.create_if_missing(true);
    }
    // WAL mode allows concurrent readers while a writer is active, dramatically
    // reducing SQLITE_BUSY errors under load. The busy timeout lets SQLite retry
    // automatically instead of failing immediately when it cannot obtain a lock.
    options = options
        .journal_mode(SqliteJournalMode::Wal)
        .busy_timeout(Duration::from_secs(5));
    let pool = sqlx::SqlitePool::connect_with(options).await?;

    // Increase cache size to 32MB. SQLite page size is 4KB by default (usually),
    // so 32MB is 8192 pages. The `-32000` syntax tells SQLite 32MB.
    sqlx::query("PRAGMA cache_size = -32000")
        .execute(&pool)
        .await?;

    sqlx::migrate!("./migrations/sqlite").run(&pool).await?;
    let instance_id = instance_identity::ensure(&pool).await?;
    media::backfill_post_media_references(&pool).await?;
    Ok((make_app_state(pool.clone()), pool, instance_id))
}

/// Returns `true` if the `SQLite` database holds no user data — every table
/// except the migration-seeded lookups is empty.
pub(crate) async fn database_is_empty(
    options: &SqliteConnectOptions,
    runtime: &StorageRuntimeConfig,
) -> sqlx::Result<bool> {
    let pool = SqlitePool::connect_with(resolved_sqlite_options(options, runtime)).await?;
    let tables = sqlx::query_scalar::<_, CatalogTableName>(
        "SELECT name FROM sqlite_master \
         WHERE type = 'table' AND name NOT LIKE 'sqlite_%' AND name <> '_sqlx_migrations'",
    )
    .fetch_all(&pool)
    .await?;
    for table in tables {
        if crate::db::MIGRATION_SEEDED_TABLES.contains(&table.as_str()) {
            continue;
        }
        let has_row = sqlx::query_scalar::<_, Exists>(&format!(
            "SELECT EXISTS(SELECT 1 FROM {} LIMIT 1)",
            crate::sql::quote_identifier(table.as_str())
        ))
        .fetch_one(&pool)
        .await?
        .into_bool();
        if has_row {
            return Ok(false);
        }
    }
    Ok(true)
}
