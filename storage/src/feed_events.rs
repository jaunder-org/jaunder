//! Queue of feed-regeneration events driven by post mutations and drained by
//! the feed worker. Rows transition pending → claimed → done|failed; stuck
//! claims are re-eligible after `lease_timeout` elapses (claim-lease pattern).

use std::str::FromStr;
#[cfg(test)]
use std::sync::Arc;

use async_trait::async_trait;
use chrono::Duration;
use common::ids::FeedEventId;
use common::pagination::{PageSize, RowLimit};
use common::time::UtcInstant;
use host::{
    error::{self, ErrorClass, ErrorKind, SwallowedSource},
    feed::{FeedEventClaimLimit, FeedEventPhase, FeedEventStatus, FeedPath},
    metrics,
    retention::Domain,
};
use sqlx::{Database, Pool};
use thiserror::Error;
#[cfg(test)]
use tokio::sync::{Notify, RwLock};

#[cfg(test)]
use crate::WriteScope;
use crate::{WriteTransaction, backend::Backend, sql::QueryStorageExt};

/// A nonnegative retry count stored on a feed event.
#[derive(Clone, Copy, Debug, PartialEq, Eq, macros::NumNewtype)]
#[num_newtype(
    inner = i32,
    min = 0,
    error = "feed event attempts must be a non-negative integer"
)]
pub(crate) struct FeedEventAttempts(i32);

impl FeedEventAttempts {
    const fn into_i32(self) -> i32 {
        self.0
    }
}

/// Free-form feed processing diagnostic retained for operators.
#[derive(Debug, macros::SqlxBridge)]
pub(crate) struct StoredFeedDiagnostic(String);

impl StoredFeedDiagnostic {
    fn bounded(value: &str) -> Self {
        const LIMIT: usize = 1_024;
        if value.chars().count() <= LIMIT {
            return Self(value.to_owned());
        }
        let prefix: String = value.chars().take(LIMIT - 1).collect();
        Self(format!("{prefix}…"))
    }

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
    phase: FeedEventPhase,
    regeneration_attempts: FeedEventAttempts,
    publication_attempts: FeedEventAttempts,
    regeneration_diagnostic: Option<StoredFeedDiagnostic>,
    publication_diagnostic: Option<StoredFeedDiagnostic>,
    next_attempt_at: UtcInstant,
    claimed_at: Option<UtcInstant>,
    terminal_at: Option<UtcInstant>,
    created_at: UtcInstant,
    regenerated_at: Option<UtcInstant>,
    pinged_at: Option<UtcInstant>,
}

/// A feed event after the claim query's fully typed intermediate has passed
/// feed-URL-only policy conversion.
#[derive(Debug, sqlx::FromRow)]
pub(crate) struct DeadLetterRow {
    id: FeedEventId,
    feed_url: StoredFeedUrl,
    phase: FeedEventPhase,
    attempts: FeedEventAttempts,
    terminal_at: UtcInstant,
    diagnostic: Option<StoredFeedDiagnostic>,
}

