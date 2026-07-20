//! Invite code storage.

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use host::invite::InviteCode;
use sqlx::{Database, Pool};
use thiserror::Error;

use crate::backend::Backend;
use common::ids::UserId;

/// An invite code record returned by [`InviteStorage`] queries.
#[derive(Clone, Debug)]
pub struct InviteRecord {
    /// The invite code.
    pub code: InviteCode,
    /// When the code was generated.
    pub created_at: DateTime<Utc>,
    /// When the code will expire.
    pub expires_at: DateTime<Utc>,
    /// When the code was consumed (None if still active).
    pub used_at: Option<DateTime<Utc>>,
    /// ID of the user who was created using this code.
    pub used_by: Option<UserId>,
}

/// Errors that can occur when consuming an invite code.
#[derive(Debug, Error)]
pub enum UseInviteError {
    /// The invite code does not exist.
    #[error("invite code not found")]
    NotFound,
    /// The invite code has passed its expiration date.
    #[error("invite code has expired")]
    Expired,
    /// The invite code has already been consumed.
    #[error("invite code has already been used")]
    AlreadyUsed,
}

/// Async operations on the `invites` table.
///
/// This trait manages the lifecycle of invite codes used for registration.
#[async_trait]
pub trait InviteStorage: Send + Sync {
    /// Generates and stores a new invite code.
    ///
    /// Returns the generated [`InviteCode`].
    async fn create_invite(&self, expires_at: DateTime<Utc>) -> sqlx::Result<InviteCode>;

    /// Marks an invite code as used by a specific user.
    ///
    /// # Errors
    ///
    /// Returns [`UseInviteError`] if the code is invalid, expired, or already used.
    async fn use_invite(&self, code: &InviteCode, user_id: UserId) -> Result<(), UseInviteError>;

    /// Returns a list of all invite codes in the system.
    async fn list_invites(&self) -> sqlx::Result<Vec<InviteRecord>>;
}

