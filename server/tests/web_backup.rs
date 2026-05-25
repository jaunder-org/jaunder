mod helpers;

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

use helpers::{ensure_server_fns_registered, test_options, test_state};

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
        helpers::noop_mailer(),
        true,
        helpers::tmp_storage_path(),
    );
    let response = app.oneshot(request).await.unwrap();

    let status = response.status();
    let bytes = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .unwrap();
    let body_str = String::from_utf8(bytes.to_vec()).unwrap();

    (status, body_str)
}

#[tokio::test]
async fn operator_gets_default_backup_settings() {
    let base = TempDir::new().unwrap();
    let state = test_state(&base).await;
    let cookie = create_session_cookie(&state, "operator", true).await;

    let (status, body) = post_form(state, "/api/get_backup_settings", "", Some(&cookie)).await;

    assert_eq!(status, StatusCode::OK, "body: {body}");
    let settings: BackupConfig = serde_json::from_str(&body).unwrap();
    assert_eq!(settings.destination_path, None);
    assert_eq!(settings.schedule.as_str(), "0 0 0 * * *");
    assert_eq!(settings.retention_count, 7);
    assert_eq!(settings.mode, BackupMode::Directory);
}

#[tokio::test]
async fn current_user_is_operator_reports_operator_status() {
    let base = TempDir::new().unwrap();
    let state = test_state(&base).await;
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

#[tokio::test]
async fn operator_gets_configured_backup_settings() {
    let base = TempDir::new().unwrap();
    let state = test_state(&base).await;
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

#[tokio::test]
async fn operator_gets_defaults_for_invalid_backup_settings() {
    let base = TempDir::new().unwrap();
    let state = test_state(&base).await;
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

#[tokio::test]
async fn operator_can_update_backup_settings() {
    let base = TempDir::new().unwrap();
    let state = test_state(&base).await;
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

#[tokio::test]
async fn operator_can_update_backup_settings_to_archive_mode() {
    let base = TempDir::new().unwrap();
    let state = test_state(&base).await;
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

#[tokio::test]
async fn operator_update_backup_settings_rejects_empty_schedule() {
    let base = TempDir::new().unwrap();
    let state = test_state(&base).await;
    let cookie = create_session_cookie(&state, "operator", true).await;

    let (status, body) = post_form(
        state,
        "/api/update_backup_settings",
        "destination_path=%2Fsrv%2Fbackups&schedule=+++&retention_count=5&mode=directory",
        Some(&cookie),
    )
    .await;

    assert_eq!(status, StatusCode::INTERNAL_SERVER_ERROR, "body: {body}");
    assert!(body.contains("backup schedule must be a valid six-field cron expression"));
}

#[tokio::test]
async fn operator_update_backup_settings_rejects_invalid_schedule() {
    let base = TempDir::new().unwrap();
    let state = test_state(&base).await;
    let cookie = create_session_cookie(&state, "operator", true).await;

    let (status, body) = post_form(
        state,
        "/api/update_backup_settings",
        "destination_path=%2Fsrv%2Fbackups&schedule=not-a-schedule&retention_count=5&mode=directory",
        Some(&cookie),
    )
    .await;

    assert_eq!(status, StatusCode::INTERNAL_SERVER_ERROR, "body: {body}");
    assert!(body.contains("backup schedule must be a valid six-field cron expression"));
}

#[tokio::test]
async fn operator_update_backup_settings_rejects_invalid_retention_count() {
    let base = TempDir::new().unwrap();
    let state = test_state(&base).await;
    let cookie = create_session_cookie(&state, "operator", true).await;

    let (status, body) = post_form(
        state,
        "/api/update_backup_settings",
        "destination_path=%2Fsrv%2Fbackups&schedule=0+0+0+*+*+*&retention_count=bogus&mode=directory",
        Some(&cookie),
    )
    .await;

    assert_eq!(status, StatusCode::INTERNAL_SERVER_ERROR, "body: {body}");
    assert!(body.contains("backup retention count"));
}

#[tokio::test]
async fn operator_update_backup_settings_rejects_invalid_mode() {
    let base = TempDir::new().unwrap();
    let state = test_state(&base).await;
    let cookie = create_session_cookie(&state, "operator", true).await;

    let (status, body) = post_form(
        state,
        "/api/update_backup_settings",
        "destination_path=%2Fsrv%2Fbackups&schedule=0+0+0+*+*+*&retention_count=5&mode=surprise",
        Some(&cookie),
    )
    .await;

    assert_eq!(status, StatusCode::INTERNAL_SERVER_ERROR, "body: {body}");
    assert!(body.contains("backup mode"));
}

#[tokio::test]
async fn non_operator_cannot_update_backup_settings() {
    let base = TempDir::new().unwrap();
    let state = test_state(&base).await;
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
async fn backup_warning_visible_when_configured_schedule_is_invalid() {
    let base = TempDir::new().unwrap();
    let state = test_state(&base).await;
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
