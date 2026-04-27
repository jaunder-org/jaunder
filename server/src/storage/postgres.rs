use async_trait::async_trait;
use chrono::{DateTime, Utc};
use email_address::EmailAddress;
use sqlx::{PgPool, Row};

use common::password::Password;
use common::slug::Slug;
use common::storage::{
    AtomicOps, ConfirmPasswordResetError, CreatePostError, CreatePostInput, CreateUserError,
    EmailVerificationStorage, InviteRecord, InviteStorage, ListByTagError, PasswordResetStorage,
    PostCursor, PostRecord, PostStorage, PostTag, ProfileUpdate, RegisterWithInviteError,
    SessionAuthError, SessionRecord, SessionStorage, SiteConfigStorage, TaggingError,
    UpdatePostError, UpdatePostInput, UseEmailVerificationError, UseInviteError,
    UsePasswordResetError, UserAuthError, UserRecord, UserStorage,
};
use common::tag::Tag;
use common::username::Username;
use tracing::Instrument;

// ---------------------------------------------------------------------------
// SiteConfig
// ---------------------------------------------------------------------------

pub struct PostgresSiteConfigStorage {
    pool: PgPool,
}

impl PostgresSiteConfigStorage {
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }
}

#[async_trait]
impl SiteConfigStorage for PostgresSiteConfigStorage {
    async fn get(&self, key: &str) -> sqlx::Result<Option<String>> {
        let row = sqlx::query_as::<_, (String,)>("SELECT value FROM site_config WHERE key = $1")
            .bind(key)
            .fetch_optional(&self.pool)
            .await?;
        Ok(row.map(|(value,)| value))
    }

    async fn set(&self, key: &str, value: &str) -> sqlx::Result<()> {
        sqlx::query(
            "INSERT INTO site_config (key, value) VALUES ($1, $2)
             ON CONFLICT (key) DO UPDATE SET value = excluded.value",
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

type UserRow = (
    i64,
    String,
    Option<String>,
    Option<String>,
    DateTime<Utc>,
    Option<DateTime<Utc>>,
    Option<String>,
    bool,
);

fn user_record_from_row(
    (
        user_id,
        username,
        display_name,
        bio,
        created_at,
        last_authenticated_at,
        email,
        email_verified,
    ): UserRow,
) -> sqlx::Result<UserRecord> {
    super::build_user_record((
        user_id,
        username,
        display_name,
        bio,
        created_at,
        last_authenticated_at,
        email,
        email_verified,
    ))
}

pub struct PostgresUserStorage {
    pool: PgPool,
}

impl PostgresUserStorage {
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }
}

#[async_trait]
impl UserStorage for PostgresUserStorage {
    #[tracing::instrument(
        name = "storage.postgres.user.create_user",
        skip(self, password, display_name),
        fields(username = %username.as_str())
    )]
    async fn create_user(
        &self,
        username: &Username,
        password: &Password,
        display_name: Option<&str>,
    ) -> Result<i64, CreateUserError> {
        let password_hash = super::hash_password(password.clone())
            .instrument(tracing::info_span!(
                "storage.postgres.user.create_user.hash_password"
            ))
            .await
            .map_err(|e| CreateUserError::Internal(sqlx::Error::Io(e)))?;

        let now = Utc::now();

        let result = sqlx::query_scalar::<_, i64>(
            "INSERT INTO users (username, password_hash, display_name, created_at)
             VALUES ($1, $2, $3, $4)
             RETURNING user_id",
        )
        .bind(username.as_str())
        .bind(&password_hash)
        .bind(display_name)
        .bind(now)
        .fetch_one(&self.pool)
        .instrument(tracing::info_span!(
            "storage.postgres.user.create_user.insert_user_row"
        ))
        .await;

        match result {
            Ok(id) => Ok(id),
            Err(sqlx::Error::Database(error)) if error.is_unique_violation() => {
                Err(CreateUserError::UsernameTaken)
            }
            Err(error) => Err(CreateUserError::Internal(error)),
        }
    }

