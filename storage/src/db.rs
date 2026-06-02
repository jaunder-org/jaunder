//! Database connection and initialization.
//!
//! Handles opening `SQLite` and `PostgreSQL` databases, running migrations,
//! and constructing the [`AppState`] with all storage implementations.

use std::io;
use std::path::Path;
use std::{fmt, str::FromStr, sync::Arc};

use sqlx::postgres::PgConnectOptions;
use sqlx::sqlite::SqliteConnectOptions;
use sqlx::{PgPool, SqlitePool};

use crate::postgres::open_postgres_database;
use crate::sqlite::open_sqlite_database;
use crate::AppState;

// ---------------------------------------------------------------------------
// DbConnectOptions
// ---------------------------------------------------------------------------

/// Parsed connection options for a supported database backend.
///
/// Constructed via [`FromStr`] at the CLI boundary; invalid or unsupported
/// URLs are rejected there rather than inside application logic.
#[derive(Clone, Debug)]
pub enum DbConnectOptions {
    Sqlite(SqliteConnectOptions),
    Postgres {
        url: String,
        options: PgConnectOptions,
    },
}

impl fmt::Display for DbConnectOptions {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            DbConnectOptions::Sqlite(opts) => {
                write!(f, "sqlite:{}", opts.get_filename().display())
            }
            DbConnectOptions::Postgres { url, .. } => f.write_str(url),
        }
    }
}

impl FromStr for DbConnectOptions {
    type Err = sqlx::Error;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        if s.starts_with("sqlite:") {
            Ok(DbConnectOptions::Sqlite(s.parse()?))
        } else if s.starts_with("postgres://") || s.starts_with("postgresql://") {
            Ok(DbConnectOptions::Postgres {
                url: s.to_owned(),
                options: s.parse()?,
            })
        } else {
            Err(sqlx::Error::Configuration(
                format!(
                    "unsupported database URL '{s}'; supported schemes are sqlite: and postgres://"
                )
                .into(),
            ))
        }
    }
}

// ---------------------------------------------------------------------------
// Storage directory helpers
// ---------------------------------------------------------------------------

/// Creates the storage root and required subdirectories (`media/`, `backups/`).
///
/// # Errors
///
/// Returns `Err` if the storage directory cannot be created.
pub fn init_storage(path: &Path) -> io::Result<()> {
    std::fs::create_dir(path)?;
    std::fs::create_dir_all(path.join("media"))?;
    std::fs::create_dir_all(path.join("media").join("upload"))?;
    std::fs::create_dir_all(path.join("media").join("cached"))?;
    std::fs::create_dir_all(path.join("media").join("tmp"))?;
    std::fs::create_dir_all(path.join("backups"))?;
    Ok(())
}

// ---------------------------------------------------------------------------
// Database helpers
// ---------------------------------------------------------------------------

/// Slow-query log threshold shared by both `SQLite` and Postgres backends.
///
/// Reads `JAUNDER_SQL_SLOW_MS` (milliseconds), defaulting to 100ms.
pub(crate) fn sql_slow_query_threshold() -> std::time::Duration {
    std::env::var("JAUNDER_SQL_SLOW_MS")
        .ok()
        .and_then(|value| value.parse::<u64>().ok())
        .map_or(
            std::time::Duration::from_millis(100),
            std::time::Duration::from_millis,
        )
}

/// Opens (or creates) the database described by `opts`, runs pending
/// migrations, and returns an [`AppState`] bundling all storage handles.
///
/// # Errors
///
/// Returns `Err` if the database connection pool cannot be established.
#[tracing::instrument(name = "storage.open_database", skip(opts))]
pub async fn open_database(opts: &DbConnectOptions) -> sqlx::Result<Arc<AppState>> {
    match opts {
        DbConnectOptions::Sqlite(options) => open_sqlite_database(options, true).await,
        DbConnectOptions::Postgres { options, .. } => open_postgres_database(options).await,
    }
}

/// Opens an existing database described by `opts`, runs pending migrations.
///
/// Unlike [`open_database`], fails if the database does not already exist.
///
/// # Errors
///
/// Returns `Err` if the database connection pool cannot be established.
#[tracing::instrument(name = "storage.open_existing_database", skip(opts))]
pub async fn open_existing_database(opts: &DbConnectOptions) -> sqlx::Result<Arc<AppState>> {
    match opts {
        DbConnectOptions::Sqlite(options) => open_sqlite_database(options, false).await,
        DbConnectOptions::Postgres { options, .. } => open_postgres_database(options).await,
    }
}

