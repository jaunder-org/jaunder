use async_trait::async_trait;
use thiserror::Error;

use crate::password::Password;
use crate::username::Username;

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
        is_operator: bool,
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
