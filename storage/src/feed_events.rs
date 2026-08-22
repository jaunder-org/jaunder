//! Queue of feed-regeneration events driven by post mutations and drained by
//! the feed worker. Rows transition pending → claimed → done|failed; stuck
//! claims are re-eligible after `lease_timeout` elapses (claim-lease pattern).

use async_trait::async_trait;
use chrono::{DateTime, Duration, Utc};
use common::feed::{FeedEventClaimLimit, FeedPath};
use common::ids::FeedEventId;
use sqlx::{Database, Pool};
use thiserror::Error;

use crate::backend::Backend;

// Re-exported so `server`'s feed worker keeps importing it from `storage` alongside
// `FeedEventRecord`; the type itself lives in `common` (#728, see its doc for why).
pub use common::feed::FeedEventStatus;

#[derive(Debug, Clone, PartialEq, Eq, sqlx::FromRow)]
pub struct FeedEventRecord {
    pub id: FeedEventId,
    // The column is `feed_url`; the field is the domain name. The derive binds by field
    // name, so without this every claim fails at runtime rather than at compile time.
    #[sqlx(rename = "feed_url")]
    pub feed_path: FeedPath,
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

/// One row of a claim batch: either a decoded record, or the id of a row whose `feed_url`
/// will not parse and which must therefore be purged.
///
/// The claim query cannot simply decode into [`FeedEventRecord`]. A `feed_url` that will
/// not parse can only come from DB tampering or a grammar that has since been tightened;
/// such a row names no identifiable feed, so it is an unactionable work item. Failing the
/// whole batch on it would wedge the worker on that row forever, which is what
/// [`purge_corrupt`](crate::postgres::feed_events) exists to prevent.
/// A two-variant enum rather than a `Result`: [`ClaimedRow::Corrupt`] is not an error, it
/// is a decided outcome — the row was read, understood to be unusable, and routed to the
/// purge list. Spelling it `Err` invites the next reader to `?` it and lose the id.
pub(crate) enum ClaimedRow {
    Record(Box<FeedEventRecord>),
    Corrupt(FeedEventId),
}

/// **The diversion is column-scoped, and that is the whole point of this impl**
/// (docs/adr/0122-one-bad-row-must-not-stop-the-scan.md): `purge_corrupt`
/// DELETEs, so only a `feed_url` decode failure may divert a row — re-checked
/// specifically here — and anything else propagates (#728).
///
/// If `id` itself will not decode there is no third state: the error propagates and the
/// batch fails. Stated so it is a decision rather than an accident.
impl<'r, R> sqlx::FromRow<'r, R> for ClaimedRow
where
    R: sqlx::Row,
    &'r str: sqlx::ColumnIndex<R>,
    FeedEventRecord: sqlx::FromRow<'r, R>,
    FeedPath: sqlx::Decode<'r, R::Database> + sqlx::Type<R::Database>,
    FeedEventId: sqlx::Decode<'r, R::Database> + sqlx::Type<R::Database>,
{
    fn from_row(row: &'r R) -> sqlx::Result<Self> {
        match FeedEventRecord::from_row(row) {
            Ok(record) => Ok(Self::Record(Box::new(record))),
            Err(_) if row.try_get::<FeedPath, _>("feed_url").is_err() => {
                Ok(Self::Corrupt(row.try_get::<FeedEventId, _>("id")?))
            }
            Err(e) => Err(e),
        }
    }
}

/// Splits a claim batch into the records the worker can act on and the ids to
/// purge. One-or-more corrupt rows produce one redacted report for the whole
/// batch; the stored value is never retained or rendered.
pub(crate) fn partition_claimed(rows: Vec<ClaimedRow>) -> (Vec<FeedEventRecord>, Vec<FeedEventId>) {
    let mut records = Vec::with_capacity(rows.len());
    let mut corrupt = Vec::new();
    for row in rows {
        match row {
            ClaimedRow::Record(record) => records.push(*record),
            ClaimedRow::Corrupt(id) => corrupt.push(id),
        }
    }
    if !corrupt.is_empty() {
        host::error::report_swallowed(
            host::error::ErrorKind::Storage,
            host::error::ErrorClass::Bug,
            "storage.feed_events.decode_feed_path",
            host::error::SwallowedSource::Redacted,
        );
    }
    (records, corrupt)
}

/// Returns the already-partitioned valid batch while reporting a failed purge
/// once at the dialect's useful aggregation boundary.
pub(crate) fn finish_corrupt_purge(
    records: Vec<FeedEventRecord>,
    purge: Result<(), sqlx::Error>,
    context: &'static str,
) -> Vec<FeedEventRecord> {
    crate::helpers::preserve_after_secondary(
        records,
        purge,
        host::error::ErrorKind::Storage,
        host::error::ErrorClass::Transient,
        context,
    )
}

#[cfg_attr(feature = "test-utils", mockall::automock)]
#[async_trait]
pub trait FeedEventStorage: Send + Sync {
    /// Insert a new `pending` row for `feed_path`. Returns the new row id.
    ///
    /// Single-item API. Do NOT call this in a loop from production code —
    /// per-row autocommit write loops are the `SQLite` lock-churn failure mode
    /// diagnosed in #766; a fan-out uses [`enqueue_many`](Self::enqueue_many).
    async fn enqueue(&self, feed_path: &FeedPath) -> Result<FeedEventId, FeedEventError>;

