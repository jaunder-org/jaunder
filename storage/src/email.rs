//! Email verification token storage.

use crate::WriteTransaction;
use crate::sql::QueryStorageExt;
use async_trait::async_trait;
use chrono::Duration;

use sqlx::{Database, Pool};
use thiserror::Error;

use crate::backend::Backend;
use crate::helpers::{self, TokenStateRow};
use crate::sql::RowCount;
use common::email::Email;
use common::ids::UserId;
use common::pagination::RowLimit;
use common::time::UtcInstant;
use common::token::RawToken;
use host::{metrics, retention::Domain, token};
/// Test-only invalid `email_verifications.email` column value.
///
/// Fixtures use this role to bypass `Email` validation deliberately without
/// admitting arbitrary text to production storage interfaces.
#[cfg(test)]
#[derive(macros::SqlxBridge)]
pub(crate) struct CorruptEmailAddress(String);

#[cfg(test)]
impl CorruptEmailAddress {
    fn malformed() -> Self {
        Self("not-an-email".to_owned())
    }
}

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
    /// Mirrors the password-reset mapping so `verify_email` is `?`-liftable:
    /// the three token failures are client validation errors (a stale/used/unknown
    /// verification link), and an internal failure is a masked storage error.
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
/// The email-verification consumption that must be observed only after commit
/// confirmation.
#[derive(Debug)]
pub struct EmailVerificationConsumption {
    pub user_id: UserId,
    pub email: Email,
}

/// Storage for email verification tokens.
///
/// This trait manages the lifecycle of tokens sent to users to verify their
/// email addresses.
#[cfg_attr(feature = "test-utils", mockall::automock)]
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
        transaction: &mut WriteTransaction,
        user_id: UserId,
        email: &Email,
        expires_at: UtcInstant,
    ) -> sqlx::Result<RawToken>;

    /// Validates a raw verification token and marks it as used.
    ///
    /// Returns the consumption on success. Its caller must log the consumption
    /// only after the enclosing transaction is confirmed committed.
    ///
    /// # Errors
    ///
    /// Returns [`UseEmailVerificationError`] if the token is invalid, expired,
    /// or already used.
    async fn use_email_verification(
        &self,
        transaction: &mut WriteTransaction,
        raw_token: &RawToken,
    ) -> Result<EmailVerificationConsumption, UseEmailVerificationError>;
    /// Deletes consumed verification tokens whose consumption is at or before
    /// the supplied instant, and unused tokens expired for at least 24 hours,
    /// draining bounded batches at that instant.
    async fn prune_email_verifications(&self, now: UtcInstant) -> sqlx::Result<u64>;
}

