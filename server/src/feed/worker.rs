use std::collections::HashMap;
use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;
use std::time::Duration;

use crate::websub::WebSubClient;
use chrono::{DateTime, Utc};
use common::feed::affected_feed_urls;
use storage::{
    FeedCacheStorage, FeedEventRecord, FeedEventStorage, PostStorage, SiteConfigStorage,
};
use tokio::sync::Mutex;

use super::regenerate::{regenerate_feed, RegenerateError};

const BATCH_LIMIT: usize = 200;
const LEASE_TIMEOUT: Duration = Duration::from_mins(5);
const BACKOFFS_SECS: &[u64] = &[60, 300, 1800, 7200, 7200, 7200];

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
    last_tick: Mutex<Option<DateTime<Utc>>>,
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
    /// Both branches seed `last_tick = now`.
    ///
    /// # Errors
    ///
    /// Returns an error if a storage read or feed-event enqueue fails.
    pub async fn go_live_pass(&self, now: DateTime<Utc>) -> anyhow::Result<()> {
        let mut last_tick = self.last_tick.lock().await;
        match *last_tick {
            None => {
                for url in self.posts.feed_urls_needing_catchup(now).await? {
                    self.feed_events.enqueue(&url).await?;
                }
            }
            Some(last) => {
                for post in self.posts.list_posts_gone_live_between(last, now).await? {
                    for url in affected_feed_urls(&post.username, &post.tag_slugs) {
                        self.feed_events.enqueue(&url).await?;
                    }
                }
            }
        }
        *last_tick = Some(now);
        Ok(())
    }

    /// Processes a batch of pending feed events: regenerates feeds and pings the
    /// `WebSub` hub. Groups events by `feed_url` to avoid redundant regeneration.
    pub async fn tick(&self) {
        // Enqueue go-live regeneration first so the same tick drains what it
        // just enqueued. A failure here must not abort the tick — the queue
        // drain below is independent — so it is logged, not propagated.
        if let Err(e) = self.go_live_pass(Utc::now()).await {
            tracing::error!(error = %e, "feed worker go-live pass failed");
        }

        let claimed = match self
            .feed_events
            .claim_pending_batch(
                BATCH_LIMIT,
                chrono::Duration::from_std(LEASE_TIMEOUT).unwrap_or(chrono::Duration::seconds(300)),
            )
            .await
        {
            Ok(c) => c,
            Err(e) => {
                tracing::error!(error = %e, "feed worker claim failed");
                return;
            }
        };
        if claimed.is_empty() {
            return;
        }

        // Group by feed_url to avoid redundant regeneration
        let mut groups: HashMap<String, Vec<FeedEventRecord>> = HashMap::new();
        for rec in claimed {
            groups.entry(rec.feed_url.clone()).or_default().push(rec);
        }

        // Read hub URL and site identity once per tick
        let hub_url = self
            .site_config
            .get_feeds_websub_hub_url()
            .await
            .ok()
            .flatten();
        let identity = self.site_config.get_identity().await.ok();

        for (feed_url, recs) in groups {
            self.process_feed_group(feed_url, recs, hub_url.as_deref(), identity.as_ref())
                .await;
        }
    }

    /// Regenerates one feed surface and reconciles the queued events for it: on
    /// success, marks them regenerated and pings the hub; on failure, schedules a
    /// backoff retry or marks the batch exhausted.
    async fn process_feed_group(
        &self,
        feed_url: String,
        recs: Vec<FeedEventRecord>,
        hub_url: Option<&str>,
        identity: Option<&common::site::SiteIdentity>,
    ) {
        let ids: Vec<i64> = recs.iter().map(|r| r.id).collect();
        let started = std::time::Instant::now();

        match regenerate_feed(
            self.site_config.as_ref(),
            self.posts.as_ref(),
            self.feed_cache.as_ref(),
            &feed_url,
        )
        .await
        {
            Ok(row) => {
                host::metrics::feed_regeneration(host::metrics::RegenResult::Ok);
                host::metrics::feed_regen_duration_ms(
                    u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX),
                );
                let _ = self.feed_events.mark_regenerated(&ids).await;
                let item_bytes = row.body.len();
                let duration_ms = started.elapsed().as_millis();
                tracing::info!(
                    feed_url,
                    item_bytes = item_bytes,
                    duration_ms = duration_ms,
                    "feed.regen.completed"
                );

                let attempt = recs.iter().map(|r| r.attempts).max().unwrap_or(0) + 1;
                self.ping_websub(&feed_url, &ids, attempt, hub_url, identity)
                    .await;
            }
            Err(e) => {
                self.on_regen_failure(&feed_url, &ids, &recs, &e).await;
            }
        }
    }

    /// Pings the `WebSub` hub for a freshly regenerated `feed_url`, marking the
    /// events pinged on success and scheduling a backoff retry (or marking them
    /// exhausted) on failure. With no hub configured the batch is treated as
    /// complete.
    async fn ping_websub(
        &self,
        feed_url: &str,
        ids: &[i64],
        attempt: i32,
        hub_url: Option<&str>,
        identity: Option<&common::site::SiteIdentity>,
    ) {
        if let Some(hub) = hub_url {
            let base = identity
                .and_then(|i| i.base_url.as_deref())
                .unwrap_or("")
                .trim_end_matches('/');
            let absolute = format!("{base}{feed_url}");
            tracing::info!(feed_url, hub, attempt, "feed.websub.ping.attempted");

            let result = self.websub.send_publish(hub, &absolute).await;
            match result {
                Ok(()) => {
                    host::metrics::websub_ping(host::metrics::PingOutcome::Success);
                    tracing::info!(feed_url, hub, attempt, "feed.websub.ping.succeeded");
                    let _ = self.feed_events.mark_pinged(ids).await;
                }
                Err(e) => {
                    let attempt_usize = usize::try_from(attempt).unwrap_or(0);
                    let next_attempt_idx = attempt_usize.saturating_sub(1);
                    if next_attempt_idx >= BACKOFFS_SECS.len() {
                        host::metrics::websub_ping(host::metrics::PingOutcome::Exhausted);
                        tracing::warn!(feed_url, hub, "feed.websub.ping.exhausted");
                        let _ = self.feed_events.mark_exhausted(ids, &e.to_string()).await;
                    } else {
                        let delay = chrono::Duration::seconds(
                            i64::try_from(BACKOFFS_SECS[next_attempt_idx]).unwrap_or(60),
                        );
                        let next = Utc::now() + delay;
                        host::metrics::websub_ping(host::metrics::PingOutcome::Failed);
                        tracing::warn!(feed_url, hub, attempt, error = %e, "feed.websub.ping.failed");
                        let _ = self
                            .feed_events
                            .mark_failed(ids, &e.to_string(), next)
                            .await;
                    }
                }
            }
        } else {
            // No hub configured — treat as complete.
            host::metrics::websub_ping(host::metrics::PingOutcome::NoHub);
            let _ = self.feed_events.mark_pinged(ids).await;
        }
    }

    /// Reconciles the queued events after a failed regeneration: schedules a
    /// backoff retry, or marks the batch exhausted once the backoff schedule is
    /// used up.
    async fn on_regen_failure(
        &self,
        feed_url: &str,
        ids: &[i64],
        recs: &[FeedEventRecord],
        e: &RegenerateError,
    ) {
        host::metrics::feed_regeneration(host::metrics::RegenResult::Error);
        tracing::error!(error = %e, feed_url, "feed.regen.failed");
        let attempt = recs.iter().map(|r| r.attempts).max().unwrap_or(0) + 1;
        let attempt_usize = usize::try_from(attempt).unwrap_or(0);
        let next_attempt_idx = attempt_usize.saturating_sub(1);
        if next_attempt_idx >= BACKOFFS_SECS.len() {
            // cov:ignore-start
            let _ = self.feed_events.mark_exhausted(ids, &e.to_string()).await;
            // cov:ignore-stop
        } else {
            let next = Utc::now()
                + chrono::Duration::seconds(
                    i64::try_from(BACKOFFS_SECS[next_attempt_idx]).unwrap_or(60),
                );
            let _ = self
                .feed_events
                .mark_failed(ids, &e.to_string(), next)
                .await;
        }
    }

    /// Starts the feed worker scheduler, which runs [`tick`](Self::tick)
    /// periodically. Returns the scheduler; the caller must keep it alive for
    /// the worker to run.
    ///
    /// # Errors
    ///
    /// Returns an error if the scheduler fails to start.
    pub async fn start(self) -> anyhow::Result<tokio_cron_scheduler::JobScheduler> {
        let worker = Arc::new(self);
        let scheduler = tokio_cron_scheduler::JobScheduler::new().await?;
        let job = tokio_cron_scheduler::Job::new_repeated_async(
            Duration::from_secs(10),
            // cov:ignore-start -- the closure body fires only when the 10s cron
            // timer elapses; the work it does (spawn_tick → tick) is unit-tested
            // directly, so only this scheduler-registration wrapper is uncovered.
            move |_uuid, _lock| spawn_tick(worker.clone()),
        )?;
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
        let now = Utc::now();
        FeedEventRecord {
            id,
            feed_url: feed_url.to_owned(),
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

    // guard:no-backend — mock store
    #[tokio::test]
    async fn tick_logs_and_returns_when_claim_fails() {
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
        w.tick().await;
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
    async fn tick_logs_when_go_live_pass_fails_but_still_drains() {
        let mut posts = storage::MockPostStorage::new();
        // Go-live pass fails; the error is logged, not propagated, and the tick
        // continues to the (empty) queue drain.
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
        w.tick().await;
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
                title: "Jaunder".to_owned(),
                base_url: None,
            })
        });
        site_config
            .expect_get_feeds_config()
            .times(0..)
            .returning(|| {
                Ok(common::feed::FeedsConfig {
                    min_items: 10,
                    min_days: 30,
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
    async fn tick_marks_exhausted_when_regen_fails_past_backoff_table() {
        let mut site_config = storage::MockSiteConfigStorage::new();
        site_config
            .expect_get_feeds_websub_hub_url()
            .times(0..)
            .returning(|| Ok(None));
        site_config.expect_get_identity().times(0..).returning(|| {
            Ok(SiteIdentity {
                title: "Jaunder".to_owned(),
                base_url: None,
            })
        });
        let mut posts = storage::MockPostStorage::new();
        posts
            .expect_feed_urls_needing_catchup()
            .times(0..)
            .returning(|_| Ok(vec![]));
        let mut events = storage::MockFeedEventStorage::new();
        // An unparseable feed_url makes regenerate_feed fail immediately; the
        // record's high attempt count pushes the next attempt past the backoff
        // table, so the tick marks the events exhausted (terminal failure).
        events
            .expect_claim_pending_batch()
            .times(1)
            .returning(|_, _| Ok(vec![event(1, "not-a-feed-url", 10)]));
        events
            .expect_mark_exhausted()
            .times(1)
            .returning(|_, _| Ok(()));
        let w = worker(
            site_config,
            posts,
            storage::MockFeedCacheStorage::new(),
            events,
        );
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
