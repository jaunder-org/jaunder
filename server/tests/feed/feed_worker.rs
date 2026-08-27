use std::sync::Arc;

use crate::helpers::{CapturingWebSubClient, setup_with_base_url};
use chrono::Utc;
use common::{feed::FeedFormat, ids::FeedEventId, test_support::parse_etag, time::UtcInstant};
use host::feed::{FeedPath, SyndicationFeedRepresentation};
use jaunder::feed::worker::FeedWorker;
use storage::FeedCacheRow;
use storage::test_support::{Backend, SeedRawPost, SeedUser, TestEnv, backends, fp};

use rstest::*;
use rstest_reuse::*;

/// Test double whose `WebSub` client always reports the hub refused the ping,
/// so the worker exercises its ping-failure backoff path.
struct FailingWebSubClient;

#[async_trait::async_trait]
impl jaunder::websub::WebSubClient for FailingWebSubClient {
    async fn send_publish(
        &self,
        _hub_url: &common::tagged_url::HubUrl,
        _feed_url: &common::tagged_url::FeedUrl,
    ) -> Result<(), jaunder::websub::WebSubError> {
        Err(jaunder::websub::WebSubError::HubRefused { status: 503 })
    }
}

/// Builds a [`FeedWorker`] from a test `AppState`'s handles plus an injected
/// `WebSub` client (the worker no longer reaches into a shared bundle).
fn make_worker(
    state: &std::sync::Arc<storage::AppState>,
    websub: std::sync::Arc<dyn jaunder::websub::WebSubClient>,
) -> FeedWorker {
    FeedWorker::new(
        state.site_config.clone(),
        state.posts.clone(),
        state.feed_cache.clone(),
        state.feed_events.clone(),
        websub,
    )
}

#[apply(backends)]
#[tokio::test]
async fn worker_regenerates_claimed_event_and_marks_done_when_no_hub(#[case] backend: Backend) {
    let TestEnv { state, base: _base } = setup_with_base_url(backend).await;
    let capture = Arc::new(CapturingWebSubClient::default());

    let user = SeedUser::new().seed(&state).await;

    let post = SeedRawPost::new(user.user_id).seed(&state).await;

    let feed_path = fp(&format!("/~{}/feed.rss", user.username));
    state
        .feed_events
        .enqueue(&feed_path)
        .await
        .expect("enqueue feed event");

    make_worker(&state, capture.clone()).tick().await;

    let cache_row = state
        .feed_cache
        .get(&feed_path)
        .await
        .expect("get cache")
        .expect("cache row should exist");
    assert!(
        cache_row
            .representation()
            .body()
            .contains(post.title.as_ref())
    );

    let pending = state
        .feed_events
        .claim_pending_batch(10, chrono::Duration::minutes(5))
        .await
        .expect("claim pending");
    assert!(pending.is_empty(), "event should be done, not pending");
}

#[apply(backends)]
#[tokio::test]
async fn worker_pings_hub_when_configured(#[case] backend: Backend) {
    let TestEnv { state, base: _base } = setup_with_base_url(backend).await;
    let capture = Arc::new(CapturingWebSubClient::default());

    let user = SeedUser::new().seed(&state).await;

    SeedRawPost::new(user.user_id).seed(&state).await;

    state
        .site_config
        .set(
            storage::SiteConfigKey::FeedsWebsubHubUrl,
            "https://hub.example.com/",
        )
        .await
        .expect("set hub url");

    let feed_path = fp(&format!("/~{}/feed.rss", user.username));
    state
        .feed_events
        .enqueue(&feed_path)
        .await
        .expect("enqueue feed event");

    make_worker(&state, capture.clone()).tick().await;

    let pings = capture.pings();
    assert_eq!(pings.len(), 1, "should have exactly one ping");
    assert_eq!(pings[0].hub_url, "https://hub.example.com/");
    assert!(
        pings[0]
            .feed_url
            .ends_with(&format!("/~{}/feed.rss", user.username)),
        "feed url should end with /~{}/feed.rss, got: {}",
        user.username,
        pings[0].feed_url
    );
}

