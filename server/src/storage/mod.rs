use std::{fmt, io, path::Path, str::FromStr, sync::Arc};

use chrono::{DateTime, Utc};

use async_trait::async_trait;
use sqlx::sqlite::SqliteConnectOptions;
use sqlx::SqlitePool;
use thiserror::Error;

use crate::password::Password;
use crate::username::Username;

mod sqlite;
pub use sqlite::{
    SqliteInviteStorage, SqliteSessionStorage, SqliteSiteConfigStorage, SqliteUserStorage,
};

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

/// A user account record returned by [`UserStorage`] queries.
///
/// Does not expose `password_hash`; that field is only accessed inside
/// [`SqliteUserStorage`].
#[derive(Clone, Debug)]
pub struct UserRecord {
    pub user_id: i64,
    pub username: Username,
    pub display_name: Option<String>,
    pub bio: Option<String>,
    pub created_at: DateTime<Utc>,
    pub last_authenticated_at: Option<DateTime<Utc>>,
}

/// Errors that can occur when creating a user.
#[derive(Debug, Error)]
pub enum CreateUserError {
    #[error("username is already taken")]
    UsernameTaken,
    #[error(transparent)]
    Internal(#[from] sqlx::Error),
}

/// Errors that can occur when authenticating a user by password.
#[derive(Debug, Error)]
pub enum UserAuthError {
    #[error("invalid credentials")]
    InvalidCredentials,
    #[error("internal error: {0}")]
    Internal(String),
}

/// Fields to update on a user's profile.
///
/// Each field is `Option<&str>`: `None` clears the field, `Some(v)` sets it.
/// New profile fields are added here without changing the `update_profile`
/// signature.
pub struct ProfileUpdate<'a> {
    pub display_name: Option<&'a str>,
    pub bio: Option<&'a str>,
}

/// Async operations on the `users` table.
#[async_trait]
pub trait UserStorage: Send + Sync {
    async fn create_user(
        &self,
        username: &Username,
        password: &Password,
        display_name: Option<&str>,
    ) -> Result<i64, CreateUserError>;

    async fn authenticate(
        &self,
        username: &Username,
        password: &Password,
    ) -> Result<UserRecord, UserAuthError>;

    async fn get_user(&self, user_id: i64) -> sqlx::Result<Option<UserRecord>>;

    async fn get_user_by_username(&self, username: &Username) -> sqlx::Result<Option<UserRecord>>;

    async fn update_profile(&self, user_id: i64, update: &ProfileUpdate<'_>) -> sqlx::Result<()>;
}

/// A session record returned by [`SessionStorage`] queries.
#[derive(Clone, Debug)]
pub struct SessionRecord {
    pub token_hash: String,
    pub user_id: i64,
    pub username: Username,
    pub label: Option<String>,
    pub created_at: DateTime<Utc>,
    pub last_used_at: DateTime<Utc>,
}

/// Errors that can occur when authenticating a session token.
#[derive(Debug, Error)]
pub enum SessionAuthError {
    #[error("invalid token")]
    InvalidToken,
    #[error("session not found")]
    SessionNotFound,
}

/// An invite code record returned by [`InviteStorage`] queries.
#[derive(Clone, Debug)]
pub struct InviteRecord {
    pub code: String,
    pub created_at: DateTime<Utc>,
    pub expires_at: DateTime<Utc>,
    pub used_at: Option<DateTime<Utc>>,
    pub used_by: Option<i64>,
}

/// Errors that can occur when consuming an invite code.
#[derive(Debug, Error)]
pub enum UseInviteError {
    #[error("invite code not found")]
    NotFound,
    #[error("invite code has expired")]
    Expired,
    #[error("invite code has already been used")]
    AlreadyUsed,
}

/// Errors that can occur during atomic invite-and-user creation.
#[derive(Debug, Error)]
pub enum RegisterWithInviteError {
    #[error("invite code not found")]
    InviteNotFound,
    #[error("invite code has expired")]
    InviteExpired,
    #[error("invite code has already been used")]
    InviteAlreadyUsed,
    #[error("username is already taken")]
    UsernameTaken,
    #[error(transparent)]
    Internal(#[from] sqlx::Error),
}

/// Async operations on the `sessions` table.
#[async_trait]
pub trait SessionStorage: Send + Sync {
    async fn create_session(&self, user_id: i64, label: Option<&str>) -> sqlx::Result<String>;

    async fn authenticate(&self, raw_token: &str) -> Result<SessionRecord, SessionAuthError>;

