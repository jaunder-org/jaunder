use std::{
    fs, io,
    path::{Path, PathBuf},
};

use common::backup::BackupMode;
use storage::{
    BackupError, BackupExportOptions, BackupRestoreOptions, BackupRestoreOutcome,
    DatabaseLockGuard, RestoreValidationReport, StorageRuntimeConfig,
};

use super::support;
use crate::cli::StorageArgs;
use crate::runtime_file::StartupLockGuard;

#[async_trait::async_trait]
trait RestoreOperation: Sync {
    async fn restore(
        &self,
        options: BackupRestoreOptions<'_>,
    ) -> Result<BackupRestoreOutcome, BackupError>;
}

struct OrdinaryRestore;

#[async_trait::async_trait]
impl RestoreOperation for OrdinaryRestore {
    async fn restore(
        &self,
        options: BackupRestoreOptions<'_>,
    ) -> Result<BackupRestoreOutcome, BackupError> {
        storage::restore_backup(options).await
    }
}

/// Performs a full backup of the application database and media.
///
/// # Errors
///
/// Returns an error if the backup process fails.
pub async fn cmd_backup(
    storage: &StorageArgs,
    mode: BackupMode,
    path: Option<PathBuf>,
) -> anyhow::Result<PathBuf> {
    let runtime = support::storage_runtime_config(&storage.db)?;
    let destination_path = path.unwrap_or_else(|| {
        crate::backup::backup_path_for_mode(&storage.storage_path.join("backups"), mode)
    });
    let manifest = storage::export_backup(BackupExportOptions {
        database: &storage.db,
        runtime: &runtime,
        media_path: &storage.storage_path.join("media"),
        destination_path: &destination_path,
        mode,
    })
    .await?;

    println!(
        "Backup complete: path={} tables={}",
        destination_path.display(),
        manifest.tables.len()
    );
    Ok(destination_path)
}

/// Restores the application state from a backup.
///
/// # Errors
///
/// Returns an error if the backup does not exist, or if the target database or
/// media directory is not empty.
pub async fn cmd_restore(
    storage: &StorageArgs,
    path: &Path,
) -> anyhow::Result<BackupRestoreOutcome> {
    cmd_restore_with(storage, path, &OrdinaryRestore).await
}

async fn cmd_restore_with(
    storage: &StorageArgs,
    path: &Path,
    operation: &impl RestoreOperation,
) -> anyhow::Result<BackupRestoreOutcome> {
    if !path.exists() {
        return Err(anyhow::anyhow!(
            "backup path does not exist: {}",
            path.display()
        ));
    }
    let runtime = support::storage_runtime_config(&storage.db)?;
    // Restore may not mutate a live server's data. Keep both guards through
    // validation and Media placement, not just the database import.
    let _runtime_lock = StartupLockGuard::acquire(&storage.storage_path)?;
    let _database_lock = DatabaseLockGuard::acquire(&storage.storage_path).await?;
    ensure_restore_target_empty(storage, &runtime).await?;
    let outcome = operation
        .restore(BackupRestoreOptions {
            database: &storage.db,
            runtime: &runtime,
            media_path: &storage.storage_path.join("media"),
            source_path: path,
        })
        .await?;
    println!(
        "Restore complete: path={} tables={}",
        path.display(),
        outcome.manifest.tables.len()
    );
    print_restore_validation_report(&outcome.validation_report);
    Ok(outcome)
}

fn print_restore_validation_report(report: &RestoreValidationReport) {
    if report.is_empty() {
        return;
    }

    println!(
        "Restore validation issues: count={} (data restored; repair may be needed before normal reads)",
        report.len()
    );
    for issue in report.issues() {
        println!("- {issue}");
    }
}

async fn ensure_restore_target_empty(
    storage: &StorageArgs,
    runtime: &StorageRuntimeConfig,
) -> anyhow::Result<()> {
    if !storage::database_is_empty(&storage.db, runtime).await? {
        return Err(anyhow::anyhow!(
            "refusing to restore into a non-empty database"
        ));
    }
    let media_path = storage.storage_path.join("media");
    if directory_has_entries(&media_path)? {
        return Err(anyhow::anyhow!(
            "refusing to restore into a non-empty media directory"
        ));
    }
    Ok(())
}

fn directory_has_entries(path: &Path) -> io::Result<bool> {
    if !path.exists() {
        return Ok(false);
    }
    for entry in fs::read_dir(path)? {
        let entry = entry?;
        let metadata = entry.metadata()?;
        if metadata.is_dir() {
            if directory_has_entries(&entry.path())? {
                return Ok(true);
            }
        } else {
            return Ok(true);
        }
    }
    Ok(false)
}

#[cfg(test)]
mod tests {
    use std::sync::{Mutex, mpsc};
    use std::time::Duration;

    use tokio::sync::oneshot;

