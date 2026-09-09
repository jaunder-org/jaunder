use std::path::Path;
use std::sync::Arc;

use crate::helpers::CapturingWebSubClient;
use common::{
    tagged_url::HubUrl, test_support::parse_etag, time::UtcInstant, visibility::AudienceTarget,
};
use host::feed::FeedPath;
use jaunder::feed::worker::FeedWorker;
use jiff::{Span, Timestamp, ToSpan};
use storage::MockPostStorage;
use storage::test_support::{
    Backend, SeedFeedCache, SeedRawPost, SeedUser, backends, confirmed_for, fp,
};

fn fixed_instant(value: &str) -> UtcInstant {
    UtcInstant::from(value.parse::<Timestamp>().expect("fixed test instant"))
}

fn add(instant: UtcInstant, span: Span) -> UtcInstant {
    UtcInstant::from(
        instant
            .value()
            .checked_add(span)
            .expect("fixture is within Timestamp range"),
    )
}

fn subtract(instant: UtcInstant, span: Span) -> UtcInstant {
    UtcInstant::from(
        instant
            .value()
            .checked_sub(span)
            .expect("fixture is within Timestamp range"),
    )
}
use rstest::*;
use rstest_reuse::*;

async fn event_write<T>(
    write_scope: storage::WriteScope,
    callback: impl for<'scope> FnOnce(
        &'scope mut storage::WriteTransaction,
    ) -> futures_util::future::BoxFuture<
        'scope,
        Result<T, storage::FeedEventError>,
    >,
) -> T {
    confirmed_for(
        write_scope.run(callback).await.expect("feed-event write"),
        "feed-event write acknowledgement",
    )
}

async fn set_hub(
    publisher: Arc<dyn storage::PublisherStorage>,
    write_scope: storage::WriteScope,
    storage_path: &Path,
) {
    let hub: HubUrl = "https://hub.example.com/".parse().expect("valid hub URL");
    jaunder::publisher::PublisherService::new(storage_path.to_owned(), publisher, write_scope)
        .mutate_hub(Some(&hub))
        .await
        .expect("set hub url");
}

/// Test double whose `WebSub` client reports a retryable failure, so the worker
/// exercises its ping-failure backoff path.
struct FailingWebSubClient;

#[async_trait::async_trait]
impl jaunder::websub::WebSubClient for FailingWebSubClient {
    async fn send_publish(
        &self,
        _hub_url: &common::tagged_url::HubUrl,
        _feed_url: &common::tagged_url::FeedUrl,
    ) -> Result<(), jaunder::websub::WebSubError> {
        Err(jaunder::websub::WebSubError::Retryable {
            reason: jaunder::websub::RetryableWebSubError::Http { status: 503 },
            retry_after: None,
        })
    }
}

/// Builds a [`FeedWorker`] from exact test storage handles and an injected
/// `WebSub` client.
fn make_worker(
    posts: Arc<dyn storage::PostStorage>,
    feed_cache: Arc<dyn storage::FeedCacheStorage>,
    publisher: Arc<dyn storage::PublisherStorage>,
    feed_events: Arc<dyn storage::FeedEventStorage>,
    write_scope: storage::WriteScope,
    storage_path: &Path,
    websub: Arc<dyn jaunder::websub::WebSubClient>,
) -> FeedWorker {
    FeedWorker::new(
        posts,
        feed_cache,
        Arc::new(write_scope.clone()),
        Arc::new(jaunder::publisher::PublisherService::new(
            storage_path.to_owned(),
            publisher,
            write_scope,
        )),
        feed_events,
        websub,
    )
}

