use crate::error::WebResult;
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

fn backup_retention_count_valid(retention_count: &str) -> bool {
    retention_count.parse::<usize>().is_ok()
}

fn backup_schedule_valid(schedule: &str) -> bool {
    !schedule.is_empty()
}

fn backup_mode_valid(mode: &str) -> bool {
    matches!(mode, "directory" | "archive")
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

        let destination = state
            .site_config
            .get(BACKUP_DESTINATION_PATH_KEY)
            .await
            .map_err(InternalError::storage)?;

        Ok(!backup_destination_configured(destination.as_deref()))
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
        let schedule = state
            .site_config
            .get(BACKUP_SCHEDULE_KEY)
            .await
            .map_err(InternalError::storage)?
            .unwrap_or_else(|| "0 0 0 * * *".to_owned());
        let retention_count = state
            .site_config
            .get(BACKUP_RETENTION_COUNT_KEY)
            .await
            .map_err(InternalError::storage)?
            .unwrap_or_else(|| "7".to_owned());
        let mode = state
            .site_config
            .get(BACKUP_MODE_KEY)
            .await
            .map_err(InternalError::storage)?
            .unwrap_or_else(|| "directory".to_owned());

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
            return Err(InternalError::validation("backup schedule is required"));
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
        backup_destination_configured, backup_mode_valid, backup_retention_count_valid,
        backup_schedule_valid,
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
    }

    #[test]
    fn backup_schedule_valid_rejects_empty_values() {
        assert!(!backup_schedule_valid(""));
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
}
