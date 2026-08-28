//! Backup destination, temporary-directory, and tar archive mechanics.

use std::{
    fs,
    fs::File,
    path::{Path, PathBuf},
};

use chrono::Utc;
use flate2::{Compression, read::GzDecoder, write::GzEncoder};

use super::error::BackupError;

pub(super) fn ensure_empty_or_absent(path: &Path) -> Result<(), BackupError> {
    if !path.exists() {
        return Ok(());
    }
    if fs::read_dir(path)?.next().is_some() {
        return Err(BackupError::DestinationNotEmpty(path.to_path_buf()));
    }
    Ok(())
}

pub(super) fn ensure_absent(path: &Path) -> Result<(), BackupError> {
    if path.exists() {
        return Err(BackupError::DestinationExists(path.to_path_buf()));
    }
    Ok(())
}

pub(super) struct TemporaryBackupDirectory {
    path: PathBuf,
}

impl TemporaryBackupDirectory {
    pub(super) fn near(destination_path: &Path) -> Result<Self, BackupError> {
        let parent = destination_path.parent().unwrap_or_else(|| Path::new("."));
        fs::create_dir_all(parent)?;
        let file_name = destination_path
            .file_name()
            .and_then(|name| name.to_str())
            .unwrap_or("backup");
        let suffix = Utc::now()
            .timestamp_nanos_opt()
            .unwrap_or_else(|| Utc::now().timestamp_micros());
        let path = parent.join(format!(".{file_name}.{suffix}.tmp"));
        fs::create_dir(&path)?;
        Ok(Self { path })
    }

    pub(super) fn in_temp() -> Result<Self, BackupError> {
        let suffix = Utc::now()
            .timestamp_nanos_opt()
            .unwrap_or_else(|| Utc::now().timestamp_micros());
        let path = std::env::temp_dir().join(format!("jaunder-backup-{suffix}"));
        fs::create_dir(&path)?;
        Ok(Self { path })
    }

    pub(super) fn path(&self) -> &Path {
        &self.path
    }
}

fn finish_temporary_directory_cleanup<T>(primary: T, cleanup: std::io::Result<()>) -> T {
    crate::helpers::preserve_after_secondary(
        primary,
        cleanup,
        host::error::ErrorKind::Internal,
        host::error::ErrorClass::Transient,
        "storage.backup.temporary_directory_cleanup",
    )
}

impl Drop for TemporaryBackupDirectory {
    fn drop(&mut self) {
        finish_temporary_directory_cleanup((), fs::remove_dir_all(&self.path));
    }
}

pub(super) fn write_tar_gz(source_root: &Path, destination_path: &Path) -> Result<(), BackupError> {
    let file = File::create(destination_path)?;
    let encoder = GzEncoder::new(file, Compression::default());
    let mut archive = tar::Builder::new(encoder);
    archive.append_dir_all(".", source_root)?;
    let encoder = archive.into_inner()?;
    encoder.finish()?;
    Ok(())
}

pub(super) fn extract_archive_backup(
    source_path: &Path,
) -> Result<TemporaryBackupDirectory, BackupError> {
    let destination = TemporaryBackupDirectory::in_temp()?;
    let file = File::open(source_path)?;
    let decoder = GzDecoder::new(file);
    let mut archive = tar::Archive::new(decoder);
    archive.unpack(destination.path())?;
    Ok(destination)
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[test]
    fn empty_or_absent_destination_accepts_missing_and_empty_paths() -> Result<(), BackupError> {
        let temp = TempDir::new()?;
        let missing = temp.path().join("missing");
        ensure_empty_or_absent(&missing)?;

        let empty = temp.path().join("empty");
        fs::create_dir(&empty)?;
        ensure_empty_or_absent(&empty)?;

        fs::write(empty.join("file"), "content")?;
        let error = ensure_empty_or_absent(&empty);
        assert!(matches!(error, Err(BackupError::DestinationNotEmpty(_))));
        Ok(())
    }

    #[test]
    fn ensure_absent_rejects_existing_path() -> Result<(), BackupError> {
        let temp = TempDir::new()?;
        let existing = temp.path().join("exists");
        fs::create_dir(&existing)?;
        let error = ensure_absent(&existing);
        assert!(matches!(error, Err(BackupError::DestinationExists(_))));
        Ok(())
    }

    #[test]
    fn temporary_backup_directory_drop_removes_directory() -> Result<(), BackupError> {
        let temp = TempDir::new()?;
        let destination = temp.path().join("backup");
        let path = {
            let directory = TemporaryBackupDirectory::near(&destination)?;
            let temporary_path = directory.path().to_path_buf();
            assert!(
                temporary_path.exists(),
                "directory should exist before drop"
            );
            temporary_path
        };
        assert!(!path.exists(), "directory should be removed after drop");
        Ok(())
    }

    #[test]
    fn continuation_reporting_temporary_backup_cleanup_failure_preserves_primary_results_and_reports_once()
     {
        let cleanup = || {
            Err(std::io::Error::new(
                std::io::ErrorKind::PermissionDenied,
                "cleanup denied",
            ))
        };
        let (success, trace) = crate::helpers::swallowed_test::capture(|| {
            finish_temporary_directory_cleanup(Ok::<_, BackupError>(41_u8), cleanup())
        });
        assert_eq!(success.expect("primary success"), 41);
        crate::helpers::swallowed_test::assert_one_report(
            &trace,
            "storage.backup.temporary_directory_cleanup",
        );

        let (failure, trace) = crate::helpers::swallowed_test::capture(|| {
            finish_temporary_directory_cleanup(
                Err::<u8, _>(BackupError::InvalidBackup("primary sentinel".to_owned())),
                cleanup(),
            )
        });
        assert!(
            matches!(failure, Err(BackupError::InvalidBackup(message)) if message == "primary sentinel")
        );
        crate::helpers::swallowed_test::assert_one_report(
            &trace,
            "storage.backup.temporary_directory_cleanup",
        );
    }
}
