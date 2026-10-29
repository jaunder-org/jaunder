//! Session and device token storage.

use async_trait::async_trait;
use jiff::ToSpan;

use sqlx::{Database, Encode, Error, Executor, Pool, Result, Type};

use crate::WriteTransaction;
#[cfg(test)]
use crate::helpers::StoredSessionLabel;
use crate::sql::{QueryStorageExt, RowCount};
use common::ids::UserId;
use common::pagination::RowLimit;
use common::session_label::SessionLabel;
use common::time::UtcInstant;
use common::token::{RawToken, TokenHash};
use common::username::Username;
use host::{metrics, retention::Domain, token};
/// Test-only invalid `sessions.token_hash` column value.
///
/// Fixtures use this role to bypass `TokenHash` validation deliberately without
/// admitting arbitrary text to production storage interfaces.
#[cfg(test)]
#[derive(macros::SqlxBridge)]
pub(crate) struct CorruptSessionTokenHash(String);

#[cfg(test)]
impl CorruptSessionTokenHash {
    fn malformed() -> Self {
        Self("bad hash".to_owned())
    }
}

/// Durable credential purpose; labels and authentication transports never select it.
#[macros::text_enum(sqlx, error = InvalidSessionKind, message = "session kind must be browser, machine, or unknown")]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[strum(serialize_all = "snake_case")]
pub enum SessionKind {
    /// A Session established by browser sign-in.
    Browser,
    /// An App Password or other deliberately minted machine credential.
    Machine,
    /// A legacy credential whose purpose is not known.
    Unknown,
}

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
    /// Stored credential purpose, independent of display label and transport.
    pub kind: SessionKind,
    /// When the session was first created.
    pub created_at: UtcInstant,
    /// When the session was last persisted as used to authenticate a request.
    ///
    /// This is operator-facing metadata with bounded staleness: authentication
    /// may skip updating it for up to 60 seconds while the stored value is fresh.
    pub last_used_at: UtcInstant,
}

/// Clock sampled only after acquiring the Session operation's database lock.
/// Production uses UTC wall time; controlled clocks are test-only inputs.
#[derive(Clone, Default)]
pub struct SessionClock {
    source: SessionClockSource,
}

#[derive(Clone, Default)]
enum SessionClockSource {
    #[default]
    Wall,
    #[cfg(any(test, feature = "test-utils"))]
    Controlled(std::sync::Arc<dyn Fn() -> UtcInstant + Send + Sync>),
}

impl SessionClock {
    pub(crate) fn now(&self) -> UtcInstant {
        match &self.source {
            SessionClockSource::Wall => UtcInstant::now(),
            #[cfg(any(test, feature = "test-utils"))]
            SessionClockSource::Controlled(read) => read(),
        }
    }

    #[cfg(any(test, feature = "test-utils"))]
    #[must_use]
    pub fn fixed(now: UtcInstant) -> Self {
        Self::for_test(move || now)
    }

    #[cfg(any(test, feature = "test-utils"))]
    #[must_use]
    pub fn for_test(read: impl Fn() -> UtcInstant + Send + Sync + 'static) -> Self {
        Self {
            source: SessionClockSource::Controlled(std::sync::Arc::new(read)),
        }
    }
}

/// Authentication data pending the enclosing transaction's commit acknowledgement.
#[derive(Clone, Debug)]
pub struct SessionAuthentication {
    pub record: SessionRecord,
    /// True only if this transaction actually persisted activity.
    pub touched: bool,
}

/// Errors that can occur when authenticating a session token.
#[derive(Debug, thiserror::Error)]
pub enum SessionAuthError {
    /// The token is malformed or invalid.
    #[error("invalid token")]
    InvalidToken,
    /// No active session matches the provided token.
    #[error("session not found")]
    SessionNotFound,
    /// An unexpected database error occurred.
    #[error(transparent)]
    Internal(#[from] Error),
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
        kind: SessionKind,
    ) -> Result<RawToken>;

    /// Validates a raw session token and returns the associated record.
    ///
    /// Browser Sessions expire after 30 days of inactivity. On success, refreshes
    /// `last_used_at` only at the 60-second threshold. Samples one explicit UTC
    /// `now` from `clock` after acquiring the database lock, so waiting cannot
    /// revive a Session that expires during the wait. Eligibility and activity
    /// remain serialized with revocation until the enclosing scope commits.
    async fn authenticate(
        &self,
        transaction: &mut WriteTransaction,
        raw_token: &RawToken,
        clock: SessionClock,
    ) -> Result<SessionAuthentication, SessionAuthError>;

    /// Revokes a specific session by its token hash.
    async fn revoke_session(
        &self,
        transaction: &mut WriteTransaction,
        token_hash: &TokenHash,
    ) -> Result<()>;

    /// Revokes every active session belonging to `user_id`.
    async fn revoke_all_for_user(
        &self,
        transaction: &mut WriteTransaction,
        user_id: UserId,
    ) -> sqlx::Result<()>;
    /// Revokes every active session for `user_id` except `current_token_hash`.
    ///
    /// The caller must have established ownership of the preserved cookie
    /// Session before entering its write scope.
    async fn revoke_all_for_user_except(
        &self,
        transaction: &mut WriteTransaction,
        user_id: UserId,
        current_token_hash: &TokenHash,
    ) -> sqlx::Result<()>;

    /// Drains expired typed Browser Sessions using one UTC instant and separate
    /// statements of at most 500 rows. Completed batches survive later failure;
    /// Unknown and Machine credentials are never eligible.
    async fn prune_expired_browser_sessions(&self, now: UtcInstant) -> Result<u64>;

