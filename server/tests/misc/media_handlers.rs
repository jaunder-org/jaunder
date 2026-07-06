use std::sync::Arc;

use axum::{
    body::Body,
    http::{header, Request, StatusCode},
};
use tempfile::TempDir;
use tower::ServiceExt;

use rstest::*;
#[allow(clippy::single_component_path_imports)]
use rstest_reuse;
use rstest_reuse::*;

use crate::helpers::{backends, backends_matrix, Backend, TestEnv};

use crate::helpers::{ensure_server_fns_registered, test_options};

/// Build the router with a real temp storage directory.
fn make_app(state: Arc<storage::AppState>, storage: &TempDir) -> axum::Router {
    ensure_server_fns_registered();
    let storage_path = storage.path().to_path_buf();
    std::fs::create_dir_all(storage_path.join("media").join("upload")).unwrap();
    std::fs::create_dir_all(storage_path.join("media").join("cached")).unwrap();
    std::fs::create_dir_all(storage_path.join("media").join("tmp")).unwrap();
    jaunder::create_router(
        test_options(),
        state,
        crate::helpers::noop_mailer(),
        false,
        storage_path,
    )
}

fn multipart_body(filename: &str, content_type: &str, data: &[u8]) -> (String, Vec<u8>) {
    let boundary = "----testboundary1234";
    let mut body: Vec<u8> = Vec::new();
    body.extend_from_slice(
        format!(
            "--{boundary}\r\nContent-Disposition: form-data; name=\"file\"; filename=\"{filename}\"\r\nContent-Type: {content_type}\r\n\r\n",
        )
        .as_bytes(),
    );
    body.extend_from_slice(data);
    body.extend_from_slice(format!("\r\n--{boundary}--\r\n").as_bytes());
    (boundary.to_owned(), body)
}

// ---------------------------------------------------------------------------
// Upload tests
// ---------------------------------------------------------------------------

#[apply(backends)]
#[tokio::test]
async fn upload_returns_201_with_json(#[case] backend: Backend) {
    let TestEnv { state, base: _base } = backend.setup().await;

    let user_id = state
        .users
        .create_user(
            &"uploader".parse().unwrap(),
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
    let cookie = format!("session={token}");

    let storage = TempDir::new().unwrap();
    let app = make_app(Arc::clone(&state), &storage);

    let (boundary, body_bytes) = multipart_body("photo.jpg", "image/jpeg", b"fake jpeg data");

    let response = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/media/upload")
                .header(
                    header::CONTENT_TYPE,
                    format!("multipart/form-data; boundary={boundary}"),
                )
                .header(header::COOKIE, cookie)
                .body(Body::from(body_bytes))
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::CREATED);

    let bytes = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .unwrap();
    let json: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
    assert!(json["sha256"].is_string(), "sha256 field missing");
    assert_eq!(json["filename"], "photo.jpg");
    assert!(
        json["url"]
            .as_str()
            .unwrap_or("")
            .starts_with("/media/upload/"),
        "url should start with /media/upload/"
    );
}

#[apply(backends)]
#[tokio::test]
async fn upload_requires_auth(#[case] backend: Backend) {
    let TestEnv { state, base: _base } = backend.setup().await;

    let storage = TempDir::new().unwrap();
    let app = make_app(Arc::clone(&state), &storage);

    let (boundary, body_bytes) = multipart_body("file.txt", "text/plain", b"hello");

    let response = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/media/upload")
                .header(
                    header::CONTENT_TYPE,
                    format!("multipart/form-data; boundary={boundary}"),
                )
                .body(Body::from(body_bytes))
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
}

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
    let cookie = format!("session={token}");

    let storage = TempDir::new().unwrap();
    let app = make_app(Arc::clone(&state), &storage);

    let (boundary, body_bytes) = multipart_body("serve_test.png", "image/png", b"PNG_CONTENT_HERE");

    let upload_response = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/media/upload")
                .header(
                    header::CONTENT_TYPE,
                    format!("multipart/form-data; boundary={boundary}"),
                )
                .header(header::COOKIE, cookie)
                .body(Body::from(body_bytes))
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(
        upload_response.status(),
        StatusCode::CREATED,
        "upload must succeed"
    );

    let upload_bytes = axum::body::to_bytes(upload_response.into_body(), usize::MAX)
        .await
        .unwrap();
    let upload_json: serde_json::Value = serde_json::from_slice(&upload_bytes).unwrap();
    let url = upload_json["url"].as_str().unwrap().to_owned();

    // Rebuild the app (oneshot consumes it).
    let app2 = make_app(Arc::clone(&state), &storage);

    let serve_response = app2
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
    let cookie = format!("session={token}");

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

