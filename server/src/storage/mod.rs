use std::{fmt, io, path::Path, str::FromStr, sync::Arc};

use async_trait::async_trait;
use sqlx::sqlite::SqliteConnectOptions;
use sqlx::SqlitePool;

// Storage traits, records, error types, and AppState are defined in the
// `common` crate so that both `web` (server functions) and `server` can use
// them without a circular dependency.  Re-export everything for
// backward-compatibility with existing server-crate consumers.
pub use common::storage::{
    AppState, AtomicOps, CreateUserError, InviteRecord, InviteStorage, ProfileUpdate,
    RegisterWithInviteError, SessionAuthError, SessionRecord, SessionStorage, SiteConfigStorage,
    UseInviteError, UserAuthError, UserRecord, UserStorage,
};

use common::password::Password;
use common::username::Username;

mod sqlite;
pub use sqlite::{
    SqliteInviteStorage, SqliteSessionStorage, SqliteSiteConfigStorage, SqliteUserStorage,
};

// ---------------------------------------------------------------------------
// SqliteAtomicOps
// ---------------------------------------------------------------------------

/// SQLite implementation of [`AtomicOps`].
///
/// Holds the pool directly so it can span multiple tables in a single
/// transaction without going through the individual storage trait objects.
pub struct SqliteAtomicOps {
    pool: SqlitePool,
}

impl SqliteAtomicOps {
    pub fn new(pool: SqlitePool) -> Self {
        Self { pool }
    }
}

#[async_trait]
impl AtomicOps for SqliteAtomicOps {
    async fn create_user_with_invite(
        &self,
        username: &Username,
        password: &Password,
        display_name: Option<&str>,
        invite_code: &str,
    ) -> Result<i64, RegisterWithInviteError> {
        use chrono::{DateTime, Utc};

        let mut tx = self.pool.begin().await?;

        // (a) Validate invite
        let row = sqlx::query_as::<_, (Option<DateTime<Utc>>, DateTime<Utc>)>(
            "SELECT used_at, expires_at FROM invites WHERE code = ?",
        )
        .bind(invite_code)
        .fetch_optional(&mut *tx)
        .await?
        .ok_or(RegisterWithInviteError::InviteNotFound)?;

        let (used_at, expires_at) = row;

        if used_at.is_some() {
            return Err(RegisterWithInviteError::InviteAlreadyUsed);
        }

        let now = Utc::now();
        if expires_at <= now {
            return Err(RegisterWithInviteError::InviteExpired);
        }

        // (b) Hash password outside the async executor
        let password = password.clone();
        let password_hash = tokio::task::spawn_blocking(move || password.hash())
            .await
            .map_err(|e| {
                RegisterWithInviteError::Internal(sqlx::Error::Io(std::io::Error::other(e)))
            })?
            .map_err(|e| {
                RegisterWithInviteError::Internal(sqlx::Error::Io(std::io::Error::other(e)))
            })?;

        // (c) Insert user
        let result = sqlx::query_scalar::<_, i64>(
            "INSERT INTO users (username, password_hash, display_name, created_at)
             VALUES (?, ?, ?, ?)
             RETURNING user_id",
        )
        .bind(username.as_str())
        .bind(&password_hash)
        .bind(display_name)
        .bind(now)
        .fetch_one(&mut *tx)
        .await;

        let user_id = match result {
            Ok(id) => id,
            Err(sqlx::Error::Database(e)) if e.is_unique_violation() => {
                return Err(RegisterWithInviteError::UsernameTaken);
            }
            Err(e) => return Err(RegisterWithInviteError::Internal(e)),
        };

        // (d) Mark invite used
        sqlx::query("UPDATE invites SET used_at = ?, used_by = ? WHERE code = ?")
            .bind(now)
            .bind(user_id)
            .bind(invite_code)
            .execute(&mut *tx)
            .await?;

        // (e) Commit
        tx.commit().await?;

        Ok(user_id)
    }
}

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

// ---------------------------------------------------------------------------
// Storage directory helpers
// ---------------------------------------------------------------------------

/// Creates the storage root and required subdirectories (`media/`, `backups/`).
pub fn init_storage(path: &Path) -> io::Result<()> {
    std::fs::create_dir(path)?;
    std::fs::create_dir_all(path.join("media"))?;
    std::fs::create_dir_all(path.join("backups"))?;
    Ok(())
}

// ---------------------------------------------------------------------------
// Database helpers
// ---------------------------------------------------------------------------

fn make_app_state(pool: SqlitePool) -> Arc<AppState> {
    Arc::new(AppState {
        site_config: Arc::new(SqliteSiteConfigStorage::new(pool.clone())),
        users: Arc::new(SqliteUserStorage::new(pool.clone())),
        sessions: Arc::new(SqliteSessionStorage::new(pool.clone())),
        invites: Arc::new(SqliteInviteStorage::new(pool.clone())),
        atomic: Arc::new(SqliteAtomicOps::new(pool)),
    })
}

async fn init_pool(opts: &DbConnectOptions, create_if_missing: bool) -> sqlx::Result<SqlitePool> {
    match opts {
        DbConnectOptions::Sqlite(options) => {
            let mut options = options.clone();
            if create_if_missing {
                options = options.create_if_missing(true);
            }
            let pool = sqlx::SqlitePool::connect_with(options).await?;
            sqlx::migrate!("./migrations").run(&pool).await?;
            Ok(pool)
        }
    }
}

/// Opens (or creates) the database described by `opts`, runs pending
/// migrations, and returns an [`AppState`] bundling all storage handles.
pub async fn open_database(opts: &DbConnectOptions) -> sqlx::Result<Arc<AppState>> {
    let pool = init_pool(opts, true).await?;
    Ok(make_app_state(pool))
}

/// Opens an existing database described by `opts`, runs pending migrations.
///
/// Unlike [`open_database`], fails if the database does not already exist.
pub async fn open_existing_database(opts: &DbConnectOptions) -> sqlx::Result<Arc<AppState>> {
    let pool = init_pool(opts, false).await?;
    Ok(make_app_state(pool))
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

    #[test]
    fn init_storage_fails_on_missing_parent() {
        let storage = std::path::Path::new("/nonexistent/path/to/storage");
        let result = init_storage(storage);
        assert!(result.is_err());
    }
}