    /// Returns a list of all active sessions for a user.
    async fn list_sessions(&self, user_id: UserId) -> Result<Vec<SessionRecord>>;
}

// ---------------------------------------------------------------------------
// Generic deduplication layer (Task 1-2 of bead session-storage-dedup-dialect)
// ---------------------------------------------------------------------------

use crate::backend::Backend;
use crate::helpers::{self, SessionRow};

/// Inactivity lifetime of a typed Browser Session, in elapsed seconds.
pub const BROWSER_SESSION_IDLE_SECONDS: i64 = 2_592_000;
const SESSION_TOUCH_FRESHNESS_SECONDS: i64 = 60;

pub(crate) fn session_idle_cutoff(now: UtcInstant) -> UtcInstant {
    UtcInstant::from(
        now.value()
            .saturating_sub(BROWSER_SESSION_IDLE_SECONDS.seconds())
            .map_or(jiff::Timestamp::MIN, std::convert::identity),
    )
}

pub(crate) fn session_touch_cutoff(now: UtcInstant) -> UtcInstant {
    // At Timestamp::MIN there is no earlier session timestamp to refresh, so
    // clamping preserves the freshness-window predicate.
    UtcInstant::from(
        now.value()
            .saturating_sub(SESSION_TOUCH_FRESHNESS_SECONDS.seconds())
            .map_or(jiff::Timestamp::MIN, std::convert::identity),
    )
}

