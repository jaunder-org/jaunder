use chrono::{DateTime, Utc};
use email_address::EmailAddress;
use sqlx::SqlitePool;

use async_trait::async_trait;

use common::password::Password;
use common::storage::{
    CreateUserError, EmailVerificationStorage, InviteRecord, InviteStorage, ProfileUpdate,
    SessionAuthError, SessionRecord, SessionStorage, SiteConfigStorage, UseEmailVerificationError,
    UseInviteError, UserAuthError, UserRecord, UserStorage,
};
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
        let password = password.clone();
        let password_hash = tokio::task::spawn_blocking(move || password.hash())
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

        let password = password.clone();
        let valid = tokio::task::spawn_blocking(move || password.verify(&hash))
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
                .parse::<Username>()
                .map_err(|e| UserAuthError::Internal(e.to_string()))?,
            display_name,
            bio,
            created_at,
            last_authenticated_at: Some(now),
            email: email
                .map(|s| {
                    s.parse::<EmailAddress>()
                        .map_err(|e| UserAuthError::Internal(e.to_string()))
                })
                .transpose()?,
            email_verified,
        })
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
    Ok(SessionRecord {
        token_hash,
        user_id,
        username: username
            .parse::<Username>()
            .map_err(|e| sqlx::Error::Io(std::io::Error::other(e.to_string())))?,
        label,
        created_at,
        last_used_at,
    })
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

    async fn authenticate(&self, raw_token: &str) -> Result<SessionRecord, SessionAuthError> {
        let token_hash =
            crate::auth::hash_token(raw_token).map_err(|_| SessionAuthError::InvalidToken)?;

        let mut tx = self.pool.begin().await?;

        let row = sqlx::query_as::<_, SessionRow>(
            "SELECT s.token_hash, s.user_id, u.username, s.label, s.created_at, s.last_used_at
             FROM sessions s JOIN users u ON s.user_id = u.user_id
             WHERE s.token_hash = ?",
        )
        .bind(&token_hash)
        .fetch_optional(&mut *tx)
        .await?
        .ok_or(SessionAuthError::SessionNotFound)?;

        let now = Utc::now();

        sqlx::query("UPDATE sessions SET last_used_at = ? WHERE token_hash = ?")
            .bind(now)
            .bind(&token_hash)
            .execute(&mut *tx)
            .await?;

        tx.commit().await?;

        let mut record = session_record_from_row(row)?;
        record.last_used_at = now;
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
    InviteRecord {
        code,
        created_at,
        expires_at,
        used_at,
        used_by,
    }
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