#[apply(backends)]
#[tokio::test]
async fn worker_groups_duplicate_events_into_single_regen(#[case] backend: Backend) {
    let TestEnv { state, base: _base } = setup_with_base_url(backend).await;
    let capture = Arc::new(CapturingWebSubClient::default());

    let user = SeedUser::new().seed(&state).await;

    SeedRawPost::new(user.user_id).seed(&state).await;

    state
        .site_config
        .set(
            storage::SiteConfigKey::FeedsWebsubHubUrl,
            "https://hub.example.com/",
        )
        .await
        .expect("set hub url");

    let feed_path = fp(&format!("/~{}/feed.rss", user.username));
    for _ in 0..5 {
        state
            .feed_events
            .enqueue(&feed_path)
            .await
            .expect("enqueue feed event");
    }

    make_worker(&state, capture.clone()).tick().await;

    // Verify only 1 ping was sent (grouping collapses duplicates)
    let pings = capture.pings();
    assert_eq!(
        pings.len(),
        1,
        "should have exactly one ping (duplicates grouped)"
    );
    assert_eq!(pings[0].hub_url, "https://hub.example.com/");
    assert!(
        pings[0]
            .feed_url
            .ends_with(&format!("/~{}/feed.rss", user.username))
    );
}

// Regen-failure backoff is covered by a mock-based worker unit test
// (`worker::tests::tick_reschedules_on_regen_failure_within_backoff`): a
// `FeedPath` cannot hold an unparseable value, and a real backend cannot
// cheaply inject the only representable failure (a storage error). The
// real-backend `mark_failed` scheduling SQL stays covered by the dual-backend
// `feed_events` storage test.

#[apply(backends)]
#[tokio::test]
async fn worker_applies_backoff_on_ping_failure(#[case] backend: Backend) {
    // WebSub ping-failure backoff is backend-agnostic: the shared setup runs it
    // on both backends so neither is left uncovered.
    let TestEnv { state, base: _base } = setup_with_base_url(backend).await;

    let user = SeedUser::new().seed(&state).await;

    let post = SeedRawPost::new(user.user_id).seed(&state).await;

    state
        .site_config
        .set(
            storage::SiteConfigKey::FeedsWebsubHubUrl,
            "https://hub.example.com/",
        )
        .await
        .expect("set hub url");

    let feed_path = fp(&format!("/~{}/feed.rss", user.username));
    state
        .feed_events
        .enqueue(&feed_path)
        .await
        .expect("enqueue feed event");

    // Run the worker - ping will fail
    make_worker(&state, std::sync::Arc::new(FailingWebSubClient))
        .tick()
        .await;

    // Immediately after failure, the event should NOT be claimable (scheduled for future retry)
    let immediately_claimable = state
        .feed_events
        .claim_pending_batch(10, chrono::Duration::minutes(5))
        .await
        .expect("claim pending");
    assert!(
        immediately_claimable.is_empty(),
        "event should be scheduled for retry, not immediately claimable"
    );

    // Verify the cache row was still created (regen succeeded, only ping failed)
    let cache_row = state
        .feed_cache
        .get(&feed_path)
        .await
        .expect("get cache")
        .expect("cache row should exist even though ping failed");
    assert!(
        cache_row
            .representation()
            .body()
            .contains(post.title.as_ref())
    );
}

/// Restart-straddle (the centerpiece): a future-dated post goes live while the
/// worker is down. On the worker's first `go_live_pass` (`last_tick` == None) the
/// startup catch-up must re-enqueue the cached feed whose surface gained a live
/// post newer than its `generated_at`.
#[apply(backends)]
#[tokio::test]
async fn startup_catchup_regenerates_feed_for_go_live_while_down(#[case] backend: Backend) {
    use chrono::{Duration, TimeZone};
    let TestEnv { state, base: _base } = setup_with_base_url(backend).await;
    let worker = make_worker(&state, Arc::new(CapturingWebSubClient::default()));

    let user = SeedUser::new().seed(&state).await;

    let t0 = Utc.with_ymd_and_hms(2026, 6, 26, 10, 0, 0).unwrap();
    // A cached site feed generated at t0 (stale).
    state
        .feed_cache
        .upsert(
            FeedCacheRow::new(
                fp("/feed.atom"),
                SyndicationFeedRepresentation::try_from_stored(
                    FeedFormat::Atom,
                    FeedFormat::Atom.content_type(),
                    "stale".to_string(),
                )
                .expect("matching stored representation metadata"),
                parse_etag("\"etag\""),
                UtcInstant::from(t0),
                UtcInstant::from(t0),
            )
            .expect("matching cache row formats"),
        )
        .await
        .expect("seed cached feed");

    // A post that went live at t1 > t0 while the worker was "down".
    let t1 = t0 + Duration::hours(1);
    SeedRawPost::new(user.user_id)
        .published_at(common::time::UtcInstant::from(t1))
        .seed(&state)
        .await;

    // Restart: first go-live pass at t2 > t1 (last_tick == None => catch-up).
    let t2 = t1 + Duration::hours(1);
    worker
        .go_live_pass(common::time::UtcInstant::from(t2))
        .await
        .expect("go-live pass");

    let pending = state
        .feed_events
        .claim_pending_batch(100, chrono::Duration::minutes(5))
        .await
        .expect("claim pending");
    assert!(
        pending.iter().any(|r| r.feed_path == "/feed.atom"),
        "startup catch-up must enqueue the stale site feed: {:?}",
        pending.iter().map(|r| &r.feed_path).collect::<Vec<_>>()
    );
}

