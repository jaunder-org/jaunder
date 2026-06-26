//! Shared helpers for the `server` crate's in-crate unit tests.
//!
//! These build a migrated `SQLite` database and hand back the *narrow* storage
//! handles a test needs, rather than the whole `AppState`. A test for a
//! constructor-injected subsystem should construct exactly the handles that
//! subsystem (and its fixtures) touch — see [ADR-0016]. Integration tests
//! (`server/tests/`) instead use the backend-parametric `helpers::test_state*`,
//! which exercise `SQLite` **and** `PostgreSQL`.
//!
//! [ADR-0016]: ../../docs/adr/0016-dependency-injection-and-appstate.md

use std::path::Path;
use std::sync::Arc;

use storage::{
    DbConnectOptions, MediaStorage, SiteConfigStorage, SqliteMediaStorage, SqliteSiteConfigStorage,
    SqliteUserStorage, UserStorage,
};

/// Opens a `SQLite` pool at `db_path` and runs migrations, returning the pool.
pub(crate) async fn migrated_sqlite_pool(db_path: &Path) -> sqlx::SqlitePool {
    let pool = sqlx::SqlitePool::connect_with(
        format!("sqlite:{}", db_path.display())
            .parse::<sqlx::sqlite::SqliteConnectOptions>()
            .expect("sqlite options")
            .create_if_missing(true),
    )
    .await
    .expect("connect sqlite");
    sqlx::migrate!("../storage/migrations/sqlite")
        .run(&pool)
        .await
        .expect("run migrations");
    pool
}

/// Creates a migrated `jaunder.db` inside `dir`, returning its connect options
/// (for handing to a subsystem that opens its own connection, e.g. the backup
/// worker) alongside an open pool (for building storage handles on the same DB).
pub(crate) async fn migrated_sqlite_db(dir: &Path) -> (DbConnectOptions, sqlx::SqlitePool) {
    let db_path = dir.join("jaunder.db");
    let options = format!("sqlite:{}", db_path.display())
        .parse()
        .expect("db options");
    let pool = migrated_sqlite_pool(&db_path).await;
    (options, pool)
}

/// The site-config store on `pool`.
pub(crate) fn site_config(pool: &sqlx::SqlitePool) -> Arc<dyn SiteConfigStorage> {
    Arc::new(SqliteSiteConfigStorage::new(pool.clone()))
}

/// The media store on `pool`.
pub(crate) fn media(pool: &sqlx::SqlitePool) -> Arc<dyn MediaStorage> {
    Arc::new(SqliteMediaStorage::new(pool.clone()))
}

/// The user store on `pool`.
pub(crate) fn users(pool: &sqlx::SqlitePool) -> Arc<dyn UserStorage> {
    Arc::new(SqliteUserStorage::new(pool.clone()))
}
