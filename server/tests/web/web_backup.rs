use std::sync::Arc;

use axum::http::StatusCode;
use common::backup::{BackupConfig, BackupMode};
use storage::{
    BACKUP_DESTINATION_PATH_KEY, BACKUP_MODE_KEY, BACKUP_RETENTION_COUNT_KEY, BACKUP_SCHEDULE_KEY,
};

use rstest::*;
use rstest_reuse::*;

use crate::helpers::{create_operator_and_session, create_user_and_session, post_form};
use storage::test_support::{backends, backends_matrix, Backend, TestEnv};

#[apply(backends)]
#[tokio::test]
async fn operator_gets_default_backup_settings(#[case] backend: Backend) {
    let TestEnv { state, base: _base } = backend.setup().await;
    let cookie = create_operator_and_session(&state, "operator")
        .await
        .cookie();

    let (status, body) = post_form(state, "/api/get_backup_settings", "", Some(&cookie)).await;

    assert_eq!(status, StatusCode::OK, "body: {body}");
    let settings: BackupConfig = serde_json::from_str(&body).unwrap();
    assert_eq!(settings.destination_path, None);
    assert_eq!(settings.schedule, "0 0 0 * * *");
    assert_eq!(settings.retention_count.value(), 7);
    assert_eq!(settings.mode, BackupMode::Directory);
}

#[apply(backends)]
#[tokio::test]
async fn operator_gets_configured_backup_settings(#[case] backend: Backend) {
    let TestEnv { state, base: _base } = backend.setup().await;
    let cookie = create_operator_and_session(&state, "operator")
        .await
        .cookie();
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
    assert_eq!(settings.destination_path.as_deref(), Some("/srv/backups"));
    assert_eq!(settings.schedule, "0 30 2 * * *");
    assert_eq!(settings.retention_count.value(), 4);
    assert_eq!(settings.mode, BackupMode::Archive);
}

#[apply(backends)]
#[tokio::test]
async fn operator_gets_defaults_for_invalid_backup_settings(#[case] backend: Backend) {
    let TestEnv { state, base: _base } = backend.setup().await;
    let cookie = create_operator_and_session(&state, "operator")
        .await
        .cookie();
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
    assert_eq!(settings.destination_path.as_deref(), Some("/srv/backups"));
    assert_eq!(settings.schedule, "0 0 0 * * *");
    assert_eq!(settings.retention_count.value(), 7);
    assert_eq!(settings.mode, BackupMode::Directory);
}

#[apply(backends)]
#[tokio::test]
async fn operator_can_update_backup_settings(#[case] backend: Backend) {
    let TestEnv { state, base: _base } = backend.setup().await;
    let cookie = create_operator_and_session(&state, "operator")
        .await
        .cookie();

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
    let cookie = create_operator_and_session(&state, "operator")
        .await
        .cookie();

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

// `schedule` (`BackupSchedule`, #453), `mode` (`BackupMode`, #454), and `retention_count`
// (`RetentionCount`, #455) are all typed wire args (ADR-0065): an invalid value is rejected when
// the request struct deserializes — before the fn body — so the guard is the type, not an
// in-body validation string. Assert the request is refused (non-OK), not a specific message,
// since the message is now the framework's, not ours. `retention_count=0` is rejected because
// `RetentionCount`'s min-1 invariant makes the prune-everything footgun unrepresentable.
// (`destination_path` is *not* here: it is optional, and an empty value decodes to `None` — a
// clear, not a rejection — covered by the two clear tests below.)
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
#[case::invalid_retention_count(
    "destination_path=%2Fsrv%2Fbackups&schedule=0+0+0+*+*+*&retention_count=bogus&mode=directory"
)]
#[case::zero_retention_count(
    "destination_path=%2Fsrv%2Fbackups&schedule=0+0+0+*+*+*&retention_count=0&mode=directory"
)]
#[tokio::test]
async fn operator_update_backup_settings_rejects_invalid_typed_arg(
    backend: Backend,
    #[case] form: &str,
) {
    let TestEnv { state, base: _base } = backend.setup().await;
    let cookie = create_operator_and_session(&state, "operator")
        .await
        .cookie();

    let (status, body) = post_form(state, "/api/update_backup_settings", form, Some(&cookie)).await;

    assert_ne!(status, StatusCode::OK, "body: {body}");
}

