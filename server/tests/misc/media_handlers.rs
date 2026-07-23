use std::sync::Arc;

use axum::{
    body::Body,
    http::{header, Request, StatusCode},
};
use tempfile::TempDir;
use tower::ServiceExt;

use rstest::*;
use rstest_reuse::*;

use common::ids::UserId;
use storage::test_support::{backends, backends_matrix, Backend, TestEnv};

use crate::helpers::{make_app, post_multipart, session_cookie, MultipartFile};

// ---------------------------------------------------------------------------
// Serve tests
// ---------------------------------------------------------------------------

#[apply(backends)]
#[tokio::test]
async fn serve_returns_200_with_cache_headers(#[case] backend: Backend) {
    let TestEnv { state, base: _base } = backend.setup().await;

    let user_id = state
        .users
        .create_user(
            &"server".parse().unwrap(),
            &"password123".parse().unwrap(),
            None,
            false,
        )
        .await
        .unwrap();
    let token = state
        .sessions
        .create_session(user_id, "test session")
        .await
        .unwrap();
    let cookie = session_cookie(&token);

    let storage = TempDir::new().unwrap();

    // Upload via the `upload_media` server fn so a file lands on `storage`'s disk;
    // the fn returns 200 with the bare `UploadResponse` JSON.
    let (status, body) = post_multipart(
        Arc::clone(&state),
        &storage,
        "/api/upload_media",
        MultipartFile {
            filename: "serve_test.png",
            content_type: "image/png",
            bytes: b"PNG_CONTENT_HERE",
        },
        Some(&cookie),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "upload must succeed");

    let upload_json: serde_json::Value = serde_json::from_str(&body).unwrap();
    let url = upload_json["url"].as_str().unwrap().to_owned();

    // A fresh app over the SAME storage serves the persisted file.
    let app = make_app(Arc::clone(&state), &storage);

    let serve_response = app
        .oneshot(
            Request::builder()
                .method("GET")
                .uri(&url)
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(serve_response.status(), StatusCode::OK);
    let cache_control = serve_response
        .headers()
        .get(header::CACHE_CONTROL)
        .and_then(|v| v.to_str().ok())
        .unwrap_or("");
    assert!(
        cache_control.contains("max-age=31536000"),
        "expected immutable cache-control, got: {cache_control}"
    );
}

// Shape B — both URIs are served by the same handler and must 404; identical
// setup + assertion, only the request URI varies.
#[apply(backends_matrix)]
#[case::missing_file(
    "/media/upload/ab/cd/abcdef1234567890abcdef1234567890abcdef1234567890abcdef1234567890/missing.jpg"
)]
#[case::invalid_source("/media/invalid/ab/cd/abcdef1234/file.jpg")]
#[tokio::test]
async fn serve_returns_404(backend: Backend, #[case] uri: &str) {
    let TestEnv { state, base: _base } = backend.setup().await;

    let storage = TempDir::new().unwrap();
    let app = make_app(Arc::clone(&state), &storage);

    let response = app
        .oneshot(
            Request::builder()
                .method("GET")
                .uri(uri)
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::NOT_FOUND);
}

#[apply(backends)]
#[tokio::test]
async fn serve_returns_304_on_if_none_match(#[case] backend: Backend) {
    let TestEnv { state, base: _base } = backend.setup().await;

    let user_id = state
        .users
        .create_user(
            &"etagger".parse().unwrap(),
            &"password123".parse().unwrap(),
            None,
            false,
        )
        .await
        .unwrap();
    let token = state
        .sessions
        .create_session(user_id, "test session")
        .await
        .unwrap();
    let cookie = session_cookie(&token);

    let storage = TempDir::new().unwrap();

    // Upload via the `upload_media` server fn so a file lands on `storage`'s disk.
    let (status, body) = post_multipart(
        Arc::clone(&state),
        &storage,
        "/api/upload_media",
        MultipartFile {
            filename: "etag_test.png",
            content_type: "image/png",
            bytes: b"PNG_DATA",
        },
        Some(&cookie),
    )
    .await;
    assert_eq!(status, StatusCode::OK);

    let upload_json: serde_json::Value = serde_json::from_str(&body).unwrap();
    let url = upload_json["url"].as_str().unwrap().to_owned();
    let sha256 = upload_json["sha256"].as_str().unwrap().to_owned();
    let etag = format!("\"{sha256}\"");

    let app = make_app(Arc::clone(&state), &storage);
    let resp = app
        .oneshot(
            Request::builder()
                .method("GET")
                .uri(&url)
                .header(header::IF_NONE_MATCH, &etag)
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::NOT_MODIFIED);
}

// ---------------------------------------------------------------------------
// Proxy tests
// ---------------------------------------------------------------------------

#[apply(backends)]
#[tokio::test]
async fn proxy_requires_auth(#[case] backend: Backend) {
    let TestEnv { state, base: _base } = backend.setup().await;

    let storage = TempDir::new().unwrap();
    let app = make_app(Arc::clone(&state), &storage);

    let response = app
        .oneshot(
            Request::builder()
                .method("GET")
                .uri("/media/proxy?url=http%3A%2F%2Fexample.com%2Fimage.jpg&user_id=1")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
}

#[apply(backends)]
#[tokio::test]
async fn proxy_redirects_authenticated(#[case] backend: Backend) {
    let TestEnv { state, base: _base } = backend.setup().await;

    let user_id = state
        .users
        .create_user(
            &"proxyuser".parse().unwrap(),
            &"password123".parse().unwrap(),
            None,
            false,
        )
        .await
        .unwrap();
    let token = state
        .sessions
        .create_session(user_id, "test session")
        .await
        .unwrap();
    let cookie = session_cookie(&token);

    let storage = TempDir::new().unwrap();
    let app = make_app(Arc::clone(&state), &storage);

    let url = format!("/media/proxy?url=http%3A%2F%2Fexample.com%2Fimage.jpg&user_id={user_id}");

    let response = app
        .oneshot(
            Request::builder()
                .method("GET")
                .uri(&url)
                .header(header::COOKIE, cookie)
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    let status = response.status();
    assert!(
        status == StatusCode::TEMPORARY_REDIRECT
            || status == StatusCode::FOUND
            || status == StatusCode::MOVED_PERMANENTLY
            || status == StatusCode::SEE_OTHER,
        "expected a redirect, got {status}"
    );
}

#[apply(backends)]
#[tokio::test]
async fn proxy_rejects_mismatched_user_id(#[case] backend: Backend) {
    let TestEnv { state, base: _base } = backend.setup().await;

    let user_id = state
        .users
        .create_user(
            &"mismatch".parse().unwrap(),
            &"password123".parse().unwrap(),
            None,
            false,
        )
        .await
        .unwrap();
    let token = state
        .sessions
        .create_session(user_id, "test session")
        .await
        .unwrap();
    let cookie = session_cookie(&token);

    let storage = TempDir::new().unwrap();
    let app = make_app(Arc::clone(&state), &storage);

    // Pass a different user_id in query params.
    let wrong_user_id = UserId::from(i64::from(user_id) + 999);
    let url =
        format!("/media/proxy?url=http%3A%2F%2Fexample.com%2Fimage.jpg&user_id={wrong_user_id}");

    let response = app
        .oneshot(
            Request::builder()
                .method("GET")
                .uri(&url)
                .header(header::COOKIE, cookie)
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
}
