use async_trait::async_trait;

use sqlx::SqlitePool;

use crate::helpers::{
    self, InviteTokenStateRow, TokenState, TokenStateRow, classify_invite_token_state,
    classify_token_state,
};
use crate::{AtomicOps, ConfirmPasswordResetError, RegisterWithInviteError};
use common::display_name::DisplayName;
use common::ids::UserId;
use common::time::UtcInstant;
use common::token::RawToken;
use common::username::Username;
use host::invite::InviteCode;
use host::password::Password;

pub(crate) fn finish_password_reset_rejection(
    primary: Result<(), ConfirmPasswordResetError>,
    rollback: Result<(), sqlx::Error>,
) -> Result<(), ConfirmPasswordResetError> {
    helpers::preserve_after_secondary(
        primary,
        rollback,
        host::error::ErrorKind::Storage,
        host::error::ErrorClass::Transient,
        "storage.sqlite.password_reset.rollback",
    )
}

fn finish_invite_registration(
    primary: Result<UserId, RegisterWithInviteError>,
    rollback: Result<(), sqlx::Error>,
) -> Result<UserId, RegisterWithInviteError> {
    helpers::preserve_after_secondary(
        primary,
        rollback,
        host::error::ErrorKind::Storage,
        host::error::ErrorClass::Transient,
        "storage.sqlite.invite_registration.rollback",
    )
}

/// `SQLite` implementation of [`AtomicOps`].
///
/// Holds the pool directly so it can span multiple tables in a single
/// transaction without going through the individual storage trait objects.
pub struct SqliteAtomicOps {
    pool: SqlitePool,
}

impl SqliteAtomicOps {
    #[must_use]
    pub fn new(pool: SqlitePool) -> Self {
        Self { pool }
    }

    pub(crate) async fn confirm_password_reset_with(
        &self,
        raw_token: &RawToken,
        new_password: &Password,
        hash_operation: helpers::HashPasswordOperation,
    ) -> Result<(), ConfirmPasswordResetError> {
        let token_hash =
            host::token::hash(raw_token).map_err(|_| ConfirmPasswordResetError::NotFound)?;

        let mut tx = self.pool.begin().await?;
        let now = UtcInstant::now();

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
            let row = sqlx::query_as::<_, TokenStateRow>(
                "SELECT used_at, expires_at FROM password_resets WHERE token_hash = $1",
            )
            .bind(&token_hash)
            .fetch_optional(&mut *tx)
            .await?;

            let primary = match classify_token_state(row, now) {
                TokenState::Missing => Err(ConfirmPasswordResetError::NotFound),
                TokenState::AlreadyUsed => Err(ConfirmPasswordResetError::AlreadyUsed),
                TokenState::Expired | TokenState::Claimable => {
                    Err(ConfirmPasswordResetError::Expired)
                }
            };
            return finish_password_reset_rejection(primary, tx.rollback().await);
        };

