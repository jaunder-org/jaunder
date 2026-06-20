#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::too_many_lines,
    clippy::similar_names,
    clippy::items_after_statements,
    clippy::unused_async
)]
#![allow(unused_macros)]

mod helpers;

use std::sync::Arc;

use axum::{
    body::Body,
    http::{header, Request, StatusCode},
};
use chrono::{Timelike, Utc};
use common::password::Password;
use common::slug::Slug;
use common::username::Username;
use tempfile::TempDir;
use tower::ServiceExt;

use rstest::*;
#[allow(clippy::single_component_path_imports)]
use rstest_reuse;
use rstest_reuse::*;

use helpers::{backends, ensure_server_fns_registered, test_options, Backend, TestEnv};
use storage::CreatePostInput;
use storage::PostFormat;

/// Build the router with a real temp storage directory.
async fn make_app(state: Arc<storage::AppState>, storage: &TempDir) -> axum::Router {
    ensure_server_fns_registered();
    let storage_path = storage.path().to_path_buf();
    std::fs::create_dir_all(storage_path.join("media").join("upload")).unwrap();
    std::fs::create_dir_all(storage_path.join("media").join("cached")).unwrap();
    std::fs::create_dir_all(storage_path.join("media").join("tmp")).unwrap();
    jaunder::create_router(
        test_options(),
        state,
        helpers::noop_mailer(),
        false,
        storage_path,
    )
}