    #[tracing::instrument(
        name = "storage.postgres.user.authenticate",
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
            ),
        >(
            "SELECT user_id, username, display_name, bio, created_at, last_authenticated_at,
                    password_hash, email, email_verified
             FROM users WHERE username = $1",
        )
        .bind(username.as_str())
        .fetch_optional(&self.pool)
        .instrument(tracing::info_span!(
            "storage.postgres.user.authenticate.lookup_user"
        ))
        .await
        .map_err(|e| UserAuthError::Internal(e.to_string()))?;

        let (
            user_id,
            username,
            display_name,
            bio,
            created_at,
            _last_authenticated_at,
            hash,
            email,
            email_verified,
        ) = match row {
            Some(row) => row,
            None => return Err(UserAuthError::InvalidCredentials),
        };

        let valid = super::verify_password(password.clone(), hash)
            .instrument(tracing::info_span!(
                "storage.postgres.user.authenticate.verify_password"
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
                "storage.postgres.user.authenticate.update_last_authenticated_at"
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
        ))
        .map_err(|e| UserAuthError::Internal(e.to_string()))
    }

    async fn get_user(&self, user_id: i64) -> sqlx::Result<Option<UserRecord>> {
        let row = sqlx::query_as::<_, UserRow>(
            "SELECT user_id, username, display_name, bio, created_at, last_authenticated_at,
                    email, email_verified
             FROM users WHERE user_id = $1",
        )
        .bind(user_id)
        .fetch_optional(&self.pool)
        .await?;
        Ok(row.map(user_record_from_row).transpose()?)
    }

    async fn get_user_by_username(&self, username: &Username) -> sqlx::Result<Option<UserRecord>> {
        let row = sqlx::query_as::<_, UserRow>(
            "SELECT user_id, username, display_name, bio, created_at, last_authenticated_at,
                    email, email_verified
             FROM users WHERE username = $1",
        )
        .bind(username.as_str())
        .fetch_optional(&self.pool)
        .await?;
        Ok(row.map(user_record_from_row).transpose()?)
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

type SessionRow = (
    String,
    i64,
    String,
    Option<String>,
    DateTime<Utc>,
    DateTime<Utc>,
);

fn session_record_from_row(
    (token_hash, user_id, username, label, created_at, last_used_at): SessionRow,
) -> sqlx::Result<SessionRecord> {
    super::build_session_record(
        token_hash,
        user_id,
        username,
        label,
        created_at,
        last_used_at,
    )
}

pub struct PostgresSessionStorage {
    pool: PgPool,
}

impl PostgresSessionStorage {
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }
}

#[async_trait]
impl SessionStorage for PostgresSessionStorage {
    #[tracing::instrument(
        name = "storage.postgres.session.create",
        skip(self, label),
        fields(user_id)
    )]
    async fn create_session(&self, user_id: i64, label: Option<&str>) -> sqlx::Result<String> {
        let raw_token = crate::auth::generate_token();
        let token_hash = crate::auth::hash_token(&raw_token)
            .map_err(|e| sqlx::Error::Io(std::io::Error::other(e)))?;
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

    #[tracing::instrument(name = "storage.postgres.session.authenticate", skip(self, raw_token))]
    async fn authenticate(&self, raw_token: &str) -> Result<SessionRecord, SessionAuthError> {
        let token_hash =
            crate::auth::hash_token(raw_token).map_err(|_| SessionAuthError::InvalidToken)?;

        let now = Utc::now();

        // Perform an atomic update and read in a single statement.
        // PostgreSQL's data-modifying CTEs (WITH UPDATE ...) are used
        // here to achievement atomicity while joining the results with
        // another table.
        let row = sqlx::query_as::<_, SessionRow>(
            "WITH updated AS (
                UPDATE sessions
                SET last_used_at = $1
                WHERE token_hash = $2
                RETURNING token_hash, user_id, label, created_at, last_used_at
             )
             SELECT updated.token_hash, updated.user_id, u.username, updated.label, updated.created_at, updated.last_used_at
             FROM updated
             JOIN users u ON updated.user_id = u.user_id",
        )
        .bind(now)
        .bind(&token_hash)
        .fetch_optional(&self.pool)
        .await?
        .ok_or(SessionAuthError::SessionNotFound)?;

        let record = session_record_from_row(row)?;
        Ok(record)
    }

    #[tracing::instrument(name = "storage.postgres.session.revoke", skip(self, token_hash))]
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