impl DeadLetterRow {
    fn into_record(self) -> Result<FeedEventDeadLetter, FeedEventDeadLetterError> {
        let feed_path = self.feed_url.into_feed_path().map_err(|_| {
            error::report_swallowed(
                ErrorKind::Storage,
                ErrorClass::Bug,
                "storage.feed_events.decode_dead_letter_feed_path",
                SwallowedSource::Redacted,
            );
            FeedEventDeadLetterError::CorruptRow
        })?;
        Ok(FeedEventDeadLetter {
            id: self.id,
            feed_path,
            phase: self.phase,
            attempts: self.attempts.into_i32(),
            terminal_at: self.terminal_at,
            diagnostic: self.diagnostic.map(StoredFeedDiagnostic::into_inner),
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FeedEventRecord {
    pub id: FeedEventId,
    pub feed_path: FeedPath,
    pub status: FeedEventStatus,
    pub phase: FeedEventPhase,
    pub regeneration_attempts: i32,
    pub publication_attempts: i32,
    pub regeneration_diagnostic: Option<String>,
    pub publication_diagnostic: Option<String>,
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

/// Failure while reading an operator dead-letter page.
#[derive(Debug, Error)]
pub enum FeedEventDeadLetterError {
    #[error("database error: {0}")]
    Db(#[from] sqlx::Error),
    #[error("a terminal feed event has an invalid stored feed path")]
    CorruptRow,
}

/// Failure while atomically redriving an exact operator selection.
#[derive(Debug, Error)]
pub enum FeedEventRedriveError {
    #[error("database error: {0}")]
    Db(#[from] sqlx::Error),
    #[error(transparent)]
    Rejected(#[from] FeedEventRedriveRejected),
}
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
            phase,
            regeneration_attempts,
            publication_attempts,
            regeneration_diagnostic,
            publication_diagnostic,
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
        let regeneration_attempts = regeneration_attempts.into_i32();
        let publication_attempts = publication_attempts.into_i32();
        let regeneration_diagnostic = regeneration_diagnostic.map(StoredFeedDiagnostic::into_inner);
        let publication_diagnostic = publication_diagnostic.map(StoredFeedDiagnostic::into_inner);
        Self::Record(Box::new(FeedEventRecord {
            id,
            feed_path,
            status,
            phase,
            regeneration_attempts,
            publication_attempts,
            regeneration_diagnostic,
            publication_diagnostic,
            next_attempt_at,
            claimed_at,
            terminal_at,
            created_at,
            regenerated_at,
            pinged_at,
        }))
    }
}

/// Stable keyset position for newest-first dead-letter inspection.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct FeedEventDeadLetterCursor {
    pub terminal_at: UtcInstant,
    pub id: FeedEventId,
}

/// Operator-safe projection of one terminal feed event.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FeedEventDeadLetter {
    pub id: FeedEventId,
    pub feed_path: FeedPath,
    pub phase: FeedEventPhase,
    pub attempts: i32,
    pub terminal_at: UtcInstant,
    pub diagnostic: Option<String>,
}

/// A bounded dead-letter page and the cursor for its successor.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FeedEventDeadLetterPage {
    pub events: Vec<FeedEventDeadLetter>,
    pub next_cursor: Option<FeedEventDeadLetterCursor>,
}

/// Exact-ID redrive rejected before changing any selected row.
#[derive(Clone, Debug, PartialEq, Eq, Error)]
#[error("one or more feed events are absent, expired, or not dead-lettered")]
pub struct FeedEventRedriveRejected;

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
        error::report_swallowed(
            ErrorKind::Storage,
            ErrorClass::Bug,
            "storage.feed_events.decode_feed_path",
            SwallowedSource::Redacted,
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
        ErrorKind::Storage,
        ErrorClass::Transient,
        context,
    )
}

#[cfg(test)]
#[derive(Default)]
pub struct PruneBatchGate {
    arrived: Notify,
    resume: Notify,
}

#[cfg(test)]
impl PruneBatchGate {
    async fn wait_for_batch(&self) {
        self.arrived.notified().await;
    }

    fn resume(&self) {
        self.resume.notify_one();
    }
}

#[cfg_attr(feature = "test-utils", mockall::automock)]
#[async_trait]
pub trait FeedEventStorage: Send + Sync {
    /// Insert a new `pending` row for `feed_path` through the caller's write
    /// transaction. Returns the new row id.
    async fn enqueue(
        &self,
        transaction: &mut WriteTransaction,
        feed_path: &FeedPath,
    ) -> Result<FeedEventId, FeedEventError>;

    /// Insert `pending` rows for every path in `feed_paths` through the caller's
    /// write transaction. Production fan-outs MUST use this, not per-row
    /// `enqueue`: per-row autocommit loops are the `SQLite` lock-churn failure
    /// mode diagnosed in #766. Duplicates are inserted as-is; the drain dedupes
    /// by grouping on `feed_path`.
    async fn enqueue_many(
        &self,
        transaction: &mut WriteTransaction,
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
        transaction: &mut WriteTransaction,
        limit: usize,
        lease_timeout: Duration,
    ) -> Result<Vec<FeedEventRecord>, FeedEventError>;

    /// Count rows currently claimable by the feed worker without claiming them.
    async fn claimable_count(&self, lease_timeout: Duration) -> Result<u64, FeedEventError>;

    /// List terminal rows for one failed phase in stable newest-first order.
    async fn dead_letters(
        &self,
        phase: FeedEventPhase,
        cursor: Option<FeedEventDeadLetterCursor>,
        page_size: PageSize,
    ) -> Result<FeedEventDeadLetterPage, FeedEventDeadLetterError>;

    /// Atomically requeue exact dead-letter ids. Every supplied id must still
    /// exist, be terminal, and be inside failed-event retention.
    async fn redrive_dead_letters(
        &self,
        transaction: &mut WriteTransaction,
        ids: &[FeedEventId],
        now: UtcInstant,
    ) -> Result<(), FeedEventRedriveError>;
    /// Stamp regeneration then advance the row to the publication phase.
    async fn mark_regenerated(
        &self,
        transaction: &mut WriteTransaction,
        ids: &[FeedEventId],
    ) -> Result<(), FeedEventError>;

    /// Transition rows to `status = 'done'`, stamp `pinged_at`, and persist the
    /// supplied terminal instant for the retention cutoff.
    async fn mark_pinged(
        &self,
        transaction: &mut WriteTransaction,
        ids: &[FeedEventId],
        now: UtcInstant,
    ) -> Result<(), FeedEventError>;

    /// Re-queue rows for a regeneration attempt. This always re-enters the
    /// regeneration phase, increments its budget, records its diagnostic, and
    /// clears the claim.
    async fn retry_regeneration(
        &self,
        transaction: &mut WriteTransaction,
        ids: &[FeedEventId],
        error: &str,
        next_attempt_at: UtcInstant,
    ) -> Result<(), FeedEventError>;

    /// Terminal regeneration failure. This always terminalizes in the
    /// regeneration phase and increments only its budget.
    async fn dead_letter_regeneration(
        &self,
        transaction: &mut WriteTransaction,
        ids: &[FeedEventId],
        error: &str,
        now: UtcInstant,
    ) -> Result<(), FeedEventError>;

    /// Re-queue rows for a publication attempt, incrementing only the
    /// publication budget and retaining its diagnostic.
    async fn retry_publication(
        &self,
        transaction: &mut WriteTransaction,
        ids: &[FeedEventId],
        error: &str,
        next_attempt_at: UtcInstant,
    ) -> Result<(), FeedEventError>;

    /// Terminal publication failure, incrementing only the publication budget.
    async fn dead_letter_publication(
        &self,
        transaction: &mut WriteTransaction,
        ids: &[FeedEventId],
        error: &str,
        now: UtcInstant,
    ) -> Result<(), FeedEventError>;
    /// Requeue stale work in the regeneration phase without charging the
    /// failed attempt. A stale snapshot starts a fresh regeneration budget.
    async fn restart_regeneration(
        &self,
        transaction: &mut WriteTransaction,
        ids: &[FeedEventId],
        now: UtcInstant,
    ) -> Result<(), FeedEventError>;

    /// Re-enter regeneration after the publication cache disappeared. This is a
    /// new regeneration cycle, so its budget and diagnostic are reset.
    async fn reset_regeneration(
        &self,
        transaction: &mut WriteTransaction,
        ids: &[FeedEventId],
        now: UtcInstant,
    ) -> Result<(), FeedEventError>;

    /// Delete terminal rows eligible at the supplied instant in fixed-size
    /// statements, releasing the connection after each statement.
    #[cfg(test)]
    async fn install_prune_batch_gate(&self, gate: Option<Arc<PruneBatchGate>>);
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
/// The bulk-id methods (`mark_regenerated`, `mark_pinged`, and the explicit
/// regeneration/publication retry and dead-letter transitions) diverge:
/// `SQLite` does not support array binding so they use a dynamically-built
/// `IN (?, ?, …)` pattern; Postgres uses `WHERE id = ANY($n)` with a slice
/// binding — a cleaner and cheaper approach.
#[async_trait]
pub(crate) trait FeedEventDialect: Backend {
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

    async fn dead_letters(
        pool: &Pool<Self>,
        phase: FeedEventPhase,
        cursor: Option<FeedEventDeadLetterCursor>,
        limit: RowLimit,
    ) -> Result<Vec<DeadLetterRow>, FeedEventDeadLetterError>;

    async fn redrive_dead_letters(
        connection: &mut Self::Connection,
        ids: &[FeedEventId],
        now: UtcInstant,
        failed_cutoff: UtcInstant,
    ) -> Result<bool, FeedEventRedriveError>;

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

    /// Re-queue in the regeneration phase, charging only regeneration.
    async fn retry_regeneration(
        connection: &mut Self::Connection,
        ids: &[FeedEventId],
        error: &StoredFeedDiagnostic,
        next_attempt_at: UtcInstant,
    ) -> Result<(), FeedEventError>;

    /// Terminalize in the regeneration phase, charging only regeneration.
    async fn dead_letter_regeneration(
        connection: &mut Self::Connection,
        ids: &[FeedEventId],
        error: &StoredFeedDiagnostic,
        now: UtcInstant,
    ) -> Result<(), FeedEventError>;

    /// Re-queue in the publication phase, charging only publication.
    async fn retry_publication(
        connection: &mut Self::Connection,
        ids: &[FeedEventId],
        error: &StoredFeedDiagnostic,
        next_attempt_at: UtcInstant,
    ) -> Result<(), FeedEventError>;

    /// Terminalize in the publication phase, charging only publication.
    async fn dead_letter_publication(
        connection: &mut Self::Connection,
        ids: &[FeedEventId],
        error: &StoredFeedDiagnostic,
        now: UtcInstant,
    ) -> Result<(), FeedEventError>;
    /// Requeue a stale generation with a fresh regeneration budget.
    async fn restart_regeneration(
        connection: &mut Self::Connection,
        ids: &[FeedEventId],
        now: UtcInstant,
    ) -> Result<(), FeedEventError>;

    /// Re-enter regeneration with a fresh regeneration budget.
    async fn reset_regeneration(
        connection: &mut Self::Connection,
        ids: &[FeedEventId],
        now: UtcInstant,
    ) -> Result<(), FeedEventError>;

    /// Delete one bounded batch of terminal rows eligible at the frozen `now`.
    ///
    /// Failed rows use the separately derived `failed_cutoff` retention boundary.
    async fn prune_terminal_events(
        pool: &Pool<Self>,
        now: UtcInstant,
        failed_cutoff: UtcInstant,
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
    #[cfg(test)]
    prune_batch_gate: RwLock<Option<Arc<PruneBatchGate>>>,
}

/// The one enqueue statement, shared by [`FeedEventStorage::enqueue`] (which
/// appends `RETURNING id`) and [`FeedEventStorage::enqueue_many`] — a column
/// change edits it once.
const INSERT_FEED_EVENT: &str = "INSERT INTO feed_events (feed_url) VALUES ($1)";

/// The maximum rows one terminal-retention statement may delete.
const TERMINAL_PRUNE_LIMIT: RowLimit = RowLimit::at_most(200);
const FAILED_EVENT_RETENTION: Duration = Duration::days(7);
impl<DB: Database> FeedEventStore<DB> {
    #[must_use]
    pub fn new(pool: Pool<DB>) -> Self {
        Self {
            pool,
            #[cfg(test)]
            prune_batch_gate: RwLock::new(None),
        }
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
        transaction: &mut WriteTransaction,
        feed_path: &FeedPath,
    ) -> Result<FeedEventId, FeedEventError> {
        let connection = DB::write_connection(transaction)?;
        let sql = format!("{INSERT_FEED_EVENT} RETURNING id");
        let id = sqlx::query_scalar::<_, FeedEventId>(&sql)
            .bind_storage(feed_path)
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
        transaction: &mut WriteTransaction,
        feed_paths: &[FeedPath],
    ) -> Result<(), FeedEventError> {
        let connection = DB::write_connection(transaction)?;
        for feed_path in feed_paths {
            sqlx::query(INSERT_FEED_EVENT)
                .bind_storage(feed_path)
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
        transaction: &mut WriteTransaction,
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

    async fn dead_letters(
        &self,
        phase: FeedEventPhase,
        cursor: Option<FeedEventDeadLetterCursor>,
        page_size: PageSize,
    ) -> Result<FeedEventDeadLetterPage, FeedEventDeadLetterError> {
        let mut rows = DB::dead_letters(&self.pool, phase, cursor, page_size.fetch_limit()).await?;
        let has_more = page_size.has_more(rows.len());
        rows.truncate(page_size.page_len());
        let events = rows
            .into_iter()
            .map(DeadLetterRow::into_record)
            .collect::<Result<Vec<_>, _>>()?;
        let next_cursor = has_more
            .then(|| {
                events.last().map(|event| FeedEventDeadLetterCursor {
                    terminal_at: event.terminal_at,
                    id: event.id,
                })
            })
            .flatten();
        Ok(FeedEventDeadLetterPage {
            events,
            next_cursor,
        })
    }

    async fn redrive_dead_letters(
        &self,
        transaction: &mut WriteTransaction,
        ids: &[FeedEventId],
        now: UtcInstant,
    ) -> Result<(), FeedEventRedriveError> {
        if ids.is_empty() {
            return Err(FeedEventRedriveRejected.into());
        }
        let connection = DB::write_connection(transaction)?;
        let failed_cutoff = UtcInstant::from(now.value() - FAILED_EVENT_RETENTION);
        if DB::redrive_dead_letters(connection, ids, now, failed_cutoff).await? {
            Ok(())
        } else {
            Err(FeedEventRedriveRejected.into())
        }
    }

    #[tracing::instrument(
        name = "storage.feed_events.mark_regenerated",
        skip(self, transaction, ids),
        fields(db.system = DB::DB_SYSTEM)
    )]
    async fn mark_regenerated(
        &self,
        transaction: &mut WriteTransaction,
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
        transaction: &mut WriteTransaction,
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
        name = "storage.feed_events.retry_regeneration",
        skip(self, transaction, ids, error),
        fields(db.system = DB::DB_SYSTEM)
    )]
    async fn retry_regeneration(
        &self,
        transaction: &mut WriteTransaction,
        ids: &[FeedEventId],
        error: &str,
        next_attempt_at: UtcInstant,
    ) -> Result<(), FeedEventError> {
        if ids.is_empty() {
            return Ok(());
        }
        let connection = DB::write_connection(transaction)?;
        let error = StoredFeedDiagnostic::bounded(error);
        DB::retry_regeneration(connection, ids, &error, next_attempt_at).await
    }

    #[tracing::instrument(
        name = "storage.feed_events.dead_letter_regeneration",
        skip(self, transaction, ids, error),
        fields(db.system = DB::DB_SYSTEM)
    )]
    async fn dead_letter_regeneration(
        &self,
        transaction: &mut WriteTransaction,
        ids: &[FeedEventId],
        error: &str,
        now: UtcInstant,
    ) -> Result<(), FeedEventError> {
        if ids.is_empty() {
            return Ok(());
        }
        let connection = DB::write_connection(transaction)?;
        let error = StoredFeedDiagnostic::bounded(error);
        DB::dead_letter_regeneration(connection, ids, &error, now).await
    }

    #[tracing::instrument(
        name = "storage.feed_events.retry_publication",
        skip(self, transaction, ids, error),
        fields(db.system = DB::DB_SYSTEM)
    )]
    async fn retry_publication(
        &self,
        transaction: &mut WriteTransaction,
        ids: &[FeedEventId],
        error: &str,
        next_attempt_at: UtcInstant,
    ) -> Result<(), FeedEventError> {
        if ids.is_empty() {
            return Ok(());
        }
        let connection = DB::write_connection(transaction)?;
        let error = StoredFeedDiagnostic::bounded(error);
        DB::retry_publication(connection, ids, &error, next_attempt_at).await
    }

    #[tracing::instrument(
        name = "storage.feed_events.dead_letter_publication",
        skip(self, transaction, ids, error),
        fields(db.system = DB::DB_SYSTEM)
    )]
    async fn dead_letter_publication(
        &self,
        transaction: &mut WriteTransaction,
        ids: &[FeedEventId],
        error: &str,
        now: UtcInstant,
    ) -> Result<(), FeedEventError> {
        if ids.is_empty() {
            return Ok(());
        }
        let connection = DB::write_connection(transaction)?;
        let error = StoredFeedDiagnostic::bounded(error);
        DB::dead_letter_publication(connection, ids, &error, now).await
    }

    #[tracing::instrument(
        name = "storage.feed_events.restart_regeneration",
        skip(self, transaction, ids),
        fields(db.system = DB::DB_SYSTEM)
    )]
    async fn restart_regeneration(
        &self,
        transaction: &mut WriteTransaction,
        ids: &[FeedEventId],
        now: UtcInstant,
    ) -> Result<(), FeedEventError> {
        if ids.is_empty() {
            return Ok(());
        }
        let connection = DB::write_connection(transaction)?;
        DB::restart_regeneration(connection, ids, now).await
    }

    #[tracing::instrument(
        name = "storage.feed_events.reset_regeneration",
        skip(self, transaction, ids),
        fields(db.system = DB::DB_SYSTEM)
    )]
    async fn reset_regeneration(
        &self,
        transaction: &mut WriteTransaction,
        ids: &[FeedEventId],
        now: UtcInstant,
    ) -> Result<(), FeedEventError> {
        if ids.is_empty() {
            return Ok(());
        }
        let connection = DB::write_connection(transaction)?;
        DB::reset_regeneration(connection, ids, now).await
    }
    #[tracing::instrument(
        name = "storage.feed_events.prune_terminal_events",
        skip(self),
        fields(db.system = DB::DB_SYSTEM)
    )]
    async fn prune_terminal_events(&self, now: UtcInstant) -> Result<u64, FeedEventError> {
        let failed_cutoff = UtcInstant::from(now.value() - FAILED_EVENT_RETENTION);
        let mut deleted = 0;
        loop {
            let batch =
                DB::prune_terminal_events(&self.pool, now, failed_cutoff, TERMINAL_PRUNE_LIMIT)
                    .await?;
            if batch > 0 {
                metrics::retention_pruned(Domain::FeedEvents, batch);
            }
            deleted += batch;
            if batch < TERMINAL_PRUNE_LIMIT.value().unsigned_abs() {
                return Ok(deleted);
            }
            #[cfg(test)]
            {
                let gate = self.prune_batch_gate.read().await.clone();
                if let Some(gate) = gate {
                    gate.arrived.notify_one();
                    gate.resume.notified().await;
                }
            }
        }
    }
    #[cfg(test)]
    async fn install_prune_batch_gate(&self, gate: Option<Arc<PruneBatchGate>>) {
        *self.prune_batch_gate.write().await = gate;
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;
    use std::time::Duration as StdDuration;

    use chrono::{Duration, TimeZone, Timelike, Utc};
    use sqlx::Error as SqlxError;
    use tokio::time;

    use super::*;
    use crate::test_support::{Backend, backends, confirmed_for, fp};
    use rstest::*;
    use rstest_reuse::*;

    #[test]
    fn diagnostics_are_truncated_at_a_unicode_scalar_boundary() {
        let value = format!("{}{}", "🦀".repeat(1_024), "tail");
        let bounded = StoredFeedDiagnostic::bounded(&value).into_inner();
        assert_eq!(bounded.chars().count(), 1_024);
        assert_eq!(bounded.chars().last(), Some('…'));
        assert!(bounded.starts_with(&"🦀".repeat(1_023)));
    }

    async fn confirmed<T>(
        scope: &WriteScope,
        callback: impl for<'scope> FnOnce(
            &'scope mut WriteTransaction,
        ) -> futures_util::future::BoxFuture<
            'scope,
            Result<T, FeedEventError>,
        >,
    ) -> T {
        confirmed_for(
            scope.run(callback).await.expect("feed-event write"),
            "feed-event test write acknowledgement",
        )
    }
    async fn enqueue(
        scope: &crate::WriteScope,
        feed_events: Arc<dyn FeedEventStorage>,
        feed_path: FeedPath,
    ) -> FeedEventId {
        confirmed(scope, move |transaction| {
            Box::pin(async move { feed_events.enqueue(transaction, &feed_path).await })
        })
        .await
    }

    async fn claim(
        scope: &crate::WriteScope,
        feed_events: Arc<dyn FeedEventStorage>,
        limit: usize,
        lease: Duration,
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
        feed_events: Arc<dyn FeedEventStorage>,
        ids: Vec<FeedEventId>,
    ) {
        confirmed(scope, move |transaction| {
            Box::pin(async move { feed_events.mark_regenerated(transaction, &ids).await })
        })
        .await;
    }

    async fn mark_pinged(
        scope: &crate::WriteScope,
        feed_events: Arc<dyn FeedEventStorage>,
        ids: Vec<FeedEventId>,
        now: UtcInstant,
    ) {
        confirmed(scope, move |transaction| {
            Box::pin(async move { feed_events.mark_pinged(transaction, &ids, now).await })
        })
        .await;
    }

    async fn retry_regeneration(
        scope: &crate::WriteScope,
        feed_events: Arc<dyn FeedEventStorage>,
        ids: Vec<FeedEventId>,
        error: String,
        next_attempt_at: UtcInstant,
    ) {
        confirmed(scope, move |transaction| {
            Box::pin(async move {
                feed_events
                    .retry_regeneration(transaction, &ids, &error, next_attempt_at)
                    .await
            })
        })
        .await;
    }

    async fn dead_letter_regeneration(
        scope: &crate::WriteScope,
        feed_events: Arc<dyn FeedEventStorage>,
        ids: Vec<FeedEventId>,
        error: String,
        now: UtcInstant,
    ) {
        confirmed(scope, move |transaction| {
            Box::pin(async move {
                feed_events
                    .dead_letter_regeneration(transaction, &ids, &error, now)
                    .await
            })
        })
        .await;
    }

    async fn dead_letter_publication(
        scope: &crate::WriteScope,
        feed_events: Arc<dyn FeedEventStorage>,
        ids: Vec<FeedEventId>,
        error: String,
        now: UtcInstant,
    ) {
        confirmed(scope, move |transaction| {
            Box::pin(async move {
                feed_events
                    .dead_letter_publication(transaction, &ids, &error, now)
                    .await
            })
        })
        .await;
    }

    async fn confirmed_redrive(
        scope: &crate::WriteScope,
        feed_events: Arc<dyn FeedEventStorage>,
        ids: Vec<FeedEventId>,
        now: UtcInstant,
    ) {
        let outcome = scope
            .run(move |transaction| {
                Box::pin(async move {
                    feed_events
                        .redrive_dead_letters(transaction, &ids, now)
                        .await
                })
            })
            .await
            .expect("dead-letter redrive write");
        confirmed_for(outcome, "dead-letter redrive acknowledgement");
    }
    #[apply(backends)]
    #[tokio::test]
    async fn dead_letters_page_stably_and_redrive_exact_ids_atomically(#[case] backend: Backend) {
        let env = backend.setup().await;
        let feeds = Arc::clone(&env.state.feed_events);
        let ids = vec![
            enqueue(&env.state.write_scope, Arc::clone(&feeds), fp("/feed.rss")).await,
            enqueue(
                &env.state.write_scope,
                Arc::clone(&feeds),
                fp("/~one/feed.rss"),
            )
            .await,
            enqueue(
                &env.state.write_scope,
                Arc::clone(&feeds),
                fp("/tags/one/feed.rss"),
            )
            .await,
        ];
        claim(
            &env.state.write_scope,
            Arc::clone(&feeds),
            10,
            Duration::minutes(5),
        )
        .await;
        let terminal = fixture_instant(1_000);
        for id in &ids {
            dead_letter_regeneration(
                &env.state.write_scope,
                Arc::clone(&feeds),
                vec![*id],
                "regeneration failure".to_owned(),
                terminal,
            )
            .await;
        }
        let publication_id = enqueue(
            &env.state.write_scope,
            Arc::clone(&feeds),
            fp("/~publication/feed.rss"),
        )
        .await;
        claim(
            &env.state.write_scope,
            Arc::clone(&feeds),
            10,
            Duration::minutes(5),
        )
        .await;
        mark_regenerated(
            &env.state.write_scope,
            Arc::clone(&feeds),
            vec![publication_id],
        )
        .await;
        dead_letter_publication(
            &env.state.write_scope,
            Arc::clone(&feeds),
            vec![publication_id],
            "publication failure".to_owned(),
            terminal,
        )
        .await;
        let publication = feeds
            .dead_letters(FeedEventPhase::Publication, None, PageSize::default())
            .await
            .unwrap();
        assert_eq!(publication.events.len(), 1);
        assert_eq!(publication.events[0].id, publication_id);
        assert_eq!(
            publication.events[0].attempts, 1,
            "terminal publication counts its final attempt"
        );

        let size = PageSize::try_from(2).unwrap();
        let first = feeds
            .dead_letters(FeedEventPhase::Regeneration, None, size)
            .await
            .unwrap();
        assert_eq!(first.events.len(), 2);
        assert!(
            first.events.iter().all(|event| event.attempts == 1),
            "terminal regeneration counts each final attempt"
        );
        let cursor = first.next_cursor.expect("overfetch supplies a cursor");
        let second = feeds
            .dead_letters(FeedEventPhase::Regeneration, Some(cursor), size)
            .await
            .unwrap();
        assert_eq!(second.events.len(), 1);
        assert!(second.next_cursor.is_none());
        let paged: std::collections::HashSet<_> = first
            .events
            .iter()
            .chain(&second.events)
            .map(|event| event.id)
            .collect();
        assert_eq!(
            paged.len(),
            ids.len(),
            "no terminal event is skipped or duplicated"
        );

        let duplicate = ids[0];
        let rejection = env
            .state
            .write_scope
            .run(move |transaction| {
                let feeds = Arc::clone(&feeds);
                Box::pin(async move {
                    feeds
                        .redrive_dead_letters(transaction, &[duplicate, duplicate], terminal)
                        .await
                })
            })
            .await
            .expect_err("duplicate exact ids reject the complete redrive");
        assert!(matches!(
            rejection,
            crate::WriteScopeError::Operation(FeedEventRedriveError::Rejected(_))
        ));
        let after_rejection = env
            .state
            .feed_events
            .dead_letters(FeedEventPhase::Regeneration, None, PageSize::default())
            .await
            .unwrap();
        assert_eq!(after_rejection.events.len(), ids.len());

        confirmed_redrive(
            &env.state.write_scope,
            Arc::clone(&env.state.feed_events),
            vec![ids[0], ids[1]],
            terminal,
        )
        .await;
        let remaining = env
            .state
            .feed_events
            .dead_letters(FeedEventPhase::Regeneration, None, PageSize::default())
            .await
            .unwrap();
        assert_eq!(remaining.events.len(), 1);
        assert_eq!(remaining.events[0].id, ids[2]);

        crate::with_closeable_pool!(env.base.pool(), pool, {
            sqlx::query(
                "UPDATE feed_events SET regeneration_attempts = 7, publication_attempts = 4 \
                 WHERE id = $1",
            )
            .bind_storage(publication_id)
            .execute(pool)
            .await
            .unwrap();
        });
        confirmed_redrive(
            &env.state.write_scope,
            Arc::clone(&env.state.feed_events),
            vec![publication_id],
            terminal,
        )
        .await;
        let redriven = claim(
            &env.state.write_scope,
            Arc::clone(&env.state.feed_events),
            10,
            Duration::minutes(5),
        )
        .await;
        let publication = redriven
            .into_iter()
            .find(|event| event.id == publication_id)
            .expect("publication redrive is claimable");
        assert_eq!(publication.phase, FeedEventPhase::Publication);
        assert_eq!(publication.regeneration_attempts, 7);
        assert_eq!(publication.publication_attempts, 0);
    }

    #[apply(backends)]
    #[tokio::test]
    async fn redrive_rejects_expired_dead_letter_before_pruning(#[case] backend: Backend) {
        let env = backend.setup().await;
        let feeds = Arc::clone(&env.state.feed_events);
        let id = enqueue(&env.state.write_scope, Arc::clone(&feeds), fp("/feed.rss")).await;
        claim(
            &env.state.write_scope,
            Arc::clone(&feeds),
            1,
            Duration::minutes(5),
        )
        .await;
        let terminal = fixture_instant(1_000);
        dead_letter_regeneration(
            &env.state.write_scope,
            Arc::clone(&feeds),
            vec![id],
            "expired regeneration failure".to_owned(),
            terminal,
        )
        .await;
        let now = UtcInstant::from(terminal.value() + Duration::days(8));

        let rejection = env
            .state
            .write_scope
            .run(move |transaction| {
                Box::pin(async move { feeds.redrive_dead_letters(transaction, &[id], now).await })
            })
            .await
            .expect_err("expired dead letter must reject before pruning");
        assert!(matches!(
            rejection,
            crate::WriteScopeError::Operation(FeedEventRedriveError::Rejected(_))
        ));
        let remaining = env
            .state
            .feed_events
            .dead_letters(FeedEventPhase::Regeneration, None, PageSize::default())
            .await
            .unwrap();
        assert_eq!(remaining.events.len(), 1);
        assert_eq!(remaining.events[0].id, id);
    }

    #[apply(backends)]
    #[tokio::test]
    async fn corrupt_dead_letter_path_fails_the_page_without_advancing_a_cursor(
        #[case] backend: Backend,
    ) {
        let env = backend.setup().await;
        crate::with_closeable_pool!(env.base.pool(), pool, {
            sqlx::query(
                "INSERT INTO feed_events \
                 (feed_url, status, phase, regeneration_attempts, publication_attempts, \
                  next_attempt_at, terminal_at, created_at) \
                 VALUES ('not-a-feed-path', 'failed', 'regeneration', 1, 0, \
                         CURRENT_TIMESTAMP, CURRENT_TIMESTAMP, CURRENT_TIMESTAMP)",
            )
            .execute(pool)
            .await
            .unwrap();
        });
        let error = env
            .state
            .feed_events
            .dead_letters(FeedEventPhase::Regeneration, None, PageSize::default())
            .await
            .expect_err("a corrupt terminal path must not produce an unstable short page");
        assert!(matches!(error, FeedEventDeadLetterError::CorruptRow));
    }

    // The token ↔ variant mapping is the `text_enum` attribute's, tested at the type
    // in `common/src/feed/event_status.rs`.
    #[apply(backends)]
    #[tokio::test]
    async fn enqueue_creates_pending_row(#[case] backend: Backend) {
        let env = backend.setup().await;
        let id = enqueue(
            &env.state.write_scope,
            Arc::clone(&env.state.feed_events),
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
        let feed_events = Arc::clone(&env.state.feed_events);
        env.state
            .write_scope
            .run(move |transaction| {
                Box::pin(async move { feed_events.enqueue_many(transaction, &paths).await })
            })
            .await
            .unwrap();

        let claimed = claim(
            &env.state.write_scope,
            Arc::clone(&env.state.feed_events),
            10,
            Duration::minutes(5),
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
        let feed_events = Arc::clone(&env.state.feed_events);
        env.state
            .write_scope
            .run(move |transaction| {
                Box::pin(async move { feed_events.enqueue_many(transaction, &paths).await })
            })
            .await
            .unwrap();

        let claimed = claim(
            &env.state.write_scope,
            Arc::clone(&env.state.feed_events),
            10,
            Duration::minutes(5),
        )
        .await;
        assert_eq!(claimed.len(), 2);
    }

    #[apply(backends)]
    #[tokio::test]
    async fn enqueue_many_empty_input_is_a_no_op(#[case] backend: Backend) {
        let env = backend.setup().await;
        let feed_events = Arc::clone(&env.state.feed_events);
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
            Arc::clone(&env.state.feed_events),
            10,
            Duration::minutes(5),
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
            Arc::clone(&env.state.feed_events),
            fp("/feed.rss"),
        )
        .await;

        let count = env
            .state
            .feed_events
            .claimable_count(Duration::minutes(5))
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
            Arc::clone(&env.state.feed_events),
            fp("/feed.rss"),
        )
        .await;
        retry_regeneration(
            &env.state.write_scope,
            Arc::clone(&env.state.feed_events),
            vec![id],
            "retry later".to_owned(),
            UtcInstant::from(chrono::Utc::now() + Duration::hours(1)),
        )
        .await;

        let count = env
            .state
            .feed_events
            .claimable_count(Duration::minutes(5))
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
            Arc::clone(&env.state.feed_events),
            fp("/feed.rss"),
        )
        .await;
        let claimed = claim(
            &env.state.write_scope,
            Arc::clone(&env.state.feed_events),
            10,
            Duration::minutes(5),
        )
        .await;
        assert_eq!(claimed.len(), 1);

        let count = env
            .state
            .feed_events
            .claimable_count(Duration::minutes(5))
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
            Arc::clone(&env.state.feed_events),
            fp("/feed.rss"),
        )
        .await;
        claim(
            &env.state.write_scope,
            Arc::clone(&env.state.feed_events),
            10,
            Duration::minutes(5),
        )
        .await;

        let count = env
            .state
            .feed_events
            .claimable_count(Duration::zero())
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
            Arc::clone(&env.state.feed_events),
            fp("/feed.rss"),
        )
        .await;

        // The claim skips-and-purges the corrupt row and returns only the valid
        // one — the batch is NOT failed (which would wedge the worker forever).
        // The batch-level report is redacted rather than retaining the bad value.
        let feed_events = Arc::clone(&env.state.feed_events);
        let lease = Duration::minutes(5);
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
            Arc::clone(&env.state.feed_events),
            fp("/feed.rss"),
        )
        .await;
        // Widen the active phase counter past `i32` and store an out-of-range
        // value. A claimed row must not be silently discarded for this decode
        // failure.
        if matches!(backend, Backend::Postgres) {
            env.base
                .pool()
                .execute("ALTER TABLE feed_events ALTER COLUMN regeneration_attempts TYPE bigint")
                .await
                .unwrap();
        }
        env.base
            .pool()
            .execute("UPDATE feed_events SET regeneration_attempts = 3000000000")
            .await
            .unwrap();

        let feed_events = Arc::clone(&env.state.feed_events);
        let lease = Duration::minutes(5);
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
            matches!(err, FeedEventError::Db(SqlxError::ColumnDecode { .. })),
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
            Arc::clone(&env.state.feed_events),
            fp("/feed.rss"),
        )
        .await;
        env.base
            .pool()
            .execute("UPDATE feed_events SET regeneration_attempts = -1")
            .await
            .unwrap();

        let feed_events = Arc::clone(&env.state.feed_events);
        let lease = Duration::minutes(5);
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
            matches!(err, FeedEventError::Db(SqlxError::ColumnDecode { .. })),
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
            Arc::clone(&env.state.feed_events),
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

        let feed_events = Arc::clone(&env.state.feed_events);
        let lease = Duration::minutes(5);
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
            Arc::clone(&env.state.feed_events),
            fp("/feed.rss"),
        )
        .await;
        let claimed = claim(
            &env.state.write_scope,
            Arc::clone(&env.state.feed_events),
            10,
            Duration::minutes(5),
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
            Arc::clone(&env.state.feed_events),
            fp("/feed.rss"),
        )
        .await;
        let first = claim(
            &env.state.write_scope,
            Arc::clone(&env.state.feed_events),
            10,
            Duration::minutes(5),
        )
        .await;
        let second = claim(
            &env.state.write_scope,
            Arc::clone(&env.state.feed_events),
            10,
            Duration::minutes(5),
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
            Arc::clone(&env.state.feed_events),
            fp("/feed.rss"),
        )
        .await;
        let _first = claim(
            &env.state.write_scope,
            Arc::clone(&env.state.feed_events),
            10,
            Duration::minutes(5),
        )
        .await;
        // With a zero lease, the just-claimed row is immediately re-eligible.
        let second = claim(
            &env.state.write_scope,
            Arc::clone(&env.state.feed_events),
            10,
            Duration::zero(),
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
            Arc::clone(&env.state.feed_events),
            fp("/feed.rss"),
        )
        .await;
        let claimed = claim(
            &env.state.write_scope,
            Arc::clone(&env.state.feed_events),
            10,
            Duration::minutes(5),
        )
        .await;
        let ids: Vec<FeedEventId> = claimed.iter().map(|r| r.id).collect();
        let id = ids[0];
        mark_regenerated(
            &env.state.write_scope,
            Arc::clone(&env.state.feed_events),
            ids.clone(),
        )
        .await;
        mark_pinged(
            &env.state.write_scope,
            Arc::clone(&env.state.feed_events),
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
            Arc::clone(&env.state.feed_events),
            10,
            Duration::minutes(5),
        )
        .await;
        assert!(next.is_empty());
    }

    #[apply(backends)]
    #[tokio::test]
    async fn retry_regeneration_increments_attempts_and_reschedules(#[case] backend: Backend) {
        let env = backend.setup().await;
        let id = enqueue(
            &env.state.write_scope,
            Arc::clone(&env.state.feed_events),
            fp("/feed.rss"),
        )
        .await;
        claim(
            &env.state.write_scope,
            Arc::clone(&env.state.feed_events),
            10,
            Duration::minutes(5),
        )
        .await;
        let future = UtcInstant::from(Utc::now() + Duration::minutes(1));
        retry_regeneration(
            &env.state.write_scope,
            Arc::clone(&env.state.feed_events),
            vec![id],
            "boom".to_owned(),
            future,
        )
        .await;
        // Not eligible until `future`.
        let now = claim(
            &env.state.write_scope,
            Arc::clone(&env.state.feed_events),
            10,
            Duration::minutes(5),
        )
        .await;
        assert!(now.is_empty());
    }

    #[apply(backends)]
    #[tokio::test]
    async fn dead_letter_regeneration_marks_failed_terminal(#[case] backend: Backend) {
        let env = backend.setup().await;
        let id = enqueue(
            &env.state.write_scope,
            Arc::clone(&env.state.feed_events),
            fp("/feed.rss"),
        )
        .await;
        dead_letter_regeneration(
            &env.state.write_scope,
            Arc::clone(&env.state.feed_events),
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
            Arc::clone(&env.state.feed_events),
            10,
            Duration::minutes(5),
        )
        .await;
        assert!(next.is_empty());
    }

    #[apply(backends)]
    #[tokio::test]
    async fn default_dead_letter_page_overfetches_fifty_one_rows(#[case] backend: Backend) {
        let env = backend.setup().await;
        let feeds = Arc::clone(&env.state.feed_events);
        let ids = futures_util::future::join_all((0..51).map(|index| {
            enqueue(
                &env.state.write_scope,
                Arc::clone(&feeds),
                fp(&format!("/~pagination-{index}/feed.rss")),
            )
        }))
        .await;
        claim(
            &env.state.write_scope,
            Arc::clone(&feeds),
            100,
            Duration::minutes(5),
        )
        .await;
        let terminal = fixture_instant(700_000);
        for id in &ids {
            dead_letter_regeneration(
                &env.state.write_scope,
                Arc::clone(&feeds),
                vec![*id],
                "terminal".to_owned(),
                terminal,
            )
            .await;
        }

        let first = feeds
            .dead_letters(FeedEventPhase::Regeneration, None, PageSize::default())
            .await
            .expect("first page");
        assert_eq!(first.events.len(), 50);
        let cursor = first.next_cursor.expect("51st row must produce a cursor");
        let second = feeds
            .dead_letters(
                FeedEventPhase::Regeneration,
                Some(cursor),
                PageSize::default(),
            )
            .await
            .expect("second page");
        assert_eq!(second.events.len(), 1);
        assert!(second.next_cursor.is_none());
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
                .bind_storage(&fixture.0)
                .bind_storage(fixture.1)
                .bind_storage(fixture.2)
                .bind_storage(fixture.3)
                .bind_storage(fixture_instant(100_000))
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
                let attempts = FeedEventAttempts(fixture.2);
                let diagnostic = fixture
                    .3
                    .map(|error| StoredFeedDiagnostic(error.to_owned()));
                sqlx::query(
                    "INSERT INTO feed_events \
                     (feed_url, status, phase, regeneration_attempts, publication_attempts, \
                      regeneration_diagnostic, publication_diagnostic, next_attempt_at, claimed_at, \
                      terminal_at, created_at, regenerated_at, pinged_at) \
                     VALUES ($1, $2, 'regeneration', $3, 0, $4, NULL, $5, $6, $7, $8, $9, $10)",
                )
                .bind_storage(&fixture.0)
                .bind_storage(fixture.1)
                .bind_storage(attempts)
                .bind_storage(diagnostic)
                .bind_storage(fixture.4)
                .bind_storage(fixture.5)
                .bind_storage(fixture.6)
                .bind_storage(fixture.7)
                .bind_storage(fixture.8)
                .bind_storage(fixture.9)
                .execute(pool)
                .await
                .unwrap();
            }

            sqlx::query_as::<_, ClaimedFeedEventRow>(
                "SELECT id, feed_url, status, phase, regeneration_attempts, publication_attempts, \
                 regeneration_diagnostic, publication_diagnostic, next_attempt_at, claimed_at, terminal_at, \
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
            assert_eq!(row.regeneration_attempts, fixture.2);
            assert_eq!(row.regeneration_diagnostic.as_deref(), fixture.3);
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
    async fn prune_terminal_events_obeys_frozen_now_and_preserves_nonterminal_rows(
        #[case] backend: Backend,
    ) {
        let env = backend.setup().await;
        let now = fixture_instant(900_000);
        let cutoff = UtcInstant::from(now.value() - Duration::days(7));
        crate::with_closeable_pool!(env.base.pool(), pool, {
            for (path, status, terminal_at) in [
                (
                    fp("/~completed-exact-now/feed.rss"),
                    FeedEventStatus::Done,
                    Some(now),
                ),
                (
                    fp("/~failed-boundary/feed.rss"),
                    FeedEventStatus::Failed,
                    Some(cutoff),
                ),
                (
                    fp("/~failed-newer/feed.rss"),
                    FeedEventStatus::Failed,
                    Some(UtcInstant::from(cutoff.value() + Duration::seconds(1))),
                ),
                (
                    fp("/~completed-future/feed.rss"),
                    FeedEventStatus::Done,
                    Some(UtcInstant::from(now.value() + Duration::seconds(1))),
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
            4
        );
    }

    #[apply(backends)]
    #[tokio::test]
    async fn prune_terminal_events_drains_more_than_one_fixed_batch(#[case] backend: Backend) {
        let env = backend.setup().await;
        let now = fixture_instant(900_000);
        crate::with_closeable_pool!(env.base.pool(), pool, {
            for index in 0..=TERMINAL_PRUNE_LIMIT.value().unsigned_abs() {
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
            TERMINAL_PRUNE_LIMIT.value().unsigned_abs() + 1
        );
    }

    #[apply(backends)]
    #[tokio::test]
    async fn independent_writer_progresses_between_prune_batches(#[case] backend: Backend) {
        let env = backend.setup().await;
        let now = fixture_instant(900_000);
        crate::with_closeable_pool!(env.base.pool(), pool, {
            for index in 0..=TERMINAL_PRUNE_LIMIT.value().unsigned_abs() {
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

        let gate = Arc::new(PruneBatchGate::default());
        env.state
            .feed_events
            .install_prune_batch_gate(Some(Arc::clone(&gate)))
            .await;
        let feed_events = Arc::clone(&env.state.feed_events);
        let cleanup = tokio::spawn(async move { feed_events.prune_terminal_events(now).await });

        time::timeout(StdDuration::from_secs(2), gate.wait_for_batch())
            .await
            .expect("first prune batch");
        env.base
            .pool()
            .execute("INSERT INTO feed_events (feed_url) VALUES ('/~writer/feed.rss')")
            .await
            .expect("independent writer commits between cleanup batches");
        env.state.feed_events.install_prune_batch_gate(None).await;
        gate.resume();

        assert_eq!(
            cleanup
                .await
                .expect("cleanup task")
                .expect("cleanup result"),
            TERMINAL_PRUNE_LIMIT.value().unsigned_abs() + 1
        );
        assert_eq!(
            env.base
                .pool()
                .scalar_i64("SELECT COUNT(*) FROM feed_events")
                .await
                .expect("count writer row"),
            1
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
            Err(FeedEventError::Db(SqlxError::PoolClosed))
        ));
    }

    #[apply(backends)]
    #[tokio::test]
    async fn empty_id_arrays_are_noops(#[case] backend: Backend) {
        let env = backend.setup().await;
        mark_regenerated(
            &env.state.write_scope,
            Arc::clone(&env.state.feed_events),
            Vec::new(),
        )
        .await;
        mark_pinged(
            &env.state.write_scope,
            Arc::clone(&env.state.feed_events),
            Vec::new(),
            UtcInstant::now(),
        )
        .await;
        retry_regeneration(
            &env.state.write_scope,
            Arc::clone(&env.state.feed_events),
            Vec::new(),
            "x".to_owned(),
            UtcInstant::now(),
        )
        .await;
        dead_letter_regeneration(
            &env.state.write_scope,
            Arc::clone(&env.state.feed_events),
            Vec::new(),
            "x".to_owned(),
            UtcInstant::now(),
        )
        .await;
    }
}
