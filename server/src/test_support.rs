//! Test-only construction seams for the `server` crate.
//!
//! Integration tests compile `server` as a dependency, so their router seam must
//! remain externally reachable. It lives here rather than expanding the
//! production router API at the crate root.
//!
//! The in-crate helpers build a migrated `SQLite` database and hand back the
//! *narrow* storage handles a test needs, rather than the whole `AppState`. A test
//! for a constructor-injected subsystem should construct exactly the handles that
//! subsystem (and its fixtures) touch — see [ADR-0016]. Integration tests
//! (`server/tests/`) otherwise use the backend-parametric
//! `storage::test_support::Backend`, which exercises `SQLite` and `PostgreSQL`.
//!
//! [ADR-0016]: ../../docs/adr/0016-dependency-injection-and-appstate.md

use std::{path::PathBuf, sync::Arc};

#[cfg(test)]
use std::path::Path;

use common::mailer::MailSender;
use leptos::prelude;
use storage::{
    AppState, InstanceId, PasswordResetStorage, SiteConfigStorage, UserStorage, WriteScope,
};
#[cfg(test)]
use storage::{DbConnectOptions, SqliteSiteConfigStorage};

use crate::media_ownership::LiveMediaReferenceOwnershipResolver;

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
/// Builds a production-shaped router with explicit password-reset dependencies.
///
/// This narrow seam exists for deterministic integration tests of detached work.
///
/// # Errors
///
/// Returns an error when the generated instance identity cannot form an HTTP header.
pub fn create_router_with_password_reset_dependencies(
    state: Arc<AppState>,
    mailer: Arc<dyn MailSender>,
    storage_path: PathBuf,
    users: Arc<dyn UserStorage>,
    password_resets: Arc<dyn PasswordResetStorage>,
    write_scope: WriteScope,
    site_config: Arc<dyn SiteConfigStorage>,
) -> Result<axum::Router, axum::http::header::InvalidHeaderValue> {
    let reset_mailer = Arc::clone(&mailer);
    super::create_router_with_dependencies(
        state,
        InstanceId::new(),
        mailer,
        false,
        storage_path,
        Arc::new(LiveMediaReferenceOwnershipResolver::new()),
        move || {
            prelude::provide_context::<Arc<dyn UserStorage>>(Arc::clone(&users));
            prelude::provide_context::<Arc<dyn PasswordResetStorage>>(Arc::clone(&password_resets));
            prelude::provide_context::<WriteScope>(write_scope.clone());
            prelude::provide_context::<Arc<dyn SiteConfigStorage>>(Arc::clone(&site_config));
            crate::context::provide_mailer_context(&reset_mailer);
        },
    )
}
