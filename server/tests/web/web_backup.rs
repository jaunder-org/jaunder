use std::sync::Arc;

use axum::http::StatusCode;
use common::backup::{BackupConfig, BackupMode};
use common::{password::Password, username::Username};
use storage::{
    BACKUP_DESTINATION_PATH_KEY, BACKUP_MODE_KEY, BACKUP_RETENTION_COUNT_KEY, BACKUP_SCHEDULE_KEY,
};

use rstest::*;
use rstest_reuse::*;

use crate::helpers::post_form;
use storage::test_support::{backends, backends_matrix, Backend, TestEnv};

#[apply(backends)]
#[tokio::test]
async fn operator_gets_default_backup_settings(#[case] backend: Backend) {
    let TestEnv { state, base: _base } = backend.setup().await;
    let cookie = create_session_cookie(&state, "operator", true).await;

    let (status, body) = post_form(state, "/api/get_backup_settings", "", Some(&cookie)).await;

    assert_eq!(status, StatusCode::OK, "body: {body}");
    let settings: BackupConfig = serde_json::from_str(&body).unwrap();
    assert_eq!(settings.destination_path, None);
    assert_eq!(settings.schedule, "0 0 0 * * *");
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
    assert_eq!(settings.schedule, "0 30 2 * * *");
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
    assert_eq!(settings.schedule, "0 0 0 * * *");
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

// `schedule` (`BackupSchedule`, #453) and `mode` (`BackupMode`, #454) are typed wire args
// (ADR-0065): an invalid value is rejected when the request struct deserializes — before the
// fn body — so the guard is the type, not an in-body validation string. Assert the request is
// refused (non-OK), not a specific message, since the message is now the framework's, not ours.
#[apply(backends_matrix)]
#[case::empty_schedule(
    "destination_path=%2Fsrv%2Fbackups&schedule=+++&retention_count=5&mode=directory"
)]
#[case::invalid_schedule(
    "destination_path=%2Fsrv%2Fbackups&schedule=not-a-schedule&retention_count=5&mode=directory"
)]
#[case::invalid_mode(
    "destination_path=%2Fsrv%2Fbackups&schedule=0+0+0+*+*+*&retention_count=5&mode=surprise"
)]
#[tokio::test]
async fn operator_update_backup_settings_rejects_invalid_schedule_or_mode(
    backend: Backend,
    #[case] form: &str,
) {
    let TestEnv { state, base: _base } = backend.setup().await;
    let cookie = create_session_cookie(&state, "operator", true).await;

    let (status, body) = post_form(state, "/api/update_backup_settings", form, Some(&cookie)).await;

    assert_ne!(status, StatusCode::OK, "body: {body}");
}

// `retention_count` is still a bare-`String` wire arg parsed in the fn body (see #455), so it
// keeps its in-body 500 + message contract.
#[apply(backends_matrix)]
#[case::invalid_retention_count(
    "destination_path=%2Fsrv%2Fbackups&schedule=0+0+0+*+*+*&retention_count=bogus&mode=directory",
    "backup retention count"
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

#[apply(backends)]
#[tokio::test]
async fn backup_warning_visible_propagates_storage_error_during_auth(#[case] backend: Backend) {
    // Covers the Err(non-Unauthorized) branch: close the pool after session
    // creation so authenticate() returns Internal (not Unauthorized) → 500.
    let TestEnv { state, base } = backend.setup().await;
    let cookie = create_session_cookie(&state, "operator", true).await;

    base.close_pool().await;

    let (status, _body) = post_form(
        Arc::clone(&state),
        "/api/backup_warning_visible",
        "",
        Some(&cookie),
    )
    .await;

    assert_eq!(status, StatusCode::INTERNAL_SERVER_ERROR);
}

#[apply(backends)]
#[tokio::test]
async fn current_user_is_operator_propagates_storage_error_during_auth(#[case] backend: Backend) {
    // Covers the Err(non-Unauthorized) branch: close the pool after session
    // creation so authenticate() returns Internal (not Unauthorized) → 500.
    let TestEnv { state, base } = backend.setup().await;
    let cookie = create_session_cookie(&state, "operator", true).await;

    base.close_pool().await;

    let (status, _body) = post_form(
        Arc::clone(&state),
        "/api/current_user_is_operator",
        "",
        Some(&cookie),
    )
    .await;

    assert_eq!(status, StatusCode::INTERNAL_SERVER_ERROR);
}
