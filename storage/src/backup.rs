//! Database + media backup: exports each table to per-table NDJSON and mirrors
//! the media tree, as either a directory or a gzipped tar archive; restore
//! reverses it. Media is content-hash deduplicated against the previous backup
//! via hard links, so a series of backups doesn't re-store unchanged blobs.

use std::{
    fs::{self, File},
    io::{BufRead, BufReader, BufWriter, Write},
    path::{Path, PathBuf},
};

use chrono::{DateTime, Utc};
use flate2::{read::GzDecoder, write::GzEncoder, Compression};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use thiserror::Error;

use crate::{resolved_postgres_options, DbConnectOptions};
pub use common::backup::BackupMode;

// Ordered so a restore can insert tables in array order without tripping a
// foreign key: a referenced table (`users`, `posts`, `tags`) always precedes the
// tables that point at it (`sessions`, `post_revisions`, `post_tags`).
pub(crate) const TABLES_IN_EXPORT_ORDER: &[&str] = &[
    "site_config",
    "users",
    "sessions",
    "invites",
    "email_verifications",
    "password_resets",
    "posts",
    "post_revisions",
    "tags",
    "post_tags",
];

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BackupManifest {
    pub version: String,
    pub schema_version: i64,
    pub schema_checksum: String,
    pub timestamp: DateTime<Utc>,
    pub mode: BackupMode,
    pub tables: Vec<String>,
}

#[derive(Debug, Clone, Copy)]
pub struct BackupExportOptions<'a> {
    pub database: &'a DbConnectOptions,
    pub media_path: &'a Path,
    pub destination_path: &'a Path,
    pub mode: BackupMode,
}

#[derive(Debug, Clone, Copy)]
pub struct BackupRestoreOptions<'a> {
    pub database: &'a DbConnectOptions,
    pub media_path: &'a Path,
    pub source_path: &'a Path,
}

#[derive(Debug, Error)]
pub enum BackupError {
    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),
    #[error("database error: {0}")]
    Sqlx(#[from] sqlx::Error),
    #[error("migration error: {0}")]
    Migrate(#[from] sqlx::migrate::MigrateError),
    #[error("JSON error: {0}")]
    Json(#[from] serde_json::Error),
    #[error("backup destination is not empty: {0}")]
    DestinationNotEmpty(PathBuf),
    #[error("backup destination already exists: {0}")]
    DestinationExists(PathBuf),
    #[error("invalid backup: {0}")]
    InvalidBackup(String),
    #[error(
        "backup was created by jaunder {backup_version}, but this binary is {current_version}"
    )]
    VersionMismatch {
        backup_version: String,
        current_version: &'static str,
    },
    #[error(
        "backup schema version {backup_version} does not match target schema version {target_version}"
    )]
    SchemaVersionMismatch {
        backup_version: i64,
        target_version: i64,
    },
    #[error("restored database failed constraint validation: {0}")]
    ConstraintViolation(String),
}

#[derive(Debug, Clone)]
pub(crate) struct ColumnInfo {
    pub(crate) name: String,
    pub(crate) type_name: String,
}

/// # Errors
///
/// Returns `Err(BackupError)` if the backup export fails.
pub async fn export_backup(
    options: BackupExportOptions<'_>,
) -> Result<BackupManifest, BackupError> {
    match options.mode {
        BackupMode::Directory => export_directory_backup(options).await,
        BackupMode::Archive => export_archive_backup(options).await,
    }
}

/// # Errors
///
/// Returns `Err(BackupError)` if the backup restore fails.
pub async fn restore_backup(
    options: BackupRestoreOptions<'_>,
) -> Result<BackupManifest, BackupError> {
    let extracted_archive = if options.source_path.is_file() {
        Some(extract_archive_backup(options.source_path)?)
    } else {
        None
    };
    let source_path = extracted_archive
        .as_ref()
        .map_or(options.source_path, TemporaryBackupDirectory::path);

    let manifest = read_manifest(source_path)?;
    validate_manifest(&manifest)?;

    match manifest.mode {
        BackupMode::Directory | BackupMode::Archive => {
            restore_directory_backup(
                BackupRestoreOptions {
                    database: options.database,
                    media_path: options.media_path,
                    source_path,
                },
                &manifest,
            )
            .await?;
        }
    }

    restore_media_directory(&source_path.join("media"), options.media_path)?;
    Ok(manifest)
}

