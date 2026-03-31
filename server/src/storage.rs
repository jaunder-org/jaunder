use std::{fmt, io, path::Path, str::FromStr, sync::Arc};

use async_trait::async_trait;
use sqlx::{sqlite::SqliteConnectOptions, SqlitePool};

/// Parsed connection options for a supported database backend.
///
/// Constructed via [`FromStr`] at the CLI boundary; invalid or unsupported
/// URLs are rejected there rather than inside application logic.
#[derive(Clone, Debug)]
pub enum DbConnectOptions {
    Sqlite(SqliteConnectOptions),
}

impl fmt::Display for DbConnectOptions {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            DbConnectOptions::Sqlite(opts) => {
                write!(f, "sqlite:{}", opts.get_filename().display())
            }
        }
    }
}

impl FromStr for DbConnectOptions {
    type Err = sqlx::Error;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        if s.starts_with("sqlite:") {
            Ok(DbConnectOptions::Sqlite(s.parse()?))
        } else {
            Err(sqlx::Error::Configuration(
                format!("unsupported database URL '{s}'; only sqlite: is supported").into(),
            ))
        }
    }
}

/// Creates the storage root and required subdirectories (`media/`, `backups/`).
///
/// Fails with [`io::ErrorKind::AlreadyExists`] if the storage root already
/// exists, so callers can detect the case where the directory belongs to
/// something else. The caller is responsible for deciding whether to treat
/// that as an error (the default) or ignore it (`--skip-if-exists`).
pub fn init_storage(path: &Path) -> io::Result<()> {
    std::fs::create_dir(path)?;
    std::fs::create_dir_all(path.join("media"))?;
    std::fs::create_dir_all(path.join("backups"))?;
    Ok(())
}

/// Async operations on the `site_config` key-value table.
///
/// All database code operates through this trait; nothing outside the concrete
/// implementations imports sqlx or a backend-specific type.
#[async_trait]
pub trait SiteConfigStorage: Send + Sync {
    /// Returns the value for `key`, or `None` if the key is not set.
    async fn get(&self, key: &str) -> sqlx::Result<Option<String>>;

    /// Inserts or replaces the value for `key`.
    async fn set(&self, key: &str, value: &str) -> sqlx::Result<()>;
}

/// SQLite-backed [`SiteConfigStorage`].
pub struct SqliteSiteConfigStorage {
    pool: SqlitePool,
}

#[async_trait]
impl SiteConfigStorage for SqliteSiteConfigStorage {
    async fn get(&self, key: &str) -> sqlx::Result<Option<String>> {
        let row = sqlx::query_as::<_, (String,)>("SELECT value FROM site_config WHERE key = ?")
            .bind(key)
            .fetch_optional(&self.pool)
            .await?;
        Ok(row.map(|(v,)| v))
    }

    async fn set(&self, key: &str, value: &str) -> sqlx::Result<()> {
        sqlx::query(
            "INSERT INTO site_config (key, value) VALUES (?, ?)
             ON CONFLICT(key) DO UPDATE SET value = excluded.value",
        )
        .bind(key)
        .bind(value)
        .execute(&self.pool)
        .await?;
        Ok(())
    }
}

/// Opens (or creates) the database described by `opts`, runs pending
/// migrations, and returns a [`SiteConfigStorage`] implementation.
///
/// Only `sqlite:` URLs are supported in M1; PostgreSQL support is added in M20.
pub async fn open_database(opts: &DbConnectOptions) -> sqlx::Result<Arc<dyn SiteConfigStorage>> {
    match opts {
        DbConnectOptions::Sqlite(options) => {
            let pool = SqlitePool::connect_with(options.clone().create_if_missing(true)).await?;
            sqlx::migrate!("./migrations").run(&pool).await?;
            Ok(Arc::new(SqliteSiteConfigStorage { pool }))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[test]
    fn new_path_created_with_subdirs() {
        let base = TempDir::new().unwrap();
        let storage = base.path().join("storage");

        init_storage(&storage).unwrap();

        assert!(storage.is_dir());
        assert!(storage.join("media").is_dir());
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
}
