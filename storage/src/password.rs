//! Password reset token storage.

use async_trait::async_trait;

use sqlx::Database;
use thiserror::Error;

use common::time::UtcInstant;
use common::token::RawToken;

use crate::WriteTransaction;
use crate::backend::Backend;
use crate::helpers::TokenStateRow;
use crate::sql::RowCount;
use common::ids::UserId;
use host::token;

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
#[cfg_attr(feature = "test-utils", mockall::automock)]
#[async_trait]
pub trait PasswordResetStorage: Send + Sync {
    /// Stores a new reset token for a user.
    ///
    /// Returns the raw (un-hashed) token to be delivered to the user.
    async fn create_password_reset(
        &self,
        transaction: &mut WriteTransaction,
        user_id: UserId,
        expires_at: UtcInstant,
    ) -> sqlx::Result<RawToken>;

    /// Validates a raw reset token and marks it as used.
    ///
    /// Returns the consumption on success. Its caller must log the consumption
    /// only after the enclosing transaction is confirmed committed.
    ///
    /// # Errors
    ///
    /// Returns [`UsePasswordResetError`] if the token is invalid, expired,
    /// or already used.
    async fn use_password_reset(
        &self,
        transaction: &mut WriteTransaction,
        raw_token: &RawToken,
    ) -> Result<crate::PasswordResetConsumption, UsePasswordResetError>;

    /// Deletes consumed reset tokens and unused tokens expired for at least 24
    /// hours, draining bounded batches at the supplied instant.
    async fn prune_password_resets(&self, now: UtcInstant) -> sqlx::Result<u64>;
}

/// Generic [`PasswordResetStorage`] backed by any [`Backend`] database.
///
/// Zero backend divergence (identical SQL across `SQLite` and Postgres),
pub struct PasswordResetStore<DB: Database> {
    pool: sqlx::Pool<DB>,
}

const PRUNE_BATCH_SIZE: i64 = 100;

impl<DB: Database> PasswordResetStore<DB> {
    #[must_use]
    pub fn new(pool: sqlx::Pool<DB>) -> Self {
        Self { pool }
    }
}

