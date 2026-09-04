use std::collections::{HashMap, HashSet};
use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;
use std::time::{Duration, Instant};

use crate::publisher::{PublisherFinalizationGuard, PublisherService};
use crate::scheduled_worker::{ScheduledWorkerGuard, WorkTracker};
use crate::websub::{WebSubClient, WebSubError};
use chrono::Utc;
use common::ids::FeedEventId;
use common::mutation::MutationOutcome;
use common::tagged_url::{self, FeedUrl};
use common::time::UtcInstant;
use host::{
    error::{self, ErrorClass, ErrorKind, SwallowedSource},
    feed::{self, FeedEventPhase, FeedPath},
    metrics,
};
use storage::{
    CacheCommitOutcome, FeedCacheRow, FeedCacheStorage, FeedEventError, FeedEventRecord,
    FeedEventStorage, PostStorage, PublisherSnapshot, WriteScope, WriteTransaction,
};
use tokio::{
    sync::Mutex,
    time::{self, MissedTickBehavior},
};

use super::regenerate;

const BATCH_LIMIT: usize = 200;
const LEASE_TIMEOUT: Duration = Duration::from_mins(5);
const REGEN_BACKOFFS_SECS: &[u64] = &[60, 300, 1800, 7200, 7200, 7200];
const PUBLICATION_BACKOFFS_SECS: &[u64] =
    &[60, 300, 1800, 7200, 14_400, 28_800, 43_200, 86_400, 86_400];
/// Max URLs per `enqueue_many` transaction: bounds the write-lock hold of a
/// go-live fan-out (a post-outage catch-up can be arbitrarily large) so
/// batching #766's churn away can't reintroduce the long-hold failure mode.
const ENQUEUE_CHUNK: usize = 256;

