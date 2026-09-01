use std::io;

use crate::cli::{AppTarget, BootstrapDb, StorageArgs};
use common::pg_role_password::PgRolePassword;

use super::support::storage_runtime_config;

/// Initializes the application's storage directory and database.
///
/// # Errors
///
/// Returns an error if the storage directory cannot be created, or if the
/// database cannot be initialized.
pub async fn cmd_init(storage: &StorageArgs, skip_if_exists: bool) -> anyhow::Result<()> {
    match storage::init_storage(&storage.storage_path) {
        Ok(()) => {}
        Err(e) if skip_if_exists && e.kind() == io::ErrorKind::AlreadyExists => {}
        Err(e) => return Err(e.into()),
    }
    let runtime = storage_runtime_config(&storage.db)?;
    storage::open_database(&storage.db, &runtime).await?;
    println!(
        "Initialized: storage={} db={}",
        storage.storage_path.display(),
        storage.db,
    );
    Ok(())
}

/// Maps a [`storage::PgBootstrapError`] to a user-facing CLI error.
fn describe_bootstrap_error(err: storage::PgBootstrapError) -> anyhow::Error {
    match err {
        storage::PgBootstrapError::RoleExists(role) => anyhow::anyhow!(
            "application role '{role}' already exists; refusing to modify existing role state"
        ),
        storage::PgBootstrapError::DatabaseExists(name) => anyhow::anyhow!(
            "database '{name}' already exists; refusing to modify existing database state"
        ),
        storage::PgBootstrapError::Sqlx(err) => err.into(),
    }
}

/// Bootstraps a `PostgreSQL` database and application role.
///
/// Every argument is already validated by the time it arrives: the CLI is the parse
/// boundary, so a non-`PostgreSQL` URL, a URL naming no database, and an empty password
/// are all rejected at argument parsing rather than here (#693).
///
/// # Errors
///
/// Returns an error if the bootstrap connection fails, or if the role or
/// database already exists.
pub async fn cmd_create_pg_db(
    bootstrap_db: &BootstrapDb,
    app_db: &AppTarget,
    app_role_password: &PgRolePassword,
) -> anyhow::Result<()> {
    let app_role = app_db.role();
    let database_name = app_db.database();

    storage::create_postgres_database_and_role(
        bootstrap_db.options(),
        app_role,
        app_role_password,
        database_name,
    )
    .await
    .map_err(describe_bootstrap_error)?;

    println!("PostgreSQL ready: role='{app_role}' database='{database_name}' owner='{app_role}'");
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn describe_bootstrap_error_role_exists_message() {
        let msg =
            describe_bootstrap_error(storage::PgBootstrapError::RoleExists("alice".to_owned()))
                .to_string();
        assert!(msg.contains("application role 'alice' already exists"));
        assert!(msg.contains("refusing to modify existing role state"));
    }

    #[test]
    fn describe_bootstrap_error_database_exists_message() {
        let msg =
            describe_bootstrap_error(storage::PgBootstrapError::DatabaseExists("blog".to_owned()))
                .to_string();
        assert!(msg.contains("database 'blog' already exists"));
        assert!(msg.contains("refusing to modify existing database state"));
    }

    #[test]
    fn describe_bootstrap_error_sqlx_passes_through_source_message() {
        let expected = sqlx::Error::PoolClosed.to_string();
        let err =
            describe_bootstrap_error(storage::PgBootstrapError::Sqlx(sqlx::Error::PoolClosed));
        assert_eq!(err.to_string(), expected);
    }
}
