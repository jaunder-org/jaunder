use std::{
    fs::{self, File},
    io::{BufWriter, Write},
    path::{Path, PathBuf},
};

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use thiserror::Error;

use super::{resolved_postgres_options, DbConnectOptions};

mod postgres;
mod sqlite;

pub(super) const TABLES_IN_EXPORT_ORDER: &[&str] = &[
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

#[derive(Debug, Clone, Copy, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BackupMode {
    Directory,
}

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
}

#[derive(Debug, Clone)]
pub(super) struct ColumnInfo {
    pub(super) name: String,
    pub(super) type_name: String,
}

pub async fn export_backup(
    options: BackupExportOptions<'_>,
) -> Result<BackupManifest, BackupError> {
    match options.mode {
        BackupMode::Directory => export_directory_backup(options).await,
    }
}

async fn export_directory_backup(
    options: BackupExportOptions<'_>,
) -> Result<BackupManifest, BackupError> {
    ensure_empty_or_absent(options.destination_path)?;
    fs::create_dir_all(options.destination_path.join("db"))?;

    let manifest = match options.database {
        DbConnectOptions::Sqlite(connect_options) => {
            let pool = sqlx::SqlitePool::connect_with(connect_options.clone()).await?;
            sqlite::export_database(&pool, options.destination_path, options.mode).await?
        }
        DbConnectOptions::Postgres {
            options: pg_options,
            ..
        } => {
            let resolved = resolved_postgres_options(pg_options)?;
            let pool = sqlx::PgPool::connect_with(resolved).await?;
            postgres::export_database(&pool, options.destination_path, options.mode).await?
        }
    };

    let previous_backup = previous_directory_backup(options.destination_path)?;
    mirror_media_directory(
        options.media_path,
        &options.destination_path.join("media"),
        previous_backup.as_deref(),
    )?;

    write_manifest(options.destination_path, &manifest)?;
    Ok(manifest)
}

pub(super) fn build_manifest(
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

pub(super) fn order_by_clause(table: &str, quote_identifier: fn(&str) -> String) -> String {
    let columns = match table {
        "site_config" => &["key"][..],
        "users" => &["user_id"],
        "sessions" => &["token_hash"],
        "invites" => &["code"],
        "email_verifications" => &["token_hash"],
        "password_resets" => &["token_hash"],
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

fn ensure_empty_or_absent(path: &Path) -> Result<(), BackupError> {
    if !path.exists() {
        return Ok(());
    }
    if fs::read_dir(path)?.next().is_some() {
        return Err(BackupError::DestinationNotEmpty(path.to_path_buf()));
    }
    Ok(())
}

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
        }
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
    }

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
    let parent = match destination_path.parent() {
        Some(parent) => parent,
        None => return Ok(None),
    };
    if !parent.exists() {
        return Ok(None);
    }

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
    use std::str::FromStr;
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
        )?;

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
        )?);
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

    #[tokio::test]
    async fn export_backup_writes_ndjson_media_and_manifest() -> Result<(), BackupError> {
        let temp = TempDir::new()?;
        let database_url = format!("sqlite://{}", temp.path().join("test.db").display());
        let options =
            sqlx::sqlite::SqliteConnectOptions::from_str(&database_url)?.create_if_missing(true);
        let pool = sqlx::SqlitePool::connect_with(options).await?;
        sqlx::migrate!("./migrations/sqlite").run(&pool).await?;
        sqlx::query(
            "INSERT INTO users (username, password_hash, created_at, is_operator)
             VALUES ('admin', 'hash', '2026-04-29T00:00:00Z', TRUE)",
        )
        .execute(&pool)
        .await?;

        let media = temp.path().join("media");
        fs::create_dir_all(&media)?;
        fs::write(media.join("avatar.txt"), "image")?;

        let previous = temp.path().join("backup-previous");
        fs::create_dir_all(previous.join("media"))?;
        fs::write(previous.join("manifest.json"), "{}")?;
        fs::write(previous.join("media").join("avatar.txt"), "image")?;

        let backup_path = temp.path().join("backup");
        let db_options = DbConnectOptions::from_str(&database_url)?;
        let manifest = export_backup(BackupExportOptions {
            database: &db_options,
            media_path: &media,
            destination_path: &backup_path,
            mode: BackupMode::Directory,
        })
        .await?;

        assert!(manifest.tables.contains(&"users".to_owned()));
        let users = fs::read_to_string(backup_path.join("db").join("users.ndjson"))?;
        assert!(users.contains("\"username\":\"admin\""));
        assert!(users.contains("\"is_operator\":true"));
        assert!(backup_path.join("manifest.json").is_file());
        assert_eq!(
            fs::read_to_string(backup_path.join("media").join("avatar.txt"))?,
            "image"
        );
        Ok(())
    }
}
