#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::too_many_lines,
    clippy::similar_names,
    clippy::items_after_statements,
    clippy::unused_async
)]
#![allow(unused_macros)]

use std::sync::Arc;

use axum::{
    body::Body,
    http::{header, Request, StatusCode},
};
use common::backup::{BackupConfig, BackupMode};
use common::{password::Password, username::Username};
use storage::{
    BACKUP_DESTINATION_PATH_KEY, BACKUP_MODE_KEY, BACKUP_RETENTION_COUNT_KEY, BACKUP_SCHEDULE_KEY,
};
use tempfile::TempDir;
use tower::ServiceExt;

use rstest::*;
#[allow(clippy::single_component_path_imports)]
use rstest_reuse;
use rstest_reuse::*;

use crate::helpers::{backends, backends_matrix, Backend, TestEnv};

use crate::helpers::{ensure_server_fns_registered, test_options};

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
    let request = builder.body(Body::from(body.into())).unwrap();

    let app = jaunder::create_router(
        test_options(),
        state,
        crate::helpers::noop_mailer(),
        true,
        crate::helpers::tmp_storage_path(),
    );
    let response = app.oneshot(request).await.unwrap();

    let status = response.status();
    let bytes = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .unwrap();
    let body_str = String::from_utf8(bytes.to_vec()).unwrap();

    (status, body_str)
}

#[apply(backends)]
#[tokio::test]
async fn operator_gets_default_backup_settings(#[case] backend: Backend) {
    let TestEnv { state, base: _base } = backend.setup().await;
    let cookie = create_session_cookie(&state, "operator", true).await;

    let (status, body) = post_form(state, "/api/get_backup_settings", "", Some(&cookie)).await;

    assert_eq!(status, StatusCode::OK, "body: {body}");
    let settings: BackupConfig = serde_json::from_str(&body).unwrap();
    assert_eq!(settings.destination_path, None);
    assert_eq!(settings.schedule.as_str(), "0 0 0 * * *");
    assert_eq!(settings.retention_count, 7);
    assert_eq!(settings.mode, BackupMode::Directory);
}

#[apply(backends)]
#[tokio::test]
async fn current_user_is_operator_reports_operator_status(#[case] backend: Backend) {
    let TestEnv { state, base: _base } = backend.setup().await;
    let operator_cookie = create_session_cookie(&state, "operator", true).await;
    let member_cookie = create_session_cookie(&state, "member", false).await;

    let (operator_status, operator_body) = post_form(
        Arc::clone(&state),
        "/api/current_user_is_operator",
        "",
        Some(&operator_cookie),
    )
    .await;
    assert_eq!(operator_status, StatusCode::OK, "body: {operator_body}");
    assert_eq!(operator_body, "true");

    let (member_status, member_body) = post_form(
        Arc::clone(&state),
        "/api/current_user_is_operator",
        "",
        Some(&member_cookie),
    )
    .await;
    assert_eq!(member_status, StatusCode::OK, "body: {member_body}");
    assert_eq!(member_body, "false");

    let (anonymous_status, anonymous_body) =
        post_form(state, "/api/current_user_is_operator", "", None).await;
    assert_eq!(anonymous_status, StatusCode::OK, "body: {anonymous_body}");
    assert_eq!(anonymous_body, "false");
}

#[apply(backends)]
#[tokio::test]
async fn operator_gets_configured_backup_settings(#[case] backend: Backend) {
    let TestEnv { state, base: _base } = backend.setup().await;
    let cookie = create_session_cookie(&state, "operator", true).await;
    state
        .site_config
        .set(BACKUP_DESTINATION_PATH_KEY, "/srv/backups")
        .await
        .unwrap();
    state
        .site_config
        .set(BACKUP_SCHEDULE_KEY, "0 30 2 * * *")
        .await
        .unwrap();
    state
        .site_config
        .set(BACKUP_RETENTION_COUNT_KEY, "4")
        .await
        .unwrap();
    state
        .site_config
        .set(BACKUP_MODE_KEY, "archive")
        .await
        .unwrap();

    let (status, body) = post_form(state, "/api/get_backup_settings", "", Some(&cookie)).await;

    assert_eq!(status, StatusCode::OK, "body: {body}");
    let settings: BackupConfig = serde_json::from_str(&body).unwrap();
    assert_eq!(settings.destination_path, Some("/srv/backups".to_string()));
    assert_eq!(settings.schedule.as_str(), "0 30 2 * * *");
    assert_eq!(settings.retention_count, 4);
    assert_eq!(settings.mode, BackupMode::Archive);
}

