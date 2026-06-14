#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::too_many_lines,
    clippy::similar_names,
    clippy::items_after_statements,
    clippy::unused_async
)]

mod helpers;

use chrono::Utc;
use common::password::Password;
use common::slug::Slug;
use common::username::Username;
use jaunder::feed::worker::tick;
use storage::{CreatePostInput, PostFormat};
use tempfile::TempDir;

#[tokio::test]
async fn worker_regenerates_claimed_event_and_marks_done_when_no_hub() {
    let base = TempDir::new().expect("temp dir");
    let (state, _capture) = helpers::test_state_with_websub(&base).await;

    // Create a user and a published post
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
        })
        .await
        .expect("create post");

    // Enqueue a feed event
    let feed_url = "/~alice/feed.rss";
    state
        .feed_events
        .enqueue(feed_url)
        .await
        .expect("enqueue feed event");

    // Run the worker
    tick(state.clone()).await;

    // Verify the feed was regenerated
    let cache_row = state
        .feed_cache
        .get(feed_url)
        .await
        .expect("get cache")
        .expect("cache row should exist");
    assert!(cache_row.body.contains("Test Post"));

    // Verify the event is marked as done (not claimable)
    let pending = state
        .feed_events
        .claim_pending_batch(10, chrono::Duration::minutes(5))
        .await
        .expect("claim pending");
    assert!(pending.is_empty(), "event should be done, not pending");
}

#[tokio::test]
async fn worker_pings_hub_when_configured() {
    let base = TempDir::new().expect("temp dir");
    let (state, capture) = helpers::test_state_with_websub(&base).await;

    // Create a user and a published post
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
        })
        .await
        .expect("create post");

    // Set hub URL
    const HUB_URL_KEY: &str = "feeds.websub_hub_url";
    state
        .site_config
        .set(HUB_URL_KEY, "https://hub.example.com/")
        .await
        .expect("set hub url");

    // Enqueue a feed event
    let feed_url = "/~alice/feed.rss";
    state
        .feed_events
        .enqueue(feed_url)
        .await
        .expect("enqueue feed event");

    // Run the worker
    tick(state.clone()).await;

    // Verify the ping was captured
    let pings = capture.pings();
    assert_eq!(pings.len(), 1, "should have exactly one ping");
    assert_eq!(pings[0].hub_url, "https://hub.example.com/");
    assert!(
        pings[0].feed_url.ends_with("/~alice/feed.rss"),
        "feed url should end with /~alice/feed.rss, got: {}",
        pings[0].feed_url
    );
}

#[tokio::test]
async fn worker_groups_duplicate_events_into_single_regen() {
    let base = TempDir::new().expect("temp dir");
    let (state, capture) = helpers::test_state_with_websub(&base).await;

    // Create a user and a published post
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
        })
        .await
        .expect("create post");

    // Set hub URL
    const HUB_URL_KEY: &str = "feeds.websub_hub_url";
    state
        .site_config
        .set(HUB_URL_KEY, "https://hub.example.com/")
        .await
        .expect("set hub url");

    // Enqueue the same feed 5 times
    let feed_url = "/~alice/feed.rss";
    for _ in 0..5 {
        state
            .feed_events
            .enqueue(feed_url)
            .await
            .expect("enqueue feed event");
    }

    // Run the worker
    tick(state.clone()).await;

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

#[tokio::test]
async fn worker_applies_backoff_on_regen_failure() {
    let base = TempDir::new().expect("temp dir");
    let (state, capture) = helpers::test_state_with_websub(&base).await;

    // Enqueue an event whose feed_url cannot be parsed into a feed surface;
    // regenerate_feed returns BadUrl before any hub logic runs.
    let feed_url = "/this-is-not-a-feed-url";
    state
        .feed_events
        .enqueue(feed_url)
        .await
        .expect("enqueue feed event");

    // Run the worker - regeneration will fail.
    tick(state.clone()).await;

    // No cache row should have been written.
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

    // Create a failing WebSub client
    struct FailingWebSubClient;
    #[async_trait::async_trait]
    impl common::websub::WebSubClient for FailingWebSubClient {
        async fn send_publish(
            &self,
            _hub_url: &str,
            _feed_url: &str,
        ) -> Result<(), common::websub::WebSubError> {
            Err(common::websub::WebSubError::HubRefused { status: 503 })
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
        media: std::sync::Arc::new(storage::SqliteMediaStorage::new(pool.clone())),
        user_config: std::sync::Arc::new(storage::SqliteUserConfigStorage::new(pool.clone())),
        feed_cache: std::sync::Arc::new(storage::SqliteFeedCacheStorage::new(pool.clone())),
        feed_events: std::sync::Arc::new(storage::SqliteFeedEventStorage::new(pool)),
        websub: std::sync::Arc::new(FailingWebSubClient),
    });

    // Create a user and a published post
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
        })
        .await
        .expect("create post");

    // Set hub URL
    const HUB_URL_KEY: &str = "feeds.websub_hub_url";
    state
        .site_config
        .set(HUB_URL_KEY, "https://hub.example.com/")
        .await
        .expect("set hub url");

    // Enqueue a feed event
    let feed_url = "/~alice/feed.rss";
    state
        .feed_events
        .enqueue(feed_url)
        .await
        .expect("enqueue feed event");

    // Run the worker - ping will fail
    tick(state.clone()).await;

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