    /// Insert `pending` rows for every path in `feed_paths`, in ONE write-first
    /// transaction — a single write-lock acquisition for the whole batch.
    /// Production fan-outs MUST use this, not per-row `enqueue`: per-row
    /// autocommit loops are the `SQLite` lock-churn failure mode diagnosed in
    /// #766. Duplicates are inserted as-is; the drain dedupes by grouping on
    /// `feed_path`.
    async fn enqueue_many(&self, feed_paths: &[FeedPath]) -> Result<(), FeedEventError>;

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

    /// Count rows currently claimable by the feed worker without claiming them.
    async fn claimable_count(&self, lease_timeout: Duration) -> Result<u64, FeedEventError>;

    /// Stamp `regenerated_at = now` on the given rows. Status is unchanged
    /// (still `claimed` until ping resolves).
    async fn mark_regenerated(&self, ids: &[FeedEventId]) -> Result<(), FeedEventError>;

    /// Transition rows to `status = 'done'` and stamp `pinged_at = now`.
    async fn mark_pinged(&self, ids: &[FeedEventId]) -> Result<(), FeedEventError>;

    /// Re-queue rows for another attempt: status back to `pending`,
    /// increment attempts, record the error, schedule the next attempt,
    /// and clear `claimed_at`.
    async fn mark_failed(
        &self,
        ids: &[FeedEventId],
        error: &str,
        next_attempt_at: DateTime<Utc>,
    ) -> Result<(), FeedEventError>;

    /// Terminal failure: status = 'failed', record the final error.
    async fn mark_exhausted(&self, ids: &[FeedEventId], error: &str) -> Result<(), FeedEventError>;
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
        limit: FeedEventClaimLimit,
    ) -> Result<Vec<FeedEventRecord>, FeedEventError>;

    /// Count rows eligible under the same predicate as `claim_pending_batch`.
    async fn claimable_count(
        pool: &Pool<Self>,
        now: DateTime<Utc>,
        lease_cutoff: DateTime<Utc>,
    ) -> Result<u64, FeedEventError>;

    /// Stamp `regenerated_at = now` on all rows whose id is in `ids`.
    async fn mark_regenerated(pool: &Pool<Self>, ids: &[FeedEventId])
    -> Result<(), FeedEventError>;

    /// Transition rows to `done` and stamp `pinged_at = now`.
    async fn mark_pinged(pool: &Pool<Self>, ids: &[FeedEventId]) -> Result<(), FeedEventError>;

    /// Re-queue rows for another attempt.
    async fn mark_failed(
        pool: &Pool<Self>,
        ids: &[FeedEventId],
        error: &str,
        next_attempt_at: DateTime<Utc>,
    ) -> Result<(), FeedEventError>;

