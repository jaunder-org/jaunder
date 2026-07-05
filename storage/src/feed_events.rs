//! Queue of feed-regeneration events driven by post mutations and drained by
//! the feed worker. Rows transition pending → claimed → done|failed; stuck
//! claims are re-eligible after `lease_timeout` elapses (claim-lease pattern).

use async_trait::async_trait;
use chrono::{DateTime, Duration, Utc};
use sqlx::{Database, Pool};
use thiserror::Error;

use crate::backend::Backend;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FeedEventStatus {
    Pending,
    Claimed,
    Done,
    Failed,
}

/// Parses a persisted status string into a [`FeedEventStatus`], falling back to
/// `Failed` for any unrecognized value (defensive against schema drift). Shared
/// by both dialects' row mappers.
pub(crate) fn parse_status(s: &str) -> FeedEventStatus {
    match s {
        "pending" => FeedEventStatus::Pending,
        "claimed" => FeedEventStatus::Claimed,
        "done" => FeedEventStatus::Done,
        _ => FeedEventStatus::Failed,
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FeedEventRecord {
    pub id: i64,
    pub feed_url: String,
    pub status: FeedEventStatus,
    pub attempts: i32,
    pub last_error: Option<String>,
    pub next_attempt_at: DateTime<Utc>,
    pub claimed_at: Option<DateTime<Utc>>,
    pub created_at: DateTime<Utc>,
    pub regenerated_at: Option<DateTime<Utc>>,
    pub pinged_at: Option<DateTime<Utc>>,
}

#[derive(Debug, Error)]
pub enum FeedEventError {
    #[error("database error: {0}")]
    Db(#[from] sqlx::Error),
}

#[async_trait]
pub trait FeedEventStorage: Send + Sync {
    /// Insert a new `pending` row for `feed_url`. Returns the new row id.
    async fn enqueue(&self, feed_url: &str) -> Result<i64, FeedEventError>;

    /// Atomically claim up to `limit` rows that are either:
    ///   * `status = 'pending' AND next_attempt_at <= now`, or
    ///   * `status = 'claimed' AND claimed_at < now - lease_timeout`
    ///     (stuck-claim recovery).
    /// Transitions claimed rows to `status = 'claimed'` and stamps
    /// `claimed_at = now`.
    async fn claim_pending_batch(
        &self,
        limit: usize,
        lease_timeout: Duration,
    ) -> Result<Vec<FeedEventRecord>, FeedEventError>;

    /// Stamp `regenerated_at = now` on the given rows. Status is unchanged
    /// (still `claimed` until ping resolves).
    async fn mark_regenerated(&self, ids: &[i64]) -> Result<(), FeedEventError>;

    /// Transition rows to `status = 'done'` and stamp `pinged_at = now`.
    async fn mark_pinged(&self, ids: &[i64]) -> Result<(), FeedEventError>;

    /// Re-queue rows for another attempt: status back to `pending`,
    /// increment attempts, record the error, schedule the next attempt,
    /// and clear `claimed_at`.
    async fn mark_failed(
        &self,
        ids: &[i64],
        error: &str,
        next_attempt_at: DateTime<Utc>,
    ) -> Result<(), FeedEventError>;

    /// Terminal failure: status = 'failed', record the final error.
    async fn mark_exhausted(&self, ids: &[i64], error: &str) -> Result<(), FeedEventError>;
}

/// Backend-specific divergence for [`FeedEventStore`].
///
/// [`claim_pending_batch`][FeedEventDialect::claim_pending_batch] diverges in SQL
/// shape: both backends claim in a single `UPDATE … RETURNING` statement, but
/// Postgres uses a `FOR UPDATE SKIP LOCKED` CTE for inter-worker skip-locking,
/// while `SQLite` (which lacks `SKIP LOCKED`) drives the same write from an
/// `id IN (SELECT … LIMIT …)` subquery. `SQLite` must avoid the earlier
/// read-then-write transaction (SELECT ids → UPDATE → SELECT rows), which is
/// `SQLITE_BUSY`-prone under concurrency; see ADR-0021.
///
/// The bulk-id methods (`mark_regenerated`, `mark_pinged`, `mark_failed`,
/// `mark_exhausted`) also diverge: `SQLite` does not support array binding so
/// they use a dynamically-built `IN (?, ?, …)` pattern; Postgres uses
/// `WHERE id = ANY($n)` with a slice binding — a cleaner and cheaper approach.
#[async_trait]
pub trait FeedEventDialect: Backend {
    /// Atomically claim and return up to `limit` eligible rows.
    async fn claim_pending_batch(
        pool: &Pool<Self>,
        now: DateTime<Utc>,
        lease_cutoff: DateTime<Utc>,
        limit_i: i64,
    ) -> Result<Vec<FeedEventRecord>, FeedEventError>;

    /// Stamp `regenerated_at = now` on all rows whose id is in `ids`.
    async fn mark_regenerated(pool: &Pool<Self>, ids: &[i64]) -> Result<(), FeedEventError>;

    /// Transition rows to `done` and stamp `pinged_at = now`.
    async fn mark_pinged(pool: &Pool<Self>, ids: &[i64]) -> Result<(), FeedEventError>;

    /// Re-queue rows for another attempt.
    async fn mark_failed(
        pool: &Pool<Self>,
        ids: &[i64],
        error: &str,
        next_attempt_at: DateTime<Utc>,
    ) -> Result<(), FeedEventError>;

    /// Terminal failure: set `status = 'failed'` and record the final error.
    async fn mark_exhausted(
        pool: &Pool<Self>,
        ids: &[i64],
        error: &str,
    ) -> Result<(), FeedEventError>;
}

/// Generic [`FeedEventStorage`] backed by any [`FeedEventDialect`] database.
///
/// Only `enqueue` is shared directly here (its SQL is identical across
/// backends). All other methods delegate to [`FeedEventDialect`] because they
/// diverge in either transaction strategy or bulk-id binding approach.
/// See ADR-0019.
pub struct FeedEventStore<DB: Database> {
    pool: Pool<DB>,
}

impl<DB: Database> FeedEventStore<DB> {
    #[must_use]
    pub fn new(pool: Pool<DB>) -> Self {
        Self { pool }
    }
}

#[async_trait]
impl<DB> FeedEventStorage for FeedEventStore<DB>
where
    DB: FeedEventDialect,
    for<'q> &'q str: sqlx::Encode<'q, DB> + sqlx::Type<DB>,
    for<'c> &'c Pool<DB>: sqlx::Executor<'c, Database = DB>,
    for<'q> DB::Arguments<'q>: sqlx::IntoArguments<'q, DB>,
    (i64,): for<'r> sqlx::FromRow<'r, DB::Row>,
{
    #[tracing::instrument(
        name = "storage.feed_events.enqueue",
        skip(self),
        fields(db.system = DB::DB_SYSTEM)
    )]
    async fn enqueue(&self, feed_url: &str) -> Result<i64, FeedEventError> {
        let id: i64 =
            sqlx::query_scalar("INSERT INTO feed_events (feed_url) VALUES ($1) RETURNING id")
                .bind(feed_url)
                .fetch_one(&self.pool)
                .await?;
        Ok(id)
    }

    #[tracing::instrument(
        name = "storage.feed_events.claim_pending_batch",
        skip(self),
        fields(db.system = DB::DB_SYSTEM)
    )]
    async fn claim_pending_batch(
        &self,
        limit: usize,
        lease_timeout: Duration,
    ) -> Result<Vec<FeedEventRecord>, FeedEventError> {
        let now = Utc::now();
        let lease_cutoff = now - lease_timeout;
        let limit_i = i64::try_from(limit).unwrap_or(i64::MAX);
        DB::claim_pending_batch(&self.pool, now, lease_cutoff, limit_i).await
    }

    #[tracing::instrument(
        name = "storage.feed_events.mark_regenerated",
        skip(self),
        fields(db.system = DB::DB_SYSTEM)
    )]
    async fn mark_regenerated(&self, ids: &[i64]) -> Result<(), FeedEventError> {
        if ids.is_empty() {
            return Ok(());
        }
        DB::mark_regenerated(&self.pool, ids).await
    }

    #[tracing::instrument(
        name = "storage.feed_events.mark_pinged",
        skip(self),
        fields(db.system = DB::DB_SYSTEM)
    )]
    async fn mark_pinged(&self, ids: &[i64]) -> Result<(), FeedEventError> {
        if ids.is_empty() {
            return Ok(());
        }
        DB::mark_pinged(&self.pool, ids).await
    }

    #[tracing::instrument(
        name = "storage.feed_events.mark_failed",
        skip(self),
        fields(db.system = DB::DB_SYSTEM)
    )]
    async fn mark_failed(
        &self,
        ids: &[i64],
        error: &str,
        next_attempt_at: DateTime<Utc>,
    ) -> Result<(), FeedEventError> {
        if ids.is_empty() {
            return Ok(());
        }
        DB::mark_failed(&self.pool, ids, error, next_attempt_at).await
    }

    #[tracing::instrument(
        name = "storage.feed_events.mark_exhausted",
        skip(self),
        fields(db.system = DB::DB_SYSTEM)
    )]
    async fn mark_exhausted(&self, ids: &[i64], error: &str) -> Result<(), FeedEventError> {
        if ids.is_empty() {
            return Ok(());
        }
        DB::mark_exhausted(&self.pool, ids, error).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_status_handles_all_statuses() {
        assert_eq!(parse_status("pending"), FeedEventStatus::Pending);
        assert_eq!(parse_status("claimed"), FeedEventStatus::Claimed);
        assert_eq!(parse_status("done"), FeedEventStatus::Done);
        assert_eq!(parse_status("failed"), FeedEventStatus::Failed);
        // Defensive fallback for unknown status strings.
        assert_eq!(parse_status("???"), FeedEventStatus::Failed);
    }
}
