use std::{fmt, io, path::Path, str::FromStr, sync::Arc};

use chrono::{DateTime, Utc};

use crate::password::Password;
use crate::username::Username;

use argon2::{
    password_hash::{rand_core::OsRng, SaltString},
    Argon2, PasswordHash, PasswordHasher, PasswordVerifier,
};
use async_trait::async_trait;
use sqlx::{sqlite::SqliteConnectOptions, SqlitePool};
use thiserror::Error;

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

type UserRow = (
    i64,
    String,
    Option<String>,
    Option<String>,
    DateTime<Utc>,
    Option<DateTime<Utc>>,
);

fn user_record_from_row(
    (user_id, username, display_name, bio, created_at, last_authenticated_at): UserRow,
) -> UserRecord {
    UserRecord {
        user_id,
        username: username
            .parse()
            .expect("username stored in database is always valid"),
        display_name,
        bio,
        created_at,
        last_authenticated_at,
    }
}

/// SQLite-backed [`UserStorage`].
pub struct SqliteUserStorage {
    pool: SqlitePool,
}

impl SqliteUserStorage {
    pub fn new(pool: SqlitePool) -> Self {
        Self { pool }
    }
}

#[async_trait]
impl UserStorage for SqliteUserStorage {
    async fn create_user(
        &self,
        username: &Username,
        password: &Password,
        display_name: Option<&str>,
    ) -> Result<i64, CreateUserError> {
        let password = password.as_str().to_owned();
        let password_hash = tokio::task::spawn_blocking(move || -> Result<String, String> {
            let salt = SaltString::generate(&mut OsRng);
            Argon2::default()
                .hash_password(password.as_bytes(), &salt)
                .map(|h| h.to_string())
                .map_err(|e| e.to_string())
        })
        .await
        .map_err(|e| CreateUserError::Internal(sqlx::Error::Io(std::io::Error::other(e))))?
        .map_err(|e| CreateUserError::Internal(sqlx::Error::Io(std::io::Error::other(e))))?;

        let now = Utc::now();

        let result = sqlx::query_scalar::<_, i64>(
            "INSERT INTO users (username, password_hash, display_name, created_at)
             VALUES (?, ?, ?, ?)
             RETURNING user_id",
        )
        .bind(username.as_str())
        .bind(&password_hash)
        .bind(display_name)
        .bind(now)
        .fetch_one(&self.pool)
        .await;

        match result {
            Ok(id) => Ok(id),
            Err(sqlx::Error::Database(e)) if e.is_unique_violation() => {
                Err(CreateUserError::UsernameTaken)
            }
            Err(e) => Err(CreateUserError::Internal(e)),
        }
    }

    async fn authenticate(
        &self,
        username: &Username,
        password: &Password,
    ) -> Result<UserRecord, UserAuthError> {
        let row = sqlx::query_as::<
            _,
            (
                i64,
                String,
                Option<String>,
                Option<String>,
                DateTime<Utc>,
                Option<DateTime<Utc>>,
                String,
            ),
        >(
            "SELECT user_id, username, display_name, bio, created_at, last_authenticated_at, password_hash
             FROM users WHERE username = ?",
        )
        .bind(username.as_str())
        .fetch_optional(&self.pool)
        .await
        .map_err(|e| UserAuthError::Internal(e.to_string()))?;

        let (user_id, username, display_name, bio, created_at, _last_authenticated_at, hash) =
            match row {
                Some(r) => r,
                None => return Err(UserAuthError::InvalidCredentials),
            };

        let password = password.as_str().to_owned();
        let hash_clone = hash.clone();
        let valid = tokio::task::spawn_blocking(move || {
            let parsed = PasswordHash::new(&hash_clone).map_err(|e| e.to_string())?;
            Argon2::default()
                .verify_password(password.as_bytes(), &parsed)
                .map(|_| true)
                .or_else(|e| match e {
                    argon2::password_hash::Error::Password => Ok(false),
                    other => Err(other.to_string()),
                })
        })
        .await
        .map_err(|e| UserAuthError::Internal(e.to_string()))?
        .map_err(UserAuthError::Internal)?;

        if !valid {
            return Err(UserAuthError::InvalidCredentials);
        }

        let now = Utc::now();

        sqlx::query("UPDATE users SET last_authenticated_at = ? WHERE user_id = ?")
            .bind(now)
            .bind(user_id)
            .execute(&self.pool)
            .await
            .map_err(|e| UserAuthError::Internal(e.to_string()))?;

        Ok(UserRecord {
            user_id,
            username: username
                .parse()
                .expect("username stored in database is always valid"),
            display_name,
            bio,
            created_at,
            last_authenticated_at: Some(now),
        })
    }

    async fn get_user(&self, user_id: i64) -> sqlx::Result<Option<UserRecord>> {
        let row = sqlx::query_as::<_, UserRow>(
            "SELECT user_id, username, display_name, bio, created_at, last_authenticated_at
             FROM users WHERE user_id = ?",
        )
        .bind(user_id)
        .fetch_optional(&self.pool)
        .await?;
        Ok(row.map(user_record_from_row))
    }

    async fn get_user_by_username(&self, username: &Username) -> sqlx::Result<Option<UserRecord>> {
        let row = sqlx::query_as::<_, UserRow>(
            "SELECT user_id, username, display_name, bio, created_at, last_authenticated_at
             FROM users WHERE username = ?",
        )
        .bind(username.as_str())
        .fetch_optional(&self.pool)
        .await?;
        Ok(row.map(user_record_from_row))
    }

    async fn update_profile(&self, user_id: i64, update: &ProfileUpdate<'_>) -> sqlx::Result<()> {
        sqlx::query("UPDATE users SET display_name = ?, bio = ? WHERE user_id = ?")
            .bind(update.display_name)
            .bind(update.bio)
            .bind(user_id)
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

/// Opens an existing database described by `opts`, runs pending migrations, and
/// returns a [`SiteConfigStorage`] implementation.
///
/// Unlike [`open_database`], this function fails if the database does not
/// already exist. Use [`open_database`] in `cmd_init`, which creates the
/// database when it is missing.
pub async fn open_existing_database(
    opts: &DbConnectOptions,
) -> sqlx::Result<Arc<dyn SiteConfigStorage>> {
    match opts {
        DbConnectOptions::Sqlite(options) => {
            // create_if_missing defaults to false, so this fails when the file
            // does not exist.
            let pool = SqlitePool::connect_with(options.clone()).await?;
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