#[apply(backends)]
#[tokio::test]
async fn worker_regenerates_claimed_event_and_marks_done_when_no_hub(#[case] backend: Backend) {
    let env = backend.setup().await;
    let capture = Arc::new(CapturingWebSubClient::default());

    let user = SeedUser::new()
        .seed(std::sync::Arc::clone(&env.users()), env.write_scope())
        .await;

    let post = SeedRawPost::new(user.user_id)
        .seed(std::sync::Arc::clone(&env.posts()), env.write_scope())
        .await;

    let feed_path = fp(&format!("/~{}/feed.rss", user.username));
    let event_feed_path = feed_path.clone();
    let feed_events = Arc::clone(&env.feed_events());
    event_write(env.write_scope(), move |transaction| {
        Box::pin(async move { feed_events.enqueue(transaction, &event_feed_path).await })
    })
    .await;

    make_worker(
        Arc::clone(&env.posts()),
        Arc::clone(&env.feed_cache()),
        Arc::clone(&env.publisher()),
        Arc::clone(&env.feed_events()),
        env.write_scope(),
        env.base.path(),
        capture.clone(),
    )
    .tick()
    .await;

    let cache_row = env
        .feed_cache()
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

    let feed_events = Arc::clone(&env.feed_events());
    let pending = event_write(env.write_scope(), move |transaction| {
        Box::pin(async move {
            feed_events
                .claim_pending_batch(
                    transaction,
                    10,
                    std::time::Duration::from_secs((5 * 60) as u64),
                )
                .await
        })
    })
    .await;
    assert!(pending.is_empty(), "event should be done, not pending");
}

