//! Password reset token storage.

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use sqlx::{Database, Pool};
use thiserror::Error;

use common::token::RawToken;

use crate::backend::Backend;

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
        user_id: i64,
        expires_at: DateTime<Utc>,
    ) -> sqlx::Result<RawToken>;

    /// Validates a raw reset token and marks it as used.
    ///
    /// Returns the associated `user_id` on success.
    ///
    /// # Errors
    ///
    /// Returns [`UsePasswordResetError`] if the token is invalid, expired,
    /// or already used.
    async fn use_password_reset(&self, raw_token: &RawToken) -> Result<i64, UsePasswordResetError>;
}

/// Generic [`PasswordResetStorage`] backed by any [`Backend`] database.
///
/// Zero backend divergence (identical SQL across `SQLite` and Postgres),
/// so it is implemented once here; see ADR-0019.
pub struct PasswordResetStore<DB: Database> {
    pool: Pool<DB>,
}

impl<DB: Database> PasswordResetStore<DB> {
    #[must_use]
    pub fn new(pool: Pool<DB>) -> Self {
        Self { pool }
    }
}

#[async_trait]
impl<DB> PasswordResetStorage for PasswordResetStore<DB>
where
    DB: Backend,
    (i64,): for<'r> sqlx::FromRow<'r, DB::Row>,
    (Option<DateTime<Utc>>, DateTime<Utc>): for<'r> sqlx::FromRow<'r, DB::Row>,
    for<'q> i64: sqlx::Encode<'q, DB> + sqlx::Type<DB>,
    for<'q> &'q str: sqlx::Encode<'q, DB> + sqlx::Type<DB>,
    for<'q> DateTime<Utc>: sqlx::Encode<'q, DB> + sqlx::Type<DB>,
    for<'c> &'c Pool<DB>: sqlx::Executor<'c, Database = DB>,
    for<'q> DB::Arguments<'q>: sqlx::IntoArguments<'q, DB>,
{
    async fn create_password_reset(
        &self,
        user_id: i64,
        expires_at: DateTime<Utc>,
    ) -> sqlx::Result<RawToken> {
        let (raw_token, token_hash) = host::token::generate_hashed();
        let now = Utc::now();

        sqlx::query(
            "INSERT INTO password_resets (token_hash, user_id, created_at, expires_at)
             VALUES ($1, $2, $3, $4)",
        )
        .bind(token_hash.as_ref())
        .bind(user_id)
        .bind(now)
        .bind(expires_at)
        .execute(&self.pool)
        .await?;

        Ok(raw_token)
    }

    async fn use_password_reset(&self, raw_token: &RawToken) -> Result<i64, UsePasswordResetError> {
        let token_hash =
            host::token::hash(raw_token).map_err(|_| UsePasswordResetError::NotFound)?;

        let now = Utc::now();

        // Atomically claim the token in one statement: the UPDATE succeeds only
        // when it exists, is unused, and is unexpired, so two concurrent requests
        // cannot both succeed and no read-then-write lock upgrade is needed
        // (ADR-0021). A miss falls through to a read that classifies the failure.
        let claimed = sqlx::query_as::<_, (i64,)>(
            "UPDATE password_resets SET used_at = $1
             WHERE token_hash = $2 AND used_at IS NULL AND expires_at > $3
             RETURNING user_id",
        )
        .bind(now)
        .bind(token_hash.as_ref())
        .bind(now)
        .fetch_optional(&self.pool)
        .await?;

        if let Some((user_id,)) = claimed {
            return Ok(user_id);
        }

        let row = sqlx::query_as::<_, (Option<DateTime<Utc>>, DateTime<Utc>)>(
            "SELECT used_at, expires_at FROM password_resets WHERE token_hash = $1",
        )
        .bind(token_hash.as_ref())
        .fetch_optional(&self.pool)
        .await?;

        Err(crate::helpers::password_reset_claim_error(row))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::{backends, Backend, TestEnv};
    use rstest::*;
    use rstest_reuse::*;

    #[apply(backends)]
    #[tokio::test]
    async fn create_password_reset_with_closed_pool_returns_error(#[case] backend: Backend) {
        let TestEnv { state, base } = backend.setup().await;
        base.close_pool().await;
        let expires_at = chrono::Utc::now();
        let result = state
            .password_resets
            .create_password_reset(1, expires_at)
            .await;
        assert!(result.is_err());
    }

    #[apply(backends)]
    #[tokio::test]
    async fn use_password_reset_with_closed_pool_returns_error(#[case] backend: Backend) {
        let TestEnv { state, base } = backend.setup().await;
        base.close_pool().await;
        let result = state
            .password_resets
            .use_password_reset(&RawToken::try_from("dGVzdA".to_string()).unwrap())
            .await;
        assert!(matches!(result, Err(UsePasswordResetError::Internal(_))));
    }
}
