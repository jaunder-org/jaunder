//! Cross-store account mutations.
//!
//! This module owns the transaction-local orchestration for account flows that
//! span storage traits. Callers own the [`WriteTransaction`] through
//! [`WriteScope`](crate::WriteScope); these functions neither begin nor commit a
//! transaction. Their dependencies are the exact object-safe stores needed by
//! each flow so the composition root does not leak into application code.

use common::{display_name::DisplayName, ids::UserId, token::RawToken, username::Username};
use host::{invite::InviteCode, password::Password};
use thiserror::Error;

use crate::{
    CreateUserError, InviteStorage, PasswordResetStorage, SessionStorage, UserStorage,
    WriteTransaction, prepare_password,
};

/// Errors returned by [`register_with_invite`].
#[derive(Debug, Error)]
pub enum RegisterWithInviteError {
    /// The provided invite code does not exist.
    #[error("invite code not found")]
    InviteNotFound,
    /// The provided invite code has expired.
    #[error("invite code has expired")]
    InviteExpired,
    /// The provided invite code has already been consumed.
    #[error("invite code has already been used")]
    InviteAlreadyUsed,
    /// The requested username is already in use.
    #[error("username is already taken")]
    UsernameTaken,
    /// An unexpected storage or password-preparation error occurred.
    #[error(transparent)]
    Internal(#[from] sqlx::Error),
}

impl From<RegisterWithInviteError> for host::error::InternalError {
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
            RegisterWithInviteError::Internal(error) => InternalError::storage(error),
        }
    }
}

impl From<crate::UseInviteError> for RegisterWithInviteError {
    fn from(error: crate::UseInviteError) -> Self {
        match error {
            crate::UseInviteError::NotFound => Self::InviteNotFound,
            crate::UseInviteError::Expired => Self::InviteExpired,
            crate::UseInviteError::AlreadyUsed => Self::InviteAlreadyUsed,
            crate::UseInviteError::Internal(error) => Self::Internal(error),
        }
    }
}

/// Errors returned by [`confirm_password_reset`].
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
    /// An unexpected storage or password-preparation error occurred.
    #[error(transparent)]
    Internal(#[from] sqlx::Error),
}

impl From<ConfirmPasswordResetError> for host::error::InternalError {
    fn from(error: ConfirmPasswordResetError) -> Self {
        use host::error::InternalError;
        match error {
            ConfirmPasswordResetError::NotFound => InternalError::validation("token not found"),
            ConfirmPasswordResetError::Expired => InternalError::validation("token has expired"),
            ConfirmPasswordResetError::AlreadyUsed => {
                InternalError::validation("token has already been used")
            }
            ConfirmPasswordResetError::Internal(error) => InternalError::storage(error),
        }
    }
}

impl From<crate::UsePasswordResetError> for ConfirmPasswordResetError {
    fn from(error: crate::UsePasswordResetError) -> Self {
        match error {
            crate::UsePasswordResetError::NotFound => Self::NotFound,
            crate::UsePasswordResetError::Expired => Self::Expired,
            crate::UsePasswordResetError::AlreadyUsed => Self::AlreadyUsed,
            crate::UsePasswordResetError::Internal(error) => Self::Internal(error),
        }
    }
}

/// Values required by [`register_with_invite`].
///
/// The caller owns all referenced values for the duration of the registration
/// operation. Storage dependencies remain function parameters so this carrier
/// describes only the registration request.
pub struct RegisterWithInviteInput<'a> {
    /// Username assigned to the newly created user.
    pub username: &'a Username,
    /// Plaintext password to prepare before creating the user.
    pub password: &'a Password,
    /// Optional display name assigned to the newly created user.
    pub display_name: Option<&'a DisplayName>,
    /// Whether the newly created user receives operator privileges.
    pub is_operator: bool,
    /// Capability that authorizes the registration.
    pub invite_code: &'a InviteCode,
}

