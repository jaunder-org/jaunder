use std::time::Duration;
use std::{fmt, io, path::Path, str::FromStr, sync::Arc};

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use log::LevelFilter;
use sqlx::postgres::PgConnectOptions;
use sqlx::sqlite::{SqliteConnectOptions, SqliteJournalMode};
use sqlx::ConnectOptions;
use sqlx::PgPool;
use sqlx::SqlitePool;

// Storage traits, records, error types, and AppState are defined in the
// `common` crate so that both `web` (server functions) and `server` can use
// them without a circular dependency.  Re-export everything for
// backward-compatibility with existing server-crate consumers.
pub use common::storage::{
    AppState, AtomicOps, ConfirmPasswordResetError, CreatePostError, CreatePostInput,
    CreateUserError, EmailVerificationStorage, InviteRecord, InviteStorage, ListByTagError,
    PasswordResetStorage, PostCursor, PostFormat, PostRecord, PostRevisionRecord, PostStorage,
    PostTag, ProfileUpdate, RegisterWithInviteError, SessionAuthError, SessionRecord,
    SessionStorage, SiteConfigStorage, TaggingError, UpdatePostError, UpdatePostInput,
    UseEmailVerificationError, UseInviteError, UsePasswordResetError, UserAuthError, UserRecord,
    UserStorage,
};

use crate::mailer::FileMailSender;
use common::mailer::{MailSender, NoopMailSender};
use common::password::Password;
use common::smtp::load_smtp_config;
use common::username::Username;

mod sqlite;
pub use sqlite::{
    SqliteEmailVerificationStorage, SqliteInviteStorage, SqlitePasswordResetStorage,
    SqlitePostStorage, SqliteSessionStorage, SqliteSiteConfigStorage, SqliteUserStorage,
};
mod postgres;
pub use postgres::{
    PostgresAtomicOps, PostgresEmailVerificationStorage, PostgresInviteStorage,
    PostgresPasswordResetStorage, PostgresPostStorage, PostgresSessionStorage,
    PostgresSiteConfigStorage, PostgresUserStorage,
};

pub(super) type UserRecordParts = (
    i64,
    String,
    Option<String>,
    Option<String>,
    DateTime<Utc>,
    Option<DateTime<Utc>>,
    Option<String>,
    bool,
);

pub(super) fn build_user_record(
    (
        user_id,
        username,
        display_name,
        bio,
        created_at,
        last_authenticated_at,
        email,
        email_verified,
    ): UserRecordParts,
) -> sqlx::Result<UserRecord> {
    let username = username
        .parse()
        .map_err(|e| sqlx::Error::Decode(Box::new(e)))?;
    let email = email
        .map(|s| s.parse().map_err(|e| sqlx::Error::Decode(Box::new(e))))
        .transpose()?;
    Ok(UserRecord {
        user_id,
        username,
        display_name,
        bio,
        created_at,
        last_authenticated_at,
        email,
        email_verified,
    })
}

pub(super) fn build_session_record(
    token_hash: String,
    user_id: i64,
    username: String,
    label: Option<String>,
    created_at: DateTime<Utc>,
    last_used_at: DateTime<Utc>,
) -> sqlx::Result<SessionRecord> {
    Ok(SessionRecord {
        token_hash,
        user_id,
        username: username
            .parse::<Username>()
            .map_err(|e| sqlx::Error::Decode(Box::new(e)))?,
        label,
        created_at,
        last_used_at,
    })
}

pub(super) fn build_invite_record(
    code: String,
    created_at: DateTime<Utc>,
    expires_at: DateTime<Utc>,
    used_at: Option<DateTime<Utc>>,
    used_by: Option<i64>,
) -> InviteRecord {
    InviteRecord {
        code,
        created_at,
        expires_at,
        used_at,
        used_by,
    }
}

