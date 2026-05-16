mod helpers;

use std::sync::Arc;

use axum::{
    body::Body,
    http::{header, Request, StatusCode},
};
use chrono::Utc;
use common::storage::{CreatePostInput, PostFormat};
use tempfile::TempDir;
use tower::ServiceExt;
use web::tags::TagSummary;

use helpers::{ensure_server_fns_registered, test_options, test_state};

async fn post_json(
    state: Arc<jaunder::storage::AppState>,
    uri: &str,
    body: serde_json::Value,
) -> (StatusCode, String) {
    ensure_server_fns_registered();

    let request = Request::builder()
        .method("POST")
        .uri(uri)
        .header(header::CONTENT_TYPE, "application/json")
        .body(Body::from(body.to_string()))
        .unwrap();

    let app = jaunder::create_router(test_options(), state, true, helpers::tmp_storage_path());
    let response = app.oneshot(request).await.unwrap();

    let status = response.status();
    let bytes = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .unwrap();
    let body_str = String::from_utf8(bytes.to_vec()).unwrap();

    (status, body_str)
}

async fn seed_user_and_tagged_post(
    state: &Arc<jaunder::storage::AppState>,
    username: &str,
    slug: &str,
    tags: &[&str],
) -> i64 {
    let user_id = state
        .users
        .create_user(
            &username.parse().unwrap(),
            &"password123".parse().unwrap(),
            None,
            false,
        )
        .await
        .expect("create_user failed");
    let post_id = state
        .posts
        .create_post(&CreatePostInput {
            user_id,
            title: Some(format!("Post {slug}")),
            slug: slug.parse().unwrap(),
            body: format!("body {slug}"),
            format: PostFormat::Markdown,
            rendered_html: format!("<p>body {slug}</p>"),
            published_at: Some(Utc::now()),
        })
        .await
        .expect("create_post failed");
    for display in tags {
        state.posts.tag_post(post_id, display).await.unwrap();
    }
    post_id
}

#[tokio::test]
async fn list_tags_returns_empty_when_no_tags() {
    let base = TempDir::new().unwrap();
    let state = test_state(&base).await;

    let (status, body) = post_json(state, "/api/list_tags", serde_json::json!({})).await;

    assert_eq!(status, StatusCode::OK, "body: {body}");
    let tags: Vec<TagSummary> = serde_json::from_str(&body).unwrap();
    assert!(tags.is_empty());
}

#[tokio::test]
async fn list_tags_returns_all_when_prefix_absent() {
    let base = TempDir::new().unwrap();
    let state = test_state(&base).await;
    seed_user_and_tagged_post(
        &state,
        "alice",
        "post-1",
        &["Rust", "rust-lang", "performance", "web"],
    )
    .await;

    let (status, body) = post_json(
        Arc::clone(&state),
        "/api/list_tags",
        serde_json::json!({ "prefix": null, "limit": null }),
    )
    .await;

    assert_eq!(status, StatusCode::OK, "body: {body}");
    let tags: Vec<TagSummary> = serde_json::from_str(&body).unwrap();
    let slugs: Vec<&str> = tags.iter().map(|t| t.slug.as_str()).collect();
    assert_eq!(slugs, vec!["performance", "rust", "rust-lang", "web"]);
    // display currently mirrors the slug (M5's display-casing wiring lands in
    // tags.5 alongside the tags param on create/update).
    for tag in &tags {
        assert_eq!(tag.display, tag.slug);
    }
}

#[tokio::test]
async fn list_tags_filters_by_prefix_case_insensitive() {
    let base = TempDir::new().unwrap();
    let state = test_state(&base).await;
    seed_user_and_tagged_post(
        &state,
        "bob",
        "post-2",
        &["rust", "rust-lang", "javascript", "web"],
    )
    .await;

    let (status, body) = post_json(
        Arc::clone(&state),
        "/api/list_tags",
        serde_json::json!({ "prefix": "RUST" }),
    )
    .await;

    assert_eq!(status, StatusCode::OK, "body: {body}");
    let tags: Vec<TagSummary> = serde_json::from_str(&body).unwrap();
    let slugs: Vec<&str> = tags.iter().map(|t| t.slug.as_str()).collect();
    assert_eq!(slugs, vec!["rust", "rust-lang"]);
}

#[tokio::test]
async fn list_tags_clamps_limit_to_max() {
    let base = TempDir::new().unwrap();
    let state = test_state(&base).await;
    let post = seed_user_and_tagged_post(&state, "carol", "post-3", &[]).await;
    // 60 tags — exceeds the MAX_TAG_LIMIT of 50.
    for n in 0..60 {
        state
            .posts
            .tag_post(post, &format!("tag{n:02}"))
            .await
            .unwrap();
    }

    let (status, body) = post_json(
        Arc::clone(&state),
        "/api/list_tags",
        serde_json::json!({ "limit": 1000 }),
    )
    .await;

    assert_eq!(status, StatusCode::OK, "body: {body}");
    let tags: Vec<TagSummary> = serde_json::from_str(&body).unwrap();
    assert_eq!(tags.len(), 50, "limit must be clamped to MAX_TAG_LIMIT");
}

#[tokio::test]
async fn list_tags_uses_default_limit_when_unspecified() {
    let base = TempDir::new().unwrap();
    let state = test_state(&base).await;
    let post = seed_user_and_tagged_post(&state, "dan", "post-4", &[]).await;
    for n in 0..20 {
        state
            .posts
            .tag_post(post, &format!("tag{n:02}"))
            .await
            .unwrap();
    }

    let (status, body) =
        post_json(Arc::clone(&state), "/api/list_tags", serde_json::json!({})).await;

    assert_eq!(status, StatusCode::OK, "body: {body}");
    let tags: Vec<TagSummary> = serde_json::from_str(&body).unwrap();
    assert_eq!(tags.len(), 10, "DEFAULT_TAG_LIMIT is 10");
}
