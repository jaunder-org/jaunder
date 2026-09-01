//! Invite code storage.

use async_trait::async_trait;
use chrono::Duration;
use thiserror::Error;

use host::{
    invite::{self, InviteCode},
    metrics,
    retention::Domain,
};
use sqlx::{Database, Pool};

use crate::WriteTransaction;
use crate::backend::Backend;
use crate::helpers::{self, InviteTokenStateRow, TokenState};
use crate::sql::RowCount;
use common::ids::UserId;
use common::time::UtcInstant;

/// An invite code record returned by [`InviteStorage`] queries.
#[derive(Clone, Debug)]
pub struct InviteRecord {
    /// The invite code.
    pub code: InviteCode,
    /// When the code was generated.
    pub created_at: UtcInstant,
    /// When the code will expire.
    pub expires_at: UtcInstant,
    /// When the code was consumed (None if still active).
    pub used_at: Option<UtcInstant>,
    /// ID of the user who was created using this code.
    pub used_by: Option<UserId>,
}

/// Errors returned while checking or conditionally claiming an invite.
#[derive(Debug, Error)]
pub enum UseInviteError {
    /// The invite code does not exist.
    #[error("invite code not found")]
    NotFound,
    /// The invite code has expired.
    #[error("invite code has expired")]
    Expired,
    /// The invite code has already been consumed.
    #[error("invite code has already been used")]
    AlreadyUsed,
    /// An unexpected database error occurred.
    #[error(transparent)]
    Internal(#[from] sqlx::Error),
}

/// Async operations on the `invites` table.
///
/// This trait manages the lifecycle of invite codes used for registration.
#[cfg_attr(feature = "test-utils", mockall::automock)]
#[async_trait]
pub trait InviteStorage: Send + Sync {
    /// Generates and stores a new invite code.
    ///
    /// Returns the generated [`InviteCode`].
    async fn create_invite(
        &self,
        transaction: &mut WriteTransaction,
        expires_at: UtcInstant,
    ) -> sqlx::Result<InviteCode>;

    /// Verifies that an invite is currently usable without acquiring a write
    /// capability. Account registration calls this before password preparation.
    async fn precheck_invite(&self, code: &InviteCode) -> Result<(), UseInviteError>;

    /// Conditionally consumes an invite and attributes it to `user_id`.
    ///
    /// The update rechecks that the invite remains unused and unexpired. A
    /// concurrent claimant receives [`UseInviteError::AlreadyUsed`].
    async fn claim_invite(
        &self,
        transaction: &mut WriteTransaction,
        code: &InviteCode,
        user_id: UserId,
    ) -> Result<(), UseInviteError>;

