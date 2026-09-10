use std::path::PathBuf;

use anyhow::Context;
use common::{
    display_name::DisplayName, email::Email, invite::InviteTtlHours, session_label::SessionLabel,
    tagged_url::HubUrl, username::Username,
};
use host::{config_key::SiteConfigKey, password::Password};
use storage::{BackupRestoreOutcome, FeedWindowMutation, StorageFactory};

use crate::cli::{
    Commands, DeadLetterAction, DeadLetterCursor, SiteConfigAction, StorageArgs, WebsubAction,
};

use super::{
    account, backup,
    lifecycle::{self, ServeCapturePaths},
    shut_down, site_config, storage_bootstrap, support, websub,
};

pub enum CommandOutput {
    None,
    Backup(PathBuf),
    Restore(BackupRestoreOutcome),
}

async fn open_existing_storage(storage: &StorageArgs) -> anyhow::Result<StorageFactory> {
    let runtime = support::storage_runtime_config(&storage.db)?;
    Ok(storage::open_existing_database(&storage.db, &runtime).await?)
}

async fn open_account_storage(storage: &StorageArgs) -> anyhow::Result<StorageFactory> {
    let runtime = support::storage_runtime_config(&storage.db)?;
    storage::open_existing_database(&storage.db, &runtime)
        .await
        .context(support::INIT_FIRST_CONTEXT)
}

async fn execute_user_create(
    storage: StorageArgs,
    username: Username,
    password: Option<Password>,
    display_name: Option<DisplayName>,
    operator: bool,
) -> anyhow::Result<()> {
    let factory = open_account_storage(&storage).await?;
    account::cmd_user_create(
        factory.users(),
        &factory.write_scope(),
        &username,
        password,
        display_name.as_ref(),
        operator,
    )
    .await
}

async fn execute_app_password_create(
    storage: StorageArgs,
    username: Username,
    label: SessionLabel,
) -> anyhow::Result<()> {
    let factory = open_account_storage(&storage).await?;
    account::cmd_app_password_create(
        factory.users(),
        factory.sessions(),
        &factory.write_scope(),
        &username,
        &label,
    )
    .await
}

async fn execute_user_invite(
    storage: StorageArgs,
    expires_in: Option<InviteTtlHours>,
) -> anyhow::Result<()> {
    let factory = open_account_storage(&storage).await?;
    account::cmd_user_invite(
        factory.site_config().as_ref(),
        factory.invites(),
        &factory.write_scope(),
        expires_in,
    )
    .await
}

async fn execute_smtp_test(storage: StorageArgs, to: Email) -> anyhow::Result<()> {
    let factory = open_account_storage(&storage).await?;
    account::cmd_smtp_test(factory.site_config().as_ref(), &to).await
}

async fn execute_site_config_set(
    storage: StorageArgs,
    key: SiteConfigKey,
    value: String,
) -> anyhow::Result<()> {
    key.validate(&value)?;
    let factory = open_existing_storage(&storage).await?;
    match key {
        SiteConfigKey::FeedsMinItems => {
            site_config::cmd_feed_window_set(
                storage.storage_path,
                factory.publisher(),
                factory.write_scope(),
                FeedWindowMutation::SetMinItems(value.parse()?),
                key,
                &value,
            )
            .await
        }
        SiteConfigKey::FeedsMinDays => {
            site_config::cmd_feed_window_set(
                storage.storage_path,
                factory.publisher(),
                factory.write_scope(),
                FeedWindowMutation::SetMinDays(value.parse()?),
                key,
                &value,
            )
            .await
        }
        SiteConfigKey::FeedsWebsubHubUrl => {
            let hub = if value.is_empty() {
                None
            } else {
                Some(value.parse::<HubUrl>()?)
            };
            site_config::cmd_websub_hub_set(
                storage.storage_path,
                factory.publisher(),
                factory.write_scope(),
                hub.as_ref(),
                &value,
            )
            .await
        }
        _ => {
            site_config::cmd_site_config_set(
                factory.site_config(),
                &factory.write_scope(),
                key,
                &value,
            )
            .await
        }
    }
}

async fn execute_site_config_unset(storage: StorageArgs, key: SiteConfigKey) -> anyhow::Result<()> {
    let factory = open_existing_storage(&storage).await?;
    match key {
        SiteConfigKey::FeedsMinItems => {
            site_config::cmd_feed_window_unset(
                storage.storage_path,
                factory.publisher(),
                factory.write_scope(),
                FeedWindowMutation::UnsetMinItems,
                key,
            )
            .await
        }
        SiteConfigKey::FeedsMinDays => {
            site_config::cmd_feed_window_unset(
                storage.storage_path,
                factory.publisher(),
                factory.write_scope(),
                FeedWindowMutation::UnsetMinDays,
                key,
            )
            .await
        }
        SiteConfigKey::FeedsWebsubHubUrl => {
            site_config::cmd_websub_hub_unset(
                storage.storage_path,
                factory.publisher(),
                factory.write_scope(),
            )
            .await
        }
        _ => {
            site_config::cmd_site_config_unset(factory.site_config(), &factory.write_scope(), key)
                .await
        }
    }
}

