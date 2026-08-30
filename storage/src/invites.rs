//! Invite code storage.

use async_trait::async_trait;

use host::invite::InviteCode;
use sqlx::{Database, Pool};

use crate::WriteTransaction;
use crate::backend::Backend;
use crate::helpers;
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

/// Async operations on the `invites` table.
///
/// This trait manages the lifecycle of invite codes used for registration.
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
    helpers::InviteRow: for<'r> sqlx::FromRow<'r, DB::Row>,
    for<'q> i64: sqlx::Encode<'q, DB> + sqlx::Type<DB>,
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
        let code = host::invite::generate();
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
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::{Backend, TestEnv, backends};
    use chrono::Utc;
    use rstest::*;
    use rstest_reuse::*;
    use std::sync::Arc;

    #[apply(backends)]
    #[tokio::test]
    async fn create_invite_round_trips_the_code(#[case] backend: Backend) {
        // Keep the whole `TestEnv` bound: dropping `base` unlinks the SQLite file
        // (ADR-0053 TempDir hazard).
        let env = backend.setup().await;
        let expires_at = UtcInstant::from(Utc::now() + chrono::Duration::days(7));

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
        let code = match outcome {
            common::MutationOutcome::Confirmed(code) => code,
            common::MutationOutcome::CommitIndeterminate(_) => {
                panic!("invite fixture setup requires a confirmed commit")
            }
        };
        let invites = env.state.invites.list_invites().await.unwrap();

        assert_eq!(invites.len(), 1);
        assert_eq!(invites[0].code.as_ref(), code.as_ref());
    }

    #[apply(backends)]
    #[tokio::test]
    async fn list_invites_rejects_a_malformed_code_column(#[case] backend: Backend) {
        let TestEnv { state, base } = backend.setup().await;
        let now = UtcInstant::now();
        let expires_at = UtcInstant::from(now.value() + chrono::Duration::days(7));

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
            matches!(err, sqlx::Error::ColumnDecode { .. }),
            "expected a column-decode error, got: {err:?}"
        );
    }

    #[apply(backends)]
    #[tokio::test]
    async fn create_invite_with_closed_pool_returns_error(#[case] backend: Backend) {
        let TestEnv { state, base } = backend.setup().await;
        base.close_pool().await;
        let expires_at = UtcInstant::now();
        let invites = Arc::clone(&state.invites);
        let result = state
            .write_scope
            .run(|transaction| {
                Box::pin(async move { invites.create_invite(transaction, expires_at).await })
            })
            .await;
        assert!(matches!(
            result,
            Err(crate::WriteScopeError::Begin(sqlx::Error::PoolClosed))
        ));
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