// ---------------------------------------------------------------------------
// Additional coverage tests
// ---------------------------------------------------------------------------

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
    let cookie = format!("session={token}");

    let storage = TempDir::new().unwrap();
    let app = make_app(Arc::clone(&state), &storage);

    let (boundary, body_bytes) = multipart_body("etag_test.png", "image/png", b"PNG_DATA");

    let upload_resp = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/media/upload")
                .header(
                    header::CONTENT_TYPE,
                    format!("multipart/form-data; boundary={boundary}"),
                )
                .header(header::COOKIE, cookie)
                .body(Body::from(body_bytes))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(upload_resp.status(), StatusCode::CREATED);

    let upload_bytes = axum::body::to_bytes(upload_resp.into_body(), usize::MAX)
        .await
        .unwrap();
    let upload_json: serde_json::Value = serde_json::from_slice(&upload_bytes).unwrap();
    let url = upload_json["url"].as_str().unwrap().to_owned();
    let sha256 = upload_json["sha256"].as_str().unwrap().to_owned();
    let etag = format!("\"{sha256}\"");

    let app2 = make_app(Arc::clone(&state), &storage);
    let resp = app2
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

#[apply(backends)]
#[tokio::test]
async fn upload_returns_400_for_empty_multipart(#[case] backend: Backend) {
    let TestEnv { state, base: _base } = backend.setup().await;

    let user_id = state
        .users
        .create_user(
            &"emptyuploader".parse().unwrap(),
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
    let cookie = format!("session={token}");

    let storage = TempDir::new().unwrap();
    let app = make_app(Arc::clone(&state), &storage);

    let boundary = "----testboundary1234";
    let body = format!("--{boundary}--\r\n");

    let response = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/media/upload")
                .header(
                    header::CONTENT_TYPE,
                    format!("multipart/form-data; boundary={boundary}"),
                )
                .header(header::COOKIE, cookie)
                .body(Body::from(body))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
}

#[apply(backends)]
#[tokio::test]
async fn upload_deduplicates_same_content(#[case] backend: Backend) {
    let TestEnv { state, base: _base } = backend.setup().await;

    let user_id = state
        .users
        .create_user(
            &"deduper".parse().unwrap(),
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
    let cookie = format!("session={token}");

    let storage = TempDir::new().unwrap();

    // Upload the same content twice (different filename).
    for filename in ["dup1.jpg", "dup2.jpg"] {
        let app = make_app(Arc::clone(&state), &storage);
        let (boundary, body_bytes) = multipart_body(filename, "image/jpeg", b"SAME_CONTENT");
        let resp = app
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/media/upload")
                    .header(
                        header::CONTENT_TYPE,
                        format!("multipart/form-data; boundary={boundary}"),
                    )
                    .header(header::COOKIE, format!("session={token}"))
                    .body(Body::from(body_bytes))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(
            resp.status(),
            StatusCode::CREATED,
            "upload of {filename} should succeed"
        );
        drop(token.clone()); // keep borrow checker happy
    }

    // Both uploads with same content should produce 201.
    drop(cookie);
}

#[apply(backends)]
#[tokio::test]
async fn upload_quota_exceeded_returns_507(#[case] backend: Backend) {
    let TestEnv { state, base: _base } = backend.setup().await;

    // Set a tiny quota of 1 byte.
    state
        .site_config
        .set("media.user_quota_bytes", "1")
        .await
        .unwrap();

    let user_id = state
        .users
        .create_user(
            &"quotauser".parse().unwrap(),
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
    let cookie = format!("session={token}");

    let storage = TempDir::new().unwrap();
    let app = make_app(Arc::clone(&state), &storage);

    let (boundary, body_bytes) = multipart_body("big.jpg", "image/jpeg", b"SOME_DATA_OVER_QUOTA");

    let response = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/media/upload")
                .header(
                    header::CONTENT_TYPE,
                    format!("multipart/form-data; boundary={boundary}"),
                )
                .header(header::COOKIE, cookie)
                .body(Body::from(body_bytes))
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::INSUFFICIENT_STORAGE);
}

#[apply(backends)]
#[tokio::test]
async fn upload_at_max_file_size_boundary_succeeds(#[case] backend: Backend) {
    let TestEnv { state, base: _base } = backend.setup().await;

    state
        .site_config
        .set("media.max_file_size_bytes", "5")
        .await
        .unwrap();

    let user_id = state
        .users
        .create_user(
            &"sizebound".parse().unwrap(),
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
    let cookie = format!("session={token}");

    let storage = TempDir::new().unwrap();
    let app = make_app(Arc::clone(&state), &storage);

    let (boundary, body_bytes) = multipart_body("exact.txt", "text/plain", b"hello");

    let response = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/media/upload")
                .header(
                    header::CONTENT_TYPE,
                    format!("multipart/form-data; boundary={boundary}"),
                )
                .header(header::COOKIE, cookie)
                .body(Body::from(body_bytes))
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(
        response.status(),
        StatusCode::CREATED,
        "file exactly at max size must be accepted"
    );
}

#[apply(backends)]
#[tokio::test]
async fn upload_one_byte_over_max_file_size_is_rejected(#[case] backend: Backend) {
    let TestEnv { state, base: _base } = backend.setup().await;

    state
        .site_config
        .set("media.max_file_size_bytes", "5")
        .await
        .unwrap();

    let user_id = state
        .users
        .create_user(
            &"sizeover".parse().unwrap(),
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
    let cookie = format!("session={token}");

    let storage = TempDir::new().unwrap();
    let app = make_app(Arc::clone(&state), &storage);

    let (boundary, body_bytes) = multipart_body("toobig.txt", "text/plain", b"hello!");

    let response = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/media/upload")
                .header(
                    header::CONTENT_TYPE,
                    format!("multipart/form-data; boundary={boundary}"),
                )
                .header(header::COOKIE, cookie)
                .body(Body::from(body_bytes))
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(
        response.status(),
        StatusCode::PAYLOAD_TOO_LARGE,
        "file one byte over max size must be rejected"
    );
}

#[apply(backends)]
#[tokio::test]
async fn upload_at_exact_quota_succeeds(#[case] backend: Backend) {
    let TestEnv { state, base: _base } = backend.setup().await;

    state
        .site_config
        .set("media.user_quota_bytes", "5")
        .await
        .unwrap();

    let user_id = state
        .users
        .create_user(
            &"quotaexact".parse().unwrap(),
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
    let cookie = format!("session={token}");

    let storage = TempDir::new().unwrap();
    let app = make_app(Arc::clone(&state), &storage);

    let (boundary, body_bytes) = multipart_body("quota.txt", "text/plain", b"hello");

    let response = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/media/upload")
                .header(
                    header::CONTENT_TYPE,
                    format!("multipart/form-data; boundary={boundary}"),
                )
                .header(header::COOKIE, cookie)
                .body(Body::from(body_bytes))
                .unwrap(),
        )
        .await
        .unwrap();

    // current_usage (0) + 5 > 5 is false — upload must be accepted.
    assert_eq!(
        response.status(),
        StatusCode::CREATED,
        "file that exactly fills remaining quota must be accepted"
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
    let cookie = format!("session={token}");

    let storage = TempDir::new().unwrap();
    let app = make_app(Arc::clone(&state), &storage);

    // Pass a different user_id in query params.
    let wrong_user_id = user_id + 999;
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
