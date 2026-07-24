//! Email verification token storage.

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use sqlx::{Database, Pool};
use thiserror::Error;

use crate::backend::Backend;
use common::email::Email;
use common::ids::UserId;
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
        user_id: UserId,
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
    ) -> Result<(UserId, Email), UseEmailVerificationError>;
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
    (i64, Email): for<'r> sqlx::FromRow<'r, DB::Row>,
    (Option<DateTime<Utc>>, DateTime<Utc>): for<'r> sqlx::FromRow<'r, DB::Row>,
    for<'q> i64: sqlx::Encode<'q, DB> + sqlx::Type<DB>,
    // `TokenHash` binds and `Email` binds/decodes as themselves via the sqlx
    // bridge (#438), which delegates to `String`; these bounds make that bridge
    // available on the generic backend (the `(i64, Email): FromRow` bound above
    // threads the `Email` decode).
    String: sqlx::Type<DB>,
    for<'q> String: sqlx::Encode<'q, DB>,
    for<'q> DateTime<Utc>: sqlx::Encode<'q, DB> + sqlx::Type<DB>,
    for<'c> &'c Pool<DB>: sqlx::Executor<'c, Database = DB>,
    for<'c> &'c mut DB::Connection: sqlx::Executor<'c, Database = DB>,
    for<'q> DB::Arguments<'q>: sqlx::IntoArguments<'q, DB>,
{
    async fn create_email_verification(
        &self,
        user_id: UserId,
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
        .bind(i64::from(user_id))
        .bind(now)
        .execute(&mut *tx)
        .await?;

        sqlx::query(
            "INSERT INTO email_verifications
             (token_hash, user_id, email, created_at, expires_at)
             VALUES ($1, $2, $3, $4, $5)",
        )
        .bind(token_hash)
        .bind(i64::from(user_id))
        .bind(email)
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
    ) -> Result<(UserId, Email), UseEmailVerificationError> {
        let token_hash =
            host::token::hash(raw_token).map_err(|_| UseEmailVerificationError::NotFound)?;

        let now = Utc::now();

        // Atomically claim the token: the UPDATE succeeds only when the token
        // exists, has not yet been used, and has not expired.  This single
        // statement is the "claim" — no separate read is needed first, so two
        // concurrent requests cannot both succeed.  RETURNING gives us the
        // data we need without a second round-trip.
        // The `email` column decodes straight into `Email` via the sqlx bridge
        // (#438), which validates through `FromStr`. A genuine storage fault (e.g.
        // a closed pool) still maps to `NotFound` as before, but a corrupt/migrated
        // `email` value is a data-integrity fault, so its `ColumnDecode` error is
        // surfaced as `Internal` — mirroring the pre-bridge hand-parse, which also
        // reported a corrupt value as an internal decode error.
        let claimed = sqlx::query_as::<_, (i64, Email)>(
            "UPDATE email_verifications SET used_at = $1
             WHERE token_hash = $2 AND used_at IS NULL AND expires_at > $3
             RETURNING user_id, email",
        )
        .bind(now)
        .bind(&token_hash)
        .bind(now)
        .fetch_optional(&self.pool)
        .await
        .map_err(|e| match e {
            sqlx::Error::ColumnDecode { .. } => UseEmailVerificationError::Internal(e),
            _ => UseEmailVerificationError::NotFound,
        })?;

        if let Some((user_id, email)) = claimed {
            return Ok((UserId::from(user_id), email));
        }

        // Zero rows affected — inspect the row to return the right error.
        let row = sqlx::query_as::<_, (Option<DateTime<Utc>>, DateTime<Utc>)>(
            "SELECT used_at, expires_at FROM email_verifications WHERE token_hash = $1",
        )
        .bind(&token_hash)
        .fetch_optional(&self.pool)
        .await
        .map_err(|_| UseEmailVerificationError::NotFound)?;

        Err(crate::helpers::email_verification_claim_error(row))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::{backends, Backend, CloseablePool, SeedUser, TestEnv};
    use common::test_support::{parse_email, parse_raw_token};
    use rstest::*;
    use rstest_reuse::*;

    #[apply(backends)]
    #[tokio::test]
    async fn email_verification_round_trips_user_and_email(#[case] backend: Backend) {
        // Keep the whole `TestEnv` bound: dropping `base` unlinks the SQLite file
        // (ADR-0053 TempDir hazard).
        let env = backend.setup().await;
        let user_id = SeedUser::new("testuser").seed(&env.state).await;
        let email = parse_email("alice@example.com");
        let expires_at = chrono::Utc::now() + chrono::Duration::hours(1);

        // `create_email_verification` binds the `TokenHash` and the `Email`;
        // `use_email_verification` re-binds the hash to claim the row and decodes
        // the `email` column straight back into `Email` via the sqlx bridge (#438).
        let raw_token = env
            .state
            .email_verifications
            .create_email_verification(user_id, &email, expires_at)
            .await
            .unwrap();

        let (claimed_user, claimed_email) = env
            .state
            .email_verifications
            .use_email_verification(&raw_token)
            .await
            .unwrap();
        assert_eq!(claimed_user, user_id);
        assert_eq!(claimed_email, email);
    }

    #[apply(backends)]
    #[tokio::test]
    async fn use_email_verification_with_corrupt_email_column_returns_internal(
        #[case] backend: Backend,
    ) {
        let env = backend.setup().await;
        let user_id = SeedUser::new("testuser").seed(&env.state).await;
        let email = parse_email("alice@example.com");
        let expires_at = chrono::Utc::now() + chrono::Duration::hours(1);
        let raw_token = env
            .state
            .email_verifications
            .create_email_verification(user_id, &email, expires_at)
            .await
            .unwrap();

        // Overwrite the `email` column with a value `Email::from_str` rejects,
        // binding it as a raw `&str` so the bad value actually lands in the column.
        let sql = "UPDATE email_verifications SET email = $1";
        match env.base.pool() {
            CloseablePool::Sqlite(pool) => {
                sqlx::query(sql)
                    .bind("not-an-email")
                    .execute(pool)
                    .await
                    .unwrap();
            }
            CloseablePool::Postgres(pool) => {
                sqlx::query(sql)
                    .bind("not-an-email")
                    .execute(pool)
                    .await
                    .unwrap();
            }
        }

        // The claim query decodes the `email` column into `Email` via the sqlx
        // bridge; a corrupt value is a data-integrity fault, surfaced as
        // `Internal(ColumnDecode)` — distinct from the not-found path (covers the
        // decode arm of the claim query's error mapping).
        let err = env
            .state
            .email_verifications
            .use_email_verification(&raw_token)
            .await
            .unwrap_err();
        assert!(
            matches!(
                err,
                UseEmailVerificationError::Internal(sqlx::Error::ColumnDecode { .. })
            ),
            "expected Internal(ColumnDecode), got: {err:?}"
        );
    }

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
            .create_email_verification(UserId::from(1), &email, expires_at)
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