    async fn revoke_session(&self, token_hash: &str) -> sqlx::Result<()>;

    async fn list_sessions(&self, user_id: i64) -> sqlx::Result<Vec<SessionRecord>>;
}

/// Async operations on the `invites` table.
#[async_trait]
pub trait InviteStorage: Send + Sync {
    async fn create_invite(&self, expires_at: DateTime<Utc>) -> sqlx::Result<String>;

    async fn use_invite(&self, code: &str, user_id: i64) -> Result<(), UseInviteError>;

    async fn list_invites(&self) -> sqlx::Result<Vec<InviteRecord>>;
}

/// Atomically creates a user and marks an invite code as used within a single
/// transaction. This spans two tables so it cannot be expressed through the
/// single-table trait objects.
///
/// Steps:
/// (a) SELECT the invite row; return `InviteNotFound`, `InviteAlreadyUsed`, or
///     `InviteExpired` as appropriate.
/// (b) Hash the password via `spawn_blocking`.
/// (c) INSERT the user row; map a unique-constraint violation to `UsernameTaken`.
/// (d) UPDATE the invite row setting `used_at` and `used_by`.
/// (e) COMMIT.
pub async fn create_user_with_invite(
    pool: &SqlitePool,
    username: &Username,
    password: &Password,
    display_name: Option<&str>,
    invite_code: &str,
) -> Result<i64, RegisterWithInviteError> {
    use argon2::{
        password_hash::{rand_core::OsRng, SaltString},
        Argon2, PasswordHasher,
    };

    let mut tx = pool.begin().await?;

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
    let password_str = password.as_str().to_owned();
    let password_hash = tokio::task::spawn_blocking(move || -> Result<String, String> {
        let salt = SaltString::generate(&mut OsRng);
        Argon2::default()
            .hash_password(password_str.as_bytes(), &salt)
            .map(|h| h.to_string())
            .map_err(|e| e.to_string())
    })
    .await
    .map_err(|e| RegisterWithInviteError::Internal(sqlx::Error::Io(std::io::Error::other(e))))?
    .map_err(|e| RegisterWithInviteError::Internal(sqlx::Error::Io(std::io::Error::other(e))))?;

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

/// Application-wide state bundling all storage handles.
///
/// The raw `pool` is stored so free functions like `create_user_with_invite`
/// that span multiple tables can execute transactions directly.
pub struct AppState {
    pub pool: SqlitePool,
    pub site_config: Arc<dyn SiteConfigStorage>,
    pub users: Arc<dyn UserStorage>,
    pub sessions: Arc<dyn SessionStorage>,
    pub invites: Arc<dyn InviteStorage>,
}

fn make_app_state(pool: SqlitePool) -> Arc<AppState> {
    Arc::new(AppState {
        site_config: Arc::new(SqliteSiteConfigStorage::new(pool.clone())),
        users: Arc::new(SqliteUserStorage::new(pool.clone())),
        sessions: Arc::new(SqliteSessionStorage::new(pool.clone())),
        invites: Arc::new(SqliteInviteStorage::new(pool.clone())),
        pool,
    })
}

/// Opens (or creates) the database described by `opts`, runs pending
/// migrations, and returns an [`AppState`] bundling all storage handles.
///
/// Only `sqlite:` URLs are supported in M1; PostgreSQL support is added in M20.
pub async fn open_database(opts: &DbConnectOptions) -> sqlx::Result<Arc<AppState>> {
    match opts {
        DbConnectOptions::Sqlite(options) => {
            let pool =
                sqlx::SqlitePool::connect_with(options.clone().create_if_missing(true)).await?;
            sqlx::migrate!("./migrations").run(&pool).await?;
            Ok(make_app_state(pool))
        }
    }
}

/// Opens an existing database described by `opts`, runs pending migrations, and
/// returns an [`AppState`] bundling all storage handles.
///
/// Unlike [`open_database`], this function fails if the database does not
/// already exist. Use [`open_database`] in `cmd_init`, which creates the
/// database when it is missing.
pub async fn open_existing_database(opts: &DbConnectOptions) -> sqlx::Result<Arc<AppState>> {
    match opts {
        DbConnectOptions::Sqlite(options) => {
            // create_if_missing defaults to false, so this fails when the file
            // does not exist.
            let pool = sqlx::SqlitePool::connect_with(options.clone()).await?;
            sqlx::migrate!("./migrations").run(&pool).await?;
            Ok(make_app_state(pool))
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
