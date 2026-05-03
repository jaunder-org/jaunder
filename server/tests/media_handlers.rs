mod helpers;

use std::sync::Arc;

use axum::{
    body::Body,
    http::{header, Request, StatusCode},
};
use tempfile::TempDir;
use tower::ServiceExt;

use helpers::{ensure_server_fns_registered, test_options, test_state};

/// Build the router with a real temp storage directory.
async fn make_app(
    state: Arc<jaunder::storage::AppState>,
    storage: &TempDir,
) -> axum::Router {
    ensure_server_fns_registered();
    let storage_path = storage.path().to_path_buf();
    std::fs::create_dir_all(storage_path.join("media").join("upload")).unwrap();
    std::fs::create_dir_all(storage_path.join("media").join("cached")).unwrap();
    std::fs::create_dir_all(storage_path.join("media").join("tmp")).unwrap();
    jaunder::create_router(test_options(), state, false, storage_path)
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

#[tokio::test]
async fn upload_returns_201_with_json() {
    let base = TempDir::new().unwrap();
    let state = test_state(&base).await;

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
    let token = state.sessions.create_session(user_id, None).await.unwrap();
    let cookie = format!("session={token}");

    let storage = TempDir::new().unwrap();
    let app = make_app(Arc::clone(&state), &storage).await;

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

#[tokio::test]
async fn upload_requires_auth() {
    let base = TempDir::new().unwrap();
    let state = test_state(&base).await;

    let storage = TempDir::new().unwrap();
    let app = make_app(Arc::clone(&state), &storage).await;

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

#[tokio::test]
async fn serve_returns_200_with_cache_headers() {
    let base = TempDir::new().unwrap();
    let state = test_state(&base).await;

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
    let token = state.sessions.create_session(user_id, None).await.unwrap();
    let cookie = format!("session={token}");

    let storage = TempDir::new().unwrap();
    let app = make_app(Arc::clone(&state), &storage).await;

    // Upload a file first.
    let (boundary, body_bytes) =
        multipart_body("serve_test.png", "image/png", b"PNG_CONTENT_HERE");

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
    let app2 = make_app(Arc::clone(&state), &storage).await;

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

#[tokio::test]
async fn serve_returns_404_for_missing_file() {
    let base = TempDir::new().unwrap();
    let state = test_state(&base).await;

    let storage = TempDir::new().unwrap();
    let app = make_app(Arc::clone(&state), &storage).await;

    let response = app
        .oneshot(
            Request::builder()
                .method("GET")
                .uri("/media/upload/abcd/abcdef1234567890abcdef1234567890abcdef1234567890abcdef1234567890/missing.jpg")
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

#[tokio::test]
async fn proxy_requires_auth() {
    let base = TempDir::new().unwrap();
    let state = test_state(&base).await;

    let storage = TempDir::new().unwrap();
    let app = make_app(Arc::clone(&state), &storage).await;

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

#[tokio::test]
async fn proxy_redirects_authenticated() {
    let base = TempDir::new().unwrap();
    let state = test_state(&base).await;

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
    let token = state.sessions.create_session(user_id, None).await.unwrap();
    let cookie = format!("session={token}");

    let storage = TempDir::new().unwrap();
    let app = make_app(Arc::clone(&state), &storage).await;

    let url = format!(
        "/media/proxy?url=http%3A%2F%2Fexample.com%2Fimage.jpg&user_id={user_id}"
    );

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
