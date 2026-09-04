use axum::{
    body::Body,
    http::{Request, StatusCode, header},
};
use chrono::{Timelike, Utc};
use common::{tagged_url::HubUrl, test_support::parse_etag, time::UtcInstant};
use tower::ServiceExt;

use rstest::*;
use rstest_reuse::*;
use std::error::Error;
use std::sync::Arc;

use crate::helpers::{body_string, make_app};
use storage::test_support::{
    Backend, SeedFeedCache, SeedRawPost, SeedUser, TestEnv, backends, backends_matrix, fp,
};

fn with_feed_cache(
    state: &Arc<storage::AppState>,
    feed_cache: Arc<dyn storage::FeedCacheStorage>,
) -> Arc<storage::AppState> {
    Arc::new(storage::AppState {
        site_config: state.site_config.clone(),
        users: state.users.clone(),
        sessions: state.sessions.clone(),
        invites: state.invites.clone(),
        email_verifications: state.email_verifications.clone(),
        password_resets: state.password_resets.clone(),
        posts: state.posts.clone(),
        subscriptions: state.subscriptions.clone(),
        audiences: state.audiences.clone(),
        media: state.media.clone(),
        user_config: state.user_config.clone(),
        feed_cache,
        feed_events: state.feed_events.clone(),
        publisher: state.publisher.clone(),
        write_scope: state.write_scope.clone(),
    })
}

fn with_posts_and_publisher(
    state: &Arc<storage::AppState>,
    posts: Arc<dyn storage::PostStorage>,
    publisher: Arc<dyn storage::PublisherStorage>,
) -> Arc<storage::AppState> {
    Arc::new(storage::AppState {
        site_config: state.site_config.clone(),
        users: state.users.clone(),
        sessions: state.sessions.clone(),
        invites: state.invites.clone(),
        email_verifications: state.email_verifications.clone(),
        password_resets: state.password_resets.clone(),
        posts,
        subscriptions: state.subscriptions.clone(),
        audiences: state.audiences.clone(),
        media: state.media.clone(),
        user_config: state.user_config.clone(),
        feed_cache: state.feed_cache.clone(),
        feed_events: state.feed_events.clone(),
        publisher,
        write_scope: state.write_scope.clone(),
    })
}

fn typed_source<T: Error + 'static>(error: &web::error::InternalError) -> Option<&T> {
    let mut current: &(dyn Error + 'static) = error;
    loop {
        if let Some(source) = current.downcast_ref::<T>() {
            return Some(source);
        }
        current = current.source()?;
    }
}

