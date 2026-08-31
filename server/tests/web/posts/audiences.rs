use std::sync::Arc;

use axum::http::StatusCode;
use common::render::PostFormat;
use common::test_support::parse_post_body;
use common::visibility::{AudienceBase, AudienceSelection};
use server_fn::ServerFn;
use web::posts::{PostInputs, SavedPost};

use rstest::*;
use rstest_reuse::*;

use crate::helpers::{
    confirmed_mutation, create_post_json, create_user_and_session, post_form, post_json,
};
use storage::test_support::{Backend, TestEnv, backends};

// ── Audience-picker server fns ────────────────────────────────

/// Creates a user and returns a session cookie for the audience-picker tests.
async fn author_with_cookie(state: &Arc<storage::AppState>) -> String {
    user_with_cookie(state).await
}

/// Creates a user and returns a session cookie.
async fn user_with_cookie(state: &Arc<storage::AppState>) -> String {
    create_user_and_session(state).await.cookie()
}

#[apply(backends)]
#[tokio::test]
async fn default_audience_selection_returns_private_by_default(#[case] backend: Backend) {
    let TestEnv { state, base: _base } = backend.setup().await;
    let cookie = author_with_cookie(&state).await;

    let (status, body) = post_json(
        &state,
        <web::posts::GetDefaultAudienceSelection as ServerFn>::PATH,
        serde_json::json!({}),
        Some(&cookie),
    )
    .await;

    assert_eq!(status, StatusCode::OK, "body: {body}");
    let selection: AudienceSelection = serde_json::from_str(&body).unwrap();
    assert_eq!(selection.base, AudienceBase::Private);
    assert!(selection.named.is_empty());
}

#[apply(backends)]
#[tokio::test]
async fn default_audience_selection_rejects_unauthenticated(#[case] backend: Backend) {
    let TestEnv { state, base: _base } = backend.setup().await;

    let (status, body) = post_json(
        &state,
        <web::posts::GetDefaultAudienceSelection as ServerFn>::PATH,
        serde_json::json!({}),
        None,
    )
    .await;

    assert_eq!(status, StatusCode::INTERNAL_SERVER_ERROR, "body: {body}");
    assert!(body.contains("unauthorized"), "body: {body}");
}

#[apply(backends)]
#[tokio::test]
async fn post_audience_selection_returns_public_for_new_post(#[case] backend: Backend) {
    let TestEnv { state, base: _base } = backend.setup().await;
    let cookie = author_with_cookie(&state).await;

    let (status, body) = create_post_json(
        &state,
        PostInputs {
            publish: Some(true),
            ..PostInputs::new(parse_post_body("Hello"), PostFormat::Markdown)
        },
        Some(&cookie),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "create body: {body}");
    let created: SavedPost = confirmed_mutation(&body);

    let (status, body) = post_form(
        &state,
        <web::posts::GetAudienceSelection as ServerFn>::PATH,
        format!("post_id={}", created.post_id),
        Some(&cookie),
    )
    .await;

    assert_eq!(status, StatusCode::OK, "body: {body}");
    let selection: AudienceSelection = serde_json::from_str(&body).unwrap();
    // A post created with no audience field defaults to Public.
    assert_eq!(selection.base, AudienceBase::Public);
    assert!(selection.named.is_empty());
}

#[apply(backends)]
#[tokio::test]
async fn post_audience_selection_rejects_missing_post(#[case] backend: Backend) {
    let TestEnv { state, base: _base } = backend.setup().await;
    let cookie = author_with_cookie(&state).await;

    let (status, body) = post_form(
        &state,
        <web::posts::GetAudienceSelection as ServerFn>::PATH,
        "post_id=99999".to_string(),
        Some(&cookie),
    )
    .await;

    assert_eq!(status, StatusCode::NOT_FOUND, "body: {body}");
    assert!(body.contains("Post not found"), "body: {body}");
}

#[apply(backends)]
#[tokio::test]
async fn post_audience_selection_rejects_non_owner(#[case] backend: Backend) {
    let TestEnv { state, base: _base } = backend.setup().await;
    let author_cookie = user_with_cookie(&state).await;
    let other_cookie = user_with_cookie(&state).await;

    let (status, body) = create_post_json(
        &state,
        PostInputs {
            publish: Some(true),
            ..PostInputs::new(parse_post_body("Hello"), PostFormat::Markdown)
        },
        Some(&author_cookie),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "create body: {body}");
    let created: SavedPost = confirmed_mutation(&body);

    // A different user must not learn another author's targeting.
    let (status, body) = post_form(
        &state,
        <web::posts::GetAudienceSelection as ServerFn>::PATH,
        format!("post_id={}", created.post_id),
        Some(&other_cookie),
    )
    .await;

    assert_eq!(status, StatusCode::NOT_FOUND, "body: {body}");
    assert!(body.contains("Post not found"), "body: {body}");
}
