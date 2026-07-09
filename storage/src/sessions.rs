//! Session and device token storage.

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use thiserror::Error;

use common::username::Username;

/// A session record returned by [`SessionStorage`] queries.
#[derive(Clone, Debug)]
pub struct SessionRecord {
    /// SHA-256 hash of the session token.
    pub token_hash: String,
    /// ID of the user associated with this session.
    pub user_id: i64,
    /// Username at the time of session creation.
    pub username: Username,
    /// Label for the device/client (e.g., "Mobile App", "Safari on macOS", "Sign-up session").
    pub label: String,
    /// When the session was first created.
    pub created_at: DateTime<Utc>,
    /// When the session was last used to authenticate a request.
    pub last_used_at: DateTime<Utc>,
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
pub fn session_outcome(error: &SessionAuthError) -> common::metrics::SessionOutcome {
    match error {
        SessionAuthError::InvalidToken => common::metrics::SessionOutcome::InvalidToken,
        SessionAuthError::SessionNotFound => common::metrics::SessionOutcome::SessionNotFound,
        SessionAuthError::Internal(_) => common::metrics::SessionOutcome::Internal,
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
    async fn create_session(&self, user_id: i64, label: &str) -> sqlx::Result<String>;

    /// Validates a raw session token and returns the associated record.
    ///
    /// On success, updates the `last_used_at` timestamp for the session.
    ///
    /// # Errors
    ///
    /// Returns [`SessionAuthError`] if the token is invalid or the session has
    /// been revoked.
    async fn authenticate(&self, raw_token: &str) -> Result<SessionRecord, SessionAuthError>;

    /// Revokes a specific session by its token hash.
    async fn revoke_session(&self, token_hash: &str) -> sqlx::Result<()>;

    /// Returns a list of all active sessions for a user.
    async fn list_sessions(&self, user_id: i64) -> sqlx::Result<Vec<SessionRecord>>;
}

// ---------------------------------------------------------------------------
// Generic deduplication layer (Task 1-2 of bead session-storage-dedup-dialect)
// ---------------------------------------------------------------------------

use crate::backend::Backend;
use crate::helpers::{generate_hashed_token, session_record_from_row, SessionRow};
use sqlx::{Database, Pool};

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
    for<'q> DateTime<Utc>: sqlx::Encode<'q, Self> + sqlx::Type<Self>,
    for<'c> &'c sqlx::Pool<Self>: sqlx::Executor<'c, Database = Self>,
    SessionRow: for<'r> sqlx::FromRow<'r, Self::Row>,
{
    /// Update `last_used_at` for `token_hash` to `now` and return the joined
    /// session row (with username), atomically. `None` if no such session.
    async fn touch_and_load(
        pool: &Pool<Self>,
        token_hash: &str,
        now: DateTime<Utc>,
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
    for<'q> DateTime<Utc>: sqlx::Encode<'q, DB> + sqlx::Type<DB>,
    for<'c> &'c Pool<DB>: sqlx::Executor<'c, Database = DB>,
    for<'q> DB::Arguments<'q>: sqlx::IntoArguments<'q, DB>,
{
    #[tracing::instrument(
        name = "storage.session.create",
        skip(self, label),
        fields(user_id, db.system = DB::DB_SYSTEM)
    )]
    async fn create_session(&self, user_id: i64, label: &str) -> sqlx::Result<String> {
        let (raw_token, token_hash) = generate_hashed_token()?;
        let now = Utc::now();

        sqlx::query(
            "INSERT INTO sessions (token_hash, user_id, label, created_at, last_used_at)
             VALUES ($1, $2, $3, $4, $5)",
        )
        .bind(token_hash.as_str())
        .bind(user_id)
        .bind(label)
        .bind(now)
        .bind(now)
        .execute(&self.pool)
        .await?;

        Ok(raw_token)
    }

    #[tracing::instrument(
        name = "storage.session.authenticate",
        skip(self, raw_token),
        fields(db.system = DB::DB_SYSTEM)
    )]
    async fn authenticate(&self, raw_token: &str) -> Result<SessionRecord, SessionAuthError> {
        let token_hash =
            crate::auth::hash_token(raw_token).map_err(|_| SessionAuthError::InvalidToken)?;

        let now = Utc::now();

        let row = DB::touch_and_load(&self.pool, &token_hash, now)
            .await?
            .ok_or(SessionAuthError::SessionNotFound)?;

        let record = session_record_from_row(row)?;
        Ok(record)
    }

    #[tracing::instrument(
        name = "storage.session.revoke",
        skip(self, token_hash),
        fields(db.system = DB::DB_SYSTEM)
    )]
    async fn revoke_session(&self, token_hash: &str) -> sqlx::Result<()> {
        sqlx::query("DELETE FROM sessions WHERE token_hash = $1")
            .bind(token_hash)
            .execute(&self.pool)
            .await?;
        Ok(())
    }

    async fn list_sessions(&self, user_id: i64) -> sqlx::Result<Vec<SessionRecord>> {
        let rows = sqlx::query_as::<_, SessionRow>(
            "SELECT s.token_hash, s.user_id, u.username, s.label, s.created_at, s.last_used_at
             FROM sessions s JOIN users u ON s.user_id = u.user_id
             WHERE s.user_id = $1",
        )
        .bind(user_id)
        .fetch_all(&self.pool)
        .await?;

        rows.into_iter().map(session_record_from_row).collect()
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
    async fn authenticate_with_closed_pool_returns_internal_error(#[case] backend: Backend) {
        let TestEnv { state, base } = backend.setup().await;
        base.close_pool().await;
        let result = state.sessions.authenticate("dGVzdA").await;
        assert!(matches!(result, Err(SessionAuthError::Internal(_))));
    }

    #[test]
    fn session_outcome_maps_each_variant() {
        use common::metrics::SessionOutcome;
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
