#[allow(dead_code)]
use crate::error::WebResult;
use croner::Cron;
use leptos::prelude::*;
use serde::{Deserialize, Serialize};

#[cfg(feature = "ssr")]
use crate::{
    auth::require_auth,
    error::{InternalError, InternalResult, WebError},
};
#[cfg(feature = "ssr")]
use common::storage::{
    AppState, BACKUP_DESTINATION_PATH_KEY, BACKUP_MODE_KEY, BACKUP_RETENTION_COUNT_KEY,
    BACKUP_SCHEDULE_KEY,
};
#[cfg(feature = "ssr")]
use std::sync::Arc;

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct BackupSettings {
    pub destination_path: String,
    pub schedule: String,
    pub retention_count: String,
    pub mode: String,
}

fn backup_destination_configured(destination: Option<&str>) -> bool {
    destination
        .map(str::trim)
        .is_some_and(|value| !value.is_empty())
}

fn default_backup_schedule() -> String {
    "0 0 0 * * *".to_owned()
}

fn default_backup_retention_count() -> String {
    "7".to_owned()
}

fn default_backup_mode() -> String {
    "directory".to_owned()
}

fn backup_retention_count_valid(retention_count: &str) -> bool {
    retention_count.trim().parse::<usize>().is_ok()
}

fn backup_schedule_valid(schedule: &str) -> bool {
    Cron::new(schedule.trim())
        .with_seconds_required()
        .parse()
        .is_ok()
}

fn backup_mode_valid(mode: &str) -> bool {
    matches!(mode.trim(), "directory" | "archive")
}

fn backup_schedule_value(value: Option<String>) -> String {
    value
        .filter(|value| backup_schedule_valid(value))
        .map_or_else(default_backup_schedule, |value| value.trim().to_owned())
}

fn backup_retention_count_value(value: Option<String>) -> String {
    value
        .filter(|value| backup_retention_count_valid(value))
        .map_or_else(default_backup_retention_count, |value| {
            value.trim().to_owned()
        })
}

fn backup_mode_value(value: Option<String>) -> String {
    value
        .filter(|value| backup_mode_valid(value))
        .map_or_else(default_backup_mode, |value| value.trim().to_owned())
}

fn optional_backup_schedule_valid(value: Option<&str>) -> bool {
    value.is_none_or(backup_schedule_valid)
}

fn optional_backup_retention_count_valid(value: Option<&str>) -> bool {
    value.is_none_or(backup_retention_count_valid)
}

fn optional_backup_mode_valid(value: Option<&str>) -> bool {
    value.is_none_or(backup_mode_valid)
}

fn backup_configuration_complete_and_valid(
    destination_path: Option<&str>,
    schedule: Option<&str>,
    retention_count: Option<&str>,
    mode: Option<&str>,
) -> bool {
    backup_destination_configured(destination_path)
        && optional_backup_schedule_valid(schedule)
        && optional_backup_retention_count_valid(retention_count)
        && optional_backup_mode_valid(mode)
}

#[cfg(feature = "ssr")]
async fn require_operator() -> InternalResult<Arc<AppState>> {
    let auth = require_auth().await?;
    let state = expect_context::<Arc<AppState>>();
    let Some(user) = state
        .users
        .get_user(auth.user_id)
        .await
        .map_err(InternalError::storage)?
    else {
        return Err(InternalError::unauthorized("user does not exist"));
    };

    if !user.is_operator {
        return Err(InternalError::unauthorized("operator access required"));
    }

    Ok(state)
}

