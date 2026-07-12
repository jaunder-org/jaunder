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

// Tables deliberately excluded from backup: `_sqlx_migrations` is schema state
// re-applied by migrations on the restore target, and `feed_cache` is a
// regenerable HTTP response cache. Every other live table is backed up by
// default, so a table added by a future migration is picked up automatically
// rather than silently dropped.
pub(crate) const TABLES_EXCLUDED_FROM_BACKUP: &[&str] = &["_sqlx_migrations", "feed_cache"];

/// The set of tables to back up, derived from the live schema: every table
/// except the `SQLite`-internal `sqlite_%` tables and the explicit
/// [`TABLES_EXCLUDED_FROM_BACKUP`] denylist, sorted for a reproducible manifest.
/// Restore no longer depends on the order (foreign keys are deferred on Postgres
/// and off on `SQLite` during import), so alphabetical is sufficient.
pub(crate) fn backup_table_set(live: impl IntoIterator<Item = String>) -> Vec<String> {
    let mut tables: Vec<String> = live
        .into_iter()
        .filter(|table| {
            !table.starts_with("sqlite_") && !TABLES_EXCLUDED_FROM_BACKUP.contains(&table.as_str())
        })
        .collect();
    tables.sort();
    tables
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
    )?;

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

pub(crate) fn order_by_clause(
    columns: &[ColumnInfo],
    quote_identifier: fn(&str) -> String,
) -> String {
    // Order by every column, in schema order, so the exported NDJSON is
    // row-stable: re-exporting unchanged data yields byte-identical files,
    // keeping successive backups diffable. Ordering by all columns — rather than
    // a hand-maintained per-table key — needs no bespoke entry for a newly added
    // table and works on Postgres, which has no `rowid` to fall back on. (For a
    // table with a leading unique column, e.g. a primary key, this reproduces the
    // old key-only order, since ties never reach the trailing columns.)
    columns
        .iter()
        .map(|column| quote_identifier(&column.name))
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
    // Parallel fixture names (e.g. user1/user2) aid test clarity. Single-compiled test
    // module, so `#[expect]` self-removes if the names ever diverge. (#94)
    #![expect(clippy::similar_names)]
    use super::*;
    use crate::test_support::{
        backends, recorded_postgres_url, sqlite_only, sqlite_url, Backend, CloseablePool,
    };
    use rstest::*;
    use rstest_reuse::*;
    use std::str::FromStr;
    use tempfile::TempDir;

    /// The [`DbConnectOptions`] addressing the database behind a backend's
    /// [`Backend::setup`] test env, so a backend-parametric backup test can drive
    /// the public `export_backup`/`restore_backup` API against either backend.
    fn backup_db_options(
        backend: Backend,
        base: &TempDir,
    ) -> Result<DbConnectOptions, BackupError> {
        Ok(match backend {
            Backend::Sqlite => sqlite_url(base),
            Backend::Postgres => DbConnectOptions::from_str(&recorded_postgres_url(base))?,
        })
    }

    fn quote_test_identifier(identifier: &str) -> String {
        format!("\"{identifier}\"")
    }

    #[test]
    fn order_by_clause_orders_by_every_column_in_schema_order() {
        let columns = [
            ColumnInfo {
                name: "post_id".to_owned(),
                type_name: "integer".to_owned(),
            },
            ColumnInfo {
                name: "tag_id".to_owned(),
                type_name: "integer".to_owned(),
            },
        ];
        assert_eq!(
            order_by_clause(&columns, quote_test_identifier),
            "\"post_id\", \"tag_id\""
        );

        let single = [ColumnInfo {
            name: "user_id".to_owned(),
            type_name: "integer".to_owned(),
        }];
        assert_eq!(
            order_by_clause(&single, quote_test_identifier),
            "\"user_id\""
        );
    }

    #[test]
    fn backup_table_set_drops_internal_and_denylisted_and_sorts() {
        let live = [
            "posts",
            "users",
            "feed_cache",
            "_sqlx_migrations",
            "sqlite_sequence",
            "channels",
        ]
        .into_iter()
        .map(str::to_owned);
        assert_eq!(
            backup_table_set(live),
            vec![
                "channels".to_owned(),
                "posts".to_owned(),
                "users".to_owned()
            ]
        );
    }

    // Guardrail: a real export of a fresh database backs up exactly the expected
    // set of tables, and every live table is either backed up or a deliberate
    // exclusion. A migration that adds a table trips the golden assertion (if
    // auto-included) or the count assertion (if denylisted), forcing the coverage
    // decision to be made consciously rather than by omission.
    #[apply(backends)]
    #[tokio::test]
    async fn backup_covers_every_table_or_deliberately_excludes_it(
        #[case] backend: Backend,
    ) -> Result<(), BackupError> {
        let env = backend.setup().await;
        let temp = TempDir::new()?;
        let media = temp.path().join("media");
        fs::create_dir_all(&media)?;
        let db = backup_db_options(backend, &env.base)?;
        let manifest = export_backup(BackupExportOptions {
            database: &db,
            media_path: &media,
            destination_path: &temp.path().join("backup"),
            mode: BackupMode::Directory,
        })
        .await?;

        let mut tables = manifest.tables.clone();
        tables.sort();
        let expected: Vec<String> = [
            "audience_members",
            "audiences",
            "channels",
            "email_verifications",
            "feed_events",
            "invites",
            "media",
            "password_resets",
            "post_audiences",
            "post_revisions",
            "post_tags",
            "posts",
            "sessions",
            "site_config",
            "subscription_statuses",
            "subscriptions",
            "tags",
            "target_kinds",
            "user_config",
            "users",
        ]
        .iter()
        .map(|table| (*table).to_owned())
        .collect();
        assert_eq!(
            tables, expected,
            "backup set drifted — add the new table to the golden list or to TABLES_EXCLUDED_FROM_BACKUP"
        );

        // Bidirectional: the whole schema is 20 backed-up tables + feed_cache +
        // _sqlx_migrations. A table added and then denylisted (so the manifest
        // stays 20) still trips this count.
        let live_count: i64 = match env.base.pool() {
            CloseablePool::Sqlite(pool) => sqlx::query_scalar(
                "SELECT COUNT(*) FROM sqlite_master WHERE type = 'table' AND name NOT LIKE 'sqlite_%'",
            )
            .fetch_one(pool)
            .await?,
            CloseablePool::Postgres(pool) => sqlx::query_scalar(
                "SELECT COUNT(*) FROM information_schema.tables \
                 WHERE table_schema = 'public' AND table_type = 'BASE TABLE'",
            )
            .fetch_one(pool)
            .await?,
        };
        assert_eq!(
            live_count, 22,
            "a table was added or removed — update the golden set and denylist deliberately"
        );

        Ok(())
    }

    // The restore emptiness guard: a freshly-initialized database counts as empty
    // (only the migration-seeded lookups are populated), and any other row makes
    // it non-empty. A future migration that seeds a new table would make the fresh
    // database read as non-empty and fail the first assertion here.
    #[apply(backends)]
    #[tokio::test]
    async fn database_is_empty_ignores_only_seeded_lookups(
        #[case] backend: Backend,
    ) -> Result<(), BackupError> {
        let env = backend.setup().await;
        let db = backup_db_options(backend, &env.base)?;

        assert!(
            crate::database_is_empty(&db).await?,
            "a freshly-initialized database must count as empty"
        );
        for table in ["channels", "subscription_statuses", "target_kinds"] {
            let count: i64 = match env.base.pool() {
                CloseablePool::Sqlite(pool) => {
                    sqlx::query_scalar::<_, i64>(&format!("SELECT COUNT(*) FROM {table}"))
                        .fetch_one(pool)
                        .await?
                }
                CloseablePool::Postgres(pool) => {
                    sqlx::query_scalar::<_, i64>(&format!("SELECT COUNT(*) FROM {table}"))
                        .fetch_one(pool)
                        .await?
                }
            };
            assert!(count > 0, "{table} must be seeded by migrations");
        }

        // A single non-seeded row (a user) makes the database non-empty.
        env.state
            .users
            .create_user(
                &"alice".parse().expect("valid username"),
                &"password123".parse().expect("valid password"),
                None,
                false,
            )
            .await
            .expect("create user");
        assert!(
            !crate::database_is_empty(&db).await?,
            "a database holding a user must not count as empty"
        );
        Ok(())
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
        assert!(!files_have_same_content(
            &source.join("image.txt"),
            &previous.join("media").join("image.txt")
        )
        .expect("compare source and previous media files"));
        Ok(())
    }

    #[test]
    fn mirror_media_propagates_copy_failure() -> Result<(), BackupError> {
        // Structural (root-immune) fs failure: pre-create the destination file
        // path as a *directory* so `fs::copy` into it fails with EISDIR. The
        // error propagates out of `copy_or_link_media_file` and back up through
        // the recursive `mirror_media_entries` call — both `?` arms that were
        // previously cov:ignored.
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
        // restore walk takes the fallthrough arm (previously cov:ignored) and
        // silently skips it, while a sibling regular file still copies.
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

    #[apply(sqlite_only)]
    #[tokio::test]
    async fn export_propagates_media_mirror_failure(
        #[case] backend: Backend,
        // reason: the media-mirror `?` propagation in `export_directory_backup` is
        // backend-independent filesystem code; SQLite exercises it fully.
    ) -> Result<(), BackupError> {
        let source = backend.setup().await;
        let temp = TempDir::new()?;
        // A *regular file* where a media directory is expected: `mirror` calls
        // `fs::read_dir` on it, which fails with ENOTDIR. That structural error
        // propagates out of `mirror_media_directory` and through the
        // `export_directory_backup` `?` that was cov:ignored.
        let media = temp.path().join("media-not-a-dir");
        fs::write(&media, "not a directory")?;
        let backup = temp.path().join("backup");
        let source_db = backup_db_options(backend, &source.base)?;

        let error = export_backup(BackupExportOptions {
            database: &source_db,
            media_path: &media,
            destination_path: &backup,
            mode: BackupMode::Directory,
        })
        .await
        .expect_err("a dangling media symlink must fail the export");
        assert!(matches!(error, BackupError::Io(_)));
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
        )
        .expect("write users.ndjson fixture");

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
    // Both backends' restore path shares the ragged-NDJSON contract: a row that
    // omits a column present in row 0 is rejected as `InvalidBackup`, and the
    // failed import rolls the restore transaction back. One `#[apply(backends)]`
    // test covers the SQLite and PostgreSQL `import_table` missing-column arms
    // plus the PostgreSQL `restore_database` rollback arm.
    #[apply(backends)]
    #[tokio::test]
    async fn restore_rejects_row_missing_a_column(
        #[case] backend: Backend,
    ) -> Result<(), BackupError> {
        let source = backend.setup().await;
        // Two users so the exported users.ndjson has a later row to corrupt while
        // leaving row 0 (which seeds `column_names`) complete.
        for username in ["userone", "usertwo"] {
            source
                .state
                .users
                .create_user(
                    &username.parse().expect("valid username"),
                    &"password123".parse().expect("valid password"),
                    None,
                    false,
                )
                .await
                .expect("seed user");
        }

        let temp = TempDir::new()?;
        let media = temp.path().join("media");
        fs::create_dir_all(&media)?;
        // A regular media file so the export mirrors it through the `is_file`
        // success branch (not just the empty-dir / failure paths).
        fs::write(media.join("avatar.txt"), b"img")?;
        let backup = temp.path().join("backup");

        let source_db = backup_db_options(backend, &source.base)?;
        export_backup(BackupExportOptions {
            database: &source_db,
            media_path: &media,
            destination_path: &backup,
            mode: BackupMode::Directory,
        })
        .await?;

        // Drop a column from the last exported user row: row 0 still carries every
        // column (so `column_names` includes it), but the last row omits it, so the
        // per-row bind trips the missing-column check during restore.
        let users_ndjson = backup.join("db").join("users.ndjson");
        let mut rows: Vec<serde_json::Map<String, serde_json::Value>> =
            fs::read_to_string(&users_ndjson)?
                .lines()
                .filter(|line| !line.trim().is_empty())
                .map(serde_json::from_str)
                .collect::<Result<_, _>>()?;
        assert!(rows.len() >= 2, "expected at least two exported user rows");
        let victim = rows[0]
            .keys()
            .next()
            .expect("exported row has columns")
            .clone();
        rows.last_mut().expect("non-empty rows").remove(&victim);
        let mut corrupted = String::new();
        for row in &rows {
            corrupted.push_str(&serde_json::to_string(row)?);
            corrupted.push('\n');
        }
        fs::write(&users_ndjson, corrupted)?;

        let target = backend.setup().await;
        let target_db = backup_db_options(backend, &target.base)?;
        let error = restore_backup(BackupRestoreOptions {
            database: &target_db,
            media_path: &temp.path().join("restored-media"),
            source_path: &backup,
        })
        .await
        .expect_err("restore should reject a row missing a column");

        assert!(matches!(error, BackupError::InvalidBackup(_)));
        Ok(())
    }
}
