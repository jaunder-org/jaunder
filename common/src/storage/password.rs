use async_trait::async_trait;
use chrono::{DateTime, Utc};
use thiserror::Error;

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