#[server(endpoint = "/backup_warning_visible")]
#[cfg_attr(
    feature = "ssr",
    tracing::instrument(name = "web.backup.warning_visible")
)]
pub async fn backup_warning_visible() -> WebResult<bool> {
    crate::web_server_fn!("backup_warning_visible", => {
        let auth = match require_auth().await {
            Ok(auth) => auth,
            Err(error) if matches!(error.public(), WebError::Unauthorized) => return Ok(false),
            Err(error) => return Err(error),
        };

        let state = expect_context::<Arc<AppState>>();
        let Some(user) = state
            .users
            .get_user(auth.user_id)
            .await
            .map_err(InternalError::storage)?
        else {
            return Ok(false);
        };

        if !user.is_operator {
            return Ok(false);
        }

        let destination_path = state
            .site_config
            .get(BACKUP_DESTINATION_PATH_KEY)
            .await
            .map_err(InternalError::storage)?;
        let schedule = state
            .site_config
            .get(BACKUP_SCHEDULE_KEY)
            .await
            .map_err(InternalError::storage)?;
        let retention_count = state
            .site_config
            .get(BACKUP_RETENTION_COUNT_KEY)
            .await
            .map_err(InternalError::storage)?;
        let mode = state
            .site_config
            .get(BACKUP_MODE_KEY)
            .await
            .map_err(InternalError::storage)?;

        Ok(!backup_configuration_complete_and_valid(
            destination_path.as_deref(),
            schedule.as_deref(),
            retention_count.as_deref(),
            mode.as_deref(),
        ))
    })
}

#[server(endpoint = "/current_user_is_operator")]
#[cfg_attr(
    feature = "ssr",
    tracing::instrument(name = "web.backup.current_user_is_operator")
)]
pub async fn current_user_is_operator() -> WebResult<bool> {
    crate::web_server_fn!("current_user_is_operator", => {
        let auth = match require_auth().await {
            Ok(auth) => auth,
            Err(error) if matches!(error.public(), WebError::Unauthorized) => return Ok(false),
            Err(error) => return Err(error),
        };

        let state = expect_context::<Arc<AppState>>();
        let Some(user) = state
            .users
            .get_user(auth.user_id)
            .await
            .map_err(InternalError::storage)?
        else {
            return Ok(false);
        };

        Ok(user.is_operator)
    })
}

#[server(endpoint = "/get_backup_settings")]
#[cfg_attr(feature = "ssr", tracing::instrument(name = "web.backup.get_settings"))]
pub async fn get_backup_settings() -> WebResult<BackupSettings> {
    crate::web_server_fn!("get_backup_settings", => {
        let state = require_operator().await?;
        let destination_path = state
            .site_config
            .get(BACKUP_DESTINATION_PATH_KEY)
            .await
            .map_err(InternalError::storage)?
            .unwrap_or_default();
        let schedule = backup_schedule_value(state
            .site_config
            .get(BACKUP_SCHEDULE_KEY)
            .await
            .map_err(InternalError::storage)?);
        let retention_count = backup_retention_count_value(state
            .site_config
            .get(BACKUP_RETENTION_COUNT_KEY)
            .await
            .map_err(InternalError::storage)?);
        let mode = backup_mode_value(state
            .site_config
            .get(BACKUP_MODE_KEY)
            .await
            .map_err(InternalError::storage)?);

        Ok(BackupSettings {
            destination_path,
            schedule,
            retention_count,
            mode,
        })
    })
}

#[server(endpoint = "/update_backup_settings")]
#[cfg_attr(
    feature = "ssr",
    tracing::instrument(
        name = "web.backup.update_settings",
        skip(destination_path, schedule, retention_count, mode)
    )
)]
pub async fn update_backup_settings(
    destination_path: String,
    schedule: String,
    retention_count: String,
    mode: String,
) -> WebResult<()> {
    crate::web_server_fn!("update_backup_settings", destination_path, schedule, retention_count, mode => {
        let state = require_operator().await?;
        let destination_path = destination_path.trim();
        let schedule = schedule.trim();
        let retention_count = retention_count.trim();
        let mode = mode.trim();

        if !backup_schedule_valid(schedule) {
            return Err(InternalError::validation(
                "backup schedule must be a valid six-field cron expression",
            ));
        }
        if !backup_retention_count_valid(retention_count) {
            return Err(InternalError::validation(
                "backup retention count must be a non-negative integer",
            ));
        }
        if !backup_mode_valid(mode) {
            return Err(InternalError::validation(
                "backup mode must be directory or archive",
            ));
        }

        state
            .site_config
            .set(BACKUP_DESTINATION_PATH_KEY, destination_path)
            .await
            .map_err(InternalError::storage)?;
        state
            .site_config
            .set(BACKUP_SCHEDULE_KEY, schedule)
            .await
            .map_err(InternalError::storage)?;
        state
            .site_config
            .set(BACKUP_RETENTION_COUNT_KEY, retention_count)
            .await
            .map_err(InternalError::storage)?;
        state
            .site_config
            .set(BACKUP_MODE_KEY, mode)
            .await
            .map_err(InternalError::storage)?;

        Ok(())
    })
}