#[apply(backends)]
#[tokio::test]
async fn worker_pings_hub_when_configured(#[case] backend: Backend) {
    let env = backend.setup().await;
    let capture = Arc::new(CapturingWebSubClient::default());

    let user = SeedUser::new()
        .seed(std::sync::Arc::clone(&env.users()), env.write_scope())
        .await;

    SeedRawPost::new(user.user_id)
        .seed(std::sync::Arc::clone(&env.posts()), env.write_scope())
        .await;

    set_hub(
        Arc::clone(&env.publisher()),
        env.write_scope(),
        env.base.path(),
    )
    .await;

    let feed_path = fp(&format!("/~{}/feed.rss", user.username));
    let feed_events = Arc::clone(&env.feed_events());
    event_write(env.write_scope(), move |transaction| {
        Box::pin(async move { feed_events.enqueue(transaction, &feed_path).await })
    })
    .await;

    make_worker(
        Arc::clone(&env.posts()),
        Arc::clone(&env.feed_cache()),
        Arc::clone(&env.publisher()),
        Arc::clone(&env.feed_events()),
        env.write_scope(),
        env.base.path(),
        capture.clone(),
    )
    .tick()
    .await;

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
    let env = backend.setup().await;
    let capture = Arc::new(CapturingWebSubClient::default());

    let user = SeedUser::new()
        .seed(std::sync::Arc::clone(&env.users()), env.write_scope())
        .await;

    SeedRawPost::new(user.user_id)
        .seed(std::sync::Arc::clone(&env.posts()), env.write_scope())
        .await;

    set_hub(
        Arc::clone(&env.publisher()),
        env.write_scope(),
        env.base.path(),
    )
    .await;

    let feed_path = fp(&format!("/~{}/feed.rss", user.username));
    let feed_events = Arc::clone(&env.feed_events());
    for _ in 0..5 {
        let feed_events = Arc::clone(&feed_events);
        let feed_path = feed_path.clone();
        event_write(env.write_scope(), move |transaction| {
            Box::pin(async move { feed_events.enqueue(transaction, &feed_path).await })
        })
        .await;
    }

    make_worker(
        Arc::clone(&env.posts()),
        Arc::clone(&env.feed_cache()),
        Arc::clone(&env.publisher()),
        Arc::clone(&env.feed_events()),
        env.write_scope(),
        env.base.path(),
        capture.clone(),
    )
    .tick()
    .await;

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
// real-backend explicit retry SQL stays covered by the dual-backend
// `feed_events` storage test.

#[apply(backends)]
#[tokio::test]
async fn grouped_regeneration_failure_leaves_publication_retry_in_its_phase(
    #[case] backend: Backend,
) {
    let env = backend.setup().await;
    let feed_path = fp("/feed.rss");
    let feed_events = Arc::clone(&env.feed_events());
    let regeneration_id = event_write(env.write_scope(), {
        let feed_path = feed_path.clone();
        move |transaction| {
            Box::pin(async move { feed_events.enqueue(transaction, &feed_path).await })
        }
    })
    .await;
    let feed_events = Arc::clone(&env.feed_events());
    let publication_id = event_write(env.write_scope(), {
        let feed_path = feed_path.clone();
        move |transaction| {
            Box::pin(async move { feed_events.enqueue(transaction, &feed_path).await })
        }
    })
    .await;
    let feed_events = Arc::clone(&env.feed_events());
    event_write(env.write_scope(), move |transaction| {
        Box::pin(async move {
            feed_events
                .mark_regenerated(transaction, &[publication_id])
                .await
        })
    })
    .await;

    let mut posts = MockPostStorage::new();
    posts
        .expect_feed_urls_needing_catchup()
        .times(1)
        .returning(|_| Ok(Vec::new()));
    posts
        .expect_list_published_in_window()
        .times(1)
        .returning(|_, _, _, _| Err(sqlx::Error::PoolClosed));
    FeedWorker::new(
        Arc::new(posts),
        env.feed_cache(),
        Arc::new(env.write_scope()),
        Arc::new(jaunder::publisher::PublisherService::new(
            env.base.path().to_owned(),
            env.publisher(),
            env.write_scope(),
        )),
        env.feed_events(),
        Arc::new(jaunder::websub::NoopWebSubClient),
    )
    .tick()
    .await;

    let feed_events = Arc::clone(&env.feed_events());
    let reclaimed = event_write(env.write_scope(), move |transaction| {
        Box::pin(async move {
            feed_events
                .claim_pending_batch(transaction, 10, std::time::Duration::ZERO)
                .await
        })
    })
    .await;
    assert_eq!(reclaimed.len(), 1);
    assert_eq!(reclaimed[0].id, publication_id);
    assert_eq!(reclaimed[0].phase, host::feed::FeedEventPhase::Publication);
    assert_eq!(reclaimed[0].regeneration_attempts, 0);
    assert_eq!(reclaimed[0].publication_attempts, 0);

    assert_ne!(regeneration_id, publication_id);
}

#[apply(backends)]
#[tokio::test]
async fn worker_applies_backoff_on_ping_failure(#[case] backend: Backend) {
    // WebSub ping-failure backoff is backend-agnostic: the shared setup runs it
    // on both backends so neither is left uncovered.
    let env = backend.setup().await;

    let user = SeedUser::new()
        .seed(std::sync::Arc::clone(&env.users()), env.write_scope())
        .await;

    let post = SeedRawPost::new(user.user_id)
        .seed(std::sync::Arc::clone(&env.posts()), env.write_scope())
        .await;

    set_hub(
        Arc::clone(&env.publisher()),
        env.write_scope(),
        env.base.path(),
    )
    .await;

    let feed_path = fp(&format!("/~{}/feed.rss", user.username));
    let event_feed_path = feed_path.clone();
    let feed_events = Arc::clone(&env.feed_events());
    event_write(env.write_scope(), move |transaction| {
        Box::pin(async move { feed_events.enqueue(transaction, &event_feed_path).await })
    })
    .await;

    // Run the worker - ping will fail
    make_worker(
        Arc::clone(&env.posts()),
        Arc::clone(&env.feed_cache()),
        Arc::clone(&env.publisher()),
        Arc::clone(&env.feed_events()),
        env.write_scope(),
        env.base.path(),
        std::sync::Arc::new(FailingWebSubClient),
    )
    .tick()
    .await;

    // Immediately after failure, the event should NOT be claimable (scheduled for future retry)
    let feed_events = Arc::clone(&env.feed_events());
    let immediately_claimable = event_write(env.write_scope(), move |transaction| {
        Box::pin(async move {
            feed_events
                .claim_pending_batch(
                    transaction,
                    10,
                    std::time::Duration::from_secs((5 * 60) as u64),
                )
                .await
        })
    })
    .await;
    assert!(
        immediately_claimable.is_empty(),
        "event should be scheduled for retry, not immediately claimable"
    );

    // Verify the cache row was still created (regen succeeded, only ping failed)
    let cache_row = env
        .feed_cache()
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
    let env = backend.setup().await;
    let worker = make_worker(
        Arc::clone(&env.posts()),
        Arc::clone(&env.feed_cache()),
        Arc::clone(&env.publisher()),
        Arc::clone(&env.feed_events()),
        env.write_scope(),
        env.base.path(),
        Arc::new(CapturingWebSubClient::default()),
    );

    let user = SeedUser::new()
        .seed(std::sync::Arc::clone(&env.users()), env.write_scope())
        .await;

    let t0 = fixed_instant("2026-06-26T10:00:00Z");
    // A cached site feed generated at t0 (stale).
    SeedFeedCache::new(fp("/feed.atom"))
        .body("stale".to_owned())
        .etag(parse_etag("\"etag\""))
        .representation_modified_at(t0)
        .generated_at(t0)
        .seed(env.feed_cache(), env.write_scope())
        .await;

    // A post that went live at t1 > t0 while the worker was "down".
    let t1 = add(t0, 1.hour());
    SeedRawPost::new(user.user_id)
        .published_at(t1)
        .seed(std::sync::Arc::clone(&env.posts()), env.write_scope())
        .await;

    // Restart: first go-live pass at t2 > t1 (last_tick == None => catch-up).
    let t2 = add(t1, 1.hour());
    worker.go_live_pass(t2).await.expect("go-live pass");

    let feed_events = Arc::clone(&env.feed_events());
    let pending = event_write(env.write_scope(), move |transaction| {
        Box::pin(async move {
            feed_events
                .claim_pending_batch(
                    transaction,
                    100,
                    std::time::Duration::from_secs((5 * 60) as u64),
                )
                .await
        })
    })
    .await;
    assert!(
        pending.iter().any(|r| r.feed_path == "/feed.atom"),
        "startup catch-up must enqueue the stale site feed: {:?}",
        pending.iter().map(|r| &r.feed_path).collect::<Vec<_>>()
    );
}

#[apply(backends)]
#[tokio::test]
async fn startup_catchup_ignores_nonpublic_posts(#[case] backend: Backend) {
    let env = backend.setup().await;
    let worker = make_worker(
        Arc::clone(&env.posts()),
        Arc::clone(&env.feed_cache()),
        Arc::clone(&env.publisher()),
        Arc::clone(&env.feed_events()),
        env.write_scope(),
        env.base.path(),
        Arc::new(CapturingWebSubClient::default()),
    );
    let user = SeedUser::new()
        .seed(std::sync::Arc::clone(&env.users()), env.write_scope())
        .await;
    let t0 = fixed_instant("2026-06-26T10:00:00Z");
    SeedFeedCache::new(fp("/feed.atom"))
        .body("stale".to_owned())
        .etag(parse_etag("\"etag\""))
        .representation_modified_at(t0)
        .generated_at(t0)
        .seed(env.feed_cache(), env.write_scope())
        .await;
    let go_live = add(t0, 1.hour());
    SeedRawPost::new(user.user_id)
        .published_at(go_live)
        .audiences(vec![AudienceTarget::Private])
        .seed(env.posts(), env.write_scope())
        .await;

    worker
        .go_live_pass(add(go_live, 1.hour()))
        .await
        .expect("go-live pass");

    let feed_events = Arc::clone(&env.feed_events());
    let pending = event_write(env.write_scope(), move |transaction| {
        Box::pin(async move {
            feed_events
                .claim_pending_batch(
                    transaction,
                    100,
                    std::time::Duration::from_secs((5 * 60) as u64),
                )
                .await
        })
    })
    .await;
    assert!(
        pending.is_empty(),
        "restart catch-up must ignore non-Public Posts: {pending:?}"
    );
}

/// Steady state: once seeded, each pass enqueues the author's feed surfaces for
/// every post that crossed into "live" within the `(last_tick, now]` window.
#[apply(backends)]
#[tokio::test]
async fn steady_state_window_enqueues_newly_live_posts(#[case] backend: Backend) {
    let env = backend.setup().await;
    let worker = make_worker(
        Arc::clone(&env.posts()),
        Arc::clone(&env.feed_cache()),
        Arc::clone(&env.publisher()),
        Arc::clone(&env.feed_events()),
        env.write_scope(),
        env.base.path(),
        Arc::new(CapturingWebSubClient::default()),
    );

    let user = SeedUser::new()
        .seed(std::sync::Arc::clone(&env.users()), env.write_scope())
        .await;

    // First pass seeds last_tick = t0 (startup branch; nothing cached/live).
    let t0 = fixed_instant("2026-06-26T10:00:00Z");
    worker.go_live_pass(t0).await.expect("seed last_tick");

    // A post that goes live between t0 and t1.
    let go_live = add(t0, 30.minute());
    SeedRawPost::new(user.user_id)
        .published_at(go_live)
        .seed(std::sync::Arc::clone(&env.posts()), env.write_scope())
        .await;

    let private_user = SeedUser::new()
        .seed(std::sync::Arc::clone(&env.users()), env.write_scope())
        .await;
    SeedRawPost::new(private_user.user_id)
        .published_at(go_live)
        .audiences(vec![AudienceTarget::Private])
        .seed(env.posts(), env.write_scope())
        .await;

    let t1 = add(t0, 1.hour());
    worker.go_live_pass(t1).await.expect("window pass");

    let feed_events = Arc::clone(&env.feed_events());
    let pending = event_write(env.write_scope(), move |transaction| {
        Box::pin(async move {
            feed_events
                .claim_pending_batch(
                    transaction,
                    100,
                    std::time::Duration::from_secs((5 * 60) as u64),
                )
                .await
        })
    })
    .await;
    let urls: Vec<&FeedPath> = pending.iter().map(|r| &r.feed_path).collect();
    assert!(
        urls.iter().any(|u| u.contains(&*user.username)),
        "the author's feeds must be enqueued on go-live: {urls:?}"
    );
    assert!(
        urls.iter().any(|u| u.as_ref() == "/feed.atom"),
        "the site feed must be enqueued on go-live: {urls:?}"
    );
    assert!(
        urls.iter()
            .all(|url| !url.contains(&*private_user.username)),
        "the non-Public author's feeds must not be enqueued: {urls:?}"
    );
}

#[apply(backends)]
#[tokio::test]
async fn worker_marks_exhausted_after_backoff_attempts_are_used_up(#[case] backend: Backend) {
    let env = backend.setup().await;

    // A published post so regeneration succeeds: the exhausted branch lives in
    // the ping sub-path, reached only after a successful regen.
    let user = SeedUser::new()
        .seed(std::sync::Arc::clone(&env.users()), env.write_scope())
        .await;
    SeedRawPost::new(user.user_id)
        .seed(std::sync::Arc::clone(&env.posts()), env.write_scope())
        .await;

    set_hub(
        Arc::clone(&env.publisher()),
        env.write_scope(),
        env.base.path(),
    )
    .await;

    let feed_path = fp(&format!("/~{}/feed.rss", user.username));
    let feed_events = Arc::clone(&env.feed_events());
    let event_id = event_write(env.write_scope(), move |transaction| {
        Box::pin(async move { feed_events.enqueue(transaction, &feed_path).await })
    })
    .await;
    let worker = make_worker(
        Arc::clone(&env.posts()),
        Arc::clone(&env.feed_cache()),
        Arc::clone(&env.publisher()),
        Arc::clone(&env.feed_events()),
        env.write_scope(),
        env.base.path(),
        std::sync::Arc::new(FailingWebSubClient),
    );
    worker.tick().await;

    // The first worker pass regenerates and commits the feed, then records
    // publication attempt one. Seed attempts two through nine with an elapsed
    // retry time. The next real ping failure is attempt ten, which consumes
    // the publication budget.
    let past = subtract(UtcInstant::now(), 1.hour());
    let feed_events = Arc::clone(&env.feed_events());
    for _ in 0..8 {
        let retry_publication_events = Arc::clone(&feed_events);
        event_write(env.write_scope(), move |transaction| {
            Box::pin(async move {
                retry_publication_events
                    .retry_publication(transaction, &[event_id], "seed", past)
                    .await
            })
        })
        .await;
    }

    worker.tick().await;

    // Exhausted events move to a terminal status and are no longer claimable,
    // even with a fully-elapsed retry window.
    let feed_events = Arc::clone(&env.feed_events());
    let claimable = event_write(env.write_scope(), move |transaction| {
        Box::pin(async move {
            feed_events
                .claim_pending_batch(
                    transaction,
                    10,
                    std::time::Duration::from_secs((5 * 60) as u64),
                )
                .await
        })
    })
    .await;
    assert!(
        claimable.is_empty(),
        "exhausted event should not be claimable"
    );
}
