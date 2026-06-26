#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::too_many_lines,
    clippy::similar_names,
    clippy::items_after_statements,
    clippy::unused_async
)]
#![allow(unused_macros)]

use common::visibility::AudienceTarget;
use std::sync::Arc;

use crate::helpers::{backends, Backend, CapturingWebSubClient, TestEnv};
use chrono::Utc;
use common::password::Password;
use common::slug::Slug;
use common::username::Username;
use jaunder::feed::worker::FeedWorker;
use storage::{CreatePostInput, PostFormat};
use tempfile::TempDir;

use rstest::*;
#[allow(clippy::single_component_path_imports)]
use rstest_reuse;
use rstest_reuse::*;

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
    let TestEnv { state, base: _base } = backend.setup().await;
    let capture = Arc::new(CapturingWebSubClient::default());

    let username: Username = "alice".parse().expect("valid username");
    let password: Password = "password123".parse().expect("valid password");
    let user_id = state
        .users
        .create_user(&username, &password, None, false)
        .await
        .expect("create user");

    let now = Utc::now();
    let _post_id = state
        .posts
        .create_post(&CreatePostInput {
            user_id,
            title: Some("Test Post".to_string()),
            slug: "test-post".parse::<Slug>().expect("valid slug"),
            body: "# Test\n\nContent".to_string(),
            format: PostFormat::Markdown,
            rendered_html: "<h1>Test</h1>\n<p>Content</p>".to_string(),
            published_at: Some(now),
            summary: None,
            audiences: vec![AudienceTarget::Public],
        })
        .await
        .expect("create post");

    let feed_url = "/~alice/feed.rss";
    state
        .feed_events
        .enqueue(feed_url)
        .await
        .expect("enqueue feed event");

    make_worker(&state, capture.clone()).tick().await;

    let cache_row = state
        .feed_cache
        .get(feed_url)
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
    let TestEnv { state, base: _base } = backend.setup().await;
    let capture = Arc::new(CapturingWebSubClient::default());

    let username: Username = "alice".parse().expect("valid username");
    let password: Password = "password123".parse().expect("valid password");
    let user_id = state
        .users
        .create_user(&username, &password, None, false)
        .await
        .expect("create user");

    let now = Utc::now();
    let _post_id = state
        .posts
        .create_post(&CreatePostInput {
            user_id,
            title: Some("Test Post".to_string()),
            slug: "test-post".parse::<Slug>().expect("valid slug"),
            body: "# Test\n\nContent".to_string(),
            format: PostFormat::Markdown,
            rendered_html: "<h1>Test</h1>\n<p>Content</p>".to_string(),
            published_at: Some(now),
            summary: None,
            audiences: vec![AudienceTarget::Public],
        })
        .await
        .expect("create post");

    const HUB_URL_KEY: &str = "feeds.websub_hub_url";
    state
        .site_config
        .set(HUB_URL_KEY, "https://hub.example.com/")
        .await
        .expect("set hub url");

    let feed_url = "/~alice/feed.rss";
    state
        .feed_events
        .enqueue(feed_url)
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
    let TestEnv { state, base: _base } = backend.setup().await;
    let capture = Arc::new(CapturingWebSubClient::default());

    let username: Username = "alice".parse().expect("valid username");
    let password: Password = "password123".parse().expect("valid password");
    let user_id = state
        .users
        .create_user(&username, &password, None, false)
        .await
        .expect("create user");

    let now = Utc::now();
    let _post_id = state
        .posts
        .create_post(&CreatePostInput {
            user_id,
            title: Some("Test Post".to_string()),
            slug: "test-post".parse::<Slug>().expect("valid slug"),
            body: "# Test\n\nContent".to_string(),
            format: PostFormat::Markdown,
            rendered_html: "<h1>Test</h1>\n<p>Content</p>".to_string(),
            published_at: Some(now),
            summary: None,
            audiences: vec![AudienceTarget::Public],
        })
        .await
        .expect("create post");

    const HUB_URL_KEY: &str = "feeds.websub_hub_url";
    state
        .site_config
        .set(HUB_URL_KEY, "https://hub.example.com/")
        .await
        .expect("set hub url");

    let feed_url = "/~alice/feed.rss";
    for _ in 0..5 {
        state
            .feed_events
            .enqueue(feed_url)
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

#[apply(backends)]
#[tokio::test]
async fn worker_applies_backoff_on_regen_failure(#[case] backend: Backend) {
    let TestEnv { state, base: _base } = backend.setup().await;
    let capture = Arc::new(CapturingWebSubClient::default());

    // Enqueue an event whose feed_url cannot be parsed into a feed surface;
    // regenerate_feed returns BadUrl before any hub logic runs.
    let feed_url = "/this-is-not-a-feed-url";
    state
        .feed_events
        .enqueue(feed_url)
        .await
        .expect("enqueue feed event");

    // Run the worker - regeneration will fail.
    make_worker(&state, capture.clone()).tick().await;

    let cache = state.feed_cache.get(feed_url).await.expect("get cache");
    assert!(
        cache.is_none(),
        "no cache row should exist on regen failure"
    );

    // No ping should have been attempted (regen failed first).
    assert!(
        capture.pings().is_empty(),
        "no ping should be sent when regeneration fails"
    );

    // The event is scheduled for a future retry, not immediately claimable.
    let immediately_claimable = state
        .feed_events
        .claim_pending_batch(10, chrono::Duration::minutes(5))
        .await
        .expect("claim pending");
    assert!(
        immediately_claimable.is_empty(),
        "event should be scheduled for retry, not immediately claimable"
    );
}

#[tokio::test]
async fn worker_applies_backoff_on_ping_failure() {
    let base = TempDir::new().expect("temp dir");
    let pool = sqlx::SqlitePool::connect_with(
        format!("sqlite:{}", base.path().join("test.db").display())
            .parse::<sqlx::sqlite::SqliteConnectOptions>()
            .unwrap()
            .create_if_missing(true),
    )
    .await
    .unwrap();
    sqlx::migrate!("../storage/migrations/sqlite")
        .run(&pool)
        .await
        .unwrap();

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

    let state = std::sync::Arc::new(storage::AppState {
        site_config: std::sync::Arc::new(storage::SqliteSiteConfigStorage::new(pool.clone())),
        users: std::sync::Arc::new(storage::SqliteUserStorage::new(pool.clone())),
        sessions: std::sync::Arc::new(storage::SqliteSessionStorage::new(pool.clone())),
        invites: std::sync::Arc::new(storage::SqliteInviteStorage::new(pool.clone())),
        atomic: std::sync::Arc::new(storage::SqliteAtomicOps::new(pool.clone())),
        email_verifications: std::sync::Arc::new(storage::SqliteEmailVerificationStorage::new(
            pool.clone(),
        )),
        password_resets: std::sync::Arc::new(storage::SqlitePasswordResetStorage::new(
            pool.clone(),
        )),
        posts: std::sync::Arc::new(storage::SqlitePostStorage::new(pool.clone())),
        subscriptions: std::sync::Arc::new(storage::SqliteSubscriptionStorage::new(
            pool.clone(),
            std::sync::Arc::new(common::visibility::OpenSubscriptionPolicy),
        )),
        audiences: std::sync::Arc::new(storage::SqliteAudienceStorage::new(pool.clone())),
        media: std::sync::Arc::new(storage::SqliteMediaStorage::new(pool.clone())),
        user_config: std::sync::Arc::new(storage::SqliteUserConfigStorage::new(pool.clone())),
        feed_cache: std::sync::Arc::new(storage::SqliteFeedCacheStorage::new(pool.clone())),
        feed_events: std::sync::Arc::new(storage::SqliteFeedEventStorage::new(pool)),
    });

    let username: Username = "alice".parse().expect("valid username");
    let password: Password = "password123".parse().expect("valid password");
    let user_id = state
        .users
        .create_user(&username, &password, None, false)
        .await
        .expect("create user");

    let now = Utc::now();
    let _post_id = state
        .posts
        .create_post(&CreatePostInput {
            user_id,
            title: Some("Test Post".to_string()),
            slug: "test-post".parse::<Slug>().expect("valid slug"),
            body: "# Test\n\nContent".to_string(),
            format: PostFormat::Markdown,
            rendered_html: "<h1>Test</h1>\n<p>Content</p>".to_string(),
            published_at: Some(now),
            summary: None,
            audiences: vec![AudienceTarget::Public],
        })
        .await
        .expect("create post");

    const HUB_URL_KEY: &str = "feeds.websub_hub_url";
    state
        .site_config
        .set(HUB_URL_KEY, "https://hub.example.com/")
        .await
        .expect("set hub url");

    let feed_url = "/~alice/feed.rss";
    state
        .feed_events
        .enqueue(feed_url)
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
        .get(feed_url)
        .await
        .expect("get cache")
        .expect("cache row should exist even though ping failed");
    assert!(cache_row.body.contains("Test Post"));
}

#[apply(backends)]
#[tokio::test]
async fn worker_marks_exhausted_after_backoff_attempts_are_used_up(#[case] backend: Backend) {
    let TestEnv { state, base: _base } = backend.setup().await;

    // A published post so regeneration succeeds: the exhausted branch lives in
    // the ping sub-path, reached only after a successful regen.
    let username: Username = "alice".parse().expect("valid username");
    let password: Password = "password123".parse().expect("valid password");
    let user_id = state
        .users
        .create_user(&username, &password, None, false)
        .await
        .expect("create user");
    let now = Utc::now();
    state
        .posts
        .create_post(&CreatePostInput {
            user_id,
            title: Some("Test Post".to_string()),
            slug: "test-post".parse::<Slug>().expect("valid slug"),
            body: "# Test\n\nContent".to_string(),
            format: PostFormat::Markdown,
            rendered_html: "<h1>Test</h1>\n<p>Content</p>".to_string(),
            published_at: Some(now),
            summary: None,
            audiences: vec![AudienceTarget::Public],
        })
        .await
        .expect("create post");

    const HUB_URL_KEY: &str = "feeds.websub_hub_url";
    state
        .site_config
        .set(HUB_URL_KEY, "https://hub.example.com/")
        .await
        .expect("set hub url");

    let feed_url = "/~alice/feed.rss";
    state.feed_events.enqueue(feed_url).await.expect("enqueue");

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
        let ids: Vec<i64> = claimed.iter().map(|r| r.id).collect();
        assert!(!ids.is_empty(), "event should be claimable while seeding");
        state
            .feed_events
            .mark_failed(&ids, "seed", past)
            .await
            .expect("mark failed");
    }

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
