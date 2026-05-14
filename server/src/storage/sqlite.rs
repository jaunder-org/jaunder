use std::{sync::Arc, time::Duration};

use chrono::{DateTime, Utc};
use email_address::EmailAddress;
use log::LevelFilter;
use sqlx::{
    sqlite::{SqliteConnectOptions, SqliteJournalMode},
    ConnectOptions, Row, SqlitePool,
};

use async_trait::async_trait;

use common::mailer::MailSender;
use common::password::Password;
use common::slug::Slug;
use common::storage::{
    AppState, AtomicOps, ConfirmPasswordResetError, CreateMediaError, CreatePostError,
    CreatePostInput, CreateUserError, DeleteMediaError, EmailVerificationStorage, InviteRecord,
    InviteStorage, ListByTagError, MediaRecord, MediaSource, MediaStorage, PasswordResetStorage,
    PostCursor, PostRecord, PostStorage, PostTag, ProfileUpdate, RegisterWithInviteError,
    SessionAuthError, SessionRecord, SessionStorage, SiteConfigStorage, TagRecord, TaggingError,
    UpdatePostError, UpdatePostInput, UseEmailVerificationError, UseInviteError,
    UsePasswordResetError, UserAuthError, UserConfigStorage, UserRecord, UserStorage,
};
use common::tag::Tag;
use common::username::Username;
use tracing::Instrument;

use super::{
    build_mailer, email_verification_claim_error, generate_hashed_token, invite_record_from_row,
    media_record_from_row, password_reset_claim_error, post_record_from_row,
    session_record_from_row, sql_slow_query_threshold, user_record_from_row, InviteRow, MediaRow,
    PostRow, SessionRow, UserRow,
};

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
        posts: Arc::new(SqlitePostStorage::new(pool.clone())),
        media: Arc::new(SqliteMediaStorage::new(pool.clone())),
        user_config: Arc::new(SqliteUserConfigStorage::new(pool)),
        mailer,
    })
}

#[tracing::instrument(
    name = "storage.sqlite.open_database",
    skip(options),
    fields(create_if_missing)
)]
pub(super) async fn open_sqlite_database(
    options: &SqliteConnectOptions,
    create_if_missing: bool,
) -> sqlx::Result<Arc<AppState>> {
    let mut options = options.clone();
    if create_if_missing {
        options = options.create_if_missing(true);
    }
    // WAL mode allows concurrent readers while a writer is active, dramatically
    // reducing SQLITE_BUSY errors under load. The busy timeout lets SQLite retry
    // automatically instead of failing immediately when it cannot obtain a lock.
    options = options
        .journal_mode(SqliteJournalMode::Wal)
        .busy_timeout(Duration::from_secs(5))
        .log_slow_statements(LevelFilter::Warn, sql_slow_query_threshold());

    let pool = sqlx::SqlitePool::connect_with(options).await?;

    // Increase cache size to 32MB. SQLite page size is 4KB by default (usually),
    // so 32MB is 8192 pages. The `-32000` syntax tells SQLite 32MB.
    sqlx::query("PRAGMA cache_size = -32000")
        .execute(&pool)
        .await?;

    sqlx::migrate!("./migrations/sqlite").run(&pool).await?;
    let site_config = SqliteSiteConfigStorage::new(pool.clone());
    let mailer = build_mailer(&site_config).await;
    Ok(make_app_state(pool, mailer))
}

// ---------------------------------------------------------------------------
// SiteConfig
// ---------------------------------------------------------------------------

/// SQLite-backed [`SiteConfigStorage`].
pub struct SqliteSiteConfigStorage {
    pool: SqlitePool,
}

impl SqliteSiteConfigStorage {
    #[must_use]
    pub fn new(pool: SqlitePool) -> Self {
        Self { pool }
    }
}

#[async_trait]
impl SiteConfigStorage for SqliteSiteConfigStorage {
    async fn get(&self, key: &str) -> sqlx::Result<Option<String>> {
        let row = sqlx::query_as::<_, (String,)>("SELECT value FROM site_config WHERE key = $1")
            .bind(key)
            .fetch_optional(&self.pool)
            .await?;
        Ok(row.map(|(v,)| v))
    }

    async fn set(&self, key: &str, value: &str) -> sqlx::Result<()> {
        sqlx::query(
            "INSERT INTO site_config (key, value) VALUES ($1, $2)
             ON CONFLICT(key) DO UPDATE SET value = excluded.value",
        )
        .bind(key)
        .bind(value)
        .execute(&self.pool)
        .await?;
        Ok(())
    }
}

// ---------------------------------------------------------------------------
// Users
// ---------------------------------------------------------------------------

/// SQLite-backed [`UserStorage`].
pub struct SqliteUserStorage {
    pool: SqlitePool,
}

impl SqliteUserStorage {
    #[must_use]
    pub fn new(pool: SqlitePool) -> Self {
        Self { pool }
    }
}

