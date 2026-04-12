mod helpers;

use std::sync::Arc;

use axum::{
    body::Body,
    http::{header, Request, StatusCode},
};
use common::storage::PostFormat;
use tempfile::TempDir;
use tower::ServiceExt;
use web::posts::CreatePostResult;

use helpers::{ensure_server_fns_registered, test_options, test_state};

async fn post_form(
    state: Arc<jaunder::storage::AppState>,
    uri: &str,
    body: impl Into<String>,
    cookie: Option<&str>,
) -> (StatusCode, String) {
    ensure_server_fns_registered();

    let mut builder = Request::builder()
        .method("POST")
        .uri(uri)
        .header(header::CONTENT_TYPE, "application/x-www-form-urlencoded");
    if let Some(c) = cookie {
        builder = builder.header(header::COOKIE, c);
    }
    let request = builder.body(Body::from(body.into())).unwrap();

    let app = jaunder::create_router(test_options(), state, true);
    let response = app.oneshot(request).await.unwrap();

    let status = response.status();
    let bytes = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .unwrap();
    let body_str = String::from_utf8(bytes.to_vec()).unwrap();

    (status, body_str)
}

#[tokio::test]
async fn create_post_persists_rendered_published_post() {
    let base = TempDir::new().unwrap();
    let state = test_state(&base).await;
    let user_id = state
        .users
        .create_user(
            &"author".parse().unwrap(),
            &"password123".parse().unwrap(),
            None,
        )
        .await
        .unwrap();
    let token = state.sessions.create_session(user_id, None).await.unwrap();
    let cookie = format!("session={token}");

    let (status, body) = post_form(
        Arc::clone(&state),
        "/api/create_post",
        "title=Hello+World&body=%2A%2Abold%2A%2A&format=markdown&publish=true",
        Some(&cookie),
    )
    .await;

    assert_eq!(status, StatusCode::OK, "body: {body}");
    let created: CreatePostResult = serde_json::from_str(&body).unwrap();
    assert_eq!(created.slug, "hello-world");
    assert!(created.published_at.is_some());

    let record = state
        .posts
        .get_post_by_id(created.post_id)
        .await
        .unwrap()
        .expect("post should exist");
    assert_eq!(record.title, "Hello World");
    assert_eq!(record.slug.to_string(), "hello-world");
    assert_eq!(record.format, PostFormat::Markdown);
    assert!(record.published_at.is_some());
    assert!(
        record.rendered_html.contains("<strong>bold</strong>"),
        "rendered_html: {}",
        record.rendered_html
    );
}

#[tokio::test]
async fn create_post_retries_slug_conflicts_for_same_user_and_date() {
    let base = TempDir::new().unwrap();
    let state = test_state(&base).await;
    let user_id = state
        .users
        .create_user(
            &"author".parse().unwrap(),
            &"password123".parse().unwrap(),
            None,
        )
        .await
        .unwrap();
    let token = state.sessions.create_session(user_id, None).await.unwrap();
    let cookie = format!("session={token}");

    let (first_status, first_body) = post_form(
        Arc::clone(&state),
        "/api/create_post",
        "title=Repeated+Title&body=first&format=markdown&publish=true",
        Some(&cookie),
    )
    .await;
    assert_eq!(first_status, StatusCode::OK, "body: {first_body}");

    let (second_status, second_body) = post_form(
        Arc::clone(&state),
        "/api/create_post",
        "title=Repeated+Title&body=second&format=markdown&publish=true",
        Some(&cookie),
    )
    .await;

    assert_eq!(second_status, StatusCode::OK, "body: {second_body}");
    let created: CreatePostResult = serde_json::from_str(&second_body).unwrap();
    assert_eq!(created.slug, "repeated-title-2");
}

#[tokio::test]
async fn create_post_rejects_requests_without_authentication() {
    let base = TempDir::new().unwrap();
    let state = test_state(&base).await;

    let (status, body) = post_form(
        state,
        "/api/create_post",
        "title=Unauthorized&body=body&format=markdown&publish=false",
        None,
    )
    .await;

    assert_eq!(status, StatusCode::INTERNAL_SERVER_ERROR, "body: {body}");
    assert!(body.contains("unauthorized"), "body: {body}");
}

