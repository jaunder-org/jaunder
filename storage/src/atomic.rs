//! Atomic cross-trait operations.

use async_trait::async_trait;
use thiserror::Error;

use common::display_name::DisplayName;
use common::ids::UserId;
use common::token::RawToken;
use common::username::Username;
use host::invite::InviteCode;
use host::password::Password;

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

impl From<RegisterWithInviteError> for host::error::InternalError {
    /// Reproduces the former `web::auth::server::register_invite_error`
    /// `(kind, class, public_message)`: a taken username is a client conflict,
    /// the invite-code failures are client validation errors, and an internal
    /// failure is a masked storage error.
    fn from(error: RegisterWithInviteError) -> Self {
        use host::error::InternalError;
        match error {
            RegisterWithInviteError::UsernameTaken => {
                InternalError::conflict("username is already taken")
            }
            RegisterWithInviteError::InviteNotFound => {
                InternalError::validation("invite code not found")
            }
            RegisterWithInviteError::InviteExpired => {
                InternalError::validation("invite code has expired")
            }
            RegisterWithInviteError::InviteAlreadyUsed => {
                InternalError::validation("invite code has already been used")
            }
            RegisterWithInviteError::Internal(e) => InternalError::storage(e),
        }
    }
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

impl From<ConfirmPasswordResetError> for host::error::InternalError {
    /// Mirrors the sibling [`RegisterWithInviteError`] mapping so
    /// `confirm_password_reset` is `?`-liftable in `web`: the three token
    /// failures are client validation errors (a stale/used/unknown reset link,
    /// not a server fault), and an internal failure is a masked storage error
    /// (#344).
    fn from(error: ConfirmPasswordResetError) -> Self {
        use host::error::InternalError;
        match error {
            ConfirmPasswordResetError::NotFound => InternalError::validation("token not found"),
            ConfirmPasswordResetError::Expired => InternalError::validation("token has expired"),
            ConfirmPasswordResetError::AlreadyUsed => {
                InternalError::validation("token has already been used")
            }
            ConfirmPasswordResetError::Internal(e) => InternalError::storage(e),
        }
    }
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
        display_name: Option<&DisplayName>,
        is_operator: bool,
        invite_code: &InviteCode,
    ) -> Result<UserId, RegisterWithInviteError>;

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
        raw_token: &RawToken,
        new_password: &Password,
    ) -> Result<(), ConfirmPasswordResetError>;
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::{Backend, CloseablePool, SeedUser, backends, parse_invite_code};
    use common::test_support::{parse_display_name, parse_username};
    use rstest::*;
    use rstest_reuse::*;

    #[test]
    fn confirm_reset_error_maps_each_variant_to_expected_kind() {
        use host::error::{ErrorKind, InternalError};
        // #344: the three token failures are client validation errors, not a
        // masked storage 500 — while a genuine DB fault still masks as storage.
        for error in [
            ConfirmPasswordResetError::NotFound,
            ConfirmPasswordResetError::Expired,
            ConfirmPasswordResetError::AlreadyUsed,
        ] {
            let mapped: InternalError = error.into();
            assert_eq!(mapped.kind(), ErrorKind::Validation);
        }
        let mapped: InternalError =
            ConfirmPasswordResetError::Internal(sqlx::Error::RowNotFound).into();
        assert_eq!(mapped.kind(), ErrorKind::Storage);
    }

    async fn seed_invite(state: &std::sync::Arc<crate::AppState>) -> InviteCode {
        state
            .invites
            .create_invite(common::time::UtcInstant::from(
                chrono::Utc::now() + chrono::Duration::hours(1),
            ))
            .await
            .unwrap()
    }

    #[apply(backends)]
    #[tokio::test]
    async fn create_user_with_invite_hash_failure_returns_internal_error(#[case] backend: Backend) {
        let env = backend.setup().await;
        let code = seed_invite(&env.state).await;
        let username = parse_username("alice");
        let password: Password =
            host::test_support::parse_password("force-hash-error-for-test-coverage");
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
        let username = parse_username("alice");
        let password: Password = host::test_support::parse_password("password123");
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
        let username = parse_username("alice");
        let password: Password = host::test_support::parse_password("password123");
        let display_name = parse_display_name("Alice");

        assert!(
            env.state
                .site_config
                .get_raw(crate::SiteConfigKey::SiteRegistrationPolicy)
                .await
                .is_err()
        );
        assert!(
            env.state
                .site_config
                .set(crate::SiteConfigKey::SiteRegistrationPolicy, "open")
                .await
                .is_err()
        );
        assert!(
            env.state
                .atomic
                .create_user_with_invite(
                    &username,
                    &password,
                    Some(&display_name),
                    false,
                    &parse_invite_code("code"),
                )
                .await
                .is_err()
        );
    }

    #[apply(backends)]
    #[tokio::test]
    async fn confirm_password_reset_with_closed_pool_returns_error(#[case] backend: Backend) {
        let env = backend.setup().await;
        let (raw_token, _) = host::token::generate_hashed();
        env.base.close_pool().await;
        let password = host::test_support::parse_password("password123");

        let result = env
            .state
            .atomic
            .confirm_password_reset(&raw_token, &password)
            .await;

        assert!(matches!(
            result,
            Err(ConfirmPasswordResetError::Internal(_))
        ));
    }

    #[apply(backends)]
    #[tokio::test]
    async fn continuation_reporting_password_reset_not_found_survives_injected_rollback_failure(
        #[case] backend: Backend,
    ) {
        let env = backend.setup().await;
        let (raw_token, _) = host::token::generate_hashed();
        let password = host::test_support::parse_password("password123");
        let primary = env
            .state
            .atomic
            .confirm_password_reset(&raw_token, &password)
            .await;
        assert!(matches!(&primary, Err(ConfirmPasswordResetError::NotFound)));

        let (result, trace) = crate::helpers::swallowed_test::capture(|| match backend {
            Backend::Sqlite => crate::sqlite::atomic::finish_password_reset_rejection(
                primary,
                Err(sqlx::Error::PoolClosed),
            ),
            Backend::Postgres => crate::postgres::atomic::finish_password_reset_rejection(
                primary,
                Err(sqlx::Error::PoolClosed),
            ),
        });

        assert!(matches!(result, Err(ConfirmPasswordResetError::NotFound)));
        let context = match backend {
            Backend::Sqlite => "storage.sqlite.password_reset.rollback",
            Backend::Postgres => "storage.postgres.password_reset.rollback",
        };
        crate::helpers::swallowed_test::assert_one_report(&trace, context);
    }

    #[apply(backends)]
    #[tokio::test]
    async fn password_reset_hash_failure_retains_source_chain(#[case] backend: Backend) {
        let env = backend.setup().await;
        let user_id = SeedUser::new().seed(&env.state).await.user_id;
        let raw_token = env
            .state
            .password_resets
            .create_password_reset(
                user_id,
                common::time::UtcInstant::from(chrono::Utc::now() + chrono::Duration::hours(1)),
            )
            .await
            .unwrap();
        let password = host::test_support::parse_password("password123");
        let expected = crate::helpers::forced_hash_failure(&password).unwrap_err();

        let result = match env.base.pool() {
            CloseablePool::Sqlite(pool) => {
                crate::sqlite::SqliteAtomicOps::new(pool.clone())
                    .confirm_password_reset_with(
                        &raw_token,
                        &password,
                        crate::helpers::forced_hash_failure,
                    )
                    .await
            }
            CloseablePool::Postgres(pool) => {
                crate::postgres::PostgresAtomicOps::new(pool.clone())
                    .confirm_password_reset_with(
                        &raw_token,
                        &password,
                        crate::helpers::forced_hash_failure,
                    )
                    .await
            }
        };

        let error = result.unwrap_err();
        let ConfirmPasswordResetError::Internal(sqlx::Error::Io(io_error)) = &error else {
            panic!("expected SQL I/O password-reset failure");
        };
        let password_error = io_error
            .get_ref()
            .and_then(|source| source.downcast_ref::<host::password::PasswordError>())
            .expect("sqlx io::Error retains PasswordError");
        let (
            host::password::PasswordError::HashingFailed(actual),
            host::password::PasswordError::HashingFailed(expected),
        ) = (password_error, &expected)
        else {
            panic!("expected typed hashing failures");
        };

        assert_eq!(actual, expected);
    }

    // Each variant maps to a fixed `(kind, public_message)` pair.
    #[test]
    fn from_register_with_invite_error_maps_variants() {
        use host::error::{ErrorKind, InternalError};

        let taken: InternalError = RegisterWithInviteError::UsernameTaken.into();
        assert_eq!(taken.kind(), ErrorKind::Conflict);
        assert_eq!(taken.public_message(), "username is already taken");

        let not_found: InternalError = RegisterWithInviteError::InviteNotFound.into();
        assert_eq!(not_found.kind(), ErrorKind::Validation);
        assert_eq!(not_found.public_message(), "invite code not found");

        let expired: InternalError = RegisterWithInviteError::InviteExpired.into();
        assert_eq!(expired.kind(), ErrorKind::Validation);
        assert_eq!(expired.public_message(), "invite code has expired");

        let used: InternalError = RegisterWithInviteError::InviteAlreadyUsed.into();
        assert_eq!(used.kind(), ErrorKind::Validation);
        assert_eq!(used.public_message(), "invite code has already been used");

        let internal: InternalError =
            RegisterWithInviteError::Internal(sqlx::Error::PoolClosed).into();
        assert_eq!(internal.kind(), ErrorKind::Storage);
        assert_eq!(internal.public_message(), "storage operation failed");
    }
}