/// Per-backend serialization of Session eligibility and conditional activity.
/// `SQLite` relies on its enclosing immediate write scope; `PostgreSQL` locks the
/// Session row until the enclosing scope finishes.
#[async_trait]
pub trait SessionDialect: Backend
where
    // Bounds repeated from `Backend`: Rust does not propagate a supertrait's
    // `where`-clause to subtraits or `impl` headers, so each generic user must
    // restate them (see ADR-0019).
    for<'q> i64: Encode<'q, Self> + Type<Self>,
    for<'q> &'q str: Encode<'q, Self> + Type<Self>,
    for<'q> UtcInstant: Encode<'q, Self> + Type<Self>,
    for<'c> &'c Pool<Self>: Executor<'c, Database = Self>,
    SessionRow: for<'r> sqlx::FromRow<'r, Self::Row>,
{
    /// Returns the eligible joined row and whether this operation persisted
    /// activity. Touches at/before `stale_before`; Browser rows at/before the
    /// expiry cutoff are ineligible. Locks remain held until enclosing commit.
    async fn touch_and_load(
        transaction: &mut WriteTransaction,
        token_hash: &TokenHash,
        clock: &SessionClock,
    ) -> Result<Option<(SessionRow, bool)>>;
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
    for<'q> RowCount: sqlx::Decode<'q, DB> + Type<DB>,
    usize: sqlx::ColumnIndex<DB::Row>,
    SessionRow: for<'r> sqlx::FromRow<'r, DB::Row>,
    for<'q> i64: Encode<'q, DB> + Type<DB>,
    for<'q> &'q str: Encode<'q, DB> + Type<DB>,
    // `TokenHash`/`Username` bind/decode as themselves via the ADR-0071 sqlx
    // bridge (the `SessionRow: FromRow` bound above threads the decode).
    String: Type<DB>,
    for<'q> String: Encode<'q, DB>,
    for<'q> SessionKind: Encode<'q, DB> + Type<DB>,
    for<'q> UtcInstant: Encode<'q, DB> + Type<DB>,
    for<'c> &'c Pool<DB>: Executor<'c, Database = DB>,
    for<'c> &'c mut DB::Connection: Executor<'c, Database = DB>,
    DB::Arguments: sqlx::IntoArguments<DB>,
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
        kind: SessionKind,
    ) -> Result<RawToken> {
        let (raw_token, token_hash) = token::generate_hashed();
        let now = UtcInstant::now();
        let connection = DB::write_connection(transaction)?;

        sqlx::query(
            "INSERT INTO sessions (token_hash, user_id, label, created_at, last_used_at, kind)
             VALUES ($1, $2, $3, $4, $5, $6)",
        )
        .bind_storage(token_hash)
        .bind_storage(user_id)
        .bind_storage(label)
        .bind_storage(now)
        .bind_storage(now)
        .bind_storage(kind)
        .execute(&mut *connection)
        .await?;

        Ok(raw_token)
    }

    #[tracing::instrument(
        name = "storage.session.authenticate",
        skip(self, transaction, raw_token, clock),
        fields(db.system = DB::DB_SYSTEM)
    )]
    async fn authenticate(
        &self,
        transaction: &mut WriteTransaction,
        raw_token: &RawToken,
        clock: SessionClock,
    ) -> Result<SessionAuthentication, SessionAuthError> {
        let token_hash = token::hash(raw_token).map_err(|_| SessionAuthError::InvalidToken)?;

        let (row, touched) = DB::touch_and_load(transaction, &token_hash, &clock)
            .await?
            .ok_or(SessionAuthError::SessionNotFound)?;

        let record = helpers::session_record_from_row(row);
        Ok(SessionAuthentication { record, touched })
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
    ) -> Result<()> {
        let connection = DB::write_connection(transaction)?;
        sqlx::query("DELETE FROM sessions WHERE token_hash = $1")
            .bind_storage(token_hash)
            .execute(&mut *connection)
            .await?;
        Ok(())
    }

    async fn revoke_all_for_user(
        &self,
        transaction: &mut WriteTransaction,
        user_id: UserId,
    ) -> Result<()> {
        let connection = DB::write_connection(transaction)?;
        sqlx::query("DELETE FROM sessions WHERE user_id = $1")
            .bind_storage(user_id)
            .execute(&mut *connection)
            .await?;
        Ok(())
    }

    async fn revoke_all_for_user_except(
        &self,
        transaction: &mut WriteTransaction,
        user_id: UserId,
        current_token_hash: &TokenHash,
    ) -> Result<()> {
        let connection = DB::write_connection(transaction)?;
        sqlx::query("DELETE FROM sessions WHERE user_id = $1 AND token_hash != $2")
            .bind_storage(user_id)
            .bind_storage(current_token_hash)
            .execute(&mut *connection)
            .await?;
        Ok(())
    }

    async fn prune_expired_browser_sessions(&self, now: UtcInstant) -> Result<u64> {
        const BATCH_SIZE: RowLimit = RowLimit::at_most(500);
        let cutoff = session_idle_cutoff(now);
        let mut pruned = 0;
        loop {
            // Pool execution ends each statement's transaction before the next
            // batch, releasing locks even when a later batch fails. Recheck
            // eligibility on the DELETE itself so PostgreSQL cannot delete a
            // row renewed while this statement waited for its row lock.
            let batch = sqlx::query_scalar::<_, RowCount>(
                "DELETE FROM sessions WHERE kind = 'browser' AND last_used_at <= $1
                 AND token_hash IN (
                     SELECT token_hash FROM sessions
                     WHERE kind = 'browser' AND last_used_at <= $1
                     ORDER BY token_hash LIMIT $2
                 ) RETURNING CAST(1 AS BIGINT)",
            )
            .bind_storage(cutoff)
            .bind_storage(BATCH_SIZE)
            .fetch_all(&self.pool)
            .await?
            .len() as u64;
            if batch != 0 {
                metrics::retention_pruned(Domain::BrowserSessions, batch);
            }
            pruned += batch;
            if batch == 0 {
                return Ok(pruned);
            }
        }
    }

    async fn list_sessions(&self, user_id: UserId) -> Result<Vec<SessionRecord>> {
        let rows = sqlx::query_as::<_, SessionRow>(
            "SELECT s.token_hash, s.user_id, u.username, s.label, s.created_at, s.last_used_at, s.kind
             FROM sessions s JOIN users u ON s.user_id = u.user_id
             WHERE s.user_id = $1",
        )
        .bind_storage(user_id)
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
    use crate::test_support::{Backend, SeedUser, backends};
    use common::test_support::parse_session_label;
    use rstest::*;
    use rstest_reuse::*;
    use std::sync::Arc;

    #[apply(backends)]
    #[tokio::test]
    async fn session_kinds_survive_authentication_listing_and_revocation(#[case] backend: Backend) {
        let env = backend.setup().await;
        let user_id = SeedUser::new()
            .seed(env.users(), env.write_scope().clone())
            .await
            .user_id;
        for kind in [
            SessionKind::Browser,
            SessionKind::Machine,
            SessionKind::Unknown,
        ] {
            let sessions = env.sessions();
            // Identical labels deliberately cannot classify credentials.
            let label = parse_session_label("Browser");
            let outcome = env
                .write_scope()
                .run(|transaction| {
                    Box::pin(async move {
                        sessions
                            .create_session(transaction, user_id, &label, kind)
                            .await
                    })
                })
                .await
                .unwrap();
            let raw = crate::test_support::confirmed_for(outcome, "kind creation");
            let sessions = env.sessions();
            let outcome = env
                .write_scope()
                .run(|transaction| {
                    Box::pin(async move {
                        sessions
                            .authenticate(transaction, &raw, SessionClock::default())
                            .await
                            .map(|auth| auth.record)
                    })
                })
                .await
                .unwrap();
            let record = crate::test_support::confirmed_for(outcome, "kind authentication");
            assert_eq!(record.kind, kind);
            let listed = env.sessions().list_sessions(user_id).await.unwrap();
            assert_eq!(listed.len(), 1);
            assert_eq!(listed[0].kind, kind);
            let sessions = env.sessions();
            env.write_scope()
                .run(|transaction| {
                    Box::pin(async move {
                        sessions
                            .revoke_session(transaction, &record.token_hash)
                            .await
                    })
                })
                .await
                .unwrap();
            assert!(
                env.sessions()
                    .list_sessions(user_id)
                    .await
                    .unwrap()
                    .is_empty()
            );
        }
    }

    #[apply(backends)]
    #[tokio::test]
    async fn browser_cleanup_preserves_other_kinds_and_obeys_exact_cutoff(
        #[case] backend: Backend,
    ) {
        let env = backend.setup().await;
        for kind in [
            SessionKind::Browser,
            SessionKind::Machine,
            SessionKind::Unknown,
        ] {
            let (user_id, _, used) = idle_fixture(&env, kind).await;
            let before = UtcInstant::from(used.value().checked_add(2_591_999.seconds()).unwrap());
            let cutoff = UtcInstant::from(used.value().checked_add(2_592_000.seconds()).unwrap());
            assert_eq!(
                env.sessions()
                    .prune_expired_browser_sessions(before)
                    .await
                    .unwrap(),
                0
            );
            assert_eq!(
                env.sessions()
                    .prune_expired_browser_sessions(cutoff)
                    .await
                    .unwrap(),
                u64::from(kind == SessionKind::Browser)
            );
            assert_eq!(
                env.sessions().list_sessions(user_id).await.unwrap().len(),
                usize::from(kind != SessionKind::Browser)
            );
            assert_eq!(
                env.sessions()
                    .prune_expired_browser_sessions(cutoff)
                    .await
                    .unwrap(),
                0
            );
        }
    }

    #[apply(backends)]
    #[tokio::test]
    async fn browser_cleanup_commits_500_row_batches_and_retries_partial_progress(
        #[case] backend: Backend,
    ) {
        let env = backend.setup().await;
        let user_id = SeedUser::new()
            .seed(env.users(), env.write_scope())
            .await
            .user_id;
        let sessions = env.sessions();
        crate::test_support::confirmed_for(
            env.write_scope()
                .run(|transaction| {
                    Box::pin(async move {
                        for _ in 0..1001 {
                            sessions
                                .create_session(
                                    transaction,
                                    user_id,
                                    &parse_session_label("cleanup backlog"),
                                    SessionKind::Browser,
                                )
                                .await?;
                        }
                        Ok::<_, sqlx::Error>(())
                    })
                })
                .await
                .unwrap(),
            "seed backlog",
        );
        env.base
            .pool()
            .execute("UPDATE sessions SET last_used_at = '2026-01-01 00:00:00+00'")
            .await
            .unwrap();
        // Fail after exactly one full batch. A statement exceeding 500 rows
        // would roll back its entire first batch, making the remaining count wrong.
        if matches!(backend, Backend::Postgres) {
            env.base.pool().execute("CREATE FUNCTION fail_browser_cleanup() RETURNS trigger LANGUAGE plpgsql AS $$ BEGIN IF (SELECT COUNT(*) FROM sessions) <= 501 THEN RAISE EXCEPTION 'injected cleanup failure'; END IF; RETURN OLD; END $$").await.unwrap();
        }
        let install = match backend {
            Backend::Sqlite => {
                "CREATE TRIGGER fail_browser_cleanup BEFORE DELETE ON sessions WHEN (SELECT COUNT(*) FROM sessions) <= 501 BEGIN SELECT RAISE(ABORT, 'injected cleanup failure'); END"
            }
            Backend::Postgres => {
                "CREATE TRIGGER fail_browser_cleanup BEFORE DELETE ON sessions FOR EACH ROW EXECUTE FUNCTION fail_browser_cleanup()"
            }
        };
        env.base.pool().execute(install).await.unwrap();
        let now = "2026-01-31T00:00:00Z".parse().unwrap();
        assert!(
            env.sessions()
                .prune_expired_browser_sessions(now)
                .await
                .is_err()
        );
        assert_eq!(
            env.sessions().list_sessions(user_id).await.unwrap().len(),
            501
        );
        let remove = match backend {
            Backend::Sqlite => "DROP TRIGGER fail_browser_cleanup",
            Backend::Postgres => "DROP TRIGGER fail_browser_cleanup ON sessions",
        };
        env.base.pool().execute(remove).await.unwrap();
        assert_eq!(
            env.sessions()
                .prune_expired_browser_sessions(now)
                .await
                .unwrap(),
            501
        );
        assert!(
            env.sessions()
                .list_sessions(user_id)
                .await
                .unwrap()
                .is_empty()
        );
        assert_eq!(
            env.sessions()
                .prune_expired_browser_sessions(now)
                .await
                .unwrap(),
            0
        );
    }

    #[apply(backends)]
    #[tokio::test]
    async fn browser_session_is_expired_at_the_idle_cutoff(#[case] backend: Backend) {
        let env = backend.setup().await;
        let user_id = SeedUser::new()
            .seed(env.users(), env.write_scope().clone())
            .await
            .user_id;
        let sessions = env.sessions();
        let label = parse_session_label("cutoff");
        let raw = crate::test_support::confirmed_for(
            env.write_scope()
                .run(|transaction| {
                    Box::pin(async move {
                        sessions
                            .create_session(transaction, user_id, &label, SessionKind::Browser)
                            .await
                    })
                })
                .await
                .unwrap(),
            "create browser Session",
        );
        let last_used = env.sessions().list_sessions(user_id).await.unwrap()[0].last_used_at;
        let now = UtcInstant::from(last_used.value().checked_add(2_592_000.seconds()).unwrap());
        let sessions = env.sessions();
        let result = env
            .write_scope()
            .run(|transaction| {
                Box::pin(async move {
                    sessions
                        .authenticate(transaction, &raw, SessionClock::fixed(now))
                        .await
                })
            })
            .await;
        assert!(matches!(
            result,
            Err(crate::WriteScopeError::Operation(
                SessionAuthError::SessionNotFound
            ))
        ));
        assert_eq!(
            env.sessions().list_sessions(user_id).await.unwrap()[0].last_used_at,
            last_used
        );
    }

    #[apply(backends)]
    #[tokio::test]
    async fn browser_session_activity_is_persisted_only_at_the_threshold(#[case] backend: Backend) {
        let env = backend.setup().await;
        let user_id = SeedUser::new()
            .seed(env.users(), env.write_scope().clone())
            .await
            .user_id;
        let sessions = env.sessions();
        let label = parse_session_label("activity");
        let raw = crate::test_support::confirmed_for(
            env.write_scope()
                .run(|transaction| {
                    Box::pin(async move {
                        sessions
                            .create_session(transaction, user_id, &label, SessionKind::Browser)
                            .await
                    })
                })
                .await
                .unwrap(),
            "create Session",
        );
        let original = env.sessions().list_sessions(user_id).await.unwrap()[0].last_used_at;
        for (elapsed, touched) in [(59, false), (60, true), (61, false)] {
            let now = UtcInstant::from(original.value().checked_add(elapsed.seconds()).unwrap());
            let sessions = env.sessions();
            let token = raw.clone();
            let authentication = crate::test_support::confirmed_for(
                env.write_scope()
                    .run(|transaction| {
                        Box::pin(async move {
                            sessions
                                .authenticate(transaction, &token, SessionClock::fixed(now))
                                .await
                        })
                    })
                    .await
                    .unwrap(),
                "authenticate Session",
            );
            assert_eq!(authentication.touched, touched);
            assert_eq!(
                authentication.record.last_used_at,
                if elapsed < 60 {
                    original
                } else {
                    UtcInstant::from(original.value().checked_add(60.seconds()).unwrap())
                }
            );
        }
    }

    async fn idle_fixture(
        env: &crate::test_support::TestEnv,
        kind: SessionKind,
    ) -> (UserId, RawToken, UtcInstant) {
        let user_id = SeedUser::new()
            .seed(env.users(), env.write_scope())
            .await
            .user_id;
        let sessions = env.sessions();
        let label = parse_session_label("idle policy");
        let raw = crate::test_support::confirmed_for(
            env.write_scope()
                .run(|transaction| {
                    Box::pin(async move {
                        sessions
                            .create_session(transaction, user_id, &label, kind)
                            .await
                    })
                })
                .await
                .unwrap(),
            "idle fixture",
        );
        let used = env.sessions().list_sessions(user_id).await.unwrap()[0].last_used_at;
        (user_id, raw, used)
    }

    #[apply(backends)]
    #[tokio::test]
    async fn session_idle_policy_uses_durable_kind_at_before_equal_and_after_cutoff(
        #[case] backend: Backend,
    ) {
        let env = backend.setup().await;
        for kind in [
            SessionKind::Browser,
            SessionKind::Machine,
            SessionKind::Unknown,
        ] {
            for elapsed in [2_591_999, 2_592_000, 2_592_001] {
                let (user_id, raw, used) = idle_fixture(&env, kind).await;
                let now = UtcInstant::from(used.value().checked_add(elapsed.seconds()).unwrap());
                let sessions = env.sessions();
                let result = env
                    .write_scope()
                    .run(|transaction| {
                        Box::pin(async move {
                            sessions
                                .authenticate(transaction, &raw, SessionClock::fixed(now))
                                .await
                        })
                    })
                    .await;
                let expired = kind == SessionKind::Browser && elapsed >= 2_592_000;
                if expired {
                    assert!(matches!(
                        result,
                        Err(crate::WriteScopeError::Operation(
                            SessionAuthError::SessionNotFound
                        ))
                    ));
                } else {
                    let authenticated =
                        crate::test_support::confirmed_for(result.unwrap(), "idle authentication");
                    assert!(authenticated.touched);
                    assert_eq!(authenticated.record.last_used_at, now);
                }
                assert_eq!(
                    env.sessions().list_sessions(user_id).await.unwrap()[0].last_used_at,
                    if expired { used } else { now }
                );
            }
        }
    }

    #[apply(backends)]
    #[tokio::test]
    async fn throttled_browser_use_does_not_extend_inactivity_or_revive_revoked_session(
        #[case] backend: Backend,
    ) {
        let env = backend.setup().await;
        let (user_id, raw, used) = idle_fixture(&env, SessionKind::Browser).await;
        for elapsed in [59, 2_592_000, 2_592_001] {
            let sessions = env.sessions();
            let token = raw.clone();
            let now = UtcInstant::from(used.value().checked_add(elapsed.seconds()).unwrap());
            let result = env
                .write_scope()
                .run(|transaction| {
                    Box::pin(async move {
                        sessions
                            .authenticate(transaction, &token, SessionClock::fixed(now))
                            .await
                    })
                })
                .await;
            if elapsed == 59 {
                assert!(
                    !crate::test_support::confirmed_for(result.unwrap(), "throttled use").touched
                );
            } else {
                assert!(matches!(
                    result,
                    Err(crate::WriteScopeError::Operation(
                        SessionAuthError::SessionNotFound
                    ))
                ));
            }
        }
        assert_eq!(
            env.sessions().list_sessions(user_id).await.unwrap()[0].last_used_at,
            used
        );
        let sessions = env.sessions();
        let hash = token::hash(&raw).unwrap();
        env.write_scope()
            .run(|transaction| {
                Box::pin(async move { sessions.revoke_session(transaction, &hash).await })
            })
            .await
            .unwrap();
        let sessions = env.sessions();
        assert!(matches!(
            env.write_scope()
                .run(|transaction| Box::pin(async move {
                    sessions
                        .authenticate(transaction, &raw, SessionClock::fixed(used))
                        .await
                }))
                .await,
            Err(crate::WriteScopeError::Operation(
                SessionAuthError::SessionNotFound
            ))
        ));
    }

    fn contending_scope(
        env: &crate::test_support::TestEnv,
    ) -> (
        crate::WriteScope,
        Arc<tokio::sync::Notify>,
        Arc<tokio::sync::Notify>,
    ) {
        let observed = Arc::new(tokio::sync::Notify::new());
        let retry = Arc::new(tokio::sync::Notify::new());
        (
            env.write_scope()
                .clone()
                .observing_sqlite_begin_contention(observed.clone(), retry.clone()),
            observed,
            retry,
        )
    }

    async fn contender_is_waiting_for_session_lock(
        env: &crate::test_support::TestEnv,
        backend: Backend,
        observed: &tokio::sync::Notify,
    ) -> bool {
        if matches!(backend, Backend::Sqlite) {
            return tokio::time::timeout(std::time::Duration::from_secs(5), observed.notified())
                .await
                .is_ok();
        }
        // Observe the database wait rather than task scheduling: expiry must not
        // reject a snapshot while an earlier eligible touch is uncommitted.
        tokio::time::timeout(std::time::Duration::from_secs(5), async {
            loop {
                let waiting = crate::with_closeable_pool!(env.base.pool(), pool, {
                    sqlx::query_scalar::<_, bool>("SELECT EXISTS (SELECT 1 FROM pg_stat_activity WHERE datname = current_database() AND wait_event_type = 'Lock' AND query LIKE 'SELECT s.token_hash%')")
                        .fetch_one(pool).await.unwrap()
                });
                if waiting { return; }
                tokio::task::yield_now().await;
            }
        }).await.is_ok()
    }

    #[apply(backends)]
    #[tokio::test]
    async fn serialized_touch_before_old_cutoff_governs_the_waiting_authentication(
        #[case] backend: Backend,
    ) {
        let env = backend.setup().await;
        let (_, raw, used) = idle_fixture(&env, SessionKind::Browser).await;
        let first_now = UtcInstant::from(used.value().checked_add(2_591_999.seconds()).unwrap());
        let later_now = UtcInstant::from(used.value().checked_add(2_592_000.seconds()).unwrap());
        let (locked_tx, locked_rx) = tokio::sync::oneshot::channel();
        let (release_tx, release_rx) = tokio::sync::oneshot::channel();
        let sessions = env.sessions();
        let scope = env.write_scope();
        let first_token = raw.clone();
        let first = tokio::spawn(async move {
            scope
                .run(|transaction| {
                    Box::pin(async move {
                        let authentication = sessions
                            .authenticate(transaction, &first_token, SessionClock::fixed(first_now))
                            .await?;
                        locked_tx.send(()).unwrap();
                        release_rx.await.unwrap();
                        Ok::<_, SessionAuthError>(authentication)
                    })
                })
                .await
        });
        locked_rx.await.unwrap();
        let sessions = env.sessions();
        let (scope, observed, retry) = contending_scope(&env);
        let second = tokio::spawn(async move {
            scope
                .run(|transaction| {
                    Box::pin(async move {
                        sessions
                            .authenticate(transaction, &raw, SessionClock::fixed(later_now))
                            .await
                    })
                })
                .await
        });
        let waited_for_lock = contender_is_waiting_for_session_lock(&env, backend, &observed).await;
        release_tx.send(()).unwrap();
        retry.notify_one();
        assert!(
            crate::test_support::confirmed_for(first.await.unwrap().unwrap(), "first touch")
                .touched
        );
        let later = crate::test_support::confirmed_for(
            second.await.unwrap().unwrap(),
            "waiting authentication",
        );
        assert!(!later.touched);
        assert_eq!(later.record.last_used_at, first_now);
        assert!(waited_for_lock, "expiry must wait for the Session lock");
    }

    #[apply(backends)]
    #[tokio::test]
    async fn session_wait_crossing_idle_cutoff_samples_clock_only_after_lock(
        #[case] backend: Backend,
    ) {
        use std::sync::atomic::{AtomicUsize, Ordering};
        let env = backend.setup().await;
        let (user_id, raw, used) = idle_fixture(&env, SessionKind::Browser).await;
        let before = UtcInstant::from(used.value().checked_add(2_591_999.seconds()).unwrap());
        let cutoff = UtcInstant::from(used.value().checked_add(2_592_000.seconds()).unwrap());
        let current = Arc::new(std::sync::Mutex::new(before));
        let reads = Arc::new(AtomicUsize::new(0));
        let clock = SessionClock::for_test({
            let current = current.clone();
            let reads = reads.clone();
            move || {
                reads.fetch_add(1, Ordering::SeqCst);
                *current.lock().unwrap()
            }
        });
        let (locked_tx, locked_rx) = tokio::sync::oneshot::channel();
        let (release_tx, release_rx) = tokio::sync::oneshot::channel();
        let sessions = env.sessions();
        let scope = env.write_scope();
        let first_token = raw.clone();
        let blocker = tokio::spawn(async move {
            scope
                .run(|transaction| {
                    Box::pin(async move {
                        let auth = sessions
                            .authenticate(transaction, &first_token, SessionClock::fixed(used))
                            .await?;
                        assert!(!auth.touched);
                        locked_tx.send(()).unwrap();
                        release_rx.await.unwrap();
                        Ok::<_, SessionAuthError>(auth)
                    })
                })
                .await
        });
        locked_rx.await.unwrap();
        let sessions = env.sessions();
        let (scope, observed, retry) = contending_scope(&env);
        let contender = tokio::spawn(async move {
            scope
                .run(|transaction| {
                    Box::pin(async move { sessions.authenticate(transaction, &raw, clock).await })
                })
                .await
        });
        let waited_for_lock = contender_is_waiting_for_session_lock(&env, backend, &observed).await;
        let premature_reads = reads.load(Ordering::SeqCst);
        *current.lock().unwrap() = cutoff;
        release_tx.send(()).unwrap();
        crate::test_support::confirmed_for(blocker.await.unwrap().unwrap(), "blocking auth");
        retry.notify_one();
        assert!(matches!(
            contender.await.unwrap(),
            Err(crate::WriteScopeError::Operation(
                SessionAuthError::SessionNotFound
            ))
        ));
        assert!(waited_for_lock);
        assert_eq!(
            premature_reads, 0,
            "request-start time must not become eligibility time"
        );
        assert_eq!(
            reads.load(Ordering::SeqCst),
            1,
            "one explicit time governs expiry and touch"
        );
        assert_eq!(
            env.sessions().list_sessions(user_id).await.unwrap()[0].last_used_at,
            used
        );
    }

    #[apply(backends)]
    #[tokio::test]
    async fn serialized_revoke_prevents_waiting_stale_cookie_revival(#[case] backend: Backend) {
        let env = backend.setup().await;
        let (user_id, raw, used) = idle_fixture(&env, SessionKind::Browser).await;
        let now = UtcInstant::from(used.value().checked_add(60.seconds()).unwrap());
        let (locked_tx, locked_rx) = tokio::sync::oneshot::channel();
        let (release_tx, release_rx) = tokio::sync::oneshot::channel();
        let sessions = env.sessions();
        let scope = env.write_scope();
        let hash = token::hash(&raw).unwrap();
        let revoke = tokio::spawn(async move {
            scope
                .run(|transaction| {
                    Box::pin(async move {
                        sessions.revoke_session(transaction, &hash).await?;
                        locked_tx.send(()).unwrap();
                        release_rx.await.unwrap();
                        Ok::<_, sqlx::Error>(())
                    })
                })
                .await
        });
        locked_rx.await.unwrap();
        let sessions = env.sessions();
        let (scope, observed, retry) = contending_scope(&env);
        let authentication = tokio::spawn(async move {
            scope
                .run(|transaction| {
                    Box::pin(async move {
                        sessions
                            .authenticate(transaction, &raw, SessionClock::fixed(now))
                            .await
                    })
                })
                .await
        });
        let waited_for_lock = contender_is_waiting_for_session_lock(&env, backend, &observed).await;
        release_tx.send(()).unwrap();
        crate::test_support::confirmed_for(revoke.await.unwrap().unwrap(), "revoke");
        retry.notify_one();
        assert!(matches!(
            authentication.await.unwrap(),
            Err(crate::WriteScopeError::Operation(
                SessionAuthError::SessionNotFound
            ))
        ));
        assert!(
            waited_for_lock,
            "authentication must wait for the earlier revoke"
        );
        assert!(
            env.sessions()
                .list_sessions(user_id)
                .await
                .unwrap()
                .is_empty()
        );
    }

    #[test]
    fn session_touch_cutoff_clamps_at_the_earliest_timestamp() {
        assert_eq!(
            session_touch_cutoff(UtcInstant::from(jiff::Timestamp::MIN)),
            UtcInstant::from(jiff::Timestamp::MIN)
        );
    }

    #[apply(backends)]
    #[tokio::test]
    async fn session_round_trips_token_hash_and_username(#[case] backend: Backend) {
        // Keep the whole `TestEnv` bound: dropping `base` unlinks the SQLite file
        // (ADR-0053 TempDir hazard).
        let env = backend.setup().await;
        let user_id = SeedUser::new()
            .seed(
                std::sync::Arc::clone(&env.users()),
                env.write_scope().clone(),
            )
            .await
            .user_id;

        // `create_session` binds the `TokenHash`; `authenticate`/`list_sessions`
        // decode the `token_hash` and joined `username` columns straight back into
        // their newtypes via the sqlx bridge (#438).
        let sessions = Arc::clone(&env.sessions());
        let label = parse_session_label("Test Device");
        let outcome = env
            .write_scope()
            .run(|transaction| {
                Box::pin(async move {
                    sessions
                        .create_session(transaction, user_id, &label, SessionKind::Unknown)
                        .await
                })
            })
            .await
            .unwrap();
        let raw_token = crate::test_support::confirmed_for(outcome, "session fixture setup");
        let expected_hash = token::hash(&raw_token).unwrap();

        let sessions = Arc::clone(&env.sessions());
        let outcome = env
            .write_scope()
            .run(|transaction| {
                Box::pin(async move {
                    sessions
                        .authenticate(transaction, &raw_token, SessionClock::default())
                        .await
                        .map(|auth| auth.record)
                })
            })
            .await
            .unwrap();
        let record = crate::test_support::confirmed_for(outcome, "session authentication");
        assert_eq!(record.token_hash, expected_hash);
        assert_eq!(record.user_id, user_id);

        let listed = env.sessions().list_sessions(user_id).await.unwrap();
        assert_eq!(listed.len(), 1);
        assert_eq!(listed[0].token_hash, expected_hash);
    }

    #[apply(backends)]
    #[tokio::test]
    async fn list_sessions_rejects_a_malformed_token_hash_column(#[case] backend: Backend) {
        let env = backend.setup().await;
        let user_id = SeedUser::new()
            .seed(
                std::sync::Arc::clone(&env.users()),
                env.write_scope().clone(),
            )
            .await
            .user_id;
        let sessions = Arc::clone(&env.sessions());
        let label = parse_session_label("Test Device");
        let outcome = env
            .write_scope()
            .run(|transaction| {
                Box::pin(async move {
                    sessions
                        .create_session(transaction, user_id, &label, SessionKind::Unknown)
                        .await
                })
            })
            .await
            .unwrap();
        assert!(matches!(outcome, common::MutationOutcome::Confirmed(_)));

        // Overwrite the `token_hash` column with a value `TokenHash::from_str`
        // rejects (a space is not a valid token character).
        let sql = "UPDATE sessions SET token_hash = $1";
        crate::with_closeable_pool!(env.base.pool(), pool, {
            sqlx::query(sql)
                .bind_storage(CorruptSessionTokenHash::malformed())
                .execute(pool)
                .await
                .unwrap();
        });

        // The read decodes the `token_hash` column into `TokenHash` via the sqlx
        // bridge, which validates through `FromStr`; the malformed value surfaces
        // as a column-decode error rather than being silently admitted (covers the
        // bridge's `Decode` error arm).
        let err = env.sessions().list_sessions(user_id).await.unwrap_err();
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
        let user_id = SeedUser::new()
            .seed(
                std::sync::Arc::clone(&env.users()),
                env.write_scope().clone(),
            )
            .await
            .user_id;
        let sessions = Arc::clone(&env.sessions());
        let label = parse_session_label("Test Device");
        let outcome = env
            .write_scope()
            .run(|transaction| {
                Box::pin(async move {
                    sessions
                        .create_session(transaction, user_id, &label, SessionKind::Unknown)
                        .await
                })
            })
            .await
            .unwrap();
        assert!(matches!(outcome, common::MutationOutcome::Confirmed(_)));
        let stored = StoredSessionLabel::new("x".repeat(1_000));
        crate::with_closeable_pool!(env.base.pool(), pool, {
            sqlx::query("UPDATE sessions SET label = $1")
                .bind_storage(&stored)
                .execute(pool)
                .await
                .unwrap();
        });

        let sessions = env.sessions().list_sessions(user_id).await.unwrap();
        assert_eq!(sessions.len(), 1);
        assert_eq!(sessions[0].label, SessionLabel::from_lossy(stored.as_str()));
    }
    #[apply(backends)]
    #[tokio::test]
    async fn session_rows_preserve_created_and_last_used_roles(#[case] backend: Backend) {
        let env = backend.setup().await;
        let user_id = SeedUser::new()
            .seed(
                std::sync::Arc::clone(&env.users()),
                env.write_scope().clone(),
            )
            .await
            .user_id;
        let sessions = Arc::clone(&env.sessions());
        let label = parse_session_label("Test Device");
        let outcome = env
            .write_scope()
            .run(|transaction| {
                Box::pin(async move {
                    sessions
                        .create_session(transaction, user_id, &label, SessionKind::Unknown)
                        .await
                })
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
            .bind_storage(created_at)
            .bind_storage(last_used_at)
            .bind_storage(&token_hash)
            .execute(pool)
            .await
            .unwrap();
        });

        let sessions = env.sessions().list_sessions(user_id).await.unwrap();
        assert_eq!(sessions.len(), 1);
        assert_eq!(sessions[0].created_at, created_at);
        assert_eq!(sessions[0].last_used_at, last_used_at);
    }

    #[apply(backends)]
    #[tokio::test]
    async fn touch_and_load_observes_stale_exact_and_fresh_boundaries(#[case] backend: Backend) {
        let env = backend.setup().await;
        let user_id = SeedUser::new()
            .seed(
                std::sync::Arc::clone(&env.users()),
                env.write_scope().clone(),
            )
            .await
            .user_id;
        let now = "2026-04-05T06:07:08.123456Z".parse::<UtcInstant>().unwrap();
        let stale_before = session_touch_cutoff(now);
        let cases = [
            (
                "stale",
                UtcInstant::from(
                    stale_before
                        .value()
                        .checked_sub(1.microsecond())
                        .expect("fixture is within Timestamp range"),
                ),
                now,
            ),
            ("exact", stale_before, now),
            (
                "fresh",
                UtcInstant::from(
                    stale_before
                        .value()
                        .checked_add(1.microsecond())
                        .expect("fixture is within Timestamp range"),
                ),
                UtcInstant::from(
                    stale_before
                        .value()
                        .checked_add(1.microsecond())
                        .expect("fixture is within Timestamp range"),
                ),
            ),
        ];

        for (label, stored_last_used_at, expected_last_used_at) in cases {
            let sessions = Arc::clone(&env.sessions());
            let label = parse_session_label(label);
            let label_for_create = label.clone();
            let outcome = env
                .write_scope()
                .run(move |transaction| {
                    Box::pin(async move {
                        sessions
                            .create_session(
                                transaction,
                                user_id,
                                &label_for_create,
                                SessionKind::Unknown,
                            )
                            .await
                    })
                })
                .await
                .unwrap();
            let raw_token = crate::test_support::confirmed_for(outcome, "session fixture setup");
            let token_hash = token::hash(&raw_token).unwrap();
            crate::with_closeable_pool!(env.base.pool(), pool, {
                sqlx::query("UPDATE sessions SET last_used_at = $1 WHERE token_hash = $2")
                    .bind_storage(stored_last_used_at)
                    .bind_storage(&token_hash)
                    .execute(pool)
                    .await
                    .unwrap();
            });
            let token_hash_for_touch = token_hash.clone();

            let outcome = env
                .write_scope()
                .run(move |transaction| {
                    Box::pin(async move {
                        match backend {
                            Backend::Sqlite => {
                                <sqlx::Sqlite as SessionDialect>::touch_and_load(
                                    transaction,
                                    &token_hash_for_touch,
                                    &SessionClock::fixed(now),
                                )
                                .await
                            }
                            Backend::Postgres => {
                                <sqlx::Postgres as SessionDialect>::touch_and_load(
                                    transaction,
                                    &token_hash_for_touch,
                                    &SessionClock::fixed(now),
                                )
                                .await
                            }
                        }
                    })
                })
                .await
                .unwrap();
            let row = match outcome {
                common::MutationOutcome::Confirmed(Some((row, _))) => row,
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