pub(super) type PostRecordParts = (
    i64,                   // post_id
    i64,                   // user_id
    String,                // title
    String,                // slug
    String,                // body
    String,                // format
    String,                // rendered_html
    DateTime<Utc>,         // created_at
    DateTime<Utc>,         // updated_at
    Option<DateTime<Utc>>, // published_at
    Option<DateTime<Utc>>, // deleted_at
);

pub(super) fn build_post_record(
    (
        post_id,
        user_id,
        title,
        slug,
        body,
        format,
        rendered_html,
        created_at,
        updated_at,
        published_at,
        deleted_at,
    ): PostRecordParts,
) -> sqlx::Result<PostRecord> {
    use common::slug::Slug;
    let slug = slug
        .parse::<Slug>()
        .map_err(|e| sqlx::Error::Decode(Box::new(e)))?;
    let format = format
        .parse::<PostFormat>()
        .map_err(|e| sqlx::Error::Decode(Box::new(e)))?;
    Ok(PostRecord {
        post_id,
        user_id,
        title,
        slug,
        body,
        format,
        rendered_html,
        created_at,
        updated_at,
        published_at,
        deleted_at,
    })
}

#[tracing::instrument(name = "crypto.password.hash", skip(password))]
pub(super) async fn hash_password(password: Password) -> io::Result<String> {
    tokio::task::spawn_blocking(move || password.hash())
        .await
        .map_err(io::Error::other)?
        .map_err(io::Error::other)
}

#[tracing::instrument(name = "crypto.password.verify", skip(password, hash))]
pub(super) async fn verify_password(password: Password, hash: String) -> io::Result<bool> {
    tokio::task::spawn_blocking(move || password.verify(&hash))
        .await
        .map_err(io::Error::other)?
        .map_err(io::Error::other)
}

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
        let password_hash = hash_password(password.clone())
            .await
            .map_err(|e| RegisterWithInviteError::Internal(sqlx::Error::Io(e)))?;

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

        let password_hash = hash_password(new_password.clone())
            .await
            .map_err(|e| ConfirmPasswordResetError::Internal(sqlx::Error::Io(e)))?;

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
        password_resets: Arc::new(SqlitePasswordResetStorage::new(pool.clone())),
        posts: Arc::new(SqlitePostStorage::new(pool)),
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
        password_resets: Arc::new(PostgresPasswordResetStorage::new(pool.clone())),
        posts: Arc::new(PostgresPostStorage::new(pool)),
        mailer,
    })
}

#[tracing::instrument(name = "storage.mailer.build", skip(site_config))]
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

#[tracing::instrument(
    name = "storage.sqlite.open_database",
    skip(options),
    fields(create_if_missing)
)]
async fn open_sqlite_database(
    options: &SqliteConnectOptions,
    create_if_missing: bool,
) -> sqlx::Result<Arc<AppState>> {
    let mut options = options.clone();
    if create_if_missing {
        options = options.create_if_missing(true);
    }
    // WAL mode allows concurrent readers while a writer is active, dramatically
    // reducing SQLITE_BUSY errors under load.  The 5-second busy timeout lets
    // SQLite retry automatically instead of failing immediately when it cannot
    // obtain a lock.
    options = options
        .journal_mode(SqliteJournalMode::Wal)
        .busy_timeout(Duration::from_secs(5))
        .log_slow_statements(LevelFilter::Warn, sql_slow_query_threshold());
    let pool = sqlx::SqlitePool::connect_with(options).await?;
    sqlx::migrate!("./migrations/sqlite").run(&pool).await?;
    let site_config = SqliteSiteConfigStorage::new(pool.clone());
    let mailer = build_mailer(&site_config).await;
    Ok(make_app_state(pool, mailer))
}

fn postgres_password_from_env() -> io::Result<Option<String>> {
    if let Ok(path) = std::env::var("JAUNDER_DB_PASSWORD_FILE") {
        return std::fs::read_to_string(path).map(|s| Some(s.trim_end().to_owned()));
    }

    Ok(std::env::var("JAUNDER_DB_PASSWORD").ok())
}

