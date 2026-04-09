use std::{fmt, io, path::Path, str::FromStr, sync::Arc};

use async_trait::async_trait;
use sqlx::postgres::PgConnectOptions;
use sqlx::sqlite::SqliteConnectOptions;
use sqlx::PgPool;
use sqlx::SqlitePool;

// Storage traits, records, error types, and AppState are defined in the
// `common` crate so that both `web` (server functions) and `server` can use
// them without a circular dependency.  Re-export everything for
// backward-compatibility with existing server-crate consumers.
pub use common::storage::{
    AppState, AtomicOps, ConfirmPasswordResetError, CreateUserError, EmailVerificationStorage,
    InviteRecord, InviteStorage, PasswordResetStorage, ProfileUpdate, RegisterWithInviteError,
    SessionAuthError, SessionRecord, SessionStorage, SiteConfigStorage, UseEmailVerificationError,
    UseInviteError, UsePasswordResetError, UserAuthError, UserRecord, UserStorage,
};

use crate::mailer::FileMailSender;
use common::mailer::{MailSender, NoopMailSender};
use common::password::Password;
use common::smtp::load_smtp_config;
use common::username::Username;

mod sqlite;
pub use sqlite::{
    SqliteEmailVerificationStorage, SqliteInviteStorage, SqlitePasswordResetStorage,
    SqliteSessionStorage, SqliteSiteConfigStorage, SqliteUserStorage,
};
mod postgres;
pub use postgres::{
    PostgresAtomicOps, PostgresEmailVerificationStorage, PostgresInviteStorage,
    PostgresPasswordResetStorage, PostgresSessionStorage, PostgresSiteConfigStorage,
    PostgresUserStorage,
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

    async fn confirm_password_reset(
        &self,
        raw_token: &str,
        new_password: &Password,
    ) -> Result<(), ConfirmPasswordResetError> {
        let token_hash =
            crate::auth::hash_token(raw_token).map_err(|_| ConfirmPasswordResetError::NotFound)?;

        let password = new_password.clone();
        let password_hash = tokio::task::spawn_blocking(move || password.hash())
            .await
            .map_err(|e| {
                ConfirmPasswordResetError::Internal(sqlx::Error::Io(std::io::Error::other(e)))
            })?
            .map_err(|e| {
                ConfirmPasswordResetError::Internal(sqlx::Error::Io(std::io::Error::other(e)))
            })?;

        let mut tx = self.pool.begin().await?;
        let now = chrono::Utc::now();

        let claimed = sqlx::query_as::<_, (i64,)>(
            "UPDATE password_resets SET used_at = ?\n             WHERE token_hash = ? AND used_at IS NULL AND expires_at > ?\n             RETURNING user_id",
        )
        .bind(now)
        .bind(&token_hash)
        .bind(now)
        .fetch_optional(&mut *tx)
        .await?;

        let user_id = if let Some((user_id,)) = claimed {
            user_id
        } else {
            let row = sqlx::query_as::<
                _,
                (
                    Option<chrono::DateTime<chrono::Utc>>,
                    chrono::DateTime<chrono::Utc>,
                ),
            >(
                "SELECT used_at, expires_at FROM password_resets WHERE token_hash = ?"
            )
            .bind(&token_hash)
            .fetch_optional(&mut *tx)
            .await?;

            tx.rollback().await.ok();

            return match row {
                None => Err(ConfirmPasswordResetError::NotFound),
                Some((Some(_), _)) => Err(ConfirmPasswordResetError::AlreadyUsed),
                Some((None, expires_at)) if expires_at <= now => {
                    Err(ConfirmPasswordResetError::Expired)
                }
                Some((None, _)) => Err(ConfirmPasswordResetError::Expired),
            };
        };

        sqlx::query("UPDATE users SET password_hash = ? WHERE user_id = ?")
            .bind(&password_hash)
            .bind(user_id)
            .execute(&mut *tx)
            .await?;

        sqlx::query("DELETE FROM sessions WHERE user_id = ?")
            .bind(user_id)
            .execute(&mut *tx)
            .await?;

        tx.commit().await?;
        Ok(())
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
pub fn init_storage(path: &Path) -> io::Result<()> {
    std::fs::create_dir(path)?;
    std::fs::create_dir_all(path.join("media"))?;
    std::fs::create_dir_all(path.join("backups"))?;
    Ok(())
}

// ---------------------------------------------------------------------------
// Database helpers
// ---------------------------------------------------------------------------

fn make_app_state(pool: SqlitePool, mailer: Arc<dyn MailSender>) -> Arc<AppState> {
    Arc::new(AppState {
        site_config: Arc::new(SqliteSiteConfigStorage::new(pool.clone())),
        users: Arc::new(SqliteUserStorage::new(pool.clone())),
        sessions: Arc::new(SqliteSessionStorage::new(pool.clone())),
        invites: Arc::new(SqliteInviteStorage::new(pool.clone())),
        atomic: Arc::new(SqliteAtomicOps::new(pool.clone())),
        email_verifications: Arc::new(SqliteEmailVerificationStorage::new(pool.clone())),
        password_resets: Arc::new(SqlitePasswordResetStorage::new(pool)),
        mailer,
    })
}

fn make_postgres_app_state(pool: PgPool, mailer: Arc<dyn MailSender>) -> Arc<AppState> {
    Arc::new(AppState {
        site_config: Arc::new(PostgresSiteConfigStorage::new(pool.clone())),
        users: Arc::new(PostgresUserStorage::new(pool.clone())),
        sessions: Arc::new(PostgresSessionStorage::new(pool.clone())),
        invites: Arc::new(PostgresInviteStorage::new(pool.clone())),
        atomic: Arc::new(PostgresAtomicOps::new(pool.clone())),
        email_verifications: Arc::new(PostgresEmailVerificationStorage::new(pool.clone())),
        password_resets: Arc::new(PostgresPasswordResetStorage::new(pool)),
        mailer,
    })
}

async fn build_mailer(site_config: &dyn SiteConfigStorage) -> Arc<dyn MailSender> {
    if let Ok(path) = std::env::var("JAUNDER_MAIL_CAPTURE_FILE") {
        return Arc::new(FileMailSender::new(path)) as Arc<dyn MailSender>;
    }
    match load_smtp_config(site_config).await {
        Ok(Some(cfg)) => match crate::mailer::LettreMailSender::from_config(&cfg) {
            Ok(sender) => Arc::new(sender) as Arc<dyn MailSender>,
            Err(_) => Arc::new(NoopMailSender) as Arc<dyn MailSender>,
        },
        Ok(None) | Err(_) => Arc::new(NoopMailSender) as Arc<dyn MailSender>,
    }
}

async fn open_sqlite_database(
    options: &SqliteConnectOptions,
    create_if_missing: bool,
) -> sqlx::Result<Arc<AppState>> {
    let mut options = options.clone();
    if create_if_missing {
        options = options.create_if_missing(true);
    }
    let pool = sqlx::SqlitePool::connect_with(options).await?;
    sqlx::migrate!("./migrations/sqlite").run(&pool).await?;
    let site_config = SqliteSiteConfigStorage::new(pool.clone());
    let mailer = build_mailer(&site_config).await;
    Ok(make_app_state(pool, mailer))
}

async fn open_postgres_database(options: &PgConnectOptions) -> sqlx::Result<Arc<AppState>> {
    let pool = PgPool::connect_with(options.clone()).await?;
    sqlx::migrate!("./migrations/postgres").run(&pool).await?;
    let site_config = PostgresSiteConfigStorage::new(pool.clone());
    let mailer = build_mailer(&site_config).await;
    Ok(make_postgres_app_state(pool, mailer))
}

/// Opens (or creates) the database described by `opts`, runs pending
/// migrations, and returns an [`AppState`] bundling all storage handles.
pub async fn open_database(opts: &DbConnectOptions) -> sqlx::Result<Arc<AppState>> {
    match opts {
        DbConnectOptions::Sqlite(options) => open_sqlite_database(options, true).await,
        DbConnectOptions::Postgres { options, .. } => open_postgres_database(options).await,
    }
}

/// Opens an existing database described by `opts`, runs pending migrations.
///
/// Unlike [`open_database`], fails if the database does not already exist.
pub async fn open_existing_database(opts: &DbConnectOptions) -> sqlx::Result<Arc<AppState>> {
    match opts {
        DbConnectOptions::Sqlite(options) => open_sqlite_database(options, false).await,
        DbConnectOptions::Postgres { options, .. } => open_postgres_database(options).await,
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

    #[test]
    fn init_storage_fails_on_missing_parent() {
        let storage = std::path::Path::new("/nonexistent/path/to/storage");
        let result = init_storage(storage);
        assert!(result.is_err());
    }
}
