//! Queue of feed-regeneration events driven by post mutations and drained by
//! the feed worker. Rows transition pending → claimed → done|failed; stuck
//! claims are re-eligible after `lease_timeout` elapses (claim-lease pattern).

use std::str::FromStr;

use async_trait::async_trait;
use chrono::Duration;
use common::ids::FeedEventId;
use common::pagination::RowLimit;
use common::time::UtcInstant;
use host::feed::{FeedEventClaimLimit, FeedEventStatus, FeedPath};
use sqlx::{Database, Pool};
use thiserror::Error;

use crate::backend::Backend;

/// A nonnegative retry count stored on a feed event.
#[derive(Clone, Copy, Debug, PartialEq, Eq, macros::NumNewtype)]
#[num_newtype(
    inner = i32,
    min = 0,
    error = "feed event attempts must be a non-negative integer"
)]
struct FeedEventAttempts(i32);

impl FeedEventAttempts {
    const fn into_i32(self) -> i32 {
        self.0
    }
}

/// Free-form feed processing diagnostic retained exactly for the public record.
#[derive(Debug, macros::SqlxBridge)]
struct StoredFeedDiagnostic(String);

impl StoredFeedDiagnostic {
    fn into_inner(self) -> String {
        self.0
    }
}

/// A feed URL retained exactly until claim policy decides whether it is actionable.
#[derive(Debug, macros::SqlxBridge)]
struct StoredFeedUrl(String);

impl StoredFeedUrl {
    fn into_feed_path(self) -> Result<FeedPath, <FeedPath as FromStr>::Err> {
        self.0.parse()
    }
}
#[derive(Debug, sqlx::FromRow)]
pub(crate) struct ClaimedFeedEventRow {
    id: FeedEventId,
    feed_url: StoredFeedUrl,
    status: FeedEventStatus,
    attempts: FeedEventAttempts,
    last_error: Option<StoredFeedDiagnostic>,
    next_attempt_at: UtcInstant,
    claimed_at: Option<UtcInstant>,
    terminal_at: Option<UtcInstant>,
    created_at: UtcInstant,
    regenerated_at: Option<UtcInstant>,
    pinged_at: Option<UtcInstant>,
}

/// A feed event after the claim query's fully typed intermediate has passed
/// feed-URL-only policy conversion.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FeedEventRecord {
    pub id: FeedEventId,
    pub feed_path: FeedPath,
    pub status: FeedEventStatus,
    pub attempts: i32,
    pub last_error: Option<String>,
    pub next_attempt_at: UtcInstant,
    pub claimed_at: Option<UtcInstant>,
    pub terminal_at: Option<UtcInstant>,
    pub created_at: UtcInstant,
    pub regenerated_at: Option<UtcInstant>,
    pub pinged_at: Option<UtcInstant>,
}