#[async_trait]
impl UserStorage for SqliteUserStorage {
    #[tracing::instrument(
        name = "storage.sqlite.user.create_user",
        skip(self, password, display_name),
        fields(username = %username.as_str())
    )]
    async fn create_user(
        &self,
        username: &Username,
        password: &Password,
        display_name: Option<&str>,
        is_operator: bool,
    ) -> Result<i64, CreateUserError> {
        let password_hash = super::hash_password(password.clone())
            .instrument(tracing::info_span!(
                "storage.sqlite.user.create_user.hash_password"
            ))
            .await
            .map_err(|e| CreateUserError::Internal(sqlx::Error::Io(e)))?;

        let now = Utc::now();

        let result = sqlx::query_scalar::<_, i64>(
            "INSERT INTO users (username, password_hash, display_name, created_at, is_operator)
             VALUES ($1, $2, $3, $4, $5)
             RETURNING user_id",
        )
        .bind(username.as_str())
        .bind(&password_hash)
        .bind(display_name)
        .bind(now)
        .bind(is_operator)
        .fetch_one(&self.pool)
        .instrument(tracing::info_span!(
            "storage.sqlite.user.create_user.insert_user_row"
        ))
        .await;

        match result {
            Ok(id) => Ok(id),
            Err(sqlx::Error::Database(e)) if e.is_unique_violation() => {
                Err(CreateUserError::UsernameTaken)
            }
            Err(e) => Err(CreateUserError::Internal(e)),
        }
    }

    #[tracing::instrument(
        name = "storage.sqlite.user.authenticate",
        skip(self, password),
        fields(username = %username.as_str())
    )]
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
                Option<String>,
                bool,
                bool,
            ),
        >(
            "SELECT user_id, username, display_name, bio, created_at, last_authenticated_at, password_hash, email, email_verified, is_operator
             FROM users WHERE username = $1",
        )
        .bind(username.as_str())
        .fetch_optional(&self.pool)
        .instrument(tracing::info_span!(
            "storage.sqlite.user.authenticate.lookup_user"
        ))
        .await
        .map_err(|e| UserAuthError::Internal(e.to_string()))?;

        let Some((
            user_id,
            username,
            display_name,
            bio,
            created_at,
            _last_authenticated_at,
            hash,
            email,
            email_verified,
            is_operator,
        )) = row
        else {
            return Err(UserAuthError::InvalidCredentials);
        };

        let valid = super::verify_password(password.clone(), hash)
            .instrument(tracing::info_span!(
                "storage.sqlite.user.authenticate.verify_password"
            ))
            .await
            .map_err(|e| UserAuthError::Internal(e.to_string()))?;

        if !valid {
            return Err(UserAuthError::InvalidCredentials);
        }

        let now = Utc::now();

        sqlx::query("UPDATE users SET last_authenticated_at = $1 WHERE user_id = $2")
            .bind(now)
            .bind(user_id)
            .execute(&self.pool)
            .instrument(tracing::info_span!(
                "storage.sqlite.user.authenticate.update_last_authenticated_at"
            ))
            .await
            .map_err(|e| UserAuthError::Internal(e.to_string()))?;

        super::build_user_record((
            user_id,
            username,
            display_name,
            bio,
            created_at,
            Some(now),
            email,
            email_verified,
            is_operator,
        ))
        .map_err(|e| UserAuthError::Internal(e.to_string()))
    }

    async fn get_user(&self, user_id: i64) -> sqlx::Result<Option<UserRecord>> {
        let row = sqlx::query_as::<_, UserRow>(
            "SELECT user_id, username, display_name, bio, created_at, last_authenticated_at, email, email_verified, is_operator
             FROM users WHERE user_id = $1",
        )
        .bind(user_id)
        .fetch_optional(&self.pool)
        .await?;
        Ok(row.map(user_record_from_row).transpose()?)
    }

    async fn get_user_by_username(&self, username: &Username) -> sqlx::Result<Option<UserRecord>> {
        let row = sqlx::query_as::<_, UserRow>(
            "SELECT user_id, username, display_name, bio, created_at, last_authenticated_at, email, email_verified, is_operator
             FROM users WHERE username = $1",
        )
        .bind(username.as_str())
        .fetch_optional(&self.pool)
        .await?;
        Ok(row.map(user_record_from_row).transpose()?)
    }

    async fn set_email(
        &self,
        user_id: i64,
        email: Option<&EmailAddress>,
        verified: bool,
    ) -> sqlx::Result<()> {
        sqlx::query("UPDATE users SET email = $1, email_verified = $2 WHERE user_id = $3")
            .bind(email.map(EmailAddress::as_str))
            .bind(verified)
            .bind(user_id)
            .execute(&self.pool)
            .await?;
        Ok(())
    }

    async fn update_profile(&self, user_id: i64, update: &ProfileUpdate<'_>) -> sqlx::Result<()> {
        sqlx::query("UPDATE users SET display_name = $1, bio = $2 WHERE user_id = $3")
            .bind(update.display_name)
            .bind(update.bio)
            .bind(user_id)
            .execute(&self.pool)
            .await?;
        Ok(())
    }

    async fn set_password(&self, user_id: i64, new_password: &Password) -> sqlx::Result<()> {
        let password_hash = super::hash_password(new_password.clone())
            .await
            .map_err(sqlx::Error::Io)?;

        sqlx::query("UPDATE users SET password_hash = $1 WHERE user_id = $2")
            .bind(&password_hash)
            .bind(user_id)
            .execute(&self.pool)
            .await?;
        Ok(())
    }
}

// ---------------------------------------------------------------------------
// Sessions
// ---------------------------------------------------------------------------

/// SQLite-backed [`SessionStorage`].
pub struct SqliteSessionStorage {
    pool: SqlitePool,
}

impl SqliteSessionStorage {
    #[must_use]
    pub fn new(pool: SqlitePool) -> Self {
        Self { pool }
    }
}

#[async_trait]
impl SessionStorage for SqliteSessionStorage {
    #[tracing::instrument(
        name = "storage.sqlite.session.create",
        skip(self, label),
        fields(user_id)
    )]
    async fn create_session(&self, user_id: i64, label: Option<&str>) -> sqlx::Result<String> {
        let (raw_token, token_hash) = generate_hashed_token()?;
        let now = Utc::now();

        sqlx::query(
            "INSERT INTO sessions (token_hash, user_id, label, created_at, last_used_at)
             VALUES ($1, $2, $3, $4, $5)",
        )
        .bind(&token_hash)
        .bind(user_id)
        .bind(label)
        .bind(now)
        .bind(now)
        .execute(&self.pool)
        .await?;

        Ok(raw_token)
    }

    #[tracing::instrument(name = "storage.sqlite.session.authenticate", skip(self, raw_token))]
    async fn authenticate(&self, raw_token: &str) -> Result<SessionRecord, SessionAuthError> {
        let token_hash =
            crate::auth::hash_token(raw_token).map_err(|_| SessionAuthError::InvalidToken)?;

        let now = Utc::now();

        // Perform an atomic update and read in a single statement. This
        // avoids the need for a multi-statement transaction, which can
        // cause SQLITE_BUSY contention in high-concurrency environments.
        //
        // Note: SQLite's RETURNING clause is used with a correlated subquery
        // Split the update and select into two operations to avoid the
        // subquery overhead in the RETURNING clause and potentially
        // reduce disk I/O contention.
        let mut tx = self.pool.begin().await?;
        sqlx::query("UPDATE sessions SET last_used_at = $1 WHERE token_hash = $2")
            .bind(now)
            .bind(&token_hash)
            .execute(&mut *tx)
            .await?;

        let row = sqlx::query_as::<_, SessionRow>(
            "SELECT s.token_hash, s.user_id, u.username, s.label, s.created_at, s.last_used_at
             FROM sessions s
             JOIN users u ON u.user_id = s.user_id
             WHERE s.token_hash = $1",
        )
        .bind(&token_hash)
        .fetch_optional(&mut *tx)
        .await?
        .ok_or(SessionAuthError::SessionNotFound)?;

        tx.commit().await?;

        let record = session_record_from_row(row)?;
        Ok(record)
    }

    #[tracing::instrument(name = "storage.sqlite.session.revoke", skip(self, token_hash))]
    async fn revoke_session(&self, token_hash: &str) -> sqlx::Result<()> {
        sqlx::query("DELETE FROM sessions WHERE token_hash = $1")
            .bind(token_hash)
            .execute(&self.pool)
            .await?;
        Ok(())
    }

    async fn list_sessions(&self, user_id: i64) -> sqlx::Result<Vec<SessionRecord>> {
        let rows = sqlx::query_as::<_, SessionRow>(
            "SELECT s.token_hash, s.user_id, u.username, s.label, s.created_at, s.last_used_at
             FROM sessions s JOIN users u ON s.user_id = u.user_id
             WHERE s.user_id = $1",
        )
        .bind(user_id)
        .fetch_all(&self.pool)
        .await?;

        rows.into_iter().map(session_record_from_row).collect()
    }
}

// ---------------------------------------------------------------------------
// Invites
// ---------------------------------------------------------------------------

/// SQLite-backed [`InviteStorage`].
pub struct SqliteInviteStorage {
    pool: SqlitePool,
}