fn report_continuation(
    kind: ErrorKind,
    class: ErrorClass,
    context: &'static str,
    error: &(dyn std::error::Error + 'static),
) {
    error::report_swallowed(kind, class, context, SwallowedSource::Error(error));
}

/// Converts a mutation outcome into its confirmed value for a feed operation.
fn require_confirmed_mutation<T>(
    outcome: MutationOutcome<T>,
    operation: &str,
) -> anyhow::Result<T> {
    match outcome {
        MutationOutcome::Confirmed(value) => Ok(value),
        MutationOutcome::CommitIndeterminate(_) => Err(anyhow::anyhow!(
            "{operation} commit acknowledgement was indeterminate"
        )),
    }
}
struct RetryPartition {
    buckets: HashMap<usize, Vec<FeedEventId>>,
    exhausted: Vec<FeedEventId>,
}

fn partition_retry_attempts(
    ids: &[FeedEventId],
    rows: &[FeedEventRecord],
    attempts: impl Fn(&FeedEventRecord) -> i32,
    budget_len: usize,
) -> RetryPartition {
    let attempts_by_id = rows
        .iter()
        .map(|row| (row.id, attempts(row)))
        .collect::<HashMap<_, _>>();
    let mut buckets = HashMap::<usize, Vec<FeedEventId>>::new();
    let mut exhausted = Vec::new();
    for id in ids {
        let attempts = attempts_by_id.get(id).copied().unwrap_or_default();
        let next_index = usize::try_from(attempts).unwrap_or(usize::MAX);
        if next_index >= budget_len {
            exhausted.push(*id);
        } else {
            buckets.entry(next_index).or_default().push(*id);
        }
    }
    RetryPartition { buckets, exhausted }
}

/// The background feed worker: the dependencies it needs to regenerate feeds
/// and ping the `WebSub` hub, declared explicitly as constructor parameters
/// rather than reached through a shared bundle (see [ADR-0016]).
///
/// [ADR-0016]: ../../../docs/adr/0016-dependency-injection-and-appstate.md
pub struct FeedWorker {
    posts: Arc<dyn PostStorage>,
    feed_cache: Arc<dyn FeedCacheStorage>,
    write_scope: Arc<WriteScope>,
    publisher: Arc<PublisherService>,
    feed_events: Arc<dyn FeedEventStorage>,
    websub: Arc<dyn WebSubClient>,
    /// The instant of the previous [`go_live_pass`](Self::go_live_pass), or
    /// `None` before the first pass. `None` triggers the feed-relative startup
    /// catch-up; a `Some(last)` runs the steady-state `(last, now]` window.
    last_tick: Mutex<Option<UtcInstant>>,
}

impl FeedWorker {
    /// Builds a feed worker from explicit rendering, publication, event, and
    /// `WebSub` seams. [`PublisherService`] owns coherent snapshots and finalization.
    #[must_use]
    pub fn new(
        posts: Arc<dyn PostStorage>,
        feed_cache: Arc<dyn FeedCacheStorage>,
        write_scope: Arc<WriteScope>,
        publisher: Arc<PublisherService>,
        feed_events: Arc<dyn FeedEventStorage>,
        websub: Arc<dyn WebSubClient>,
    ) -> Self {
        Self {
            posts,
            feed_cache,
            write_scope,
            publisher,
            feed_events,
            websub,
            last_tick: Mutex::new(None),
        }
    }

    #[must_use]
    pub fn posts(&self) -> &dyn PostStorage {
        self.posts.as_ref()
    }

    async fn claim_pending_batch(&self) -> anyhow::Result<Vec<FeedEventRecord>> {
        let feed_events = Arc::clone(&self.feed_events);
        require_confirmed_mutation(
            self.write_scope
                .run(move |transaction| {
                    Box::pin(async move {
                        feed_events
                            .claim_pending_batch(
                                transaction,
                                BATCH_LIMIT,
                                chrono::Duration::from_std(LEASE_TIMEOUT)
                                    .unwrap_or(chrono::Duration::seconds(300)),
                            )
                            .await
                    })
                })
                .await?,
            "feed-event claim",
        )
    }

    async fn write_event_status(
        &self,
        operation: impl for<'scope> FnOnce(
            &'scope mut WriteTransaction,
        ) -> futures_util::future::BoxFuture<
            'scope,
            Result<(), FeedEventError>,
        >,
    ) -> anyhow::Result<()> {
        require_confirmed_mutation(self.write_scope.run(operation).await?, "feed-event status")
    }

    /// Enqueues feed regeneration for posts that crossed into "live" since the
    /// last pass — the durability mechanism for future-dated posts, which reach
    /// cached feeds with no accompanying write (immediate/backdated publishes
    /// self-enqueue on the write path and are never reasoned about here).
    ///
    /// The first call (`last_tick == None`) runs the feed-relative startup
    /// catch-up: any cached feed whose surface has a live post newer than its
    /// `generated_at` is re-enqueued, healing a restart that straddled a
    /// go-live. Every later call runs the steady-state `(last_tick, now]` window
    /// pass, fanning each newly-live post out to its affected feed surfaces.
    /// Both branches seed `last_tick = now`, and both enqueue their fan-out as
    /// deduped, `ENQUEUE_CHUNK`-bounded `enqueue_many` batches — per-row
    /// autocommit enqueue loops are the `SQLite` write-lock churn that starved
    /// live requests in #766, and one unbounded transaction would be the
    /// equally-banned long hold.
    ///
    /// # Errors
    ///
    /// Returns an error if a storage read, transaction acquisition, feed-event
    /// enqueue, or commit acknowledgement fails.
    pub async fn go_live_pass(&self, now: UtcInstant) -> anyhow::Result<()> {
        let mut last_tick = self.last_tick.lock().await;
        let urls = match *last_tick {
            None => self.posts().feed_urls_needing_catchup(now).await?,
            Some(last) => {
                let mut urls = Vec::new();
                for post in self.posts().list_posts_gone_live_between(last, now).await? {
                    urls.extend(feed::affected_feed_urls(&post.username, &post.tag_slugs));
                }
                urls
            }
        };
        // Dedupe across posts: every post's fan-out contains the site-wide
        // surfaces, so two untagged posts share 3 of their 12 URLs — fewer
        // rows is the point of batching. (The drain also groups by feed_path,
        // so this only trims volume, never changes which feeds regenerate.)
        let mut seen = HashSet::new();
        let urls: Vec<FeedPath> = urls
            .into_iter()
            .filter(|url| seen.insert(url.clone()))
            .collect();
        // Bounded chunks per transaction: an idle pass yields zero chunks (no
        // storage call at all), a normal tick one, and a post-outage catch-up
        // as many bounded holds as it needs — never one unbounded hold.
        for chunk in urls.chunks(ENQUEUE_CHUNK) {
            let feed_events = Arc::clone(&self.feed_events);
            let paths = chunk.to_vec();
            let outcome = self
                .write_scope
                .run(move |transaction| {
                    Box::pin(async move { feed_events.enqueue_many(transaction, &paths).await })
                })
                .await?;
            require_confirmed_mutation(outcome, "feed-event enqueue")?;
        }
        *last_tick = Some(now);
        Ok(())
    }

    /// Claims rows, then processes each feed surface from exactly one publisher
    /// snapshot. A snapshot failure is a regeneration failure, never `NoHub`.
    pub async fn tick(&self) {
        if let Err(error) = self.go_live_pass(UtcInstant::now()).await {
            report_continuation(
                ErrorKind::Storage,
                ErrorClass::Transient,
                "server.feed.go_live_pass",
                error.as_ref(),
            );
        }
        let claimed = match self.claim_pending_batch().await {
            Ok(rows) => rows,
            Err(error) => {
                report_continuation(
                    ErrorKind::Storage,
                    ErrorClass::Transient,
                    "server.feed.claim_pending",
                    error.as_ref(),
                );
                return;
            }
        };
        let mut groups: HashMap<FeedPath, Vec<FeedEventRecord>> = HashMap::new();
        for row in claimed {
            groups.entry(row.feed_path.clone()).or_default().push(row);
        }
        for (path, rows) in groups {
            self.process_feed_group(path, rows).await;
        }
    }

    async fn process_feed_group(&self, feed_path: FeedPath, rows: Vec<FeedEventRecord>) {
        let ids: Vec<_> = rows.iter().map(|row| row.id).collect();
        let Some(snapshot) = self.snapshot_for_group(&ids, &rows).await else {
            return;
        };
        let regeneration_ids: Vec<_> = rows
            .iter()
            .filter(|row| row.phase == FeedEventPhase::Regeneration)
            .map(|row| row.id)
            .collect();
        let Some(row) = self
            .load_feed_row(&snapshot, &feed_path, &regeneration_ids, &ids, &rows)
            .await
        else {
            return;
        };
        let guard = match self.publisher.finalization_guard().await {
            Ok(guard) => guard,
            Err(error) => {
                report_continuation(
                    ErrorKind::Storage,
                    ErrorClass::Transient,
                    "server.feed.finalization_guard",
                    error.as_ref(),
                );
                self.retry_regeneration(&ids, &rows, "publisher finalization unavailable")
                    .await;
                return;
            }
        };
        if !self
            .finalize_feed_group(&guard, &snapshot, row, &regeneration_ids, &ids, &rows)
            .await
        {
            return;
        }
        self.publish_feed(&guard, &snapshot, &feed_path, &ids, &rows)
            .await;
    }

    async fn snapshot_for_group(
        &self,
        ids: &[FeedEventId],
        rows: &[FeedEventRecord],
    ) -> Option<PublisherSnapshot> {
        match self.publisher.snapshot().await {
            Ok(snapshot) => Some(snapshot),
            Err(error) => {
                report_continuation(
                    ErrorKind::Storage,
                    ErrorClass::Transient,
                    "server.feed.publisher_snapshot",
                    error.as_ref(),
                );
                self.retry_regeneration(ids, rows, "publisher snapshot read failed")
                    .await;
                None
            }
        }
    }

    async fn load_feed_row(
        &self,
        snapshot: &PublisherSnapshot,
        feed_path: &FeedPath,
        regeneration_ids: &[FeedEventId],
        ids: &[FeedEventId],
        rows: &[FeedEventRecord],
    ) -> Option<FeedCacheRow> {
        if regeneration_ids.is_empty() {
            return match self.feed_cache.get(feed_path).await {
                Ok(Some(row)) => Some(row),
                Ok(None) => {
                    self.reset_regeneration(ids).await;
                    None
                }
                Err(error) => {
                    report_continuation(
                        ErrorKind::Storage,
                        ErrorClass::Transient,
                        "server.feed.publication_cache_read",
                        &error,
                    );
                    self.retry_regeneration(ids, rows, "publication cache read failed")
                        .await;
                    None
                }
            };
        }
        let started = Instant::now();
        match regenerate::render(snapshot, self.posts(), feed_path.clone()).await {
            Ok(row) => {
                metrics::feed_regeneration(metrics::RegenResult::Ok);
                metrics::feed_regen_duration_ms(
                    u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX),
                );
                Some(row)
            }
            Err(error) => {
                report_continuation(
                    ErrorKind::Internal,
                    ErrorClass::Bug,
                    "server.feed.regenerate",
                    &error,
                );
                self.retry_regeneration(regeneration_ids, rows, &error.to_string())
                    .await;
                None
            }
        }
    }

    async fn finalize_feed_group(
        &self,
        guard: &PublisherFinalizationGuard,
        snapshot: &PublisherSnapshot,
        row: FeedCacheRow,
        regeneration_ids: &[FeedEventId],
        ids: &[FeedEventId],
        rows: &[FeedEventRecord],
    ) -> bool {
        if regeneration_ids.is_empty() {
            return match guard.is_current(snapshot.generation).await {
                Ok(true) => true,
                Ok(false) => {
                    self.restart_regeneration(ids).await;
                    false
                }
                Err(error) => {
                    report_continuation(
                        ErrorKind::Storage,
                        ErrorClass::Transient,
                        "server.feed.publication_generation_check",
                        &error,
                    );
                    self.retry_regeneration(ids, rows, "publisher generation check failed")
                        .await;
                    false
                }
            };
        }
        match guard.commit_cache(snapshot.generation, row).await {
            Ok(CacheCommitOutcome::Committed) => self.mark_regenerated(regeneration_ids).await,
            Ok(CacheCommitOutcome::StaleGeneration) => {
                self.restart_regeneration(ids).await;
                false
            }
            Err(error) => {
                report_continuation(
                    ErrorKind::Storage,
                    ErrorClass::Transient,
                    "server.feed.cache_commit",
                    &error,
                );
                self.retry_regeneration(ids, rows, "cache commit failed")
                    .await;
                false
            }
        }
    }

    async fn publish_feed(
        &self,
        _guard: &PublisherFinalizationGuard,
        snapshot: &PublisherSnapshot,
        feed_path: &FeedPath,
        ids: &[FeedEventId],
        rows: &[FeedEventRecord],
    ) {
        let Some(hub) = snapshot.feeds.websub_hub_url.as_ref() else {
            metrics::websub_ping(metrics::PingOutcome::NoHub);
            self.mark_pinged(ids).await;
            return;
        };
        let Some(base) = snapshot.identity.base_url.as_ref() else {
            self.retry_regeneration(ids, rows, "site.base_url is unset")
                .await;
            return;
        };
        let absolute: FeedUrl = tagged_url::compose(base, feed_path);
        match self.websub.send_publish(hub, &absolute).await {
            Ok(()) => {
                metrics::websub_ping(metrics::PingOutcome::Success);
                self.mark_pinged(ids).await;
            }
            Err(WebSubError::Terminal { reason }) => {
                metrics::websub_ping(metrics::PingOutcome::Terminal);
                self.dead_letter_publication(ids, &reason.to_string()).await;
            }
            Err(WebSubError::Retryable {
                reason,
                retry_after,
            }) => {
                self.retry_publication(ids, rows, &reason.to_string(), retry_after)
                    .await;
            }
        }
    }

    async fn mark_regenerated(&self, ids: &[FeedEventId]) -> bool {
        let events = Arc::clone(&self.feed_events);
        let ids = ids.to_vec();
        match self
            .write_event_status(move |tx| {
                Box::pin(async move { events.mark_regenerated(tx, &ids).await })
            })
            .await
        {
            Ok(()) => true,
            Err(error) => {
                report_continuation(
                    ErrorKind::Storage,
                    ErrorClass::Transient,
                    "server.feed.status_write.regenerated",
                    error.as_ref(),
                );
                false
            }
        }
    }

    async fn mark_pinged(&self, ids: &[FeedEventId]) {
        let events = Arc::clone(&self.feed_events);
        let ids = ids.to_vec();
        let now = UtcInstant::now();
        match self
            .write_event_status(move |tx| {
                Box::pin(async move { events.mark_pinged(tx, &ids, now).await })
            })
            .await
        {
            Ok(()) => tracing::info!(
                message = "feed.event.terminal",
                phase = "publication",
                outcome = "completed"
            ),
            Err(error) => report_continuation(
                ErrorKind::Storage,
                ErrorClass::Transient,
                "server.feed.status_write.publication_complete",
                error.as_ref(),
            ),
        }
    }

    async fn dead_letter_regeneration(&self, ids: &[FeedEventId], message: &str) {
        let events = Arc::clone(&self.feed_events);
        let ids = ids.to_vec();
        let message = message.to_owned();
        let now = UtcInstant::now();
        match self
            .write_event_status(move |tx| {
                Box::pin(async move {
                    events
                        .dead_letter_regeneration(tx, &ids, &message, now)
                        .await
                })
            })
            .await
        {
            Ok(()) => tracing::info!(
                message = "feed.event.terminal",
                phase = "regeneration",
                outcome = "exhausted"
            ),
            Err(error) => report_continuation(
                ErrorKind::Storage,
                ErrorClass::Transient,
                "server.feed.status_write.regeneration_dead_letter",
                error.as_ref(),
            ),
        }
    }

    async fn dead_letter_publication(&self, ids: &[FeedEventId], message: &str) {
        let events = Arc::clone(&self.feed_events);
        let ids = ids.to_vec();
        let message = message.to_owned();
        let now = UtcInstant::now();
        match self
            .write_event_status(move |tx| {
                Box::pin(async move {
                    events
                        .dead_letter_publication(tx, &ids, &message, now)
                        .await
                })
            })
            .await
        {
            Ok(()) => tracing::info!(
                message = "feed.event.terminal",
                phase = "publication",
                outcome = "exhausted"
            ),
            Err(error) => report_continuation(
                ErrorKind::Storage,
                ErrorClass::Transient,
                "server.feed.status_write.publication_dead_letter",
                error.as_ref(),
            ),
        }
    }

    async fn retry_regeneration(
        &self,
        ids: &[FeedEventId],
        rows: &[FeedEventRecord],
        message: &str,
    ) {
        let RetryPartition {
            buckets: retry_buckets,
            exhausted,
        } = partition_retry_attempts(
            ids,
            rows,
            |row| row.regeneration_attempts,
            REGEN_BACKOFFS_SECS.len(),
        );
        if !exhausted.is_empty() {
            metrics::feed_regeneration(metrics::RegenResult::Exhausted);
            self.dead_letter_regeneration(&exhausted, message).await;
        }
        if retry_buckets.is_empty() {
            return;
        }
        metrics::feed_regeneration(metrics::RegenResult::Error);
        for (next_index, ids) in retry_buckets {
            let next = UtcInstant::from(
                Utc::now()
                    + chrono::Duration::from_std(Duration::from_secs(
                        REGEN_BACKOFFS_SECS[next_index],
                    ))
                    .unwrap_or(chrono::Duration::hours(24)),
            );
            let events = Arc::clone(&self.feed_events);
            let message = message.to_owned();
            if let Err(error) = self
                .write_event_status(move |tx| {
                    Box::pin(
                        async move { events.retry_regeneration(tx, &ids, &message, next).await },
                    )
                })
                .await
            {
                report_continuation(
                    ErrorKind::Storage,
                    ErrorClass::Transient,
                    "server.feed.status_write.regeneration_retry",
                    error.as_ref(),
                );
            }
        }
    }

    async fn retry_publication(
        &self,
        ids: &[FeedEventId],
        rows: &[FeedEventRecord],
        message: &str,
        retry_after: Option<Duration>,
    ) {
        let RetryPartition {
            buckets: retry_buckets,
            exhausted,
        } = partition_retry_attempts(
            ids,
            rows,
            |row| row.publication_attempts,
            PUBLICATION_BACKOFFS_SECS.len(),
        );
        if !exhausted.is_empty() {
            metrics::websub_ping(metrics::PingOutcome::Exhausted);
            self.dead_letter_publication(&exhausted, message).await;
        }
        if retry_buckets.is_empty() {
            return;
        }
        metrics::websub_ping(metrics::PingOutcome::Failed);
        for (next_index, ids) in retry_buckets {
            let delay = retry_after.map_or_else(
                || Duration::from_secs(PUBLICATION_BACKOFFS_SECS[next_index]),
                |delay| delay.min(Duration::from_hours(24)),
            );
            let next = UtcInstant::from(
                Utc::now()
                    + chrono::Duration::from_std(delay).unwrap_or(chrono::Duration::hours(24)),
            );
            let events = Arc::clone(&self.feed_events);
            let message = message.to_owned();
            if let Err(error) = self
                .write_event_status(move |tx| {
                    Box::pin(
                        async move { events.retry_publication(tx, &ids, &message, next).await },
                    )
                })
                .await
            {
                report_continuation(
                    ErrorKind::Storage,
                    ErrorClass::Transient,
                    "server.feed.status_write.publication_retry",
                    error.as_ref(),
                );
            }
        }
    }

    async fn restart_regeneration(&self, ids: &[FeedEventId]) {
        let events = Arc::clone(&self.feed_events);
        let ids = ids.to_vec();
        let now = UtcInstant::now();
        if let Err(error) = self
            .write_event_status(move |tx| {
                Box::pin(async move { events.restart_regeneration(tx, &ids, now).await })
            })
            .await
        {
            report_continuation(
                ErrorKind::Storage,
                ErrorClass::Transient,
                "server.feed.status_write.stale_generation",
                error.as_ref(),
            );
        }
    }

    async fn reset_regeneration(&self, ids: &[FeedEventId]) {
        let events = Arc::clone(&self.feed_events);
        let ids = ids.to_vec();
        let now = UtcInstant::now();
        if let Err(error) = self
            .write_event_status(move |tx| {
                Box::pin(async move { events.reset_regeneration(tx, &ids, now).await })
            })
            .await
        {
            report_continuation(
                ErrorKind::Storage,
                ErrorClass::Transient,
                "server.feed.status_write.missing_cache",
                error.as_ref(),
            );
        }
    }

    /// Starts the feed worker scheduler at the cadence selected by the
    /// composition root. Subsecond cadences use one scheduler activation to
    /// drive a `Tokio` interval because `tokio-cron-scheduler` stores repeated
    /// durations at whole-second precision. The returned guard owns scheduler
    /// admission and drains admitted work during shutdown.
    ///
    /// # Errors
    ///
    /// Returns an error if the interval is zero or the scheduler fails to start.
    pub(crate) async fn start(self, interval: Duration) -> anyhow::Result<ScheduledWorkerGuard> {
        anyhow::ensure!(!interval.is_zero(), "feed worker interval must be non-zero");

        let worker = Arc::new(self);
        let scheduler = tokio_cron_scheduler::JobScheduler::new().await?;
        let tracker = WorkTracker::default();
        let job = if interval < Duration::from_secs(1) {
            // cov:ignore-start -- the closure body fires only when the scheduler
            // activates it; tick behavior is unit-tested through spawn_tick.
            let job_tracker = tracker.clone();
            let stop_tracker = tracker.clone();
            tokio_cron_scheduler::Job::new_one_shot_async(Duration::ZERO, move |_uuid, _lock| {
                let worker = worker.clone();
                let tracker = job_tracker.clone();
                let stop = stop_tracker.clone();
                Box::pin(tracker.run(async move {
                    let mut ticker = time::interval(interval);
                    ticker.set_missed_tick_behavior(MissedTickBehavior::Skip);
                    loop {
                        tokio::select! {
                            biased;
                            () = stop.stopped() => break,
                            _ = ticker.tick() => spawn_tick(worker.clone()).await,
                        }
                    }
                }))
            })?
        } else {
            let job_tracker = tracker.clone();
            tokio_cron_scheduler::Job::new_repeated_async(interval, move |_uuid, _lock| {
                let tracker = job_tracker.clone();
                Box::pin(tracker.run(spawn_tick(worker.clone())))
            })?
        };
        // cov:ignore-stop
        scheduler.add(job).await?;
        ScheduledWorkerGuard::start(scheduler, tracker).await
    }
}

