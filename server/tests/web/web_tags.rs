use common::ids::PostId;
use common::seed::TagSummary;
use common::tag::TagLabel;
use server_fn::ServerFn;
use std::sync::Arc;

use axum::http::StatusCode;

use rstest::*;
use rstest_reuse::*;

use crate::helpers::post_json;
use storage::test_support::{backends, Backend, SeedRawPost, SeedUser, TestEnv};

async fn seed_user_and_tagged_post(
    state: &Arc<storage::AppState>,
    slug: &str,
    tags: &[&str],
) -> PostId {
    let user_id = SeedUser::new().seed(state).await.user_id;
    SeedRawPost::new(user_id)
        .slug(slug)
        .tags(tags.iter().copied())
        .seed(state)
        .await
        .post_id
}

#[apply(backends)]
#[tokio::test]
async fn list_tags_returns_empty_when_no_tags(#[case] backend: Backend) {
    let TestEnv { state, base: _base } = backend.setup().await;

    let (status, body) = post_json(
        &state,
        <web::tags::List as ServerFn>::PATH,
        serde_json::json!({}),
        None,
    )
    .await;

    assert_eq!(status, StatusCode::OK, "body: {body}");
    let tags: Vec<TagSummary> = serde_json::from_str(&body).unwrap();
    assert!(tags.is_empty());
}

#[apply(backends)]
#[tokio::test]
async fn list_tags_returns_all_when_prefix_absent(#[case] backend: Backend) {
    let TestEnv { state, base: _base } = backend.setup().await;
    seed_user_and_tagged_post(
        &state,
        "post-1",
        &["Rust", "rust-lang", "performance", "web"],
    )
    .await;

    let (status, body) = post_json(
        &state,
        <web::tags::List as ServerFn>::PATH,
        serde_json::json!({ "prefix": null, "limit": null }),
        None,
    )
    .await;

    assert_eq!(status, StatusCode::OK, "body: {body}");
    let tags: Vec<TagSummary> = serde_json::from_str(&body).unwrap();
    let slugs: Vec<&str> = tags.iter().map(|t| t.slug.as_ref()).collect();
    assert_eq!(slugs, vec!["performance", "rust", "rust-lang", "web"]);
    // display currently mirrors the slug (M5's display-casing wiring lands in
    // tags.5 alongside the tags param on create/update).
    for tag in &tags {
        assert_eq!(tag.display, tag.slug.as_ref());
    }
}

#[apply(backends)]
#[tokio::test]
async fn list_tags_filters_by_prefix_case_insensitive(#[case] backend: Backend) {
    let TestEnv { state, base: _base } = backend.setup().await;
    seed_user_and_tagged_post(
        &state,
        "post-2",
        &["rust", "rust-lang", "javascript", "web"],
    )
    .await;

    let (status, body) = post_json(
        &state,
        <web::tags::List as ServerFn>::PATH,
        serde_json::json!({ "prefix": "RUST" }),
        None,
    )
    .await;

    assert_eq!(status, StatusCode::OK, "body: {body}");
    let tags: Vec<TagSummary> = serde_json::from_str(&body).unwrap();
    let slugs: Vec<&str> = tags.iter().map(|t| t.slug.as_ref()).collect();
    assert_eq!(slugs, vec!["rust", "rust-lang"]);
}

#[apply(backends)]
#[tokio::test]
async fn list_tags_rejects_out_of_range_limit(#[case] backend: Backend) {
    let TestEnv { state, base: _base } = backend.setup().await;

    // `limit=1000` is outside `PageSize`'s `1..=50`; the typed wire arg rejects it on
    // deserialization instead of coercing it down to the cap (#691). Mirrors
    // `list_my_media_rejects_out_of_range_limit`; the status is asserted only as
    // "not OK" because this endpoint is `input = Json` and that one is form-encoded.
    let (status, _body) = post_json(
        &state,
        <web::tags::List as ServerFn>::PATH,
        serde_json::json!({ "limit": 1000 }),
        None,
    )
    .await;

    assert_ne!(
        status,
        StatusCode::OK,
        "out-of-range tag limit must be rejected"
    );
}

#[apply(backends)]
#[tokio::test]
async fn list_tags_uses_default_limit_when_unspecified(#[case] backend: Backend) {
    let TestEnv { state, base: _base } = backend.setup().await;
    let post = seed_user_and_tagged_post(&state, "post-4", &[]).await;
    let labels: Vec<TagLabel> = (0..20)
        .map(|n| format!("tag{n:02}").parse().expect("valid tag label"))
        .collect();
    state.posts.set_post_tags(post, &labels).await.unwrap();

    let (status, body) = post_json(
        &state,
        <web::tags::List as ServerFn>::PATH,
        serde_json::json!({}),
        None,
    )
    .await;

    assert_eq!(status, StatusCode::OK, "body: {body}");
    let tags: Vec<TagSummary> = serde_json::from_str(&body).unwrap();
    assert_eq!(tags.len(), 10, "the default limit is 10");
}
