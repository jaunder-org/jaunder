//! Recursive backup media mirroring, restoration, and content deduplication.

use std::{
    fs,
    fs::File,
    path::{Path, PathBuf},
};

use sha2::{Digest, Sha256};

use super::error::BackupError;

pub(super) fn restore_media_directory(
    source: &Path,
    destination: &Path,
) -> Result<(), BackupError> {
    fs::create_dir_all(destination)?;
    if !source.exists() {
        return Ok(());
    }
    restore_media_entries(source, destination, Path::new(""))
}

fn restore_media_entries(
    source_root: &Path,
    destination_root: &Path,
    relative_path: &Path,
) -> Result<(), BackupError> {
    let source_dir = source_root.join(relative_path);
    for entry in fs::read_dir(source_dir)? {
        let entry = entry?;
        let file_name = entry.file_name();
        let child_relative_path = relative_path.join(file_name);
        let source_path = entry.path();
        let destination_path = destination_root.join(&child_relative_path);
        let metadata = entry.metadata()?;

        if metadata.is_dir() {
            fs::create_dir_all(&destination_path)?;
            restore_media_entries(source_root, destination_root, &child_relative_path)?;
        } else if metadata.is_file() {
            let Some(parent) = destination_path.parent() else {
                unreachable!("a joined destination path always has a parent")
            };
            fs::create_dir_all(parent)?;
            fs::copy(source_path, destination_path)?;
        }
        // Entries that are neither a directory nor a regular file (sockets,
        // FIFOs, devices, broken symlinks whose target vanished) are silently
        // skipped — media backups only carry regular files.
    }
    Ok(())
}

/// # Errors
///
/// Returns `Err(BackupError)` if copying or removing media files fails.
pub fn mirror_media_directory(
    source: &Path,
    destination: &Path,
    previous_backup: Option<&Path>,
) -> Result<(), BackupError> {
    fs::create_dir_all(destination)?;
    if !source.exists() {
        return Ok(());
    }
    mirror_media_entries(source, destination, previous_backup, Path::new(""))
}

fn mirror_media_entries(
    source_root: &Path,
    destination_root: &Path,
    previous_backup: Option<&Path>,
    relative_path: &Path,
) -> Result<(), BackupError> {
    let source_dir = source_root.join(relative_path);
    for entry in fs::read_dir(source_dir)? {
        let entry = entry?;
        let file_name = entry.file_name();
        let child_relative_path = relative_path.join(file_name);
        let source_path = entry.path();
        let destination_path = destination_root.join(&child_relative_path);
        let metadata = entry.metadata()?;

        if metadata.is_dir() {
            fs::create_dir_all(&destination_path)?;
            mirror_media_entries(
                source_root,
                destination_root,
                previous_backup,
                &child_relative_path,
            )?;
        } else if metadata.is_file() {
            copy_or_link_media_file(
                &source_path,
                &destination_path,
                previous_backup,
                &child_relative_path,
            )?;
        } // cov:ignore is_file arm's closing brace; llvm-cov leaves it unmarked though the arm's copy-success and `?`-failure paths are both tested
    }
    Ok(())
}

fn copy_or_link_media_file(
    source_path: &Path,
    destination_path: &Path,
    previous_backup: Option<&Path>,
    relative_path: &Path,
) -> Result<(), BackupError> {
    let Some(parent) = destination_path.parent() else {
        unreachable!("a joined destination path always has a parent")
    };
    fs::create_dir_all(parent)?;

    // Deduplicate against the previous backup: when this file is byte-identical
    // to its counterpart there, hard-link to that copy instead of writing a new
    // one, so a chain of backups doesn't store N copies of an unchanged blob.
    // Fall through to a real copy if the content differs or the link can't be
    // made (e.g. the previous backup is on a different filesystem).
    if let Some(previous_file) = previous_backup
        .map(|backup| backup.join("media").join(relative_path))
        .filter(|path| path.is_file())
        && files_have_same_content(source_path, &previous_file)?
        && fs::hard_link(&previous_file, destination_path).is_ok()
    {
        return Ok(());
    }

    fs::copy(source_path, destination_path)?;
    Ok(())
}

fn files_have_same_content(left: &Path, right: &Path) -> Result<bool, BackupError> {
    let left_metadata = fs::metadata(left)?;
    let right_metadata = fs::metadata(right)?;
    if left_metadata.len() != right_metadata.len() {
        return Ok(false);
    }
    Ok(file_sha256(left)? == file_sha256(right)?)
}

