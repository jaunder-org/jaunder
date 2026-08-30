//! Password reset token storage.

use std::marker::PhantomData;

use async_trait::async_trait;

use sqlx::Database;
use thiserror::Error;

use common::time::UtcInstant;
use common::token::RawToken;

use crate::WriteTransaction;
use crate::backend::Backend;
use crate::helpers::TokenStateRow;
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
    /// Returns the associated `user_id` on success.
    ///
    /// # Errors
    ///
    /// Returns [`UsePasswordResetError`] if the token is invalid, expired,
    /// or already used.
    async fn use_password_reset(
        &self,
        transaction: &mut WriteTransaction,
        raw_token: &RawToken,
    ) -> Result<UserId, UsePasswordResetError>;
}

/// Generic [`PasswordResetStorage`] backed by any [`Backend`] database.
///
/// Zero backend divergence (identical SQL across `SQLite` and Postgres),
pub struct PasswordResetStore<DB: Database> {
    _database: PhantomData<fn() -> DB>,
}

impl<DB: Database> PasswordResetStore<DB> {
    #[must_use]
    pub fn new(_pool: sqlx::Pool<DB>) -> Self {
        Self {
            _database: PhantomData,
        }
    }
}

#[async_trait]
impl<DB> PasswordResetStorage for PasswordResetStore<DB>
where
    DB: Backend,
    (UserId,): for<'r> sqlx::FromRow<'r, DB::Row>,
    TokenStateRow: for<'r> sqlx::FromRow<'r, DB::Row>,
    for<'q> i64: sqlx::Encode<'q, DB> + sqlx::Type<DB>,
    // `TokenHash` binds as itself via the ADR-0071 sqlx bridge.
    String: sqlx::Type<DB>,
    for<'q> String: sqlx::Encode<'q, DB>,
    for<'q> UtcInstant: sqlx::Encode<'q, DB> + sqlx::Type<DB>,
    for<'c> &'c mut DB::Connection: sqlx::Executor<'c, Database = DB>,
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
    ) -> Result<UserId, UsePasswordResetError> {
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
            return Ok(user_id);
        }

        let row = sqlx::query_as::<_, TokenStateRow>(
            "SELECT used_at, expires_at FROM password_resets WHERE token_hash = $1",
        )
        .bind(&token_hash)
        .fetch_optional(&mut *connection)
        .await?;

        Err(crate::helpers::password_reset_claim_error(row, now))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::{Backend, SeedUser, TestEnv, backends};
    use common::test_support::parse_raw_token;
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
        let raw_token = match outcome {
            common::MutationOutcome::Confirmed(raw_token) => raw_token,
            common::MutationOutcome::CommitIndeterminate(_) => {
                panic!("password-reset fixture setup requires a confirmed commit")
            }
        };
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
        let claimed = match outcome {
            common::MutationOutcome::Confirmed(claimed) => claimed,
            common::MutationOutcome::CommitIndeterminate(_) => {
                panic!("password reset requires a confirmed commit")
            }
        };
        assert_eq!(claimed, user_id);
    }

    #[apply(backends)]
    #[tokio::test]
    async fn create_password_reset_with_closed_pool_returns_error(#[case] backend: Backend) {
        let TestEnv { state, base } = backend.setup().await;
        base.close_pool().await;
        let expires_at = UtcInstant::now();
        let password_resets = Arc::clone(&state.password_resets);
        let result = state
            .write_scope
            .run(|transaction| {
                Box::pin(async move {
                    password_resets
                        .create_password_reset(transaction, UserId::from(1), expires_at)
                        .await
                })
            })
            .await;
        assert!(matches!(
            result,
            Err(crate::WriteScopeError::Begin(sqlx::Error::PoolClosed))
        ));
    }

    #[apply(backends)]
    #[tokio::test]
    async fn use_password_reset_with_closed_pool_returns_error(#[case] backend: Backend) {
        let TestEnv { state, base } = backend.setup().await;
        base.close_pool().await;
        let password_resets = Arc::clone(&state.password_resets);
        let result = state
            .write_scope
            .run(|transaction| {
                let token = parse_raw_token("dGVzdA");
                Box::pin(async move {
                    password_resets
                        .use_password_reset(transaction, &token)
                        .await
                })
            })
            .await;
        assert!(matches!(
            result,
            Err(crate::WriteScopeError::Begin(sqlx::Error::PoolClosed))
        ));
    }
}
