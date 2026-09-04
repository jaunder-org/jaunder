//! Backup manifest, schema, table-set, and NDJSON format mechanics.

use std::{
    fs::File,
    io::{BufRead, BufReader, BufWriter, Write},
    path::Path,
};

use common::time::UtcInstant;
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

use super::{BackupMode, error::BackupError};

const LEGACY_BACKUP_FORMAT_VERSION: u32 = 1;
pub(crate) const CURRENT_BACKUP_FORMAT_VERSION: u32 = LEGACY_BACKUP_FORMAT_VERSION;

const fn legacy_backup_format_version() -> u32 {
    LEGACY_BACKUP_FORMAT_VERSION
}

// Tables deliberately excluded from backup: _sqlx_migrations is schema state and feed_cache is regenerable.
pub(crate) const TABLES_EXCLUDED_FROM_BACKUP: &[&str] = &["_sqlx_migrations", "feed_cache"];

/// The set of tables to back up, derived from the live schema.
pub(crate) fn table_set(live: impl IntoIterator<Item = String>) -> Vec<String> {
    let mut tables: Vec<String> = live
        .into_iter()
        .filter(|table| {
            !table.starts_with("sqlite_") && !TABLES_EXCLUDED_FROM_BACKUP.contains(&table.as_str())
        })
        .collect();
    tables.sort();
    tables
}

