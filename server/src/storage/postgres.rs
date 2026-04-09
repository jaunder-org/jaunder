use async_trait::async_trait;
use chrono::{DateTime, Utc};
use email_address::EmailAddress;
use sqlx::PgPool;

use common::password::Password;
use common::storage::{
    AtomicOps, ConfirmPasswordResetError, CreateUserError, EmailVerificationStorage, InviteRecord,
    InviteStorage, PasswordResetStorage, ProfileUpdate, RegisterWithInviteError, SessionAuthError,
    SessionRecord, SessionStorage, SiteConfigStorage, UseEmailVerificationError, UseInviteError,
    UsePasswordResetError, UserAuthError, UserRecord, UserStorage,
};
use common::username::Username;

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
             VALUES ($1, $2, $3, $4)
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
            Err(sqlx::Error::Database(error)) if error.is_unique_violation() => {
                Err(CreateUserError::UsernameTaken)
            }
            Err(error) => Err(CreateUserError::Internal(error)),
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
            "SELECT user_id, username, display_name, bio, created_at, last_authenticated_at,
                    password_hash, email, email_verified
             FROM users WHERE username = $1",
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
            Some(row) => row,
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
        sqlx::query("UPDATE users SET last_authenticated_at = $1 WHERE user_id = $2")
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
        let password = new_password.clone();
        let password_hash = tokio::task::spawn_blocking(move || password.hash())
            .await
            .map_err(|e| sqlx::Error::Io(std::io::Error::other(e)))?
            .map_err(|e| sqlx::Error::Io(std::io::Error::other(e)))?;

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

    async fn authenticate(&self, raw_token: &str) -> Result<SessionRecord, SessionAuthError> {
        let token_hash =
            crate::auth::hash_token(raw_token).map_err(|_| SessionAuthError::InvalidToken)?;

        let mut tx = self.pool.begin().await?;
        let row = sqlx::query_as::<_, SessionRow>(
            "SELECT s.token_hash, s.user_id, u.username, s.label, s.created_at, s.last_used_at
             FROM sessions s JOIN users u ON s.user_id = u.user_id
             WHERE s.token_hash = $1",
        )
        .bind(&token_hash)
        .fetch_optional(&mut *tx)
        .await?
        .ok_or(SessionAuthError::SessionNotFound)?;

        let now = Utc::now();
        sqlx::query("UPDATE sessions SET last_used_at = $1 WHERE token_hash = $2")
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
    InviteRecord {
        code,
        created_at,
        expires_at,
        used_at,
        used_by,
    }
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
        let mut tx = self.pool.begin().await.map_err(|_| UseInviteError::NotFound)?;
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

        let password = password.clone();
        let password_hash = tokio::task::spawn_blocking(move || password.hash())
            .await
            .map_err(|e| {
                RegisterWithInviteError::Internal(sqlx::Error::Io(std::io::Error::other(e)))
            })?
            .map_err(|e| {
                RegisterWithInviteError::Internal(sqlx::Error::Io(std::io::Error::other(e)))
            })?;

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