/// Generic [`EmailVerificationStorage`] backed by any [`Backend`] database.
///
/// Zero backend divergence (identical SQL across `SQLite` and Postgres),
/// so it is implemented once here; see ADR-0019.
const PRUNE_BATCH_SIZE: RowLimit = RowLimit::at_most(100);

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
    (UserId, Email): for<'r> sqlx::FromRow<'r, DB::Row>,
    TokenStateRow: for<'r> sqlx::FromRow<'r, DB::Row>,
    for<'q> i64: sqlx::Decode<'q, DB> + sqlx::Encode<'q, DB> + sqlx::Type<DB>,
    usize: sqlx::ColumnIndex<DB::Row>,
    for<'q> RowCount: sqlx::Decode<'q, DB> + sqlx::Type<DB>,
    // `TokenHash` binds and `Email` binds/decodes as themselves via the ADR-0071
    // sqlx bridge (the `(UserId, Email): FromRow` bound above threads the `Email`
    // decode).
    String: sqlx::Type<DB>,
    for<'q> String: sqlx::Encode<'q, DB>,
    for<'q> UtcInstant: sqlx::Encode<'q, DB> + sqlx::Type<DB>,
    for<'c> &'c Pool<DB>: sqlx::Executor<'c, Database = DB>,
    for<'c> &'c mut DB::Connection: sqlx::Executor<'c, Database = DB>,
    for<'q> DB::Arguments<'q>: sqlx::IntoArguments<'q, DB>,
{
    async fn create_email_verification(
        &self,
        transaction: &mut WriteTransaction,
        user_id: UserId,
        email: &Email,
        expires_at: UtcInstant,
    ) -> sqlx::Result<RawToken> {
        let (raw_token, token_hash) = token::generate_hashed();
        let now = UtcInstant::now();
        let connection = DB::write_connection(transaction)?;

        // Supersede any existing pending token for this user by setting its
        // expires_at to its created_at, making it appear immediately expired.
        sqlx::query(
            "UPDATE email_verifications
             SET expires_at = created_at
             WHERE user_id = $1 AND used_at IS NULL AND expires_at > $2",
        )
        .bind_storage(user_id)
        .bind_storage(now)
        .execute(&mut *connection)
        .await?;

        sqlx::query(
            "INSERT INTO email_verifications
             (token_hash, user_id, email, created_at, expires_at)
             VALUES ($1, $2, $3, $4, $5)",
        )
        .bind_storage(token_hash)
        .bind_storage(user_id)
        .bind_storage(email)
        .bind_storage(now)
        .bind_storage(expires_at)
        .execute(&mut *connection)
        .await?;

        Ok(raw_token)
    }

    async fn use_email_verification(
        &self,
        transaction: &mut WriteTransaction,
        raw_token: &RawToken,
    ) -> Result<EmailVerificationConsumption, UseEmailVerificationError> {
        let token_hash = token::hash(raw_token).map_err(|_| UseEmailVerificationError::NotFound)?;

        let now = UtcInstant::now();

        // Atomically claim the token: the UPDATE succeeds only when the token
        // exists, has not yet been used, and has not expired. This single
        // statement is the claim, so two concurrent requests cannot both
        // succeed. RETURNING supplies the verified address without a second
        // round-trip. SQL and decode failures are infrastructure failures;
        // only a successful `Ok(None)` is a domain miss to disambiguate below.
        let connection =
            DB::write_connection(transaction).map_err(UseEmailVerificationError::Internal)?;
        let claimed = sqlx::query_as::<_, (UserId, Email)>(
            "UPDATE email_verifications SET used_at = $1
             WHERE token_hash = $2 AND used_at IS NULL AND expires_at > $3
             RETURNING user_id, email",
        )
        .bind_storage(now)
        .bind_storage(&token_hash)
        .bind_storage(now)
        .fetch_optional(&mut *connection)
        .await
        .map_err(UseEmailVerificationError::Internal)?;

        if let Some((user_id, email)) = claimed {
            return Ok(EmailVerificationConsumption { user_id, email });
        }

        // A successful claim miss is the only path that reaches the domain
        // classifier. Failure to read the row remains an infrastructure error.
        let row = sqlx::query_as::<_, TokenStateRow>(
            "SELECT used_at, expires_at FROM email_verifications WHERE token_hash = $1",
        )
        .bind_storage(&token_hash)
        .fetch_optional(&self.pool)
        .await
        .map_err(UseEmailVerificationError::Internal)?;
        Err(helpers::email_verification_claim_error(row, now))
    }

    async fn prune_email_verifications(&self, now: UtcInstant) -> sqlx::Result<u64> {
        let unused_cutoff = UtcInstant::from(now.value() - Duration::hours(24));
        let mut deleted = 0;

        loop {
            // Executing through the pool confines each deletion batch to one
            // statement and releases its connection before the next batch.
            let batch = sqlx::query_scalar::<_, RowCount>(
                "DELETE FROM email_verifications
                 WHERE token_hash IN (
                     SELECT token_hash FROM email_verifications
                     WHERE (used_at IS NOT NULL AND used_at <= $1) OR expires_at <= $2
                     ORDER BY token_hash
                     LIMIT $3
                 )
                 RETURNING CAST(1 AS BIGINT)",
            )
            .bind_storage(now)
            .bind_storage(unused_cutoff)
            .bind_storage(PRUNE_BATCH_SIZE)
            .fetch_all(&self.pool)
            .await?
            .len() as u64;
            if batch > 0 {
                metrics::retention_pruned(Domain::EmailVerifications, batch);
            }
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
    use crate::test_support::{Backend, SeedUser, TestEnv, backends, confirmed_for};
    use chrono::Duration;
    use common::test_support::parse_email;
    use host::token;
    use rstest::*;
    use rstest_reuse::*;
    use sqlx::Error as SqlxError;
    use std::sync::Arc;

    #[apply(backends)]
    #[tokio::test]
    async fn email_verification_round_trips_user_and_email(#[case] backend: Backend) {
        // Keep the whole `TestEnv` bound: dropping `base` unlinks the SQLite file
        // (ADR-0053 TempDir hazard).
        let env = backend.setup().await;
        let user_id = SeedUser::new().seed(&env.state).await.user_id;
        let email = parse_email("alice@example.com");
        let expires_at: UtcInstant = "2099-01-02T03:04:05.123456Z".parse().unwrap();

        // `create_email_verification` binds the `TokenHash` and the `Email`;
        // `use_email_verification` re-binds the hash to claim the row and decodes
        // the `email` column straight back into `Email` via the sqlx bridge (#438).
        let email_verifications = Arc::clone(&env.state.email_verifications);
        let verification_email = email.clone();
        let outcome = env
            .state
            .write_scope
            .run(|transaction| {
                Box::pin(async move {
                    email_verifications
                        .create_email_verification(
                            transaction,
                            user_id,
                            &verification_email,
                            expires_at,
                        )
                        .await
                })
            })
            .await
            .unwrap();
        let raw_token = confirmed_for(outcome, "email-verification fixture setup");

        let email_verifications = Arc::clone(&env.state.email_verifications);
        let outcome = env
            .state
            .write_scope
            .run(|transaction| {
                Box::pin(async move {
                    email_verifications
                        .use_email_verification(transaction, &raw_token)
                        .await
                })
            })
            .await
            .unwrap();
        let consumption = confirmed_for(outcome, "email verification");
        assert_eq!(consumption.user_id, user_id);
        assert_eq!(consumption.email, email);
    }

    #[apply(backends)]
    #[tokio::test]
    async fn use_email_verification_with_corrupt_email_column_returns_internal(
        #[case] backend: Backend,
    ) {
        let env = backend.setup().await;
        let user_id = SeedUser::new().seed(&env.state).await.user_id;
        let email = parse_email("alice@example.com");
        let expires_at: UtcInstant = "2099-01-02T03:04:05.123456Z".parse().unwrap();
        let email_verifications = Arc::clone(&env.state.email_verifications);
        let outcome = env
            .state
            .write_scope
            .run(|transaction| {
                Box::pin(async move {
                    email_verifications
                        .create_email_verification(transaction, user_id, &email, expires_at)
                        .await
                })
            })
            .await
            .unwrap();
        let raw_token = confirmed_for(outcome, "email-verification fixture setup");

        // Overwrite the `email` column with a value `Email::from_str` rejects.
        let sql = "UPDATE email_verifications SET email = $1";
        crate::with_closeable_pool!(env.base.pool(), pool, {
            sqlx::query(sql)
                .bind_storage(CorruptEmailAddress::malformed())
                .execute(pool)
                .await
                .unwrap();
        });

        // The claim query decodes the `email` column into `Email` via the sqlx
        // bridge; a corrupt value is a data-integrity fault, surfaced as
        // `Internal(ColumnDecode)` — distinct from the not-found path (covers the
        // decode arm of the claim query's error mapping).
        let email_verifications = Arc::clone(&env.state.email_verifications);
        let err = env
            .state
            .write_scope
            .run(|transaction| {
                Box::pin(async move {
                    email_verifications
                        .use_email_verification(transaction, &raw_token)
                        .await
                })
            })
            .await
            .unwrap_err();
        assert!(
            matches!(
                err,
                crate::WriteScopeError::Operation(UseEmailVerificationError::Internal(
                    SqlxError::ColumnDecode { .. }
                ))
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
            UseEmailVerificationError::Internal(SqlxError::RowNotFound).into();
        assert_eq!(mapped.kind(), ErrorKind::Storage);
    }

    #[apply(backends)]
    #[tokio::test]
    async fn prune_email_verifications_uses_the_supplied_instant_for_consumed_rows(
        #[case] backend: Backend,
    ) {
        let env = backend.setup().await;
        let user_id = SeedUser::new().seed(&env.state).await.user_id;
        let email = parse_email("alice@example.com");
        let now: UtcInstant = "2050-01-02T03:04:05Z".parse().unwrap();
        let expired_at = UtcInstant::from(now.value() - Duration::hours(24));
        let valid_until = UtcInstant::from(now.value() + Duration::hours(1));

        let expired_email = email.clone();
        let email_verifications = Arc::clone(&env.state.email_verifications);
        let outcome = env
            .state
            .write_scope
            .run(|transaction| {
                Box::pin(async move {
                    email_verifications
                        .create_email_verification(transaction, user_id, &expired_email, expired_at)
                        .await
                })
            })
            .await
            .unwrap();
        let expired_token = confirmed_for(outcome, "expired email-verification fixture");

        let boundary_email = email.clone();
        let email_verifications = Arc::clone(&env.state.email_verifications);
        let outcome = env
            .state
            .write_scope
            .run(|transaction| {
                Box::pin(async move {
                    email_verifications
                        .create_email_verification(
                            transaction,
                            user_id,
                            &boundary_email,
                            valid_until,
                        )
                        .await
                })
            })
            .await
            .unwrap();
        let boundary_token = confirmed_for(outcome, "boundary email-verification fixture");
        let boundary_hash = token::hash(&boundary_token).unwrap();

        let email_verifications = Arc::clone(&env.state.email_verifications);
        let outcome = env
            .state
            .write_scope
            .run(|transaction| {
                Box::pin(async move {
                    email_verifications
                        .create_email_verification(transaction, user_id, &email, valid_until)
                        .await
                })
            })
            .await
            .unwrap();
        let future_token = confirmed_for(outcome, "future email-verification fixture");
        let future_hash = token::hash(&future_token).unwrap();

        crate::with_closeable_pool!(env.base.pool(), pool, {
            sqlx::query("UPDATE email_verifications SET used_at = $1 WHERE token_hash = $2")
                .bind(now)
                .bind(boundary_hash)
                .execute(pool)
                .await
                .unwrap();
            sqlx::query("UPDATE email_verifications SET used_at = $1 WHERE token_hash = $2")
                .bind(valid_until)
                .bind(future_hash)
                .execute(pool)
                .await
                .unwrap();
        });

        assert_eq!(
            env.state
                .email_verifications
                .prune_email_verifications(now)
                .await
                .unwrap(),
            2
        );

        let email_verifications = Arc::clone(&env.state.email_verifications);
        let expired = env
            .state
            .write_scope
            .run(|transaction| {
                Box::pin(async move {
                    email_verifications
                        .use_email_verification(transaction, &expired_token)
                        .await
                })
            })
            .await
            .unwrap_err();
        assert!(matches!(
            expired,
            crate::WriteScopeError::Operation(UseEmailVerificationError::NotFound)
        ));

        let email_verifications = Arc::clone(&env.state.email_verifications);
        let boundary = env
            .state
            .write_scope
            .run(|transaction| {
                Box::pin(async move {
                    email_verifications
                        .use_email_verification(transaction, &boundary_token)
                        .await
                })
            })
            .await
            .unwrap_err();
        assert!(matches!(
            boundary,
            crate::WriteScopeError::Operation(UseEmailVerificationError::NotFound)
        ));

        let email_verifications = Arc::clone(&env.state.email_verifications);
        let future = env
            .state
            .write_scope
            .run(|transaction| {
                Box::pin(async move {
                    email_verifications
                        .use_email_verification(transaction, &future_token)
                        .await
                })
            })
            .await
            .unwrap_err();
        assert!(matches!(
            future,
            crate::WriteScopeError::Operation(UseEmailVerificationError::AlreadyUsed)
        ));
    }

    #[apply(backends)]
    #[tokio::test]
    async fn prune_email_verifications_with_closed_pool_returns_error(#[case] backend: Backend) {
        let TestEnv { state, base } = backend.setup().await;
        base.close_pool().await;
        assert!(
            state
                .email_verifications
                .prune_email_verifications(UtcInstant::now())
                .await
                .is_err()
        );
    }
}