fn sql_slow_query_threshold() -> Duration {
    std::env::var("JAUNDER_SQL_SLOW_MS")
        .ok()
        .and_then(|value| value.parse::<u64>().ok())
        .map(Duration::from_millis)
        .unwrap_or(Duration::from_millis(100))
}

fn resolved_postgres_options(options: &PgConnectOptions) -> sqlx::Result<PgConnectOptions> {
    let mut options = options.clone();
    if let Some(password) = postgres_password_from_env().map_err(sqlx::Error::Io)? {
        options = options.password(&password);
    }
    options = options.log_slow_statements(LevelFilter::Warn, sql_slow_query_threshold());
    Ok(options)
}

#[tracing::instrument(name = "storage.postgres.open_database", skip(options))]
async fn open_postgres_database(options: &PgConnectOptions) -> sqlx::Result<Arc<AppState>> {
    let options = resolved_postgres_options(options)?;
    let pool = PgPool::connect_with(options).await?;
    sqlx::migrate!("./migrations/postgres").run(&pool).await?;
    let site_config = PostgresSiteConfigStorage::new(pool.clone());
    let mailer = build_mailer(&site_config).await;
    Ok(make_postgres_app_state(pool, mailer))
}

/// Opens (or creates) the database described by `opts`, runs pending
/// migrations, and returns an [`AppState`] bundling all storage handles.
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
#[tracing::instrument(name = "storage.open_existing_database", skip(opts))]
pub async fn open_existing_database(opts: &DbConnectOptions) -> sqlx::Result<Arc<AppState>> {
    match opts {
        DbConnectOptions::Sqlite(options) => open_sqlite_database(options, false).await,
        DbConnectOptions::Postgres { options, .. } => open_postgres_database(options).await,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;
    use tempfile::TempDir;

    static ENV_LOCK: Mutex<()> = Mutex::new(());

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

    #[test]
    fn postgres_password_prefers_file_over_env() {
        let _guard = ENV_LOCK.lock().unwrap();
        std::env::set_var("JAUNDER_DB_PASSWORD", "from-env");
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("db-password");
        std::fs::write(&path, "from-file\n").unwrap();
        std::env::set_var("JAUNDER_DB_PASSWORD_FILE", &path);

        let password = postgres_password_from_env().unwrap();

        std::env::remove_var("JAUNDER_DB_PASSWORD");
        std::env::remove_var("JAUNDER_DB_PASSWORD_FILE");
        assert_eq!(password.as_deref(), Some("from-file"));
    }

    #[test]
    fn postgres_password_uses_env_when_file_unset() {
        let _guard = ENV_LOCK.lock().unwrap();
        std::env::remove_var("JAUNDER_DB_PASSWORD_FILE");
        std::env::set_var("JAUNDER_DB_PASSWORD", "from-env");

        let password = postgres_password_from_env().unwrap();

        std::env::remove_var("JAUNDER_DB_PASSWORD");
        assert_eq!(password.as_deref(), Some("from-env"));
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
    fn test_build_user_record() {
        let now = Utc::now();
        let parts: UserRecordParts = (
            1,
            "alice".to_string(),
            Some("Alice".to_string()),
            Some("Bio".to_string()),
            now,
            Some(now),
            Some("alice@example.com".to_string()),
            true,
        );
        let record = build_user_record(parts).unwrap();
        assert_eq!(record.user_id, 1);
        assert_eq!(record.username.as_str(), "alice");
        assert_eq!(record.email.as_ref().unwrap().as_str(), "alice@example.com");
    }

    #[test]
    fn test_build_session_record() {
        let now = Utc::now();
        let record = build_session_record(
            "hash".to_string(),
            1,
            "alice".to_string(),
            Some("label".to_string()),
            now,
            now,
        )
        .unwrap();
        assert_eq!(record.token_hash, "hash");
        assert_eq!(record.username.as_str(), "alice");
    }

    #[test]
    fn test_build_invite_record() {
        let created_at = Utc::now();
        let expires_at = created_at + chrono::Duration::days(7);
        let used_at = created_at + chrono::Duration::hours(1);
        let record = build_invite_record(
            "invite-code".to_string(),
            created_at,
            expires_at,
            Some(used_at),
            Some(7),
        );

        assert_eq!(record.code, "invite-code");
        assert_eq!(record.created_at, created_at);
        assert_eq!(record.expires_at, expires_at);
        assert_eq!(record.used_at, Some(used_at));
        assert_eq!(record.used_by, Some(7));
    }

    #[test]
    fn test_build_post_record() {
        let now = Utc::now();
        let record = build_post_record((
            10,
            20,
            "Hello".to_string(),
            "hello-world".to_string(),
            "Body".to_string(),
            "markdown".to_string(),
            "<p>Body</p>".to_string(),
            now,
            now,
            Some(now),
            None,
        ))
        .unwrap();

        assert_eq!(record.post_id, 10);
        assert_eq!(record.user_id, 20);
        assert_eq!(record.slug.as_str(), "hello-world");
        assert_eq!(record.format, PostFormat::Markdown);
        assert_eq!(record.published_at, Some(now));
        assert_eq!(record.deleted_at, None);
    }

    #[test]
    fn test_build_post_record_rejects_invalid_slug() {
        let now = Utc::now();
        let err = build_post_record((
            10,
            20,
            "Hello".to_string(),
            "not a slug".to_string(),
            "Body".to_string(),
            "markdown".to_string(),
            "<p>Body</p>".to_string(),
            now,
            now,
            None,
            None,
        ))
        .unwrap_err();

        assert!(matches!(err, sqlx::Error::Decode(_)));
    }

    #[test]
    fn test_build_post_record_rejects_invalid_format() {
        let now = Utc::now();
        let err = build_post_record((
            10,
            20,
            "Hello".to_string(),
            "hello-world".to_string(),
            "Body".to_string(),
            "html".to_string(),
            "<p>Body</p>".to_string(),
            now,
            now,
            None,
            None,
        ))
        .unwrap_err();

        assert!(matches!(err, sqlx::Error::Decode(_)));
    }

    #[tokio::test]
    async fn test_hash_and_verify_password() {
        let password: Password = "password123".parse().unwrap();
        let hash = hash_password(password.clone()).await.unwrap();

        assert!(verify_password(password.clone(), hash.clone())
            .await
            .unwrap());
        assert!(!verify_password("other-pass".parse().unwrap(), hash)
            .await
            .unwrap());
    }

    #[tokio::test]
    async fn test_verify_password_rejects_invalid_hash() {
        let err = verify_password("password123".parse().unwrap(), "not-a-hash".to_string())
            .await
            .unwrap_err();

        assert_eq!(err.kind(), io::ErrorKind::Other);
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

    #[test]
    fn postgres_password_returns_none_when_unset() {
        let _guard = ENV_LOCK.lock().unwrap();
        std::env::remove_var("JAUNDER_DB_PASSWORD");
        std::env::remove_var("JAUNDER_DB_PASSWORD_FILE");

        let password = postgres_password_from_env().unwrap();

        assert_eq!(password, None);
    }

    #[tokio::test]
    async fn test_make_postgres_app_state() {
        let pool = sqlx::PgPool::connect_lazy("postgres://localhost/db").unwrap();
        let mailer = Arc::new(common::mailer::NoopMailSender);
        let _ = make_postgres_app_state(pool, mailer);
    }

    #[tokio::test]
    async fn test_open_database_postgres() {
        let opts = "postgres://localhost:1/db"
            .parse::<DbConnectOptions>()
            .unwrap();
        let _ =
            tokio::time::timeout(std::time::Duration::from_millis(50), open_database(&opts)).await;
    }

    #[tokio::test]
    async fn test_open_existing_database_postgres() {
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