/// Steady state: once seeded, each pass enqueues the author's feed surfaces for
/// every post that crossed into "live" within the `(last_tick, now]` window.
#[apply(backends)]
#[tokio::test]
async fn steady_state_window_enqueues_newly_live_posts(#[case] backend: Backend) {
    use chrono::{Duration, TimeZone};
    let TestEnv { state, base: _base } = setup_with_base_url(backend).await;
    let worker = make_worker(&state, Arc::new(CapturingWebSubClient::default()));

    let user = SeedUser::new().seed(&state).await;

    // First pass seeds last_tick = t0 (startup branch; nothing cached/live).
    let t0 = Utc.with_ymd_and_hms(2026, 6, 26, 10, 0, 0).unwrap();
    worker
        .go_live_pass(common::time::UtcInstant::from(t0))
        .await
        .expect("seed last_tick");

    // A post that goes live between t0 and t1.
    let go_live = t0 + Duration::minutes(30);
    SeedRawPost::new(user.user_id)
        .published_at(common::time::UtcInstant::from(go_live))
        .seed(&state)
        .await;

    let t1 = t0 + Duration::hours(1);
    worker
        .go_live_pass(common::time::UtcInstant::from(t1))
        .await
        .expect("window pass");

    let pending = state
        .feed_events
        .claim_pending_batch(100, chrono::Duration::minutes(5))
        .await
        .expect("claim pending");
    let urls: Vec<&FeedPath> = pending.iter().map(|r| &r.feed_path).collect();
    assert!(
        urls.iter().any(|u| u.contains(&*user.username)),
        "the author's feeds must be enqueued on go-live: {urls:?}"
    );
    assert!(
        urls.iter().any(|u| u.as_ref() == "/feed.atom"),
        "the site feed must be enqueued on go-live: {urls:?}"
    );
}

#[apply(backends)]
#[tokio::test]
async fn worker_marks_exhausted_after_backoff_attempts_are_used_up(#[case] backend: Backend) {
    let TestEnv { state, base: _base } = setup_with_base_url(backend).await;

    // A published post so regeneration succeeds: the exhausted branch lives in
    // the ping sub-path, reached only after a successful regen.
    let user = SeedUser::new().seed(&state).await;
    SeedRawPost::new(user.user_id).seed(&state).await;

    state
        .site_config
        .set(
            storage::SiteConfigKey::FeedsWebsubHubUrl,
            "https://hub.example.com/",
        )
        .await
        .expect("set hub url");

    let feed_path = fp(&format!("/~{}/feed.rss", user.username));
    state
        .feed_events
        .enqueue(&feed_path)
        .await
        .expect("enqueue");

    // Drive the attempt count up to the backoff-table length by repeatedly
    // claiming and re-queuing with a past retry time (so it stays claimable).
    // The next real ping failure then exceeds the table and exhausts the event.
    let past = UtcInstant::from(Utc::now() - chrono::Duration::hours(1));
    for _ in 0..6 {
        let claimed = state
            .feed_events
            .claim_pending_batch(10, chrono::Duration::minutes(5))
            .await
            .expect("claim pending");
        let ids: Vec<FeedEventId> = claimed.iter().map(|r| r.id).collect();
        assert!(!ids.is_empty(), "event should be claimable while seeding");
        state
            .feed_events
            .mark_failed(&ids, "seed", past)
            .await
            .expect("mark failed");
    }

    make_worker(&state, std::sync::Arc::new(FailingWebSubClient))
        .tick()
        .await;

    // Exhausted events move to a terminal status and are no longer claimable,
    // even with a fully-elapsed retry window.
    let claimable = state
        .feed_events
        .claim_pending_batch(10, chrono::Duration::minutes(5))
        .await
        .expect("claim pending");
    assert!(
        claimable.is_empty(),
        "exhausted event should not be claimable"
    );
}
