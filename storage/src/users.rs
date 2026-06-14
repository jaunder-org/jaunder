//! User account and profile storage.

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use email_address::EmailAddress;
use thiserror::Error;

use common::password::Password;
use common::username::Username;

/// A user account record returned by [`UserStorage`] queries.
///
/// Does not expose `password_hash`; that field is only accessed inside the
/// storage implementation to ensure it is never accidentally leaked to
/// higher-level application logic.
#[derive(Clone, Debug)]
pub struct UserRecord {
    /// Unique internal identifier.
    pub user_id: i64,
    /// Unique username (canonicalized).
    pub username: Username,
    /// User's preferred display name.
    pub display_name: Option<String>,
    /// Optional short biography.
    pub bio: Option<String>,
    /// When the account was created.
    pub created_at: DateTime<Utc>,
    /// When the user last successfully authenticated.
    pub last_authenticated_at: Option<DateTime<Utc>>,
    /// User's verified or pending email address.
    pub email: Option<EmailAddress>,
    /// Whether the email address has been verified.
    pub email_verified: bool,
    /// Whether the user has site-wide administrative privileges.
    pub is_operator: bool,
}

/// Errors that can occur when creating a user.
#[derive(Debug, Error)]
pub enum CreateUserError {
    /// The requested username is already in use by another account.
    #[error("username is already taken")]
    UsernameTaken,
    /// An unexpected database error occurred.
    #[error(transparent)]
    Internal(#[from] sqlx::Error),
}

/// Errors that can occur when authenticating a user by password.
#[derive(Debug, Error)]
pub enum UserAuthError {
    /// The username or password was incorrect.
    #[error("invalid credentials")]
    InvalidCredentials,
    /// An unexpected error occurred during the authentication process.
    ///
    /// Carries the underlying error as a typed source (a `sqlx::Error` from the
    /// DB lookup/update, an `io::Error` from password verification, or a record
    /// conversion error) rather than a flattened string, so the boundary can
    /// downcast for classification.
    #[error("internal error: {0}")]
    Internal(#[source] Box<dyn std::error::Error + Send + Sync>),
}

/// Fields to update on a user's profile.
///
/// Each field is `Option<&str>`: `None` clears the field, `Some(v)` sets it.
pub struct ProfileUpdate<'a> {
    /// New display name, or `None` to clear.
    pub display_name: Option<&'a str>,
    /// New bio text, or `None` to clear.
    pub bio: Option<&'a str>,
}

/// Async operations on the `users` table.
///
/// This trait defines the core interface for managing user accounts, including
/// creation, authentication, and profile management.
#[async_trait]
pub trait UserStorage: Send + Sync {
    /// Creates a new user account.
    ///
    /// The password will be hashed using a cryptographically secure algorithm
    /// (e.g., bcrypt) before being stored.
    ///
    /// # Errors
    ///
    /// Returns [`CreateUserError::UsernameTaken`] if the username exists, or
    /// [`CreateUserError::Internal`] on database failure.
    async fn create_user(
        &self,
        username: &Username,
        password: &Password,
        display_name: Option<&str>,
        is_operator: bool,
    ) -> Result<i64, CreateUserError>;

    /// Authenticates a user by username and password.
    ///
    /// On success, updates `last_authenticated_at` for the user.
    ///
    /// # Errors
    ///
    /// Returns [`UserAuthError::InvalidCredentials`] if the credentials don't match,
    /// or [`UserAuthError::Internal`] on unexpected failures.
    async fn authenticate(
        &self,
        username: &Username,
        password: &Password,
    ) -> Result<UserRecord, UserAuthError>;

    /// Fetches a user record by its internal ID.
    async fn get_user(&self, user_id: i64) -> sqlx::Result<Option<UserRecord>>;

    /// Fetches a user record by their username.
    async fn get_user_by_username(&self, username: &Username) -> sqlx::Result<Option<UserRecord>>;

    /// Updates the display name and/or bio for a user.
    async fn update_profile(&self, user_id: i64, update: &ProfileUpdate<'_>) -> sqlx::Result<()>;

    /// Sets or clears a user's email address and verification status.
    async fn set_email(
        &self,
        user_id: i64,
        email: Option<&EmailAddress>,
        verified: bool,
    ) -> sqlx::Result<()>;

    /// Replaces the stored password hash for `user_id` with a hash of `new_password`.
    ///
    /// This is typically used during password resets. Hashing is performed
    /// asynchronously on a blocking thread.
    async fn set_password(&self, user_id: i64, new_password: &Password) -> sqlx::Result<()>;
}