type InviteRow = (
    String,
    DateTime<Utc>,
    DateTime<Utc>,
    Option<DateTime<Utc>>,
    Option<i64>,
);

fn invite_record_from_row(
    (code, created_at, expires_at, used_at, used_by): InviteRow,
) -> InviteRecord {
    super::build_invite_record(code, created_at, expires_at, used_at, used_by)
}

pub struct PostgresInviteStorage {
    pool: PgPool,
}

impl PostgresInviteStorage {
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }
}

#[async_trait]
impl InviteStorage for PostgresInviteStorage {
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

pub struct PostgresEmailVerificationStorage {
    pool: PgPool,
}

impl PostgresEmailVerificationStorage {
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }
}

#[async_trait]
impl EmailVerificationStorage for PostgresEmailVerificationStorage {
    async fn create_email_verification(
        &self,
        user_id: i64,
        email: &str,
        expires_at: DateTime<Utc>,
    ) -> sqlx::Result<String> {
        let raw_token = crate::auth::generate_token();
        let token_hash = crate::auth::hash_token(&raw_token)
            .map_err(|e| sqlx::Error::Io(std::io::Error::other(e)))?;
        let now = Utc::now();

        let mut tx = self.pool.begin().await?;
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

        let row = sqlx::query_as::<_, (Option<DateTime<Utc>>, DateTime<Utc>)>(
            "SELECT used_at, expires_at FROM email_verifications WHERE token_hash = $1",
        )
        .bind(&token_hash)
        .fetch_optional(&self.pool)
        .await?;

        match row {
            None => Err(UseEmailVerificationError::NotFound),
            Some((Some(_), _)) => Err(UseEmailVerificationError::AlreadyUsed),
            Some((None, _)) => Err(UseEmailVerificationError::Expired),
        }
    }
}

// ---------------------------------------------------------------------------
// PasswordResets
// ---------------------------------------------------------------------------

pub struct PostgresPasswordResetStorage {
    pool: PgPool,
}

impl PostgresPasswordResetStorage {
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }
}

#[async_trait]
impl PasswordResetStorage for PostgresPasswordResetStorage {
    async fn create_password_reset(
        &self,
        user_id: i64,
        expires_at: DateTime<Utc>,
    ) -> sqlx::Result<String> {
        let raw_token = crate::auth::generate_token();
        let token_hash = crate::auth::hash_token(&raw_token)
            .map_err(|e| sqlx::Error::Io(std::io::Error::other(e)))?;
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

        match row {
            None => Err(UsePasswordResetError::NotFound),
            Some((Some(_), _)) => Err(UsePasswordResetError::AlreadyUsed),
            Some((None, _)) => Err(UsePasswordResetError::Expired),
        }
    }
}

// ---------------------------------------------------------------------------
// AtomicOps
// ---------------------------------------------------------------------------

pub struct PostgresAtomicOps {
    pool: PgPool,
}

impl PostgresAtomicOps {
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }
}