impl SqliteInviteStorage {
    #[must_use]
    pub fn new(pool: SqlitePool) -> Self {
        Self { pool }
    }
}

#[async_trait]
impl InviteStorage for SqliteInviteStorage {
    async fn create_invite(&self, expires_at: DateTime<Utc>) -> sqlx::Result<String> {
        let code = crate::auth::generate_token();
        let now = Utc::now();

        sqlx::query("INSERT INTO invites (code, created_at, expires_at) VALUES ($1, $2, $3)")
            .bind(&code)
            .bind(now)
            .bind(expires_at)
            .execute(&self.pool)
            .await?;

        Ok(code)
    }

    async fn use_invite(&self, code: &str, user_id: i64) -> Result<(), UseInviteError> {
        let mut tx = self
            .pool
            .begin()
            .await
            .map_err(|_| UseInviteError::NotFound)?;

        let row = sqlx::query_as::<_, InviteRow>(
            "SELECT code, created_at, expires_at, used_at, used_by
             FROM invites WHERE code = $1",
        )
        .bind(code)
        .fetch_optional(&mut *tx)
        .await
        .map_err(|_| UseInviteError::NotFound)?
        .ok_or(UseInviteError::NotFound)?;

        let record = invite_record_from_row(row);

        if record.used_at.is_some() {
            return Err(UseInviteError::AlreadyUsed);
        }

        let now = Utc::now();
        if record.expires_at <= now {
            return Err(UseInviteError::Expired);
        }

        sqlx::query("UPDATE invites SET used_at = $1, used_by = $2 WHERE code = $3")
            .bind(now)
            .bind(user_id)
            .bind(code)
            .execute(&mut *tx)
            .await
            .map_err(|_| UseInviteError::NotFound)?;

        tx.commit().await.map_err(|_| UseInviteError::NotFound)?;

        Ok(())
    }

    async fn list_invites(&self) -> sqlx::Result<Vec<InviteRecord>> {
        let rows = sqlx::query_as::<_, InviteRow>(
            "SELECT code, created_at, expires_at, used_at, used_by FROM invites",
        )
        .fetch_all(&self.pool)
        .await?;

        Ok(rows.into_iter().map(invite_record_from_row).collect())
    }
}

// ---------------------------------------------------------------------------
// EmailVerifications
// ---------------------------------------------------------------------------

/// SQLite-backed [`EmailVerificationStorage`].
pub struct SqliteEmailVerificationStorage {
    pool: SqlitePool,
}

impl SqliteEmailVerificationStorage {
    #[must_use]
    pub fn new(pool: SqlitePool) -> Self {
        Self { pool }
    }
}

#[async_trait]
impl EmailVerificationStorage for SqliteEmailVerificationStorage {
    async fn create_email_verification(
        &self,
        user_id: i64,
        email: &str,
        expires_at: DateTime<Utc>,
    ) -> sqlx::Result<String> {
        let (raw_token, token_hash) = generate_hashed_token()?;
        let now = Utc::now();

        let mut tx = self.pool.begin().await?;

        // Supersede any existing pending token for this user by setting its
        // expires_at to its created_at, making it appear immediately expired.
        sqlx::query(
            "UPDATE email_verifications
             SET expires_at = created_at
             WHERE user_id = $1 AND used_at IS NULL AND expires_at > $2",
        )
        .bind(user_id)
        .bind(now)
        .execute(&mut *tx)
        .await?;

        sqlx::query(
            "INSERT INTO email_verifications
             (token_hash, user_id, email, created_at, expires_at)
             VALUES ($1, $2, $3, $4, $5)",
        )
        .bind(&token_hash)
        .bind(user_id)
        .bind(email)
        .bind(now)
        .bind(expires_at)
        .execute(&mut *tx)
        .await?;

        tx.commit().await?;

        Ok(raw_token)
    }

    async fn use_email_verification(
        &self,
        raw_token: &str,
    ) -> Result<(i64, String), UseEmailVerificationError> {
        let token_hash =
            crate::auth::hash_token(raw_token).map_err(|_| UseEmailVerificationError::NotFound)?;

        let now = Utc::now();

        // Atomically claim the token: the UPDATE succeeds only when the token
        // exists, has not yet been used, and has not expired.  This single
        // statement is the "claim" — no separate read is needed first, so two
        // concurrent requests cannot both succeed.  RETURNING gives us the
        // data we need without a second round-trip.
        let claimed = sqlx::query_as::<_, (i64, String)>(
            "UPDATE email_verifications SET used_at = $1
             WHERE token_hash = $2 AND used_at IS NULL AND expires_at > $3
             RETURNING user_id, email",
        )
        .bind(now)
        .bind(&token_hash)
        .bind(now)
        .fetch_optional(&self.pool)
        .await?;

        if let Some((user_id, email)) = claimed {
            return Ok((user_id, email));
        }

        // Zero rows affected — inspect the row to return the right error.
        let row = sqlx::query_as::<_, (Option<DateTime<Utc>>, DateTime<Utc>)>(
            "SELECT used_at, expires_at FROM email_verifications WHERE token_hash = $1",
        )
        .bind(&token_hash)
        .fetch_optional(&self.pool)
        .await?;

        Err(email_verification_claim_error(row))
    }
}

// ---------------------------------------------------------------------------
// PasswordResets
// ---------------------------------------------------------------------------

/// SQLite-backed [`PasswordResetStorage`].
pub struct SqlitePasswordResetStorage {
    pool: SqlitePool,
}

impl SqlitePasswordResetStorage {
    #[must_use]
    pub fn new(pool: SqlitePool) -> Self {
        Self { pool }
    }
}

#[async_trait]
impl PasswordResetStorage for SqlitePasswordResetStorage {
    async fn create_password_reset(
        &self,
        user_id: i64,
        expires_at: DateTime<Utc>,
    ) -> sqlx::Result<String> {
        let (raw_token, token_hash) = generate_hashed_token()?;
        let now = Utc::now();

        sqlx::query(
            "INSERT INTO password_resets (token_hash, user_id, created_at, expires_at)
             VALUES ($1, $2, $3, $4)",
        )
        .bind(&token_hash)
        .bind(user_id)
        .bind(now)
        .bind(expires_at)
        .execute(&self.pool)
        .await?;

        Ok(raw_token)
    }

    async fn use_password_reset(&self, raw_token: &str) -> Result<i64, UsePasswordResetError> {
        let token_hash =
            crate::auth::hash_token(raw_token).map_err(|_| UsePasswordResetError::NotFound)?;

        let now = Utc::now();

        let claimed = sqlx::query_as::<_, (i64,)>(
            "UPDATE password_resets SET used_at = $1
             WHERE token_hash = $2 AND used_at IS NULL AND expires_at > $3
             RETURNING user_id",
        )
        .bind(now)
        .bind(&token_hash)
        .bind(now)
        .fetch_optional(&self.pool)
        .await?;

        if let Some((user_id,)) = claimed {
            return Ok(user_id);
        }

        let row = sqlx::query_as::<_, (Option<DateTime<Utc>>, DateTime<Utc>)>(
            "SELECT used_at, expires_at FROM password_resets WHERE token_hash = $1",
        )
        .bind(&token_hash)
        .fetch_optional(&self.pool)
        .await?;

        Err(password_reset_claim_error(row))
    }
}