#[cfg(test)]
mod tests {
    use super::{
        backup_configuration_complete_and_valid, backup_destination_configured, backup_mode_valid,
        backup_mode_value, backup_retention_count_valid, backup_retention_count_value,
        backup_schedule_valid, backup_schedule_value,
    };

    #[test]
    fn backup_destination_configured_rejects_empty_values() {
        assert!(!backup_destination_configured(None));
        assert!(!backup_destination_configured(Some("")));
        assert!(!backup_destination_configured(Some("  ")));
    }

    #[test]
    fn backup_destination_configured_accepts_nonempty_values() {
        assert!(backup_destination_configured(Some("/srv/backups")));
    }

    #[test]
    fn backup_retention_count_valid_accepts_non_negative_integers() {
        assert!(backup_retention_count_valid("0"));
        assert!(backup_retention_count_valid("7"));
    }

    #[test]
    fn backup_retention_count_valid_rejects_invalid_values() {
        assert!(!backup_retention_count_valid(""));
        assert!(!backup_retention_count_valid("-1"));
        assert!(!backup_retention_count_valid("daily"));
    }

    #[test]
    fn backup_schedule_valid_accepts_nonempty_values() {
        assert!(backup_schedule_valid("0 0 0 * * *"));
        assert!(backup_schedule_valid("0 */15 1-4 * * MON-FRI"));
    }

    #[test]
    fn backup_schedule_valid_rejects_invalid_values() {
        assert!(!backup_schedule_valid(""));
        assert!(!backup_schedule_valid("not a schedule"));
        assert!(!backup_schedule_valid("* * * * *"));
        assert!(!backup_schedule_valid("99 0 0 * * *"));
    }

    #[test]
    fn backup_schedule_value_uses_default_for_invalid_values() {
        assert_eq!(backup_schedule_value(None), "0 0 0 * * *");
        assert_eq!(
            backup_schedule_value(Some("not a schedule".to_owned())),
            "0 0 0 * * *"
        );
        assert_eq!(
            backup_schedule_value(Some(" 0 15 2 * * * ".to_owned())),
            "0 15 2 * * *"
        );
    }

    #[test]
    fn backup_mode_valid_accepts_supported_modes() {
        assert!(backup_mode_valid("directory"));
        assert!(backup_mode_valid("archive"));
    }

    #[test]
    fn backup_mode_valid_rejects_unsupported_modes() {
        assert!(!backup_mode_valid(""));
        assert!(!backup_mode_valid("tar.gz"));
        assert!(!backup_mode_valid("postgres"));
    }

    #[test]
    fn backup_setting_values_use_defaults_for_invalid_values() {
        assert_eq!(backup_retention_count_value(None), "7");
        assert_eq!(backup_retention_count_value(Some("daily".to_owned())), "7");
        assert_eq!(backup_retention_count_value(Some(" 5 ".to_owned())), "5");
        assert_eq!(backup_mode_value(None), "directory");
        assert_eq!(backup_mode_value(Some("surprise".to_owned())), "directory");
        assert_eq!(backup_mode_value(Some(" archive ".to_owned())), "archive");
    }

    #[test]
    fn backup_configuration_complete_and_valid_rejects_invalid_stored_values() {
        assert!(backup_configuration_complete_and_valid(
            Some("/srv/backups"),
            None,
            None,
            None,
        ));
        assert!(!backup_configuration_complete_and_valid(
            None, None, None, None,
        ));
        assert!(!backup_configuration_complete_and_valid(
            Some("/srv/backups"),
            Some("not a schedule"),
            None,
            None,
        ));
        assert!(!backup_configuration_complete_and_valid(
            Some("/srv/backups"),
            None,
            Some("daily"),
            None,
        ));
        assert!(!backup_configuration_complete_and_valid(
            Some("/srv/backups"),
            None,
            None,
            Some("surprise"),
        ));
    }
}