    /// Terminal failure: set `status = 'failed'` and record the final error.
    async fn mark_exhausted(
        pool: &Pool<Self>,
        ids: &[FeedEventId],
        error: &str,
    ) -> Result<(), FeedEventError>;
}

/// Generic [`FeedEventStorage`] backed by any [`FeedEventDialect`] database.
///
/// `enqueue` and `enqueue_many` are shared directly here (their SQL is
/// identical across backends). All other methods delegate to
/// [`FeedEventDialect`] because they diverge in either transaction strategy or
/// bulk-id binding approach. See ADR-0019.
pub struct FeedEventStore<DB: Database> {
    pool: Pool<DB>,
}

/// The one enqueue statement, shared by [`FeedEventStorage::enqueue`] (which
/// appends `RETURNING id`) and [`FeedEventStorage::enqueue_many`] — a column
/// change edits it once.
const INSERT_FEED_EVENT: &str = "INSERT INTO feed_events (feed_url) VALUES ($1)";

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
    // `FeedPath` binds and decodes as itself via the ADR-0071 sqlx bridge (the
    // `enqueue` bind encodes `&FeedPath`; the per-dialect claim row-mappers
    // decode the `feed_url` column into `FeedPath`).
    String: sqlx::Type<DB>,
    for<'q> String: sqlx::Encode<'q, DB>,
    for<'c> &'c Pool<DB>: sqlx::Executor<'c, Database = DB>,
    // `enqueue_many` executes on `&mut *tx` inside its batching transaction
    // (#766); same precedent as the posts generic impl.
    for<'c> &'c mut DB::Connection: sqlx::Executor<'c, Database = DB>,
    for<'q> DB::Arguments<'q>: sqlx::IntoArguments<'q, DB>,
    (FeedEventId,): for<'r> sqlx::FromRow<'r, DB::Row>,
{
    #[tracing::instrument(
        name = "storage.feed_events.enqueue",
        skip(self),
        fields(db.system = DB::DB_SYSTEM)
    )]
    async fn enqueue(&self, feed_path: &FeedPath) -> Result<FeedEventId, FeedEventError> {
        let sql = format!("{INSERT_FEED_EVENT} RETURNING id");
        let id = sqlx::query_scalar::<_, FeedEventId>(&sql)
            .bind(feed_path)
            .fetch_one(&self.pool)
            .await?;
        Ok(id)
    }

    #[tracing::instrument(
        name = "storage.feed_events.enqueue_many",
        skip(self, feed_paths),
        fields(db.system = DB::DB_SYSTEM, count = feed_paths.len())
    )]
    async fn enqueue_many(&self, feed_paths: &[FeedPath]) -> Result<(), FeedEventError> {
        if feed_paths.is_empty() {
            return Ok(());
        }
        // One write-first transaction: a single write-lock acquisition (and one
        // WAL sync) for the whole batch, instead of one per row (#766). First
        // statement is a write, so no deferred-upgrade hazard (ADR-0021) — and
        // write-first is also the only shape available here: the generic impl
        // has no dialect hook for `BEGIN IMMEDIATE`.
        let mut tx = self.pool.begin().await?;
        for feed_path in feed_paths {
            sqlx::query(INSERT_FEED_EVENT)
                .bind(feed_path)
                .execute(&mut *tx)
                .await?;
        }
        tx.commit().await?;
        Ok(())
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
        let limit = FeedEventClaimLimit::from_usize(limit);
        DB::claim_pending_batch(&self.pool, now, lease_cutoff, limit).await
    }

    #[tracing::instrument(
        name = "storage.feed_events.claimable_count",
        skip(self),
        fields(db.system = DB::DB_SYSTEM)
    )]
    async fn claimable_count(&self, lease_timeout: Duration) -> Result<u64, FeedEventError> {
        let now = Utc::now();
        let lease_cutoff = now - lease_timeout;
        DB::claimable_count(&self.pool, now, lease_cutoff).await
    }

    #[tracing::instrument(
        name = "storage.feed_events.mark_regenerated",
        skip(self),
        fields(db.system = DB::DB_SYSTEM)
    )]
    async fn mark_regenerated(&self, ids: &[FeedEventId]) -> Result<(), FeedEventError> {
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
    async fn mark_pinged(&self, ids: &[FeedEventId]) -> Result<(), FeedEventError> {
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
        ids: &[FeedEventId],
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
    async fn mark_exhausted(&self, ids: &[FeedEventId], error: &str) -> Result<(), FeedEventError> {
        if ids.is_empty() {
            return Ok(());
        }
        DB::mark_exhausted(&self.pool, ids, error).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::{Backend, backends, fp};
    use rstest::*;
    use rstest_reuse::*;

    // The token ↔ variant mapping is the `text_enum` attribute's, tested at the type
    // in `common/src/feed/event_status.rs`.

    #[apply(backends)]
    #[tokio::test]
    async fn enqueue_creates_pending_row(#[case] backend: Backend) {
        let env = backend.setup().await;
        let id = env
            .state
            .feed_events
            .enqueue(&fp("/feed.rss"))
            .await
            .unwrap();
        assert!(i64::from(id) > 0);
    }

    #[apply(backends)]
    #[tokio::test]
    async fn enqueue_many_creates_pending_rows_in_one_batch(#[case] backend: Backend) {
        let env = backend.setup().await;
        let paths = [
            fp("/feed.rss"),
            fp("/~alice/feed.rss"),
            fp("/tags/t/feed.rss"),
        ];
        env.state.feed_events.enqueue_many(&paths).await.unwrap();

        let claimed = env
            .state
            .feed_events
            .claim_pending_batch(10, chrono::Duration::minutes(5))
            .await
            .unwrap();
        // FeedPath has no Ord (deliberate), so compare as sets.
        let urls: std::collections::HashSet<_> =
            claimed.iter().map(|r| r.feed_path.clone()).collect();
        let expected: std::collections::HashSet<_> = paths.iter().cloned().collect();
        assert_eq!(urls, expected);
    }

    #[apply(backends)]
    #[tokio::test]
    async fn enqueue_many_inserts_duplicates_as_is(#[case] backend: Backend) {
        let env = backend.setup().await;
        // No dedupe: the drain groups by feed_path, so duplicate rows are
        // harmless — pin that enqueue_many does not silently collapse them.
        let paths = [fp("/feed.rss"), fp("/feed.rss")];
        env.state.feed_events.enqueue_many(&paths).await.unwrap();

        let claimed = env
            .state
            .feed_events
            .claim_pending_batch(10, chrono::Duration::minutes(5))
            .await
            .unwrap();
        assert_eq!(claimed.len(), 2);
    }

    #[apply(backends)]
    #[tokio::test]
    async fn enqueue_many_empty_input_is_a_no_op(#[case] backend: Backend) {
        let env = backend.setup().await;
        env.state.feed_events.enqueue_many(&[]).await.unwrap();

        let claimed = env
            .state
            .feed_events
            .claim_pending_batch(10, chrono::Duration::minutes(5))
            .await
            .unwrap();
        assert!(claimed.is_empty());
    }

    #[apply(backends)]
    #[tokio::test]
    async fn claimable_count_counts_pending_ready_rows(#[case] backend: Backend) {
        let env = backend.setup().await;
        env.state
            .feed_events
            .enqueue(&fp("/feed.rss"))
            .await
            .unwrap();

        let count = env
            .state
            .feed_events
            .claimable_count(chrono::Duration::minutes(5))
            .await
            .unwrap();

        assert_eq!(count, 1);
    }

    #[apply(backends)]
    #[tokio::test]
    async fn claimable_count_ignores_delayed_retries(#[case] backend: Backend) {
        let env = backend.setup().await;
        let id = env
            .state
            .feed_events
            .enqueue(&fp("/feed.rss"))
            .await
            .unwrap();
        env.state
            .feed_events
            .mark_failed(
                &[id],
                "retry later",
                chrono::Utc::now() + chrono::Duration::hours(1),
            )
            .await
            .unwrap();

        let count = env
            .state
            .feed_events
            .claimable_count(chrono::Duration::minutes(5))
            .await
            .unwrap();

        assert_eq!(count, 0);
    }

    #[apply(backends)]
    #[tokio::test]
    async fn claimable_count_ignores_live_claims(#[case] backend: Backend) {
        let env = backend.setup().await;
        env.state
            .feed_events
            .enqueue(&fp("/feed.rss"))
            .await
            .unwrap();
        let claimed = env
            .state
            .feed_events
            .claim_pending_batch(10, chrono::Duration::minutes(5))
            .await
            .unwrap();
        assert_eq!(claimed.len(), 1);

        let count = env
            .state
            .feed_events
            .claimable_count(chrono::Duration::minutes(5))
            .await
            .unwrap();

        assert_eq!(count, 0);
    }

    #[apply(backends)]
    #[tokio::test]
    async fn claimable_count_counts_expired_claims(#[case] backend: Backend) {
        let env = backend.setup().await;
        env.state
            .feed_events
            .enqueue(&fp("/feed.rss"))
            .await
            .unwrap();
        env.state
            .feed_events
            .claim_pending_batch(10, chrono::Duration::minutes(5))
            .await
            .unwrap();

        let count = env
            .state
            .feed_events
            .claimable_count(chrono::Duration::zero())
            .await
            .unwrap();

        assert_eq!(count, 1);
    }

    #[apply(backends)]
    #[tokio::test]
    async fn claim_purges_rows_with_unparseable_feed_url(#[case] backend: Backend) {
        let env = backend.setup().await;
        // Simulate DB tampering/corruption: a feed_url that bypasses FeedPath
        // validation (which `enqueue` could never write), alongside a valid row.
        env.base
            .pool()
            .execute("INSERT INTO feed_events (feed_url) VALUES ('not a feed path')")
            .await
            .unwrap();
        env.state
            .feed_events
            .enqueue(&fp("/feed.rss"))
            .await
            .unwrap();

        // The claim skips-and-purges the corrupt row and returns only the valid
        // one — the batch is NOT failed (which would wedge the worker forever).
        // The batch-level report is redacted rather than retaining the bad value.
        let (claimed, trace) = crate::helpers::swallowed_test::capture_async(
            env.state
                .feed_events
                .claim_pending_batch(50, chrono::Duration::minutes(5)),
        )
        .await;
        let claimed = claimed.unwrap();
        crate::helpers::swallowed_test::assert_one_report(
            &trace,
            "storage.feed_events.decode_feed_path",
        );
        assert!(
            !trace.contains("not a feed path"),
            "trace leaked stored value"
        );
        assert_eq!(claimed.len(), 1);
        assert_eq!(claimed[0].feed_path, "/feed.rss");

        // The corrupt row was deleted, not merely skipped, so it can never wedge
        // a future claim either.
        let corrupt_remaining = env
            .base
            .pool()
            .scalar_i64("SELECT COUNT(*) FROM feed_events WHERE feed_url = 'not a feed path'")
            .await
            .unwrap();
        assert_eq!(corrupt_remaining, 0, "corrupt row should be purged");
    }

    #[apply(backends)]
    #[tokio::test]
    async fn a_decode_failure_outside_feed_url_propagates_and_deletes_nothing(
        #[case] backend: Backend,
    ) {
        // The property that keeps the purge path narrow. `purge_corrupt` DELETEs, and
        // exactly one column may trigger it. A `ClaimedRow` that treated *any* decode
        // failure as "corrupt" would turn a schema change or a driver regression on some
        // unrelated column into silent data loss — the queue would drain itself.
        let env = backend.setup().await;
        env.state
            .feed_events
            .enqueue(&fp("/feed.rss"))
            .await
            .unwrap();
        // Widen `attempts` past `i32` and store an out-of-range value — the shape of a
        // migration that grows a column without the Rust side following. `status` would
        // be the obvious lever now that it is a closed enum, but the claim's own
        // eligibility predicate filters on `status`, so a bad one is never selected (D5).
        if matches!(backend, Backend::Postgres) {
            env.base
                .pool()
                .execute("ALTER TABLE feed_events ALTER COLUMN attempts TYPE bigint")
                .await
                .unwrap();
        }
        env.base
            .pool()
            .execute("UPDATE feed_events SET attempts = 3000000000")
            .await
            .unwrap();

        let err = env
            .state
            .feed_events
            .claim_pending_batch(50, chrono::Duration::minutes(5))
            .await
            .expect_err("a non-feed_url decode failure must propagate");
        assert!(
            matches!(err, FeedEventError::Db(sqlx::Error::ColumnDecode { .. })),
            "expected a column-decode error, got {err:?}"
        );

        // …and above all, the row is still there.
        let remaining = env
            .base
            .pool()
            .scalar_i64("SELECT COUNT(*) FROM feed_events")
            .await
            .unwrap();
        assert_eq!(
            remaining, 1,
            "a decode failure outside feed_url must never delete the row"
        );
    }

    #[apply(backends)]
    #[tokio::test]
    async fn continuation_reporting_failed_corrupt_purge_preserves_later_valid_row_and_reports_both_failures_once(
        #[case] backend: Backend,
    ) {
        let env = backend.setup().await;
        env.base
            .pool()
            .execute("INSERT INTO feed_events (feed_url) VALUES ('not a feed path')")
            .await
            .unwrap();
        env.state
            .feed_events
            .enqueue(&fp("/feed.rss"))
            .await
            .unwrap();
        env.base
            .pool()
            .execute(
                "CREATE TABLE feed_event_delete_guard (\
                    feed_event_id BIGINT PRIMARY KEY \
                    REFERENCES feed_events(id) ON DELETE RESTRICT\
                )",
            )
            .await
            .unwrap();
        env.base
            .pool()
            .execute(
                "INSERT INTO feed_event_delete_guard (feed_event_id) \
                 SELECT id FROM feed_events WHERE feed_url = 'not a feed path'",
            )
            .await
            .unwrap();

        let (claimed, trace) = crate::helpers::swallowed_test::capture_async(
            env.state
                .feed_events
                .claim_pending_batch(50, chrono::Duration::minutes(5)),
        )
        .await;
        let claimed = claimed.unwrap();
        assert_eq!(claimed.len(), 1);
        assert_eq!(claimed[0].feed_path, "/feed.rss");

        let purge_context = match backend {
            Backend::Sqlite => "storage.sqlite.feed_events.purge_corrupt",
            Backend::Postgres => "storage.postgres.feed_events.purge_corrupt",
        };
        assert_eq!(
            trace
                .matches(r#""error.context":"storage.feed_events.decode_feed_path""#)
                .count(),
            1,
            "trace: {trace}"
        );
        assert_eq!(
            trace
                .matches(format!(r#""error.context":"{purge_context}""#).as_str())
                .count(),
            1,
            "trace: {trace}"
        );
        assert_eq!(
            trace.matches(r#""error.disposition":"swallowed""#).count(),
            2,
            "trace: {trace}"
        );
        assert!(
            !trace.contains("not a feed path"),
            "trace leaked stored value"
        );

        let corrupt_remaining = env
            .base
            .pool()
            .scalar_i64("SELECT COUNT(*) FROM feed_events WHERE feed_url = 'not a feed path'")
            .await
            .unwrap();
        assert_eq!(
            corrupt_remaining, 1,
            "failed purge must leave the corrupt row in place"
        );
    }

    #[apply(backends)]
    #[tokio::test]
    async fn claim_returns_eligible_pending_row(#[case] backend: Backend) {
        let env = backend.setup().await;
        env.state
            .feed_events
            .enqueue(&fp("/feed.rss"))
            .await
            .unwrap();
        let claimed = env
            .state
            .feed_events
            .claim_pending_batch(10, chrono::Duration::minutes(5))
            .await
            .unwrap();
        assert_eq!(claimed.len(), 1);
        assert_eq!(claimed[0].status, FeedEventStatus::Claimed);
        assert!(claimed[0].claimed_at.is_some());
    }

    #[apply(backends)]
    #[tokio::test]
    async fn double_claim_returns_no_rows_within_lease(#[case] backend: Backend) {
        let env = backend.setup().await;
        env.state
            .feed_events
            .enqueue(&fp("/feed.rss"))
            .await
            .unwrap();
        let first = env
            .state
            .feed_events
            .claim_pending_batch(10, chrono::Duration::minutes(5))
            .await
            .unwrap();
        let second = env
            .state
            .feed_events
            .claim_pending_batch(10, chrono::Duration::minutes(5))
            .await
            .unwrap();
        assert_eq!(first.len(), 1);
        assert_eq!(second.len(), 0);
    }

    #[apply(backends)]
    #[tokio::test]
    async fn lease_expired_rows_are_reclaimable(#[case] backend: Backend) {
        let env = backend.setup().await;
        env.state
            .feed_events
            .enqueue(&fp("/feed.rss"))
            .await
            .unwrap();
        let _first = env
            .state
            .feed_events
            .claim_pending_batch(10, chrono::Duration::minutes(5))
            .await
            .unwrap();
        // With a zero lease, the just-claimed row is immediately re-eligible.
        let second = env
            .state
            .feed_events
            .claim_pending_batch(10, chrono::Duration::zero())
            .await
            .unwrap();
        assert_eq!(second.len(), 1);
    }

    #[apply(backends)]
    #[tokio::test]
    async fn mark_pinged_marks_done_and_removes_from_queue(#[case] backend: Backend) {
        let env = backend.setup().await;
        env.state
            .feed_events
            .enqueue(&fp("/feed.rss"))
            .await
            .unwrap();
        let claimed = env
            .state
            .feed_events
            .claim_pending_batch(10, chrono::Duration::minutes(5))
            .await
            .unwrap();
        let ids: Vec<FeedEventId> = claimed.iter().map(|r| r.id).collect();
        env.state.feed_events.mark_regenerated(&ids).await.unwrap();
        env.state.feed_events.mark_pinged(&ids).await.unwrap();
        let next = env
            .state
            .feed_events
            .claim_pending_batch(10, chrono::Duration::minutes(5))
            .await
            .unwrap();
        assert!(next.is_empty());
    }

    #[apply(backends)]
    #[tokio::test]
    async fn mark_failed_increments_attempts_and_reschedules(#[case] backend: Backend) {
        let env = backend.setup().await;
        let id = env
            .state
            .feed_events
            .enqueue(&fp("/feed.rss"))
            .await
            .unwrap();
        let _ = env
            .state
            .feed_events
            .claim_pending_batch(10, chrono::Duration::minutes(5))
            .await
            .unwrap();
        let future = Utc::now() + chrono::Duration::minutes(1);
        env.state
            .feed_events
            .mark_failed(&[id], "boom", future)
            .await
            .unwrap();
        // Not eligible until `future`.
        let now = env
            .state
            .feed_events
            .claim_pending_batch(10, chrono::Duration::minutes(5))
            .await
            .unwrap();
        assert!(now.is_empty());
    }

    #[apply(backends)]
    #[tokio::test]
    async fn mark_exhausted_marks_failed_terminal(#[case] backend: Backend) {
        let env = backend.setup().await;
        let id = env
            .state
            .feed_events
            .enqueue(&fp("/feed.rss"))
            .await
            .unwrap();
        env.state
            .feed_events
            .mark_exhausted(&[id], "gave up")
            .await
            .unwrap();
        // Failed rows are never eligible.
        let next = env
            .state
            .feed_events
            .claim_pending_batch(10, chrono::Duration::minutes(5))
            .await
            .unwrap();
        assert!(next.is_empty());
    }

    #[apply(backends)]
    #[tokio::test]
    async fn empty_id_arrays_are_noops(#[case] backend: Backend) {
        let env = backend.setup().await;
        env.state.feed_events.mark_regenerated(&[]).await.unwrap();
        env.state.feed_events.mark_pinged(&[]).await.unwrap();
        env.state
            .feed_events
            .mark_failed(&[], "x", Utc::now())
            .await
            .unwrap();
        env.state
            .feed_events
            .mark_exhausted(&[], "x")
            .await
            .unwrap();
    }
}