// ---------------------------------------------------------------------------
// AtomicOps
// ---------------------------------------------------------------------------

/// `SQLite` implementation of [`AtomicOps`].
///
/// Holds the pool directly so it can span multiple tables in a single
/// transaction without going through the individual storage trait objects.
pub struct SqliteAtomicOps {
    pool: SqlitePool,
}

impl SqliteAtomicOps {
    #[must_use]
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
        is_operator: bool,
        invite_code: &str,
    ) -> Result<i64, RegisterWithInviteError> {
        let mut tx = self.pool.begin().await?;

        let row = sqlx::query_as::<_, (Option<DateTime<Utc>>, DateTime<Utc>)>(
            "SELECT used_at, expires_at FROM invites WHERE code = $1",
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

        let password_hash = super::hash_password(password.clone())
            .await
            .map_err(|e| RegisterWithInviteError::Internal(sqlx::Error::Io(e)))?;

        let result = sqlx::query_scalar::<_, i64>(
            "INSERT INTO users (username, password_hash, display_name, created_at, is_operator)
             VALUES ($1, $2, $3, $4, $5)
             RETURNING user_id",
        )
        .bind(username.as_str())
        .bind(&password_hash)
        .bind(display_name)
        .bind(now)
        .bind(is_operator)
        .fetch_one(&mut *tx)
        .await;

        let user_id = match result {
            Ok(id) => id,
            Err(sqlx::Error::Database(error)) if error.is_unique_violation() => {
                return Err(RegisterWithInviteError::UsernameTaken);
            }
            Err(error) => return Err(RegisterWithInviteError::Internal(error)),
        };

        sqlx::query("UPDATE invites SET used_at = $1, used_by = $2 WHERE code = $3")
            .bind(now)
            .bind(user_id)
            .bind(invite_code)
            .execute(&mut *tx)
            .await?;

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

        let password_hash = super::hash_password(new_password.clone())
            .await
            .map_err(|e| ConfirmPasswordResetError::Internal(sqlx::Error::Io(e)))?;

        let mut tx = self.pool.begin().await?;
        let now = Utc::now();

        let claimed = sqlx::query_as::<_, (i64,)>(
            "UPDATE password_resets SET used_at = $1
             WHERE token_hash = $2 AND used_at IS NULL AND expires_at > $3
             RETURNING user_id",
        )
        .bind(now)
        .bind(&token_hash)
        .bind(now)
        .fetch_optional(&mut *tx)
        .await?;

        let Some((user_id,)) = claimed else {
            let row = sqlx::query_as::<_, (Option<DateTime<Utc>>, DateTime<Utc>)>(
                "SELECT used_at, expires_at FROM password_resets WHERE token_hash = $1",
            )
            .bind(&token_hash)
            .fetch_optional(&mut *tx)
            .await?;

            tx.rollback().await.ok();

            return match row {
                None => Err(ConfirmPasswordResetError::NotFound),
                Some((Some(_), _)) => Err(ConfirmPasswordResetError::AlreadyUsed),
                Some((None, _)) => Err(ConfirmPasswordResetError::Expired),
            };
        };

        sqlx::query("UPDATE users SET password_hash = $1 WHERE user_id = $2")
            .bind(&password_hash)
            .bind(user_id)
            .execute(&mut *tx)
            .await?;

        sqlx::query("DELETE FROM sessions WHERE user_id = $1")
            .bind(user_id)
            .execute(&mut *tx)
            .await?;

        tx.commit().await?;
        Ok(())
    }
}

// ---------------------------------------------------------------------------
// Posts
// ---------------------------------------------------------------------------

/// SQLite-backed [`PostStorage`].
pub struct SqlitePostStorage {
    pool: SqlitePool,
}

impl SqlitePostStorage {
    #[must_use]
    pub fn new(pool: SqlitePool) -> Self {
        Self { pool }
    }
}

#[async_trait]
impl PostStorage for SqlitePostStorage {
    async fn create_post(&self, input: &CreatePostInput) -> Result<i64, CreatePostError> {
        let now = Utc::now();

        let result = sqlx::query_scalar::<_, i64>(
            "INSERT INTO posts (user_id, title, slug, body, format, rendered_html, created_at, updated_at, published_at)
             VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9)
             RETURNING post_id",
        )
        .bind(input.user_id)
        .bind(&input.title)
        .bind(input.slug.as_str())
        .bind(&input.body)
        .bind(input.format.to_string())
        .bind(&input.rendered_html)
        .bind(now)
        .bind(now)
        .bind(input.published_at)
        .fetch_one(&self.pool)
        .await;