#[tokio::test]
async fn create_post_accepts_slug_override_and_saves_draft() {
    let base = TempDir::new().unwrap();
    let state = test_state(&base).await;
    let user_id = state
        .users
        .create_user(
            &"author".parse().unwrap(),
            &"password123".parse().unwrap(),
            None,
        )
        .await
        .unwrap();
    let token = state.sessions.create_session(user_id, None).await.unwrap();
    let cookie = format!("session={token}");

    let (status, body) = post_form(
        Arc::clone(&state),
        "/api/create_post",
        "title=Draft+Post&body=%2Abold%2A&format=org&slug_override=Custom-Slug&publish=false",
        Some(&cookie),
    )
    .await;

    assert_eq!(status, StatusCode::OK, "body: {body}");
    let created: CreatePostResult = serde_json::from_str(&body).unwrap();
    assert_eq!(created.slug, "custom-slug");
    assert!(created.published_at.is_none());

    let record = state
        .posts
        .get_post_by_id(created.post_id)
        .await
        .unwrap()
        .expect("post should exist");
    assert_eq!(record.slug.to_string(), "custom-slug");
    assert_eq!(record.format, PostFormat::Org);
    assert!(record.published_at.is_none());
    assert!(record.rendered_html.contains("<b>bold</b>"));
}

#[tokio::test]
async fn create_post_rejects_empty_title() {
    let base = TempDir::new().unwrap();
    let state = test_state(&base).await;
    let user_id = state
        .users
        .create_user(
            &"author".parse().unwrap(),
            &"password123".parse().unwrap(),
            None,
        )
        .await
        .unwrap();
    let token = state.sessions.create_session(user_id, None).await.unwrap();
    let cookie = format!("session={token}");

    let (status, body) = post_form(
        state,
        "/api/create_post",
        "title=+++&body=body&format=markdown&publish=false",
        Some(&cookie),
    )
    .await;

    assert_eq!(status, StatusCode::INTERNAL_SERVER_ERROR, "body: {body}");
    assert!(body.contains("title is required"), "body: {body}");
}

#[tokio::test]
async fn create_post_rejects_invalid_format() {
    let base = TempDir::new().unwrap();
    let state = test_state(&base).await;
    let user_id = state
        .users
        .create_user(
            &"author".parse().unwrap(),
            &"password123".parse().unwrap(),
            None,
        )
        .await
        .unwrap();
    let token = state.sessions.create_session(user_id, None).await.unwrap();
    let cookie = format!("session={token}");

    let (status, body) = post_form(
        state,
        "/api/create_post",
        "title=Bad+Format&body=body&format=html&publish=false",
        Some(&cookie),
    )
    .await;

    assert_eq!(status, StatusCode::INTERNAL_SERVER_ERROR, "body: {body}");
    assert!(body.contains("post format must be"), "body: {body}");
}

#[tokio::test]
async fn create_post_rejects_invalid_slug_override() {
    let base = TempDir::new().unwrap();
    let state = test_state(&base).await;
    let user_id = state
        .users
        .create_user(
            &"author".parse().unwrap(),
            &"password123".parse().unwrap(),
            None,
        )
        .await
        .unwrap();
    let token = state.sessions.create_session(user_id, None).await.unwrap();
    let cookie = format!("session={token}");

    let (status, body) = post_form(
        state,
        "/api/create_post",
        "title=Invalid+Slug&body=body&format=markdown&slug_override=Not Valid&publish=false",
        Some(&cookie),
    )
    .await;

    assert_eq!(status, StatusCode::INTERNAL_SERVER_ERROR, "body: {body}");
    assert!(body.contains("slug must be non-empty"), "body: {body}");
}

#[tokio::test]
async fn create_post_rejects_title_without_ascii_slug_characters() {
    let base = TempDir::new().unwrap();
    let state = test_state(&base).await;
    let user_id = state
        .users
        .create_user(
            &"author".parse().unwrap(),
            &"password123".parse().unwrap(),
            None,
        )
        .await
        .unwrap();
    let token = state.sessions.create_session(user_id, None).await.unwrap();
    let cookie = format!("session={token}");

    let (status, body) = post_form(
        state,
        "/api/create_post",
        "title=%E2%80%94%E2%80%94%E2%80%94&body=body&format=markdown&publish=false",
        Some(&cookie),
    )
    .await;

    assert_eq!(status, StatusCode::INTERNAL_SERVER_ERROR, "body: {body}");
    assert!(
        body.contains("title must contain at least one ASCII letter or digit"),
        "body: {body}"
    );
}
