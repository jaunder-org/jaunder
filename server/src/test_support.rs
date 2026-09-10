//! Test-only construction seams for the `server` crate.
//!
//! Integration tests compile `server` as a dependency, so their router seam must
//! remain externally reachable. It lives here rather than expanding the
//! production router API at the crate root.
//!
//! The in-crate helpers build a migrated `SQLite` database and hand back the
//! *narrow* storage handles a test needs. A test for a constructor-injected
//! subsystem should construct exactly the handles that subsystem (and its
//! fixtures) touch — see [ADR-0016]. Integration tests
//! (`server/tests/`) otherwise use the backend-parametric
//! `storage::test_support::Backend`, which exercises `SQLite` and `PostgreSQL`.
//!
//! [ADR-0016]: ../../docs/adr/0016-dependency-injection-and-appstate.md

#[cfg(test)]
use std::sync::Arc;

#[cfg(test)]
use std::path::Path;

#[cfg(test)]
use storage::{DbConnectOptions, SiteConfigStorage, SqliteSiteConfigStorage};

#[cfg(test)]
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

#[cfg(test)]
/// Connect options for `jaunder.db` inside `dir`.
pub(crate) fn sqlite_db_options(dir: &Path) -> DbConnectOptions {
    format!("sqlite:{}", dir.join("jaunder.db").display())
        .parse()
        .expect("db options")
}

#[cfg(test)]
/// Creates a migrated `jaunder.db` inside `dir`, returning its connect options
/// (for handing to a subsystem that opens its own connection, e.g. the backup
/// worker) alongside an open pool (for building storage handles on the same DB).
pub(crate) async fn migrated_sqlite_db(dir: &Path) -> (DbConnectOptions, sqlx::SqlitePool) {
    let db_path = dir.join("jaunder.db");
    let options = sqlite_db_options(dir);
    let pool = migrated_sqlite_pool(&db_path).await;
    (options, pool)
}

#[cfg(test)]
/// The site-config store on `pool`.
pub(crate) fn site_config(pool: &sqlx::SqlitePool) -> Arc<dyn SiteConfigStorage> {
    Arc::new(SqliteSiteConfigStorage::new(pool.clone()))
}
