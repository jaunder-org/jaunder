use std::sync::Arc;

use axum::{
    body::Body,
    http::{header, Request, StatusCode},
};
use base64::Engine as _;
use tempfile::TempDir;
use tower::ServiceExt;

use rstest::*;
use rstest_reuse::*;

use storage::test_support::{backends, backends_matrix, Backend, TestEnv};

use crate::helpers::{body_string, make_app};

const PNG: &[u8] = &[
    0x89, 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A, 0x00, 0x00, 0x00, 0x0D, 0x49, 0x48, 0x44, 0x52,
    0x00, 0x00, 0x00, 0x01, 0x00, 0x00, 0x00, 0x01, 0x08, 0x06, 0x00, 0x00, 0x00, 0x1F, 0x15, 0xC4,
    0x89, 0x00, 0x00, 0x00, 0x0A, 0x49, 0x44, 0x41, 0x54, 0x78, 0x9C, 0x63, 0x00, 0x01, 0x00, 0x00,
    0x05, 0x00, 0x01, 0x0D, 0x0A, 0x2D, 0xB4, 0x00, 0x00, 0x00, 0x00, 0x49, 0x45, 0x4E, 0x44, 0xAE,
    0x42, 0x60, 0x82,
];

fn basic_header(username: &str, password: &str) -> String {
    let raw = format!("{username}:{password}");
    let encoded = base64::engine::general_purpose::STANDARD.encode(raw);
    format!("Basic {encoded}")
}

#[apply(backends)]
#[tokio::test]
async fn upload_returns_201_and_media_link_entry(#[case] backend: Backend) {
    let TestEnv { state, base: _base } = backend.setup().await;
    let token = seed_alice(&state).await;

    let storage = TempDir::new().unwrap();
    let app = make_app(Arc::clone(&state), &storage);

    let response = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/atompub/alice/media")
                .header(header::AUTHORIZATION, basic_header("alice", &token))
                .header(header::CONTENT_TYPE, "image/png")
                .header("slug", "pic.png")
                .body(Body::from(PNG))
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::CREATED);

    let loc = response
        .headers()
        .get(header::LOCATION)
        .unwrap()
        .to_str()
        .unwrap()
        .to_string();

    assert!(loc.starts_with("/atompub/alice/media/"));

    let body = body_string(response).await;
    assert!(body.contains("rel=\"edit-media\""), "body: {body}");
    assert!(body.contains("type=\"image/png\""), "body: {body}");
    assert!(body.contains("/media/upload/"), "body: {body}");
}

#[apply(backends)]
#[tokio::test]
async fn reupload_identical_returns_200(#[case] backend: Backend) {
    let TestEnv { state, base: _base } = backend.setup().await;
    let token = seed_alice(&state).await;

    let storage = TempDir::new().unwrap();
    let app = make_app(Arc::clone(&state), &storage);

    let _resp1 = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/atompub/alice/media")
                .header(header::AUTHORIZATION, basic_header("alice", &token))
                .header(header::CONTENT_TYPE, "image/png")
                .header("slug", "pic.png")
                .body(Body::from(PNG))
                .unwrap(),
        )
        .await
        .unwrap();

    // Second upload (identical)
    let resp2 = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/atompub/alice/media")
                .header(header::AUTHORIZATION, basic_header("alice", &token))
                .header(header::CONTENT_TYPE, "image/png")
                .header("slug", "pic.png")
                .body(Body::from(PNG))
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(resp2.status(), StatusCode::OK);
}

#[apply(backends)]
#[tokio::test]
async fn get_media_member_returns_entry(#[case] backend: Backend) {
    let TestEnv { state, base: _base } = backend.setup().await;
    let token = seed_alice(&state).await;

    let storage = TempDir::new().unwrap();
    let app = make_app(Arc::clone(&state), &storage);

    let resp = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/atompub/alice/media")
                .header(header::AUTHORIZATION, basic_header("alice", &token))
                .header(header::CONTENT_TYPE, "image/png")
                .header("slug", "pic.png")
                .body(Body::from(PNG))
                .unwrap(),
        )
        .await
        .unwrap();

    let loc = resp
        .headers()
        .get(header::LOCATION)
        .unwrap()
        .to_str()
        .unwrap()
        .to_string();

    let get_resp = app
        .oneshot(
            Request::builder()
                .method("GET")
                .uri(&loc)
                .header(header::AUTHORIZATION, basic_header("alice", &token))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(get_resp.status(), StatusCode::OK);
    let body = body_string(get_resp).await;
    assert!(body.contains("rel=\"edit-media\""), "body: {body}");
}