    use super::*;

    struct PausedAfterImport {
        entered: Mutex<Option<oneshot::Sender<()>>>,
        resume: Mutex<mpsc::Receiver<()>>,
    }

    #[async_trait::async_trait]
    impl RestoreOperation for PausedAfterImport {
        async fn restore(
            &self,
            options: BackupRestoreOptions<'_>,
        ) -> Result<BackupRestoreOutcome, BackupError> {
            storage::restore_backup_paused_after_import(options, || {
                self.entered
                    .lock()
                    .unwrap()
                    .take()
                    .unwrap()
                    .send(())
                    .unwrap();
                self.resume.lock().unwrap().recv().unwrap();
            })
            .await
        }
    }

    // guard:low-level-db — the injected post-import pause must run inside the CLI restore operation.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn database_import_does_not_let_an_opener_serve_before_media_restoration() {
        let temp = tempfile::TempDir::new().unwrap();
        let storage_args = |name: &str| {
            let storage_path = temp.path().join(name);
            StorageArgs {
                db: format!("sqlite:{}", storage_path.join("jaunder.db").display())
                    .parse()
                    .unwrap(),
                storage_path,
            }
        };
        let source = storage_args("source");
        super::super::storage_bootstrap::cmd_init(&source, false)
            .await
            .unwrap();
        std::fs::write(source.storage_path.join("media/proof.bin"), b"media").unwrap();
        let backup_path = temp.path().join("backup");
        cmd_backup(&source, BackupMode::Directory, Some(backup_path.clone()))
            .await
            .unwrap();
        let target = storage_args("target");
        super::super::storage_bootstrap::cmd_init(&target, false)
            .await
            .unwrap();

        let (entered_tx, entered_rx) = oneshot::channel();
        let (resume_tx, resume_rx) = mpsc::channel();
        let pause = PausedAfterImport {
            entered: Mutex::new(Some(entered_tx)),
            resume: Mutex::new(resume_rx),
        };
        let target_path = target.storage_path.clone();
        let rival_db = target.db.clone();
        let restore = tokio::spawn(async move {
            cmd_restore_with(&target, &backup_path, &pause)
                .await
                .unwrap()
        });
        tokio::time::timeout(Duration::from_secs(30), entered_rx)
            .await
            .expect("restore reaches database import")
            .unwrap();
        assert!(!target_path.join("media/proof.bin").exists());
        let (trying_tx, trying_rx) = oneshot::channel();
        let rival_path = target_path.clone();
        let mut rival = tokio::spawn(async move {
            trying_tx.send(()).unwrap();
            let _lock = DatabaseLockGuard::acquire(&rival_path).await.unwrap();
            storage::open_existing_database(&rival_db, &StorageRuntimeConfig::default())
                .await
                .unwrap();
            assert_eq!(
                std::fs::read(rival_path.join("media/proof.bin")).unwrap(),
                b"media"
            );
        });
        trying_rx.await.unwrap();
        assert!(
            tokio::time::timeout(Duration::from_millis(100), &mut rival)
                .await
                .is_err()
        );
        resume_tx.send(()).unwrap();
        restore.await.unwrap();
        tokio::time::timeout(Duration::from_secs(30), rival)
            .await
            .expect("opener resumes after Media placement")
            .unwrap();
    }

    // guard:no-backend — runtime-lock refusal precedes every database read.
    #[tokio::test]
    async fn restore_refuses_a_live_same_directory_server_before_target_preflight() {
        let temp = tempfile::TempDir::new().expect("temp dir");
        let storage_path = temp.path().join("storage");
        let _live_server = StartupLockGuard::acquire(&storage_path).expect("live server lock");
        let source_path = temp.path().join("backup");
        std::fs::create_dir(&source_path).expect("backup directory");
        let storage = StorageArgs {
            db: format!("sqlite:{}", storage_path.join("jaunder.db").display())
                .parse()
                .expect("database URL"),
            storage_path,
        };
        let error = cmd_restore(&storage, &source_path)
            .await
            .expect_err("restore refuses live server");
        assert!(
            error
                .to_string()
                .contains("cannot acquire exclusive startup lock")
        );
    }

    #[test]
    fn directory_has_entries_handles_missing_empty_and_nested_paths() {
        let temp = tempfile::TempDir::new().expect("temp dir");
        assert!(!directory_has_entries(&temp.path().join("missing")).expect("missing"));

        let empty = temp.path().join("empty");
        std::fs::create_dir(&empty).expect("empty dir");
        assert!(!directory_has_entries(&empty).expect("empty"));

        let nested = temp.path().join("nested");
        std::fs::create_dir(&nested).expect("nested dir");
        std::fs::write(nested.join("file.txt"), "content").expect("nested file");
        assert!(directory_has_entries(temp.path()).expect("nested"));
    }
}
