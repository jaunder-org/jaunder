use std::error::Error;
use std::sync::Arc;

use async_trait::async_trait;
use leptos::prelude::use_context;

use crate::{
    auth,
    error::{InternalError, InternalResult},
};

/// Why a manual backup produced no backup.
#[derive(Debug)]
pub enum ManualBackupError {
    /// Another backup holds the run-at-a-time guard.
    AlreadyRunning,
    /// No backup destination is saved.
    NoDestination,
    /// The export or retention pruning failed.
    Failed(Box<dyn Error + Send + Sync>),
}

/// The server's backup runner as the manual trigger sees it, supplied by the
/// composition root (ADR-0016).
#[async_trait]
pub trait ManualBackupRunner: Send + Sync {
    /// Writes one backup to the saved destination now, returning its path.
    async fn run_manual_backup(&self) -> Result<String, ManualBackupError>;
}

fn map_manual_backup_error(error: ManualBackupError) -> InternalError {
    match error {
        ManualBackupError::AlreadyRunning => InternalError::conflict("a backup is already running"),
        ManualBackupError::NoDestination => {
            InternalError::validation("Save a backup destination before backing up.")
        }
        ManualBackupError::Failed(source) => {
            InternalError::server_boxed(source).with_context("backup.trigger", "manual")
        }
    }
}

pub(super) async fn back_up_now_impl() -> InternalResult<String> {
    auth::require_operator().await?;
    let runner = use_context::<Arc<dyn ManualBackupRunner>>()
        .ok_or_else(|| InternalError::server_message("backup runner is unavailable"))?;
    runner
        .run_manual_backup()
        .await
        .map_err(map_manual_backup_error)
}

#[cfg(test)]
mod tests {
    use super::{ManualBackupError, map_manual_backup_error};
    use crate::error::ErrorKind;

    #[test]
    fn manual_backup_errors_map_to_their_public_kinds() {
        assert_eq!(
            map_manual_backup_error(ManualBackupError::AlreadyRunning).kind(),
            ErrorKind::Conflict
        );
        assert_eq!(
            map_manual_backup_error(ManualBackupError::NoDestination).kind(),
            ErrorKind::Validation
        );
    }

    #[test]
    fn manual_backup_failure_report_names_the_manual_trigger_and_keeps_its_source() {
        let error = map_manual_backup_error(ManualBackupError::Failed(
            "server.backup.manual_run: destination unwritable".into(),
        ));

        assert_eq!(error.kind(), ErrorKind::Internal);
        assert_eq!(error.public_message(), "server operation failed");
        assert_eq!(error.context(), [("backup.trigger", "manual".to_owned())]);
        assert!(
            error
                .operator_message()
                .contains("server.backup.manual_run: destination unwritable"),
            "operator message: {}",
            error.operator_message()
        );
    }
}
