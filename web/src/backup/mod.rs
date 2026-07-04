use crate::error::WebResult;
use common::backup::BackupConfig;
use leptos::prelude::*;

#[cfg(feature = "server")]
pub(crate) mod server;
#[cfg(feature = "server")]
use server::require_operator;

#[cfg(feature = "server")]
use {
    crate::auth::require_auth,
    crate::error::{InternalError, WebError},
    common::backup::{BackupMode, BackupSchedule},
    std::sync::Arc,
    storage::{SiteConfigStorage, UserStorage},
};

#[server(endpoint = "/backup_warning_visible")]
#[cfg_attr(
    feature = "server",
    tracing::instrument(name = "web.backup.warning_visible")
)]
pub async fn backup_warning_visible() -> WebResult<bool> {
    boundary!("backup_warning_visible", {
        let auth = match require_auth().await {
            Ok(auth) => auth,
            Err(error) if matches!(error.public(), WebError::Unauthorized) => return Ok(false),
            Err(error) => return Err(error),
        };

        let users = expect_context::<Arc<dyn UserStorage>>();
        let site_config = expect_context::<Arc<dyn SiteConfigStorage>>();
        let is_operator = users
            .get_user(auth.user_id)
            .await
            .map_err(InternalError::storage)?
            .is_some_and(|u| u.is_operator);

        if !is_operator {
            return Ok(false);
        }

        let config = site_config
            .get_backup_config()
            .await
            .map_err(InternalError::storage)?;

        Ok(config.destination_path.is_none())
    })
}

#[server(endpoint = "/current_user_is_operator")]
#[cfg_attr(
    feature = "server",
    tracing::instrument(name = "web.backup.current_user_is_operator")
)]
pub async fn current_user_is_operator() -> WebResult<bool> {
    boundary!("current_user_is_operator", {
        let auth = match require_auth().await {
            Ok(auth) => auth,
            Err(error) if matches!(error.public(), WebError::Unauthorized) => return Ok(false),
            Err(error) => return Err(error),
        };

        let users = expect_context::<Arc<dyn UserStorage>>();
        Ok(users
            .get_user(auth.user_id)
            .await
            .map_err(InternalError::storage)?
            .is_some_and(|u| u.is_operator))
    })
}

#[server(endpoint = "/get_backup_settings")]
#[cfg_attr(
    feature = "server",
    tracing::instrument(name = "web.backup.get_settings")
)]
pub async fn get_backup_settings() -> WebResult<BackupConfig> {
    boundary!("get_backup_settings", {
        require_operator().await?;
        let site_config = expect_context::<Arc<dyn SiteConfigStorage>>();
        site_config
            .get_backup_config()
            .await
            .map_err(InternalError::storage)
    })
}

#[server(endpoint = "/update_backup_settings")]
#[cfg_attr(
    feature = "server",
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
    boundary!("update_backup_settings", {
        require_operator().await?;

        let schedule = BackupSchedule::parse(schedule.trim()).ok_or_else(|| {
            InternalError::validation("backup schedule must be a valid six-field cron expression")
        })?;
        let retention_count = retention_count.trim().parse::<usize>().map_err(|_| {
            InternalError::validation("backup retention count must be a non-negative integer")
        })?;
        let mode = match mode.trim() {
            "directory" => BackupMode::Directory,
            "archive" => BackupMode::Archive,
            _ => {
                return Err(InternalError::validation(
                    "backup mode must be directory or archive",
                ))
            }
        };
        let destination_path = common::text::non_empty(&destination_path).map(str::to_owned);

        let config = BackupConfig {
            destination_path,
            schedule,
            retention_count,
            mode,
        };
        let site_config = expect_context::<Arc<dyn SiteConfigStorage>>();
        site_config
            .set_backup_config(&config)
            .await
            .map_err(InternalError::storage)
    })
}
