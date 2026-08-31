//! Database + media backup: exports each table to per-table NDJSON and mirrors
//! the media tree, as either a directory or a gzipped tar archive; restore
//! reverses it. Media is content-hash deduplicated against the previous backup
//! via hard links, so a series of backups doesn't re-store unchanged blobs.

mod archive;
mod catalog;
mod error;
mod format;
mod media;
mod orchestration;
mod restore_bind;
mod restore_validation;

pub use common::backup::BackupMode;
pub use error::BackupError;
pub use format::BackupManifest;
pub use orchestration::{BackupExportOptions, BackupRestoreOptions, export_backup, restore_backup};
pub(crate) use restore_bind::{RestoreBindValue, RestoreText};

#[cfg(test)]
pub(crate) use catalog::CatalogDatabaseName;
pub use restore_validation::{
    BackupRestoreOutcome, RestoreValidationIssue, RestoreValidationReport,
};

pub(crate) use catalog::{
    BackupRowJson, CatalogColumnName, CatalogDefinition, CatalogNullability, CatalogTableName,
    CatalogTypeName, MigrationVersion,
};
pub(crate) use format::{
    ColumnInfo, backup_table_set, build_manifest, ensure_schema_version, order_by_clause,
    read_table_rows, restore_table_order,
};
pub(crate) use restore_validation::{validate_instance_identity_backup, validate_restore_row};
