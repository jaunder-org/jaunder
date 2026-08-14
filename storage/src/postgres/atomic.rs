use async_trait::async_trait;
use chrono::{DateTime, Utc};
use sqlx::PgPool;

use crate::{AtomicOps, ConfirmPasswordResetError, RegisterWithInviteError};
use common::display_name::DisplayName;
use common::ids::UserId;
use common::password::Password;
use common::token::RawToken;
use common::username::Username;
use host::invite::InviteCode;

pub(crate) fn finish_password_reset_rejection(
    primary: Result<(), ConfirmPasswordResetError>,
    rollback: Result<(), sqlx::Error>,
) -> Result<(), ConfirmPasswordResetError> {
    crate::helpers::preserve_after_secondary(
        primary,
        rollback,
        host::error::ErrorKind::Storage,
        host::error::ErrorClass::Transient,
        "storage.postgres.password_reset.rollback",
    )
}

pub struct PostgresAtomicOps {
    pool: PgPool,
}

impl PostgresAtomicOps {
    #[must_use]
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }

    pub(crate) async fn confirm_password_reset_with(
        &self,
        raw_token: &RawToken,
        new_password: &Password,
        hash_operation: crate::helpers::HashPasswordOperation,
    ) -> Result<(), ConfirmPasswordResetError> {
        let token_hash =
            host::token::hash(raw_token).map_err(|_| ConfirmPasswordResetError::NotFound)?;

        let mut tx = self.pool.begin().await?;
        let now = Utc::now();
        // Claim the token in one atomic UPDATE: it matches only when the token
        // exists, is unused, and is unexpired, so concurrent confirmations cannot
        // both win (ADR-0021). On a miss we re-read to classify the failure into a
        // precise NotFound / AlreadyUsed / Expired error.
        let claimed = sqlx::query_as::<_, (UserId,)>(
            "UPDATE password_resets SET used_at = $1
             WHERE token_hash = $2 AND used_at IS NULL AND expires_at > $3
             RETURNING user_id",
        )
        .bind(now)
        .bind(&token_hash)
        .bind(now)
        .fetch_optional(&mut *tx)
        .await?;

        let Some((user_id,)) = claimed else {
            let row = sqlx::query_as::<_, (Option<DateTime<Utc>>, DateTime<Utc>)>(
                "SELECT used_at, expires_at FROM password_resets WHERE token_hash = $1",
            )
            .bind(&token_hash)
            .fetch_optional(&mut *tx)
            .await?;

            let primary = match row {
                None => Err(ConfirmPasswordResetError::NotFound),
                Some((Some(_), _)) => Err(ConfirmPasswordResetError::AlreadyUsed),
                Some((None, _)) => Err(ConfirmPasswordResetError::Expired),
            };
            return finish_password_reset_rejection(primary, tx.rollback().await);
        };

        // ADR-0022: hash only after the token claim succeeds, so a bogus/used/expired
        // token is rejected above without paying the Argon2 cost. A hash failure here
        // `?`-returns and drops the tx → rollback → the claim reverts (token not consumed).
        let password_hash =
            crate::helpers::hash_password_with(new_password.clone(), hash_operation)
                .await
                .map_err(|e| ConfirmPasswordResetError::Internal(sqlx::Error::Io(e)))?;

        sqlx::query("UPDATE users SET password_hash = $1 WHERE user_id = $2")
            .bind(&password_hash)
            .bind(user_id)
            .execute(&mut *tx)
            .await?;

        sqlx::query("DELETE FROM sessions WHERE user_id = $1")
            .bind(user_id)
            .execute(&mut *tx)
            .await?;

        tx.commit().await?;
        Ok(())
    }
}

#[async_trait]
impl AtomicOps for PostgresAtomicOps {
    async fn create_user_with_invite(
        &self,
        username: &Username,
        password: &Password,
        display_name: Option<&DisplayName>,
        is_operator: bool,
        invite_code: &InviteCode,
    ) -> Result<UserId, RegisterWithInviteError> {
        let mut tx = self.pool.begin().await?;
        let row = sqlx::query_as::<_, (Option<DateTime<Utc>>, DateTime<Utc>)>(
            "SELECT used_at, expires_at FROM invites WHERE code = $1",
        )
        .bind(invite_code)
        .fetch_optional(&mut *tx)
        .await?
        .ok_or(RegisterWithInviteError::InviteNotFound)?;

        let (used_at, expires_at) = row;
        if used_at.is_some() {
            return Err(RegisterWithInviteError::InviteAlreadyUsed);
        }

        let now = Utc::now();
        if expires_at <= now {
            return Err(RegisterWithInviteError::InviteExpired);
        }

        let password_hash = crate::helpers::hash_password(password.clone())
            .await
            .map_err(|e| RegisterWithInviteError::Internal(sqlx::Error::Io(e)))?; // cov:ignore

        let result = sqlx::query_scalar::<_, UserId>(
            "INSERT INTO users (username, password_hash, display_name, created_at, is_operator)
             VALUES ($1, $2, $3, $4, $5)
             RETURNING user_id",
        )
        .bind(username)
        .bind(&password_hash)
        .bind(display_name)
        .bind(now)
        .bind(is_operator)
        .fetch_one(&mut *tx)
        .await;

        let user_id = match result {
            Ok(id) => id,
            // Let the UNIQUE(username) constraint be the arbiter rather than a
            // pre-INSERT existence check: that closes the check-then-insert race
            // between concurrent registrations.
            Err(sqlx::Error::Database(error)) if error.is_unique_violation() => {
                return Err(RegisterWithInviteError::UsernameTaken);
            }
            Err(error) => return Err(RegisterWithInviteError::Internal(error)),
        };

        sqlx::query("UPDATE invites SET used_at = $1, used_by = $2 WHERE code = $3")
            .bind(now)
            .bind(user_id)
            .bind(invite_code)
            .execute(&mut *tx)
            .await?;

        tx.commit().await?;
        Ok(user_id)
    }

    async fn confirm_password_reset(
        &self,
        raw_token: &RawToken,
        new_password: &Password,
    ) -> Result<(), ConfirmPasswordResetError> {
        self.confirm_password_reset_with(
            raw_token,
            new_password,
            crate::helpers::hash_password_operation(new_password),
        )
        .await
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn continuation_reporting_password_reset_rollback_failure_preserves_token_rejection_and_reports_once()
     {
        let (result, trace) = crate::helpers::swallowed_test::capture(|| {
            finish_password_reset_rejection(
                Err(ConfirmPasswordResetError::AlreadyUsed),
                Err(sqlx::Error::PoolClosed),
            )
        });
        assert!(matches!(
            result,
            Err(ConfirmPasswordResetError::AlreadyUsed)
        ));
        crate::helpers::swallowed_test::assert_one_report(
            &trace,
            "storage.postgres.password_reset.rollback",
        );
    }
}
