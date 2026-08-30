use axum::http::StatusCode;
use chrono::Datelike;
use common::seed::AuthoredPost;
use common::tag::TagLabel;
use web::posts::SavedPost;

use rstest::*;
use rstest_reuse::*;

use crate::helpers::create_user_and_session;
use storage::test_support::{Backend, TestEnv, backends};

use super::fixtures::{create_post_json, get_post_form};

#[apply(backends)]
#[tokio::test]
async fn get_post_returns_published_post(#[case] backend: Backend) {
    let TestEnv { state, base: _base } = backend.setup().await;
    let session = create_user_and_session(&state).await;
    let cookie = session.cookie();

    let (status, body) = create_post_json(
        &state,
        "# Permalink

**bold**",
        "markdown",
        None,
        true,
        Some(&cookie),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "create body: {body}");
    let created: SavedPost = serde_json::from_str(&body).unwrap();

    let record = state
        .posts
        .get_post_by_id(
            created.post_id,
            &common::visibility::ViewerIdentity::Anonymous,
        )
        .await
        .unwrap()
        .expect("post should exist");
    let published_at = record
        .published_at
        .expect("published post should have published_at");
    let (status, body) = get_post_form(
        &state,
        &session.username,
        published_at.value().year(),
        published_at.value().month(),
        published_at.value().day(),
        &created.slug,
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "body: {body}");
    assert!(body.contains("Permalink"));
    assert!(body.contains("rendered_html"));
    assert!(body.contains("published_at"));
}

#[apply(backends)]
#[tokio::test]
async fn get_post_rejects_invalid_username(#[case] backend: Backend) {
    let TestEnv { state, base: _base } = backend.setup().await;

    let (status, body) = get_post_form(&state, "Invalid Name", 2024, 1, 1, "missing", None).await;

    assert_eq!(status, StatusCode::INTERNAL_SERVER_ERROR, "body: {body}");
    assert!(body.contains("username"), "body: {body}");
}

#[apply(backends)]
#[tokio::test]
async fn get_post_rejects_invalid_slug(#[case] backend: Backend) {
    let TestEnv { state, base: _base } = backend.setup().await;

    let (status, body) = get_post_form(&state, "author", 2024, 1, 1, "Invalid Slug", None).await;

    assert_eq!(status, StatusCode::INTERNAL_SERVER_ERROR, "body: {body}");
    assert!(body.contains("slug"), "body: {body}");
}

#[apply(backends)]
#[tokio::test]
async fn get_post_returns_not_found_for_missing_post(#[case] backend: Backend) {
    let TestEnv { state, base: _base } = backend.setup().await;

    let (status, body) = get_post_form(&state, "author", 2024, 1, 1, "missing", None).await;

    assert_eq!(status, StatusCode::NOT_FOUND, "body: {body}");
    assert!(body.contains("Post not found"), "body: {body}");
}

#[apply(backends)]
#[tokio::test]
async fn get_post_carries_tags(#[case] backend: Backend) {
    use chrono::Datelike;

    let TestEnv { state, base: _base } = backend.setup().await;
    let session = create_user_and_session(&state).await;
    let cookie = session.cookie();

    let (status, body) = create_post_json(
        &state,
        "# Tagged Post\n\nbody",
        "markdown",
        None,
        true,
        Some(&cookie),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "create body: {body}");
    let created: SavedPost = serde_json::from_str(&body).unwrap();

    storage::test_support::set_post_tags_confirmed(
        &state.write_scope,
        std::sync::Arc::clone(&state.posts),
        created.post_id,
        session.user_id,
        &["Performance".parse::<TagLabel>().unwrap()],
    )
    .await
    .unwrap();

    let published_at = state
        .posts
        .get_post_by_id(
            created.post_id,
            &common::visibility::ViewerIdentity::Anonymous,
        )
        .await
        .unwrap()
        .unwrap()
        .published_at
        .unwrap();

    let (status, body) = get_post_form(
        &state,
        &session.username,
        published_at.value().year(),
        published_at.value().month(),
        published_at.value().day(),
        &created.slug,
        Some(&cookie),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "get body: {body}");
    let response: AuthoredPost = serde_json::from_str(&body).unwrap();
    assert_eq!(response.post.tags.len(), 1);
    assert_eq!(response.post.tags[0].slug, "performance");
    assert_eq!(response.post.tags[0].display, "Performance");
}
