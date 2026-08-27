use std::{sync::Arc, time::Duration};

use log::LevelFilter;
use sqlx::{
    ConnectOptions, SqlitePool,
    sqlite::{SqliteConnectOptions, SqliteJournalMode},
};

use super::{
    SqliteAtomicOps, SqliteAudienceStorage, SqliteEmailVerificationStorage, SqliteFeedCacheStorage,
    SqliteFeedEventStorage, SqliteInviteStorage, SqliteMediaStorage, SqlitePasswordResetStorage,
    SqlitePostStorage, SqliteSessionStorage, SqliteSiteConfigStorage, SqliteSubscriptionStorage,
    SqliteUserConfigStorage, SqliteUserStorage,
};
use crate::AppState;
use crate::db::StorageRuntimeConfig;
use crate::instance_identity::ensure_instance_identity;
use crate::posts::backfill_post_media_references;

fn make_sqlite_app_state(pool: SqlitePool) -> Arc<AppState> {
    Arc::new(AppState {
        site_config: Arc::new(SqliteSiteConfigStorage::new(pool.clone())),
        users: Arc::new(SqliteUserStorage::new(pool.clone())),
        sessions: Arc::new(SqliteSessionStorage::new(pool.clone())),
        invites: Arc::new(SqliteInviteStorage::new(pool.clone())),
        atomic: Arc::new(SqliteAtomicOps::new(pool.clone())),
        email_verifications: Arc::new(SqliteEmailVerificationStorage::new(pool.clone())),
        password_resets: Arc::new(SqlitePasswordResetStorage::new(pool.clone())),
        posts: Arc::new(SqlitePostStorage::new(pool.clone())),
        subscriptions: Arc::new(SqliteSubscriptionStorage::new(
            pool.clone(),
            Arc::new(common::visibility::OpenSubscriptionPolicy),
        )),
        audiences: Arc::new(SqliteAudienceStorage::new(pool.clone())),
        media: Arc::new(SqliteMediaStorage::new(pool.clone())),
        user_config: Arc::new(SqliteUserConfigStorage::new(pool.clone())),
        feed_cache: Arc::new(SqliteFeedCacheStorage::new(pool.clone())),
        feed_events: Arc::new(SqliteFeedEventStorage::new(pool)),
    })
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
    let mut options = options.clone();
    if create_if_missing {
        options = options.create_if_missing(true);
    }
    // WAL mode allows concurrent readers while a writer is active, dramatically
    // reducing SQLITE_BUSY errors under load. The busy timeout lets SQLite retry
    // automatically instead of failing immediately when it cannot obtain a lock.
    options = options
        .journal_mode(SqliteJournalMode::Wal)
        .busy_timeout(Duration::from_secs(5))
        .log_slow_statements(LevelFilter::Warn, runtime.sql_slow_query_threshold());
    let pool = sqlx::SqlitePool::connect_with(options).await?;

    // Increase cache size to 32MB. SQLite page size is 4KB by default (usually),
    // so 32MB is 8192 pages. The `-32000` syntax tells SQLite 32MB.
    sqlx::query("PRAGMA cache_size = -32000")
        .execute(&pool)
        .await?;

    sqlx::migrate!("./migrations/sqlite").run(&pool).await?;
    let instance_id = ensure_instance_identity(&pool).await?;
    backfill_post_media_references(&pool).await?;
    Ok((make_sqlite_app_state(pool.clone()), pool, instance_id))
}

/// Returns `true` if the `SQLite` database holds no user data — every table
/// except the migration-seeded lookups is empty.
pub(crate) async fn database_is_empty(options: &SqliteConnectOptions) -> sqlx::Result<bool> {
    let pool = SqlitePool::connect_with(options.clone()).await?;
    let tables = sqlx::query_scalar::<_, String>(
        "SELECT name FROM sqlite_master \
         WHERE type = 'table' AND name NOT LIKE 'sqlite_%' AND name <> '_sqlx_migrations'",
    )
    .fetch_all(&pool)
    .await?;
    for table in tables {
        if crate::db::MIGRATION_SEEDED_TABLES.contains(&table.as_str()) {
            continue;
        }
        let has_row = sqlx::query_scalar::<_, i64>(&format!(
            "SELECT EXISTS(SELECT 1 FROM {} LIMIT 1)",
            crate::sql::quote_identifier(&table)
        ))
        .fetch_one(&pool)
        .await?
            != 0;
        if has_row {
            return Ok(false);
        }
    }
    Ok(true)
}