#[apply(backends)]
#[tokio::test]
async fn handler_cache_miss_lazy_regens_and_returns_200_with_correct_content_type(
    #[case] backend: Backend,
) {
    let TestEnv { state, base } = backend.setup().await;
    let app = make_app(&state, &base);

    let user = SeedUser::new().seed(&state).await;

    SeedRawPost::new(user.user_id).seed(&state).await;

    let req = Request::builder()
        .method("GET")
        .uri(format!("/~{}/feed.rss", user.username))
        .body(Body::empty())
        .expect("build request");

    let resp = app.clone().oneshot(req).await.expect("request");

    assert_eq!(resp.status(), StatusCode::OK, "should return 200");

    let content_type = resp
        .headers()
        .get(header::CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .expect("content-type header");
    assert_eq!(
        content_type, "application/rss+xml; charset=utf-8",
        "RSS content type"
    );

    let body = axum::body::to_bytes(resp.into_body(), usize::MAX)
        .await
        .expect("read body");
    assert!(!body.is_empty(), "response body should not be empty");

    let req = Request::builder()
        .method("GET")
        .uri(format!("/~{}/feed.rss", user.username))
        .body(Body::empty())
        .expect("build request");
    let resp = app.clone().oneshot(req).await.expect("request");
    assert!(
        resp.headers().get(header::ETAG).is_some(),
        "ETag header should be present"
    );
    assert!(
        resp.headers().get(header::LAST_MODIFIED).is_some(),
        "Last-Modified header should be present"
    );

    let cached = state
        .feed_cache
        .get(&fp(&format!("/~{}/feed.rss", user.username)))
        .await
        .expect("get from cache")
        .expect("cache entry should exist");
    assert!(
        !cached.representation().body().is_empty(),
        "cached body should not be empty"
    );
}

#[apply(backends)]
#[tokio::test]
async fn handler_cache_miss_restarts_after_hub_mutation_and_caches_only_new_discovery(
    #[case] backend: Backend,
) {
    let TestEnv { state, base } = backend.setup().await;
    let publisher = Arc::new(jaunder::publisher::PublisherService::new(
        base.path().to_path_buf(),
        Arc::clone(&state.publisher),
        state.write_scope.clone(),
    ));
    let old_hub: HubUrl = "https://old-hub.example/".parse().expect("valid hub URL");
    let new_hub: HubUrl = "https://new-hub.example/".parse().expect("valid hub URL");
    publisher
        .mutate_hub(Some(&old_hub))
        .await
        .expect("seed old normalized hub");
    let old_snapshot = state.publisher.snapshot().await.expect("old snapshot");
    publisher
        .mutate_hub(Some(&new_hub))
        .await
        .expect("replace normalized hub");
    let new_snapshot = state.publisher.snapshot().await.expect("new snapshot");

    let snapshots = Arc::new(std::sync::Mutex::new(std::collections::VecDeque::from([
        old_snapshot,
        new_snapshot.clone(),
    ])));
    let snapshots_for_read = Arc::clone(&snapshots);
    let committed = Arc::new(std::sync::Mutex::new(None));
    let committed_for_write = Arc::clone(&committed);
    let old_hub_for_write = old_hub.clone();
    let new_hub_for_write = new_hub.clone();
    let expected_generation = new_snapshot.generation;
    let mut publisher_storage = storage::MockPublisherStorage::new();
    publisher_storage
        .expect_snapshot()
        .times(2)
        .returning(move || {
            Ok(snapshots_for_read
                .lock()
                .expect("snapshot queue")
                .pop_front()
                .expect("expected snapshot"))
        });
    publisher_storage
        .expect_commit_cache()
        .times(2)
        .returning(move |_, generation, row| {
            if row
                .representation()
                .body()
                .contains(old_hub_for_write.as_ref())
            {
                return Ok(storage::CacheCommitOutcome::StaleGeneration);
            }
            assert_eq!(generation, expected_generation);
            assert!(
                row.representation()
                    .body()
                    .contains(new_hub_for_write.as_ref())
            );
            *committed_for_write.lock().expect("committed row") = Some(row);
            Ok(storage::CacheCommitOutcome::Committed)
        });
    let mut posts = storage::MockPostStorage::new();
    posts
        .expect_list_published_in_window()
        .times(2)
        .returning(|_, _, _, _| Ok(vec![]));
    let state = with_posts_and_publisher(&state, Arc::new(posts), Arc::new(publisher_storage));
    let app = make_app(&state, &base);

    let response = app
        .oneshot(
            Request::builder()
                .method("GET")
                .uri("/feed.rss")
                .body(Body::empty())
                .expect("build request"),
        )
        .await
        .expect("request");
    assert_eq!(response.status(), StatusCode::OK);
    let body = body_string(response).await;
    assert!(body.contains(new_hub.as_ref()));
    assert!(!body.contains(old_hub.as_ref()));

    let cached = committed
        .lock()
        .expect("committed row")
        .take()
        .expect("cache entry committed");
    assert!(cached.representation().body().contains(new_hub.as_ref()));
    assert!(!cached.representation().body().contains(old_hub.as_ref()));
}

#[apply(backends)]
#[tokio::test]
async fn handler_serves_site_tag_feed_with_200(#[case] backend: Backend) {
    let TestEnv { state, base } = backend.setup().await;
    let app = make_app(&state, &base);

    // A tagged, published post so the site-tag surface has content.
    let user_id = SeedUser::new().seed(&state).await.user_id;
    SeedRawPost::new(user_id).tags(["rust"]).seed(&state).await;

    // The valid site-tag route exercises feed_site_tag's happy path: parse the
    // tag, then serve/regenerate the SiteTag surface.
    let req = Request::builder()
        .method("GET")
        .uri("/tags/rust/feed.rss")
        .body(Body::empty())
        .expect("build request");
    let resp = app.oneshot(req).await.expect("request");

    assert_eq!(
        resp.status(),
        StatusCode::OK,
        "valid site-tag feed should return 200"
    );
    let content_type = resp
        .headers()
        .get(header::CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .expect("content-type header");
    assert_eq!(content_type, "application/rss+xml; charset=utf-8");
}

#[apply(backends)]
#[tokio::test]
async fn handler_cache_hit_serves_stored_body_without_regeneration(#[case] backend: Backend) {
    let TestEnv { state, base } = backend.setup().await;
    let app = make_app(&state, &base);

    // Pre-populate the cache with a known body and validators.
    let known_body = "known feed body";
    let etag = "\"known-etag\"";
    let updated_at = UtcInstant::now();
    SeedFeedCache::new(fp("/~bob/feed.rss"))
        .body(known_body.to_owned())
        .etag(parse_etag(etag))
        .updated_at(updated_at)
        .generated_at(UtcInstant::now())
        .seed(&state)
        .await;

    let req = Request::builder()
        .method("GET")
        .uri("/~bob/feed.rss")
        .body(Body::empty())
        .expect("build request");

    let resp = app.clone().oneshot(req).await.expect("request");

    assert_eq!(resp.status(), StatusCode::OK, "should return 200");

    assert_eq!(
        resp.headers().get(header::CONTENT_TYPE),
        Some(&header::HeaderValue::from_static(
            "application/rss+xml; charset=utf-8"
        )),
        "cached representation derives its RSS content type"
    );
    assert_eq!(
        resp.headers()
            .get(header::ETAG)
            .and_then(|value| value.to_str().ok()),
        Some(etag),
        "cached ETag is preserved"
    );
    assert_eq!(
        resp.headers()
            .get(header::LAST_MODIFIED)
            .and_then(|value| value.to_str().ok()),
        Some(updated_at.value().to_rfc2822().as_str()),
        "cached Last-Modified is preserved"
    );

    // Assert body is the stored body (not regenerated)
    let body = axum::body::to_bytes(resp.into_body(), usize::MAX)
        .await
        .expect("read body");
    assert_eq!(
        String::from_utf8_lossy(&body),
        known_body,
        "should serve the exact cached body"
    );
}

#[apply(backends)]
#[tokio::test]
async fn handler_rejects_corrupt_cache_hit_without_serving_or_rewriting_it(
    #[case] backend: Backend,
) {
    let TestEnv { state, base } = backend.setup().await;
    let user = SeedUser::new().seed(&state).await;
    SeedRawPost::new(user.user_id).seed(&state).await;

    let feed_path = format!("/~{}/feed.rss", user.username);
    let cached_body = "corrupt-cache-body";
    let etag = "\"corrupt-cache-etag\"";
    SeedFeedCache::new(fp(&feed_path))
        .body(cached_body.to_owned())
        .etag(parse_etag(etag))
        .updated_at(UtcInstant::now())
        .generated_at(UtcInstant::now())
        .seed(&state)
        .await;

    // Bypass the invariant-bearing storage API to model a corrupted persisted
    // metadata column while retaining an otherwise coherent cache row.
    base.pool()
        .execute(&format!(
            "UPDATE feed_cache SET content_type = 'application/atom+xml; charset=utf-8' \
             WHERE feed_url = '{feed_path}'"
        ))
        .await
        .expect("corrupt stored content type");

    let request = Request::builder()
        .method("GET")
        .uri(&feed_path)
        .body(Body::empty())
        .expect("build request");
    let response = make_app(&state, &base)
        .oneshot(request)
        .await
        .expect("request");

    assert_eq!(
        response.status(),
        StatusCode::INTERNAL_SERVER_ERROR,
        "corrupt cache hits are server failures, not cache misses"
    );
    let response_body = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .expect("read body");
    assert!(
        response_body.is_empty(),
        "corrupt cache hits must not serve the cached body: {response_body:?}"
    );

    let raw_rows = base
        .pool()
        .string_quintuples(&format!(
            "SELECT feed_url, body, etag, content_type, CAST(generated_at AS TEXT) \
             FROM feed_cache WHERE feed_url = '{feed_path}'"
        ))
        .await
        .expect("inspect raw cache row after request");
    let [(stored_path, stored_body, stored_etag, stored_content_type, _)] = raw_rows.as_slice()
    else {
        panic!("expected exactly one raw cache row, got {raw_rows:?}");
    };
    assert_eq!(stored_path, &feed_path, "cache key is unchanged");
    assert_eq!(stored_body, cached_body, "cache body is unchanged");
    assert_eq!(stored_etag, etag, "cache ETag is unchanged");
    assert_eq!(
        stored_content_type, "application/atom+xml; charset=utf-8",
        "corrupt stored content type is not repaired"
    );
}

#[apply(backends)]
#[tokio::test]
async fn handler_if_none_match_returns_304(#[case] backend: Backend) {
    let TestEnv { state, base } = backend.setup().await;
    let app = make_app(&state, &base);

    // The stored ETag and the `If-None-Match` header must be the same quoted string.
    let etag = "\"test-etag-123\"";
    SeedFeedCache::new(fp("/~charlie/feed.rss"))
        .body("feed body".to_owned())
        .etag(parse_etag(etag))
        .updated_at(UtcInstant::now())
        .generated_at(UtcInstant::now())
        .seed(&state)
        .await;

    let req = Request::builder()
        .method("GET")
        .uri("/~charlie/feed.rss")
        .header(header::IF_NONE_MATCH, etag)
        .body(Body::empty())
        .expect("build request");

    let resp = app.oneshot(req).await.expect("request");

    assert_eq!(
        resp.status(),
        StatusCode::NOT_MODIFIED,
        "should return 304 when ETag matches"
    );
}

#[apply(backends)]
#[tokio::test]
async fn handler_if_modified_since_returns_304_when_unchanged(#[case] backend: Backend) {
    let TestEnv { state, base } = backend.setup().await;
    let app = make_app(&state, &base);

    // Round to seconds to ensure RFC2822 conversion is lossless
    let update_time = Utc::now()
        .with_nanosecond(0)
        .expect("valid nanosecond value");
    SeedFeedCache::new(fp("/~dave/feed.rss"))
        .body("feed body".to_owned())
        .etag(parse_etag("\"test-etag\""))
        .updated_at(UtcInstant::from(update_time))
        .generated_at(UtcInstant::now())
        .seed(&state)
        .await;

    // Request with If-Modified-Since set to the same time
    let req = Request::builder()
        .method("GET")
        .uri("/~dave/feed.rss")
        .header(header::IF_MODIFIED_SINCE, update_time.to_rfc2822())
        .body(Body::empty())
        .expect("build request");

    let resp = app.oneshot(req).await.expect("request");

    assert_eq!(
        resp.status(),
        StatusCode::NOT_MODIFIED,
        "should return 304 when If-Modified-Since matches"
    );
}

// Feed extensions soft-parse at every public feed route, so both unknown and
// case-variant extensions reach the handler and become 404s rather than Axum
// extractor 400s. The remaining cases cover invalid route-value soft misses.
#[apply(backends_matrix)]
#[case::site_unknown_extension("/feed.xml")]
#[case::site_case_variant_extension("/feed.RSS")]
#[case::site_tag_unknown_extension("/tags/rust/feed.xml")]
#[case::site_tag_case_variant_extension("/tags/rust/feed.RSS")]
#[case::user_unknown_extension("/~alice/feed.xml")]
#[case::user_case_variant_extension("/~alice/feed.RSS")]
#[case::user_tag_unknown_extension("/~alice/tags/rust/feed.xml")]
#[case::user_tag_case_variant_extension("/~alice/tags/rust/feed.RSS")]
#[case::invalid_tag("/tags/-rust/feed.rss")]
#[case::invalid_username("/~al.ice/feed.rss")]
#[case::invalid_user_tag("/~alice/tags/-rust/feed.rss")]
#[tokio::test]
async fn handler_rejects_invalid_request_with_404(backend: Backend, #[case] uri: &str) {
    let TestEnv { state, base } = backend.setup().await;
    let app = make_app(&state, &base);

    let req = Request::builder()
        .method("GET")
        .uri(uri)
        .body(Body::empty())
        .expect("build request");

    let resp = app.oneshot(req).await.expect("request");

    assert_eq!(
        resp.status(),
        StatusCode::NOT_FOUND,
        "should return 404 for an invalid public feed route: {uri}"
    );
}

#[apply(backends)]
#[tokio::test]
async fn handler_returns_correct_content_type_per_format(#[case] backend: Backend) {
    let TestEnv { state, base } = backend.setup().await;

    let user = SeedUser::new().seed(&state).await;

    SeedRawPost::new(user.user_id).seed(&state).await;

    let test_cases = [
        ("rss", "application/rss+xml; charset=utf-8"),
        ("atom", "application/atom+xml; charset=utf-8"),
        ("json", "application/feed+json"),
    ];

    for (ext, expected_content_type) in &test_cases {
        let app = make_app(&state, &base);
        let req = Request::builder()
            .method("GET")
            .uri(format!("/~{}/feed.{ext}", user.username))
            .body(Body::empty())
            .expect("build request");

        let resp = app.oneshot(req).await.expect("request");

        assert_eq!(resp.status(), StatusCode::OK, "should return 200 for {ext}");

        let content_type = resp
            .headers()
            .get(header::CONTENT_TYPE)
            .and_then(|v| v.to_str().ok())
            .unwrap_or_else(|| panic!("content-type header for {ext}"));
        assert_eq!(
            content_type, *expected_content_type,
            "content type for {ext}"
        );
    }
}

// guard:no-backend — pure source-preserving boundary adapters
#[test]
fn feed_failure_adapters_retain_typed_sources() {
    let cache = jaunder::feed::handlers::map_feed_cache_failure(storage::FeedCacheError::Db(
        sqlx::Error::PoolClosed,
    ));
    let cache_source = typed_source::<storage::FeedCacheError>(&cache)
        .expect("typed feed-cache source reaches boundary carrier");
    assert!(matches!(
        cache_source
            .source()
            .and_then(|source| source.downcast_ref()),
        Some(sqlx::Error::PoolClosed)
    ));

    let regeneration = jaunder::feed::handlers::map_regeneration_failure(
        jaunder::feed::regenerate::RegenerateError::Storage(Box::new(sqlx::Error::PoolClosed)),
    );
    let regeneration_source =
        typed_source::<jaunder::feed::regenerate::RegenerateError>(&regeneration)
            .expect("typed regeneration source reaches boundary carrier");
    assert!(matches!(
        regeneration_source
            .source()
            .and_then(|source| source.downcast_ref()),
        Some(sqlx::Error::PoolClosed)
    ));
}

#[apply(backends)]
#[tokio::test]
async fn handler_cache_read_failure_is_sanitized_and_reports_boundary_once(
    #[case] backend: Backend,
) {
    let TestEnv { state, base } = backend.setup().await;
    let app = make_app(&state, &base);
    base.close_pool().await;
    let request = Request::builder()
        .method("GET")
        .uri("/feed.rss")
        .body(Body::empty())
        .expect("build request");

    let (response, event) = crate::assert_error_signal!(
        async { app.oneshot(request).await.expect("request") },
        event = "server function failed",
        event_kind = "Storage",
        event_class = "Bug",
        metric_kind = "storage",
        metric_class = "bug",
        disposition = "boundary",
        context = ""
    );

    assert_eq!(response.status(), StatusCode::INTERNAL_SERVER_ERROR);
    let body = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .expect("read body");
    assert!(body.is_empty(), "cache failure body is sanitized: {body:?}");
    assert!(
        event.contains("pool"),
        "typed cache source reaches event: {event}"
    );
}

#[apply(backends)]
#[tokio::test]
async fn handler_regeneration_failure_is_sanitized_and_reports_boundary_once(
    #[case] backend: Backend,
) {
    let TestEnv { state, base } = backend.setup().await;
    let mut cache = storage::MockFeedCacheStorage::new();
    cache.expect_get().times(1).return_once(|_| Ok(None));
    let state = with_feed_cache(&state, Arc::new(cache));
    let app = make_app(&state, &base);
    base.close_pool().await;
    let request = Request::builder()
        .method("GET")
        .uri("/feed.rss")
        .body(Body::empty())
        .expect("build request");

    let (response, event) = crate::assert_error_signal!(
        async { app.oneshot(request).await.expect("request") },
        event = "server function failed",
        event_kind = "Storage",
        event_class = "Bug",
        metric_kind = "storage",
        metric_class = "bug",
        disposition = "boundary",
        context = ""
    );

    assert_eq!(response.status(), StatusCode::INTERNAL_SERVER_ERROR);
    let body = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .expect("read body");
    assert!(
        body.is_empty(),
        "regeneration failure body is sanitized: {body:?}"
    );
    assert!(
        event.contains("pool"),
        "typed regeneration source reaches event: {event}"
    );
}
