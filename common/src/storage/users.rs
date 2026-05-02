use async_trait::async_trait;
use chrono::{DateTime, Utc};
use email_address::EmailAddress;
use thiserror::Error;

use crate::password::Password;
use crate::username::Username;

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
    pub is_operator: bool,
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
        is_operator: bool,
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
