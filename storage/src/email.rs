//! Email verification token storage.

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use sqlx::{Database, Pool};
use thiserror::Error;

use crate::backend::Backend;
use common::email::Email;
use common::token::RawToken;

/// Errors returned by [`EmailVerificationStorage::use_email_verification`].
#[derive(Debug, Error)]
pub enum UseEmailVerificationError {
    /// The verification token does not exist.
    #[error("token not found")]
    NotFound,
    /// The token has passed its expiration date.
    #[error("token has expired")]
    Expired,
    /// The token has already been used.
    #[error("token has already been used")]
    AlreadyUsed,
    /// An unexpected database error occurred.
    #[error(transparent)]
    Internal(#[from] sqlx::Error),
}

impl From<UseEmailVerificationError> for host::error::InternalError {
    /// Mirrors the sibling [`crate::atomic::ConfirmPasswordResetError`] mapping so
    /// `verify_email` is `?`-liftable: the three token failures are client
    /// validation errors (a stale/used/unknown verification link), and an
    /// internal failure is a masked storage error. Previously `web` hand-mapped
    /// every variant to `storage`, masking all three as a 500.
    fn from(error: UseEmailVerificationError) -> Self {
        use host::error::InternalError;
        match error {
            UseEmailVerificationError::NotFound => InternalError::validation("token not found"),
            UseEmailVerificationError::Expired => InternalError::validation("token has expired"),
            UseEmailVerificationError::AlreadyUsed => {
                InternalError::validation("token has already been used")
            }
            UseEmailVerificationError::Internal(e) => InternalError::storage(e),
        }
    }
}

/// Storage for email verification tokens.
///
/// This trait manages the lifecycle of tokens sent to users to verify their
/// email addresses.
#[async_trait]
pub trait EmailVerificationStorage: Send + Sync {
    /// Stores a new verification token for a user's email address.
    ///
    /// Any existing pending token for the same user is invalidated (marked
    /// expired) so that only the most recently issued token is active.
    ///
    /// Returns the raw (un-hashed) token to be delivered to the user.
    async fn create_email_verification(
        &self,
        user_id: i64,
        email: &Email,
        expires_at: DateTime<Utc>,
    ) -> sqlx::Result<RawToken>;

