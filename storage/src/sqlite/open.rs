use std::time::Duration;

use log::LevelFilter;
use sqlx::{
    AssertSqlSafe, ConnectOptions, SqlitePool,
    sqlite::{SqliteConnectOptions, SqliteJournalMode},
};

use crate::backup::CatalogTableName;
use crate::code_migrations;
use crate::db::StorageRuntimeConfig;
use crate::sql::{Exists, QueryStorageExt, RowCount};
use crate::{StorageFactory, instance_identity};

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
    skip(options, runtime, authorize_drain),
    fields(create_if_missing)
)]
pub(crate) async fn open_sqlite_database_with_pool(
    options: &SqliteConnectOptions,
    create_if_missing: bool,
    runtime: &StorageRuntimeConfig,
    authorize_drain: &(dyn Fn() -> sqlx::Result<()> + Sync),
) -> sqlx::Result<(StorageFactory, SqlitePool, crate::InstanceId)> {
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

    // Report legacy fallback rows only after the paired schema migration succeeds.
    let timezone_fallback_rows = publication_timezone_fallback_rows(&pool).await?;
    sqlx::migrate!("./migrations/sqlite").run(&pool).await?;
    if timezone_fallback_rows > 0 {
        tracing::warn!(
            timezone_fallback_rows,
            "publication timezone migration used UTC fallback for existing Post and revision rows"
        );
    }
    let instance_id = instance_identity::ensure(&pool).await?;
    {
        let mut connection = pool.acquire().await?;
        let (pending,): (crate::sql::Exists,) = sqlx::query_as("SELECT EXISTS(SELECT 1 FROM pending_code_migrations WHERE operation = 'normalize_comparable_instants_v1')").fetch_one(&mut *connection).await?;
        code_migrations::check_sqlite_enforcement(&mut connection, pending.into_bool()).await?;
    }
    code_migrations::drain_pending(&pool, authorize_drain).await?;
    code_migrations::check_sqlite_enforcement(&mut *pool.acquire().await?, false).await?;
    crate::posts::search::backfill_post_search_projections(&pool).await?;
    Ok((StorageFactory::sqlite(pool.clone()), pool, instance_id))
}

/// Counts rows requiring the explicit UTC fallback before the timezone upgrade.
pub(crate) async fn publication_timezone_fallback_rows(pool: &SqlitePool) -> sqlx::Result<u64> {
    let mut rows = 0_u64;
    for table in ["posts", "post_revisions"] {
        let legacy: Exists = sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE type = 'table' AND name = $1) AND NOT EXISTS(SELECT 1 FROM pragma_table_info($1) WHERE name = 'timezone')")
            .bind_storage(CatalogTableName::from(table)).fetch_one(pool).await?;
        if legacy.into_bool() {
            let count: RowCount =
                sqlx::query_scalar(AssertSqlSafe(format!("SELECT COUNT(*) FROM {table}")))
                    .fetch_one(pool)
                    .await?;
            rows += count.into_u64();
        }
    }
    Ok(rows)
}

/// Returns `true` if the `SQLite` database holds no user data — every table
/// except the migration/identity bootstrap tables is empty.
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
        // Catalog names are quoted as SQLite identifiers, so the constructed
        // structural query cannot reinterpret a table name as SQL syntax.
        let sql = format!(
            "SELECT EXISTS(SELECT 1 FROM {} LIMIT 1)",
            crate::sql::quote_identifier(table.as_str())
        );
        let has_row = sqlx::query_scalar::<_, Exists>(AssertSqlSafe(sql))
            .fetch_one(&pool)
            .await?
            .into_bool();
        if has_row {
            return Ok(false);
        }
    }
    Ok(true)
}
