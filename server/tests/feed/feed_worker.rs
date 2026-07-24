use common::visibility::AudienceTarget;
use std::sync::Arc;

use crate::helpers::{setup_with_base_url, CapturingWebSubClient};
use chrono::Utc;
use common::feed::FeedPath;
use common::ids::FeedEventId;
use common::slug::Slug;
use jaunder::feed::worker::FeedWorker;
use storage::test_support::{backends, fp, Backend, SeedUser, TestEnv};
use storage::{CreatePostInput, FeedCacheRow, PostFormat, RenderedHtml};

use rstest::*;
use rstest_reuse::*;

/// Test double whose `WebSub` client always reports the hub refused the ping,
/// so the worker exercises its ping-failure backoff path.
struct FailingWebSubClient;

#[async_trait::async_trait]
impl jaunder::websub::WebSubClient for FailingWebSubClient {
    async fn send_publish(
        &self,
        _hub_url: &str,
        _feed_url: &str,
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

    let user_id = SeedUser::new("alice").seed(&state).await;

    let now = Utc::now();
    let _post_id = state
        .posts
        .create_post(&CreatePostInput {
            user_id,
            title: Some("Test Post".into()),
            slug: "test-post".parse::<Slug>().expect("valid slug"),
            body: "# Test\n\nContent".into(),
            format: PostFormat::Markdown,
            rendered_html: RenderedHtml::from_trusted("<h1>Test</h1>\n<p>Content</p>"),
            published_at: Some(now),
            summary: None,
            audiences: vec![AudienceTarget::Public],
            idempotency_key: None,
        })
        .await
        .expect("create post");

    let feed_path = fp("/~alice/feed.rss");
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
    assert!(cache_row.body.contains("Test Post"));

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

    let user_id = SeedUser::new("alice").seed(&state).await;

    let now = Utc::now();
    let _post_id = state
        .posts
        .create_post(&CreatePostInput {
            user_id,
            title: Some("Test Post".into()),
            slug: "test-post".parse::<Slug>().expect("valid slug"),
            body: "# Test\n\nContent".into(),
            format: PostFormat::Markdown,
            rendered_html: RenderedHtml::from_trusted("<h1>Test</h1>\n<p>Content</p>"),
            published_at: Some(now),
            summary: None,
            audiences: vec![AudienceTarget::Public],
            idempotency_key: None,
        })
        .await
        .expect("create post");

    state
        .site_config
        .set("feeds.websub_hub_url", "https://hub.example.com/")
        .await
        .expect("set hub url");

    let feed_path = fp("/~alice/feed.rss");
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
        pings[0].feed_url.ends_with("/~alice/feed.rss"),
        "feed url should end with /~alice/feed.rss, got: {}",
        pings[0].feed_url
    );
}

#[apply(backends)]
#[tokio::test]
async fn worker_groups_duplicate_events_into_single_regen(#[case] backend: Backend) {
    let TestEnv { state, base: _base } = setup_with_base_url(backend).await;
    let capture = Arc::new(CapturingWebSubClient::default());

    let user_id = SeedUser::new("alice").seed(&state).await;

    let now = Utc::now();
    let _post_id = state
        .posts
        .create_post(&CreatePostInput {
            user_id,
            title: Some("Test Post".into()),
            slug: "test-post".parse::<Slug>().expect("valid slug"),
            body: "# Test\n\nContent".into(),
            format: PostFormat::Markdown,
            rendered_html: RenderedHtml::from_trusted("<h1>Test</h1>\n<p>Content</p>"),
            published_at: Some(now),
            summary: None,
            audiences: vec![AudienceTarget::Public],
            idempotency_key: None,
        })
        .await
        .expect("create post");

    state
        .site_config
        .set("feeds.websub_hub_url", "https://hub.example.com/")
        .await
        .expect("set hub url");

    let feed_path = fp("/~alice/feed.rss");
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
    assert!(pings[0].feed_url.ends_with("/~alice/feed.rss"));
}

// The regen-failure backoff behavior moved to a mock-based worker unit test
// (`worker::tests::tick_reschedules_on_regen_failure_within_backoff`): a
// `FeedPath` can no longer hold an unparseable value, so the former bad-URL
// trigger is unrepresentable, and a real backend cannot cheaply inject the only
// remaining failure (a storage error). The real-backend `mark_failed`
// scheduling SQL stays covered by the dual-backend `feed_events` storage test.

