use axum::{
    body::Body,
    http::{header, Request, StatusCode},
};
use common::tag::TagLabel;
use rstest::*;
use rstest_reuse::*;
use tower::ServiceExt;

use crate::helpers::{
    atompub_at, atompub_xml, body_string, create_user_and_session, make_app, setup_with_base_url,
};
use storage::test_support::{backends, Backend, TestEnv};

#[apply(backends)]
#[tokio::test]
async fn service_document_returns_200_with_app_password(#[case] backend: Backend) {
    let TestEnv { state, base } = setup_with_base_url(backend).await;
    let session = create_user_and_session(&state).await;
    let name: &str = &session.username;
    // Give the user a tagged post so the service document's category list is
    // non-empty (exercises the tag-collection path in `service_document`).
    let post = session.seed_post().seed(&state).await;
    state
        .posts
        .tag_post(post.post_id, &"rust".parse::<TagLabel>().unwrap())
        .await
        .unwrap();
    let app = make_app(&state, &base);

    let response = app
        .oneshot(
            atompub_at(&session, "GET", "/atompub/service")
                .body(Body::empty())
                .expect("failed to build atompub GET request"),
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
    assert!(body.contains(&format!("https://example.com/atompub/{name}/posts")));
    assert!(body.contains(&format!("https://example.com/atompub/{name}/media")));
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
    let session = create_user_and_session(&state).await;
    let app = make_app(&state, &base);

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
    let app = make_app(&state, &base);

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