#[async_trait]
impl<DB> PasswordResetStorage for PasswordResetStore<DB>
where
    DB: Backend,
    (UserId,): for<'r> sqlx::FromRow<'r, DB::Row>,
    TokenStateRow: for<'r> sqlx::FromRow<'r, DB::Row>,
    for<'q> i64: sqlx::Decode<'q, DB> + sqlx::Encode<'q, DB> + sqlx::Type<DB>,
    usize: sqlx::ColumnIndex<DB::Row>,
    for<'q> RowCount: sqlx::Decode<'q, DB> + sqlx::Type<DB>,
    String: sqlx::Type<DB>,
    for<'q> String: sqlx::Encode<'q, DB>,
    for<'q> UtcInstant: sqlx::Encode<'q, DB> + sqlx::Type<DB>,
    for<'c> &'c mut DB::Connection: sqlx::Executor<'c, Database = DB>,
    for<'c> &'c sqlx::Pool<DB>: sqlx::Executor<'c, Database = DB>,
    for<'q> DB::Arguments<'q>: sqlx::IntoArguments<'q, DB>,
{
    async fn create_password_reset(
        &self,
        transaction: &mut WriteTransaction,
        user_id: UserId,
        expires_at: UtcInstant,
    ) -> sqlx::Result<RawToken> {
        let (raw_token, token_hash) = token::generate_hashed();
        let now = UtcInstant::now();
        let connection = DB::write_connection(transaction)?;

        sqlx::query(
            "INSERT INTO password_resets (token_hash, user_id, created_at, expires_at)
             VALUES ($1, $2, $3, $4)",
        )
        .bind(token_hash)
        .bind(user_id)
        .bind(now)
        .bind(expires_at)
        .execute(&mut *connection)
        .await?;

        Ok(raw_token)
    }

    async fn use_password_reset(
        &self,
        transaction: &mut WriteTransaction,
        raw_token: &RawToken,
    ) -> Result<crate::PasswordResetConsumption, UsePasswordResetError> {
        let token_hash = token::hash(raw_token).map_err(|_| UsePasswordResetError::NotFound)?;

        let now = UtcInstant::now();
        let connection = DB::write_connection(transaction)?;

        // Atomically claim the token in one statement: the UPDATE succeeds only
        // when it exists, is unused, and is unexpired, so two concurrent requests
        // cannot both succeed and no read-then-write lock upgrade is needed
        // (ADR-0021). A miss falls through to a read that classifies the failure.
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

        if let Some((user_id,)) = claimed {
            return Ok(crate::PasswordResetConsumption { user_id });
        }
        let row = sqlx::query_as::<_, TokenStateRow>(
            "SELECT used_at, expires_at FROM password_resets WHERE token_hash = $1",
        )
        .bind(&token_hash)
        .fetch_optional(&mut *connection)
        .await?;

        Err(crate::helpers::password_reset_claim_error(row, now))
    }
    async fn prune_password_resets(&self, now: UtcInstant) -> sqlx::Result<u64> {
        let unused_cutoff = UtcInstant::from(now.value() - chrono::Duration::hours(24));
        let mut deleted = 0;

        loop {
            // Pool execution releases the connection after this bounded statement
            // before the next batch starts.
            let batch = sqlx::query_scalar::<_, RowCount>(
                "DELETE FROM password_resets
                 WHERE token_hash IN (
                     SELECT token_hash FROM password_resets
                     WHERE used_at IS NOT NULL OR expires_at <= $1
                     ORDER BY token_hash
                     LIMIT $2
                 )
                 RETURNING CAST(1 AS BIGINT)",
            )
            .bind(unused_cutoff)
            .bind(PRUNE_BATCH_SIZE)
            .fetch_all(&self.pool)
            .await?
            .len() as u64;
            deleted += batch;
            if batch == 0 {
                return Ok(deleted);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::{Backend, SeedUser, backends};
    use rstest::*;
    use rstest_reuse::*;
    use std::sync::Arc;

    #[apply(backends)]
    #[tokio::test]
    async fn password_reset_round_trips_the_token(#[case] backend: Backend) {
        // Keep the whole `TestEnv` bound: dropping `base` unlinks the SQLite file
        // (ADR-0053 TempDir hazard).
        let env = backend.setup().await;
        let user_id = SeedUser::new().seed(&env.state).await.user_id;
        let expires_at: UtcInstant = "2099-01-02T03:04:05.123456Z".parse().unwrap();

        // `create_password_reset` binds the `TokenHash`; `use_password_reset`
        // re-binds the hash of the same raw token to atomically claim the stored
        // row — a round trip through the `token_hash` column's sqlx bridge (#438).
        let password_resets = Arc::clone(&env.state.password_resets);
        let outcome = env
            .state
            .write_scope
            .run(|transaction| {
                Box::pin(async move {
                    password_resets
                        .create_password_reset(transaction, user_id, expires_at)
                        .await
                })
            })
            .await
            .unwrap();
        let raw_token = crate::test_support::confirmed_for(outcome, "password-reset fixture setup");
        let password_resets = Arc::clone(&env.state.password_resets);
        let outcome = env
            .state
            .write_scope
            .run(|transaction| {
                Box::pin(async move {
                    password_resets
                        .use_password_reset(transaction, &raw_token)
                        .await
                })
            })
            .await
            .unwrap();
        let consumption = crate::test_support::confirmed_for(outcome, "password reset");
        assert_eq!(consumption.user_id, user_id);
    }

    #[apply(backends)]
    #[tokio::test]
    async fn prune_password_resets_removes_eligible_rows_without_touching_valid_tokens(
        #[case] backend: Backend,
    ) {
        let env = backend.setup().await;
        let user_id = SeedUser::new().seed(&env.state).await.user_id;
        let now: UtcInstant = "2050-01-02T03:04:05Z".parse().unwrap();
        let eligible_at = UtcInstant::from(now.value() - chrono::Duration::hours(24));
        let valid_until = UtcInstant::from(now.value() + chrono::Duration::hours(1));

        let password_resets = Arc::clone(&env.state.password_resets);
        let outcome = env
            .state
            .write_scope
            .run(|transaction| {
                Box::pin(async move {
                    password_resets
                        .create_password_reset(transaction, user_id, eligible_at)
                        .await
                })
            })
            .await
            .unwrap();
        let expired_token =
            crate::test_support::confirmed_for(outcome, "expired password-reset fixture");
        let password_resets = Arc::clone(&env.state.password_resets);
        let outcome = env
            .state
            .write_scope
            .run(|transaction| {
                Box::pin(async move {
                    password_resets
                        .create_password_reset(transaction, user_id, valid_until)
                        .await
                })
            })
            .await
            .unwrap();
        let valid_token =
            crate::test_support::confirmed_for(outcome, "valid password-reset fixture");

        assert_eq!(
            env.state
                .password_resets
                .prune_password_resets(now)
                .await
                .unwrap(),
            1
        );
        let password_resets = Arc::clone(&env.state.password_resets);
        let expired = env
            .state
            .write_scope
            .run(|transaction| {
                Box::pin(async move {
                    password_resets
                        .use_password_reset(transaction, &expired_token)
                        .await
                })
            })
            .await
            .unwrap_err();
        assert!(matches!(
            expired,
            crate::WriteScopeError::Operation(UsePasswordResetError::NotFound)
        ));
        let password_resets = Arc::clone(&env.state.password_resets);
        let valid = env
            .state
            .write_scope
            .run(|transaction| {
                Box::pin(async move {
                    password_resets
                        .use_password_reset(transaction, &valid_token)
                        .await
                })
            })
            .await
            .unwrap();
        assert_eq!(
            crate::test_support::confirmed_for(valid, "valid password reset").user_id,
            user_id
        );
        assert_eq!(
            env.state
                .password_resets
                .prune_password_resets(now)
                .await
                .unwrap(),
            1
        );
    }

    #[apply(backends)]
    #[tokio::test]
    async fn prune_password_resets_with_closed_pool_returns_error(#[case] backend: Backend) {
        let env = backend.setup().await;
        env.base.close_pool().await;
        assert!(
            env.state
                .password_resets
                .prune_password_resets(UtcInstant::now())
                .await
                .is_err()
        );
    }
}