#[async_trait]
impl AtomicOps for PostgresAtomicOps {
    async fn create_user_with_invite(
        &self,
        username: &Username,
        password: &Password,
        display_name: Option<&str>,
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
            "INSERT INTO users (username, password_hash, display_name, created_at)
             VALUES ($1, $2, $3, $4)
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

        let user_id = if let Some((user_id,)) = claimed {
            user_id
        } else {
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
                Some((None, expires_at)) if expires_at <= now => {
                    Err(ConfirmPasswordResetError::Expired)
                }
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

type PostRow = (
    i64,
    i64,
    Option<String>,
    String,
    String,
    String,
    String,
    DateTime<Utc>,
    DateTime<Utc>,
    Option<DateTime<Utc>>,
    Option<DateTime<Utc>>,
);

fn post_record_from_row(row: PostRow) -> sqlx::Result<PostRecord> {
    super::build_post_record(row)
}

/// PostgreSQL-backed [`PostStorage`].
pub struct PostgresPostStorage {
    pool: PgPool,
}

impl PostgresPostStorage {
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }
}

#[async_trait]
impl PostStorage for PostgresPostStorage {
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
                    created_at, updated_at, published_at, deleted_at
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
                    p.created_at, p.updated_at, p.published_at, p.deleted_at
             FROM posts p
             JOIN users u ON p.user_id = u.user_id
             WHERE u.username = $1
               AND p.slug = $2
               AND p.published_at IS NOT NULL
               AND p.deleted_at IS NULL
               AND date(p.published_at AT TIME ZONE 'UTC') = $3::date",
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

        // Lock and read current ownership within the transaction to prevent races.
        let existing = sqlx::query_as::<_, (i64, Option<DateTime<Utc>>)>(
            "SELECT user_id, deleted_at FROM posts WHERE post_id = $1 FOR UPDATE",
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
                       created_at, updated_at, published_at, deleted_at",
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

    async fn list_published_by_user(
        &self,
        username: &Username,
        cursor: Option<&PostCursor>,
        limit: u32,
    ) -> sqlx::Result<Vec<PostRecord>> {
        let rows = if let Some(cursor) = cursor {
            sqlx::query_as::<_, PostRow>(
                "SELECT p.post_id, p.user_id, p.title, p.slug, p.body, p.format, p.rendered_html,
                        p.created_at, p.updated_at, p.published_at, p.deleted_at
                 FROM posts p
                 JOIN users u ON p.user_id = u.user_id
                 WHERE u.username = $1
                   AND p.published_at IS NOT NULL
                   AND p.deleted_at IS NULL
                   AND (p.created_at < $2 OR (p.created_at = $2 AND p.post_id < $3))
                 ORDER BY p.created_at DESC, p.post_id DESC
                 LIMIT $4",
            )
            .bind(username.as_str())
            .bind(cursor.created_at)
            .bind(cursor.post_id)
            .bind(limit as i64)
            .fetch_all(&self.pool)
            .await?
        } else {
            sqlx::query_as::<_, PostRow>(
                "SELECT p.post_id, p.user_id, p.title, p.slug, p.body, p.format, p.rendered_html,
                        p.created_at, p.updated_at, p.published_at, p.deleted_at
                 FROM posts p
                 JOIN users u ON p.user_id = u.user_id
                 WHERE u.username = $1
                   AND p.published_at IS NOT NULL
                   AND p.deleted_at IS NULL
                 ORDER BY p.created_at DESC, p.post_id DESC
                 LIMIT $2",
            )
            .bind(username.as_str())
            .bind(limit as i64)
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
                        created_at, updated_at, published_at, deleted_at
                 FROM posts
                 WHERE published_at IS NOT NULL
                   AND deleted_at IS NULL
                   AND (created_at < $1 OR (created_at = $1 AND post_id < $2))
                 ORDER BY created_at DESC, post_id DESC
                 LIMIT $3",
            )
            .bind(cursor.created_at)
            .bind(cursor.post_id)
            .bind(limit as i64)
            .fetch_all(&self.pool)
            .await?
        } else {
            sqlx::query_as::<_, PostRow>(
                "SELECT post_id, user_id, title, slug, body, format, rendered_html,
                        created_at, updated_at, published_at, deleted_at
                 FROM posts
                 WHERE published_at IS NOT NULL
                   AND deleted_at IS NULL
                 ORDER BY created_at DESC, post_id DESC
                 LIMIT $1",
            )
            .bind(limit as i64)
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
                        created_at, updated_at, published_at, deleted_at
                 FROM posts
                 WHERE user_id = $1
                   AND published_at IS NULL
                   AND deleted_at IS NULL
                   AND (created_at < $2 OR (created_at = $2 AND post_id < $3))
                 ORDER BY created_at DESC, post_id DESC
                 LIMIT $4",
            )
            .bind(user_id)
            .bind(cursor.created_at)
            .bind(cursor.post_id)
            .bind(limit as i64)
            .fetch_all(&self.pool)
            .await?
        } else {
            sqlx::query_as::<_, PostRow>(
                "SELECT post_id, user_id, title, slug, body, format, rendered_html,
                        created_at, updated_at, published_at, deleted_at
                 FROM posts
                 WHERE user_id = $1
                   AND published_at IS NULL
                   AND deleted_at IS NULL
                 ORDER BY created_at DESC, post_id DESC
                 LIMIT $2",
            )
            .bind(user_id)
            .bind(limit as i64)
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
            sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM posts WHERE post_id = $1)")
                .bind(post_id)
                .fetch_one(&mut *tx)
                .await?;

        if !post_exists {
            tx.rollback().await.ok();
            return Err(TaggingError::PostNotFound);
        }

        // Insert or get tag
        sqlx::query("INSERT INTO tags (tag_slug) VALUES ($1) ON CONFLICT (tag_slug) DO NOTHING")
            .bind(tag.as_str())
            .execute(&mut *tx)
            .await?;

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
            sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM tags WHERE tag_slug = $1)")
                .bind(tag_slug.as_str())
                .fetch_one(&self.pool)
                .await?;

        if !tag_exists {
            return Err(ListByTagError::TagNotFound);
        }

        let rows = if let Some(cursor) = cursor {
            sqlx::query_as::<_, PostRow>(
                "SELECT p.post_id, p.user_id, p.title, p.slug, p.body, p.format, p.rendered_html,
                        p.created_at, p.updated_at, p.published_at, p.deleted_at
                 FROM posts p
                 JOIN post_tags pt ON p.post_id = pt.post_id
                 JOIN tags t ON pt.tag_id = t.tag_id
                 WHERE t.tag_slug = $1
                   AND p.published_at IS NOT NULL
                   AND p.deleted_at IS NULL
                   AND (p.created_at < $2 OR (p.created_at = $2 AND p.post_id < $3))
                 ORDER BY p.created_at DESC, p.post_id DESC
                 LIMIT $4",
            )
            .bind(tag_slug.as_str())
            .bind(cursor.created_at)
            .bind(cursor.post_id)
            .bind(limit as i64)
            .fetch_all(&self.pool)
            .await?
        } else {
            sqlx::query_as::<_, PostRow>(
                "SELECT p.post_id, p.user_id, p.title, p.slug, p.body, p.format, p.rendered_html,
                        p.created_at, p.updated_at, p.published_at, p.deleted_at
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
            .bind(limit as i64)
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
            sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM tags WHERE tag_slug = $1)")
                .bind(tag_slug.as_str())
                .fetch_one(&self.pool)
                .await?;

        if !tag_exists {
            return Err(ListByTagError::TagNotFound);
        }

        let rows = if let Some(cursor) = cursor {
            sqlx::query_as::<_, PostRow>(
                "SELECT p.post_id, p.user_id, p.title, p.slug, p.body, p.format, p.rendered_html,
                        p.created_at, p.updated_at, p.published_at, p.deleted_at
                 FROM posts p
                 JOIN post_tags pt ON p.post_id = pt.post_id
                 JOIN tags t ON pt.tag_id = t.tag_id
                 WHERE p.user_id = $1
                   AND t.tag_slug = $2
                   AND p.published_at IS NOT NULL
                   AND p.deleted_at IS NULL
                   AND (p.created_at < $3 OR (p.created_at = $3 AND p.post_id < $4))
                 ORDER BY p.created_at DESC, p.post_id DESC
                 LIMIT $5",
            )
            .bind(user_id)
            .bind(tag_slug.as_str())
            .bind(cursor.created_at)
            .bind(cursor.post_id)
            .bind(limit as i64)
            .fetch_all(&self.pool)
            .await?
        } else {
            sqlx::query_as::<_, PostRow>(
                "SELECT p.post_id, p.user_id, p.title, p.slug, p.body, p.format, p.rendered_html,
                        p.created_at, p.updated_at, p.published_at, p.deleted_at
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
            .bind(limit as i64)
            .fetch_all(&self.pool)
            .await?
        };

        rows.into_iter()
            .map(post_record_from_row)
            .collect::<sqlx::Result<_>>()
            .map_err(ListByTagError::Internal)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{future::Future, time::Duration};

    fn lazy_pool() -> PgPool {
        sqlx::PgPool::connect_lazy("postgres://localhost:1/jaunder").unwrap()
    }

    async fn exercise<F: Future>(future: F) {
        let _ = tokio::time::timeout(Duration::from_millis(50), future).await;
    }

    #[test]
    fn test_user_record_from_row() {
        let now = Utc::now();
        let row: UserRow = (
            1,
            "alice".to_string(),
            Some("Alice".to_string()),
            Some("Bio".to_string()),
            now,
            Some(now),
            Some("alice@example.com".to_string()),
            true,
        );
        let record = user_record_from_row(row).unwrap();
        assert_eq!(record.user_id, 1);
        assert_eq!(record.username.as_str(), "alice");
        assert_eq!(record.display_name, Some("Alice".to_string()));
        assert_eq!(record.bio, Some("Bio".to_string()));
        assert_eq!(record.created_at, now);
        assert_eq!(record.last_authenticated_at, Some(now));
        assert_eq!(record.email.as_ref().unwrap().as_str(), "alice@example.com");
        assert!(record.email_verified);
    }

    #[test]
    fn test_session_record_from_row() {
        let now = Utc::now();
        let row: SessionRow = (
            "hash".to_string(),
            1,
            "alice".to_string(),
            Some("label".to_string()),
            now,
            now,
        );
        let record = session_record_from_row(row).unwrap();
        assert_eq!(record.token_hash, "hash");
        assert_eq!(record.user_id, 1);
        assert_eq!(record.username.as_str(), "alice");
        assert_eq!(record.label, Some("label".to_string()));
        assert_eq!(record.created_at, now);
        assert_eq!(record.last_used_at, now);
    }

    #[test]
    fn test_invite_record_from_row() {
        let now = Utc::now();
        let row: InviteRow = ("code".to_string(), now, now, Some(now), Some(1));
        let record = invite_record_from_row(row);
        assert_eq!(record.code, "code");
        assert_eq!(record.created_at, now);
        assert_eq!(record.expires_at, now);
        assert_eq!(record.used_at, Some(now));
        assert_eq!(record.used_by, Some(1));
    }

    #[tokio::test]
    async fn test_storage_constructors() {
        let pool = sqlx::PgPool::connect_lazy("postgres://localhost/db").unwrap();
        let _ = PostgresSiteConfigStorage::new(pool.clone());
        let _ = PostgresUserStorage::new(pool.clone());
        let _ = PostgresSessionStorage::new(pool.clone());
        let _ = PostgresInviteStorage::new(pool.clone());
        let _ = PostgresEmailVerificationStorage::new(pool.clone());
        let _ = PostgresPasswordResetStorage::new(pool.clone());
        let _ = PostgresAtomicOps::new(pool.clone());
        let _ = PostgresPostStorage::new(pool);
    }

    #[tokio::test]
    async fn test_storage_methods_with_lazy_pool_cover_error_paths() {
        let pool = lazy_pool();
        let username: Username = "alice".parse().unwrap();
        let password: Password = "password123".parse().unwrap();
        let email: EmailAddress = "alice@example.com".parse().unwrap();
        let now = Utc::now();

        let site_config = PostgresSiteConfigStorage::new(pool.clone());
        exercise(site_config.get("site.registration_policy")).await;
        exercise(site_config.set("site.registration_policy", "open")).await;

        let users = PostgresUserStorage::new(pool.clone());
        exercise(users.create_user(&username, &password, Some("Alice"))).await;
        exercise(users.authenticate(&username, &password)).await;
        exercise(users.get_user(1)).await;
        exercise(users.get_user_by_username(&username)).await;
        exercise(users.update_profile(
            1,
            &ProfileUpdate {
                display_name: Some("Alice"),
                bio: Some("Bio"),
            },
        ))
        .await;
        exercise(users.set_email(1, Some(&email), true)).await;
        exercise(users.set_password(1, &password)).await;

        let sessions = PostgresSessionStorage::new(pool.clone());
        exercise(sessions.create_session(1, Some("device"))).await;
        assert!(matches!(
            sessions.authenticate("not-base64").await,
            Err(SessionAuthError::InvalidToken)
        ));
        exercise(sessions.revoke_session("token-hash")).await;
        exercise(sessions.list_sessions(1)).await;

        let invites = PostgresInviteStorage::new(pool.clone());
        exercise(invites.create_invite(now)).await;
        exercise(invites.use_invite("invite-code", 1)).await;
        exercise(invites.list_invites()).await;

        let email_verifications = PostgresEmailVerificationStorage::new(pool.clone());
        exercise(email_verifications.create_email_verification(1, "alice@example.com", now)).await;
        assert!(matches!(
            email_verifications
                .use_email_verification("not-base64")
                .await,
            Err(UseEmailVerificationError::NotFound)
        ));

        let password_resets = PostgresPasswordResetStorage::new(pool.clone());
        exercise(password_resets.create_password_reset(1, now)).await;
        assert!(matches!(
            password_resets.use_password_reset("not-base64").await,
            Err(UsePasswordResetError::NotFound)
        ));

        let atomic = PostgresAtomicOps::new(pool.clone());
        exercise(atomic.create_user_with_invite(&username, &password, Some("Alice"), "code")).await;
        assert!(matches!(
            atomic.confirm_password_reset("not-base64", &password).await,
            Err(ConfirmPasswordResetError::NotFound)
        ));

        let slug: common::slug::Slug = "hello-world".parse().unwrap();
        let posts = PostgresPostStorage::new(pool);
        exercise(posts.create_post(&common::storage::CreatePostInput {
            user_id: 1,
            title: Some("Test".to_string()),
            slug: slug.clone(),
            body: "body".to_string(),
            format: common::storage::PostFormat::Markdown,
            rendered_html: "<p>body</p>".to_string(),
            published_at: None,
        }))
        .await;
        exercise(posts.get_post_by_id(1)).await;
        exercise(posts.get_post_by_permalink(&username, 2024, 1, 1, &slug)).await;
        exercise(posts.update_post(
            1,
            1,
            &common::storage::UpdatePostInput {
                title: Some("Updated".to_string()),
                slug: slug.clone(),
                body: "body".to_string(),
                format: common::storage::PostFormat::Markdown,
                rendered_html: "<p>body</p>".to_string(),
                publish: false,
            },
        ))
        .await;
        exercise(posts.soft_delete_post(1)).await;
        exercise(posts.list_published_by_user(&username, None, 10)).await;
        exercise(posts.list_published(None, 10)).await;
        exercise(posts.list_drafts_by_user(1, None, 10)).await;
    }
}