async fn export_archive_backup(
    options: BackupExportOptions<'_>,
) -> Result<BackupManifest, BackupError> {
    ensure_absent(options.destination_path)?;
    let staging = TemporaryBackupDirectory::near(options.destination_path)?;
    let manifest = export_directory_backup(BackupExportOptions {
        database: options.database,
        media_path: options.media_path,
        destination_path: staging.path(),
        mode: BackupMode::Archive,
    })
    .await?;
    write_tar_gz(staging.path(), options.destination_path)?;
    Ok(manifest)
}

async fn export_directory_backup(
    options: BackupExportOptions<'_>,
) -> Result<BackupManifest, BackupError> {
    ensure_empty_or_absent(options.destination_path)?;
    fs::create_dir_all(options.destination_path.join("db"))?;

    let manifest = match options.database {
        DbConnectOptions::Sqlite(connect_options) => {
            let pool = sqlx::SqlitePool::connect_with(connect_options.clone()).await?;
            crate::sqlite::backup::export_database(&pool, options.destination_path, options.mode)
                .await?
        }
        DbConnectOptions::Postgres {
            options: pg_options,
            ..
        } => {
            let resolved = resolved_postgres_options(pg_options)?;
            let pool = sqlx::PgPool::connect_with(resolved).await?;
            crate::postgres::backup::export_database(&pool, options.destination_path, options.mode)
                .await?
        }
    };

    let previous_backup = previous_directory_backup(options.destination_path)?;
    mirror_media_directory(
        options.media_path,
        &options.destination_path.join("media"),
        previous_backup.as_deref(),
    )?; // cov:ignore

    write_manifest(options.destination_path, &manifest)?;
    Ok(manifest)
}

async fn restore_directory_backup(
    options: BackupRestoreOptions<'_>,
    manifest: &BackupManifest,
) -> Result<(), BackupError> {
    if !options.source_path.join("db").is_dir() {
        return Err(BackupError::InvalidBackup(format!(
            "missing db directory: {}",
            options.source_path.join("db").display()
        )));
    }

    match options.database {
        DbConnectOptions::Sqlite(connect_options) => {
            let pool = sqlx::SqlitePool::connect_with(connect_options.clone()).await?;
            crate::sqlite::backup::restore_database(&pool, options.source_path, manifest).await
        }
        DbConnectOptions::Postgres {
            options: pg_options,
            ..
        } => {
            let resolved = resolved_postgres_options(pg_options)?;
            let pool = sqlx::PgPool::connect_with(resolved).await?;
            crate::postgres::backup::restore_database(&pool, options.source_path, manifest).await
        }
    }
}

pub(crate) fn build_manifest(
    schema_version: i64,
    schema_checksum: String,
    mode: BackupMode,
    tables: Vec<String>,
) -> BackupManifest {
    BackupManifest {
        version: env!("CARGO_PKG_VERSION").to_owned(),
        schema_version,
        schema_checksum,
        timestamp: Utc::now(),
        mode,
        tables,
    }
}

pub(crate) fn order_by_clause(table: &str, quote_identifier: fn(&str) -> String) -> String {
    // Sort each table by its real primary/unique key (falling back to `rowid`)
    // so the exported NDJSON is row-stable: re-exporting unchanged data yields
    // byte-identical files, keeping successive backups diffable.
    let columns = match table {
        "site_config" => &["key"][..],
        "users" => &["user_id"],
        "sessions" | "email_verifications" | "password_resets" => &["token_hash"],
        "invites" => &["code"],
        "posts" => &["post_id"],
        "post_revisions" => &["revision_id"],
        "tags" => &["tag_id"],
        "post_tags" => &["post_id", "tag_id"],
        _ => &["rowid"],
    };
    columns
        .iter()
        .map(|column| quote_identifier(column))
        .collect::<Vec<_>>()
        .join(", ")
}

