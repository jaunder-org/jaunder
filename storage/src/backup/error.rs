//! Backup error surface shared by the backup concerns and database backends.

use std::path::PathBuf;

use thiserror::Error;

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
        "backup format version {backup_version} is unsupported; this binary supports version {current_version}"
    )]
    UnsupportedFormatVersion {
        backup_version: u32,
        current_version: u32,
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