        match result {
            Ok(id) => Ok(id),
            Err(sqlx::Error::Database(e)) if e.is_unique_violation() => {
                Err(CreatePostError::SlugConflict)
            }
            Err(e) => Err(CreatePostError::Internal(e)),
        }
    }

    async fn get_post_by_id(&self, post_id: i64) -> sqlx::Result<Option<PostRecord>> {
        let row = sqlx::query_as::<_, PostRow>(
            "SELECT post_id, user_id, title, slug, body, format, rendered_html,
                    created_at, updated_at, published_at, deleted_at,
                    COALESCE((SELECT json_group_array(json_object('tag_id', t.tag_id, 'tag_slug', t.tag_slug, 'tag_display', pt.tag_display)) FROM post_tags pt JOIN tags t ON pt.tag_id = t.tag_id WHERE pt.post_id = posts.post_id), '[]') AS tags
             FROM posts WHERE post_id = $1",
        )
        .bind(post_id)
        .fetch_optional(&self.pool)
        .await?;
        Ok(row.map(post_record_from_row).transpose()?)
    }

    async fn get_post_by_permalink(
        &self,
        username: &Username,
        year: i32,
        month: u32,
        day: u32,
        slug: &Slug,
    ) -> sqlx::Result<Option<PostRecord>> {
        let date_str = format!("{year:04}-{month:02}-{day:02}");
        let row = sqlx::query_as::<_, PostRow>(
            "SELECT p.post_id, p.user_id, p.title, p.slug, p.body, p.format, p.rendered_html,
                    p.created_at, p.updated_at, p.published_at, p.deleted_at,
                    COALESCE((SELECT json_group_array(json_object('tag_id', t.tag_id, 'tag_slug', t.tag_slug, 'tag_display', pt.tag_display)) FROM post_tags pt JOIN tags t ON pt.tag_id = t.tag_id WHERE pt.post_id = p.post_id), '[]') AS tags
             FROM posts p
             JOIN users u ON p.user_id = u.user_id
             WHERE u.username = $1
               AND p.slug = $2
               AND p.published_at IS NOT NULL
               AND p.deleted_at IS NULL
               AND date(p.published_at) = $3",
        )
        .bind(username.as_str())
        .bind(slug.as_str())
        .bind(&date_str)
        .fetch_optional(&self.pool)
        .await?;
        Ok(row.map(post_record_from_row).transpose()?)
    }

    async fn update_post(
        &self,
        post_id: i64,
        editor_user_id: i64,
        input: &UpdatePostInput,
    ) -> Result<PostRecord, UpdatePostError> {
        let mut tx = self.pool.begin().await?;
        let now = Utc::now();

        // Read current ownership within the transaction to prevent races.
        let existing = sqlx::query_as::<_, (i64, Option<DateTime<Utc>>)>(
            "SELECT user_id, deleted_at FROM posts WHERE post_id = $1",
        )
        .bind(post_id)
        .fetch_optional(&mut *tx)
        .await?;

        match existing {
            None => {
                tx.rollback().await.ok();
                return Err(UpdatePostError::NotFound);
            }
            Some((owner_id, deleted_at)) if owner_id != editor_user_id || deleted_at.is_some() => {
                tx.rollback().await.ok();
                return Err(UpdatePostError::Unauthorized);
            }
            Some(_) => {}
        }

        // Save a revision of current state.
        sqlx::query(
            "INSERT INTO post_revisions (post_id, user_id, title, slug, body, format, rendered_html, edited_at)
             SELECT post_id, user_id, title, slug, body, format, rendered_html, $1
             FROM posts WHERE post_id = $2",
        )
        .bind(now)
        .bind(post_id)
        .execute(&mut *tx)
        .await?;

        // Update the post:
        //   - slug frozen once published (CASE WHEN published_at IS NULL)
        //   - publish=true  → COALESCE(published_at, now)  (preserves existing date)
        //   - publish=false → NULL                          (un-publish)
        let row = sqlx::query_as::<_, PostRow>(
            "UPDATE posts
             SET title = $1,
                 slug = CASE WHEN published_at IS NULL THEN $2 ELSE slug END,
                 body = $3,
                 format = $4,
                 rendered_html = $5,
                 published_at = CASE WHEN $6 THEN COALESCE(published_at, $7) ELSE NULL END,
                 updated_at = $8
             WHERE post_id = $9
             RETURNING post_id, user_id, title, slug, body, format, rendered_html,
                       created_at, updated_at, published_at, deleted_at,
                       COALESCE((SELECT json_group_array(json_object('tag_id', t.tag_id, 'tag_slug', t.tag_slug, 'tag_display', pt.tag_display)) FROM post_tags pt JOIN tags t ON pt.tag_id = t.tag_id WHERE pt.post_id = posts.post_id), '[]') AS tags",
        )
        .bind(&input.title)
        .bind(input.slug.as_str())
        .bind(&input.body)
        .bind(input.format.to_string())
        .bind(&input.rendered_html)
        .bind(input.publish)
        .bind(now)
        .bind(now)
        .bind(post_id)
        .fetch_one(&mut *tx)
        .await?;

        tx.commit().await?;
        post_record_from_row(row).map_err(UpdatePostError::Internal)
    }

    async fn soft_delete_post(&self, post_id: i64) -> sqlx::Result<()> {
        let now = Utc::now();
        sqlx::query("UPDATE posts SET deleted_at = $1 WHERE post_id = $2")
            .bind(now)
            .bind(post_id)
            .execute(&self.pool)
            .await?;
        Ok(())
    }

    async fn unpublish_post(&self, post_id: i64) -> sqlx::Result<()> {
        sqlx::query("UPDATE posts SET published_at = NULL WHERE post_id = ?")
            .bind(post_id)
            .execute(&self.pool)
            .await?;
        Ok(())
    }

    async fn list_published_by_user(
        &self,
        username: &Username,
        cursor: Option<&PostCursor>,
        limit: u32,
    ) -> sqlx::Result<Vec<PostRecord>> {
        let rows = if let Some(cursor) = cursor {
            sqlx::query_as::<_, PostRow>(
                "SELECT p.post_id, p.user_id, p.title, p.slug, p.body, p.format, p.rendered_html,
                        p.created_at, p.updated_at, p.published_at, p.deleted_at,
                        COALESCE((SELECT json_group_array(json_object('tag_id', t.tag_id, 'tag_slug', t.tag_slug, 'tag_display', pt.tag_display)) FROM post_tags pt JOIN tags t ON pt.tag_id = t.tag_id WHERE pt.post_id = p.post_id), '[]') AS tags
                 FROM posts p
                 JOIN users u ON p.user_id = u.user_id
                 WHERE u.username = $1
                   AND p.published_at IS NOT NULL
                   AND p.deleted_at IS NULL
                   AND (p.created_at < $2 OR (p.created_at = $3 AND p.post_id < $4))
                 ORDER BY p.created_at DESC, p.post_id DESC
                 LIMIT $5",
            )
            .bind(username.as_str())
            .bind(cursor.created_at)
            .bind(cursor.created_at)
            .bind(cursor.post_id)
            .bind(i64::from(limit))
            .fetch_all(&self.pool)
            .await?
        } else {
            sqlx::query_as::<_, PostRow>(
                "SELECT p.post_id, p.user_id, p.title, p.slug, p.body, p.format, p.rendered_html,
                        p.created_at, p.updated_at, p.published_at, p.deleted_at,
                        COALESCE((SELECT json_group_array(json_object('tag_id', t.tag_id, 'tag_slug', t.tag_slug, 'tag_display', pt.tag_display)) FROM post_tags pt JOIN tags t ON pt.tag_id = t.tag_id WHERE pt.post_id = p.post_id), '[]') AS tags
                 FROM posts p
                 JOIN users u ON p.user_id = u.user_id
                 WHERE u.username = $1
                   AND p.published_at IS NOT NULL
                   AND p.deleted_at IS NULL
                 ORDER BY p.created_at DESC, p.post_id DESC
                 LIMIT $2",
            )
            .bind(username.as_str())
            .bind(i64::from(limit))
            .fetch_all(&self.pool)
            .await?
        };
        rows.into_iter().map(post_record_from_row).collect()
    }

    async fn list_published(
        &self,
        cursor: Option<&PostCursor>,
        limit: u32,
    ) -> sqlx::Result<Vec<PostRecord>> {
        let rows = if let Some(cursor) = cursor {
            sqlx::query_as::<_, PostRow>(
                "SELECT post_id, user_id, title, slug, body, format, rendered_html,
                        created_at, updated_at, published_at, deleted_at,
                        COALESCE((SELECT json_group_array(json_object('tag_id', t.tag_id, 'tag_slug', t.tag_slug, 'tag_display', pt.tag_display)) FROM post_tags pt JOIN tags t ON pt.tag_id = t.tag_id WHERE pt.post_id = posts.post_id), '[]') AS tags
                 FROM posts
                 WHERE published_at IS NOT NULL
                   AND deleted_at IS NULL
                   AND (created_at < $1 OR (created_at = $2 AND post_id < $3))
                 ORDER BY created_at DESC, post_id DESC
                 LIMIT $4",
            )
            .bind(cursor.created_at)
            .bind(cursor.created_at)
            .bind(cursor.post_id)
            .bind(i64::from(limit))
            .fetch_all(&self.pool)
            .await?
        } else {
            sqlx::query_as::<_, PostRow>(
                "SELECT post_id, user_id, title, slug, body, format, rendered_html,
                        created_at, updated_at, published_at, deleted_at,
                        COALESCE((SELECT json_group_array(json_object('tag_id', t.tag_id, 'tag_slug', t.tag_slug, 'tag_display', pt.tag_display)) FROM post_tags pt JOIN tags t ON pt.tag_id = t.tag_id WHERE pt.post_id = posts.post_id), '[]') AS tags
                 FROM posts
                 WHERE published_at IS NOT NULL
                   AND deleted_at IS NULL
                 ORDER BY created_at DESC, post_id DESC
                 LIMIT $1",
            )
            .bind(i64::from(limit))
            .fetch_all(&self.pool)
            .await?
        };
        rows.into_iter().map(post_record_from_row).collect()
    }

    async fn list_drafts_by_user(
        &self,
        user_id: i64,
        cursor: Option<&PostCursor>,
        limit: u32,
    ) -> sqlx::Result<Vec<PostRecord>> {
        let rows = if let Some(cursor) = cursor {
            sqlx::query_as::<_, PostRow>(
                "SELECT post_id, user_id, title, slug, body, format, rendered_html,
                        created_at, updated_at, published_at, deleted_at,
                        COALESCE((SELECT json_group_array(json_object('tag_id', t.tag_id, 'tag_slug', t.tag_slug, 'tag_display', pt.tag_display)) FROM post_tags pt JOIN tags t ON pt.tag_id = t.tag_id WHERE pt.post_id = posts.post_id), '[]') AS tags
                 FROM posts
                 WHERE user_id = $1
                   AND published_at IS NULL
                   AND deleted_at IS NULL
                   AND (created_at < $2 OR (created_at = $3 AND post_id < $4))
                 ORDER BY created_at DESC, post_id DESC
                 LIMIT $5",
            )
            .bind(user_id)
            .bind(cursor.created_at)
            .bind(cursor.created_at)
            .bind(cursor.post_id)
            .bind(i64::from(limit))
            .fetch_all(&self.pool)
            .await?
        } else {
            sqlx::query_as::<_, PostRow>(
                "SELECT post_id, user_id, title, slug, body, format, rendered_html,
                        created_at, updated_at, published_at, deleted_at,
                        COALESCE((SELECT json_group_array(json_object('tag_id', t.tag_id, 'tag_slug', t.tag_slug, 'tag_display', pt.tag_display)) FROM post_tags pt JOIN tags t ON pt.tag_id = t.tag_id WHERE pt.post_id = posts.post_id), '[]') AS tags
                 FROM posts
                 WHERE user_id = $1
                   AND published_at IS NULL
                   AND deleted_at IS NULL
                 ORDER BY created_at DESC, post_id DESC
                 LIMIT $2",
            )
            .bind(user_id)
            .bind(i64::from(limit))
            .fetch_all(&self.pool)
            .await?
        };
        rows.into_iter().map(post_record_from_row).collect()
    }

    async fn tag_post(&self, post_id: i64, tag_display: &str) -> Result<(), TaggingError> {
        // Parse and normalize tag
        let tag: Tag = tag_display.parse().map_err(|_| {
            TaggingError::Internal(sqlx::Error::Decode("invalid tag format".into()))
        })?;

        let mut tx = self.pool.begin().await?;

        // Check if post exists
        let post_exists: bool =
            sqlx::query_scalar("SELECT COUNT(*) > 0 FROM posts WHERE post_id = $1")
                .bind(post_id)
                .fetch_one(&mut *tx)
                .await?;

        if !post_exists {
            tx.rollback().await.ok();
            return Err(TaggingError::PostNotFound);
        }

        // Insert or ignore tag (if it already exists)
        sqlx::query("INSERT OR IGNORE INTO tags (tag_slug) VALUES ($1)")
            .bind(tag.as_str())
            .execute(&mut *tx)
            .await?;

        // Get the tag_id (either from this insert or from existing)
        let tag_id: i64 =
            sqlx::query_scalar::<_, i64>("SELECT tag_id FROM tags WHERE tag_slug = $1")
                .bind(tag.as_str())
                .fetch_one(&mut *tx)
                .await?;

        // Insert post_tags link
        let result =
            sqlx::query("INSERT INTO post_tags (post_id, tag_id, tag_display) VALUES ($1, $2, $3)")
                .bind(post_id)
                .bind(tag_id)
                .bind(tag_display)
                .execute(&mut *tx)
                .await;

        match result {
            Ok(_) => {
                tx.commit().await?;
                Ok(())
            }
            Err(sqlx::Error::Database(e)) if e.is_unique_violation() => {
                tx.rollback().await.ok();
                Err(TaggingError::AlreadyTagged)
            }
            Err(e) => {
                tx.rollback().await.ok();
                Err(TaggingError::Internal(e))
            }
        }
    }

    async fn untag_post(&self, post_id: i64, tag_slug: &Tag) -> Result<(), TaggingError> {
        let rows_deleted = sqlx::query(
            "DELETE FROM post_tags
             WHERE post_id = $1 AND tag_id = (SELECT tag_id FROM tags WHERE tag_slug = $2)",
        )
        .bind(post_id)
        .bind(tag_slug.as_str())
        .execute(&self.pool)
        .await?
        .rows_affected();

        if rows_deleted == 0 {
            Err(TaggingError::TagNotFound)
        } else {
            Ok(())
        }
    }

    async fn get_tags_for_post(&self, post_id: i64) -> sqlx::Result<Vec<PostTag>> {
        let rows = sqlx::query(
            "SELECT pt.post_id, pt.tag_id, t.tag_slug, pt.tag_display
             FROM post_tags pt
             JOIN tags t ON pt.tag_id = t.tag_id
             WHERE pt.post_id = $1
             ORDER BY t.tag_slug",
        )
        .bind(post_id)
        .fetch_all(&self.pool)
        .await?;

        rows.into_iter()
            .map(|row| {
                let tag_slug_str: String = row.get("tag_slug");
                let tag_slug: Tag = tag_slug_str
                    .parse()
                    .map_err(|_| sqlx::Error::Decode("invalid tag format".into()))?;
                Ok(PostTag {
                    post_id: row.get("post_id"),
                    tag_id: row.get("tag_id"),
                    tag_slug,
                    tag_display: row.get("tag_display"),
                })
            })
            .collect()
    }

    async fn list_posts_by_tag(
        &self,
        tag_slug: &Tag,
        cursor: Option<&PostCursor>,
        limit: u32,
    ) -> Result<Vec<PostRecord>, ListByTagError> {
        // Check tag exists
        let tag_exists: bool =
            sqlx::query_scalar("SELECT COUNT(*) > 0 FROM tags WHERE tag_slug = $1")
                .bind(tag_slug.as_str())
                .fetch_one(&self.pool)
                .await?;

        if !tag_exists {
            return Err(ListByTagError::TagNotFound);
        }

        let rows = if let Some(cursor) = cursor {
            sqlx::query_as::<_, PostRow>(
                "SELECT p.post_id, p.user_id, p.title, p.slug, p.body, p.format, p.rendered_html,
                        p.created_at, p.updated_at, p.published_at, p.deleted_at,
                        COALESCE((SELECT json_group_array(json_object('tag_id', t.tag_id, 'tag_slug', t.tag_slug, 'tag_display', pt.tag_display)) FROM post_tags pt JOIN tags t ON pt.tag_id = t.tag_id WHERE pt.post_id = p.post_id), '[]') AS tags
                 FROM posts p
                 JOIN post_tags pt ON p.post_id = pt.post_id
                 JOIN tags t ON pt.tag_id = t.tag_id
                 WHERE t.tag_slug = $1
                   AND p.published_at IS NOT NULL
                   AND p.deleted_at IS NULL
                   AND (p.created_at < $2 OR (p.created_at = $3 AND p.post_id < $4))
                 ORDER BY p.created_at DESC, p.post_id DESC
                 LIMIT $5",
            )
            .bind(tag_slug.as_str())
            .bind(cursor.created_at)
            .bind(cursor.created_at)
            .bind(cursor.post_id)
            .bind(i64::from(limit))
            .fetch_all(&self.pool)
            .await?
        } else {
            sqlx::query_as::<_, PostRow>(
                "SELECT p.post_id, p.user_id, p.title, p.slug, p.body, p.format, p.rendered_html,
                        p.created_at, p.updated_at, p.published_at, p.deleted_at,
                        COALESCE((SELECT json_group_array(json_object('tag_id', t.tag_id, 'tag_slug', t.tag_slug, 'tag_display', pt.tag_display)) FROM post_tags pt JOIN tags t ON pt.tag_id = t.tag_id WHERE pt.post_id = p.post_id), '[]') AS tags
                 FROM posts p
                 JOIN post_tags pt ON p.post_id = pt.post_id
                 JOIN tags t ON pt.tag_id = t.tag_id
                 WHERE t.tag_slug = $1
                   AND p.published_at IS NOT NULL
                   AND p.deleted_at IS NULL
                 ORDER BY p.created_at DESC, p.post_id DESC
                 LIMIT $2",
            )
            .bind(tag_slug.as_str())
            .bind(i64::from(limit))
            .fetch_all(&self.pool)
            .await?
        };

        rows.into_iter()
            .map(post_record_from_row)
            .collect::<sqlx::Result<_>>()
            .map_err(ListByTagError::Internal)
    }

    async fn list_user_posts_by_tag(
        &self,
        user_id: i64,
        tag_slug: &Tag,
        cursor: Option<&PostCursor>,
        limit: u32,
    ) -> Result<Vec<PostRecord>, ListByTagError> {
        // Check tag exists
        let tag_exists: bool =
            sqlx::query_scalar("SELECT COUNT(*) > 0 FROM tags WHERE tag_slug = $1")
                .bind(tag_slug.as_str())
                .fetch_one(&self.pool)
                .await?;

        if !tag_exists {
            return Err(ListByTagError::TagNotFound);
        }

        let rows = if let Some(cursor) = cursor {
            sqlx::query_as::<_, PostRow>(
                "SELECT p.post_id, p.user_id, p.title, p.slug, p.body, p.format, p.rendered_html,
                        p.created_at, p.updated_at, p.published_at, p.deleted_at,
                        COALESCE((SELECT json_group_array(json_object('tag_id', t.tag_id, 'tag_slug', t.tag_slug, 'tag_display', pt.tag_display)) FROM post_tags pt JOIN tags t ON pt.tag_id = t.tag_id WHERE pt.post_id = p.post_id), '[]') AS tags
                 FROM posts p
                 JOIN post_tags pt ON p.post_id = pt.post_id
                 JOIN tags t ON pt.tag_id = t.tag_id
                 WHERE p.user_id = $1
                   AND t.tag_slug = $2
                   AND p.published_at IS NOT NULL
                   AND p.deleted_at IS NULL
                   AND (p.created_at < $3 OR (p.created_at = $4 AND p.post_id < $5))
                 ORDER BY p.created_at DESC, p.post_id DESC
                 LIMIT $6",
            )
            .bind(user_id)
            .bind(tag_slug.as_str())
            .bind(cursor.created_at)
            .bind(cursor.created_at)
            .bind(cursor.post_id)
            .bind(i64::from(limit))
            .fetch_all(&self.pool)
            .await?
        } else {
            sqlx::query_as::<_, PostRow>(
                "SELECT p.post_id, p.user_id, p.title, p.slug, p.body, p.format, p.rendered_html,
                        p.created_at, p.updated_at, p.published_at, p.deleted_at,
                        COALESCE((SELECT json_group_array(json_object('tag_id', t.tag_id, 'tag_slug', t.tag_slug, 'tag_display', pt.tag_display)) FROM post_tags pt JOIN tags t ON pt.tag_id = t.tag_id WHERE pt.post_id = p.post_id), '[]') AS tags
                 FROM posts p
                 JOIN post_tags pt ON p.post_id = pt.post_id
                 JOIN tags t ON pt.tag_id = t.tag_id
                 WHERE p.user_id = $1
                   AND t.tag_slug = $2
                   AND p.published_at IS NOT NULL
                   AND p.deleted_at IS NULL
                 ORDER BY p.created_at DESC, p.post_id DESC
                 LIMIT $3",
            )
            .bind(user_id)
            .bind(tag_slug.as_str())
            .bind(i64::from(limit))
            .fetch_all(&self.pool)
            .await?
        };

        rows.into_iter()
            .map(post_record_from_row)
            .collect::<sqlx::Result<_>>()
            .map_err(ListByTagError::Internal)
    }

    async fn list_tags(&self, prefix: Option<&str>, limit: u32) -> sqlx::Result<Vec<TagRecord>> {
        let normalized = prefix
            .map(str::trim)
            .filter(|p| !p.is_empty())
            .map(str::to_ascii_lowercase);
        let pattern = normalized.as_deref().map(|p| format!("{p}%"));
        let limit_i64 = i64::from(limit);

        let rows = match pattern {
            Some(ref like) => {
                sqlx::query(
                    "SELECT tag_id, tag_slug FROM tags
                     WHERE tag_slug LIKE $1
                     ORDER BY tag_slug
                     LIMIT $2",
                )
                .bind(like)
                .bind(limit_i64)
                .fetch_all(&self.pool)
                .await?
            }
            None => {
                sqlx::query(
                    "SELECT tag_id, tag_slug FROM tags
                     ORDER BY tag_slug
                     LIMIT $1",
                )
                .bind(limit_i64)
                .fetch_all(&self.pool)
                .await?
            }
        };

        rows.into_iter()
            .map(|row| {
                let tag_slug_str: String = row.get("tag_slug");
                let tag_slug: Tag = tag_slug_str
                    .parse()
                    .map_err(|_| sqlx::Error::Decode("invalid tag format".into()))?;
                Ok(TagRecord {
                    tag_id: row.get("tag_id"),
                    tag_slug,
                })
            })
            .collect()
    }
}

