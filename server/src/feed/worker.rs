use std::collections::{HashMap, HashSet};
use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;
use std::time::Duration;

use crate::websub::WebSubClient;
use chrono::Utc;
use common::ids::FeedEventId;
use common::tagged_url::{FeedUrl, HubUrl, compose};
use common::time::UtcInstant;
use host::feed::{FeedPath, affected_feed_urls};
use storage::{
    FeedCacheStorage, FeedEventRecord, FeedEventStorage, PostStorage, SiteConfigStorage,
};
use tokio::sync::Mutex;

use super::regenerate::{RegenerateError, regenerate_feed};

const BATCH_LIMIT: usize = 200;
const LEASE_TIMEOUT: Duration = Duration::from_mins(5);
const BACKOFFS_SECS: &[u64] = &[60, 300, 1800, 7200, 7200, 7200];
/// Max URLs per `enqueue_many` transaction: bounds the write-lock hold of a
/// go-live fan-out (a post-outage catch-up can be arbitrarily large) so
/// batching #766's churn away can't reintroduce the long-hold failure mode.
const ENQUEUE_CHUNK: usize = 256;

fn report_continuation(
    kind: host::error::ErrorKind,
    class: host::error::ErrorClass,
    context: &'static str,
    error: &(dyn std::error::Error + 'static),
) {
    host::error::report_swallowed(
        kind,
        class,
        context,
        host::error::SwallowedSource::Error(error),
    );
}

/// The background feed worker: the deps it needs to regenerate feeds and ping
/// the `WebSub` hub, declared explicitly as constructor parameters rather than
/// reached through a shared bundle (see [ADR-0016]).
///
/// [ADR-0016]: ../../../docs/adr/0016-dependency-injection-and-appstate.md
pub struct FeedWorker {
    site_config: Arc<dyn SiteConfigStorage>,
    posts: Arc<dyn PostStorage>,
    feed_cache: Arc<dyn FeedCacheStorage>,
    feed_events: Arc<dyn FeedEventStorage>,
    websub: Arc<dyn WebSubClient>,
    /// The instant of the previous [`go_live_pass`](Self::go_live_pass), or
    /// `None` before the first pass. `None` triggers the feed-relative startup
    /// catch-up; a `Some(last)` runs the steady-state `(last, now]` window.
    last_tick: Mutex<Option<UtcInstant>>,
}

impl FeedWorker {
    /// Builds a feed worker from exactly the storage handles and the `WebSub`
    /// publisher it uses.
    #[must_use]
    pub fn new(
        site_config: Arc<dyn SiteConfigStorage>,
        posts: Arc<dyn PostStorage>,
        feed_cache: Arc<dyn FeedCacheStorage>,
        feed_events: Arc<dyn FeedEventStorage>,
        websub: Arc<dyn WebSubClient>,
    ) -> Self {
        Self {
            site_config,
            posts,
            feed_cache,
            feed_events,
            websub,
            last_tick: Mutex::new(None),
        }
    }

    /// Borrows the site configuration store.
    #[must_use]
    pub fn site_config(&self) -> &dyn SiteConfigStorage {
        self.site_config.as_ref()
    }

    /// Borrows the post store.
    #[must_use]
    pub fn posts(&self) -> &dyn PostStorage {
        self.posts.as_ref()
    }

    /// Borrows the rendered-feed cache store.
    #[must_use]
    pub fn feed_cache(&self) -> &dyn FeedCacheStorage {
        self.feed_cache.as_ref()
    }

    /// Borrows the feed-regeneration event store.
    #[must_use]
    pub fn feed_events(&self) -> &dyn FeedEventStorage {
        self.feed_events.as_ref()
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
    /// Returns an error if a storage read or feed-event enqueue fails.
    pub async fn go_live_pass(&self, now: UtcInstant) -> anyhow::Result<()> {
        let mut last_tick = self.last_tick.lock().await;
        let urls = match *last_tick {
            None => self.posts().feed_urls_needing_catchup(now).await?,
            Some(last) => {
                let mut urls = Vec::new();
                for post in self.posts().list_posts_gone_live_between(last, now).await? {
                    urls.extend(affected_feed_urls(&post.username, &post.tag_slugs));
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
            self.feed_events().enqueue_many(chunk).await?;
        }
        *last_tick = Some(now);
        Ok(())
    }

    /// Processes a batch of pending feed events: regenerates feeds and pings the
    /// `WebSub` hub. Groups events by `feed_path` to avoid redundant regeneration.
    pub async fn tick(&self) {
        // Enqueue go-live regeneration first so the same tick drains what it
        // just enqueued. A failure here must not abort the independent queue
        // drain, but it remains operationally visible.
        if let Err(e) = self.go_live_pass(UtcInstant::now()).await {
            report_continuation(
                host::error::ErrorKind::Storage,
                host::error::ErrorClass::Transient,
                "server.feed.go_live_pass",
                e.as_ref(),
            );
        }

        let claimed = match self
            .feed_events()
            .claim_pending_batch(
                BATCH_LIMIT,
                chrono::Duration::from_std(LEASE_TIMEOUT).unwrap_or(chrono::Duration::seconds(300)),
            )
            .await
        {
            Ok(c) => c,
            Err(e) => {
                report_continuation(
                    host::error::ErrorKind::Storage,
                    host::error::ErrorClass::Transient,
                    "server.feed.claim_pending",
                    &e,
                );
                return;
            }
        };
        if claimed.is_empty() {
            return;
        }

        // Group by feed_path to avoid redundant regeneration
        let mut groups: HashMap<FeedPath, Vec<FeedEventRecord>> = HashMap::new();
        for rec in claimed {
            groups.entry(rec.feed_path.clone()).or_default().push(rec);
        }

        // Read hub URL and site identity once per tick. Their absence is normal;
        // a failed read degrades the tick in the same way but must be reported.
        let hub_url = match self.site_config().get_feeds_websub_hub_url().await {
            Ok(hub) => hub,
            Err(error) => {
                report_continuation(
                    host::error::ErrorKind::Storage,
                    host::error::ErrorClass::Transient,
                    "server.feed.websub_config_read",
                    &error,
                );
                None
            }
        };
        let identity = match self.site_config().get_identity().await {
            Ok(identity) => Some(identity),
            Err(error) => {
                report_continuation(
                    host::error::ErrorKind::Storage,
                    host::error::ErrorClass::Transient,
                    "server.feed.identity_read",
                    &error,
                );
                None
            }
        };

        for (feed_path, recs) in groups {
            self.process_feed_group(feed_path, recs, hub_url.as_ref(), identity.as_ref())
                .await;
        }
    }

    /// Regenerates one feed surface and reconciles the queued events for it: on
    /// success, marks them regenerated and pings the hub; on failure, schedules a
    /// backoff retry or marks the batch exhausted.
    async fn process_feed_group(
        &self,
        feed_path: FeedPath,
        recs: Vec<FeedEventRecord>,
        hub_url: Option<&HubUrl>,
        identity: Option<&common::site::SiteIdentity>,
    ) {
        let ids: Vec<FeedEventId> = recs.iter().map(|r| r.id).collect();
        let started = std::time::Instant::now();

        match regenerate_feed(
            self.site_config(),
            self.posts(),
            self.feed_cache(),
            &feed_path,
        )
        .await
        {
            Ok(row) => {
                host::metrics::feed_regeneration(host::metrics::RegenResult::Ok);
                host::metrics::feed_regen_duration_ms(
                    u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX),
                );
                if let Err(error) = self.feed_events().mark_regenerated(&ids).await {
                    report_continuation(
                        host::error::ErrorKind::Storage,
                        host::error::ErrorClass::Transient,
                        "server.feed.status_write.190",
                        &error,
                    );
                }
                let item_bytes = row.representation().body().len();
                let duration_ms = started.elapsed().as_millis();
                tracing::info!(
                    feed_path = %feed_path,
                    item_bytes = item_bytes,
                    duration_ms = duration_ms,
                    "feed.regen.completed"
                );

                let attempt = recs.iter().map(|r| r.attempts).max().unwrap_or(0) + 1;
                self.ping_websub(&feed_path, &ids, attempt, hub_url, identity)
                    .await;
            }
            Err(e) => {
                report_continuation(
                    host::error::ErrorKind::Internal,
                    host::error::ErrorClass::Bug,
                    "server.feed.regenerate",
                    &e,
                );
                self.on_regen_failure(&feed_path, &ids, &recs, &e).await;
            }
        }
    }

    /// Pings the `WebSub` hub for a freshly regenerated `feed_url`, marking the
    /// events pinged on success and scheduling a backoff retry (or marking them
    /// exhausted) on failure. With no hub configured the batch is treated as
    /// complete.
    async fn ping_websub(
        &self,
        feed_url: &FeedPath,
        ids: &[FeedEventId],
        attempt: i32,
        hub_url: Option<&HubUrl>,
        identity: Option<&common::site::SiteIdentity>,
    ) {
        if let Some(hub) = hub_url {
            // Feeds require site.base_url (#560), and regeneration fails closed without it,
            // so a reached ping always has a base. Guard defensively; the skip is
            // unreachable in practice.
            let Some(base) = identity.and_then(|i| i.base_url.as_ref()) else {
                // cov:ignore-start
                tracing::warn!("feed.websub.ping skipped: site.base_url is unset");
                return;
                // cov:ignore-stop
            };
            // `compose` joins the required base + the feed path into an absolute URL.
            let absolute: FeedUrl = compose(base, feed_url);
            tracing::info!(feed_url = %feed_url, hub = %hub, attempt, "feed.websub.ping.attempted");

            let result = self.websub.send_publish(hub, &absolute).await;
            match result {
                Ok(()) => {
                    host::metrics::websub_ping(host::metrics::PingOutcome::Success);
                    tracing::info!(feed_url = %feed_url, hub = %hub, attempt, "feed.websub.ping.succeeded");
                    if let Err(error) = self.feed_events().mark_pinged(ids).await {
                        report_continuation(
                            host::error::ErrorKind::Storage,
                            host::error::ErrorClass::Transient,
                            "server.feed.status_write.mark_pinged",
                            &error,
                        );
                    }
                }
                Err(e) => {
                    report_continuation(
                        host::error::ErrorKind::Internal,
                        host::error::ErrorClass::External,
                        "server.feed.websub_ping",
                        &e,
                    );
                    let attempt_usize = usize::try_from(attempt).unwrap_or(0);
                    let next_attempt_idx = attempt_usize.saturating_sub(1);
                    if next_attempt_idx >= BACKOFFS_SECS.len() {
                        host::metrics::websub_ping(host::metrics::PingOutcome::Exhausted);
                        if let Err(error) =
                            self.feed_events().mark_exhausted(ids, &e.to_string()).await
                        {
                            report_continuation(
                                host::error::ErrorKind::Storage,
                                host::error::ErrorClass::Transient,
                                "server.feed.status_write.249",
                                &error,
                            );
                        }
                    } else {
                        let delay = chrono::Duration::seconds(
                            i64::try_from(BACKOFFS_SECS[next_attempt_idx]).unwrap_or(60),
                        );
                        let next = UtcInstant::from(Utc::now() + delay);
                        host::metrics::websub_ping(host::metrics::PingOutcome::Failed);
                        if let Err(error) = self
                            .feed_events()
                            .mark_failed(ids, &e.to_string(), next)
                            .await
                        {
                            report_continuation(
                                host::error::ErrorKind::Storage,
                                host::error::ErrorClass::Transient,
                                "server.feed.status_write.257",
                                &error,
                            );
                        }
                    }
                }
            }
        } else {
            // No hub configured — treat as complete.
            host::metrics::websub_ping(host::metrics::PingOutcome::NoHub);
            if let Err(error) = self.feed_events().mark_pinged(ids).await {
                report_continuation(
                    host::error::ErrorKind::Storage,
                    host::error::ErrorClass::Transient,
                    "server.feed.status_write.mark_pinged",
                    &error,
                );
            }
        }
    }

    /// Reconciles the queued events after a failed regeneration: schedules a
    /// backoff retry, or marks the batch exhausted once the backoff schedule is
    /// used up.
    async fn on_regen_failure(
        &self,
        _feed_url: &FeedPath,
        ids: &[FeedEventId],
        recs: &[FeedEventRecord],
        e: &RegenerateError,
    ) {
        host::metrics::feed_regeneration(host::metrics::RegenResult::Error);
        let attempt = recs.iter().map(|r| r.attempts).max().unwrap_or(0) + 1;
        let attempt_usize = usize::try_from(attempt).unwrap_or(0);
        let next_attempt_idx = attempt_usize.saturating_sub(1);
        if next_attempt_idx >= BACKOFFS_SECS.len() {
            if let Err(error) = self.feed_events().mark_exhausted(ids, &e.to_string()).await {
                report_continuation(
                    host::error::ErrorKind::Storage,
                    host::error::ErrorClass::Transient,
                    "server.feed.status_write.288",
                    &error,
                );
            }
        } else {
            let next = UtcInstant::from(
                Utc::now()
                    + chrono::Duration::seconds(
                        i64::try_from(BACKOFFS_SECS[next_attempt_idx]).unwrap_or(60),
                    ),
            );
            if let Err(error) = self
                .feed_events()
                .mark_failed(ids, &e.to_string(), next)
                .await
            {
                report_continuation(
                    host::error::ErrorKind::Storage,
                    host::error::ErrorClass::Transient,
                    "server.feed.status_write.295",
                    &error,
                );
            }
        }
    }

    /// Starts the feed worker scheduler at the cadence selected by the
    /// composition root. Subsecond cadences use one scheduler activation to
    /// drive a Tokio interval because `tokio-cron-scheduler` stores repeated
    /// durations at whole-second precision. Returns the scheduler; the caller
    /// must keep it alive for the worker to run.
    ///
    /// # Errors
    ///
    /// Returns an error if the interval is zero or the scheduler fails to start.
    pub async fn start(
        self,
        interval: Duration,
    ) -> anyhow::Result<tokio_cron_scheduler::JobScheduler> {
        anyhow::ensure!(!interval.is_zero(), "feed worker interval must be non-zero");

        let worker = Arc::new(self);
        let scheduler = tokio_cron_scheduler::JobScheduler::new().await?;
        let job = if interval < Duration::from_secs(1) {
            // cov:ignore-start -- the closure body fires only when the scheduler
            // activates it; tick behavior is unit-tested through spawn_tick.
            tokio_cron_scheduler::Job::new_one_shot_async(Duration::ZERO, move |_uuid, _lock| {
                let worker = worker.clone();
                Box::pin(async move {
                    let mut ticker = tokio::time::interval(interval);
                    ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
                    loop {
                        ticker.tick().await;
                        spawn_tick(worker.clone()).await;
                    }
                })
            })?
        } else {
            tokio_cron_scheduler::Job::new_repeated_async(interval, move |_uuid, _lock| {
                spawn_tick(worker.clone())
            })?
        };
        // cov:ignore-stop
        scheduler.add(job).await?;
        scheduler.start().await?;
        Ok(scheduler)
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
    use common::site::SiteIdentity;
    use storage::{FeedEventError, FeedEventRecord, FeedEventStatus};

    fn event(id: i64, feed_url: &str, attempts: i32) -> FeedEventRecord {
        let now = UtcInstant::now();
        FeedEventRecord {
            id: FeedEventId::from(id),
            feed_path: feed_url.parse().expect("valid feed path in test"),
            status: FeedEventStatus::Claimed,
            attempts,
            last_error: None,
            next_attempt_at: now,
            claimed_at: Some(now),
            created_at: now,
            regenerated_at: None,
            pinged_at: None,
        }
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
            Err(crate::websub::WebSubError::Http(Box::new(
                std::io::Error::other("worker WebSub transport failure"),
            )))
        }
    }

    fn worker(
        site_config: storage::MockSiteConfigStorage,
        posts: storage::MockPostStorage,
        feed_cache: storage::MockFeedCacheStorage,
        feed_events: storage::MockFeedEventStorage,
    ) -> FeedWorker {
        FeedWorker::new(
            Arc::new(site_config),
            Arc::new(posts),
            Arc::new(feed_cache),
            Arc::new(feed_events),
            Arc::new(NoopWebSubClient),
        )
    }

    fn worker_with_websub(
        feed_events: storage::MockFeedEventStorage,
        websub: Arc<dyn WebSubClient>,
    ) -> FeedWorker {
        FeedWorker::new(
            Arc::new(storage::MockSiteConfigStorage::new()),
            Arc::new(storage::MockPostStorage::new()),
            Arc::new(storage::MockFeedCacheStorage::new()),
            Arc::new(feed_events),
            websub,
        )
    }

    fn test_identity() -> SiteIdentity {
        SiteIdentity {
            title: common::test_support::parse_site_title("Jaunder"),
            base_url: Some(common::test_support::parse_url("https://example.com/")),
        }
    }

    fn test_feeds_config() -> host::feed::FeedsConfig {
        host::feed::FeedsConfig {
            min_items: host::test_support::parse_feed_min_items("10"),
            min_days: host::test_support::parse_feed_min_days("30"),
            websub_hub_url: None,
        }
    }

    fn successful_tick_posts() -> storage::MockPostStorage {
        let mut posts = storage::MockPostStorage::new();
        posts
            .expect_feed_urls_needing_catchup()
            .times(1)
            .returning(|_| Ok(vec![]));
        posts
            .expect_list_published_in_window()
            .times(1)
            .returning(|_, _, _, _| Ok(vec![]));
        posts
    }

    fn successful_feed_cache() -> storage::MockFeedCacheStorage {
        let mut cache = storage::MockFeedCacheStorage::new();
        cache.expect_upsert().times(1).returning(|_| Ok(()));
        cache
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

    // guard:no-backend — mock stores isolate the config-read continuation.
    #[tokio::test]
    async fn continuation_reporting_tick_preserves_processing_after_websub_config_read_failure() {
        let mut site_config = storage::MockSiteConfigStorage::new();
        site_config
            .expect_get_feeds_websub_hub_url()
            .times(1)
            .returning(|| Err(sqlx::Error::PoolClosed));
        site_config
            .expect_get_identity()
            .times(2)
            .returning(|| Ok(test_identity()));
        site_config
            .expect_get_feeds_config()
            .times(1)
            .returning(|| Ok(test_feeds_config()));
        let mut events = storage::MockFeedEventStorage::new();
        events
            .expect_claim_pending_batch()
            .times(1)
            .returning(|_, _| Ok(vec![event(1, "/feed.rss", 0)]));
        events
            .expect_mark_regenerated()
            .times(1)
            .returning(|_| Ok(()));
        events.expect_mark_pinged().times(1).returning(|_| Ok(()));
        let worker = worker(
            site_config,
            successful_tick_posts(),
            successful_feed_cache(),
            events,
        );

        let (guard, output) = trace_capture();
        worker.tick().await;
        drop(guard);
        assert_context_once(&trace_text(&output), "server.feed.websub_config_read");
    }

    // guard:no-backend — mock stores isolate the identity-read continuation.
    #[tokio::test]
    async fn continuation_reporting_tick_preserves_processing_after_identity_read_failure() {
        let mut site_config = storage::MockSiteConfigStorage::new();
        site_config
            .expect_get_feeds_websub_hub_url()
            .times(1)
            .returning(|| Ok(None));
        let identity_reads = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        site_config
            .expect_get_identity()
            .times(2)
            .returning(move || {
                if identity_reads.fetch_add(1, std::sync::atomic::Ordering::Relaxed) == 0 {
                    Err(sqlx::Error::PoolClosed)
                } else {
                    Ok(test_identity())
                }
            });
        site_config
            .expect_get_feeds_config()
            .times(1)
            .returning(|| Ok(test_feeds_config()));
        let mut events = storage::MockFeedEventStorage::new();
        events
            .expect_claim_pending_batch()
            .times(1)
            .returning(|_, _| Ok(vec![event(1, "/feed.rss", 0)]));
        events
            .expect_mark_regenerated()
            .times(1)
            .returning(|_| Ok(()));
        events.expect_mark_pinged().times(1).returning(|_| Ok(()));
        let worker = worker(
            site_config,
            successful_tick_posts(),
            successful_feed_cache(),
            events,
        );

        let (guard, output) = trace_capture();
        worker.tick().await;
        drop(guard);
        assert_context_once(&trace_text(&output), "server.feed.identity_read");
    }

    // guard:no-backend — mock stores isolate the regenerated-status continuation.
    #[tokio::test]
    async fn continuation_reporting_successful_regeneration_survives_status_write_failure() {
        let mut site_config = storage::MockSiteConfigStorage::new();
        site_config
            .expect_get_feeds_config()
            .times(1)
            .returning(|| Ok(test_feeds_config()));
        site_config
            .expect_get_identity()
            .times(1)
            .returning(|| Ok(test_identity()));
        let mut posts = storage::MockPostStorage::new();
        posts
            .expect_list_published_in_window()
            .times(1)
            .returning(|_, _, _, _| Ok(vec![]));
        let mut events = storage::MockFeedEventStorage::new();
        events
            .expect_mark_regenerated()
            .times(1)
            .returning(|_| Err(FeedEventError::Db(sqlx::Error::PoolClosed)));
        events.expect_mark_pinged().times(1).returning(|_| Ok(()));
        let worker = worker(site_config, posts, successful_feed_cache(), events);
        let identity = test_identity();

        let (guard, output) = trace_capture();
        worker
            .process_feed_group(
                "/feed.rss".parse().expect("feed path"),
                vec![event(1, "/feed.rss", 0)],
                None,
                Some(&identity),
            )
            .await;
        drop(guard);
        assert_context_once(&trace_text(&output), "server.feed.status_write.190");
    }

    // guard:no-backend — mock status store and successful protocol client.
    #[tokio::test]
    async fn continuation_reporting_websub_success_survives_mark_pinged_failure() {
        let mut events = storage::MockFeedEventStorage::new();
        events
            .expect_mark_pinged()
            .times(1)
            .returning(|_| Err(FeedEventError::Db(sqlx::Error::PoolClosed)));
        let worker = worker_with_websub(events, Arc::new(NoopWebSubClient));
        let hub = "https://hub.example/".parse().expect("hub URL");
        let identity = test_identity();

        let (guard, output) = trace_capture();
        worker
            .ping_websub(
                &"/feed.rss".parse().expect("feed path"),
                &[FeedEventId::from(1)],
                1,
                Some(&hub),
                Some(&identity),
            )
            .await;
        drop(guard);
        assert_context_once(&trace_text(&output), "server.feed.status_write.mark_pinged");
    }

    // guard:no-backend — mock status store and failing protocol client.
    #[tokio::test]
    async fn continuation_reporting_websub_exhaustion_survives_status_write_failure() {
        let mut events = storage::MockFeedEventStorage::new();
        events
            .expect_mark_exhausted()
            .times(1)
            .returning(|_, _| Err(FeedEventError::Db(sqlx::Error::PoolClosed)));
        let worker = worker_with_websub(events, Arc::new(FailingWebSubClient));
        let hub = "https://hub.example/".parse().expect("hub URL");
        let identity = test_identity();

        let (guard, output) = trace_capture();
        worker
            .ping_websub(
                &"/feed.rss".parse().expect("feed path"),
                &[FeedEventId::from(1)],
                i32::try_from(BACKOFFS_SECS.len() + 1).expect("small backoff table"),
                Some(&hub),
                Some(&identity),
            )
            .await;
        drop(guard);
        let trace = trace_text(&output);
        assert_context_once(&trace, "server.feed.websub_ping");
        assert_context_once(&trace, "server.feed.status_write.249");
    }

    // guard:no-backend — mock status store and failing protocol client.
    #[tokio::test]
    async fn continuation_reporting_websub_retry_survives_status_write_failure() {
        let mut events = storage::MockFeedEventStorage::new();
        events
            .expect_mark_failed()
            .times(1)
            .returning(|_, _, _| Err(FeedEventError::Db(sqlx::Error::PoolClosed)));
        let worker = worker_with_websub(events, Arc::new(FailingWebSubClient));
        let hub = "https://hub.example/".parse().expect("hub URL");
        let identity = test_identity();

        let (guard, output) = trace_capture();
        worker
            .ping_websub(
                &"/feed.rss".parse().expect("feed path"),
                &[FeedEventId::from(1)],
                1,
                Some(&hub),
                Some(&identity),
            )
            .await;
        drop(guard);
        let trace = trace_text(&output);
        assert_context_once(&trace, "server.feed.websub_ping");
        assert_context_once(&trace, "server.feed.status_write.257");
    }

    // guard:no-backend — mock status store isolates regeneration retry.
    #[tokio::test]
    async fn continuation_reporting_regeneration_retry_survives_status_write_failure() {
        let mut events = storage::MockFeedEventStorage::new();
        events
            .expect_mark_failed()
            .times(1)
            .returning(|_, _, _| Err(FeedEventError::Db(sqlx::Error::PoolClosed)));
        let worker = worker_with_websub(events, Arc::new(NoopWebSubClient));
        let record = event(1, "/feed.rss", 0);
        let error =
            RegenerateError::Storage(Box::new(std::io::Error::other("regeneration failed")));

        let (guard, output) = trace_capture();
        worker
            .on_regen_failure(
                &record.feed_path,
                &[record.id],
                std::slice::from_ref(&record),
                &error,
            )
            .await;
        drop(guard);
        assert_context_once(&trace_text(&output), "server.feed.status_write.295");
    }

    // guard:no-backend — mock store
    #[tokio::test]
    async fn tick_reports_and_returns_when_claim_fails() {
        let mut posts = storage::MockPostStorage::new();
        posts
            .expect_feed_urls_needing_catchup()
            .times(0..)
            .returning(|_| Ok(vec![]));
        let mut events = storage::MockFeedEventStorage::new();
        events
            .expect_claim_pending_batch()
            .times(1)
            .returning(|_, _| Err(FeedEventError::Db(sqlx::Error::PoolClosed)));
        // No mark_* expectation is set: any call after the claim error would
        // panic as an unexpected call, proving the tick returned early.
        let w = worker(
            storage::MockSiteConfigStorage::new(),
            posts,
            storage::MockFeedCacheStorage::new(),
            events,
        );
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
        let mut events = storage::MockFeedEventStorage::new();
        events
            .expect_claim_pending_batch()
            .times(1)
            .returning(|_, _| Ok(vec![]));
        let w = worker(
            storage::MockSiteConfigStorage::new(),
            posts,
            storage::MockFeedCacheStorage::new(),
            events,
        );
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
            .returning(|_| Err(sqlx::Error::PoolClosed));
        let mut events = storage::MockFeedEventStorage::new();
        events
            .expect_claim_pending_batch()
            .times(1)
            .returning(|_, _| Ok(vec![]));
        let w = worker(
            storage::MockSiteConfigStorage::new(),
            posts,
            storage::MockFeedCacheStorage::new(),
            events,
        );
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
    async fn websub_transport_failure_reaches_retry_boundary_and_reports_once() {
        let mut events = storage::MockFeedEventStorage::new();
        events
            .expect_mark_failed()
            .times(1)
            .returning(|_, error, _| {
                assert_eq!(error, "WebSub transport failed");
                Ok(())
            });
        let worker = worker_with_websub(events, Arc::new(FailingWebSubClient));
        let hub = "https://hub.example/".parse().expect("hub URL");
        let identity = SiteIdentity {
            title: common::test_support::parse_site_title("Jaunder"),
            base_url: Some(common::test_support::parse_url("https://example.com/")),
        };
        let (guard, output) = trace_capture();
        worker
            .ping_websub(
                &"/feed.rss".parse().expect("feed path"),
                &[FeedEventId::from(1)],
                1,
                Some(&hub),
                Some(&identity),
            )
            .await;
        drop(guard);
        let trace = trace_text(&output);
        assert_eq!(
            trace
                .matches(r#""error.context":"server.feed.websub_ping""#)
                .count(),
            1,
            "trace: {trace}"
        );
        assert!(
            trace.contains("worker WebSub transport failure"),
            "typed source chain was lost: {trace}"
        );
    }

    // guard:no-backend — mock status store.
    #[tokio::test]
    async fn completed_ping_keeps_success_primary_and_reports_status_failure_once() {
        let mut events = storage::MockFeedEventStorage::new();
        events
            .expect_mark_pinged()
            .times(1)
            .returning(|_| Err(FeedEventError::Db(sqlx::Error::PoolClosed)));
        let worker = worker_with_websub(events, Arc::new(NoopWebSubClient));
        let (guard, output) = trace_capture();
        worker
            .ping_websub(
                &"/feed.rss".parse().expect("feed path"),
                &[FeedEventId::from(1)],
                1,
                None,
                None,
            )
            .await;
        drop(guard);
        let trace = trace_text(&output);
        assert_eq!(
            trace
                .matches(r#""error.context":"server.feed.status_write.mark_pinged""#)
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
        let mut events = storage::MockFeedEventStorage::new();
        // The regression gate for #766: the whole catch-up fans out as ONE
        // batched write, and the per-row API is never used.
        events
            .expect_enqueue_many()
            .times(1)
            .withf(|paths| paths.len() == 3)
            .returning(|_| Ok(()));
        events.expect_enqueue().times(0);
        let w = worker(
            storage::MockSiteConfigStorage::new(),
            posts,
            storage::MockFeedCacheStorage::new(),
            events,
        );
        w.go_live_pass(UtcInstant::now())
            .await
            .expect("catch-up pass");
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
            .withf(|paths| paths.len() == 9)
            .returning(|_| Ok(()));
        events.expect_enqueue().times(0);
        let w = worker(
            storage::MockSiteConfigStorage::new(),
            posts,
            storage::MockFeedCacheStorage::new(),
            events,
        );
        w.go_live_pass(UtcInstant::now())
            .await
            .expect("priming pass");
        w.go_live_pass(UtcInstant::now())
            .await
            .expect("windowed pass");
    }

    // guard:no-backend — mock store
    #[tokio::test]
    async fn tick_regenerates_and_completes_without_hub() {
        let mut site_config = storage::MockSiteConfigStorage::new();
        site_config
            .expect_get_feeds_websub_hub_url()
            .times(0..)
            .returning(|| Ok(None));
        site_config.expect_get_identity().times(0..).returning(|| {
            Ok(SiteIdentity {
                title: common::test_support::parse_site_title("Jaunder"),
                base_url: Some(common::test_support::parse_url("https://example.com/")),
            })
        });
        site_config
            .expect_get_feeds_config()
            .times(0..)
            .returning(|| {
                Ok(host::feed::FeedsConfig {
                    min_items: host::test_support::parse_feed_min_items("10"),
                    min_days: host::test_support::parse_feed_min_days("30"),
                    websub_hub_url: None,
                })
            });
        let mut posts = storage::MockPostStorage::new();
        posts
            .expect_feed_urls_needing_catchup()
            .times(0..)
            .returning(|_| Ok(vec![]));
        posts
            .expect_list_published_in_window()
            .times(0..)
            .returning(|_, _, _, _| Ok(vec![]));
        let mut cache = storage::MockFeedCacheStorage::new();
        cache.expect_upsert().times(0..).returning(|_| Ok(()));
        let mut events = storage::MockFeedEventStorage::new();
        events
            .expect_claim_pending_batch()
            .times(1)
            .returning(|_, _| Ok(vec![event(1, "/feed.rss", 0)]));
        events
            .expect_mark_regenerated()
            .times(1)
            .returning(|_| Ok(()));
        // No hub configured -> the tick treats the event as complete (mark_pinged).
        events.expect_mark_pinged().times(1).returning(|_| Ok(()));
        let w = worker(site_config, posts, cache, events);
        w.tick().await;
    }

    // guard:no-backend — mock store
    #[tokio::test]
    async fn continuation_reporting_tick_regeneration_exhaustion_survives_status_write_failure() {
        // A FeedPath is always parseable, so regen can only fail on a storage
        // error: make the first read inside regenerate_feed fail. The record's
        // high attempt count pushes the next attempt past the backoff table, so
        // the tick marks the events exhausted (terminal failure).
        let mut site_config = storage::MockSiteConfigStorage::new();
        site_config
            .expect_get_feeds_config()
            .times(1)
            .returning(|| Err(sqlx::Error::PoolClosed));
        site_config
            .expect_get_feeds_websub_hub_url()
            .times(1)
            .returning(|| Ok(None));
        site_config
            .expect_get_identity()
            .times(1)
            .returning(|| Ok(test_identity()));
        let mut posts = storage::MockPostStorage::new();
        posts
            .expect_feed_urls_needing_catchup()
            .times(1)
            .returning(|_| Ok(vec![]));
        let mut events = storage::MockFeedEventStorage::new();
        events
            .expect_claim_pending_batch()
            .times(1)
            .returning(|_, _| Ok(vec![event(1, "/feed.rss", 10)]));
        events
            .expect_mark_exhausted()
            .times(1)
            .returning(|_, _| Err(FeedEventError::Db(sqlx::Error::PoolClosed)));
        let w = worker(
            site_config,
            posts,
            storage::MockFeedCacheStorage::new(),
            events,
        );
        let (guard, output) = trace_capture();
        w.tick().await;
        drop(guard);
        let trace = trace_text(&output);
        assert_context_once(&trace, "server.feed.regenerate");
        assert_context_once(&trace, "server.feed.status_write.288");
        assert_eq!(
            trace.matches(r#""error.disposition":"swallowed""#).count(),
            2,
            "trace: {trace}"
        );
    }

    // guard:no-backend — mock store
    #[tokio::test]
    async fn tick_reschedules_on_regen_failure_within_backoff() {
        // A bad-URL trigger is unrepresentable (a `FeedPath` is always valid), so
        // the failure is a valid path plus a forced storage error inside
        // regenerate_feed. attempts = 0 keeps the next attempt inside the backoff
        // table, so the batch is rescheduled (mark_failed), the cache is never
        // written, and no hub ping is attempted.
        let mut site_config = storage::MockSiteConfigStorage::new();
        site_config
            .expect_get_feeds_config()
            .times(0..)
            .returning(|| Err(sqlx::Error::PoolClosed));
        site_config
            .expect_get_feeds_websub_hub_url()
            .times(0..)
            .returning(|| Ok(None));
        site_config.expect_get_identity().times(0..).returning(|| {
            Ok(SiteIdentity {
                title: common::test_support::parse_site_title("Jaunder"),
                base_url: Some(common::test_support::parse_url("https://example.com/")),
            })
        });
        let mut posts = storage::MockPostStorage::new();
        posts
            .expect_feed_urls_needing_catchup()
            .times(0..)
            .returning(|_| Ok(vec![]));
        let mut cache = storage::MockFeedCacheStorage::new();
        cache.expect_upsert().times(0); // no cache row on regen failure
        let mut events = storage::MockFeedEventStorage::new();
        events
            .expect_claim_pending_batch()
            .times(1)
            .returning(|_, _| Ok(vec![event(1, "/feed.rss", 0)]));
        events
            .expect_mark_failed()
            .times(1)
            .returning(|_, _, _| Ok(()));
        let w = worker(site_config, posts, cache, events);
        w.tick().await;
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
            .returning(|_, _| Ok(vec![]));
        let w = worker(
            storage::MockSiteConfigStorage::new(),
            posts,
            storage::MockFeedCacheStorage::new(),
            events,
        );
        spawn_tick(Arc::new(w)).await;
    }
}
