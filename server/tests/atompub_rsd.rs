mod helpers;

use std::sync::Arc;

use axum::{
    body::Body,
    http::{header, Request, StatusCode},
};
use tempfile::TempDir;
use tower::ServiceExt;

use helpers::{ensure_server_fns_registered, noop_mailer, test_options, test_state};

async fn make_app(state: Arc<storage::AppState>, storage: &TempDir) -> axum::Router {
    ensure_server_fns_registered();
    let storage_path = storage.path().to_path_buf();
    jaunder::create_router(test_options(), state, noop_mailer(), false, storage_path)
}

async fn body_string(response: axum::response::Response) -> String {
    let bytes = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .unwrap();
    String::from_utf8(bytes.to_vec()).unwrap()
}

#[tokio::test]
async fn rsd_document_advertises_service_url() {
    let base = TempDir::new().unwrap();
    let state = test_state(&base).await;
    state
        .site_config
        .set_identity(&common::site::SiteIdentity {
            title: "Test".to_string(),
            base_url: Some("https://example.test".to_string()),
        })
        .await
        .unwrap();
    let app = make_app(state, &base).await;

    // RSD is public — no authentication required.
    let response = app
        .oneshot(
            Request::builder()
                .uri("/~alice/rsd.xml")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);
    let content_type = response
        .headers()
        .get(header::CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .unwrap_or_default()
        .to_string();
    assert!(
        content_type.contains("application/rsd+xml"),
        "content-type was {content_type}"
    );

    let body = body_string(response).await;
    assert!(body.contains("<engineName>Jaunder</engineName>"), "{body}");
    assert!(
        body.contains("apiLink=\"https://example.test/atompub/service\""),
        "{body}"
    );
    assert!(body.contains("https://example.test/~alice"), "{body}");
}

#[tokio::test]
async fn user_page_includes_rsd_autodiscovery_link() {
    let base = TempDir::new().unwrap();
    let state = test_state(&base).await;
    state
        .users
        .create_user(
            &"alice".parse().unwrap(),
            &"password123".parse().unwrap(),
            None,
            false,
        )
        .await
        .unwrap();
    let app = make_app(state, &base).await;

    // Rendering the user page (server-side) hoists the EditURI autodiscovery
    // link into the document head.
    let response = app
        .oneshot(
            Request::builder()
                .uri("/~alice")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);
    let body = body_string(response).await;
    assert!(body.contains("rel=\"EditURI\""), "{body}");
    assert!(body.contains("/~alice/rsd.xml"), "{body}");
}