fn file_sha256(path: &Path) -> Result<[u8; 32], BackupError> {
    let mut file = File::open(path)?;
    let mut hasher = Sha256::new();
    std::io::copy(&mut file, &mut hasher)?;
    Ok(hasher.finalize().into())
}

pub(super) fn previous_directory_backup(
    destination_path: &Path,
) -> Result<Option<PathBuf>, BackupError> {
    let Some(parent) = destination_path.parent() else {
        return Ok(None);
    };
    if !parent.exists() {
        return Ok(None);
    }

    // The previous backup is the newest sibling directory, used only as a
    // hard-link source for media dedup. Both marker files are required so a
    // half-written directory is never linked against; date-stamped names sort
    // lexicographically, so the last after sorting is the most recent.
    let mut candidates = Vec::new();
    for entry in fs::read_dir(parent)? {
        let entry = entry?;
        let path = entry.path();
        if path != destination_path
            && path.join("manifest.json").is_file()
            && path.join("media").is_dir()
        {
            candidates.push(path);
        }
    }
    candidates.sort();
    Ok(candidates.pop())
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[test]
    fn missing_media_directory_creates_empty_destination() -> Result<(), BackupError> {
        let temp = TempDir::new()?;
        let source = temp.path().join("missing");
        let destination = temp.path().join("destination");

        mirror_media_directory(&source, &destination, None)?;

        assert!(destination.is_dir());
        assert!(fs::read_dir(destination)?.next().is_none());
        Ok(())
    }

    #[test]
    fn media_mirror_hard_links_unchanged_previous_file() -> Result<(), BackupError> {
        let temp = TempDir::new()?;
        let source = temp.path().join("source");
        let previous = temp.path().join("previous");
        let destination = temp.path().join("destination");
        fs::create_dir_all(source.join("nested"))?;
        fs::create_dir_all(previous.join("media").join("nested"))?;
        fs::write(source.join("nested").join("image.txt"), "same")?;
        fs::write(
            previous.join("media").join("nested").join("image.txt"),
            "same",
        )
        .expect("write previous nested media file");

        mirror_media_directory(&source, &destination, Some(&previous))?;

        assert_eq!(
            fs::read_to_string(destination.join("nested").join("image.txt"))?,
            "same"
        );
        Ok(())
    }

    #[test]
    fn media_mirror_copies_when_previous_file_differs() -> Result<(), BackupError> {
        let temp = TempDir::new()?;
        let source = temp.path().join("source");
        let previous = temp.path().join("previous");
        let destination = temp.path().join("destination");
        fs::create_dir_all(&source)?;
        fs::create_dir_all(previous.join("media"))?;
        fs::write(source.join("image.txt"), "new")?;
        fs::write(previous.join("media").join("image.txt"), "old")?;

        mirror_media_directory(&source, &destination, Some(&previous))?;

        assert_eq!(fs::read_to_string(destination.join("image.txt"))?, "new");
        assert!(
            !files_have_same_content(
                &source.join("image.txt"),
                &previous.join("media").join("image.txt")
            )
            .expect("compare source and previous media files")
        );
        Ok(())
    }

    #[test]
    fn mirror_media_propagates_copy_failure() -> Result<(), BackupError> {
        // Structural (root-immune) fs failure: pre-create the destination file
        // path as a *directory* so `fs::copy` into it fails with EISDIR. The
        // error propagates out of `copy_or_link_media_file` and back up through
        // the recursive `mirror_media_entries` call — covering both `?` arms.
        let temp = TempDir::new()?;
        let source = temp.path().join("source");
        let destination = temp.path().join("destination");
        fs::create_dir_all(source.join("dir1"))?;
        fs::write(source.join("dir1").join("file.txt"), "x")?;
        // A directory sitting where the copied file must be written.
        fs::create_dir_all(destination.join("dir1").join("file.txt"))?;

        let error = mirror_media_directory(&source, &destination, None)
            .expect_err("copying onto a directory must fail");
        assert!(matches!(error, BackupError::Io(_)));
        Ok(())
    }

    #[test]
    fn restore_media_skips_non_regular_entries() -> Result<(), BackupError> {
        // A Unix-domain socket is neither a directory nor a regular file, so the
        // restore walk takes the fallthrough arm and silently skips it, while a
        // sibling regular file still copies.
        let temp = TempDir::new()?;
        let source = temp.path().join("source");
        let destination = temp.path().join("destination");
        fs::create_dir_all(&source)?;
        let _listener =
            std::os::unix::net::UnixListener::bind(source.join("sock")).expect("bind unix socket");
        fs::write(source.join("real.txt"), "keep")?;

        restore_media_directory(&source, &destination)?;

        assert_eq!(fs::read_to_string(destination.join("real.txt"))?, "keep");
        assert!(
            !destination.join("sock").exists(),
            "a non-regular entry must not be copied"
        );
        Ok(())
    }

    #[test]
    fn files_have_same_content_returns_false_for_different_size_files() -> Result<(), BackupError> {
        let temp = TempDir::new()?;
        let a = temp.path().join("a.txt");
        let b = temp.path().join("b.txt");
        fs::write(&a, "short")?;
        fs::write(&b, "longer content")?;
        assert!(!files_have_same_content(&a, &b)?);
        Ok(())
    }

    #[test]
    fn previous_directory_backup_returns_none_for_nonexistent_parent() -> Result<(), BackupError> {
        let temp = TempDir::new()?;
        let destination = temp.path().join("nonexistent_parent").join("backup");
        assert_eq!(previous_directory_backup(&destination)?, None);
        Ok(())
    }

    #[test]
    fn previous_directory_backup_returns_none_for_parentless_path() -> Result<(), BackupError> {
        // The filesystem root has no parent, so there is no sibling directory to
        // source a previous backup from.
        assert_eq!(previous_directory_backup(Path::new("/"))?, None);
        Ok(())
    }

    #[test]
    fn files_have_same_content_returns_true_for_identical_files() -> Result<(), BackupError> {
        let temp = TempDir::new()?;
        let a = temp.path().join("a.txt");
        let b = temp.path().join("b.txt");
        fs::write(&a, "identical")?;
        fs::write(&b, "identical")?;
        assert!(files_have_same_content(&a, &b)?);
        Ok(())
    }

    #[test]
    fn previous_directory_backup_selects_latest_manifest_directory() -> Result<(), BackupError> {
        let temp = TempDir::new()?;
        let first = temp.path().join("2026-04-28");
        let second = temp.path().join("2026-04-29");
        let current = temp.path().join("2026-04-30");
        for path in [&first, &second] {
            fs::create_dir_all(path.join("media"))?;
            fs::write(path.join("manifest.json"), "{}")?;
        }

        assert_eq!(previous_directory_backup(&current)?, Some(second));
        Ok(())
    }

    #[test]
    fn previous_directory_backup_excludes_dirs_without_both_marker_files() -> Result<(), BackupError>
    {
        let temp = TempDir::new()?;
        let current = temp.path().join("2026-04-30");
        // Has manifest.json but no media/ — must not be treated as a valid previous backup.
        let manifest_only = temp.path().join("2026-04-29");
        fs::create_dir_all(&manifest_only)?;
        fs::write(manifest_only.join("manifest.json"), "{}")?;
        // Has media/ but no manifest.json — also invalid.
        let media_only = temp.path().join("2026-04-28");
        fs::create_dir_all(media_only.join("media"))?;

        assert_eq!(previous_directory_backup(&current)?, None);
        Ok(())
    }

    #[test]
    fn restore_media_directory_accepts_missing_source() -> Result<(), BackupError> {
        let temp = TempDir::new()?;
        let destination = temp.path().join("destination");

        restore_media_directory(&temp.path().join("missing"), &destination)?;

        assert!(destination.is_dir());
        assert!(fs::read_dir(destination)?.next().is_none());
        Ok(())
    }

    #[test]
    fn restore_media_directory_copies_nested_files() -> Result<(), BackupError> {
        let temp = TempDir::new()?;
        let source = temp.path().join("source");
        let destination = temp.path().join("destination");
        fs::create_dir_all(source.join("nested"))?;
        fs::write(source.join("nested").join("avatar.txt"), "image")?;

        restore_media_directory(&source, &destination)?;

        assert_eq!(
            fs::read_to_string(destination.join("nested").join("avatar.txt"))?,
            "image"
        );
        Ok(())
    }

    // Both backends' restore path shares the ragged-NDJSON contract: a row that
    // omits a column present in row 0 is rejected as `InvalidBackup`, and the
    // failed import rolls the restore transaction back. One `#[apply(backends)]`
    // test covers the SQLite and PostgreSQL `import_table` missing-column arms
    // plus the PostgreSQL `restore_database` rollback arm.
}
