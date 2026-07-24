use axum::{
    body::Body,
    http::{header, Request, StatusCode},
};
use common::tag::TagLabel;
use rstest::*;
use rstest_reuse::*;
use tower::ServiceExt;

use crate::helpers::{
    atompub_xml, body_string, create_user_and_session, make_app, setup_with_base_url,
};
use storage::test_support::{backends, Backend, TestEnv};

#[apply(backends)]
#[tokio::test]
async fn service_document_returns_200_with_app_password(#[case] backend: Backend) {
    let TestEnv { state, base } = setup_with_base_url(backend).await;
    let session = create_user_and_session(&state, "alice").await;
    // Give the user a tagged post so the service document's category list is
    // non-empty (exercises the tag-collection path in `service_document`).
    let post = storage::perform_post_creation(
        state.posts.as_ref(),
        storage::PostCreation {
            user_id: session.user_id,
            body: "a tagged post".into(),
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
    state
        .posts
        .tag_post(post.post_id, &"rust".parse::<TagLabel>().unwrap())
        .await
        .unwrap();
    let app = make_app(state, &base);

    let response = app
        .oneshot(atompub_xml(
            "GET",
            "/atompub/service",
            "alice",
            &session.token,
            None,
        ))
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
    assert!(body.contains("https://example.com/atompub/alice/posts"));
    assert!(body.contains("https://example.com/atompub/alice/media"));
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
    let session = create_user_and_session(&state, "alice").await;
    let app = make_app(state, &base);

    // Correct token, but the Basic username does not match the session's user.
    let response = app
        .oneshot(atompub_xml(
            "GET",
            "/atompub/service",
            "mallory",
            &session.token,
            None,
        ))
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
