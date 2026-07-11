use std::sync::Arc;

use axum::{
    body::Body,
    http::{header, Request, StatusCode},
};
use base64::Engine as _;
use rstest::*;
use rstest_reuse::*;
use tempfile::TempDir;
use tower::ServiceExt;

use crate::helpers::{
    backends, ensure_server_fns_registered, noop_mailer, test_options, Backend, TestEnv,
};

fn make_app(state: Arc<storage::AppState>, storage: &TempDir) -> axum::Router {
    ensure_server_fns_registered();
    let storage_path = storage.path().to_path_buf();
    jaunder::create_router(test_options(), state, noop_mailer(), false, storage_path)
}

fn basic_header(username: &str, password: &str) -> String {
    let raw = format!("{username}:{password}");
    let encoded = base64::engine::general_purpose::STANDARD.encode(raw);
    format!("Basic {encoded}")
}

async fn body_string(response: axum::response::Response) -> String {
    let bytes = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .unwrap();
    String::from_utf8(bytes.to_vec()).unwrap()
}

#[apply(backends)]
#[tokio::test]
async fn service_document_returns_200_with_app_password(#[case] backend: Backend) {
    let TestEnv { state, base } = backend.setup().await;
    let user_id = state
        .users
        .create_user(
            &"alice".parse().unwrap(),
            &"password123".parse().unwrap(),
            None,
            false,
        )
        .await
        .unwrap();
    let token = state
        .sessions
        .create_session(user_id, "MarsEdit")
        .await
        .unwrap();
    // Give the user a tagged post so the service document's category list is
    // non-empty (exercises the tag-collection path in `service_document`).
    let post = storage::perform_post_creation(
        state.posts.as_ref(),
        storage::PostCreation {
            user_id,
            body: "a tagged post".to_string(),
            title: Some("Tagged"),
            format: storage::PostFormat::Markdown,
            slug_override: None,
            published_at: Some(chrono::Utc::now()),
            max_attempts: 100,
            summary: None,
            audiences: vec![common::visibility::AudienceTarget::Public],
            idempotency_key: None,
        },
    )
    .await
    .unwrap();
    state.posts.tag_post(post.post_id, "rust").await.unwrap();
    let app = make_app(state, &base);

    let response = app
        .oneshot(
            Request::builder()
                .uri("/atompub/service")
                .header(header::AUTHORIZATION, basic_header("alice", &token))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);
    let ctype = response
        .headers()
        .get(header::CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .unwrap_or_default()
        .to_string();
    assert!(
        ctype.contains("application/atomsvc+xml"),
        "content-type was {ctype}"
    );
    let body = body_string(response).await;
    assert!(body.contains("app:service"));
    assert!(body.contains("/atompub/alice/posts"));
    assert!(body.contains("/atompub/alice/media"));
    assert!(body.contains("image/webp"));
    // The tagged post surfaces as an inline category in the posts collection.
    assert!(body.contains("term=\"rust\""), "categories missing: {body}");
    // Capability discovery (ADR-0023): the service document advertises the
    // Jaunder wire extensions this server understands.
    assert!(body.contains("j:extension"), "j:extension missing: {body}");
    assert!(
        body.contains("features=\"format-media-type slug\""),
        "extension features missing: {body}"
    );
}

#[apply(backends)]
#[tokio::test]
async fn service_document_rejects_basic_username_mismatch(#[case] backend: Backend) {
    let TestEnv { state, base } = backend.setup().await;
    let user_id = state
        .users
        .create_user(
            &"alice".parse().unwrap(),
            &"password123".parse().unwrap(),
            None,
            false,
        )
        .await
        .unwrap();
    let token = state
        .sessions
        .create_session(user_id, "MarsEdit")
        .await
        .unwrap();
    let app = make_app(state, &base);

    // Correct token, but the Basic username does not match the session's user.
    let response = app
        .oneshot(
            Request::builder()
                .uri("/atompub/service")
                .header(header::AUTHORIZATION, basic_header("mallory", &token))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
}

#[apply(backends)]
#[tokio::test]
async fn service_document_requires_authentication(#[case] backend: Backend) {
    let TestEnv { state, base } = backend.setup().await;
    let app = make_app(state, &base);

    let response = app
        .oneshot(
            Request::builder()
                .uri("/atompub/service")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
}