// ---------------------------------------------------------------------------
// Media
// ---------------------------------------------------------------------------

/// SQLite-backed [`MediaStorage`].
pub struct SqliteMediaStorage {
    pool: SqlitePool,
}

impl SqliteMediaStorage {
    #[must_use]
    pub fn new(pool: SqlitePool) -> Self {
        Self { pool }
    }
}

#[async_trait]
impl MediaStorage for SqliteMediaStorage {
    #[tracing::instrument(name = "storage.sqlite.media.create", skip(self, record))]
    async fn create_media(&self, record: &MediaRecord) -> Result<(), CreateMediaError> {
        let result = sqlx::query(
            "INSERT INTO media (user_id, sha256, filename, source, content_type, size_bytes, source_url, created_at)
             VALUES ($1, $2, $3, $4, $5, $6, $7, $8)",
        )
        .bind(record.user_id)
        .bind(&record.sha256)
        .bind(&record.filename)
        .bind(record.source.as_str())
        .bind(&record.content_type)
        .bind(record.size_bytes)
        .bind(&record.source_url)
        .bind(record.created_at)
        .execute(&self.pool)
        .await;

        match result {
            Ok(_) => Ok(()),
            Err(e)
                if e.as_database_error()
                    .is_some_and(sqlx::error::DatabaseError::is_unique_violation) =>
            {
                Err(CreateMediaError::AlreadyExists)
            }
            Err(e) => Err(CreateMediaError::Internal(e)),
        }
    }

