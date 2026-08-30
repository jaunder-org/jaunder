use crate::error::WebResult;
// `BackupSchedule`/`BackupMode`/`RetentionCount` are unconditional: they're the typed
// `#[server]` arguments, so the generated request struct must carry them on both the client
// (serialize) and server (deserialize) sides.
use common::backup::{BackupConfig, BackupMode, BackupSchedule, DestinationPath, RetentionCount};

#[cfg(feature = "server")]
use {
    crate::auth, crate::error::InternalError, leptos::prelude::*, std::sync::Arc,
    storage::SiteConfigStorage,
};

#[macros::server]
pub async fn is_warning_visible() -> WebResult<bool> {
    if !auth::is_operator_soft().await? {
        return Ok(false);
    }
    let site_config = expect_context::<Arc<dyn SiteConfigStorage>>();
    let config = site_config.get_backup_config().await?;
    Ok(config.destination_path.is_none())
}

#[macros::server]
pub async fn get_settings() -> WebResult<BackupConfig> {
    auth::require_operator().await?;
    let site_config = expect_context::<Arc<dyn SiteConfigStorage>>();
    site_config
        .get_backup_config()
        .await
        .map_err(InternalError::storage)
}

#[macros::server]
pub async fn update_settings(
    destination_path: Option<DestinationPath>,
    schedule: BackupSchedule,
    retention_count: RetentionCount,
    mode: BackupMode,
) -> WebResult<()> {
    auth::require_operator().await?;

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
}