fn write_manifest(destination_path: &Path, manifest: &BackupManifest) -> Result<(), BackupError> {
    let file = File::create(destination_path.join("manifest.json"))?;
    let mut writer = BufWriter::new(file);
    serde_json::to_writer_pretty(&mut writer, manifest)?;
    writer.write_all(b"\n")?;
    writer.flush()?;
    Ok(())
}

fn read_manifest(source_path: &Path) -> Result<BackupManifest, BackupError> {
    let manifest_path = source_path.join("manifest.json");
    if !manifest_path.is_file() {
        return Err(BackupError::InvalidBackup(format!(
            "missing manifest: {}",
            manifest_path.display()
        )));
    }

    let file = File::open(manifest_path)?;
    Ok(serde_json::from_reader(file)?)
}

fn validate_manifest(manifest: &BackupManifest) -> Result<(), BackupError> {
    let current_version = env!("CARGO_PKG_VERSION");
    if manifest.version != current_version {
        return Err(BackupError::VersionMismatch {
            backup_version: manifest.version.clone(),
            current_version,
        });
    }
    Ok(())
}

pub(crate) fn ensure_schema_version(
    manifest: &BackupManifest,
    target_version: i64,
) -> Result<(), BackupError> {
    if manifest.schema_version != target_version {
        return Err(BackupError::SchemaVersionMismatch {
            backup_version: manifest.schema_version,
            target_version,
        });
    }
    Ok(())
}

pub(crate) fn read_table_rows(
    source_path: &Path,
    table: &str,
) -> Result<Vec<serde_json::Map<String, serde_json::Value>>, BackupError> {
    let path = source_path.join("db").join(format!("{table}.ndjson"));
    if !path.is_file() {
        return Err(BackupError::InvalidBackup(format!(
            "missing table export: {}",
            path.display()
        )));
    }

    let mut rows = Vec::new();
    let file = File::open(path)?;
    for line in BufReader::new(file).lines() {
        let line = line?;
        if line.trim().is_empty() {
            continue;
        }

        let value: serde_json::Value = serde_json::from_str(&line)?;
        let serde_json::Value::Object(row) = value else {
            return Err(BackupError::InvalidBackup(format!(
                "table {table} contains a non-object row"
            )));
        };
        rows.push(row);
    }
    Ok(rows)
}

pub(crate) fn json_value_as_restore_text(value: &serde_json::Value) -> Option<String> {
    match value {
        serde_json::Value::Null => None,
        serde_json::Value::String(value) => Some(value.clone()),
        serde_json::Value::Bool(value) => Some(value.to_string()),
        serde_json::Value::Number(value) => Some(value.to_string()),
        serde_json::Value::Array(_) | serde_json::Value::Object(_) => Some(value.to_string()),
    }
}

fn ensure_empty_or_absent(path: &Path) -> Result<(), BackupError> {
    if !path.exists() {
        return Ok(());
    }
    if fs::read_dir(path)?.next().is_some() {
        return Err(BackupError::DestinationNotEmpty(path.to_path_buf()));
    }
    Ok(())
}

fn ensure_absent(path: &Path) -> Result<(), BackupError> {
    if path.exists() {
        return Err(BackupError::DestinationExists(path.to_path_buf()));
    }
    Ok(())
}

struct TemporaryBackupDirectory {
    path: PathBuf,
}

impl TemporaryBackupDirectory {
    fn near(destination_path: &Path) -> Result<Self, BackupError> {
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

    fn in_temp() -> Result<Self, BackupError> {
        let suffix = Utc::now()
            .timestamp_nanos_opt()
            .unwrap_or_else(|| Utc::now().timestamp_micros());
        let path = std::env::temp_dir().join(format!("jaunder-backup-{suffix}"));
        fs::create_dir(&path)?;
        Ok(Self { path })
    }

    fn path(&self) -> &Path {
        &self.path
    }
}

impl Drop for TemporaryBackupDirectory {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.path);
    }
}

