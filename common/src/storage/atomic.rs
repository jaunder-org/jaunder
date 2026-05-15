//! Atomic cross-trait operations.

use async_trait::async_trait;
use thiserror::Error;

use crate::password::Password;
use crate::username::Username;

/// Errors that can occur during atomic invite-and-user creation.
#[derive(Debug, Error)]
pub enum RegisterWithInviteError {
    /// The provided invite code does not exist.
    #[error("invite code not found")]
    InviteNotFound,
    /// The invite code has passed its expiration date.
    #[error("invite code has expired")]
    InviteExpired,
    /// The invite code has already been used by another user.
    #[error("invite code has already been used")]
    InviteAlreadyUsed,
    /// The requested username is already taken.
    #[error("username is already taken")]
    UsernameTaken,
    /// An unexpected database error occurred.
    #[error(transparent)]
    Internal(#[from] sqlx::Error),
}

/// Errors returned by an atomic password-reset confirmation.
#[derive(Debug, Error)]
pub enum ConfirmPasswordResetError {
    /// The reset token does not exist.
    #[error("token not found")]
    NotFound,
    /// The reset token has expired.
    #[error("token has expired")]
    Expired,
    /// The reset token has already been consumed.
    #[error("token has already been used")]
    AlreadyUsed,
    /// An unexpected database error occurred.
    #[error(transparent)]
    Internal(#[from] sqlx::Error),
}

/// Cross-table operations that must be executed atomically.
///
/// These operations span multiple storage traits (e.g., `users` and `invites`)
/// and are implemented as single database transactions in the concrete backend
/// to ensure data consistency.
#[async_trait]
pub trait AtomicOps: Send + Sync {
    /// Atomically creates a user and marks an invite code as used.
    ///
    /// This ensures that a user is never created without a valid invite,
    /// and an invite is never "lost" if user creation fails.
    ///
    /// # Errors
    ///
    /// Returns [`RegisterWithInviteError`] if any part of the transaction fails.
    async fn create_user_with_invite(
        &self,
        username: &Username,
        password: &Password,
        display_name: Option<&str>,
        is_operator: bool,
        invite_code: &str,
    ) -> Result<i64, RegisterWithInviteError>;

    /// Atomically consumes a password-reset token and updates the user's password.
    ///
    /// This operation also revokes all active sessions for the user to ensure
    /// account security after a password change.
    ///
    /// # Errors
    ///
    /// Returns [`ConfirmPasswordResetError`] if any part of the transaction fails.
    async fn confirm_password_reset(
        &self,
        raw_token: &str,
        new_password: &Password,
    ) -> Result<(), ConfirmPasswordResetError>;
}