/// Returns `true` if the target database already contains at least one user.
///
/// Used as a restore preflight: refusing to restore into a non-empty database.
///
/// # Errors
///
/// Returns the underlying [`sqlx::Error`] if the database cannot be reached or
/// the query fails.
pub async fn database_has_users(options: &DbConnectOptions) -> sqlx::Result<bool> {
    match options {
        DbConnectOptions::Sqlite(options) => {
            let pool = SqlitePool::connect_with(options.clone()).await?;
            Ok(
                sqlx::query_scalar::<_, i64>("SELECT EXISTS(SELECT 1 FROM users LIMIT 1)")
                    .fetch_one(&pool)
                    .await?
                    != 0,
            )
        }
        DbConnectOptions::Postgres { options, .. } => {
            let options = crate::resolved_postgres_options(options)?;
            let pool = PgPool::connect_with(options).await?;
            Ok(
                sqlx::query_scalar::<_, bool>("SELECT EXISTS(SELECT 1 FROM users LIMIT 1)")
                    .fetch_one(&pool)
                    .await?,
            )
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;
    use std::time::Duration;
    use tempfile::TempDir;

    static ENV_LOCK: Mutex<()> = Mutex::new(());

    #[test]
    fn new_path_created_with_subdirs() {
        let base = TempDir::new().unwrap();
        let storage = base.path().join("storage");

        init_storage(&storage).unwrap();

        assert!(storage.is_dir());
        assert!(storage.join("media").is_dir());
        assert!(storage.join("media").join("upload").is_dir());
        assert!(storage.join("media").join("cached").is_dir());
        assert!(storage.join("media").join("tmp").is_dir());
        assert!(storage.join("backups").is_dir());
    }

    #[test]
    fn existing_path_returns_already_exists_error() {
        let base = TempDir::new().unwrap();
        let storage = base.path().join("storage");

        init_storage(&storage).unwrap();

        let err = init_storage(&storage).unwrap_err();
        assert_eq!(err.kind(), io::ErrorKind::AlreadyExists);
    }

    #[test]
    fn init_storage_fails_on_missing_parent() {
        let storage = std::path::Path::new("/nonexistent/path/to/storage");
        let result = init_storage(storage);
        assert!(result.is_err());
    }

    #[test]
    fn sql_slow_query_threshold_defaults_to_100ms() {
        let _guard = ENV_LOCK.lock().unwrap();
        std::env::remove_var("JAUNDER_SQL_SLOW_MS");
        assert_eq!(sql_slow_query_threshold(), Duration::from_millis(100));
    }

    #[test]
    fn sql_slow_query_threshold_uses_env_override() {
        let _guard = ENV_LOCK.lock().unwrap();
        std::env::set_var("JAUNDER_SQL_SLOW_MS", "250");
        assert_eq!(sql_slow_query_threshold(), Duration::from_millis(250));
        std::env::remove_var("JAUNDER_SQL_SLOW_MS");
    }

    #[test]
    fn test_db_connect_options_parsing() {
        let sqlite = "sqlite:jaunder.db".parse::<DbConnectOptions>().unwrap();
        assert!(matches!(sqlite, DbConnectOptions::Sqlite(_)));
        assert_eq!(sqlite.to_string(), "sqlite:jaunder.db");

        let pg = "postgres://user:pass@localhost/db"
            .parse::<DbConnectOptions>()
            .unwrap();
        assert!(matches!(pg, DbConnectOptions::Postgres { .. }));
        assert_eq!(pg.to_string(), "postgres://user:pass@localhost/db");

        let pgs = "postgresql://user:pass@localhost/db"
            .parse::<DbConnectOptions>()
            .unwrap();
        assert!(matches!(pgs, DbConnectOptions::Postgres { .. }));

        let invalid = "mysql://localhost".parse::<DbConnectOptions>();
        assert!(invalid.is_err());
    }

    #[test]
    fn test_db_connect_options_invalid_sqlite() {
        // Starts with sqlite: but is invalid
        let invalid = "sqlite:??invalid??".parse::<DbConnectOptions>();
        assert!(invalid.is_err());
    }

    #[test]
    fn test_db_connect_options_invalid_postgres() {
        let invalid = "postgres://[invalid]".parse::<DbConnectOptions>();
        assert!(invalid.is_err());
    }

    #[tokio::test]
    async fn open_database_routes_to_postgres_backend() {
        let opts = "postgres://localhost:1/db"
            .parse::<DbConnectOptions>()
            .unwrap();
        let _ =
            tokio::time::timeout(std::time::Duration::from_millis(50), open_database(&opts)).await;
    }

    #[tokio::test]
    async fn open_existing_database_routes_to_postgres_backend() {
        let opts = "postgres://localhost:1/db"
            .parse::<DbConnectOptions>()
            .unwrap();
        let _ = tokio::time::timeout(
            std::time::Duration::from_millis(50),
            open_existing_database(&opts),
        )
        .await;
    }
}