fn write_tar_gz(source_root: &Path, destination_path: &Path) -> Result<(), BackupError> {
    let file = File::create(destination_path)?;
    let encoder = GzEncoder::new(file, Compression::default());
    let mut archive = tar::Builder::new(encoder);
    archive.append_dir_all(".", source_root)?;
    let encoder = archive.into_inner()?;
    encoder.finish()?;
    Ok(())
}

fn extract_archive_backup(source_path: &Path) -> Result<TemporaryBackupDirectory, BackupError> {
    let destination = TemporaryBackupDirectory::in_temp()?;
    let file = File::open(source_path)?;
    let decoder = GzDecoder::new(file);
    let mut archive = tar::Archive::new(decoder);
    archive.unpack(destination.path())?;
    Ok(destination)
}

fn restore_media_directory(source: &Path, destination: &Path) -> Result<(), BackupError> {
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
            if let Some(parent) = destination_path.parent() {
                fs::create_dir_all(parent)?;
            } // cov:ignore
            fs::copy(source_path, destination_path)?;
        } // cov:ignore
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
            )?; // cov:ignore
        } else if metadata.is_file() {
            copy_or_link_media_file(
                &source_path,
                &destination_path,
                previous_backup,
                &child_relative_path,
                // cov:ignore-start
            )?;
        }
        // cov:ignore-stop
    }
    Ok(())
}

