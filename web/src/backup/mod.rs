use crate::error::WebResult;
// `BackupSchedule`/`BackupMode`/`RetentionCount` are unconditional: they're the typed
// `#[server]` arguments, so the generated request struct must carry them on both the client
// (serialize) and server (deserialize) sides.
use common::backup::{BackupConfig, BackupMode, BackupSchedule, RetentionCount};
use leptos::prelude::*;

// Reactive UI co-located with its server fns + wire types (ADR-0056). Unconditional (no
// `target_arch` gate): the `#[component]`s host-compile for coverage, as the audiences
// vertical's components do (ungated, host-compiled — audiences keeps them inline in its
// mod.rs; backup splits them into this `ui` submodule).
mod ui;
pub use ui::{BackupBanner, BackupSettingsPage};

#[cfg(feature = "server")]
pub(crate) mod server;
#[cfg(feature = "server")]
use server::require_operator;

#[cfg(feature = "server")]
use {
    crate::auth::require_auth,
    crate::error::{ErrorKind, InternalError},
    std::sync::Arc,
    storage::{SiteConfigStorage, UserStorage},
};

#[server(endpoint = "/backup_warning_visible")]
#[tracing::instrument(name = "web.backup.warning_visible")]
pub async fn backup_warning_visible() -> WebResult<bool> {
    boundary!("backup_warning_visible", {
        let auth = match require_auth().await {
            Ok(auth) => auth,
            Err(error) if error.kind() == ErrorKind::Auth => return Ok(false),
            Err(error) => return Err(error),
        };

        let users = expect_context::<Arc<dyn UserStorage>>();
        let site_config = expect_context::<Arc<dyn SiteConfigStorage>>();
        let is_operator = users
            .get_user(auth.user_id)
            .await?
            .is_some_and(|u| u.is_operator);

        if !is_operator {
            return Ok(false);
        }

        let config = site_config.get_backup_config().await?;

        Ok(config.destination_path.is_none())
    })
}

#[server(endpoint = "/current_user_is_operator")]
#[tracing::instrument(name = "web.backup.current_user_is_operator")]
pub async fn current_user_is_operator() -> WebResult<bool> {
    boundary!("current_user_is_operator", {
        let auth = match require_auth().await {
            Ok(auth) => auth,
            Err(error) if error.kind() == ErrorKind::Auth => return Ok(false),
            Err(error) => return Err(error),
        };

        let users = expect_context::<Arc<dyn UserStorage>>();
        Ok(users
            .get_user(auth.user_id)
            .await?
            .is_some_and(|u| u.is_operator))
    })
}

#[server(endpoint = "/get_backup_settings")]
#[tracing::instrument(name = "web.backup.get_settings")]
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
#[tracing::instrument(
    name = "web.backup.update_settings",
    skip(destination_path, schedule, retention_count, mode)
)]
pub async fn update_backup_settings(
    destination_path: String,
    schedule: BackupSchedule,
    retention_count: RetentionCount,
    mode: BackupMode,
) -> WebResult<()> {
    boundary!("update_backup_settings", {
        require_operator().await?;

        // `schedule`, `retention_count`, and `mode` are already validated: they arrive typed
        // (`BackupSchedule` / `RetentionCount` / `BackupMode`), so the arg `Deserialize` ran
        // their `FromStr`/`NonZeroUsize`/enum parse. Legitimate clients only submit valid values
        // (the cron and retention fields pre-validate per ADR-0065, and the mode `<select>` can
        // only emit a real variant), so an invalid value reaches here only from a non-browser
        // caller.
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
