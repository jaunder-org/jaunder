use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

use crate::websub::WebSubClient;
use chrono::Utc;
use storage::{
    FeedCacheStorage, FeedEventRecord, FeedEventStorage, PostStorage, SiteConfigStorage,
};

use super::regenerate::regenerate_feed;

const BATCH_LIMIT: usize = 200;
#[allow(clippy::duration_suboptimal_units)]
const LEASE_TIMEOUT: Duration = Duration::from_secs(300); // 5 minutes
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
        }
    }

    /// Processes a batch of pending feed events: regenerates feeds and pings the
    /// `WebSub` hub. Groups events by `feed_url` to avoid redundant regeneration.
    #[allow(clippy::too_many_lines)]
    pub async fn tick(&self) {
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
                    common::metrics::feed_regeneration(common::metrics::RegenResult::Ok);
                    common::metrics::feed_regen_duration_ms(
                        u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX),
                    );
                    let _ = self.feed_events.mark_regenerated(&ids).await;
                    tracing::info!(
                        feed_url,
                        item_bytes = row.body.len(),
                        duration_ms = started.elapsed().as_millis(),
                        "feed.regen.completed"
                    );

                    let attempt = recs.iter().map(|r| r.attempts).max().unwrap_or(0) + 1;
                    if let Some(hub) = &hub_url {
                        let base = identity
                            .as_ref()
                            .and_then(|i| i.base_url.as_deref())
                            .unwrap_or("")
                            .trim_end_matches('/');
                        let absolute = format!("{base}{feed_url}");
                        tracing::info!(feed_url, hub, attempt, "feed.websub.ping.attempted");

                        let result = self.websub.send_publish(hub, &absolute).await;
                        match result {
                            Ok(()) => {
                                common::metrics::websub_ping(common::metrics::PingOutcome::Success);
                                tracing::info!(
                                    feed_url,
                                    hub,
                                    attempt,
                                    "feed.websub.ping.succeeded"
                                );
                                let _ = self.feed_events.mark_pinged(&ids).await;
                            }
                            Err(e) => {
                                let attempt_usize = usize::try_from(attempt).unwrap_or(0);
                                let next_attempt_idx = attempt_usize.saturating_sub(1);
                                if next_attempt_idx >= BACKOFFS_SECS.len() {
                                    common::metrics::websub_ping(
                                        common::metrics::PingOutcome::Exhausted,
                                    );
                                    tracing::warn!(feed_url, hub, "feed.websub.ping.exhausted");
                                    let _ =
                                        self.feed_events.mark_exhausted(&ids, &e.to_string()).await;
                                } else {
                                    let delay = chrono::Duration::seconds(
                                        i64::try_from(BACKOFFS_SECS[next_attempt_idx])
                                            .unwrap_or(60),
                                    );
                                    let next = Utc::now() + delay;
                                    common::metrics::websub_ping(
                                        common::metrics::PingOutcome::Failed,
                                    );
                                    tracing::warn!(feed_url, hub, attempt, error = %e, "feed.websub.ping.failed");
                                    let _ = self
                                        .feed_events
                                        .mark_failed(&ids, &e.to_string(), next)
                                        .await;
                                }
                            }
                        }
                    } else {
                        // No hub configured — treat as complete.
                        common::metrics::websub_ping(common::metrics::PingOutcome::NoHub);
                        let _ = self.feed_events.mark_pinged(&ids).await;
                    }
                }
                Err(e) => {
                    common::metrics::feed_regeneration(common::metrics::RegenResult::Error);
                    tracing::error!(error = %e, feed_url, "feed.regen.failed");
                    let attempt = recs.iter().map(|r| r.attempts).max().unwrap_or(0) + 1;
                    let attempt_usize = usize::try_from(attempt).unwrap_or(0);
                    let next_attempt_idx = attempt_usize.saturating_sub(1);
                    if next_attempt_idx >= BACKOFFS_SECS.len() {
                        let _ = self.feed_events.mark_exhausted(&ids, &e.to_string()).await;
                    } else {
                        let next = Utc::now()
                            + chrono::Duration::seconds(
                                i64::try_from(BACKOFFS_SECS[next_attempt_idx]).unwrap_or(60),
                            );
                        let _ = self
                            .feed_events
                            .mark_failed(&ids, &e.to_string(), next)
                            .await;
                    }
                }
            }
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
            move |_uuid, _lock| {
                let worker = worker.clone();
                Box::pin(async move {
                    worker.tick().await;
                })
            },
        )?;
        scheduler.add(job).await?;
        scheduler.start().await?;
        Ok(scheduler)
    }
}