    #[tracing::instrument(name = "storage.sqlite.media.get", skip(self))]
    async fn get_media(
        &self,
        user_id: i64,
        sha256: &str,
        filename: &str,
        source: &MediaSource,
    ) -> sqlx::Result<Option<MediaRecord>> {
        let row = sqlx::query_as::<_, MediaRow>(
            "SELECT user_id, sha256, filename, source, content_type, size_bytes, source_url, created_at
             FROM media
             WHERE user_id = $1 AND sha256 = $2 AND filename = $3 AND source = $4",
        )
        .bind(user_id)
        .bind(sha256)
        .bind(filename)
        .bind(source.as_str())
        .fetch_optional(&self.pool)
        .await?;

        row.map(media_record_from_row).transpose()
    }

    #[tracing::instrument(name = "storage.sqlite.media.list", skip(self))]
    async fn list_media(
        &self,
        user_id: i64,
        source: Option<&MediaSource>,
        limit: u32,
        offset: u32,
    ) -> sqlx::Result<Vec<MediaRecord>> {
        let rows = if let Some(src) = source {
            sqlx::query_as::<_, MediaRow>(
                "SELECT user_id, sha256, filename, source, content_type, size_bytes, source_url, created_at
                 FROM media
                 WHERE user_id = $1 AND source = $2
                 ORDER BY created_at DESC
                 LIMIT $3 OFFSET $4",
            )
            .bind(user_id)
            .bind(src.as_str())
            .bind(i64::from(limit))
            .bind(i64::from(offset))
            .fetch_all(&self.pool)
            .await?
        } else {
            sqlx::query_as::<_, MediaRow>(
                "SELECT user_id, sha256, filename, source, content_type, size_bytes, source_url, created_at
                 FROM media
                 WHERE user_id = $1
                 ORDER BY created_at DESC
                 LIMIT $2 OFFSET $3",
            )
            .bind(user_id)
            .bind(i64::from(limit))
            .bind(i64::from(offset))
            .fetch_all(&self.pool)
            .await?
        };

        rows.into_iter().map(media_record_from_row).collect()
    }

