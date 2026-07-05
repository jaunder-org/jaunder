//! Atomic cross-trait operations.

use async_trait::async_trait;
use thiserror::Error;

use common::password::Password;
use common::username::Username;

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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::{backends, Backend};
    use rstest::*;
    use rstest_reuse::*;

    async fn seed_invite(state: &std::sync::Arc<crate::AppState>) -> String {
        state
            .invites
            .create_invite(chrono::Utc::now() + chrono::Duration::hours(1))
            .await
            .unwrap()
    }

    #[apply(backends)]
    #[tokio::test]
    async fn create_user_with_invite_hash_failure_returns_internal_error(#[case] backend: Backend) {
        let env = backend.setup().await;
        let code = seed_invite(&env.state).await;
        let username: Username = "alice".parse().unwrap();
        let password: Password = "force-hash-error-for-test-coverage".parse().unwrap();
        let result = env
            .state
            .atomic
            .create_user_with_invite(&username, &password, None, false, &code)
            .await;
        assert!(matches!(result, Err(RegisterWithInviteError::Internal(_))));
    }

    #[apply(backends)]
    #[tokio::test]
    async fn create_user_with_invite_insert_error_returns_internal(#[case] backend: Backend) {
        let env = backend.setup().await;
        let code = seed_invite(&env.state).await;
        // Break the users INSERT (but not the invite SELECT) so the user insert
        // returns a non-unique Database error, exercising the catch-all `Internal`
        // arm and the transaction rollback path on an unexpected failure.
        env.base
            .pool()
            .execute("ALTER TABLE users RENAME COLUMN username TO username_renamed")
            .await
            .unwrap();
        let username: Username = "alice".parse().unwrap();
        let password: Password = "password123".parse().unwrap();
        let result = env
            .state
            .atomic
            .create_user_with_invite(&username, &password, None, false, &code)
            .await;
        assert!(matches!(result, Err(RegisterWithInviteError::Internal(_))));
    }

    #[apply(backends)]
    #[tokio::test]
    async fn storage_methods_on_closed_pool_return_errors(#[case] backend: Backend) {
        let env = backend.setup().await;
        env.base.close_pool().await;
        let username: Username = "alice".parse().unwrap();
        let password: Password = "password123".parse().unwrap();

        assert!(env
            .state
            .site_config
            .get("site.registration_policy")
            .await
            .is_err());
        assert!(env
            .state
            .site_config
            .set("site.registration_policy", "open")
            .await
            .is_err());
        assert!(env
            .state
            .atomic
            .create_user_with_invite(&username, &password, Some("Alice"), false, "code")
            .await
            .is_err());
        // `not-base64` fails token hashing before touching the pool, so the
        // classification is `NotFound` on both backends regardless of pool state.
        assert!(matches!(
            env.state
                .atomic
                .confirm_password_reset("not-base64", &password)
                .await,
            Err(ConfirmPasswordResetError::NotFound)
        ));
    }
}
