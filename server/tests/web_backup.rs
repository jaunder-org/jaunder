mod helpers;

use std::sync::Arc;

use axum::{
    body::Body,
    http::{header, Request, StatusCode},
};
use common::storage::BACKUP_DESTINATION_PATH_KEY;
use jaunder::{password::Password, username::Username};
use tempfile::TempDir;
use tower::ServiceExt;

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

async fn create_session_cookie(
    state: &Arc<jaunder::storage::AppState>,
    username: &str,
    is_operator: bool,
) -> String {
    let username: Username = username.parse().unwrap();
    let password: Password = "password123".parse().unwrap();
    let user_id = state
        .users
        .create_user(&username, &password, None, is_operator)
        .await
        .unwrap();
    let token = state.sessions.create_session(user_id, None).await.unwrap();

    format!("session={token}")
}

#[tokio::test]
async fn backup_warning_visible_for_operator_without_destination() {
    let base = TempDir::new().unwrap();
    let state = test_state(&base).await;
    let cookie = create_session_cookie(&state, "operator", true).await;

    let (status, body) = post_form(
        Arc::clone(&state),
        "/api/backup_warning_visible",
        "",
        Some(&cookie),
    )
    .await;

    assert_eq!(status, StatusCode::OK, "body: {body}");
    assert_eq!(body, "true");
}

#[tokio::test]
async fn backup_warning_hidden_when_destination_configured() {
    let base = TempDir::new().unwrap();
    let state = test_state(&base).await;
    let cookie = create_session_cookie(&state, "operator", true).await;
    state
        .site_config
        .set(BACKUP_DESTINATION_PATH_KEY, "/srv/backups")
        .await
        .unwrap();

    let (status, body) = post_form(
        Arc::clone(&state),
        "/api/backup_warning_visible",
        "",
        Some(&cookie),
    )
    .await;

    assert_eq!(status, StatusCode::OK, "body: {body}");
    assert_eq!(body, "false");
}

#[tokio::test]
async fn backup_warning_hidden_for_non_operator() {
    let base = TempDir::new().unwrap();
    let state = test_state(&base).await;
    let cookie = create_session_cookie(&state, "member", false).await;

    let (status, body) = post_form(
        Arc::clone(&state),
        "/api/backup_warning_visible",
        "",
        Some(&cookie),
    )
    .await;

    assert_eq!(status, StatusCode::OK, "body: {body}");
    assert_eq!(body, "false");
}

#[tokio::test]
async fn backup_warning_hidden_without_authentication() {
    let base = TempDir::new().unwrap();
    let state = test_state(&base).await;

    let (status, body) = post_form(state, "/api/backup_warning_visible", "", None).await;

    assert_eq!(status, StatusCode::OK, "body: {body}");
    assert_eq!(body, "false");
}
