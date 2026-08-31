//! Session and device token storage.

use async_trait::async_trait;

use thiserror::Error;

use crate::WriteTransaction;
use common::ids::UserId;
use common::session_label::SessionLabel;
use common::time::UtcInstant;
use common::token::{RawToken, TokenHash};
use common::username::Username;
use host::token;

/// A session record returned by [`SessionStorage`] queries.
#[derive(Clone, Debug)]
pub struct SessionRecord {
    /// SHA-256 hash of the session token.
    pub token_hash: TokenHash,
    /// ID of the user associated with this session.
    pub user_id: UserId,
    /// Username at the time of session creation.
    pub username: Username,
    /// Validated label for the device/client (e.g., "Mobile App", "Safari on
    /// macOS", "Sign-up session").
    pub label: SessionLabel,
    /// When the session was first created.
    pub created_at: UtcInstant,
    /// When the session was last persisted as used to authenticate a request.
    ///
    /// This is operator-facing metadata with bounded staleness: authentication
    /// may skip updating it for up to 60 seconds while the stored value is fresh.
    pub last_used_at: UtcInstant,
}

/// Errors that can occur when authenticating a session token.
#[derive(Debug, Error)]
pub enum SessionAuthError {
    /// The token is malformed or invalid.
    #[error("invalid token")]
    InvalidToken,
    /// No active session matches the provided token.
    #[error("session not found")]
    SessionNotFound,
    /// An unexpected database error occurred.
    #[error(transparent)]
    Internal(#[from] sqlx::Error),
}

/// Maps a session-validation failure to its bounded `outcome` attribute for the
/// `jaunder.auth.session_validations` metric. Kept separate (and exhaustively
/// tested) so every variant's mapping is covered independent of which errors a
/// given request path happens to produce.
#[must_use]
pub fn session_outcome(error: &SessionAuthError) -> host::metrics::SessionOutcome {
    match error {
        SessionAuthError::InvalidToken => host::metrics::SessionOutcome::InvalidToken,
        SessionAuthError::SessionNotFound => host::metrics::SessionOutcome::SessionNotFound,
        SessionAuthError::Internal(_) => host::metrics::SessionOutcome::Internal,
    }
}

/// Async operations on the `sessions` table.
///
/// This trait manages the lifecycle of session tokens used for authenticating
/// web and API requests.
#[cfg_attr(feature = "test-utils", mockall::automock)]
#[async_trait]
pub trait SessionStorage: Send + Sync {
    /// Creates a new session for a user.
    ///
    /// The `label` should be a meaningful identifier for the session (e.g., browser/device name).
    /// It is stored in the database and returned in session listings.
    ///
    /// Returns the raw (un-hashed) token to be delivered to the client.
    async fn create_session(
        &self,
        transaction: &mut WriteTransaction,
        user_id: UserId,
        label: &SessionLabel,
    ) -> sqlx::Result<RawToken>;

    /// Validates a raw session token and returns the associated record.
    ///
    /// On success, refreshes `last_used_at` only when the stored value is older
    /// than the 60 second freshness window.
    async fn authenticate(
        &self,
        transaction: &mut WriteTransaction,
        raw_token: &RawToken,
    ) -> Result<SessionRecord, SessionAuthError>;

    /// Revokes a specific session by its token hash.
    async fn revoke_session(
        &self,
        transaction: &mut WriteTransaction,
        token_hash: &TokenHash,
    ) -> sqlx::Result<()>;