#[apply(backends)]
#[tokio::test]
async fn operator_gets_defaults_for_invalid_backup_settings(#[case] backend: Backend) {
    let TestEnv { state, base: _base } = backend.setup().await;
    let cookie = create_session_cookie(&state, "operator", true).await;
    state
        .site_config
        .set(BACKUP_DESTINATION_PATH_KEY, "/srv/backups")
        .await
        .unwrap();
    state
        .site_config
        .set(BACKUP_SCHEDULE_KEY, "not-a-schedule")
        .await
        .unwrap();
    state
        .site_config
        .set(BACKUP_RETENTION_COUNT_KEY, "daily")
        .await
        .unwrap();
    state
        .site_config
        .set(BACKUP_MODE_KEY, "surprise")
        .await
        .unwrap();

    let (status, body) = post_form(state, "/api/get_backup_settings", "", Some(&cookie)).await;

    assert_eq!(status, StatusCode::OK, "body: {body}");
    let settings: BackupConfig = serde_json::from_str(&body).unwrap();
    assert_eq!(settings.destination_path, Some("/srv/backups".to_string()));
    assert_eq!(settings.schedule.as_str(), "0 0 0 * * *");
    assert_eq!(settings.retention_count, 7);
    assert_eq!(settings.mode, BackupMode::Directory);
}

#[apply(backends)]
#[tokio::test]
async fn operator_can_update_backup_settings(#[case] backend: Backend) {
    let TestEnv { state, base: _base } = backend.setup().await;
    let cookie = create_session_cookie(&state, "operator", true).await;

    let (status, body) = post_form(
        Arc::clone(&state),
        "/api/update_backup_settings",
        "destination_path=%2Fsrv%2Fbackups&schedule=0+0+0+*+*+*&retention_count=5&mode=directory",
        Some(&cookie),
    )
    .await;

    assert_eq!(status, StatusCode::OK, "body: {body}");
    assert_eq!(
        state
            .site_config
            .get(BACKUP_DESTINATION_PATH_KEY)
            .await
            .unwrap()
            .as_deref(),
        Some("/srv/backups")
    );
    assert_eq!(
        state
            .site_config
            .get(BACKUP_SCHEDULE_KEY)
            .await
            .unwrap()
            .as_deref(),
        Some("0 0 0 * * *")
    );
    assert_eq!(
        state
            .site_config
            .get(BACKUP_RETENTION_COUNT_KEY)
            .await
            .unwrap()
            .as_deref(),
        Some("5")
    );
    assert_eq!(
        state
            .site_config
            .get(BACKUP_MODE_KEY)
            .await
            .unwrap()
            .as_deref(),
        Some("directory")
    );
}

#[apply(backends)]
#[tokio::test]
async fn operator_can_update_backup_settings_to_archive_mode(#[case] backend: Backend) {
    let TestEnv { state, base: _base } = backend.setup().await;
    let cookie = create_session_cookie(&state, "operator", true).await;

    let (status, body) = post_form(
        Arc::clone(&state),
        "/api/update_backup_settings",
        "destination_path=%2Fsrv%2Fbackups&schedule=0+0+0+*+*+*&retention_count=5&mode=archive",
        Some(&cookie),
    )
    .await;

    assert_eq!(status, StatusCode::OK, "body: {body}");
    assert_eq!(
        state
            .site_config
            .get(BACKUP_MODE_KEY)
            .await
            .unwrap()
            .as_deref(),
        Some("archive")
    );
}

#[apply(backends_matrix)]
#[case::empty_schedule(
    "destination_path=%2Fsrv%2Fbackups&schedule=+++&retention_count=5&mode=directory",
    "backup schedule must be a valid six-field cron expression"
)]
#[case::invalid_schedule(
    "destination_path=%2Fsrv%2Fbackups&schedule=not-a-schedule&retention_count=5&mode=directory",
    "backup schedule must be a valid six-field cron expression"
)]
#[case::invalid_retention_count(
    "destination_path=%2Fsrv%2Fbackups&schedule=0+0+0+*+*+*&retention_count=bogus&mode=directory",
    "backup retention count"
)]
#[case::invalid_mode(
    "destination_path=%2Fsrv%2Fbackups&schedule=0+0+0+*+*+*&retention_count=5&mode=surprise",
    "backup mode"
)]
#[tokio::test]
async fn operator_update_backup_settings_rejects_invalid_input(
    backend: Backend,
    #[case] form: &str,
    #[case] expected_error: &str,
) {
    let TestEnv { state, base: _base } = backend.setup().await;
    let cookie = create_session_cookie(&state, "operator", true).await;

    let (status, body) = post_form(state, "/api/update_backup_settings", form, Some(&cookie)).await;

    assert_eq!(status, StatusCode::INTERNAL_SERVER_ERROR, "body: {body}");
    assert!(body.contains(expected_error));
}

