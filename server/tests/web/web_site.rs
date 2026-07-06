use std::sync::Arc;

use axum::{
    body::Body,
    http::{header, Request, StatusCode},
};
use common::site::SiteIdentity;
use common::{password::Password, username::Username};
use tower::ServiceExt;

use rstest::*;
#[allow(clippy::single_component_path_imports)]
use rstest_reuse;
use rstest_reuse::*;

use crate::helpers::{backends, ensure_server_fns_registered, test_options, Backend, TestEnv};

async fn post_form(
    state: Arc<storage::AppState>,
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
    let request = builder.body(Body::from(body.into())).expect("request");

    let app = jaunder::create_router(
        test_options(),
        state,
        crate::helpers::noop_mailer(),
        true,
        crate::helpers::tmp_storage_path(),
    );
    let response = app.oneshot(request).await.expect("response");

    let status = response.status();
    let bytes = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .expect("body");
    let body_str = String::from_utf8(bytes.to_vec()).expect("utf-8");

    (status, body_str)
}

#[apply(backends)]
#[tokio::test]
async fn get_site_identity_requires_operator(#[case] backend: Backend) {
    let TestEnv { state, base: _base } = backend.setup().await;
    let anonymous_cookie = None;
    let member_cookie = create_session_cookie(&state, "member", false).await;

    let (anon_status, anon_body) = post_form(
        Arc::clone(&state),
        "/api/get_site_identity",
        "",
        anonymous_cookie,
    )
    .await;
    assert_eq!(
        anon_status,
        StatusCode::INTERNAL_SERVER_ERROR,
        "body: {anon_body}"
    );
    assert!(anon_body.contains("unauthorized"), "body: {anon_body}");

    let (member_status, member_body) = post_form(
        Arc::clone(&state),
        "/api/get_site_identity",
        "",
        Some(&member_cookie),
    )
    .await;
    assert_eq!(
        member_status,
        StatusCode::INTERNAL_SERVER_ERROR,
        "body: {member_body}"
    );
    assert!(member_body.contains("unauthorized"), "body: {member_body}");
}

#[apply(backends)]
#[tokio::test]
async fn get_site_identity_returns_defaults_when_unconfigured(#[case] backend: Backend) {
    let TestEnv { state, base: _base } = backend.setup().await;
    let cookie = create_session_cookie(&state, "operator", true).await;

    let (status, body) = post_form(state, "/api/get_site_identity", "", Some(&cookie)).await;

    assert_eq!(status, StatusCode::OK, "body: {body}");
    let identity: SiteIdentity = serde_json::from_str(&body).expect("json");
    assert_eq!(identity.title, "Jaunder");
    assert_eq!(identity.base_url, None);
}

#[apply(backends)]
#[tokio::test]
async fn update_site_identity_round_trips_via_get(#[case] backend: Backend) {
    let TestEnv { state, base: _base } = backend.setup().await;
    let cookie = create_session_cookie(&state, "operator", true).await;

    let update_body = "title=My+Blog&base_url=https%3A%2F%2Fexample.com%2F";
    let (update_status, update_body_resp) = post_form(
        Arc::clone(&state),
        "/api/update_site_identity",
        update_body,
        Some(&cookie),
    )
    .await;
    assert_eq!(update_status, StatusCode::OK, "body: {update_body_resp}");

    let (get_status, get_body) = post_form(
        Arc::clone(&state),
        "/api/get_site_identity",
        "",
        Some(&cookie),
    )
    .await;
    assert_eq!(get_status, StatusCode::OK, "body: {get_body}");
    let identity: SiteIdentity = serde_json::from_str(&get_body).expect("json");
    assert_eq!(identity.title, "My Blog");
    assert_eq!(identity.base_url, Some("https://example.com".to_string()));
}

#[apply(backends)]
#[tokio::test]
async fn update_site_identity_rejects_empty_title(#[case] backend: Backend) {
    let TestEnv { state, base: _base } = backend.setup().await;
    let cookie = create_session_cookie(&state, "operator", true).await;

    let (status, body) = post_form(
        state,
        "/api/update_site_identity",
        "title=+++&base_url=https%3A%2F%2Fexample.com",
        Some(&cookie),
    )
    .await;

    assert_eq!(status, StatusCode::INTERNAL_SERVER_ERROR, "body: {body}");
    assert!(body.contains("site title cannot be empty"), "body: {body}");
}

#[apply(backends)]
#[tokio::test]
async fn update_site_identity_rejects_non_http_base_url(#[case] backend: Backend) {
    let TestEnv { state, base: _base } = backend.setup().await;
    let cookie = create_session_cookie(&state, "operator", true).await;

    let (status, body) = post_form(
        state,
        "/api/update_site_identity",
        "title=My+Blog&base_url=ftp%3A%2F%2Fexample.com",
        Some(&cookie),
    )
    .await;

    assert_eq!(status, StatusCode::INTERNAL_SERVER_ERROR, "body: {body}");
    assert!(
        body.contains("base URL must be an absolute http or https URL"),
        "body: {body}"
    );
}

#[apply(backends)]
#[tokio::test]
async fn update_site_identity_accepts_empty_base_url_as_none(#[case] backend: Backend) {
    let TestEnv { state, base: _base } = backend.setup().await;
    let cookie = create_session_cookie(&state, "operator", true).await;

    let (update_status, update_body) = post_form(
        Arc::clone(&state),
        "/api/update_site_identity",
        "title=My+Blog&base_url=",
        Some(&cookie),
    )
    .await;
    assert_eq!(update_status, StatusCode::OK, "body: {update_body}");

    let (get_status, get_body) = post_form(
        Arc::clone(&state),
        "/api/get_site_identity",
        "",
        Some(&cookie),
    )
    .await;
    assert_eq!(get_status, StatusCode::OK, "body: {get_body}");
    let identity: SiteIdentity = serde_json::from_str(&get_body).expect("json");
    assert_eq!(identity.base_url, None);
}

#[apply(backends)]
#[tokio::test]
async fn update_site_identity_requires_operator(#[case] backend: Backend) {
    let TestEnv { state, base: _base } = backend.setup().await;
    let anonymous_cookie = None;
    let member_cookie = create_session_cookie(&state, "member", false).await;

    let body = "title=My+Blog&base_url=https%3A%2F%2Fexample.com";

    let (anon_status, anon_body) = post_form(
        Arc::clone(&state),
        "/api/update_site_identity",
        body,
        anonymous_cookie,
    )
    .await;
    assert_eq!(
        anon_status,
        StatusCode::INTERNAL_SERVER_ERROR,
        "body: {anon_body}"
    );
    assert!(anon_body.contains("unauthorized"), "body: {anon_body}");

    let (member_status, member_body) = post_form(
        state,
        "/api/update_site_identity",
        body,
        Some(&member_cookie),
    )
    .await;
    assert_eq!(
        member_status,
        StatusCode::INTERNAL_SERVER_ERROR,
        "body: {member_body}"
    );
    assert!(member_body.contains("unauthorized"), "body: {member_body}");
}

async fn create_session_cookie(
    state: &Arc<storage::AppState>,
    username: &str,
    is_operator: bool,
) -> String {
    let username: Username = username.parse().expect("username");
    let password: Password = "password123".parse().expect("password");
    let user_id = state
        .users
        .create_user(&username, &password, None, is_operator)
        .await
        .expect("create_user");
    let token = state
        .sessions
        .create_session(user_id, "test session")
        .await
        .expect("create_session");

    format!("session={token}")
}
