use chrono::{DateTime, Utc};
use email_address::EmailAddress;
use sqlx::{Row, SqlitePool};

use async_trait::async_trait;

use common::password::Password;
use common::slug::Slug;
use common::storage::{
    CreatePostError, CreatePostInput, CreateUserError, EmailVerificationStorage, InviteRecord,
    InviteStorage, ListByTagError, PasswordResetStorage, PostCursor, PostRecord, PostStorage,
    PostTag, ProfileUpdate, SessionAuthError, SessionRecord, SessionStorage, SiteConfigStorage,
    TaggingError, UpdatePostError, UpdatePostInput, UseEmailVerificationError, UseInviteError,
    UsePasswordResetError, UserAuthError, UserRecord, UserStorage,
};
use common::tag::Tag;
use common::username::Username;

// ---------------------------------------------------------------------------
// SiteConfig
// ---------------------------------------------------------------------------

/// SQLite-backed [`SiteConfigStorage`].
pub struct SqliteSiteConfigStorage {
    pool: SqlitePool,
}

impl SqliteSiteConfigStorage {
    pub fn new(pool: SqlitePool) -> Self {
        Self { pool }
    }
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
    ) -> Result<i64, CreateUserError> {
        let password_hash = super::hash_password(password.clone())
            .await
            .map_err(|e| CreateUserError::Internal(sqlx::Error::Io(e)))?;

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
            ),
        >(
            "SELECT user_id, username, display_name, bio, created_at, last_authenticated_at, password_hash, email, email_verified
             FROM users WHERE username = ?",
        )
        .bind(username.as_str())
        .fetch_optional(&self.pool)
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
            Some(r) => r,
            None => return Err(UserAuthError::InvalidCredentials),
        };

        let valid = super::verify_password(password.clone(), hash)
            .await
            .map_err(|e| UserAuthError::Internal(e.to_string()))?;

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
            "SELECT user_id, username, display_name, bio, created_at, last_authenticated_at, email, email_verified
             FROM users WHERE user_id = ?",
        )
        .bind(user_id)
        .fetch_optional(&self.pool)
        .await?;
        Ok(row.map(user_record_from_row).transpose()?)
    }

    async fn get_user_by_username(&self, username: &Username) -> sqlx::Result<Option<UserRecord>> {
        let row = sqlx::query_as::<_, UserRow>(
            "SELECT user_id, username, display_name, bio, created_at, last_authenticated_at, email, email_verified
             FROM users WHERE username = ?",
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
        sqlx::query("UPDATE users SET email = ?, email_verified = ? WHERE user_id = ?")
            .bind(email.map(EmailAddress::as_str))
            .bind(verified)
            .bind(user_id)
            .execute(&self.pool)
            .await?;
        Ok(())
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

    async fn set_password(&self, user_id: i64, new_password: &Password) -> sqlx::Result<()> {
        let password_hash = super::hash_password(new_password.clone())
            .await
            .map_err(sqlx::Error::Io)?;

        sqlx::query("UPDATE users SET password_hash = ? WHERE user_id = ?")
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
) -> Result<SessionRecord, sqlx::Error> {
    super::build_session_record(
        token_hash,
        user_id,
        username,
        label,
        created_at,
        last_used_at,
    )
}

/// SQLite-backed [`SessionStorage`].
pub struct SqliteSessionStorage {
    pool: SqlitePool,
}

impl SqliteSessionStorage {
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
        let raw_token = crate::auth::generate_token();
        let token_hash = crate::auth::hash_token(&raw_token)
            .map_err(|e| sqlx::Error::Io(std::io::Error::other(e)))?;
        let now = Utc::now();

        sqlx::query(
            "INSERT INTO sessions (token_hash, user_id, label, created_at, last_used_at)
             VALUES (?, ?, ?, ?, ?)",
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
        // here because SQLite does not support data-modifying CTEs (WITH
        // UPDATE ...).
        let row = sqlx::query_as::<_, SessionRow>(
            "UPDATE sessions
             SET last_used_at = ?
             WHERE token_hash = ?
             RETURNING token_hash, user_id, (SELECT username FROM users WHERE user_id = sessions.user_id), label, created_at, last_used_at",
        )
        .bind(now)
        .bind(&token_hash)
        .fetch_optional(&self.pool)
        .await?
        .ok_or(SessionAuthError::SessionNotFound)?;

        let record = session_record_from_row(row)?;
        Ok(record)
    }

    async fn revoke_session(&self, token_hash: &str) -> sqlx::Result<()> {
        sqlx::query("DELETE FROM sessions WHERE token_hash = ?")
            .bind(token_hash)
            .execute(&self.pool)
            .await?;
        Ok(())
    }

    async fn list_sessions(&self, user_id: i64) -> sqlx::Result<Vec<SessionRecord>> {
        let rows = sqlx::query_as::<_, SessionRow>(
            "SELECT s.token_hash, s.user_id, u.username, s.label, s.created_at, s.last_used_at
             FROM sessions s JOIN users u ON s.user_id = u.user_id
             WHERE s.user_id = ?",
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

/// SQLite-backed [`InviteStorage`].
pub struct SqliteInviteStorage {
    pool: SqlitePool,
}

impl SqliteInviteStorage {
    pub fn new(pool: SqlitePool) -> Self {
        Self { pool }
    }
}

#[async_trait]
impl InviteStorage for SqliteInviteStorage {
    async fn create_invite(&self, expires_at: DateTime<Utc>) -> sqlx::Result<String> {
        let code = crate::auth::generate_token();
        let now = Utc::now();

        sqlx::query("INSERT INTO invites (code, created_at, expires_at) VALUES (?, ?, ?)")
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
             FROM invites WHERE code = ?",
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

        sqlx::query("UPDATE invites SET used_at = ?, used_by = ? WHERE code = ?")
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
        let raw_token = crate::auth::generate_token();
        let token_hash = crate::auth::hash_token(&raw_token)
            .map_err(|e| sqlx::Error::Io(std::io::Error::other(e)))?;
        let now = Utc::now();

        let mut tx = self.pool.begin().await?;

        // Supersede any existing pending token for this user by setting its
        // expires_at to its created_at, making it appear immediately expired.
        sqlx::query(
            "UPDATE email_verifications
             SET expires_at = created_at
             WHERE user_id = ? AND used_at IS NULL AND expires_at > ?",
        )
        .bind(user_id)
        .bind(now)
        .execute(&mut *tx)
        .await?;

        sqlx::query(
            "INSERT INTO email_verifications
             (token_hash, user_id, email, created_at, expires_at)
             VALUES (?, ?, ?, ?, ?)",
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
            "UPDATE email_verifications SET used_at = ?
             WHERE token_hash = ? AND used_at IS NULL AND expires_at > ?
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
            "SELECT used_at, expires_at FROM email_verifications WHERE token_hash = ?",
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

/// SQLite-backed [`PasswordResetStorage`].
pub struct SqlitePasswordResetStorage {
    pool: SqlitePool,
}

impl SqlitePasswordResetStorage {
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
        let raw_token = crate::auth::generate_token();
        let token_hash = crate::auth::hash_token(&raw_token)
            .map_err(|e| sqlx::Error::Io(std::io::Error::other(e)))?;
        let now = Utc::now();

        sqlx::query(
            "INSERT INTO password_resets (token_hash, user_id, created_at, expires_at)
             VALUES (?, ?, ?, ?)",
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
            "UPDATE password_resets SET used_at = ?
             WHERE token_hash = ? AND used_at IS NULL AND expires_at > ?
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
            "SELECT used_at, expires_at FROM password_resets WHERE token_hash = ?",
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
// Posts
// ---------------------------------------------------------------------------

type PostRow = (
    i64,
    i64,
    String,
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

/// SQLite-backed [`PostStorage`].
pub struct SqlitePostStorage {
    pool: SqlitePool,
}

impl SqlitePostStorage {
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
             VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?)
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
             FROM posts WHERE post_id = ?",
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
             WHERE u.username = ?
               AND p.slug = ?
               AND p.published_at IS NOT NULL
               AND p.deleted_at IS NULL
               AND date(p.published_at) = ?",
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
            "SELECT user_id, deleted_at FROM posts WHERE post_id = ?",
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
             SELECT post_id, user_id, title, slug, body, format, rendered_html, ?
             FROM posts WHERE post_id = ?",
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
             SET title = ?,
                 slug = CASE WHEN published_at IS NULL THEN ? ELSE slug END,
                 body = ?,
                 format = ?,
                 rendered_html = ?,
                 published_at = CASE WHEN ? THEN COALESCE(published_at, ?) ELSE NULL END,
                 updated_at = ?
             WHERE post_id = ?
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
        sqlx::query("UPDATE posts SET deleted_at = ? WHERE post_id = ?")
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
                 WHERE u.username = ?
                   AND p.published_at IS NOT NULL
                   AND p.deleted_at IS NULL
                   AND (p.created_at < ? OR (p.created_at = ? AND p.post_id < ?))
                 ORDER BY p.created_at DESC, p.post_id DESC
                 LIMIT ?",
            )
            .bind(username.as_str())
            .bind(cursor.created_at)
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
                 WHERE u.username = ?
                   AND p.published_at IS NOT NULL
                   AND p.deleted_at IS NULL
                 ORDER BY p.created_at DESC, p.post_id DESC
                 LIMIT ?",
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
                   AND (created_at < ? OR (created_at = ? AND post_id < ?))
                 ORDER BY created_at DESC, post_id DESC
                 LIMIT ?",
            )
            .bind(cursor.created_at)
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
                 LIMIT ?",
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
                 WHERE user_id = ?
                   AND published_at IS NULL
                   AND deleted_at IS NULL
                   AND (created_at < ? OR (created_at = ? AND post_id < ?))
                 ORDER BY created_at DESC, post_id DESC
                 LIMIT ?",
            )
            .bind(user_id)
            .bind(cursor.created_at)
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
                 WHERE user_id = ?
                   AND published_at IS NULL
                   AND deleted_at IS NULL
                 ORDER BY created_at DESC, post_id DESC
                 LIMIT ?",
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
            sqlx::query_scalar("SELECT COUNT(*) > 0 FROM posts WHERE post_id = ?")
                .bind(post_id)
                .fetch_one(&mut *tx)
                .await?;

        if !post_exists {
            tx.rollback().await.ok();
            return Err(TaggingError::PostNotFound);
        }

        // Insert or ignore tag (if it already exists)
        sqlx::query("INSERT OR IGNORE INTO tags (tag_slug) VALUES (?)")
            .bind(tag.as_str())
            .execute(&mut *tx)
            .await?;

        // Get the tag_id (either from this insert or from existing)
        let tag_id: i64 =
            sqlx::query_scalar::<_, i64>("SELECT tag_id FROM tags WHERE tag_slug = ?")
                .bind(tag.as_str())
                .fetch_one(&mut *tx)
                .await?;

        // Insert post_tags link
        let result =
            sqlx::query("INSERT INTO post_tags (post_id, tag_id, tag_display) VALUES (?, ?, ?)")
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
             WHERE post_id = ? AND tag_id = (SELECT tag_id FROM tags WHERE tag_slug = ?)",
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
             WHERE pt.post_id = ?
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
            sqlx::query_scalar("SELECT COUNT(*) > 0 FROM tags WHERE tag_slug = ?")
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
                 WHERE t.tag_slug = ?
                   AND p.published_at IS NOT NULL
                   AND p.deleted_at IS NULL
                   AND (p.created_at < ? OR (p.created_at = ? AND p.post_id < ?))
                 ORDER BY p.created_at DESC, p.post_id DESC
                 LIMIT ?",
            )
            .bind(tag_slug.as_str())
            .bind(cursor.created_at)
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
                 WHERE t.tag_slug = ?
                   AND p.published_at IS NOT NULL
                   AND p.deleted_at IS NULL
                 ORDER BY p.created_at DESC, p.post_id DESC
                 LIMIT ?",
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
            sqlx::query_scalar("SELECT COUNT(*) > 0 FROM tags WHERE tag_slug = ?")
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
                 WHERE p.user_id = ?
                   AND t.tag_slug = ?
                   AND p.published_at IS NOT NULL
                   AND p.deleted_at IS NULL
                   AND (p.created_at < ? OR (p.created_at = ? AND p.post_id < ?))
                 ORDER BY p.created_at DESC, p.post_id DESC
                 LIMIT ?",
            )
            .bind(user_id)
            .bind(tag_slug.as_str())
            .bind(cursor.created_at)
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
                 WHERE p.user_id = ?
                   AND t.tag_slug = ?
                   AND p.published_at IS NOT NULL
                   AND p.deleted_at IS NULL
                 ORDER BY p.created_at DESC, p.post_id DESC
                 LIMIT ?",
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
