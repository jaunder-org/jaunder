use crate::error::WebResult;
// `BackupSchedule`/`BackupMode`/`RetentionCount` are unconditional: they're the typed
// `#[server]` arguments, so the generated request struct must carry them on both the client
// (serialize) and server (deserialize) sides.
use common::backup::{BackupConfig, BackupMode, BackupSchedule, DestinationPath, RetentionCount};
use leptos::prelude::*;

#[cfg(feature = "server")]
use {
    crate::auth::{is_operator_soft, require_operator},
    crate::error::InternalError,
    std::sync::Arc,
    storage::SiteConfigStorage,
};

#[server(endpoint = "/backup/warning_visible")]
#[tracing::instrument(name = "web.backup.warning_visible")]
pub async fn warning_visible() -> WebResult<bool> {
    boundary!("warning_visible", {
        if !is_operator_soft().await? {
            return Ok(false);
        }
        let site_config = expect_context::<Arc<dyn SiteConfigStorage>>();
        let config = site_config.get_backup_config().await?;
        Ok(config.destination_path.is_none())
    })
}

#[server(endpoint = "/backup/get_settings")]
#[tracing::instrument(name = "web.backup.get_settings")]
pub async fn get_settings() -> WebResult<BackupConfig> {
    boundary!("get_settings", {
        require_operator().await?;
        let site_config = expect_context::<Arc<dyn SiteConfigStorage>>();
        site_config
            .get_backup_config()
            .await
            .map_err(InternalError::storage)
    })
}

#[server(endpoint = "/backup/update_settings")]
#[tracing::instrument(name = "web.backup.update_settings")]
pub async fn update_settings(
    destination_path: Option<DestinationPath>,
    schedule: BackupSchedule,
    retention_count: RetentionCount,
    mode: BackupMode,
) -> WebResult<()> {
    boundary!("update_settings", {
        require_operator().await?;

        // All four fields arrive already validated by the typed arg `Deserialize`: the required
        // `schedule`/`retention_count`/`mode` ran their `FromStr`/min-1-bound/enum parse, and
        // `destination_path` is an `Option<DestinationPath>` — `None` when omitted or blank (both
        // clear the destination), else a non-empty validated path. Legitimate clients pre-validate
        // per ADR-0065, so an invalid value reaches here only from a non-browser caller.
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