    /// Returns a list of all invite codes in the system.
    async fn list_invites(&self) -> sqlx::Result<Vec<InviteRecord>>;
    /// Deletes consumed invites whose consumption is at or before the supplied
    /// instant, and unused invites expired for at least 24 hours.
    ///
    /// Each call drains fixed-size batches at that instant, releasing the
    /// database connection after every statement.
    async fn prune_invites(&self, now: UtcInstant) -> sqlx::Result<u64>;
}

/// Generic [`InviteStorage`] backed by any [`Backend`] database.
///
/// Zero backend divergence (identical SQL across `SQLite` and Postgres),
/// so it is implemented once here; see ADR-0019.
pub struct InviteStore<DB: Database> {
    pool: Pool<DB>,
}

const PRUNE_BATCH_SIZE: i64 = 100;

impl<DB: Database> InviteStore<DB> {
    #[must_use]
    pub fn new(pool: Pool<DB>) -> Self {
        Self { pool }
    }
}

#[async_trait]
impl<DB> InviteStorage for InviteStore<DB>
where
    DB: Backend,
    helpers::InviteRow: for<'r> sqlx::FromRow<'r, DB::Row>,
    InviteTokenStateRow: for<'r> sqlx::FromRow<'r, DB::Row>,
    for<'q> i64: sqlx::Decode<'q, DB> + sqlx::Encode<'q, DB> + sqlx::Type<DB>,
    usize: sqlx::ColumnIndex<DB::Row>,
    for<'q> RowCount: sqlx::Decode<'q, DB> + sqlx::Type<DB>,
    // `InviteCode` binds/decodes as itself via the ADR-0071 sqlx bridge.
    String: sqlx::Type<DB>,
    for<'q> String: sqlx::Encode<'q, DB>,
    for<'q> UtcInstant: sqlx::Encode<'q, DB> + sqlx::Type<DB>,
    for<'c> &'c Pool<DB>: sqlx::Executor<'c, Database = DB>,
    for<'c> &'c mut DB::Connection: sqlx::Executor<'c, Database = DB>,
    for<'q> DB::Arguments<'q>: sqlx::IntoArguments<'q, DB>,
{
    async fn create_invite(
        &self,
        transaction: &mut WriteTransaction,
        expires_at: UtcInstant,
    ) -> sqlx::Result<InviteCode> {
        // Mint a typed `InviteCode` up front (infallible trusted door) and bind it
        // directly, so the code is a domain value end-to-end with no raw-`String` bind
        // and no fallible re-parse on the return (#438).
        let code = invite::generate();
        let now = UtcInstant::now();
        let connection = DB::write_connection(transaction)?;

        sqlx::query("INSERT INTO invites (code, created_at, expires_at) VALUES ($1, $2, $3)")
            .bind(&code)
            .bind(now)
            .bind(expires_at)
            .execute(&mut *connection)
            .await?;

        Ok(code)
    }

    async fn precheck_invite(&self, code: &InviteCode) -> Result<(), UseInviteError> {
        let now = UtcInstant::now();
        let row = sqlx::query_as::<_, InviteTokenStateRow>(
            "SELECT used_at, expires_at FROM invites WHERE code = $1",
        )
        .bind(code)
        .fetch_optional(&self.pool)
        .await?;

        match helpers::classify_invite_token_state(row, now) {
            TokenState::Missing => Err(UseInviteError::NotFound),
            TokenState::AlreadyUsed => Err(UseInviteError::AlreadyUsed),
            TokenState::Expired => Err(UseInviteError::Expired),
            TokenState::Claimable => Ok(()),
        }
    }

    async fn claim_invite(
        &self,
        transaction: &mut WriteTransaction,
        code: &InviteCode,
        user_id: UserId,
    ) -> Result<(), UseInviteError> {
        let now = UtcInstant::now();
        let connection = DB::write_connection(transaction)?;
        // RETURNING detects the conditional claim generically; sqlx exposes
        // `rows_affected` only on concrete backend result types.
        let claimed = sqlx::query_as::<_, InviteTokenStateRow>(
            "UPDATE invites SET used_at = $1, used_by = $2
             WHERE code = $3 AND used_at IS NULL AND expires_at > $1
             RETURNING used_at, expires_at",
        )
        .bind(now)
        .bind(user_id)
        .bind(code)
        .fetch_optional(&mut *connection)
        .await?;

        if claimed.is_some() {
            return Ok(());
        }

        let row = sqlx::query_as::<_, InviteTokenStateRow>(
            "SELECT used_at, expires_at FROM invites WHERE code = $1",
        )
        .bind(code)
        .fetch_optional(&mut *connection)
        .await?;
        match helpers::classify_invite_token_state(row, now) {
            TokenState::Missing => Err(UseInviteError::NotFound),
            TokenState::Expired => Err(UseInviteError::Expired),
            TokenState::AlreadyUsed | TokenState::Claimable => Err(UseInviteError::AlreadyUsed),
        }
    }

    async fn list_invites(&self) -> sqlx::Result<Vec<InviteRecord>> {
        let rows = sqlx::query_as::<_, helpers::InviteRow>(
            "SELECT code, created_at, expires_at, used_at, used_by FROM invites",
        )
        .fetch_all(&self.pool)
        .await?;

        // A corrupt/migrated `code` column is rejected as a decode error by the
        // `query_as` above (the sqlx bridge validates through `FromStr`), so building
        // the records here is infallible.
        Ok(rows
            .into_iter()
            .map(helpers::invite_record_from_row)
            .collect())
    }

    async fn prune_invites(&self, now: UtcInstant) -> sqlx::Result<u64> {
        let unused_cutoff = UtcInstant::from(now.value() - Duration::hours(24));
        let mut deleted = 0;

        loop {
            // A pool-executed statement acquires and releases its connection within
            // the batch, so a large retained backlog cannot hold a write lock.
            let batch = sqlx::query_scalar::<_, RowCount>(
                "DELETE FROM invites
                 WHERE code IN (
                     SELECT code FROM invites
                     WHERE (used_at IS NOT NULL AND used_at <= $1) OR expires_at <= $2
                     ORDER BY code
                     LIMIT $3
                 )
                 RETURNING CAST(1 AS BIGINT)",
            )
            .bind(now)
            .bind(unused_cutoff)
            .bind(PRUNE_BATCH_SIZE)
            .fetch_all(&self.pool)
            .await?
            .len() as u64;
            if batch > 0 {
                metrics::retention_pruned(Domain::Invites, batch);
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
    use crate::test_support::{Backend, TestEnv, backends, confirmed_for};
    use chrono::{Duration, Utc};
    use rstest::*;
    use rstest_reuse::*;
    use sqlx::Error as SqlxError;
    use std::sync::Arc;

    #[apply(backends)]
    #[tokio::test]
    async fn create_invite_round_trips_the_code(#[case] backend: Backend) {
        // Keep the whole `TestEnv` bound: dropping `base` unlinks the SQLite file
        // (ADR-0053 TempDir hazard).
        let env = backend.setup().await;
        let expires_at = UtcInstant::from(Utc::now() + Duration::days(7));

        // `create_invite` binds a typed `InviteCode`; `list_invites` decodes the
        // `code` column straight back into `InviteCode` — exercising both bridge
        // directions.
        let invites = Arc::clone(&env.state.invites);
        let outcome = env
            .state
            .write_scope
            .run(|transaction| {
                Box::pin(async move { invites.create_invite(transaction, expires_at).await })
            })
            .await
            .unwrap();
        let code = confirmed_for(outcome, "invite fixture setup");
        let invites = env.state.invites.list_invites().await.unwrap();

        assert_eq!(invites.len(), 1);
        assert_eq!(invites[0].code.as_ref(), code.as_ref());
    }

    #[apply(backends)]
    #[tokio::test]
    async fn list_invites_rejects_a_malformed_code_column(#[case] backend: Backend) {
        let TestEnv { state, base } = backend.setup().await;
        let now = UtcInstant::now();
        let expires_at = UtcInstant::from(now.value() + Duration::days(7));

        // Seed a row whose `code` column holds a value `InviteCode::from_str`
        // rejects (a space is not a base64url character), binding it as a raw `&str`
        // so the bad value actually lands in the column (the typed bind could not).
        let sql = "INSERT INTO invites (code, created_at, expires_at) VALUES ($1, $2, $3)";
        crate::with_closeable_pool!(base.pool(), pool, {
            sqlx::query(sql)
                .bind("bad code")
                .bind(now)
                .bind(expires_at)
                .execute(pool)
                .await
                .unwrap();
        });

        // The read decodes the `code` column into `InviteCode` via the sqlx bridge,
        // which validates through `FromStr`; the malformed value surfaces as a
        // `ColumnDecode` error rather than being silently admitted (covers the
        // bridge's `Decode` error arm).
        let err = state.invites.list_invites().await.unwrap_err();
        assert!(
            matches!(err, SqlxError::ColumnDecode { .. }),
            "expected a column-decode error, got: {err:?}"
        );
    }

    #[apply(backends)]
    #[tokio::test]
    async fn list_invites_with_closed_pool_returns_error(#[case] backend: Backend) {
        let TestEnv { state, base } = backend.setup().await;
        base.close_pool().await;
        let result = state.invites.list_invites().await;
        assert!(result.is_err());
    }

    #[apply(backends)]
    #[tokio::test]
    async fn prune_invites_removes_eligible_rows_without_touching_valid_invites(
        #[case] backend: Backend,
    ) {
        let env = backend.setup().await;
        let now: UtcInstant = "2050-01-02T03:04:05Z".parse().unwrap();
        let eligible_at = UtcInstant::from(now.value() - Duration::hours(24));
        let valid_until = UtcInstant::from(now.value() + Duration::hours(1));

        let invites = Arc::clone(&env.state.invites);
        env.state
            .write_scope
            .run(|transaction| {
                Box::pin(async move { invites.create_invite(transaction, eligible_at).await })
            })
            .await
            .unwrap();
        let invites = Arc::clone(&env.state.invites);
        env.state
            .write_scope
            .run(|transaction| {
                Box::pin(async move { invites.create_invite(transaction, valid_until).await })
            })
            .await
            .unwrap();

        assert_eq!(env.state.invites.prune_invites(now).await.unwrap(), 1);
        let invites = env.state.invites.list_invites().await.unwrap();
        assert_eq!(invites.len(), 1);
        assert_eq!(invites[0].expires_at, valid_until);
    }

    #[apply(backends)]
    #[tokio::test]
    async fn prune_invites_uses_the_supplied_instant_for_consumed_rows(#[case] backend: Backend) {
        let TestEnv { state, base } = backend.setup().await;
        let now: UtcInstant = "2050-01-02T03:04:05Z".parse().unwrap();
        let valid_until = UtcInstant::from(now.value() + Duration::hours(1));
        let invites = Arc::clone(&state.invites);
        let outcome = state
            .write_scope
            .run(|transaction| {
                Box::pin(async move { invites.create_invite(transaction, valid_until).await })
            })
            .await
            .unwrap();
        let boundary_code = confirmed_for(outcome, "boundary invite fixture");
        let invites = Arc::clone(&state.invites);
        let outcome = state
            .write_scope
            .run(|transaction| {
                Box::pin(async move { invites.create_invite(transaction, valid_until).await })
            })
            .await
            .unwrap();
        let future_code = confirmed_for(outcome, "future invite fixture");

        crate::with_closeable_pool!(base.pool(), pool, {
            sqlx::query("UPDATE invites SET used_at = $1 WHERE code = $2")
                .bind(now)
                .bind(&boundary_code)
                .execute(pool)
                .await
                .unwrap();
            sqlx::query("UPDATE invites SET used_at = $1 WHERE code = $2")
                .bind(valid_until)
                .bind(&future_code)
                .execute(pool)
                .await
                .unwrap();
        });

        assert_eq!(state.invites.prune_invites(now).await.unwrap(), 1);
        let invites = state.invites.list_invites().await.unwrap();
        assert_eq!(invites.len(), 1);
        assert_eq!(invites[0].code.as_ref(), future_code.as_ref());

        base.close_pool().await;
        assert!(state.invites.prune_invites(now).await.is_err());
    }
}