    /// Returns a list of all active sessions for a user.
    async fn list_sessions(&self, user_id: UserId) -> sqlx::Result<Vec<SessionRecord>>;
}

// ---------------------------------------------------------------------------
// Generic deduplication layer (Task 1-2 of bead session-storage-dedup-dialect)
// ---------------------------------------------------------------------------

use crate::backend::Backend;
use crate::helpers::{self, SessionRow};
use sqlx::{Database, Pool};

const SESSION_TOUCH_FRESHNESS_SECONDS: i64 = 60;

fn session_touch_cutoff(now: UtcInstant) -> UtcInstant {
    UtcInstant::from(now.value() - chrono::Duration::seconds(SESSION_TOUCH_FRESHNESS_SECONDS))
}

/// Per-backend divergences of [`SessionStorage`]. The only operation that differs
/// between `SQLite` and Postgres is the atomic touch-and-load used by
/// `authenticate` (`SQLite`: explicit tx; Postgres: data-modifying CTE).
#[async_trait]
pub trait SessionDialect: Backend
where
    // Bounds repeated from `Backend`: Rust does not propagate a supertrait's
    // `where`-clause to subtraits or `impl` headers, so each generic user must
    // restate them (see ADR-0019).
    for<'q> i64: sqlx::Encode<'q, Self> + sqlx::Type<Self>,
    for<'q> &'q str: sqlx::Encode<'q, Self> + sqlx::Type<Self>,
    for<'q> UtcInstant: sqlx::Encode<'q, Self> + sqlx::Type<Self>,
    for<'c> &'c sqlx::Pool<Self>: sqlx::Executor<'c, Database = Self>,
    SessionRow: for<'r> sqlx::FromRow<'r, Self::Row>,
{
    /// Return the joined session row (with username), touching `last_used_at`
    /// only when the stored value is older than `stale_before`. `None` if no
    /// such session exists.
    async fn touch_and_load(
        transaction: &mut WriteTransaction,
        token_hash: &TokenHash,
        now: UtcInstant,
        stale_before: UtcInstant,
    ) -> sqlx::Result<Option<SessionRow>>;
}

/// Generic `SessionStorage` backed by any [`SessionDialect`] database.
pub struct SessionStore<DB: Database> {
    pool: Pool<DB>,
}

impl<DB: Database> SessionStore<DB> {
    #[must_use]
    pub fn new(pool: Pool<DB>) -> Self {
        Self { pool }
    }
}

#[async_trait]
impl<DB> SessionStorage for SessionStore<DB>
where
    DB: SessionDialect,
    SessionRow: for<'r> sqlx::FromRow<'r, DB::Row>,
    for<'q> i64: sqlx::Encode<'q, DB> + sqlx::Type<DB>,
    for<'q> &'q str: sqlx::Encode<'q, DB> + sqlx::Type<DB>,
    // `TokenHash`/`Username` bind/decode as themselves via the ADR-0071 sqlx
    // bridge (the `SessionRow: FromRow` bound above threads the decode).
    String: sqlx::Type<DB>,
    for<'q> String: sqlx::Encode<'q, DB>,
    for<'q> UtcInstant: sqlx::Encode<'q, DB> + sqlx::Type<DB>,
    for<'c> &'c Pool<DB>: sqlx::Executor<'c, Database = DB>,
    for<'c> &'c mut DB::Connection: sqlx::Executor<'c, Database = DB>,
    for<'q> DB::Arguments<'q>: sqlx::IntoArguments<'q, DB>,
{
    #[tracing::instrument(
        name = "storage.session.create",
        skip(self, transaction, label),
        fields(user_id, db.system = DB::DB_SYSTEM)
    )]
    async fn create_session(
        &self,
        transaction: &mut WriteTransaction,
        user_id: UserId,
        label: &SessionLabel,
    ) -> sqlx::Result<RawToken> {
        let (raw_token, token_hash) = token::generate_hashed();
        let now = UtcInstant::now();
        let connection = DB::write_connection(transaction)?;

        sqlx::query(
            "INSERT INTO sessions (token_hash, user_id, label, created_at, last_used_at)
             VALUES ($1, $2, $3, $4, $5)",
        )
        .bind(token_hash)
        .bind(user_id)
        .bind(label)
        .bind(now)
        .bind(now)
        .execute(&mut *connection)
        .await?;

        Ok(raw_token)
    }

    #[tracing::instrument(
        name = "storage.session.authenticate",
        skip(self, transaction, raw_token),
        fields(db.system = DB::DB_SYSTEM)
    )]
    async fn authenticate(
        &self,
        transaction: &mut WriteTransaction,
        raw_token: &RawToken,
    ) -> Result<SessionRecord, SessionAuthError> {
        let token_hash = token::hash(raw_token).map_err(|_| SessionAuthError::InvalidToken)?;

        let now = UtcInstant::now();
        let stale_before = session_touch_cutoff(now);

        let row = DB::touch_and_load(transaction, &token_hash, now, stale_before)
            .await?
            .ok_or(SessionAuthError::SessionNotFound)?;

        let record = helpers::session_record_from_row(row);
        Ok(record)
    }

    #[tracing::instrument(
        name = "storage.session.revoke",
        skip(self, transaction, token_hash),
        fields(db.system = DB::DB_SYSTEM)
    )]
    async fn revoke_session(
        &self,
        transaction: &mut WriteTransaction,
        token_hash: &TokenHash,
    ) -> sqlx::Result<()> {
        let connection = DB::write_connection(transaction)?;
        sqlx::query("DELETE FROM sessions WHERE token_hash = $1")
            .bind(token_hash)
            .execute(&mut *connection)
            .await?;
        Ok(())
    }

    async fn list_sessions(&self, user_id: UserId) -> sqlx::Result<Vec<SessionRecord>> {
        let rows = sqlx::query_as::<_, SessionRow>(
            "SELECT s.token_hash, s.user_id, u.username, s.label, s.created_at, s.last_used_at
             FROM sessions s JOIN users u ON s.user_id = u.user_id
             WHERE s.user_id = $1",
        )
        .bind(user_id)
        .fetch_all(&self.pool)
        .await?;

        // A corrupt/migrated `token_hash` or `username` column is rejected as a
        // decode error by the `query_as` above (the sqlx bridge validates through
        // `FromStr`), so building the records here is infallible.
        Ok(rows
            .into_iter()
            .map(helpers::session_record_from_row)
            .collect())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::{Backend, SeedUser, TestEnv, backends};
    use common::test_support::{parse_raw_token, parse_session_label};
    use rstest::*;
    use rstest_reuse::*;
    use std::sync::Arc;

    #[apply(backends)]
    #[tokio::test]
    async fn authenticate_with_closed_pool_returns_internal_error(#[case] backend: Backend) {
        let TestEnv { state, base } = backend.setup().await;
        base.close_pool().await;
        let sessions = Arc::clone(&state.sessions);
        let result = state
            .write_scope
            .run(|transaction| {
                let token = parse_raw_token("dGVzdA");
                Box::pin(async move { sessions.authenticate(transaction, &token).await })
            })
            .await;
        assert!(matches!(
            result,
            Err(crate::WriteScopeError::Begin(sqlx::Error::PoolClosed))
        ));
    }

    #[apply(backends)]
    #[tokio::test]
    async fn session_round_trips_token_hash_and_username(#[case] backend: Backend) {
        // Keep the whole `TestEnv` bound: dropping `base` unlinks the SQLite file
        // (ADR-0053 TempDir hazard).
        let env = backend.setup().await;
        let user_id = SeedUser::new().seed(&env.state).await.user_id;

        // `create_session` binds the `TokenHash`; `authenticate`/`list_sessions`
        // decode the `token_hash` and joined `username` columns straight back into
        // their newtypes via the sqlx bridge (#438).
        let sessions = Arc::clone(&env.state.sessions);
        let label = parse_session_label("Test Device");
        let outcome = env
            .state
            .write_scope
            .run(|transaction| {
                Box::pin(async move { sessions.create_session(transaction, user_id, &label).await })
            })
            .await
            .unwrap();
        let raw_token = crate::test_support::confirmed_for(outcome, "session fixture setup");
        let expected_hash = token::hash(&raw_token).unwrap();

        let sessions = Arc::clone(&env.state.sessions);
        let outcome = env
            .state
            .write_scope
            .run(|transaction| {
                Box::pin(async move { sessions.authenticate(transaction, &raw_token).await })
            })
            .await
            .unwrap();
        let record = crate::test_support::confirmed_for(outcome, "session authentication");
        assert_eq!(record.token_hash, expected_hash);
        assert_eq!(record.user_id, user_id);

        let listed = env.state.sessions.list_sessions(user_id).await.unwrap();
        assert_eq!(listed.len(), 1);
        assert_eq!(listed[0].token_hash, expected_hash);
    }

    #[apply(backends)]
    #[tokio::test]
    async fn list_sessions_rejects_a_malformed_token_hash_column(#[case] backend: Backend) {
        let env = backend.setup().await;
        let user_id = SeedUser::new().seed(&env.state).await.user_id;
        let sessions = Arc::clone(&env.state.sessions);
        let label = parse_session_label("Test Device");
        let outcome = env
            .state
            .write_scope
            .run(|transaction| {
                Box::pin(async move { sessions.create_session(transaction, user_id, &label).await })
            })
            .await
            .unwrap();
        assert!(matches!(outcome, common::MutationOutcome::Confirmed(_)));

        // Overwrite the `token_hash` column with a value `TokenHash::from_str`
        // rejects (a space is not a valid token character), binding it as a raw
        // `&str` so the bad value actually lands in the column — the typed bind
        // could not produce it.
        let sql = "UPDATE sessions SET token_hash = $1";
        crate::with_closeable_pool!(env.base.pool(), pool, {
            sqlx::query(sql)
                .bind("bad hash")
                .execute(pool)
                .await
                .unwrap();
        });

        // The read decodes the `token_hash` column into `TokenHash` via the sqlx
        // bridge, which validates through `FromStr`; the malformed value surfaces
        // as a column-decode error rather than being silently admitted (covers the
        // bridge's `Decode` error arm).
        let err = env.state.sessions.list_sessions(user_id).await.unwrap_err();
        assert!(
            matches!(err, sqlx::Error::ColumnDecode { .. }),
            "expected a column-decode error, got: {err:?}"
        );
    }

    #[apply(backends)]
    #[tokio::test]
    async fn list_sessions_repairs_an_invalid_stored_label_without_rejecting_the_row(
        #[case] backend: Backend,
    ) {
        let env = backend.setup().await;
        let user_id = SeedUser::new().seed(&env.state).await.user_id;
        let sessions = Arc::clone(&env.state.sessions);
        let label = parse_session_label("Test Device");
        let outcome = env
            .state
            .write_scope
            .run(|transaction| {
                Box::pin(async move { sessions.create_session(transaction, user_id, &label).await })
            })
            .await
            .unwrap();
        assert!(matches!(outcome, common::MutationOutcome::Confirmed(_)));
        let stored = "x".repeat(1_000);
        crate::with_closeable_pool!(env.base.pool(), pool, {
            sqlx::query("UPDATE sessions SET label = $1")
                .bind(&stored)
                .execute(pool)
                .await
                .unwrap();
        });

        let sessions = env.state.sessions.list_sessions(user_id).await.unwrap();
        assert_eq!(sessions.len(), 1);
        assert_eq!(sessions[0].label, SessionLabel::from_lossy(&stored));
    }
    #[apply(backends)]
    #[tokio::test]
    async fn session_rows_preserve_created_and_last_used_roles(#[case] backend: Backend) {
        let env = backend.setup().await;
        let user_id = SeedUser::new().seed(&env.state).await.user_id;
        let sessions = Arc::clone(&env.state.sessions);
        let label = parse_session_label("Test Device");
        let outcome = env
            .state
            .write_scope
            .run(|transaction| {
                Box::pin(async move { sessions.create_session(transaction, user_id, &label).await })
            })
            .await
            .unwrap();
        let raw_token = crate::test_support::confirmed_for(outcome, "session fixture setup");
        let token_hash = token::hash(&raw_token).unwrap();
        let created_at = "2026-01-02T03:04:05.123456Z".parse::<UtcInstant>().unwrap();
        let last_used_at = "2026-03-04T05:06:07.654321Z".parse::<UtcInstant>().unwrap();

        crate::with_closeable_pool!(env.base.pool(), pool, {
            sqlx::query(
                "UPDATE sessions SET created_at = $1, last_used_at = $2 WHERE token_hash = $3",
            )
            .bind(created_at)
            .bind(last_used_at)
            .bind(&token_hash)
            .execute(pool)
            .await
            .unwrap();
        });

        let sessions = env.state.sessions.list_sessions(user_id).await.unwrap();
        assert_eq!(sessions.len(), 1);
        assert_eq!(sessions[0].created_at, created_at);
        assert_eq!(sessions[0].last_used_at, last_used_at);
    }

    #[apply(backends)]
    #[tokio::test]
    async fn touch_and_load_observes_stale_exact_and_fresh_boundaries(#[case] backend: Backend) {
        let env = backend.setup().await;
        let user_id = SeedUser::new().seed(&env.state).await.user_id;
        let now = "2026-04-05T06:07:08.123456Z".parse::<UtcInstant>().unwrap();
        let stale_before = session_touch_cutoff(now);
        let cases = [
            (
                "stale",
                UtcInstant::from(stale_before.value() - chrono::Duration::microseconds(1)),
                now,
            ),
            ("exact", stale_before, stale_before),
            (
                "fresh",
                UtcInstant::from(stale_before.value() + chrono::Duration::microseconds(1)),
                UtcInstant::from(stale_before.value() + chrono::Duration::microseconds(1)),
            ),
        ];

        for (label, stored_last_used_at, expected_last_used_at) in cases {
            let sessions = Arc::clone(&env.state.sessions);
            let label = parse_session_label(label);
            let label_for_create = label.clone();
            let outcome = env
                .state
                .write_scope
                .run(move |transaction| {
                    Box::pin(async move {
                        sessions
                            .create_session(transaction, user_id, &label_for_create)
                            .await
                    })
                })
                .await
                .unwrap();
            let raw_token = crate::test_support::confirmed_for(outcome, "session fixture setup");
            let token_hash = token::hash(&raw_token).unwrap();
            crate::with_closeable_pool!(env.base.pool(), pool, {
                sqlx::query("UPDATE sessions SET last_used_at = $1 WHERE token_hash = $2")
                    .bind(stored_last_used_at)
                    .bind(&token_hash)
                    .execute(pool)
                    .await
                    .unwrap();
            });
            let token_hash_for_touch = token_hash.clone();

            let outcome = env
                .state
                .write_scope
                .run(move |transaction| {
                    Box::pin(async move {
                        match backend {
                            Backend::Sqlite => {
                                <sqlx::Sqlite as SessionDialect>::touch_and_load(
                                    transaction,
                                    &token_hash_for_touch,
                                    now,
                                    stale_before,
                                )
                                .await
                            }
                            Backend::Postgres => {
                                <sqlx::Postgres as SessionDialect>::touch_and_load(
                                    transaction,
                                    &token_hash_for_touch,
                                    now,
                                    stale_before,
                                )
                                .await
                            }
                        }
                    })
                })
                .await
                .unwrap();
            let row = match outcome {
                common::MutationOutcome::Confirmed(Some(row)) => row,
                common::MutationOutcome::Confirmed(None) => panic!("session should exist"),
                common::MutationOutcome::CommitIndeterminate(_) => {
                    panic!("session touch requires a confirmed commit")
                }
            };
            assert_eq!(row.last_used_at(), expected_last_used_at, "{label}");
        }
    }

    #[test]
    fn session_outcome_maps_each_variant() {
        use host::metrics::SessionOutcome;
        assert!(matches!(
            session_outcome(&SessionAuthError::InvalidToken),
            SessionOutcome::InvalidToken
        ));
        assert!(matches!(
            session_outcome(&SessionAuthError::SessionNotFound),
            SessionOutcome::SessionNotFound
        ));
        assert!(matches!(
            session_outcome(&SessionAuthError::Internal(sqlx::Error::PoolClosed)),
            SessionOutcome::Internal
        ));
    }
}
