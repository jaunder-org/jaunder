mod helpers;

use axum::{
    body::Body,
    http::{Request, StatusCode},
};
use tempfile::TempDir;
use tower::ServiceExt;

use helpers::{test_options, test_state};

async fn get_asset(uri: &str) -> (StatusCode, Option<String>) {
    let base = TempDir::new().unwrap();
    let state = test_state(&base).await;

    let request = Request::builder()
        .method("GET")
        .uri(uri)
        .body(Body::empty())
        .unwrap();

    let app = jaunder::create_router(test_options(), state, false);
    let response = app.oneshot(request).await.unwrap();

    let status = response.status();
    let content_type = response
        .headers()
        .get(axum::http::header::CONTENT_TYPE)
        .map(|v| v.to_str().unwrap().to_string());

    (status, content_type)
}

#[tokio::test]
async fn test_jaunder_css_served() {
    let (status, content_type) = get_asset("/style/jaunder.css").await;
    assert_eq!(status, StatusCode::OK);
    let ct = content_type.expect("content-type header should be present");
    assert!(
        ct.contains("text/css"),
        "expected content-type to contain text/css, got: {ct}"
    );
}

#[tokio::test]
async fn test_jaunder_themes_css_served() {
    let (status, content_type) = get_asset("/style/jaunder-themes.css").await;
    assert_eq!(status, StatusCode::OK);
    let ct = content_type.expect("content-type header should be present");
    assert!(
        ct.contains("text/css"),
        "expected content-type to contain text/css, got: {ct}"
    );
}