    /// Validates a raw verification token and marks it as used.
    ///
    /// Returns the associated `(user_id, email)` on success.
    ///
    /// # Errors
    ///
    /// Returns [`UseEmailVerificationError`] if the token is invalid, expired,
    /// or already used.
    async fn use_email_verification(
        &self,
        raw_token: &RawToken,
    ) -> Result<(i64, Email), UseEmailVerificationError>;
}

/// Generic [`EmailVerificationStorage`] backed by any [`Backend`] database.
///
/// Zero backend divergence (identical SQL across `SQLite` and Postgres),
/// so it is implemented once here; see ADR-0019.
pub struct EmailVerificationStore<DB: Database> {
    pool: Pool<DB>,
}

impl<DB: Database> EmailVerificationStore<DB> {
    #[must_use]
    pub fn new(pool: Pool<DB>) -> Self {
        Self { pool }
    }
}

#[async_trait]
impl<DB> EmailVerificationStorage for EmailVerificationStore<DB>
where
    DB: Backend,
    (i64, String): for<'r> sqlx::FromRow<'r, DB::Row>,
    (Option<DateTime<Utc>>, DateTime<Utc>): for<'r> sqlx::FromRow<'r, DB::Row>,
    for<'q> i64: sqlx::Encode<'q, DB> + sqlx::Type<DB>,
    for<'q> &'q str: sqlx::Encode<'q, DB> + sqlx::Type<DB>,
    for<'q> DateTime<Utc>: sqlx::Encode<'q, DB> + sqlx::Type<DB>,
    for<'c> &'c Pool<DB>: sqlx::Executor<'c, Database = DB>,
    for<'c> &'c mut DB::Connection: sqlx::Executor<'c, Database = DB>,
    for<'q> DB::Arguments<'q>: sqlx::IntoArguments<'q, DB>,
{
    async fn create_email_verification(
        &self,
        user_id: i64,
        email: &Email,
        expires_at: DateTime<Utc>,
    ) -> sqlx::Result<RawToken> {
        let (raw_token, token_hash) = host::token::generate_hashed();
        let now = Utc::now();

        let mut tx = self.pool.begin().await?;

        // Supersede any existing pending token for this user by setting its
        // expires_at to its created_at, making it appear immediately expired.
        sqlx::query(
            "UPDATE email_verifications
             SET expires_at = created_at
             WHERE user_id = $1 AND used_at IS NULL AND expires_at > $2",
        )
        .bind(user_id)
        .bind(now)
        .execute(&mut *tx)
        .await?;

        sqlx::query(
            "INSERT INTO email_verifications
             (token_hash, user_id, email, created_at, expires_at)
             VALUES ($1, $2, $3, $4, $5)",
        )
        .bind(token_hash.as_ref())
        .bind(user_id)
        .bind(&**email)
        .bind(now)
        .bind(expires_at)
        .execute(&mut *tx)
        .await?;

        tx.commit().await?;

        Ok(raw_token)
    }

    async fn use_email_verification(
        &self,
        raw_token: &RawToken,
    ) -> Result<(i64, Email), UseEmailVerificationError> {
        let token_hash =
            host::token::hash(raw_token).map_err(|_| UseEmailVerificationError::NotFound)?;

        let now = Utc::now();

        // Atomically claim the token: the UPDATE succeeds only when the token
        // exists, has not yet been used, and has not expired.  This single
        // statement is the "claim" — no separate read is needed first, so two
        // concurrent requests cannot both succeed.  RETURNING gives us the
        // data we need without a second round-trip.
        let claimed = sqlx::query_as::<_, (i64, String)>(
            "UPDATE email_verifications SET used_at = $1
             WHERE token_hash = $2 AND used_at IS NULL AND expires_at > $3
             RETURNING user_id, email",
        )
        .bind(now)
        .bind(token_hash.as_ref())
        .bind(now)
        .fetch_optional(&self.pool)
        .await
        .map_err(|_| UseEmailVerificationError::NotFound)?;

        if let Some((user_id, email)) = claimed {
            // The address was validated when stored; a parse failure here means
            // the stored value was corrupted, which we surface as a decode error.
            let email = email
                .parse::<Email>()
                .map_err(|e| sqlx::Error::Decode(Box::new(e)))?;
            return Ok((user_id, email));
        }

        // Zero rows affected — inspect the row to return the right error.
        let row = sqlx::query_as::<_, (Option<DateTime<Utc>>, DateTime<Utc>)>(
            "SELECT used_at, expires_at FROM email_verifications WHERE token_hash = $1",
        )
        .bind(token_hash.as_ref())
        .fetch_optional(&self.pool)
        .await
        .map_err(|_| UseEmailVerificationError::NotFound)?;

        Err(crate::helpers::email_verification_claim_error(row))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::{backends, Backend, TestEnv};
    use common::test_support::{parse_email, parse_raw_token};
    use rstest::*;
    use rstest_reuse::*;

    #[test]
    fn use_email_verification_error_maps_each_variant_to_expected_kind() {
        use host::error::{ErrorKind, InternalError};
        // The three token failures are client validation errors, not a masked
        // storage 500 — while a genuine DB fault still masks as storage.
        for error in [
            UseEmailVerificationError::NotFound,
            UseEmailVerificationError::Expired,
            UseEmailVerificationError::AlreadyUsed,
        ] {
            let mapped: InternalError = error.into();
            assert_eq!(mapped.kind(), ErrorKind::Validation);
        }
        let mapped: InternalError =
            UseEmailVerificationError::Internal(sqlx::Error::RowNotFound).into();
        assert_eq!(mapped.kind(), ErrorKind::Storage);
    }

    #[apply(backends)]
    #[tokio::test]
    async fn create_email_verification_with_closed_pool_returns_error(#[case] backend: Backend) {
        let TestEnv { state, base } = backend.setup().await;
        base.close_pool().await;
        let expires_at = chrono::Utc::now();
        let email = parse_email("test@example.com");
        let result = state
            .email_verifications
            .create_email_verification(1, &email, expires_at)
            .await;
        assert!(result.is_err());
    }

    #[apply(backends)]
    #[tokio::test]
    async fn use_email_verification_with_closed_pool_returns_error(#[case] backend: Backend) {
        let TestEnv { state, base } = backend.setup().await;
        base.close_pool().await;
        let result = state
            .email_verifications
            .use_email_verification(&parse_raw_token("dGVzdA"))
            .await;
        assert!(matches!(result, Err(UseEmailVerificationError::NotFound)));
    }
}