#[derive(Debug, Error)]
pub enum FeedEventError {
    #[error("database error: {0}")]
    Db(#[from] sqlx::Error),
}

/// One row of a claim batch: either a converted record, or the id of a row whose
/// feed URL will not parse and which must therefore be purged.
///
/// Claim decoding itself is strict: [`ClaimedFeedEventRow`] is derived and every
/// leaf has a declaration-backed type. This conversion owns the sole policy
/// exception: a lossless stored feed URL that cannot become a [`FeedPath`] is
/// unactionable and is routed to the purge list; every other decode failure has
/// already propagated before conversion begins.
pub(crate) enum ClaimedRow {
    Record(Box<FeedEventRecord>),
    Corrupt(FeedEventId),
}

impl From<ClaimedFeedEventRow> for ClaimedRow {
    fn from(row: ClaimedFeedEventRow) -> Self {
        let ClaimedFeedEventRow {
            id,
            feed_url,
            status,
            attempts,
            last_error,
            next_attempt_at,
            claimed_at,
            terminal_at,
            created_at,
            regenerated_at,
            pinged_at,
        } = row;
        let Ok(feed_path) = feed_url.into_feed_path() else {
            return Self::Corrupt(id);
        };
        Self::Record(Box::new(FeedEventRecord {
            id,
            feed_path,
            status,
            attempts: attempts.into_i32(),
            last_error: last_error.map(StoredFeedDiagnostic::into_inner),
            next_attempt_at,
            claimed_at,
            terminal_at,
            created_at,
            regenerated_at,
            pinged_at,
        }))
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
    /// Insert a new `pending` row for `feed_path` through the caller's write
    /// transaction. Returns the new row id.
    async fn enqueue(
        &self,
        transaction: &mut crate::WriteTransaction,
        feed_path: &FeedPath,
    ) -> Result<FeedEventId, FeedEventError>;

    /// Insert `pending` rows for every path in `feed_paths` through the caller's
    /// write transaction. Production fan-outs MUST use this, not per-row
    /// `enqueue`: per-row autocommit loops are the `SQLite` lock-churn failure
    /// mode diagnosed in #766. Duplicates are inserted as-is; the drain dedupes
    /// by grouping on `feed_path`.
    async fn enqueue_many(
        &self,
        transaction: &mut crate::WriteTransaction,
        feed_paths: &[FeedPath],
    ) -> Result<(), FeedEventError>;

    /// Atomically claim up to `limit` rows that are either:
    ///   * `status = 'pending' AND next_attempt_at <= now`, or
    ///   * `status = 'claimed' AND claimed_at < now - lease_timeout`
    ///     (stuck-claim recovery).
    /// Transitions claimed rows to `status = 'claimed'` and stamps
    /// `claimed_at = now`.
    async fn claim_pending_batch(
        &self,
        transaction: &mut crate::WriteTransaction,
        limit: usize,
        lease_timeout: Duration,
    ) -> Result<Vec<FeedEventRecord>, FeedEventError>;

    /// Count rows currently claimable by the feed worker without claiming them.
    async fn claimable_count(&self, lease_timeout: Duration) -> Result<u64, FeedEventError>;

    /// Stamp `regenerated_at = now` on the given rows. Status is unchanged
    /// (still `claimed` until ping resolves).
    async fn mark_regenerated(
        &self,
        transaction: &mut crate::WriteTransaction,
        ids: &[FeedEventId],
    ) -> Result<(), FeedEventError>;

    /// Transition rows to `status = 'done'`, stamp `pinged_at`, and persist the
    /// supplied terminal instant for the retention cutoff.
    async fn mark_pinged(
        &self,
        transaction: &mut crate::WriteTransaction,
        ids: &[FeedEventId],
        now: UtcInstant,
    ) -> Result<(), FeedEventError>;

    /// Re-queue rows for another attempt: status back to `pending`,
    /// increment attempts, record the error, schedule the next attempt,
    /// and clear `claimed_at`.
    async fn mark_failed(
        &self,
        transaction: &mut crate::WriteTransaction,
        ids: &[FeedEventId],
        error: &str,
        next_attempt_at: UtcInstant,
    ) -> Result<(), FeedEventError>;

    /// Terminal failure: set `status = 'failed'`, record the final error, and
    /// persist the supplied terminal instant for the retention cutoff.
    async fn mark_exhausted(
        &self,
        transaction: &mut crate::WriteTransaction,
        ids: &[FeedEventId],
        error: &str,
        now: UtcInstant,
    ) -> Result<(), FeedEventError>;
    /// Delete terminal rows eligible at the supplied instant in fixed-size
    /// statements, releasing the connection after each statement.
    async fn prune_terminal_events(&self, now: UtcInstant) -> Result<u64, FeedEventError>;
}

/// Backend-specific divergence for [`FeedEventStore`].
///
/// [`claim_pending_batch`][FeedEventDialect::claim_pending_batch] diverges in SQL
/// shape: both backends execute one atomic `UPDATE … RETURNING` claim, then keep
/// its short transaction open only until decoding and feed-URL partitioning
/// complete. A non-URL decode failure therefore rolls the claim back.
/// Postgres uses a `FOR UPDATE SKIP LOCKED` CTE for inter-worker
/// skip-locking, while `SQLite` (which lacks `SKIP LOCKED`) drives the same write
/// from an `id IN (SELECT … LIMIT …)` subquery. `SQLite` must avoid the earlier
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
        connection: &mut Self::Connection,
        now: UtcInstant,
        lease_cutoff: UtcInstant,
        limit: FeedEventClaimLimit,
    ) -> Result<Vec<FeedEventRecord>, FeedEventError>;

    /// Count rows eligible under the same predicate as `claim_pending_batch`.
    async fn claimable_count(
        pool: &Pool<Self>,
        now: UtcInstant,
        lease_cutoff: UtcInstant,
    ) -> Result<u64, FeedEventError>;

    /// Stamp `regenerated_at = now` on all rows whose id is in `ids`.
    async fn mark_regenerated(
        connection: &mut Self::Connection,
        ids: &[FeedEventId],
    ) -> Result<(), FeedEventError>;

    /// Transition rows to `done`, stamp `pinged_at`, and persist `terminal_at`.
    async fn mark_pinged(
        connection: &mut Self::Connection,
        ids: &[FeedEventId],
        now: UtcInstant,
    ) -> Result<(), FeedEventError>;

    /// Re-queue rows for another attempt.
    async fn mark_failed(
        connection: &mut Self::Connection,
        ids: &[FeedEventId],
        error: &str,
        next_attempt_at: UtcInstant,
    ) -> Result<(), FeedEventError>;

