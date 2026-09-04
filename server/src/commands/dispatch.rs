use std::path::PathBuf;

use storage::BackupRestoreOutcome;

use crate::cli::{Commands, DeadLetterAction, DeadLetterCursor, SiteConfigAction, WebsubAction};

use super::{
    account, backup,
    lifecycle::{self, ServeCapturePaths},
    site_config, storage_bootstrap, websub,
};

pub enum CommandOutput {
    None,
    Backup(PathBuf),
    Restore(BackupRestoreOutcome),
}

impl Commands {
    /// Dispatch this parsed subcommand to its handler. A flat match-expression:
    /// each arm evaluates to the command's `Result<CommandOutput>`, so there is no `?` on
    /// the dispatch call and no trailing `Ok(())` — keeping any single function's
    /// cyclomatic complexity (and thus CRAP) low as subcommands are added (#147).
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
            Commands::UserCreate {
                storage,
                username,
                password,
                display_name,
                operator,
            } => account::cmd_user_create(
                &storage,
                &username,
                password,
                display_name.as_ref(),
                operator,
            )
            .await
            .map(|()| CommandOutput::None),
            Commands::AppPasswordCreate {
                storage,
                username,
                label,
            } => account::cmd_app_password_create(&storage, &username, &label)
                .await
                .map(|()| CommandOutput::None),
            Commands::UserInvite {
                storage,
                expires_in,
            } => account::cmd_user_invite(&storage, expires_in)
                .await
                .map(|()| CommandOutput::None),
            Commands::SmtpTest { storage, to } => account::cmd_smtp_test(&storage, &to)
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
            } => site_config::cmd_site_config_set(&storage, key, &value).await,
            SiteConfigAction::Get { storage, key } => {
                site_config::cmd_site_config_get(&storage, key).await
            }
            SiteConfigAction::List { storage } => site_config::cmd_site_config_list(&storage).await,
            SiteConfigAction::Unset { storage, key } => {
                site_config::cmd_site_config_unset(&storage, key).await
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
                websub::cmd_dead_letters_list(
                    &storage,
                    phase,
                    cursor.map(DeadLetterCursor::into_inner),
                    page_size,
                )
                .await
            }
            DeadLetterAction::Redrive { storage, ids } => {
                websub::cmd_dead_letters_redrive(&storage, &ids).await
            }
        }
    }
}
