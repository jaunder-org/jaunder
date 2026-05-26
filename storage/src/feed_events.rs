//! Queue of feed-regeneration events driven by post mutations and drained by
//! the feed worker. Rows transition pending → claimed → done|failed; stuck
//! claims are re-eligible after `lease_timeout` elapses (claim-lease pattern).

use async_trait::async_trait;
use chrono::{DateTime, Duration, Utc};
use thiserror::Error;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FeedEventStatus {
    Pending,
    Claimed,
    Done,
    Failed,
}

impl FeedEventStatus {
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            FeedEventStatus::Pending => "pending",
            FeedEventStatus::Claimed => "claimed",
            FeedEventStatus::Done => "done",
            FeedEventStatus::Failed => "failed",
        }
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
