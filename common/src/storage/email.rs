use async_trait::async_trait;
use chrono::{DateTime, Utc};
use thiserror::Error;

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