/// Orders manifest tables for import where SQL constraints cannot be deferred.
///
/// The manifest remains alphabetical and reproducible; restore derives this
/// dependency order so revision-media triggers only observe a completed
/// `Post → Revision → child` chain. Unlisted tables retain manifest order.
pub(crate) fn restore_table_order(tables: &[String]) -> Vec<&str> {
    let mut ordered = tables.iter().map(String::as_str).collect::<Vec<_>>();
    ordered.sort_by_key(|table| match *table {
        "users" | "channels" | "subscription_statuses" | "target_kinds" => 1,
        "audiences" | "subscriptions" | "media" | "posts" | "user_config" => 2,
        "audience_members"
        | "email_verifications"
        | "idempotency_keys"
        | "password_resets"
        | "post_audiences"
        | "post_tags"
        | "sessions"
        | "post_revisions" => 3,
        "post_revision_audiences" | "post_revision_tags" => 4,
        "post_media" => 5,
        _ => 0,
    });
    ordered
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BackupManifest {
    #[serde(default = "legacy_backup_format_version")]
    pub format_version: u32,
    pub version: String,
    pub schema_version: i64,
    pub schema_checksum: String,
    pub timestamp: UtcInstant,
    pub mode: super::BackupMode,
    pub tables: Vec<String>,
}

#[derive(Debug, Clone)]
pub(crate) struct ColumnInfo {
    pub(crate) name: String,
    pub(crate) type_name: String,
}

pub(crate) fn build_manifest(
    schema_version: i64,
    schema_checksum: String,
    mode: BackupMode,
    tables: Vec<String>,
) -> BackupManifest {
    BackupManifest {
        format_version: CURRENT_BACKUP_FORMAT_VERSION,
        version: env!("CARGO_PKG_VERSION").to_owned(),
        schema_version,
        schema_checksum,
        timestamp: UtcInstant::now(),
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
    // table with a leading unique column, e.g. a primary key, ties never reach the
    // trailing columns, so this equals key-only order.)
    columns
        .iter()
        .map(|column| quote_identifier(&column.name))
        .collect::<Vec<_>>()
        .join(", ")
}

pub(super) fn write_manifest(
    destination_path: &Path,
    manifest: &BackupManifest,
) -> Result<(), BackupError> {
    let file = File::create(destination_path.join("manifest.json"))?;
    let mut writer = BufWriter::new(file);
    serde_json::to_writer_pretty(&mut writer, manifest)?;
    writer.write_all(b"\n")?;
    writer.flush()?;
    Ok(())
}

pub(super) fn read_manifest(source_path: &Path) -> Result<BackupManifest, BackupError> {
    let manifest_path = source_path.join("manifest.json");
    if !manifest_path.is_file() {
        return Err(BackupError::InvalidBackup(format!(
            "missing manifest: {}",
            manifest_path.display()
        )));
    }

    let file = File::open(&manifest_path)?;
    serde_json::from_reader(file).map_err(|error| {
        BackupError::InvalidBackup(format!(
            "malformed manifest {}: {error}",
            manifest_path.display()
        ))
    })
}

pub(super) fn validate_manifest(manifest: &BackupManifest) -> Result<(), BackupError> {
    if manifest.format_version != CURRENT_BACKUP_FORMAT_VERSION {
        return Err(BackupError::UnsupportedFormatVersion {
            backup_version: manifest.format_version,
            current_version: CURRENT_BACKUP_FORMAT_VERSION,
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
) -> Result<Vec<Map<String, Value>>, BackupError> {
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

        let value: Value = serde_json::from_str(&line)?;
        let Value::Object(row) = value else {
            return Err(BackupError::InvalidBackup(format!(
                "table {table} contains a non-object row"
            )));
        };
        rows.push(row);
    }
    Ok(rows)
}

pub(crate) fn json_value_as_restore_text(value: &Value) -> Option<String> {
    match value {
        Value::Null => None,
        Value::String(value) => Some(value.clone()),
        Value::Bool(value) => Some(value.to_string()),
        Value::Number(value) => Some(value.to_string()),
        Value::Array(_) | Value::Object(_) => Some(value.to_string()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::TempDir;

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
            table_set(live),
            vec![
                "channels".to_owned(),
                "posts".to_owned(),
                "users".to_owned()
            ]
        );
    }

    #[test]
    fn restore_table_order_loads_revision_subject_parents_before_media() {
        let tables = [
            "post_media",
            "post_revision_tags",
            "post_revisions",
            "posts",
            "users",
            "post_revision_audiences",
        ]
        .into_iter()
        .map(str::to_owned)
        .collect::<Vec<_>>();
        let ordered = restore_table_order(&tables);
        let position = |table| {
            ordered
                .iter()
                .position(|candidate| *candidate == table)
                .expect("table remains in restore order")
        };
        assert!(position("users") < position("posts"));
        assert!(position("posts") < position("post_revisions"));
        assert!(position("post_revisions") < position("post_revision_tags"));
        assert!(position("post_revisions") < position("post_revision_audiences"));
        assert!(position("post_revision_tags") < position("post_media"));
        assert!(position("post_revision_audiences") < position("post_media"));
    }

    #[test]
    fn backup_manifest_serializes_format_version_and_timestamp() {
        let manifest = BackupManifest {
            format_version: 1,
            version: "0.1.0".to_owned(),
            schema_version: 1,
            schema_checksum: "checksum".to_owned(),
            timestamp: "2026-08-26T01:02:03.123456Z"
                .parse()
                .expect("valid instant"),
            mode: BackupMode::Directory,
            tables: Vec::new(),
        };

        let json = serde_json::to_value(&manifest).expect("manifest serializes");
        assert_eq!(json["format_version"], 1);
        assert_eq!(json["timestamp"], "2026-08-26T01:02:03.123456Z");
        assert_eq!(
            serde_json::from_value::<BackupManifest>(json)
                .expect("manifest deserializes")
                .timestamp,
            manifest.timestamp
        );
    }

    #[test]
    fn absent_format_version_is_legacy_v1_but_malformed_value_is_rejected() {
        let legacy = serde_json::json!({
            "version": "0.1.0",
            "schema_version": 1,
            "schema_checksum": "checksum",
            "timestamp": "2026-08-26T01:02:03Z",
            "mode": "directory",
            "tables": []
        });
        let manifest =
            serde_json::from_value::<BackupManifest>(legacy).expect("legacy v1 manifest");
        assert_eq!(manifest.format_version, 1);

        let malformed = serde_json::json!({
            "format_version": "1",
            "version": "0.1.0",
            "schema_version": 1,
            "schema_checksum": "checksum",
            "timestamp": "2026-08-26T01:02:03Z",
            "mode": "directory",
            "tables": []
        });
        serde_json::from_value::<BackupManifest>(malformed)
            .expect_err("malformed explicit format version");
    }

    #[test]
    fn read_manifest_classifies_malformed_manifest_as_invalid_backup() -> Result<(), BackupError> {
        let temp = TempDir::new()?;
        fs::write(
            temp.path().join("manifest.json"),
            r#"{"format_version":"1"}"#,
        )?;

        let error = read_manifest(temp.path()).expect_err("malformed manifest");
        assert!(matches!(error, BackupError::InvalidBackup(_)));
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
    fn validate_manifest_accepts_different_package_version() {
        let manifest = BackupManifest {
            format_version: CURRENT_BACKUP_FORMAT_VERSION,
            version: "another-release".to_owned(),
            schema_version: 11,
            schema_checksum: "checksum".to_owned(),
            timestamp: UtcInstant::now(),
            mode: BackupMode::Directory,
            tables: Vec::new(),
        };

        validate_manifest(&manifest).expect("package version is provenance");
    }

    #[test]
    fn validate_manifest_rejects_unsupported_format_version() {
        let manifest = BackupManifest {
            format_version: 2,
            version: env!("CARGO_PKG_VERSION").to_owned(),
            schema_version: 11,
            schema_checksum: "checksum".to_owned(),
            timestamp: UtcInstant::now(),
            mode: BackupMode::Directory,
            tables: Vec::new(),
        };

        let error = validate_manifest(&manifest).expect_err("unsupported format");
        assert!(matches!(
            error,
            BackupError::UnsupportedFormatVersion {
                backup_version: 2,
                current_version: CURRENT_BACKUP_FORMAT_VERSION
            }
        ));
    }

    #[test]
    fn ensure_schema_version_rejects_legacy_schema() {
        let manifest = BackupManifest {
            format_version: CURRENT_BACKUP_FORMAT_VERSION,
            version: env!("CARGO_PKG_VERSION").to_owned(),
            schema_version: 26,
            schema_checksum: "checksum".to_owned(),
            timestamp: UtcInstant::now(),
            mode: BackupMode::Directory,
            tables: Vec::new(),
        };

        let error = ensure_schema_version(&manifest, 28).expect_err("schema mismatch");
        assert!(matches!(
            error,
            BackupError::SchemaVersionMismatch {
                backup_version: 26,
                target_version: 28
            }
        ));
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
}
