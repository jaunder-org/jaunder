use std::{
    fs, io,
    path::{Path, PathBuf},
};

use common::backup::BackupMode;
use storage::{
    BackupExportOptions, BackupRestoreOptions, BackupRestoreOutcome, RestoreValidationReport,
    StorageRuntimeConfig,
};

use super::support::storage_runtime_config;
use crate::cli::StorageArgs;

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
    let runtime = storage_runtime_config(&storage.db)?;
    let destination_path = path.unwrap_or_else(|| default_backup_path(storage, mode));
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
    if !path.exists() {
        return Err(anyhow::anyhow!(
            "backup path does not exist: {}",
            path.display()
        ));
    }
    let runtime = storage_runtime_config(&storage.db)?;
    ensure_restore_target_empty(storage, &runtime).await?;
    let outcome = storage::restore_backup(BackupRestoreOptions {
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

fn default_backup_path(storage: &StorageArgs, mode: BackupMode) -> PathBuf {
    let timestamp = chrono::Utc::now().format("%Y%m%dT%H%M%SZ");
    let name = match mode {
        BackupMode::Directory => format!("backup-{timestamp}"),
        BackupMode::Archive => format!("backup-{timestamp}.tar.gz"),
    };
    storage.storage_path.join("backups").join(name)
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
    use super::*;

    #[test]
    fn default_backup_path_is_under_storage_backups() {
        let storage = StorageArgs {
            storage_path: PathBuf::from("/tmp/jaunder"),
            db: "sqlite:/tmp/jaunder.db".parse().expect("sqlite db"),
        };

        let path = default_backup_path(&storage, BackupMode::Directory);

        assert!(path.starts_with("/tmp/jaunder/backups"));
    }

    #[test]
    fn default_archive_backup_path_ends_with_tar_gz() {
        let storage = StorageArgs {
            storage_path: PathBuf::from("/tmp/jaunder"),
            db: "sqlite:/tmp/jaunder.db".parse().expect("sqlite db"),
        };

        let path = default_backup_path(&storage, BackupMode::Archive);

        assert!(path.starts_with("/tmp/jaunder/backups"));
        assert!(path.to_string_lossy().ends_with(".tar.gz"));
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