        // ADR-0022: hash only after the token claim succeeds, so a bogus/used/expired
        // token is rejected above without paying the Argon2 cost. A hash failure here
        // `?`-returns and drops the tx → rollback → the claim reverts (token not consumed).
        let password_hash = helpers::hash_password_with(new_password.clone(), hash_operation)
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
impl AtomicOps for SqliteAtomicOps {
    #[tracing::instrument(
        name = "storage.atomic.create_user_with_invite",
        skip(self, password, display_name, invite_code),
        fields(username = %username, db.system = "sqlite")
    )]
    async fn create_user_with_invite(
        &self,
        username: &Username,
        password: &Password,
        display_name: Option<&DisplayName>,
        is_operator: bool,
        invite_code: &InviteCode,
    ) -> Result<UserId, RegisterWithInviteError> {
        // ADR-0021: take the write lock up front with BEGIN IMMEDIATE rather than a
        // deferred BEGIN, so the SELECT->INSERT step performs no shared->reserved lock
        // upgrade (the SQLITE_BUSY-on-upgrade failure mode). sqlx's Transaction issues
        // its own deferred BEGIN, so drive the transaction manually on a raw connection,
        // mirroring sqlite/backup.rs.
        //
        // ADR-0022: the invite (a high-entropy secret) is validated *before* hashing, so
        // a bogus code is rejected without paying the Argon2 cost. The hash therefore runs
        // inside the immediate transaction on the success path only.
        let mut conn = self.pool.acquire().await?;
        sqlx::query("BEGIN IMMEDIATE").execute(&mut *conn).await?;

        let result: Result<UserId, RegisterWithInviteError> = async {
            // Read the invite's state first so the three failures stay distinct: no row ->
            // InviteNotFound, used_at set -> InviteAlreadyUsed, past expires_at -> InviteExpired.
            // These checks deliberately are NOT folded into the write (e.g. a single
            // `UPDATE ... WHERE used_at IS NULL AND expires_at > now` claim): that would collapse
            // all three into one indistinguishable "zero rows affected" outcome and lose the
            // specific error the caller needs. Reporting them distinctly is what keeps this a
            // read-then-write transaction (hence BEGIN IMMEDIATE above), not a single-statement
            // claim.
            let row = sqlx::query_as::<_, InviteTokenStateRow>(
                "SELECT used_at, expires_at FROM invites WHERE code = $1",
            )
            .bind(invite_code)
            .fetch_optional(&mut *conn)
            .await?;

            let now = UtcInstant::now();
            match classify_invite_token_state(row, now) {
                TokenState::Missing => return Err(RegisterWithInviteError::InviteNotFound),
                TokenState::AlreadyUsed => return Err(RegisterWithInviteError::InviteAlreadyUsed),
                TokenState::Expired => return Err(RegisterWithInviteError::InviteExpired),
                TokenState::Claimable => {}
            }

            let password_hash = helpers::hash_password(password.clone())
                .await
                .map_err(|e| RegisterWithInviteError::Internal(sqlx::Error::Io(e)))?;

            let insert = sqlx::query_scalar::<_, UserId>(
                "INSERT INTO users (username, password_hash, display_name, created_at, is_operator)
                 VALUES ($1, $2, $3, $4, $5)
                 RETURNING user_id",
            )
            .bind(username)
            .bind(&password_hash)
            .bind(display_name)
            .bind(now)
            // sqlx-newtype-bind:allow permanent-primitive — boolean operator flag has no domain identity.
            .bind(is_operator)
            .fetch_one(&mut *conn)
            .await;

            let user_id = match insert {
                Ok(id) => id,
                Err(sqlx::Error::Database(error)) if error.is_unique_violation() => {
                    return Err(RegisterWithInviteError::UsernameTaken);
                }
                Err(error) => return Err(RegisterWithInviteError::Internal(error)),
            };

            sqlx::query("UPDATE invites SET used_at = $1, used_by = $2 WHERE code = $3")
                .bind(now)
                .bind(user_id)
                .bind(invite_code)
                .execute(&mut *conn)
                .await?;

            Ok(user_id)
        }
        .await;

        match result {
            Ok(user_id) => {
                sqlx::query("COMMIT").execute(&mut *conn).await?;
                Ok(user_id)
            }
            Err(error) => finish_invite_registration(
                Err(error),
                sqlx::query("ROLLBACK")
                    .execute(&mut *conn)
                    .await
                    .map(|_| ()),
            ),
        }
    }

    async fn confirm_password_reset(
        &self,
        raw_token: &RawToken,
        new_password: &Password,
    ) -> Result<(), ConfirmPasswordResetError> {
        self.confirm_password_reset_with(
            raw_token,
            new_password,
            helpers::hash_password_operation(new_password),
        )
        .await
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn continuation_reporting_transaction_finish_failures_preserve_atomic_domain_errors_and_report_once()
     {
        let (result, trace) = crate::helpers::swallowed_test::capture(|| {
            finish_password_reset_rejection(
                Err(ConfirmPasswordResetError::Expired),
                Err(sqlx::Error::PoolClosed),
            )
        });
        assert!(matches!(result, Err(ConfirmPasswordResetError::Expired)));
        crate::helpers::swallowed_test::assert_one_report(
            &trace,
            "storage.sqlite.password_reset.rollback",
        );

        let (result, trace) = crate::helpers::swallowed_test::capture(|| {
            finish_invite_registration(
                Err(RegisterWithInviteError::InviteAlreadyUsed),
                Err(sqlx::Error::PoolClosed),
            )
        });
        assert!(matches!(
            result,
            Err(RegisterWithInviteError::InviteAlreadyUsed)
        ));
        crate::helpers::swallowed_test::assert_one_report(
            &trace,
            "storage.sqlite.invite_registration.rollback",
        );
    }
}
