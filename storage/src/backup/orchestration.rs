//! Public backup operations and their database, archive, and media sequencing.

use std::{fs, path::Path};

use crate::sqlite::resolved_sqlite_options;
use crate::{DbConnectOptions, StorageRuntimeConfig, resolved_postgres_options};

use super::{
    BackupMode, archive,
    error::BackupError,
    format::{BackupManifest, read_manifest, validate_manifest, write_manifest},
    media::{mirror_media_directory, previous_directory_backup, restore_media_directory},
    restore_validation::{BackupRestoreOutcome, RestoreValidationReport},
};

#[derive(Clone, Copy)]
pub struct BackupExportOptions<'a> {
    pub database: &'a DbConnectOptions,
    pub runtime: &'a StorageRuntimeConfig,
    pub media_path: &'a Path,
    pub destination_path: &'a Path,
    pub mode: BackupMode,
}

#[derive(Clone, Copy)]
pub struct BackupRestoreOptions<'a> {
    pub database: &'a DbConnectOptions,
    pub runtime: &'a StorageRuntimeConfig,
    pub media_path: &'a Path,
    pub source_path: &'a Path,
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
) -> Result<BackupRestoreOutcome, BackupError> {
    let extracted_archive = if options.source_path.is_file() {
        Some(archive::extract_archive_backup(options.source_path)?)
    } else {
        None
    };
    let source_path = extracted_archive
        .as_ref()
        .map_or(options.source_path, archive::TemporaryBackupDirectory::path);

    let manifest = read_manifest(source_path)?;
    validate_manifest(&manifest)?;

    let validation_report = match manifest.mode {
        BackupMode::Directory | BackupMode::Archive => {
            restore_directory_backup(
                BackupRestoreOptions {
                    database: options.database,
                    runtime: options.runtime,
                    media_path: options.media_path,
                    source_path,
                },
                &manifest,
            )
            .await?
        }
    };

    restore_media_directory(&source_path.join("media"), options.media_path)?;
    Ok(BackupRestoreOutcome {
        manifest,
        validation_report,
    })
}

async fn export_archive_backup(
    options: BackupExportOptions<'_>,
) -> Result<BackupManifest, BackupError> {
    archive::ensure_absent(options.destination_path)?;
    let staging = archive::TemporaryBackupDirectory::near(options.destination_path)?;
    let manifest = export_directory_backup(BackupExportOptions {
        database: options.database,
        runtime: options.runtime,
        media_path: options.media_path,
        destination_path: staging.path(),
        mode: BackupMode::Archive,
    })
    .await?;
    archive::write_tar_gz(staging.path(), options.destination_path)?;
    Ok(manifest)
}

async fn export_directory_backup(
    options: BackupExportOptions<'_>,
) -> Result<BackupManifest, BackupError> {
    archive::ensure_empty_or_absent(options.destination_path)?;
    fs::create_dir_all(options.destination_path.join("db"))?;

    let manifest = match options.database {
        DbConnectOptions::Sqlite(connect_options) => {
            let resolved = resolved_sqlite_options(connect_options, options.runtime);
            let pool = sqlx::SqlitePool::connect_with(resolved).await?;
            crate::sqlite::backup::export_database(&pool, options.destination_path, options.mode)
                .await?
        }
        DbConnectOptions::Postgres {
            options: pg_options,
            ..
        } => {
            let resolved = resolved_postgres_options(pg_options, options.runtime);
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
) -> Result<RestoreValidationReport, BackupError> {
    if !options.source_path.join("db").is_dir() {
        return Err(BackupError::InvalidBackup(format!(
            "missing db directory: {}",
            options.source_path.join("db").display()
        )));
    }

    match options.database {
        DbConnectOptions::Sqlite(connect_options) => {
            let resolved = resolved_sqlite_options(connect_options, options.runtime);
            let pool = sqlx::SqlitePool::connect_with(resolved).await?;
            crate::sqlite::backup::restore_database(&pool, options.source_path, manifest).await
        }
        DbConnectOptions::Postgres {
            options: pg_options,
            ..
        } => {
            let resolved = resolved_postgres_options(pg_options, options.runtime);
            let pool = sqlx::PgPool::connect_with(resolved).await?;
            crate::postgres::backup::restore_database(&pool, options.source_path, manifest).await
        }
    }
}