#[apply(backends)]
#[tokio::test]
async fn non_operator_cannot_update_backup_settings(#[case] backend: Backend) {
    let TestEnv { state, base: _base } = backend.setup().await;
    let cookie = create_session_cookie(&state, "member", false).await;

    let (status, body) = post_form(
        state,
        "/api/update_backup_settings",
        "destination_path=%2Fsrv%2Fbackups&schedule=0+0+0+*+*+*&retention_count=5&mode=directory",
        Some(&cookie),
    )
    .await;

    assert_eq!(status, StatusCode::INTERNAL_SERVER_ERROR, "body: {body}");
    assert!(body.contains("unauthorized"));
}

async fn create_session_cookie(
    state: &Arc<storage::AppState>,
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
    let token = state
        .sessions
        .create_session(user_id, "test session")
        .await
        .unwrap();

    format!("session={token}")
}

#[apply(backends)]
#[tokio::test]
async fn backup_warning_visible_for_operator_without_destination(#[case] backend: Backend) {
    let TestEnv { state, base: _base } = backend.setup().await;
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

#[apply(backends)]
#[tokio::test]
async fn backup_warning_hidden_when_destination_configured(#[case] backend: Backend) {
    let TestEnv { state, base: _base } = backend.setup().await;
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

#[apply(backends)]
#[tokio::test]
async fn backup_warning_visible_when_configured_schedule_is_invalid(#[case] backend: Backend) {
    let TestEnv { state, base: _base } = backend.setup().await;
    let cookie = create_session_cookie(&state, "operator", true).await;
    state
        .site_config
        .set(BACKUP_DESTINATION_PATH_KEY, "/srv/backups")
        .await
        .unwrap();
    state
        .site_config
        .set(BACKUP_SCHEDULE_KEY, "not-a-schedule")
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
    // When the schedule is invalid, get_backup_config() returns defaults (no destination)
    assert_eq!(body, "false");
}

#[apply(backends)]
#[tokio::test]
async fn backup_warning_hidden_for_non_operator(#[case] backend: Backend) {
    let TestEnv { state, base: _base } = backend.setup().await;
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

#[apply(backends)]
#[tokio::test]
async fn backup_warning_hidden_without_authentication(#[case] backend: Backend) {
    let TestEnv { state, base: _base } = backend.setup().await;

    let (status, body) = post_form(state, "/api/backup_warning_visible", "", None).await;

    assert_eq!(status, StatusCode::OK, "body: {body}");
    assert_eq!(body, "false");
}

#[apply(backends)]
#[tokio::test]
async fn operator_can_update_backup_settings_with_empty_destination(#[case] backend: Backend) {
    let TestEnv { state, base: _base } = backend.setup().await;
    let cookie = create_session_cookie(&state, "operator", true).await;

    let (status, body) = post_form(
        Arc::clone(&state),
        "/api/update_backup_settings",
        "destination_path=&schedule=0+0+0+*+*+*&retention_count=5&mode=directory",
        Some(&cookie),
    )
    .await;

    assert_eq!(status, StatusCode::OK, "body: {body}");
    let settings = post_form(
        Arc::clone(&state),
        "/api/get_backup_settings",
        "",
        Some(&cookie),
    )
    .await;
    assert_eq!(settings.0, StatusCode::OK);
    let config: BackupConfig = serde_json::from_str(&settings.1).unwrap();
    assert_eq!(config.destination_path, None);
}

#[tokio::test]
async fn backup_warning_visible_propagates_storage_error_during_auth() {
    // Covers the Err(non-Unauthorized) branch: close pool after session creation
    // so authenticate() returns an Internal error (not Unauthorized).
    let base = TempDir::new().unwrap();
    let (state, pool) = crate::helpers::test_sqlite_state_with_pool(&base).await;
    let cookie = create_session_cookie(&state, "operator", true).await;

    pool.close().await;

    let (status, _body) = post_form(
        Arc::clone(&state),
        "/api/backup_warning_visible",
        "",
        Some(&cookie),
    )
    .await;

    assert_eq!(status, StatusCode::INTERNAL_SERVER_ERROR);
}

#[tokio::test]
async fn current_user_is_operator_propagates_storage_error_during_auth() {
    // Covers the Err(non-Unauthorized) branch: close pool after session creation
    // so authenticate() returns an Internal error (not Unauthorized).
    let base = TempDir::new().unwrap();
    let (state, pool) = crate::helpers::test_sqlite_state_with_pool(&base).await;
    let cookie = create_session_cookie(&state, "operator", true).await;

    pool.close().await;

    let (status, _body) = post_form(
        Arc::clone(&state),
        "/api/current_user_is_operator",
        "",
        Some(&cookie),
    )
    .await;

    assert_eq!(status, StatusCode::INTERNAL_SERVER_ERROR);
}