/// Creates a user and attributes a still-valid invite to that user.
///
/// The validity precheck intentionally happens before Argon2. The claim is a
/// conditional write after user insertion, so a concurrent claimant loses with
/// `InviteAlreadyUsed`; returning the error lets the caller-owned scope roll
/// the inserted user back.
///
/// # Errors
///
/// Returns [`RegisterWithInviteError::InviteNotFound`] when the invite code is
/// unknown, [`RegisterWithInviteError::InviteExpired`] when it has expired, or
/// [`RegisterWithInviteError::InviteAlreadyUsed`] when another user has claimed
/// it. Returns [`RegisterWithInviteError::UsernameTaken`] when the username is
/// already registered, and [`RegisterWithInviteError::Internal`] if password
/// preparation or storage fails.
#[tracing::instrument(
    name = "storage.account_mutations.register_with_invite",
    skip(transaction, users, invites, input),
    fields(username = %input.username)
)]
pub async fn register_with_invite(
    transaction: &mut WriteTransaction,
    users: &dyn UserStorage,
    invites: &dyn InviteStorage,
    input: RegisterWithInviteInput<'_>,
) -> Result<UserId, RegisterWithInviteError> {
    invites
        .precheck_invite(input.invite_code)
        .await
        .map_err(RegisterWithInviteError::from)?;

    // An invite is a high-entropy capability. After its read-only precheck,
    // Argon2 may run before the transactional user insertion.
    let prepared_password = prepare_password(input.password.clone())
        .await
        .map_err(|error| RegisterWithInviteError::Internal(sqlx::Error::Io(error)))?;
    let user_id = users
        .create_user(
            transaction,
            input.username,
            &prepared_password,
            input.display_name,
            input.is_operator,
        )
        .await
        .map_err(|error| match error {
            CreateUserError::UsernameTaken => RegisterWithInviteError::UsernameTaken,
            CreateUserError::Internal(error) => RegisterWithInviteError::Internal(error),
        })?;
    invites
        .claim_invite(transaction, input.invite_code, user_id)
        .await
        .map_err(RegisterWithInviteError::from)?;
    Ok(user_id)
}

/// Consumes a reset token, replaces its user's password, and revokes every
/// session belonging to that user in the caller-owned transaction.
///
/// # Errors
///
/// Returns [`ConfirmPasswordResetError::NotFound`] when the token is unknown,
/// [`ConfirmPasswordResetError::Expired`] when it has expired, or
/// [`ConfirmPasswordResetError::AlreadyUsed`] when it was previously consumed.
/// Returns [`ConfirmPasswordResetError::Internal`] if password preparation or a
/// storage mutation fails.
#[tracing::instrument(
    name = "storage.account_mutations.confirm_password_reset",
    skip(transaction, password_resets, users, sessions, raw_token, new_password)
)]
pub async fn confirm_password_reset(
    transaction: &mut WriteTransaction,
    password_resets: &dyn PasswordResetStorage,
    users: &dyn UserStorage,
    sessions: &dyn SessionStorage,
    raw_token: &RawToken,
    new_password: &Password,
) -> Result<(), ConfirmPasswordResetError> {
    let user_id = password_resets
        .use_password_reset(transaction, raw_token)
        .await
        .map_err(ConfirmPasswordResetError::from)?;

    // Reset tokens are high-entropy capabilities: only a successful claim
    // reaches password preparation and the following mutations.
    let prepared_password = prepare_password(new_password.clone())
        .await
        .map_err(|error| ConfirmPasswordResetError::Internal(sqlx::Error::Io(error)))?;
    users
        .set_password(transaction, user_id, &prepared_password)
        .await
        .map_err(ConfirmPasswordResetError::Internal)?;
    sessions
        .revoke_all_for_user(transaction, user_id)
        .await
        .map_err(ConfirmPasswordResetError::Internal)
}

#[cfg(test)]
mod tests {
    use super::*;
    use host::error::{ErrorKind, InternalError};

    #[test]
    fn reset_token_state_errors_map_to_client_validation() {
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
}