/// Generic [`InviteStorage`] backed by any [`Backend`] database.
///
/// Zero backend divergence (identical SQL across `SQLite` and Postgres),
/// so it is implemented once here; see ADR-0019.
pub struct InviteStore<DB: Database> {
    pool: Pool<DB>,
}

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
    crate::helpers::InviteRow: for<'r> sqlx::FromRow<'r, DB::Row>,
    for<'q> i64: sqlx::Encode<'q, DB> + sqlx::Type<DB>,
    // `InviteCode` binds/decodes as itself via the sqlx bridge (#438), which delegates
    // to `String`; these bounds make that bridge available on the generic backend.
    String: sqlx::Type<DB>,
    for<'q> String: sqlx::Encode<'q, DB>,
    for<'q> DateTime<Utc>: sqlx::Encode<'q, DB> + sqlx::Type<DB>,
    for<'c> &'c Pool<DB>: sqlx::Executor<'c, Database = DB>,
    for<'c> &'c mut DB::Connection: sqlx::Executor<'c, Database = DB>,
    for<'q> DB::Arguments<'q>: sqlx::IntoArguments<'q, DB>,
{
    async fn create_invite(&self, expires_at: DateTime<Utc>) -> sqlx::Result<InviteCode> {
        // Mint a typed `InviteCode` up front (infallible trusted door) and bind it
        // directly, so the code is a domain value end-to-end with no raw-`String` bind
        // and no fallible re-parse on the return (#438).
        let code = host::invite::generate();
        let now = Utc::now();

        sqlx::query("INSERT INTO invites (code, created_at, expires_at) VALUES ($1, $2, $3)")
            .bind(&code)
            .bind(now)
            .bind(expires_at)
            .execute(&self.pool)
            .await?;

        Ok(code)
    }

    async fn use_invite(&self, code: &InviteCode, user_id: UserId) -> Result<(), UseInviteError> {
        let now = Utc::now();

        // Atomically claim the invite in one statement: the UPDATE succeeds only
        // when the invite exists, is unused, and has not expired. No prior read
        // is needed, so two concurrent requests cannot both succeed and the
        // SQLite read-then-write lock upgrade (ADR-0021) is avoided.
        let claimed = sqlx::query_as::<_, crate::helpers::InviteRow>(
            "UPDATE invites SET used_at = $1, used_by = $2 \
             WHERE code = $3 AND used_at IS NULL AND expires_at > $4 \
             RETURNING code, created_at, expires_at, used_at, used_by",
        )
        .bind(now)
        .bind(i64::from(user_id))
        .bind(code)
        .bind(now)
        .fetch_optional(&self.pool)
        .await
        .map_err(|_| UseInviteError::NotFound)?;

        if claimed.is_some() {
            return Ok(());
        }

        // Zero rows affected — read the row to return the precise error.
        let row = sqlx::query_as::<_, crate::helpers::InviteRow>(
            "SELECT code, created_at, expires_at, used_at, used_by \
             FROM invites WHERE code = $1",
        )
        .bind(code)
        .fetch_optional(&self.pool)
        .await
        .map_err(|_| UseInviteError::NotFound)?
        .ok_or(UseInviteError::NotFound)?;

        let record = crate::helpers::invite_record_from_row(row);
        if record.used_at.is_some() {
            return Err(UseInviteError::AlreadyUsed);
        }
        // Present and unused but the claim failed ⇒ expired.
        Err(UseInviteError::Expired)
    }

    async fn list_invites(&self) -> sqlx::Result<Vec<InviteRecord>> {
        let rows = sqlx::query_as::<_, crate::helpers::InviteRow>(
            "SELECT code, created_at, expires_at, used_at, used_by FROM invites",
        )
        .fetch_all(&self.pool)
        .await?;

        // A corrupt/migrated `code` column is rejected as a decode error by the
        // `query_as` above (the sqlx bridge validates through `FromStr`), so building
        // the records here is infallible.
        Ok(rows
            .into_iter()
            .map(crate::helpers::invite_record_from_row)
            .collect())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::{backends, Backend, CloseablePool, TestEnv};
    use rstest::*;
    use rstest_reuse::*;

    #[apply(backends)]
    #[tokio::test]
    async fn create_invite_round_trips_the_code(#[case] backend: Backend) {
        // Keep the whole `TestEnv` bound: dropping `base` unlinks the SQLite file
        // (ADR-0053 TempDir hazard).
        let env = backend.setup().await;
        let expires_at = Utc::now() + chrono::Duration::days(7);

        // `create_invite` binds a typed `InviteCode`; `list_invites` decodes the
        // `code` column straight back into `InviteCode` — exercising both bridge
        // directions.
        let code = env.state.invites.create_invite(expires_at).await.unwrap();
        let invites = env.state.invites.list_invites().await.unwrap();

        assert_eq!(invites.len(), 1);
        assert_eq!(invites[0].code.as_ref(), code.as_ref());
    }

    #[apply(backends)]
    #[tokio::test]
    async fn list_invites_rejects_a_malformed_code_column(#[case] backend: Backend) {
        let TestEnv { state, base } = backend.setup().await;
        let now = Utc::now();
        let expires_at = now + chrono::Duration::days(7);

        // Seed a row whose `code` column holds a value `InviteCode::from_str`
        // rejects (a space is not a base64url character), binding it as a raw `&str`
        // so the bad value actually lands in the column (the typed bind could not).
        let sql = "INSERT INTO invites (code, created_at, expires_at) VALUES ($1, $2, $3)";
        match base.pool() {
            CloseablePool::Sqlite(pool) => {
                sqlx::query(sql)
                    .bind("bad code")
                    .bind(now)
                    .bind(expires_at)
                    .execute(pool)
                    .await
                    .unwrap();
            }
            CloseablePool::Postgres(pool) => {
                sqlx::query(sql)
                    .bind("bad code")
                    .bind(now)
                    .bind(expires_at)
                    .execute(pool)
                    .await
                    .unwrap();
            }
        }

        // The read decodes the `code` column into `InviteCode` via the sqlx bridge,
        // which validates through `FromStr`; the malformed value surfaces as a
        // column-decode error rather than being silently admitted (covers the bridge's
        // `Decode` error arm). `query_as` reports a failed column decode as
        // `ColumnDecode`, not the hand-rolled `Decode` the old re-parse produced.
        let err = state.invites.list_invites().await.unwrap_err();
        assert!(
            matches!(err, sqlx::Error::ColumnDecode { .. }),
            "expected a column-decode error, got: {err:?}"
        );
    }

    #[apply(backends)]
    #[tokio::test]
    async fn create_invite_with_closed_pool_returns_error(#[case] backend: Backend) {
        let TestEnv { state, base } = backend.setup().await;
        base.close_pool().await;
        let expires_at = chrono::Utc::now();
        let result = state.invites.create_invite(expires_at).await;
        assert!(result.is_err());
    }

    #[apply(backends)]
    #[tokio::test]
    async fn use_invite_with_closed_pool_returns_error(#[case] backend: Backend) {
        let TestEnv { state, base } = backend.setup().await;
        base.close_pool().await;
        let code = "code".parse::<InviteCode>().unwrap();
        let result = state.invites.use_invite(&code, UserId::from(1)).await;
        assert!(matches!(result, Err(UseInviteError::NotFound)));
    }

    #[apply(backends)]
    #[tokio::test]
    async fn list_invites_with_closed_pool_returns_error(#[case] backend: Backend) {
        let TestEnv { state, base } = backend.setup().await;
        base.close_pool().await;
        let result = state.invites.list_invites().await;
        assert!(result.is_err());
    }
}
