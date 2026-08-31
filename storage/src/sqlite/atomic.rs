use std::sync::Arc;

use async_trait::async_trait;

use crate::backend::Backend;

use crate::helpers::{self, InviteTokenStateRow, TokenState, TokenStateRow};
use crate::{
    AtomicOps, ConfirmPasswordResetError, RegisterWithInviteError, UserStorage, WriteTransaction,
    prepare_password,
};
use common::display_name::DisplayName;
use common::ids::UserId;
use common::time::UtcInstant;
use common::token::RawToken;
use common::username::Username;
use host::invite::InviteCode;
use host::password::Password;

/// `SQLite` implementation of [`AtomicOps`].
///
/// The write scope owns the transaction. This adapter delegates user mutations
/// to the same capability-taking store used by standalone paths.
pub struct SqliteAtomicOps {
    users: Arc<dyn UserStorage>,
}

impl SqliteAtomicOps {
    #[must_use]
    pub fn new(users: Arc<dyn UserStorage>) -> Self {
        Self { users }
    }

    async fn confirm_password_reset_impl(
        &self,
        transaction: &mut WriteTransaction,
        raw_token: &RawToken,
        new_password: &Password,
    ) -> Result<(), ConfirmPasswordResetError> {
        let token_hash =
            host::token::hash(raw_token).map_err(|_| ConfirmPasswordResetError::NotFound)?;
        let now = UtcInstant::now();
        let connection = <sqlx::Sqlite as Backend>::write_connection(transaction)?;
        let claimed = sqlx::query_as::<_, (UserId,)>(
            "UPDATE password_resets SET used_at = $1
             WHERE token_hash = $2 AND used_at IS NULL AND expires_at > $3
             RETURNING user_id",
        )
        .bind(now)
        .bind(&token_hash)
        .bind(now)
        .fetch_optional(&mut *connection)
        .await?;

        let Some((user_id,)) = claimed else {
            let row = sqlx::query_as::<_, TokenStateRow>(
                "SELECT used_at, expires_at FROM password_resets WHERE token_hash = $1",
            )
            .bind(&token_hash)
            .fetch_optional(&mut *connection)
            .await?;
            return match helpers::classify_token_state(row, now) {
                TokenState::Missing => Err(ConfirmPasswordResetError::NotFound),
                TokenState::AlreadyUsed => Err(ConfirmPasswordResetError::AlreadyUsed),
                TokenState::Expired | TokenState::Claimable => {
                    Err(ConfirmPasswordResetError::Expired)
                }
            };
        };

        // Reset tokens are high-entropy capabilities: only the successful claim
        // reaches password preparation and the shared mutation (ADR-0022).
        let password = prepare_password(new_password.clone())
            .await
            .map_err(|error| ConfirmPasswordResetError::Internal(sqlx::Error::Io(error)))?;
        self.users
            .set_password(transaction, user_id, &password)
            .await
            .map_err(ConfirmPasswordResetError::Internal)?;
        let connection = <sqlx::Sqlite as Backend>::write_connection(transaction)?;
        sqlx::query("DELETE FROM sessions WHERE user_id = $1")
            .bind(user_id)
            .execute(&mut *connection)
            .await?;
        Ok(())
    }
}

#[async_trait]
impl AtomicOps for SqliteAtomicOps {
    #[tracing::instrument(
        name = "storage.atomic.create_user_with_invite",
        skip(self, transaction, password, display_name, invite_code),
        fields(username = %username, db.system = "sqlite")
    )]
    async fn create_user_with_invite(
        &self,
        transaction: &mut WriteTransaction,
        username: &Username,
        password: &Password,
        display_name: Option<&DisplayName>,
        is_operator: bool,
        invite_code: &InviteCode,
    ) -> Result<UserId, RegisterWithInviteError> {
        let connection = <sqlx::Sqlite as Backend>::write_connection(transaction)?;
        let row = sqlx::query_as::<_, InviteTokenStateRow>(
            "SELECT used_at, expires_at FROM invites WHERE code = $1",
        )
        .bind(invite_code)
        .fetch_optional(&mut *connection)
        .await?;

        let now = UtcInstant::now();
        match helpers::classify_invite_token_state(row, now) {
            TokenState::Missing => return Err(RegisterWithInviteError::InviteNotFound),
            TokenState::AlreadyUsed => return Err(RegisterWithInviteError::InviteAlreadyUsed),
            TokenState::Expired => return Err(RegisterWithInviteError::InviteExpired),
            TokenState::Claimable => {}
        }

        // The scope began with BEGIN IMMEDIATE. After this high-entropy invite
        // has been validated, preparation may perform Argon2 under lock.
        let password = prepare_password(password.clone())
            .await
            .map_err(|error| RegisterWithInviteError::Internal(sqlx::Error::Io(error)))?;
        let user_id = self
            .users
            .create_user(transaction, username, &password, display_name, is_operator)
            .await
            .map_err(|error| match error {
                crate::CreateUserError::UsernameTaken => RegisterWithInviteError::UsernameTaken,
                crate::CreateUserError::Internal(error) => RegisterWithInviteError::Internal(error),
            })?;

        let connection = <sqlx::Sqlite as Backend>::write_connection(transaction)?;
        sqlx::query("UPDATE invites SET used_at = $1, used_by = $2 WHERE code = $3")
            .bind(now)
            .bind(user_id)
            .bind(invite_code)
            .execute(&mut *connection)
            .await?;
        Ok(user_id)
    }

    async fn confirm_password_reset(
        &self,
        transaction: &mut WriteTransaction,
        raw_token: &RawToken,
        new_password: &Password,
    ) -> Result<(), ConfirmPasswordResetError> {
        self.confirm_password_reset_impl(transaction, raw_token, new_password)
            .await
    }
}