#[apply(backends)]
#[tokio::test]
async fn get_unknown_media_returns_404(#[case] backend: Backend) {
    let TestEnv { state, base: _base } = backend.setup().await;
    let token = seed_alice(&state).await;

    let storage = TempDir::new().unwrap();
    let app = make_app(Arc::clone(&state), &storage);

    let response = app
        .oneshot(
            Request::builder()
                .method("GET")
                .uri("/atompub/alice/media/deadbeef/none.png")
                .header(header::AUTHORIZATION, basic_header("alice", &token))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::NOT_FOUND);
}

#[apply(backends)]
#[tokio::test]
async fn delete_media_member_returns_204_then_404(#[case] backend: Backend) {
    let TestEnv { state, base: _base } = backend.setup().await;
    let token = seed_alice(&state).await;

    let storage = TempDir::new().unwrap();
    let app = make_app(Arc::clone(&state), &storage);

    let resp = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/atompub/alice/media")
                .header(header::AUTHORIZATION, basic_header("alice", &token))
                .header(header::CONTENT_TYPE, "image/png")
                .header("slug", "pic.png")
                .body(Body::from(PNG))
                .unwrap(),
        )
        .await
        .unwrap();

    let loc = resp
        .headers()
        .get(header::LOCATION)
        .unwrap()
        .to_str()
        .unwrap()
        .to_string();

    let del_resp = app
        .clone()
        .oneshot(
            Request::builder()
                .method("DELETE")
                .uri(&loc)
                .header(header::AUTHORIZATION, basic_header("alice", &token))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(del_resp.status(), StatusCode::NO_CONTENT);

    // Second delete (should be 404)
    let del_resp2 = app
        .oneshot(
            Request::builder()
                .method("DELETE")
                .uri(&loc)
                .header(header::AUTHORIZATION, basic_header("alice", &token))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(del_resp2.status(), StatusCode::NOT_FOUND);
}

#[apply(backends)]
#[tokio::test]
async fn upload_forbids_other_user(#[case] backend: Backend) {
    let TestEnv { state, base: _base } = backend.setup().await;
    let token = seed_alice(&state).await;

    let storage = TempDir::new().unwrap();
    let app = make_app(Arc::clone(&state), &storage);

    let response = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/atompub/bob/media")
                .header(header::AUTHORIZATION, basic_header("alice", &token))
                .header(header::CONTENT_TYPE, "image/png")
                .header("slug", "pic.png")
                .body(Body::from(PNG))
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::FORBIDDEN);
}

/// Seeds a user named `alice` and returns the session token.
async fn seed_alice(state: &Arc<storage::AppState>) -> String {
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
    state
        .sessions
        .create_session(user_id, "MarsEdit")
        .await
        .unwrap()
        .as_ref()
        .to_string()
}

#[apply(backends)]
#[tokio::test]
async fn upload_rejects_empty_slug(#[case] backend: Backend) {
    let TestEnv { state, base: _base } = backend.setup().await;
    let token = seed_alice(&state).await;
    let storage = TempDir::new().unwrap();
    let app = make_app(state, &storage);

    // ".." sanitizes to an empty filename.
    let response = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/atompub/alice/media")
                .header(header::AUTHORIZATION, basic_header("alice", &token))
                .header(header::CONTENT_TYPE, "image/png")
                .header("slug", "..")
                .body(Body::from(PNG))
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
}

// Shape B — accessing another user's media member is forbidden regardless of
// method. Identical setup (alice authenticated, bob's resource) + assertion;
// only the HTTP method varies.
#[apply(backends_matrix)]
#[case::get("GET")]
#[case::delete("DELETE")]
#[tokio::test]
async fn member_forbids_other_user(backend: Backend, #[case] method: &str) {
    let TestEnv { state, base: _base } = backend.setup().await;
    let token = seed_alice(&state).await;
    let storage = TempDir::new().unwrap();
    let app = make_app(state, &storage);

    let response = app
        .oneshot(
            Request::builder()
                .method(method)
                .uri("/atompub/bob/media/deadbeef/pic.png")
                .header(header::AUTHORIZATION, basic_header("alice", &token))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::FORBIDDEN);
}