    #[tracing::instrument(name = "storage.sqlite.media.delete", skip(self))]
    async fn delete_media(
        &self,
        user_id: i64,
        sha256: &str,
        filename: &str,
        source: &MediaSource,
    ) -> Result<(), DeleteMediaError> {
        let result = sqlx::query(
            "DELETE FROM media WHERE user_id = $1 AND sha256 = $2 AND filename = $3 AND source = $4",
        )
        .bind(user_id)
        .bind(sha256)
        .bind(filename)
        .bind(source.as_str())
        .execute(&self.pool)
        .await?;

        if result.rows_affected() == 0 {
            return Err(DeleteMediaError::NotFound);
        }
        Ok(())
    }

    #[tracing::instrument(name = "storage.sqlite.media.upload_usage", skip(self))]
    async fn get_user_upload_usage(&self, user_id: i64) -> sqlx::Result<i64> {
        let row = sqlx::query_as::<_, (i64,)>(
            "SELECT COALESCE(SUM(size_bytes), 0) FROM media WHERE user_id = $1 AND source = 'upload'",
        )
        .bind(user_id)
        .fetch_one(&self.pool)
        .await?;

        Ok(row.0)
    }

    #[tracing::instrument(name = "storage.sqlite.media.find_by_hash", skip(self))]
    async fn find_by_hash(
        &self,
        sha256: &str,
        source: &MediaSource,
    ) -> sqlx::Result<Option<MediaRecord>> {
        let row = sqlx::query_as::<_, MediaRow>(
            "SELECT user_id, sha256, filename, source, content_type, size_bytes, source_url, created_at
             FROM media
             WHERE sha256 = $1 AND source = $2
             LIMIT 1",
        )
        .bind(sha256)
        .bind(source.as_str())
        .fetch_optional(&self.pool)
        .await?;

        row.map(media_record_from_row).transpose()
    }
}

// ---------------------------------------------------------------------------
// UserConfig
// ---------------------------------------------------------------------------

/// SQLite-backed [`UserConfigStorage`].
pub struct SqliteUserConfigStorage {
    pool: SqlitePool,
}

impl SqliteUserConfigStorage {
    #[must_use]
    pub fn new(pool: SqlitePool) -> Self {
        Self { pool }
    }
}

#[async_trait]
impl UserConfigStorage for SqliteUserConfigStorage {
    #[tracing::instrument(name = "storage.sqlite.user_config.get", skip(self))]
    async fn get(&self, user_id: i64, key: &str) -> sqlx::Result<Option<String>> {
        let row = sqlx::query_as::<_, (String,)>(
            "SELECT value FROM user_config WHERE user_id = $1 AND key = $2",
        )
        .bind(user_id)
        .bind(key)
        .fetch_optional(&self.pool)
        .await?;

        Ok(row.map(|(v,)| v))
    }

    #[tracing::instrument(name = "storage.sqlite.user_config.set", skip(self))]
    async fn set(&self, user_id: i64, key: &str, value: &str) -> sqlx::Result<()> {
        sqlx::query(
            "INSERT INTO user_config (user_id, key, value) VALUES ($1, $2, $3)
             ON CONFLICT (user_id, key) DO UPDATE SET value = excluded.value",
        )
        .bind(user_id)
        .bind(key)
        .bind(value)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    #[tracing::instrument(name = "storage.sqlite.user_config.delete", skip(self))]
    async fn delete(&self, user_id: i64, key: &str) -> sqlx::Result<()> {
        sqlx::query("DELETE FROM user_config WHERE user_id = $1 AND key = $2")
            .bind(user_id)
            .bind(key)
            .execute(&self.pool)
            .await?;
        Ok(())
    }
}