#[apply(backends)]
#[tokio::test]
async fn non_operator_cannot_update_backup_settings(#[case] backend: Backend) {
    let TestEnv { state, base: _base } = backend.setup().await;
    let cookie = create_user_and_session(&state, "member").await.cookie();

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

#[apply(backends)]
#[tokio::test]
async fn backup_warning_visible_for_operator_without_destination(#[case] backend: Backend) {
    let TestEnv { state, base: _base } = backend.setup().await;
    let cookie = create_operator_and_session(&state, "operator")
        .await
        .cookie();

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
    let cookie = create_operator_and_session(&state, "operator")
        .await
        .cookie();
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
    let cookie = create_operator_and_session(&state, "operator")
        .await
        .cookie();
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
    let cookie = create_user_and_session(&state, "member").await.cookie();

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

// Clearing the destination has two equivalent wire forms, both decoding the optional
// `Option<DestinationPath>` arg to `None`: *omitting* `destination_path` (the dispatch-`None`
// path the browser client sends — this test) and an empty `destination_path=` value (the
// `clears_via_empty_destination` test below). Neither reaches a parse error. Mirrors
// `web_site.rs::update_site_identity_omits_base_url_as_none`.
#[apply(backends)]
#[tokio::test]
async fn operator_can_update_backup_settings_omits_destination_as_none(#[case] backend: Backend) {
    let TestEnv { state, base: _base } = backend.setup().await;
    let cookie = create_operator_and_session(&state, "operator")
        .await
        .cookie();

    let (status, body) = post_form(
        Arc::clone(&state),
        "/api/update_backup_settings",
        "schedule=0+0+0+*+*+*&retention_count=5&mode=directory",
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

// The other clear form: an empty `destination_path=` value (present, not omitted) also decodes
// the optional arg to `None` — so submitting the form with a blanked-out destination clears it
// rather than erroring, preserving the pre-typing behavior for non-omitting callers.
#[apply(backends)]
#[tokio::test]
async fn operator_can_update_backup_settings_clears_via_empty_destination(
    #[case] backend: Backend,
) {
    let TestEnv { state, base: _base } = backend.setup().await;
    let cookie = create_operator_and_session(&state, "operator")
        .await
        .cookie();

    let (status, body) = post_form(
        Arc::clone(&state),
        "/api/update_backup_settings",
        "destination_path=&schedule=0+0+0+*+*+*&retention_count=5&mode=directory",
        Some(&cookie),
    )
    .await;

    assert_eq!(status, StatusCode::OK, "body: {body}");
    let (get_status, get_body) = post_form(
        Arc::clone(&state),
        "/api/get_backup_settings",
        "",
        Some(&cookie),
    )
    .await;
    assert_eq!(get_status, StatusCode::OK);
    let config: BackupConfig = serde_json::from_str(&get_body).unwrap();
    assert_eq!(config.destination_path, None);
}

#[apply(backends)]
#[tokio::test]
async fn backup_warning_visible_propagates_storage_error_during_auth(#[case] backend: Backend) {
    // Covers the Err(non-Unauthorized) branch: close the pool after session
    // creation so authenticate() returns Internal (not Unauthorized) → 500.
    let TestEnv { state, base } = backend.setup().await;
    let cookie = create_operator_and_session(&state, "operator")
        .await
        .cookie();

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

// #591: `session()` is the single reconcile fetch behind the shared session context
// — it reports the viewer's username + operator flag, or `null` when anonymous. This
// replaces the retired `current_user` / `current_user_is_operator` endpoint coverage.
#[apply(backends)]
#[tokio::test]
async fn session_reports_username_and_operator(#[case] backend: Backend) {
    let TestEnv { state, base: _base } = backend.setup().await;
    let operator_cookie = create_operator_and_session(&state, "operator")
        .await
        .cookie();
    let member_cookie = create_user_and_session(&state, "member").await.cookie();

    let (status, body) = post_form(
        Arc::clone(&state),
        "/api/session",
        "",
        Some(&operator_cookie),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "body: {body}");
    assert!(body.contains(r#""username":"operator""#), "body: {body}");
    assert!(body.contains(r#""is_operator":true"#), "body: {body}");

    let (status, body) =
        post_form(Arc::clone(&state), "/api/session", "", Some(&member_cookie)).await;
    assert_eq!(status, StatusCode::OK, "body: {body}");
    assert!(body.contains(r#""username":"member""#), "body: {body}");
    assert!(body.contains(r#""is_operator":false"#), "body: {body}");

    let (status, body) = post_form(state, "/api/session", "", None).await;
    assert_eq!(status, StatusCode::OK, "body: {body}");
    assert_eq!(body.trim(), "null"); // Ok(None) serializes to JSON null
}

#[apply(backends)]
#[tokio::test]
async fn session_propagates_storage_error_during_auth(#[case] backend: Backend) {
    // Covers the Err(non-Unauthorized) branch: close the pool after session
    // creation so authenticate() returns Internal (not Unauthorized) → 500.
    let TestEnv { state, base } = backend.setup().await;
    let cookie = create_operator_and_session(&state, "operator")
        .await
        .cookie();

    base.close_pool().await;

    let (status, _body) = post_form(Arc::clone(&state), "/api/session", "", Some(&cookie)).await;

    assert_eq!(status, StatusCode::INTERNAL_SERVER_ERROR);
}