#[apply(backends)]
#[tokio::test]
async fn worker_applies_backoff_on_ping_failure(#[case] backend: Backend) {
    // WebSub ping-failure backoff is backend-agnostic: run it on both backends
    // via the shared setup instead of the hand-built SQLite-only AppState this
    // test used to construct (which left Postgres uncovered).
    let TestEnv { state, base: _base } = setup_with_base_url(backend).await;

    let user_id = SeedUser::new("alice").seed(&state).await;

    let now = Utc::now();
    let _post_id = state
        .posts
        .create_post(&CreatePostInput {
            user_id,
            title: Some("Test Post".into()),
            slug: "test-post".parse::<Slug>().expect("valid slug"),
            body: "# Test\n\nContent".into(),
            format: PostFormat::Markdown,
            rendered_html: RenderedHtml::from_trusted("<h1>Test</h1>\n<p>Content</p>"),
            published_at: Some(now),
            summary: None,
            audiences: vec![AudienceTarget::Public],
            idempotency_key: None,
        })
        .await
        .expect("create post");

    state
        .site_config
        .set("feeds.websub_hub_url", "https://hub.example.com/")
        .await
        .expect("set hub url");

    let feed_path = fp("/~alice/feed.rss");
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
    assert!(cache_row.body.contains("Test Post"));
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

    let user_id = SeedUser::new("alice").seed(&state).await;

    let t0 = Utc.with_ymd_and_hms(2026, 6, 26, 10, 0, 0).unwrap();
    // A cached site feed generated at t0 (stale).
    state
        .feed_cache
        .upsert(FeedCacheRow {
            feed_path: fp("/feed.atom"),
            body: "stale".to_string(),
            etag: "etag".to_string(),
            content_type: "application/atom+xml; charset=utf-8".to_string(),
            updated_at: t0,
            generated_at: t0,
        })
        .await
        .expect("seed cached feed");

    // A post that went live at t1 > t0 while the worker was "down".
    let t1 = t0 + Duration::hours(1);
    state
        .posts
        .create_post(&CreatePostInput {
            user_id,
            title: Some("Went live".into()),
            slug: "went-live".parse::<Slug>().expect("valid slug"),
            body: "# Went live\n\nbody".into(),
            format: PostFormat::Markdown,
            rendered_html: RenderedHtml::from_trusted("<h1>Went live</h1>"),
            published_at: Some(t1),
            summary: None,
            audiences: vec![AudienceTarget::Public],
            idempotency_key: None,
        })
        .await
        .expect("create post");

    // Restart: first go-live pass at t2 > t1 (last_tick == None => catch-up).
    let t2 = t1 + Duration::hours(1);
    worker.go_live_pass(t2).await.expect("go-live pass");

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

    let user_id = SeedUser::new("alice").seed(&state).await;

    // First pass seeds last_tick = t0 (startup branch; nothing cached/live).
    let t0 = Utc.with_ymd_and_hms(2026, 6, 26, 10, 0, 0).unwrap();
    worker.go_live_pass(t0).await.expect("seed last_tick");

    // A post that goes live between t0 and t1.
    let go_live = t0 + Duration::minutes(30);
    state
        .posts
        .create_post(&CreatePostInput {
            user_id,
            title: Some("Soon".into()),
            slug: "soon".parse::<Slug>().expect("valid slug"),
            body: "# Soon\n\nbody".into(),
            format: PostFormat::Markdown,
            rendered_html: RenderedHtml::from_trusted("<h1>Soon</h1>"),
            published_at: Some(go_live),
            summary: None,
            audiences: vec![AudienceTarget::Public],
            idempotency_key: None,
        })
        .await
        .expect("create post");

    let t1 = t0 + Duration::hours(1);
    worker.go_live_pass(t1).await.expect("window pass");

    let pending = state
        .feed_events
        .claim_pending_batch(100, chrono::Duration::minutes(5))
        .await
        .expect("claim pending");
    let urls: Vec<&FeedPath> = pending.iter().map(|r| &r.feed_path).collect();
    assert!(
        urls.iter().any(|u| u.contains("alice")),
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
    let user_id = SeedUser::new("alice").seed(&state).await;
    let now = Utc::now();
    state
        .posts
        .create_post(&CreatePostInput {
            user_id,
            title: Some("Test Post".into()),
            slug: "test-post".parse::<Slug>().expect("valid slug"),
            body: "# Test\n\nContent".into(),
            format: PostFormat::Markdown,
            rendered_html: RenderedHtml::from_trusted("<h1>Test</h1>\n<p>Content</p>"),
            published_at: Some(now),
            summary: None,
            audiences: vec![AudienceTarget::Public],
            idempotency_key: None,
        })
        .await
        .expect("create post");

    state
        .site_config
        .set("feeds.websub_hub_url", "https://hub.example.com/")
        .await
        .expect("set hub url");

    let feed_path = fp("/~alice/feed.rss");
    state
        .feed_events
        .enqueue(&feed_path)
        .await
        .expect("enqueue");

    // Drive the attempt count up to the backoff-table length by repeatedly
    // claiming and re-queuing with a past retry time (so it stays claimable).
    // The next real ping failure then exceeds the table and exhausts the event.
    let past = Utc::now() - chrono::Duration::hours(1);
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
