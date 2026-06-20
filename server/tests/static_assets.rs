#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::too_many_lines,
    clippy::similar_names,
    clippy::items_after_statements,
    clippy::unused_async
)]

mod helpers;

use axum::{
    body::Body,
    http::{Request, StatusCode},
};
use tower::ServiceExt;

use helpers::{test_options, Backend, TestEnv};

async fn get_asset(uri: &str) -> (StatusCode, Option<String>) {
    // Static-asset serving never touches storage; pin a single backend so these
    // stay plain (no need to run embedded-asset serving on both).
    let TestEnv { state, base: _base } = Backend::Sqlite.setup().await;

    let request = Request::builder()
        .method("GET")
        .uri(uri)
        .body(Body::empty())
        .unwrap();

    let app = jaunder::create_router(
        test_options(),
        state,
        helpers::noop_mailer(),
        false,
        helpers::tmp_storage_path(),
    );
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
