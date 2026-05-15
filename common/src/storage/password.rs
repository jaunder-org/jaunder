//! Password reset token storage.

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use thiserror::Error;

/// Errors returned by [`PasswordResetStorage::use_password_reset`].
#[derive(Debug, Error)]
pub enum UsePasswordResetError {
    /// The reset token does not exist.
    #[error("token not found")]
    NotFound,
    /// The token has passed its expiration date.
    #[error("token has expired")]
    Expired,
    /// The token has already been consumed.
    #[error("token has already been used")]
    AlreadyUsed,
    /// An unexpected database error occurred.
    #[error(transparent)]
    Internal(#[from] sqlx::Error),
}

/// Storage for password-reset tokens.
///
/// This trait manages the lifecycle of tokens sent to users to allow them to
/// reset their passwords via email.
#[async_trait]
pub trait PasswordResetStorage: Send + Sync {
    /// Stores a new reset token for a user.
    ///
    /// Returns the raw (un-hashed) token to be delivered to the user.
    async fn create_password_reset(
        &self,
        user_id: i64,
        expires_at: DateTime<Utc>,
    ) -> sqlx::Result<String>;

    /// Validates a raw reset token and marks it as used.
    ///
    /// Returns the associated `user_id` on success.
    ///
    /// # Errors
    ///
    /// Returns [`UsePasswordResetError`] if the token is invalid, expired,
    /// or already used.
    async fn use_password_reset(&self, raw_token: &str) -> Result<i64, UsePasswordResetError>;
}