impl Commands {
    /// Dispatch this parsed subcommand to its handler. Each non-serve storage
    /// arm is a CLI composition root: it opens the factory, mints only the
    /// selected command's dependencies, and injects them into the handler.
    ///
    /// # Errors
    ///
    /// Propagates the selected command's failure.
    pub async fn execute(
        self,
        telemetry: &host::telemetry::TelemetryConfig,
        capture: Option<ServeCapturePaths>,
    ) -> anyhow::Result<CommandOutput> {
        match self {
            Commands::Init {
                storage,
                skip_if_exists,
            } => storage_bootstrap::cmd_init(&storage, skip_if_exists)
                .await
                .map(|()| CommandOutput::None),
            Commands::CreatePgDb { pg } => storage_bootstrap::cmd_create_pg_db(
                &pg.bootstrap_db,
                &pg.app_db,
                &pg.app_role_password,
            )
            .await
            .map(|()| CommandOutput::None),
            Commands::Serve {
                storage,
                bind,
                environment,
            } => lifecycle::cmd_serve(
                &storage,
                bind,
                environment.is_prod(),
                telemetry,
                capture.as_ref(),
            )
            .await
            .map(|()| CommandOutput::None),
            Commands::ShutDown { storage, timeout } => {
                shut_down::cmd_shut_down(&storage, std::time::Duration::from_secs(timeout.get()))
                    .map(|()| CommandOutput::None)
            }
            Commands::UserCreate {
                storage,
                username,
                password,
                display_name,
                operator,
            } => execute_user_create(storage, username, password, display_name, operator)
                .await
                .map(|()| CommandOutput::None),
            Commands::AppPasswordCreate {
                storage,
                username,
                label,
            } => execute_app_password_create(storage, username, label)
                .await
                .map(|()| CommandOutput::None),
            Commands::UserInvite {
                storage,
                expires_in,
            } => execute_user_invite(storage, expires_in)
                .await
                .map(|()| CommandOutput::None),
            Commands::SmtpTest { storage, to } => execute_smtp_test(storage, to)
                .await
                .map(|()| CommandOutput::None),
            Commands::Backup {
                storage,
                mode,
                path,
            } => backup::cmd_backup(&storage, mode.into(), path)
                .await
                .map(CommandOutput::Backup),
            Commands::Restore { storage, path } => backup::cmd_restore(&storage, &path)
                .await
                .map(CommandOutput::Restore),
            // First nested subcommand group: the arm stays a thin delegation to
            // SiteConfigAction::execute (a sibling match), preserving the low-CRAP
            // one-arm-per-command dispatch shape. Copy this pattern for future groups.
            Commands::SiteConfig { action } => action.execute().await.map(|()| CommandOutput::None),
            Commands::Websub { action } => action.execute().await.map(|()| CommandOutput::None),
        }
    }
}

impl SiteConfigAction {
    /// Dispatch a `site-config` leaf to its handler (mirrors [`Commands::execute`]).
    ///
    /// # Errors
    ///
    /// Propagates the selected leaf's failure.
    pub async fn execute(self) -> anyhow::Result<()> {
        match self {
            SiteConfigAction::Set {
                storage,
                key,
                value,
            } => execute_site_config_set(storage, key, value).await,
            SiteConfigAction::Get { storage, key } => {
                let factory = open_existing_storage(&storage).await?;
                site_config::cmd_site_config_get(factory.site_config().as_ref(), key).await
            }
            SiteConfigAction::List { storage } => {
                let factory = open_existing_storage(&storage).await?;
                site_config::cmd_site_config_list(factory.site_config().as_ref()).await
            }
            SiteConfigAction::Unset { storage, key } => {
                execute_site_config_unset(storage, key).await
            }
        }
    }
}

impl WebsubAction {
    /// Dispatch a `websub` leaf group.
    ///
    /// # Errors
    ///
    /// Propagates the selected leaf's failure.
    pub async fn execute(self) -> anyhow::Result<()> {
        match self {
            WebsubAction::DeadLetters { action } => action.execute().await,
        }
    }
}

impl DeadLetterAction {
    /// Dispatch a `websub dead-letters` leaf.
    ///
    /// # Errors
    ///
    /// Propagates the selected leaf's failure.
    pub async fn execute(self) -> anyhow::Result<()> {
        match self {
            DeadLetterAction::List {
                storage,
                phase,
                cursor,
                page_size,
            } => {
                let factory = open_existing_storage(&storage).await?;
                websub::cmd_dead_letters_list(
                    factory.feed_events().as_ref(),
                    phase,
                    cursor.map(DeadLetterCursor::into_inner),
                    page_size,
                )
                .await
            }
            DeadLetterAction::Redrive { storage, ids } => {
                let factory = open_existing_storage(&storage).await?;
                websub::cmd_dead_letters_redrive(
                    factory.feed_events(),
                    &factory.write_scope(),
                    &ids,
                )
                .await
            }
        }
    }
}