fn copy_or_link_media_file(
    source_path: &Path,
    destination_path: &Path,
    previous_backup: Option<&Path>,
    relative_path: &Path,
) -> Result<(), BackupError> {
    if let Some(parent) = destination_path.parent() {
        fs::create_dir_all(parent)?;
    } // cov:ignore

    // Deduplicate against the previous backup: when this file is byte-identical
    // to its counterpart there, hard-link to that copy instead of writing a new
    // one, so a chain of backups doesn't store N copies of an unchanged blob.
    // Fall through to a real copy if the content differs or the link can't be
    // made (e.g. the previous backup is on a different filesystem).
    if let Some(previous_file) = previous_backup
        .map(|backup| backup.join("media").join(relative_path))
        .filter(|path| path.is_file())
    {
        if files_have_same_content(source_path, &previous_file)?
            && fs::hard_link(&previous_file, destination_path).is_ok()
        {
            return Ok(());
        }
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

fn previous_directory_backup(destination_path: &Path) -> Result<Option<PathBuf>, BackupError> {
    let Some(parent) = destination_path.parent() else {
        return Ok(None); // cov:ignore
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
    #![allow(clippy::similar_names)] // parallel fixture names (e.g. user1/user2) aid test clarity
    use super::*;
    use tempfile::TempDir;

    fn quote_test_identifier(identifier: &str) -> String {
        format!("\"{identifier}\"")
    }

    #[test]
    fn order_by_clause_uses_stable_table_keys() {
        assert_eq!(
            order_by_clause("users", quote_test_identifier),
            "\"user_id\""
        );
        assert_eq!(
            order_by_clause("post_tags", quote_test_identifier),
            "\"post_id\", \"tag_id\""
        );
        assert_eq!(
            order_by_clause("sessions", quote_test_identifier),
            "\"token_hash\""
        );
        assert_eq!(
            order_by_clause("email_verifications", quote_test_identifier),
            "\"token_hash\""
        );
        assert_eq!(
            order_by_clause("password_resets", quote_test_identifier),
            "\"token_hash\""
        );
        assert_eq!(
            order_by_clause("invites", quote_test_identifier),
            "\"code\""
        );
        assert_eq!(
            order_by_clause("posts", quote_test_identifier),
            "\"post_id\""
        );
        assert_eq!(
            order_by_clause("post_revisions", quote_test_identifier),
            "\"revision_id\""
        );
        assert_eq!(order_by_clause("tags", quote_test_identifier), "\"tag_id\"");
        assert_eq!(
            order_by_clause("unknown", quote_test_identifier),
            "\"rowid\""
        );
    }

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
            let tmp = TemporaryBackupDirectory::near(&destination)?;
            let p = tmp.path().to_path_buf();
            assert!(p.exists(), "directory should exist before drop");
            p
        };
        assert!(!path.exists(), "directory should be removed after drop");
        Ok(())
    }

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
        )?; // cov:ignore

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
        assert!(!files_have_same_content(
            &source.join("image.txt"),
            &previous.join("media").join("image.txt")
        )?); // cov:ignore
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
    fn read_manifest_rejects_missing_manifest() -> Result<(), BackupError> {
        let temp = TempDir::new()?;
        let error = read_manifest(temp.path()).expect_err("missing manifest");

        assert!(matches!(error, BackupError::InvalidBackup(_)));
        Ok(())
    }

    #[test]
    fn validate_manifest_rejects_wrong_version() {
        let manifest = BackupManifest {
            version: "0.0.0".to_owned(),
            schema_version: 11,
            schema_checksum: "checksum".to_owned(),
            timestamp: Utc::now(),
            mode: BackupMode::Directory,
            tables: Vec::new(),
        };

        let error = validate_manifest(&manifest).expect_err("version mismatch");
        assert!(matches!(error, BackupError::VersionMismatch { .. }));
    }

    #[test]
    fn ensure_schema_version_rejects_mismatch() {
        let manifest = BackupManifest {
            version: env!("CARGO_PKG_VERSION").to_owned(),
            schema_version: 10,
            schema_checksum: "checksum".to_owned(),
            timestamp: Utc::now(),
            mode: BackupMode::Directory,
            tables: Vec::new(),
        };

        let error = ensure_schema_version(&manifest, 11).expect_err("schema mismatch");
        assert!(matches!(error, BackupError::SchemaVersionMismatch { .. }));
    }

    #[test]
    fn read_table_rows_parses_objects_and_rejects_non_objects() -> Result<(), BackupError> {
        let temp = TempDir::new()?;
        let db = temp.path().join("db");
        fs::create_dir(&db)?;
        fs::write(
            db.join("users.ndjson"),
            "{\"user_id\":1}\n\n{\"user_id\":2}\n",
        )?; // cov:ignore

        let rows = read_table_rows(temp.path(), "users")?;
        assert_eq!(rows.len(), 2);
        assert_eq!(rows[0]["user_id"], serde_json::json!(1));

        fs::write(db.join("sessions.ndjson"), "[]\n")?;
        let error = read_table_rows(temp.path(), "sessions").expect_err("non-object row");
        assert!(matches!(error, BackupError::InvalidBackup(_)));
        Ok(())
    }

    #[test]
    fn read_table_rows_rejects_missing_table_file() -> Result<(), BackupError> {
        let temp = TempDir::new()?;
        fs::create_dir(temp.path().join("db"))?;

        let error = read_table_rows(temp.path(), "users").expect_err("missing table");

        assert!(matches!(error, BackupError::InvalidBackup(_)));
        Ok(())
    }

    #[test]
    fn json_value_as_restore_text_converts_scalar_values() {
        assert_eq!(json_value_as_restore_text(&serde_json::Value::Null), None);
        assert_eq!(
            json_value_as_restore_text(&serde_json::json!("text")),
            Some("text".to_owned())
        );
        assert_eq!(
            json_value_as_restore_text(&serde_json::json!(true)),
            Some("true".to_owned())
        );
        assert_eq!(
            json_value_as_restore_text(&serde_json::json!(42)),
            Some("42".to_owned())
        );
    }

    #[test]
    fn json_value_as_restore_text_serializes_compound_values() {
        assert_eq!(
            json_value_as_restore_text(&serde_json::json!(["a", "b"])),
            Some("[\"a\",\"b\"]".to_owned())
        );
        assert_eq!(
            json_value_as_restore_text(&serde_json::json!({"key": "value"})),
            Some("{\"key\":\"value\"}".to_owned())
        );
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
}