/// Drives one [`FeedWorker::tick`] as an owned, boxed future — the body the cron
/// scheduler runs on every fire. Extracted from the scheduler closure so its
/// single meaningful statement sits on an ordinary, testable line rather than
/// inside a closure the scheduler only ever invokes at runtime.
fn spawn_tick(worker: Arc<FeedWorker>) -> Pin<Box<dyn Future<Output = ()> + Send>> {
    Box::pin(async move {
        worker.tick().await;
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::websub::NoopWebSubClient;
    use common::tagged_url::HubUrl;
    use host::feed::FeedEventStatus;
    use sqlx::Error as SqlxError;
    use storage::{
        FeedEventError, FeedEventRecord, MockFeedCacheStorage, MockFeedEventStorage,
        MockPublisherStorage, test_support::mock_write_scope,
    };

    fn event(id: i64, feed_url: &str, attempts: i32) -> FeedEventRecord {
        let now = UtcInstant::now();
        FeedEventRecord {
            id: FeedEventId::from(id),
            feed_path: feed_url.parse().expect("valid feed path in test"),
            status: FeedEventStatus::Claimed,
            phase: host::feed::FeedEventPhase::Regeneration,
            regeneration_attempts: attempts,
            publication_attempts: 0,
            regeneration_diagnostic: None,
            publication_diagnostic: None,
            next_attempt_at: now,
            claimed_at: Some(now),
            terminal_at: None,
            created_at: now,
            regenerated_at: None,
            pinged_at: None,
        }
    }

    #[test]
    fn confirmed_mutation_outcome_returns_value() {
        assert_eq!(
            require_confirmed_mutation(
                common::mutation::MutationOutcome::Confirmed("confirmed"),
                "feed-event claim",
            )
            .expect("confirmed outcome"),
            "confirmed"
        );
    }

    #[test]
    fn indeterminate_mutation_outcome_preserves_operation_message() {
        let error = require_confirmed_mutation(
            common::mutation::MutationOutcome::CommitIndeterminate(()),
            "feed-event enqueue",
        )
        .expect_err("indeterminate outcome");

        assert_eq!(
            error.to_string(),
            "feed-event enqueue commit acknowledgement was indeterminate"
        );
    }

    #[derive(Clone)]
    struct SharedWriter(Arc<std::sync::Mutex<Vec<u8>>>);

    impl std::io::Write for SharedWriter {
        fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
            self.0.lock().expect("trace lock").extend_from_slice(bytes);
            Ok(bytes.len())
        }

        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }

    impl<'writer> tracing_subscriber::fmt::MakeWriter<'writer> for SharedWriter {
        type Writer = Self;

        fn make_writer(&'writer self) -> Self::Writer {
            self.clone()
        }
    }

    fn trace_capture() -> (
        tracing::subscriber::DefaultGuard,
        Arc<std::sync::Mutex<Vec<u8>>>,
    ) {
        let output = Arc::new(std::sync::Mutex::new(Vec::new()));
        let subscriber = tracing_subscriber::fmt()
            .json()
            .with_ansi(false)
            .with_writer(SharedWriter(output.clone()))
            .finish();
        (tracing::subscriber::set_default(subscriber), output)
    }

    fn trace_text(output: &Arc<std::sync::Mutex<Vec<u8>>>) -> String {
        std::io::Write::flush(&mut SharedWriter(output.clone())).expect("flush trace");
        String::from_utf8(output.lock().expect("trace lock").clone()).expect("utf8 trace")
    }

    struct FailingWebSubClient;

    #[async_trait::async_trait]
    impl WebSubClient for FailingWebSubClient {
        async fn send_publish(
            &self,
            _hub_url: &HubUrl,
            _feed_url: &FeedUrl,
        ) -> Result<(), crate::websub::WebSubError> {
            Err(crate::websub::WebSubError::Retryable {
                reason: crate::websub::RetryableWebSubError::Transport(Box::new(
                    std::io::Error::other("worker WebSub transport failure"),
                )),
                retry_after: None,
            })
        }
    }

    fn test_publisher() -> Arc<PublisherService> {
        Arc::new(PublisherService::new(
            std::env::temp_dir(),
            Arc::new(MockPublisherStorage::new()),
            mock_write_scope(),
        ))
    }
    fn worker(
        posts: storage::MockPostStorage,
        feed_cache: MockFeedCacheStorage,
        feed_events: MockFeedEventStorage,
    ) -> FeedWorker {
        FeedWorker::new(
            Arc::new(posts),
            Arc::new(feed_cache),
            Arc::new(mock_write_scope()),
            test_publisher(),
            Arc::new(feed_events),
            Arc::new(NoopWebSubClient),
        )
    }

    fn worker_with_websub(
        feed_events: MockFeedEventStorage,
        websub: Arc<dyn WebSubClient>,
    ) -> FeedWorker {
        FeedWorker::new(
            Arc::new(storage::MockPostStorage::new()),
            Arc::new(storage::MockFeedCacheStorage::new()),
            Arc::new(mock_write_scope()),
            test_publisher(),
            Arc::new(feed_events),
            websub,
        )
    }

    fn assert_context_once(trace: &str, context: &str) {
        assert_eq!(
            trace
                .matches(format!(r#""error.context":"{context}""#).as_str())
                .count(),
            1,
            "trace: {trace}"
        );
    }

    #[tokio::test]
    async fn terminal_transitions_use_explicit_storage_apis() {
        let mut events = MockFeedEventStorage::new();
        events
            .expect_mark_pinged()
            .times(1)
            .returning(|_, _, _| Ok(()));
        events
            .expect_dead_letter_publication()
            .times(1)
            .returning(|_, _, _, _| Ok(()));
        let worker = worker_with_websub(events, Arc::new(NoopWebSubClient));
        let (guard, output) = trace_capture();
        worker.mark_pinged(&[FeedEventId::from(7)]).await;
        worker
            .dead_letter_publication(&[FeedEventId::from(8)], "private failure detail")
            .await;
        drop(guard);

        let trace = trace_text(&output);
        assert_eq!(
            trace.matches(r#""message":"feed.event.terminal""#).count(),
            2
        );
        assert!(trace.contains(r#""phase":"publication""#));
        assert!(trace.contains(r#""outcome":"completed""#));
        assert!(trace.contains(r#""outcome":"exhausted""#));
        assert!(!trace.contains("private failure detail"));
        assert!(!trace.contains("/feed"));
    }

    #[tokio::test]
    async fn snapshot_failure_retries_regeneration_and_reports_once() {
        let mut publisher_storage = MockPublisherStorage::new();
        publisher_storage
            .expect_snapshot()
            .times(1)
            .returning(|| Err(storage::PublisherStorageError::Db(SqlxError::PoolClosed)));
        let publisher = Arc::new(PublisherService::new(
            std::env::temp_dir(),
            Arc::new(publisher_storage),
            mock_write_scope(),
        ));
        let mut posts = storage::MockPostStorage::new();
        posts
            .expect_feed_urls_needing_catchup()
            .times(1)
            .returning(|_| Ok(vec![]));
        let mut events = MockFeedEventStorage::new();
        events
            .expect_claim_pending_batch()
            .times(1)
            .returning(|_, _, _| Ok(vec![event(1, "/feed.rss", 0)]));
        events
            .expect_retry_regeneration()
            .times(1)
            .returning(|_, ids, error, _| {
                assert_eq!(ids, &[FeedEventId::from(1)]);
                assert_eq!(error, "publisher snapshot read failed");
                Ok(())
            });
        let worker = FeedWorker::new(
            Arc::new(posts),
            Arc::new(MockFeedCacheStorage::new()),
            Arc::new(mock_write_scope()),
            publisher,
            Arc::new(events),
            Arc::new(NoopWebSubClient),
        );

        let (guard, output) = trace_capture();
        worker.tick().await;
        drop(guard);
        assert_context_once(&trace_text(&output), "server.feed.publisher_snapshot");
    }
    #[tokio::test]
    async fn missing_cache_resets_regeneration_budget_without_retry() {
        let mut events = MockFeedEventStorage::new();
        events
            .expect_reset_regeneration()
            .times(1)
            .returning(|_, ids, _| {
                assert_eq!(ids, &[FeedEventId::from(1)]);
                Ok(())
            });
        let worker = worker_with_websub(events, Arc::new(NoopWebSubClient));
        worker.reset_regeneration(&[FeedEventId::from(1)]).await;
    }

    #[tokio::test]
    async fn stale_generation_restarts_regeneration_without_retry_charge() {
        let mut events = MockFeedEventStorage::new();
        events
            .expect_restart_regeneration()
            .times(1)
            .returning(|_, ids, _| {
                assert_eq!(ids, &[FeedEventId::from(1)]);
                Ok(())
            });
        let worker = worker_with_websub(events, Arc::new(NoopWebSubClient));
        worker.restart_regeneration(&[FeedEventId::from(1)]).await;
    }

    #[tokio::test]
    async fn mark_regenerated_status_failure_is_reported_once() {
        let mut events = MockFeedEventStorage::new();
        events
            .expect_mark_regenerated()
            .times(1)
            .returning(|_, _| Err(FeedEventError::Db(SqlxError::PoolClosed)));
        let worker = worker_with_websub(events, Arc::new(NoopWebSubClient));

        let (guard, output) = trace_capture();
        assert!(!worker.mark_regenerated(&[FeedEventId::from(1)]).await);
        drop(guard);
        assert_context_once(&trace_text(&output), "server.feed.status_write.regenerated");
    }

    // guard:no-backend — mock status store and successful protocol client.
    #[tokio::test]
    async fn continuation_reporting_websub_success_survives_mark_pinged_failure() {
        let mut events = MockFeedEventStorage::new();
        events
            .expect_mark_pinged()
            .times(1)
            .returning(|_, _, _| Err(FeedEventError::Db(SqlxError::PoolClosed)));
        let worker = worker_with_websub(events, Arc::new(NoopWebSubClient));

        let (guard, output) = trace_capture();
        worker.mark_pinged(&[FeedEventId::from(1)]).await;
        drop(guard);
        assert_context_once(
            &trace_text(&output),
            "server.feed.status_write.publication_complete",
        );
    }

    // guard:no-backend — mock status store and failing protocol client.
    #[tokio::test]
    async fn continuation_reporting_publication_dead_letter_survives_status_write_failure() {
        let mut events = MockFeedEventStorage::new();
        events
            .expect_dead_letter_publication()
            .times(1)
            .returning(|_, _, _, _| Err(FeedEventError::Db(SqlxError::PoolClosed)));
        let worker = worker_with_websub(events, Arc::new(FailingWebSubClient));

        let (guard, output) = trace_capture();
        worker
            .dead_letter_publication(&[FeedEventId::from(1)], "publication failed")
            .await;
        drop(guard);
        assert_context_once(
            &trace_text(&output),
            "server.feed.status_write.publication_dead_letter",
        );
    }

    // guard:no-backend — mock status store isolates the explicit transition.
    #[tokio::test]
    async fn continuation_reporting_publication_retry_survives_status_write_failure() {
        let mut events = MockFeedEventStorage::new();
        events
            .expect_retry_publication()
            .times(1)
            .returning(|_, _, _, _| Err(FeedEventError::Db(SqlxError::PoolClosed)));
        let worker = worker_with_websub(events, Arc::new(FailingWebSubClient));
        let row = event(1, "/feed.rss", 0);

        let (guard, output) = trace_capture();
        worker
            .retry_publication(
                &[row.id],
                std::slice::from_ref(&row),
                "publication failed",
                None,
            )
            .await;
        drop(guard);
        assert_context_once(
            &trace_text(&output),
            "server.feed.status_write.publication_retry",
        );
    }

    // guard:no-backend — mock status store isolates regeneration retry.
    #[tokio::test]
    async fn continuation_reporting_regeneration_retry_survives_status_write_failure() {
        let mut events = MockFeedEventStorage::new();
        events
            .expect_retry_regeneration()
            .times(1)
            .returning(|_, _, _, _| Err(FeedEventError::Db(SqlxError::PoolClosed)));
        let worker = worker_with_websub(events, Arc::new(NoopWebSubClient));
        let record = event(1, "/feed.rss", 0);

        let (guard, output) = trace_capture();
        worker
            .retry_regeneration(
                &[record.id],
                std::slice::from_ref(&record),
                "regeneration failed",
            )
            .await;
        drop(guard);
        assert_context_once(
            &trace_text(&output),
            "server.feed.status_write.regeneration_retry",
        );
    }

    // guard:no-backend — mock store
    #[tokio::test]
    async fn grouped_regeneration_attempts_partition_retry_and_dead_letter() {
        let mut events = MockFeedEventStorage::new();
        events
            .expect_retry_regeneration()
            .times(1)
            .returning(|_, ids, error, _| {
                assert_eq!(ids, &[FeedEventId::from(1)]);
                assert_eq!(error, "regeneration failed");
                Ok(())
            });
        events
            .expect_dead_letter_regeneration()
            .times(1)
            .returning(|_, ids, error, _| {
                assert_eq!(ids, &[FeedEventId::from(2)]);
                assert_eq!(error, "regeneration failed");
                Ok(())
            });
        let worker = worker_with_websub(events, Arc::new(NoopWebSubClient));
        let retry = event(1, "/feed.rss", 0);
        let exhausted = event(2, "/feed.rss", 6);
        let ids = [retry.id, exhausted.id];

        worker
            .retry_regeneration(&ids, &[retry, exhausted], "regeneration failed")
            .await;
    }

    // guard:no-backend — mock store
    #[tokio::test]
    async fn grouped_publication_attempts_partition_retry_and_dead_letter() {
        let mut events = MockFeedEventStorage::new();
        events
            .expect_retry_publication()
            .times(1)
            .returning(|_, ids, error, _| {
                assert_eq!(ids, &[FeedEventId::from(1)]);
                assert_eq!(error, "publication failed");
                Ok(())
            });
        events
            .expect_dead_letter_publication()
            .times(1)
            .returning(|_, ids, error, _| {
                assert_eq!(ids, &[FeedEventId::from(2)]);
                assert_eq!(error, "publication failed");
                Ok(())
            });
        let worker = worker_with_websub(events, Arc::new(NoopWebSubClient));
        let mut retry = event(1, "/feed.rss", 0);
        retry.phase = host::feed::FeedEventPhase::Publication;
        let mut exhausted = event(2, "/feed.rss", 0);
        exhausted.phase = host::feed::FeedEventPhase::Publication;
        exhausted.publication_attempts = 9;

        worker
            .retry_publication(
                &[retry.id, exhausted.id],
                &[retry, exhausted],
                "publication failed",
                None,
            )
            .await;
    }

    // guard:no-backend — mock store
    #[tokio::test]
    async fn regeneration_attempt_seven_is_terminal() {
        let mut events = MockFeedEventStorage::new();
        events
            .expect_dead_letter_regeneration()
            .times(1)
            .returning(|_, ids, error, _| {
                assert_eq!(ids, &[FeedEventId::from(1)]);
                assert_eq!(error, "regeneration failed");
                Ok(())
            });
        let worker = worker_with_websub(events, Arc::new(NoopWebSubClient));
        let record = event(1, "/feed.rss", 6);

        worker
            .retry_regeneration(
                &[record.id],
                std::slice::from_ref(&record),
                "regeneration failed",
            )
            .await;
    }

    #[tokio::test]
    async fn publication_attempt_ten_is_terminal() {
        let mut events = MockFeedEventStorage::new();
        events
            .expect_dead_letter_publication()
            .times(1)
            .returning(|_, ids, error, _| {
                assert_eq!(ids, &[FeedEventId::from(1)]);
                assert_eq!(error, "publication failed");
                Ok(())
            });
        let worker = worker_with_websub(events, Arc::new(NoopWebSubClient));
        let mut record = event(1, "/feed.rss", 0);
        record.phase = host::feed::FeedEventPhase::Publication;
        record.publication_attempts = 9;

        worker
            .retry_publication(
                &[record.id],
                std::slice::from_ref(&record),
                "publication failed",
                Some(Duration::from_secs(1)),
            )
            .await;
    }
    #[tokio::test]
    async fn publication_retry_after_overrides_backoff() {
        let before = Utc::now();
        let mut events = MockFeedEventStorage::new();
        events
            .expect_retry_publication()
            .times(1)
            .returning(move |_, _, _, next| {
                assert!(next.value() >= before + chrono::Duration::seconds(2));
                assert!(next.value() <= before + chrono::Duration::seconds(4));
                Ok(())
            });
        let worker = worker_with_websub(events, Arc::new(NoopWebSubClient));
        let mut record = event(1, "/feed.rss", 0);
        record.phase = host::feed::FeedEventPhase::Publication;

        worker
            .retry_publication(
                &[record.id],
                std::slice::from_ref(&record),
                "publication failed",
                Some(Duration::from_secs(3)),
            )
            .await;
    }

    #[tokio::test]
    async fn tick_reports_and_returns_when_claim_fails() {
        let mut posts = storage::MockPostStorage::new();
        posts
            .expect_feed_urls_needing_catchup()
            .times(0..)
            .returning(|_| Ok(vec![]));
        let mut events = MockFeedEventStorage::new();
        events
            .expect_claim_pending_batch()
            .times(1)
            .returning(|_, _, _| Err(FeedEventError::Db(SqlxError::PoolClosed)));
        // No mark_* expectation is set: any call after the claim error would
        // panic as an unexpected call, proving the tick returned early.
        let w = worker(posts, storage::MockFeedCacheStorage::new(), events);
        let (guard, output) = trace_capture();
        w.tick().await;
        drop(guard);
        let trace = trace_text(&output);
        assert_eq!(
            trace
                .matches(r#""error.context":"server.feed.claim_pending""#)
                .count(),
            1,
            "trace: {trace}"
        );
    }

    // guard:no-backend — mock store
    #[tokio::test]
    async fn tick_returns_when_batch_is_empty() {
        let mut posts = storage::MockPostStorage::new();
        posts
            .expect_feed_urls_needing_catchup()
            .times(0..)
            .returning(|_| Ok(vec![]));
        let mut events = MockFeedEventStorage::new();
        events
            .expect_claim_pending_batch()
            .times(1)
            .returning(|_, _, _| Ok(vec![]));
        let w = worker(posts, storage::MockFeedCacheStorage::new(), events);
        w.tick().await;
    }

    // guard:no-backend — mock store
    #[tokio::test]
    async fn tick_reports_when_go_live_pass_fails_but_still_drains() {
        let mut posts = storage::MockPostStorage::new();
        // Go-live pass fails and is reported, but the tick continues to the
        // independent empty queue drain.
        posts
            .expect_feed_urls_needing_catchup()
            .times(1)
            .returning(|_| Err(SqlxError::PoolClosed));
        let mut events = MockFeedEventStorage::new();
        events
            .expect_claim_pending_batch()
            .times(1)
            .returning(|_, _, _| Ok(vec![]));
        let w = worker(posts, storage::MockFeedCacheStorage::new(), events);
        let (guard, output) = trace_capture();
        w.tick().await;
        drop(guard);
        let trace = trace_text(&output);
        assert_eq!(
            trace
                .matches(r#""error.context":"server.feed.go_live_pass""#)
                .count(),
            1,
            "trace: {trace}"
        );
    }

    // guard:no-backend — mock store and failing protocol client.
    #[tokio::test]
    async fn publication_retry_persists_typed_error_message() {
        let mut events = MockFeedEventStorage::new();
        events
            .expect_retry_publication()
            .times(1)
            .returning(|_, _, error, _| {
                assert_eq!(
                    error,
                    "WebSub publish is retryable: WebSub transport failed"
                );
                Ok(())
            });
        let worker = worker_with_websub(events, Arc::new(FailingWebSubClient));
        let row = event(1, "/feed.rss", 0);
        worker
            .retry_publication(
                &[row.id],
                std::slice::from_ref(&row),
                "WebSub publish is retryable: WebSub transport failed",
                None,
            )
            .await;
    }

    // guard:no-backend — mock status store.
    #[tokio::test]
    async fn completed_ping_keeps_success_primary_and_reports_status_failure_once() {
        let mut events = MockFeedEventStorage::new();
        events
            .expect_mark_pinged()
            .times(1)
            .returning(|_, _, _| Err(FeedEventError::Db(SqlxError::PoolClosed)));
        let worker = worker_with_websub(events, Arc::new(NoopWebSubClient));
        let (guard, output) = trace_capture();
        worker.mark_pinged(&[FeedEventId::from(1)]).await;
        drop(guard);
        let trace = trace_text(&output);
        assert_eq!(
            trace
                .matches(r#""error.context":"server.feed.status_write.publication_complete""#)
                .count(),
            1,
            "trace: {trace}"
        );
    }

    // guard:no-backend — mock store
    #[tokio::test]
    async fn go_live_catchup_enqueues_all_surfaces_in_one_batch() {
        let mut posts = storage::MockPostStorage::new();
        posts
            .expect_feed_urls_needing_catchup()
            .times(1)
            .returning(|_| {
                Ok(vec![
                    "/feed.rss".parse().expect("valid feed path in test"),
                    "/~alice/feed.rss".parse().expect("valid feed path in test"),
                    "/tags/t/feed.rss".parse().expect("valid feed path in test"),
                ])
            });
        let mut events = MockFeedEventStorage::new();
        // The regression gate for #766: the whole catch-up fans out as ONE
        // batched write, and the per-row API is never used.
        events
            .expect_enqueue_many()
            .times(1)
            .withf(|_, paths| paths.len() == 3)
            .returning(|_, _| Ok(()));
        events.expect_enqueue().times(0);
        let w = worker(posts, storage::MockFeedCacheStorage::new(), events);
        w.go_live_pass(UtcInstant::now())
            .await
            .expect("catch-up pass");
    }
    // guard:no-backend — mock store
    #[tokio::test]
    async fn go_live_catchup_bounds_each_enqueue_batch() {
        let mut posts = storage::MockPostStorage::new();
        posts
            .expect_feed_urls_needing_catchup()
            .times(1)
            .returning(|_| {
                Ok((0..=ENQUEUE_CHUNK)
                    .map(|index| {
                        format!("/tags/{index}/feed.rss")
                            .parse()
                            .expect("valid feed path in test")
                    })
                    .collect())
            });
        let mut events = MockFeedEventStorage::new();
        events
            .expect_enqueue_many()
            .times(2)
            .withf(|_, paths| paths.len() <= ENQUEUE_CHUNK)
            .returning(|_, _| Ok(()));
        events.expect_enqueue().times(0);
        let w = worker(posts, storage::MockFeedCacheStorage::new(), events);

        w.go_live_pass(UtcInstant::now())
            .await
            .expect("bounded catch-up pass");
    }

    // guard:no-backend — mock store
    #[tokio::test]
    async fn go_live_window_enqueues_all_surfaces_in_one_batch() {
        let mut posts = storage::MockPostStorage::new();
        // First pass primes last_tick (catch-up, nothing to do); no enqueue
        // call may result from it.
        posts
            .expect_feed_urls_needing_catchup()
            .times(1)
            .returning(|_| Ok(vec![]));
        // Second pass: two just-live untagged posts -> 2 surfaces x 3 formats
        // each = 12 URLs, minus the 3 site-wide surfaces both posts share
        // (deduped) = 9, all in ONE batched write.
        posts
            .expect_list_posts_gone_live_between()
            .times(1)
            .returning(|_, _| {
                Ok(vec![
                    storage::GoLivePost {
                        username: common::test_support::parse_username("alice"),
                        tag_slugs: vec![],
                    },
                    storage::GoLivePost {
                        username: common::test_support::parse_username("bob"),
                        tag_slugs: vec![],
                    },
                ])
            });
        let mut events = storage::MockFeedEventStorage::new();
        events
            .expect_enqueue_many()
            .times(1)
            .withf(|_, paths| paths.len() == 9)
            .returning(|_, _| Ok(()));
        events.expect_enqueue().times(0);
        let w = worker(posts, storage::MockFeedCacheStorage::new(), events);
        w.go_live_pass(UtcInstant::now())
            .await
            .expect("priming pass");
        w.go_live_pass(UtcInstant::now())
            .await
            .expect("windowed pass");
    }

    #[tokio::test]
    async fn regeneration_dead_letter_status_failure_is_reported_once() {
        let mut events = MockFeedEventStorage::new();
        events
            .expect_dead_letter_regeneration()
            .times(1)
            .returning(|_, _, _, _| Err(FeedEventError::Db(SqlxError::PoolClosed)));
        let worker = worker_with_websub(events, Arc::new(NoopWebSubClient));
        let record = event(1, "/feed.rss", 6);

        let (guard, output) = trace_capture();
        worker
            .retry_regeneration(
                &[record.id],
                std::slice::from_ref(&record),
                "regeneration failed",
            )
            .await;
        drop(guard);
        assert_context_once(
            &trace_text(&output),
            "server.feed.status_write.regeneration_dead_letter",
        );
    }

    // guard:no-backend — mock store
    #[tokio::test]
    async fn spawn_tick_drives_one_tick() {
        // The scheduler-closure body: `spawn_tick` boxes a future that runs a
        // single tick. Awaiting it exercises the same code the cron job fires.
        let mut posts = storage::MockPostStorage::new();
        posts
            .expect_feed_urls_needing_catchup()
            .times(0..)
            .returning(|_| Ok(vec![]));
        let mut events = storage::MockFeedEventStorage::new();
        events
            .expect_claim_pending_batch()
            .times(1)
            .returning(|_, _, _| Ok(vec![]));
        let w = worker(posts, storage::MockFeedCacheStorage::new(), events);
        spawn_tick(Arc::new(w)).await;
    }
}