#[apply(backends)]
#[tokio::test]
async fn handler_cache_miss_lazy_regens_and_returns_200_with_correct_content_type(
    #[case] backend: Backend,
) {
    let TestEnv { state, base } = backend.setup().await;
    let app = make_app(state.clone(), &base).await;

    // Create a user
    let username: Username = "alice".parse().expect("valid username");
    let password: Password = "password123".parse().expect("valid password");
    let user_id = state
        .users
        .create_user(&username, &password, None, false)
        .await
        .expect("create user");

    // Create a published post
    let now = Utc::now();
    state
        .posts
        .create_post(&CreatePostInput {
            user_id,
            title: Some("Test Post".to_string()),
            slug: "test-post".parse::<Slug>().expect("valid slug"),
            body: "Test body".to_string(),
            format: PostFormat::Markdown,
            rendered_html: "<p>Test body</p>".to_string(),
            published_at: Some(now),
            summary: None,
        })
        .await
        .expect("create post");

    // Request the feed
    let req = Request::builder()
        .method("GET")
        .uri("/~alice/feed.rss")
        .body(Body::empty())
        .expect("build request");

    let resp = app.clone().oneshot(req).await.expect("request");

    // Assert 200
    assert_eq!(resp.status(), StatusCode::OK, "should return 200");

    // Assert correct content type
    let content_type = resp
        .headers()
        .get(header::CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .expect("content-type header");
    assert_eq!(
        content_type, "application/rss+xml; charset=utf-8",
        "RSS content type"
    );

    // Assert body is non-empty
    let body = axum::body::to_bytes(resp.into_body(), usize::MAX)
        .await
        .expect("read body");
    assert!(!body.is_empty(), "response body should not be empty");

    // Assert ETag and Last-Modified headers are present
    let req = Request::builder()
        .method("GET")
        .uri("/~alice/feed.rss")
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

    // Assert feed was cached
    let cached = state
        .feed_cache
        .get("/~alice/feed.rss")
        .await
        .expect("get from cache")
        .expect("cache entry should exist");
    assert!(!cached.body.is_empty(), "cached body should not be empty");
}

#[apply(backends)]
#[tokio::test]
async fn handler_serves_site_tag_feed_with_200(#[case] backend: Backend) {
    let TestEnv { state, base } = backend.setup().await;
    let app = make_app(state.clone(), &base).await;

    // A tagged, published post so the site-tag surface has content.
    let username: Username = "frank".parse().expect("valid username");
    let password: Password = "password123".parse().expect("valid password");
    let user_id = state
        .users
        .create_user(&username, &password, None, false)
        .await
        .expect("create user");
    let now = Utc::now();
    let post_id = state
        .posts
        .create_post(&CreatePostInput {
            user_id,
            title: Some("Tagged Post".to_string()),
            slug: "tagged-post".parse::<Slug>().expect("valid slug"),
            body: "Test body".to_string(),
            format: PostFormat::Markdown,
            rendered_html: "<p>Test body</p>".to_string(),
            published_at: Some(now),
            summary: None,
        })
        .await
        .expect("create post");
    state
        .posts
        .tag_post(post_id, "rust")
        .await
        .expect("tag post");

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
    let app = make_app(state.clone(), &base).await;

    // Pre-populate the cache with a known body
    let known_body = "known feed body";
    let row = storage::FeedCacheRow {
        feed_url: "/~bob/feed.rss".to_string(),
        body: known_body.to_string(),
        etag: "known-etag".to_string(),
        content_type: "application/rss+xml; charset=utf-8".to_string(),
        updated_at: Utc::now(),
        generated_at: Utc::now(),
    };
    state.feed_cache.upsert(row).await.expect("upsert cache");

    // Request the feed
    let req = Request::builder()
        .method("GET")
        .uri("/~bob/feed.rss")
        .body(Body::empty())
        .expect("build request");

    let resp = app.clone().oneshot(req).await.expect("request");

    // Assert 200
    assert_eq!(resp.status(), StatusCode::OK, "should return 200");

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
async fn handler_if_none_match_returns_304(#[case] backend: Backend) {
    let TestEnv { state, base } = backend.setup().await;
    let app = make_app(state.clone(), &base).await;

    // Pre-populate the cache
    let etag = "test-etag-123";
    let row = storage::FeedCacheRow {
        feed_url: "/~charlie/feed.rss".to_string(),
        body: "feed body".to_string(),
        etag: etag.to_string(),
        content_type: "application/rss+xml; charset=utf-8".to_string(),
        updated_at: Utc::now(),
        generated_at: Utc::now(),
    };
    state.feed_cache.upsert(row).await.expect("upsert cache");

    // Request with If-None-Match header
    let req = Request::builder()
        .method("GET")
        .uri("/~charlie/feed.rss")
        .header(header::IF_NONE_MATCH, etag)
        .body(Body::empty())
        .expect("build request");

    let resp = app.oneshot(req).await.expect("request");

    // Assert 304
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
    let app = make_app(state.clone(), &base).await;

    // Pre-populate the cache with a known update time
    // Round to seconds to ensure RFC2822 conversion is lossless
    let update_time = Utc::now()
        .with_nanosecond(0)
        .expect("valid nanosecond value");
    let row = storage::FeedCacheRow {
        feed_url: "/~dave/feed.rss".to_string(),
        body: "feed body".to_string(),
        etag: "test-etag".to_string(),
        content_type: "application/rss+xml; charset=utf-8".to_string(),
        updated_at: update_time,
        generated_at: Utc::now(),
    };
    state.feed_cache.upsert(row).await.expect("upsert cache");

    // Request with If-Modified-Since set to the same time
    let req = Request::builder()
        .method("GET")
        .uri("/~dave/feed.rss")
        .header(header::IF_MODIFIED_SINCE, update_time.to_rfc2822())
        .body(Body::empty())
        .expect("build request");

    let resp = app.oneshot(req).await.expect("request");

    // Assert 304
    assert_eq!(
        resp.status(),
        StatusCode::NOT_MODIFIED,
        "should return 304 when If-Modified-Since matches"
    );
}

// These surfaces must 404 when the request targets something the canonical
// validators reject: an unknown extension, a tag with a leading hyphen, a
// username with a dot, or a user-tag whose tag segment is invalid. The handler
// must 404 rather than construct an invalid surface.
#[rstest]
#[case::unknown_extension("/~alice/feed.xml")]
#[case::invalid_tag("/tags/-rust/feed.rss")]
#[case::invalid_username("/~al.ice/feed.rss")]
#[case::invalid_user_tag("/~alice/tags/-rust/feed.rss")]
#[tokio::test]
async fn handler_rejects_invalid_request_with_404(
    #[values(Backend::Sqlite, Backend::Postgres)] backend: Backend,
    #[case] uri: &str,
) {
    let TestEnv { state, base } = backend.setup().await;
    let app = make_app(state, &base).await;

    let req = Request::builder()
        .method("GET")
        .uri(uri)
        .body(Body::empty())
        .expect("build request");

    let resp = app.oneshot(req).await.expect("request");

    assert_eq!(
        resp.status(),
        StatusCode::NOT_FOUND,
        "should return 404 for a request the canonical validator rejects: {uri}"
    );
}

#[apply(backends)]
#[tokio::test]
async fn handler_returns_correct_content_type_per_format(#[case] backend: Backend) {
    let TestEnv { state, base } = backend.setup().await;

    // Create a user with one post
    let username: Username = "eve".parse().expect("valid username");
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
            body: "Test body".to_string(),
            format: PostFormat::Markdown,
            rendered_html: "<p>Test body</p>".to_string(),
            published_at: Some(now),
            summary: None,
        })
        .await
        .expect("create post");

    let test_cases = [
        ("rss", "application/rss+xml; charset=utf-8"),
        ("atom", "application/atom+xml; charset=utf-8"),
        ("json", "application/feed+json"),
    ];

    for (ext, expected_content_type) in &test_cases {
        let app = make_app(state.clone(), &base).await;
        let req = Request::builder()
            .method("GET")
            .uri(format!("/~eve/feed.{ext}"))
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