    /// Terminal failure: set `status = 'failed'`, record the final error, and
    /// persist `terminal_at`.
    async fn mark_exhausted(
        connection: &mut Self::Connection,
        ids: &[FeedEventId],
        error: &str,
        now: UtcInstant,
    ) -> Result<(), FeedEventError>;
    /// Delete one bounded batch of terminal rows eligible at `now`.
    async fn prune_terminal_events(
        pool: &Pool<Self>,
        now: UtcInstant,
        limit: RowLimit,
    ) -> Result<u64, FeedEventError>;
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

/// The maximum rows one terminal-retention statement may delete.
const TERMINAL_PRUNE_BATCH: u64 = 200;
const TERMINAL_PRUNE_LIMIT: RowLimit = RowLimit::at_most(200);
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
    for<'c> &'c mut DB::Connection: sqlx::Executor<'c, Database = DB>,
    for<'q> DB::Arguments<'q>: sqlx::IntoArguments<'q, DB>,
    (FeedEventId,): for<'r> sqlx::FromRow<'r, DB::Row>,
{
    #[tracing::instrument(
        name = "storage.feed_events.enqueue",
        skip(self, transaction, feed_path),
        fields(db.system = DB::DB_SYSTEM)
    )]
    async fn enqueue(
        &self,
        transaction: &mut crate::WriteTransaction,
        feed_path: &FeedPath,
    ) -> Result<FeedEventId, FeedEventError> {
        let connection = DB::write_connection(transaction)?;
        let sql = format!("{INSERT_FEED_EVENT} RETURNING id");
        let id = sqlx::query_scalar::<_, FeedEventId>(&sql)
            .bind(feed_path)
            .fetch_one(&mut *connection)
            .await?;
        Ok(id)
    }

    #[tracing::instrument(
        name = "storage.feed_events.enqueue_many",
        skip(self, transaction, feed_paths),
        fields(db.system = DB::DB_SYSTEM, count = feed_paths.len())
    )]
    async fn enqueue_many(
        &self,
        transaction: &mut crate::WriteTransaction,
        feed_paths: &[FeedPath],
    ) -> Result<(), FeedEventError> {
        let connection = DB::write_connection(transaction)?;
        for feed_path in feed_paths {
            sqlx::query(INSERT_FEED_EVENT)
                .bind(feed_path)
                .execute(&mut *connection)
                .await?;
        }
        Ok(())
    }

    #[tracing::instrument(
        name = "storage.feed_events.claim_pending_batch",
        skip(self, transaction),
        fields(db.system = DB::DB_SYSTEM)
    )]
    async fn claim_pending_batch(
        &self,
        transaction: &mut crate::WriteTransaction,
        limit: usize,
        lease_timeout: Duration,
    ) -> Result<Vec<FeedEventRecord>, FeedEventError> {
        let now = UtcInstant::now();
        let lease_cutoff = UtcInstant::from(now.value() - lease_timeout);
        let limit = FeedEventClaimLimit::from_usize(limit);
        let connection = DB::write_connection(transaction)?;
        DB::claim_pending_batch(connection, now, lease_cutoff, limit).await
    }

    #[tracing::instrument(
        name = "storage.feed_events.claimable_count",
        skip(self),
        fields(db.system = DB::DB_SYSTEM)
    )]
    async fn claimable_count(&self, lease_timeout: Duration) -> Result<u64, FeedEventError> {
        let now = UtcInstant::now();
        let lease_cutoff = UtcInstant::from(now.value() - lease_timeout);
        DB::claimable_count(&self.pool, now, lease_cutoff).await
    }

    #[tracing::instrument(
        name = "storage.feed_events.mark_regenerated",
        skip(self, transaction, ids),
        fields(db.system = DB::DB_SYSTEM)
    )]
    async fn mark_regenerated(
        &self,
        transaction: &mut crate::WriteTransaction,
        ids: &[FeedEventId],
    ) -> Result<(), FeedEventError> {
        if ids.is_empty() {
            return Ok(());
        }
        let connection = DB::write_connection(transaction)?;
        DB::mark_regenerated(connection, ids).await
    }

    #[tracing::instrument(
        name = "storage.feed_events.mark_pinged",
        skip(self, transaction, ids),
        fields(db.system = DB::DB_SYSTEM)
    )]
    async fn mark_pinged(
        &self,
        transaction: &mut crate::WriteTransaction,
        ids: &[FeedEventId],
        now: UtcInstant,
    ) -> Result<(), FeedEventError> {
        if ids.is_empty() {
            return Ok(());
        }
        let connection = DB::write_connection(transaction)?;
        DB::mark_pinged(connection, ids, now).await
    }

    #[tracing::instrument(
        name = "storage.feed_events.mark_failed",
        skip(self, transaction, ids, error),
        fields(db.system = DB::DB_SYSTEM)
    )]
    async fn mark_failed(
        &self,
        transaction: &mut crate::WriteTransaction,
        ids: &[FeedEventId],
        error: &str,
        next_attempt_at: UtcInstant,
    ) -> Result<(), FeedEventError> {
        if ids.is_empty() {
            return Ok(());
        }
        let connection = DB::write_connection(transaction)?;
        DB::mark_failed(connection, ids, error, next_attempt_at).await
    }

    #[tracing::instrument(
        name = "storage.feed_events.mark_exhausted",
        skip(self, transaction, ids, error),
        fields(db.system = DB::DB_SYSTEM)
    )]
    async fn mark_exhausted(
        &self,
        transaction: &mut crate::WriteTransaction,
        ids: &[FeedEventId],
        error: &str,
        now: UtcInstant,
    ) -> Result<(), FeedEventError> {
        if ids.is_empty() {
            return Ok(());
        }
        let connection = DB::write_connection(transaction)?;
        DB::mark_exhausted(connection, ids, error, now).await
    }
    #[tracing::instrument(
        name = "storage.feed_events.prune_terminal_events",
        skip(self),
        fields(db.system = DB::DB_SYSTEM)
    )]
    async fn prune_terminal_events(&self, now: UtcInstant) -> Result<u64, FeedEventError> {
        let mut deleted = 0;
        loop {
            let batch = DB::prune_terminal_events(&self.pool, now, TERMINAL_PRUNE_LIMIT).await?;
            deleted += batch;
            if batch < TERMINAL_PRUNE_BATCH {
                return Ok(deleted);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use chrono::{TimeZone, Timelike, Utc};

    use super::*;
    use crate::test_support::{Backend, backends, fp};
    use rstest::*;
    use rstest_reuse::*;

    async fn confirmed<T>(
        scope: &crate::WriteScope,
        callback: impl for<'scope> FnOnce(
            &'scope mut crate::WriteTransaction,
        ) -> futures_util::future::BoxFuture<
            'scope,
            Result<T, FeedEventError>,
        >,
    ) -> T {
        crate::test_support::confirmed_for(
            scope.run(callback).await.expect("feed-event write"),
            "feed-event test write acknowledgement",
        )
    }
    async fn enqueue(
        scope: &crate::WriteScope,
        feed_events: std::sync::Arc<dyn FeedEventStorage>,
        feed_path: FeedPath,
    ) -> FeedEventId {
        confirmed(scope, move |transaction| {
            Box::pin(async move { feed_events.enqueue(transaction, &feed_path).await })
        })
        .await
    }

    async fn claim(
        scope: &crate::WriteScope,
        feed_events: std::sync::Arc<dyn FeedEventStorage>,
        limit: usize,
        lease: chrono::Duration,
    ) -> Vec<FeedEventRecord> {
        confirmed(scope, move |transaction| {
            Box::pin(async move {
                feed_events
                    .claim_pending_batch(transaction, limit, lease)
                    .await
            })
        })
        .await
    }

    async fn mark_regenerated(
        scope: &crate::WriteScope,
        feed_events: std::sync::Arc<dyn FeedEventStorage>,
        ids: Vec<FeedEventId>,
    ) {
        confirmed(scope, move |transaction| {
            Box::pin(async move { feed_events.mark_regenerated(transaction, &ids).await })
        })
        .await;
    }

    async fn mark_pinged(
        scope: &crate::WriteScope,
        feed_events: std::sync::Arc<dyn FeedEventStorage>,
        ids: Vec<FeedEventId>,
        now: UtcInstant,
    ) {
        confirmed(scope, move |transaction| {
            Box::pin(async move { feed_events.mark_pinged(transaction, &ids, now).await })
        })
        .await;
    }

    async fn mark_failed(
        scope: &crate::WriteScope,
        feed_events: std::sync::Arc<dyn FeedEventStorage>,
        ids: Vec<FeedEventId>,
        error: String,
        next_attempt_at: UtcInstant,
    ) {
        confirmed(scope, move |transaction| {
            Box::pin(async move {
                feed_events
                    .mark_failed(transaction, &ids, &error, next_attempt_at)
                    .await
            })
        })
        .await;
    }

    async fn mark_exhausted(
        scope: &crate::WriteScope,
        feed_events: std::sync::Arc<dyn FeedEventStorage>,
        ids: Vec<FeedEventId>,
        error: String,
        now: UtcInstant,
    ) {
        confirmed(scope, move |transaction| {
            Box::pin(async move {
                feed_events
                    .mark_exhausted(transaction, &ids, &error, now)
                    .await
            })
        })
        .await;
    }

    // The token ↔ variant mapping is the `text_enum` attribute's, tested at the type
    // in `common/src/feed/event_status.rs`.

    #[apply(backends)]
    #[tokio::test]
    async fn enqueue_creates_pending_row(#[case] backend: Backend) {
        let env = backend.setup().await;
        let id = enqueue(
            &env.state.write_scope,
            std::sync::Arc::clone(&env.state.feed_events),
            fp("/feed.rss"),
        )
        .await;
        assert!(i64::from(id) > 0);
    }

    #[apply(backends)]
    #[tokio::test]
    async fn enqueue_many_creates_pending_rows_in_one_batch(#[case] backend: Backend) {
        let env = backend.setup().await;
        let paths = vec![
            fp("/feed.rss"),
            fp("/~alice/feed.rss"),
            fp("/tags/t/feed.rss"),
        ];
        let expected: std::collections::HashSet<_> = paths.iter().cloned().collect();
        let feed_events = std::sync::Arc::clone(&env.state.feed_events);
        env.state
            .write_scope
            .run(move |transaction| {
                Box::pin(async move { feed_events.enqueue_many(transaction, &paths).await })
            })
            .await
            .unwrap();

        let claimed = claim(
            &env.state.write_scope,
            std::sync::Arc::clone(&env.state.feed_events),
            10,
            chrono::Duration::minutes(5),
        )
        .await;
        // FeedPath has no Ord (deliberate), so compare as sets.
        let urls: std::collections::HashSet<_> =
            claimed.iter().map(|r| r.feed_path.clone()).collect();
        assert_eq!(urls, expected);
    }

    #[apply(backends)]
    #[tokio::test]
    async fn enqueue_many_inserts_duplicates_as_is(#[case] backend: Backend) {
        let env = backend.setup().await;
        // No dedupe: the drain groups by feed_path, so duplicate rows are
        // harmless — pin that enqueue_many does not silently collapse them.
        let paths = vec![fp("/feed.rss"), fp("/feed.rss")];
        let feed_events = std::sync::Arc::clone(&env.state.feed_events);
        env.state
            .write_scope
            .run(move |transaction| {
                Box::pin(async move { feed_events.enqueue_many(transaction, &paths).await })
            })
            .await
            .unwrap();

        let claimed = claim(
            &env.state.write_scope,
            std::sync::Arc::clone(&env.state.feed_events),
            10,
            chrono::Duration::minutes(5),
        )
        .await;
        assert_eq!(claimed.len(), 2);
    }

    #[apply(backends)]
    #[tokio::test]
    async fn enqueue_many_empty_input_is_a_no_op(#[case] backend: Backend) {
        let env = backend.setup().await;
        let feed_events = std::sync::Arc::clone(&env.state.feed_events);
        let paths = Vec::new();
        env.state
            .write_scope
            .run(move |transaction| {
                Box::pin(async move { feed_events.enqueue_many(transaction, &paths).await })
            })
            .await
            .unwrap();

        let claimed = claim(
            &env.state.write_scope,
            std::sync::Arc::clone(&env.state.feed_events),
            10,
            chrono::Duration::minutes(5),
        )
        .await;
        assert!(claimed.is_empty());
    }

    #[apply(backends)]
    #[tokio::test]
    async fn claimable_count_counts_pending_ready_rows(#[case] backend: Backend) {
        let env = backend.setup().await;
        enqueue(
            &env.state.write_scope,
            std::sync::Arc::clone(&env.state.feed_events),
            fp("/feed.rss"),
        )
        .await;

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
        let id = enqueue(
            &env.state.write_scope,
            std::sync::Arc::clone(&env.state.feed_events),
            fp("/feed.rss"),
        )
        .await;
        mark_failed(
            &env.state.write_scope,
            std::sync::Arc::clone(&env.state.feed_events),
            vec![id],
            "retry later".to_owned(),
            UtcInstant::from(chrono::Utc::now() + chrono::Duration::hours(1)),
        )
        .await;

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
        enqueue(
            &env.state.write_scope,
            std::sync::Arc::clone(&env.state.feed_events),
            fp("/feed.rss"),
        )
        .await;
        let claimed = claim(
            &env.state.write_scope,
            std::sync::Arc::clone(&env.state.feed_events),
            10,
            chrono::Duration::minutes(5),
        )
        .await;
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
        enqueue(
            &env.state.write_scope,
            std::sync::Arc::clone(&env.state.feed_events),
            fp("/feed.rss"),
        )
        .await;
        claim(
            &env.state.write_scope,
            std::sync::Arc::clone(&env.state.feed_events),
            10,
            chrono::Duration::minutes(5),
        )
        .await;

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
        enqueue(
            &env.state.write_scope,
            std::sync::Arc::clone(&env.state.feed_events),
            fp("/feed.rss"),
        )
        .await;

        // The claim skips-and-purges the corrupt row and returns only the valid
        // one — the batch is NOT failed (which would wedge the worker forever).
        // The batch-level report is redacted rather than retaining the bad value.
        let feed_events = std::sync::Arc::clone(&env.state.feed_events);
        let lease = chrono::Duration::minutes(5);
        let (claimed, trace) = crate::helpers::swallowed_test::capture_async(
            env.state.write_scope.run(move |transaction| {
                Box::pin(async move {
                    feed_events
                        .claim_pending_batch(transaction, 50, lease)
                        .await
                })
            }),
        )
        .await;
        let claimed = crate::test_support::confirmed_for(claimed.unwrap(), "claim acknowledgement");
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
        enqueue(
            &env.state.write_scope,
            std::sync::Arc::clone(&env.state.feed_events),
            fp("/feed.rss"),
        )
        .await;
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

        let feed_events = std::sync::Arc::clone(&env.state.feed_events);
        let lease = chrono::Duration::minutes(5);
        let err = match env
            .state
            .write_scope
            .run(move |transaction| {
                Box::pin(async move {
                    feed_events
                        .claim_pending_batch(transaction, 50, lease)
                        .await
                })
            })
            .await
            .expect_err("a non-feed_url decode failure must propagate")
        {
            crate::WriteScopeError::Operation(error) => error,
            crate::WriteScopeError::Begin(error) => {
                unreachable!("open-pool write scope must begin: {error}")
            }
        };
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

        // A failed decode must roll the UPDATE … RETURNING claim back, not only
        // avoid the corrupt-URL purge path.
        let untouched = env
            .base
            .pool()
            .scalar_i64(
                "SELECT COUNT(*) FROM feed_events \
                 WHERE status = 'pending' AND claimed_at IS NULL",
            )
            .await
            .unwrap();
        assert_eq!(
            untouched, 1,
            "a non-feed_url decode failure must retain pending status and null claimed_at"
        );
    }

    #[apply(backends)]
    #[tokio::test]
    async fn negative_attempts_propagate_and_leave_the_claimed_row_untouched(
        #[case] backend: Backend,
    ) {
        let env = backend.setup().await;
        enqueue(
            &env.state.write_scope,
            std::sync::Arc::clone(&env.state.feed_events),
            fp("/feed.rss"),
        )
        .await;
        env.base
            .pool()
            .execute("UPDATE feed_events SET attempts = -1")
            .await
            .unwrap();

        let feed_events = std::sync::Arc::clone(&env.state.feed_events);
        let lease = chrono::Duration::minutes(5);
        let err = match env
            .state
            .write_scope
            .run(move |transaction| {
                Box::pin(async move {
                    feed_events
                        .claim_pending_batch(transaction, 50, lease)
                        .await
                })
            })
            .await
            .expect_err("negative attempts must fail decode rather than divert")
        {
            crate::WriteScopeError::Operation(error) => error,
            crate::WriteScopeError::Begin(error) => {
                unreachable!("open-pool write scope must begin: {error}")
            }
        };
        assert!(
            matches!(err, FeedEventError::Db(sqlx::Error::ColumnDecode { .. })),
            "expected a column-decode error, got {err:?}"
        );

        let remaining = env
            .base
            .pool()
            .scalar_i64("SELECT COUNT(*) FROM feed_events")
            .await
            .unwrap();
        assert_eq!(remaining, 1, "negative attempts must not delete the row");

        // A failed decode must roll the UPDATE … RETURNING claim back, not merely
        // avoid deleting the row.
        let untouched = env
            .base
            .pool()
            .scalar_i64(
                "SELECT COUNT(*) FROM feed_events \
                 WHERE status = 'pending' AND claimed_at IS NULL",
            )
            .await
            .unwrap();
        assert_eq!(
            untouched, 1,
            "negative attempts must retain pending status and null claimed_at"
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
        enqueue(
            &env.state.write_scope,
            std::sync::Arc::clone(&env.state.feed_events),
            fp("/feed.rss"),
        )
        .await;
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

        let feed_events = std::sync::Arc::clone(&env.state.feed_events);
        let lease = chrono::Duration::minutes(5);
        let (claimed, trace) = crate::helpers::swallowed_test::capture_async(
            env.state.write_scope.run(move |transaction| {
                Box::pin(async move {
                    feed_events
                        .claim_pending_batch(transaction, 50, lease)
                        .await
                })
            }),
        )
        .await;
        let claimed = crate::test_support::confirmed_for(claimed.unwrap(), "claim acknowledgement");
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
        enqueue(
            &env.state.write_scope,
            std::sync::Arc::clone(&env.state.feed_events),
            fp("/feed.rss"),
        )
        .await;
        let claimed = claim(
            &env.state.write_scope,
            std::sync::Arc::clone(&env.state.feed_events),
            10,
            chrono::Duration::minutes(5),
        )
        .await;
        assert_eq!(claimed.len(), 1);
        assert_eq!(claimed[0].status, FeedEventStatus::Claimed);
        assert!(claimed[0].claimed_at.is_some());
    }

    #[apply(backends)]
    #[tokio::test]
    async fn double_claim_returns_no_rows_within_lease(#[case] backend: Backend) {
        let env = backend.setup().await;
        enqueue(
            &env.state.write_scope,
            std::sync::Arc::clone(&env.state.feed_events),
            fp("/feed.rss"),
        )
        .await;
        let first = claim(
            &env.state.write_scope,
            std::sync::Arc::clone(&env.state.feed_events),
            10,
            chrono::Duration::minutes(5),
        )
        .await;
        let second = claim(
            &env.state.write_scope,
            std::sync::Arc::clone(&env.state.feed_events),
            10,
            chrono::Duration::minutes(5),
        )
        .await;
        assert_eq!(first.len(), 1);
        assert_eq!(second.len(), 0);
    }

    #[apply(backends)]
    #[tokio::test]
    async fn lease_expired_rows_are_reclaimable(#[case] backend: Backend) {
        let env = backend.setup().await;
        enqueue(
            &env.state.write_scope,
            std::sync::Arc::clone(&env.state.feed_events),
            fp("/feed.rss"),
        )
        .await;
        let _first = claim(
            &env.state.write_scope,
            std::sync::Arc::clone(&env.state.feed_events),
            10,
            chrono::Duration::minutes(5),
        )
        .await;
        // With a zero lease, the just-claimed row is immediately re-eligible.
        let second = claim(
            &env.state.write_scope,
            std::sync::Arc::clone(&env.state.feed_events),
            10,
            chrono::Duration::zero(),
        )
        .await;
        assert_eq!(second.len(), 1);
    }

    #[apply(backends)]
    #[tokio::test]
    async fn mark_pinged_marks_done_and_removes_from_queue(#[case] backend: Backend) {
        let env = backend.setup().await;
        enqueue(
            &env.state.write_scope,
            std::sync::Arc::clone(&env.state.feed_events),
            fp("/feed.rss"),
        )
        .await;
        let claimed = claim(
            &env.state.write_scope,
            std::sync::Arc::clone(&env.state.feed_events),
            10,
            chrono::Duration::minutes(5),
        )
        .await;
        let ids: Vec<FeedEventId> = claimed.iter().map(|r| r.id).collect();
        let id = ids[0];
        mark_regenerated(
            &env.state.write_scope,
            std::sync::Arc::clone(&env.state.feed_events),
            ids.clone(),
        )
        .await;
        mark_pinged(
            &env.state.write_scope,
            std::sync::Arc::clone(&env.state.feed_events),
            ids,
            fixture_instant(500_000),
        )
        .await;
        let terminal_at = crate::with_closeable_pool!(env.base.pool(), pool, {
            sqlx::query_scalar::<_, Option<UtcInstant>>(
                "SELECT terminal_at FROM feed_events WHERE id = $1",
            )
            .bind(id)
            .fetch_one(pool)
            .await
            .unwrap()
        });
        assert_eq!(terminal_at, Some(fixture_instant(500_000)));
        let next = claim(
            &env.state.write_scope,
            std::sync::Arc::clone(&env.state.feed_events),
            10,
            chrono::Duration::minutes(5),
        )
        .await;
        assert!(next.is_empty());
    }

    #[apply(backends)]
    #[tokio::test]
    async fn mark_failed_increments_attempts_and_reschedules(#[case] backend: Backend) {
        let env = backend.setup().await;
        let id = enqueue(
            &env.state.write_scope,
            std::sync::Arc::clone(&env.state.feed_events),
            fp("/feed.rss"),
        )
        .await;
        claim(
            &env.state.write_scope,
            std::sync::Arc::clone(&env.state.feed_events),
            10,
            chrono::Duration::minutes(5),
        )
        .await;
        let future = UtcInstant::from(Utc::now() + chrono::Duration::minutes(1));
        mark_failed(
            &env.state.write_scope,
            std::sync::Arc::clone(&env.state.feed_events),
            vec![id],
            "boom".to_owned(),
            future,
        )
        .await;
        // Not eligible until `future`.
        let now = claim(
            &env.state.write_scope,
            std::sync::Arc::clone(&env.state.feed_events),
            10,
            chrono::Duration::minutes(5),
        )
        .await;
        assert!(now.is_empty());
    }

    #[apply(backends)]
    #[tokio::test]
    async fn mark_exhausted_marks_failed_terminal(#[case] backend: Backend) {
        let env = backend.setup().await;
        let id = enqueue(
            &env.state.write_scope,
            std::sync::Arc::clone(&env.state.feed_events),
            fp("/feed.rss"),
        )
        .await;
        mark_exhausted(
            &env.state.write_scope,
            std::sync::Arc::clone(&env.state.feed_events),
            vec![id],
            "gave up".to_owned(),
            fixture_instant(600_000),
        )
        .await;
        let terminal_at = crate::with_closeable_pool!(env.base.pool(), pool, {
            sqlx::query_scalar::<_, Option<UtcInstant>>(
                "SELECT terminal_at FROM feed_events WHERE id = $1",
            )
            .bind(id)
            .fetch_one(pool)
            .await
            .unwrap()
        });
        assert_eq!(terminal_at, Some(fixture_instant(600_000)));
        // Failed rows are never eligible.
        let next = claim(
            &env.state.write_scope,
            std::sync::Arc::clone(&env.state.feed_events),
            10,
            chrono::Duration::minutes(5),
        )
        .await;
        assert!(next.is_empty());
    }

    fn fixture_instant(microsecond: u32) -> UtcInstant {
        UtcInstant::from(
            Utc.with_ymd_and_hms(2026, 8, 26, 12, 34, 56)
                .unwrap()
                .with_nanosecond(microsecond * 1_000)
                .unwrap(),
        )
    }
    async fn claim_at<DB: FeedEventDialect>(
        pool: &Pool<DB>,
        now: UtcInstant,
        lease_cutoff: UtcInstant,
    ) -> Vec<FeedEventRecord> {
        let mut connection = pool.acquire().await.unwrap();
        DB::claim_pending_batch(
            &mut *connection,
            now,
            lease_cutoff,
            FeedEventClaimLimit::from_usize(10),
        )
        .await
        .unwrap()
    }

    #[apply(backends)]
    #[tokio::test]
    async fn claim_honors_exact_pending_and_reclaim_boundaries(#[case] backend: Backend) {
        let env = backend.setup().await;
        let now = fixture_instant(500_000);
        let cutoff = fixture_instant(400_000);
        let fixtures = [
            (
                fp("/~pending-boundary/feed.rss"),
                FeedEventStatus::Pending,
                now,
                None,
            ),
            (
                fp("/~reclaim-boundary/feed.rss"),
                FeedEventStatus::Claimed,
                fixture_instant(300_000),
                Some(cutoff),
            ),
            (
                fp("/~reclaim-expired/feed.rss"),
                FeedEventStatus::Claimed,
                fixture_instant(200_000),
                Some(fixture_instant(399_999)),
            ),
        ];

        let claimed = crate::with_closeable_pool!(env.base.pool(), pool, {
            for fixture in &fixtures {
                sqlx::query(
                    "INSERT INTO feed_events (feed_url, status, next_attempt_at, claimed_at, created_at) \
                     VALUES ($1, $2, $3, $4, $5)",
                )
                .bind(&fixture.0)
                .bind(fixture.1)
                .bind(fixture.2)
                .bind(fixture.3)
                .bind(fixture_instant(100_000))
                .execute(pool)
                .await
                .unwrap();
            }
            claim_at(pool, now, cutoff).await
        });

        let paths: std::collections::HashSet<_> =
            claimed.into_iter().map(|record| record.feed_path).collect();
        assert_eq!(
            paths,
            [
                fp("/~pending-boundary/feed.rss"),
                fp("/~reclaim-expired/feed.rss")
            ]
            .into_iter()
            .collect()
        );
    }

    #[apply(backends)]
    #[tokio::test]
    async fn feed_event_rows_decode_every_lifecycle_timestamp_shape(#[case] backend: Backend) {
        let env = backend.setup().await;
        let fixtures = [
            (
                fp("/~pending/feed.rss"),
                FeedEventStatus::Pending,
                0,
                None,
                fixture_instant(100_001),
                None,
                None,
                fixture_instant(100_002),
                None,
                None,
            ),
            (
                fp("/~claimed/feed.rss"),
                FeedEventStatus::Claimed,
                1,
                Some("claim in progress"),
                fixture_instant(200_001),
                Some(fixture_instant(200_002)),
                None,
                fixture_instant(200_003),
                None,
                None,
            ),
            (
                fp("/~done/feed.rss"),
                FeedEventStatus::Done,
                2,
                None,
                fixture_instant(300_001),
                Some(fixture_instant(300_002)),
                Some(fixture_instant(300_005)),
                fixture_instant(300_003),
                Some(fixture_instant(300_004)),
                Some(fixture_instant(300_005)),
            ),
            (
                fp("/~failed/feed.rss"),
                FeedEventStatus::Failed,
                7,
                Some("ping exhausted"),
                fixture_instant(400_001),
                Some(fixture_instant(400_002)),
                Some(fixture_instant(400_005)),
                fixture_instant(400_003),
                Some(fixture_instant(400_004)),
                None,
            ),
        ];

        let claimed_rows = crate::with_closeable_pool!(env.base.pool(), pool, {
            for fixture in &fixtures {
                sqlx::query(
                    "INSERT INTO feed_events \
                     (feed_url, status, attempts, last_error, next_attempt_at, claimed_at, terminal_at, created_at, regenerated_at, pinged_at) \
                     VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10)",
                )
                .bind(&fixture.0)
                .bind(fixture.1)
                .bind(fixture.2)
                .bind(fixture.3)
                .bind(fixture.4)
                .bind(fixture.5)
                .bind(fixture.6)
                .bind(fixture.7)
                .bind(fixture.8)
                .bind(fixture.9)
                .execute(pool)
                .await
                .unwrap();
            }

            sqlx::query_as::<_, ClaimedFeedEventRow>(
                "SELECT id, feed_url, status, attempts, last_error, next_attempt_at, claimed_at, terminal_at, \
                 created_at, regenerated_at, pinged_at FROM feed_events ORDER BY id",
            )
            .fetch_all(pool)
            .await
            .unwrap()
            .into_iter()
            .map(ClaimedRow::from)
            .collect::<Vec<ClaimedRow>>()
        });

        let (rows, corrupt_ids) = partition_claimed(claimed_rows);
        assert!(corrupt_ids.is_empty());
        assert_eq!(rows.len(), fixtures.len());
        for (row, fixture) in rows.iter().zip(fixtures.iter()) {
            assert!(i64::from(row.id) > 0);
            assert_eq!(row.feed_path, fixture.0);
            assert_eq!(row.status, fixture.1);
            assert_eq!(row.attempts, fixture.2);
            assert_eq!(row.last_error.as_deref(), fixture.3);
            assert_eq!(row.next_attempt_at, fixture.4);
            assert_eq!(row.claimed_at, fixture.5);
            assert_eq!(row.terminal_at, fixture.6);
            assert_eq!(row.created_at, fixture.7);
            assert_eq!(row.regenerated_at, fixture.8);
            assert_eq!(row.pinged_at, fixture.9);
        }
    }

    #[apply(backends)]
    #[tokio::test]
    async fn prune_terminal_events_obeys_exact_cutoff_and_preserves_nonterminal_rows(
        #[case] backend: Backend,
    ) {
        let env = backend.setup().await;
        let now = fixture_instant(900_000);
        let cutoff = UtcInstant::from(now.value() - chrono::Duration::days(7));
        crate::with_closeable_pool!(env.base.pool(), pool, {
            for (path, status, terminal_at) in [
                (fp("/~completed/feed.rss"), FeedEventStatus::Done, Some(now)),
                (
                    fp("/~failed-boundary/feed.rss"),
                    FeedEventStatus::Failed,
                    Some(cutoff),
                ),
                (
                    fp("/~failed-newer/feed.rss"),
                    FeedEventStatus::Failed,
                    Some(UtcInstant::from(
                        cutoff.value() + chrono::Duration::seconds(1),
                    )),
                ),
                (fp("/~pending/feed.rss"), FeedEventStatus::Pending, None),
                (fp("/~claimed/feed.rss"), FeedEventStatus::Claimed, None),
            ] {
                sqlx::query(
                    "INSERT INTO feed_events (feed_url, status, next_attempt_at, terminal_at) \
                     VALUES ($1, $2, $3, $4)",
                )
                .bind(path)
                .bind(status)
                .bind(now)
                .bind(terminal_at)
                .execute(pool)
                .await
                .unwrap();
            }
        });

        assert_eq!(
            env.state
                .feed_events
                .prune_terminal_events(now)
                .await
                .expect("prune terminal rows"),
            2
        );
        assert_eq!(
            env.base
                .pool()
                .scalar_i64("SELECT COUNT(*) FROM feed_events")
                .await
                .expect("count retained rows"),
            3
        );
    }

    #[apply(backends)]
    #[tokio::test]
    async fn prune_terminal_events_drains_more_than_one_fixed_batch(#[case] backend: Backend) {
        let env = backend.setup().await;
        let now = fixture_instant(900_000);
        crate::with_closeable_pool!(env.base.pool(), pool, {
            for index in 0..=TERMINAL_PRUNE_BATCH {
                sqlx::query(
                    "INSERT INTO feed_events (feed_url, status, next_attempt_at, terminal_at) \
                     VALUES ($1, 'done', $2, $2)",
                )
                .bind(format!("/~completed-{index}/feed.rss"))
                .bind(now)
                .execute(pool)
                .await
                .unwrap();
            }
        });

        assert_eq!(
            env.state
                .feed_events
                .prune_terminal_events(now)
                .await
                .expect("drain terminal rows"),
            TERMINAL_PRUNE_BATCH + 1
        );
    }

    #[apply(backends)]
    #[tokio::test]
    async fn prune_terminal_events_preserves_closed_pool_error(#[case] backend: Backend) {
        let env = backend.setup().await;
        env.base.pool().close().await;
        assert!(matches!(
            env.state
                .feed_events
                .prune_terminal_events(UtcInstant::now())
                .await,
            Err(FeedEventError::Db(sqlx::Error::PoolClosed))
        ));
    }

    #[apply(backends)]
    #[tokio::test]
    async fn empty_id_arrays_are_noops(#[case] backend: Backend) {
        let env = backend.setup().await;
        mark_regenerated(
            &env.state.write_scope,
            std::sync::Arc::clone(&env.state.feed_events),
            Vec::new(),
        )
        .await;
        mark_pinged(
            &env.state.write_scope,
            std::sync::Arc::clone(&env.state.feed_events),
            Vec::new(),
            UtcInstant::now(),
        )
        .await;
        mark_failed(
            &env.state.write_scope,
            std::sync::Arc::clone(&env.state.feed_events),
            Vec::new(),
            "x".to_owned(),
            UtcInstant::now(),
        )
        .await;
        mark_exhausted(
            &env.state.write_scope,
            std::sync::Arc::clone(&env.state.feed_events),
            Vec::new(),
            "x".to_owned(),
            UtcInstant::now(),
        )
        .await;
    }
}
