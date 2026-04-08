use std::sync::Arc;

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use thiserror::Error;

use email_address::EmailAddress;

use crate::mailer::MailSender;
use crate::password::Password;
use crate::username::Username;

// ---------------------------------------------------------------------------
// SiteConfig
// ---------------------------------------------------------------------------

/// Async operations on the `site_config` key-value table.
#[async_trait]
pub trait SiteConfigStorage: Send + Sync {
    /// Returns the value for `key`, or `None` if the key is not set.
    async fn get(&self, key: &str) -> sqlx::Result<Option<String>>;

    /// Inserts or replaces the value for `key`.
    async fn set(&self, key: &str, value: &str) -> sqlx::Result<()>;
}

// ---------------------------------------------------------------------------
// Users
// ---------------------------------------------------------------------------

/// A user account record returned by [`UserStorage`] queries.
///
/// Does not expose `password_hash`; that field is only accessed inside the
/// storage implementation.
#[derive(Clone, Debug)]
pub struct UserRecord {
    pub user_id: i64,
    pub username: Username,
    pub display_name: Option<String>,
    pub bio: Option<String>,
    pub created_at: DateTime<Utc>,
    pub last_authenticated_at: Option<DateTime<Utc>>,
    pub email: Option<EmailAddress>,
    pub email_verified: bool,
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

    async fn set_email(
        &self,
        user_id: i64,
        email: Option<&EmailAddress>,
        verified: bool,
    ) -> sqlx::Result<()>;

    /// Replaces the stored password hash for `user_id` with a hash of `new_password`.
    /// Hashing is performed inside `spawn_blocking`, consistent with `create_user`.
    async fn set_password(&self, user_id: i64, new_password: &Password) -> sqlx::Result<()>;
}

// ---------------------------------------------------------------------------
// Sessions
// ---------------------------------------------------------------------------

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

// ---------------------------------------------------------------------------
// Invites
// ---------------------------------------------------------------------------

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

/// Async operations on the `invites` table.
#[async_trait]
pub trait InviteStorage: Send + Sync {
    async fn create_invite(&self, expires_at: DateTime<Utc>) -> sqlx::Result<String>;

    async fn use_invite(&self, code: &str, user_id: i64) -> Result<(), UseInviteError>;

    async fn list_invites(&self) -> sqlx::Result<Vec<InviteRecord>>;
}

// ---------------------------------------------------------------------------
// Atomic cross-table operations
// ---------------------------------------------------------------------------

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

/// Errors returned by an atomic password-reset confirmation.
#[derive(Debug, Error)]
pub enum ConfirmPasswordResetError {
    #[error("token not found")]
    NotFound,
    #[error("token has expired")]
    Expired,
    #[error("token has already been used")]
    AlreadyUsed,
    #[error(transparent)]
    Internal(#[from] sqlx::Error),
}

/// Cross-table operations that span multiple storage traits and must be
/// executed atomically.
///
/// The concrete implementation holds the database pool; `common` never
/// depends on a specific database driver.
#[async_trait]
pub trait AtomicOps: Send + Sync {
    /// Atomically creates a user and marks an invite code as used within a
    /// single transaction.
    async fn create_user_with_invite(
        &self,
        username: &Username,
        password: &Password,
        display_name: Option<&str>,
        invite_code: &str,
    ) -> Result<i64, RegisterWithInviteError>;

    /// Atomically consumes a password-reset token, updates the password, and
    /// revokes all sessions for the user.
    async fn confirm_password_reset(
        &self,
        raw_token: &str,
        new_password: &Password,
    ) -> Result<(), ConfirmPasswordResetError>;
}

// ---------------------------------------------------------------------------
// EmailVerification (stub — full trait added in M3.7)
// ---------------------------------------------------------------------------

/// Errors returned by [`EmailVerificationStorage::use_email_verification`].
#[derive(Debug, Error)]
pub enum UseEmailVerificationError {
    #[error("token not found")]
    NotFound,
    #[error("token has expired")]
    Expired,
    #[error("token has already been used")]
    AlreadyUsed,
    #[error(transparent)]
    Internal(#[from] sqlx::Error),
}

/// Storage for email verification tokens.
#[async_trait]
pub trait EmailVerificationStorage: Send + Sync {
    /// Stores a new verification token for `user_id` / `email` expiring at
    /// `expires_at`.  Any existing pending token for the same user is
    /// superseded (marked expired) so only one pending token exists at a time.
    /// Returns the raw (un-hashed) token to be delivered to the user by email.
    async fn create_email_verification(
        &self,
        user_id: i64,
        email: &str,
        expires_at: DateTime<Utc>,
    ) -> sqlx::Result<String>;

    /// Validates `raw_token`, marks it used, and returns `(user_id, email)`.
    async fn use_email_verification(
        &self,
        raw_token: &str,
    ) -> Result<(i64, String), UseEmailVerificationError>;
}

// ---------------------------------------------------------------------------
// PasswordReset
// ---------------------------------------------------------------------------

/// Errors returned by [`PasswordResetStorage::use_password_reset`].
#[derive(Debug, Error)]
pub enum UsePasswordResetError {
    #[error("token not found")]
    NotFound,
    #[error("token has expired")]
    Expired,
    #[error("token has already been used")]
    AlreadyUsed,
    #[error(transparent)]
    Internal(#[from] sqlx::Error),
}

/// Storage for password-reset tokens.
#[async_trait]
pub trait PasswordResetStorage: Send + Sync {
    /// Stores a new reset token for `user_id` expiring at `expires_at`.
    /// Returns the raw (un-hashed) token to be delivered to the user by email.
    async fn create_password_reset(
        &self,
        user_id: i64,
        expires_at: DateTime<Utc>,
    ) -> sqlx::Result<String>;

    /// Validates `raw_token`, marks it used, and returns `user_id`.
    async fn use_password_reset(&self, raw_token: &str) -> Result<i64, UsePasswordResetError>;
}

// ---------------------------------------------------------------------------
// AppState
// ---------------------------------------------------------------------------

/// Application-wide state bundling all storage handles.
pub struct AppState {
    pub site_config: Arc<dyn SiteConfigStorage>,
    pub users: Arc<dyn UserStorage>,
    pub sessions: Arc<dyn SessionStorage>,
    pub invites: Arc<dyn InviteStorage>,
    /// Cross-table atomic operations.  The concrete implementation (in the
    /// `server` crate) holds the database pool so `common` and `web` stay
    /// free of SQLite implementation details.
    pub atomic: Arc<dyn AtomicOps>,
    /// Email verification token storage (stub until Step 7).
    pub email_verifications: Arc<dyn EmailVerificationStorage>,
    /// Password reset token storage (stub until Step 8).
    pub password_resets: Arc<dyn PasswordResetStorage>,
    /// Outbound email sender.  `NoopMailSender` when SMTP is not configured.
    pub mailer: Arc<dyn MailSender>,
}
