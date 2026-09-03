use std::{
    fs::File,
    io::{BufWriter, Write},
    path::Path,
};

use futures_util::TryStreamExt;
use sha2::{Digest, Sha256};
use sqlx::{Row, SqliteConnection, SqlitePool, error::ErrorKind};

use crate::backup::{
    self, BackupError, BackupManifest, BackupMode, BackupRowJson, CatalogColumnName,
    CatalogDefinition, CatalogTableName, CatalogTypeName, ColumnInfo, MigrationVersion,
    RestoreBindValue, RestoreText, RestoreValidationReport,
};
use crate::helpers;
use crate::sql;
use crate::sql::QueryStorageExt;

fn finish_export_rollback(
    primary: Result<BackupManifest, BackupError>,
    rollback: Result<(), sqlx::Error>,
) -> Result<BackupManifest, BackupError> {
    helpers::preserve_after_secondary(
        primary,
        rollback,
        host::error::ErrorKind::Storage,
        host::error::ErrorClass::Transient,
        "storage.sqlite.backup.export.rollback",
    )
}

fn finish_restore_rollback<T>(
    primary: Result<T, BackupError>,
    rollback: Result<(), sqlx::Error>,
) -> Result<T, BackupError> {
    helpers::preserve_after_secondary(
        primary,
        rollback,
        host::error::ErrorKind::Storage,
        host::error::ErrorClass::Transient,
        "storage.sqlite.backup.restore.rollback",
    )
}

fn finish_foreign_key_restore<T>(
    primary: Result<T, BackupError>,
    foreign_keys: Result<(), sqlx::Error>,
) -> Result<T, BackupError> {
    helpers::preserve_after_secondary(
        primary,
        foreign_keys,
        host::error::ErrorKind::Storage,
        host::error::ErrorClass::Transient,
        "storage.sqlite.backup.restore.foreign_keys_restore",
    )
}
/// Keep the public restore error category independent of which live schema
/// constraint rejected a raw backup value. Non-constraint database failures
/// remain infrastructure errors.
fn map_restore_error(error: sqlx::Error) -> BackupError {
    let constraint_message = error
        .as_database_error()
        .filter(|db| {
            db.is_unique_violation()
                || db.is_foreign_key_violation()
                || db.kind() == ErrorKind::NotNullViolation
                || db.is_check_violation()
        })
        .map(|db| db.message().to_owned());
    match constraint_message {
        Some(message) => BackupError::ConstraintViolation(message),
        None => BackupError::Sqlx(error),
    }
}

pub(crate) async fn export_database(
    pool: &SqlitePool,
    destination_path: &Path,
    mode: BackupMode,
) -> Result<BackupManifest, BackupError> {
    let mut connection = pool.acquire().await?;
    // BEGIN IMMEDIATE takes the write lock up front so the export reads one
    // consistent snapshot: no writer can interleave across the multi-table read.
    // (Postgres gets the same guarantee from a REPEATABLE READ snapshot.)
    sqlx::query("BEGIN IMMEDIATE")
        .execute(&mut *connection)
        .await?;

    let result = async {
        let tables = existing_export_tables(&mut connection).await?;
        let schema_version = schema_version(&mut connection).await?;
        let schema_checksum = schema_checksum(&mut connection).await?;

        for table in &tables {
            let table_name = CatalogTableName::from(table.as_str());
            let columns = columns(&mut connection, &table_name).await?;
            export_table(&mut connection, destination_path, table, &columns).await?;
        }

        Ok(backup::build_manifest(
            schema_version,
            schema_checksum,
            mode,
            tables,
        ))
    }
    .await;

    match result {
        Ok(manifest) => {
            sqlx::query("COMMIT").execute(&mut *connection).await?;
            Ok(manifest)
        }
        // Export rollback is unreachable through the public `export_backup`
        // (`export_directory_backup` always creates the `db/` subdir before the
        // dialect writes). It is exercised directly by pointing the dialect
        // `export_database` at a destination lacking `db/` (see tests). Parity
        // with postgres/backup.rs.
        Err(error) => finish_export_rollback(
            Err(error),
            sqlx::query("ROLLBACK")
                .execute(&mut *connection)
                .await
                .map(|_| ()),
        ),
    }
}

pub(crate) async fn restore_database(
    pool: &SqlitePool,
    source_path: &Path,
    manifest: &BackupManifest,
) -> Result<RestoreValidationReport, BackupError> {
    let mut connection = pool.acquire().await?;
    let schema_version = schema_version(&mut connection).await?;
    backup::ensure_schema_version(manifest, schema_version)?;
    backup::validate_instance_identity_backup(source_path, manifest)?;
    // Disable FK enforcement for the bulk import so rows need not be inserted in
    // referential order; integrity is verified once at the end via
    // `foreign_key_check`. FKs are re-enabled on every exit path below, since the
    // connection returns to the pool.
    sqlx::query("PRAGMA foreign_keys = OFF")
        .execute(&mut *connection)
        .await?;
    sqlx::query("BEGIN IMMEDIATE")
        .execute(&mut *connection)
        .await?;

    let result = async {
        let mut validation_report = RestoreValidationReport::default();
        // Clear every table before loading any, keeping the two backends' restore
        // shape identical (docs/adr/0115-clear-then-load-restore.md).
        for table in &manifest.tables {
            sqlx::query(&format!("DELETE FROM {}", sql::quote_identifier(table)))
                .execute(&mut *connection)
                .await
                .map_err(map_restore_error)?;
        }
        for table in backup::restore_table_order(&manifest.tables) {
            let table_name = CatalogTableName::from(table);
            let columns = columns(&mut connection, &table_name).await?;
            import_table(
                &mut connection,
                source_path,
                table,
                &columns,
                &mut validation_report,
            )
            .await?;
        }
        // Validate FKs *before* committing so a violation rolls the whole restore
        // back rather than leaving invalid data committed. `foreign_key_check`
        // scans for violations and works with `foreign_keys = OFF`, so it runs
        // correctly here inside the transaction.
        validate_foreign_keys(&mut connection).await?;
        Ok(validation_report)
    }
    .await;

    match result {
        Ok(validation_report) => {
            sqlx::query("COMMIT").execute(&mut *connection).await?;
            sqlx::query("PRAGMA foreign_keys = ON")
                .execute(&mut *connection)
                .await?;
            Ok(validation_report)
        }
        Err(error) => {
            let primary = finish_restore_rollback(
                Err(error),
                sqlx::query("ROLLBACK")
                    .execute(&mut *connection)
                    .await
                    .map(|_| ()),
            );
            finish_foreign_key_restore(
                primary,
                sqlx::query("PRAGMA foreign_keys = ON")
                    .execute(&mut *connection)
                    .await
                    .map(|_| ()),
            )
        }
    }
}

async fn existing_export_tables(
    connection: &mut SqliteConnection,
) -> Result<Vec<String>, BackupError> {
    let rows = sqlx::query("SELECT name FROM sqlite_master WHERE type = 'table'")
        .fetch_all(&mut *connection)
        .await?;
    let names = rows
        .into_iter()
        .map(|row| {
            row.try_get::<CatalogTableName, _>("name")
                .map(CatalogTableName::into_inner)
        })
        .collect::<Result<Vec<_>, _>>()?;
    Ok(backup::table_set(names))
}

async fn import_table(
    connection: &mut SqliteConnection,
    source_path: &Path,
    table: &str,
    columns: &[ColumnInfo],
    validation_report: &mut RestoreValidationReport,
) -> Result<(), BackupError> {
    let rows = backup::read_table_rows(source_path, table)?;
    if rows.is_empty() {
        return Ok(());
    }

    let column_names = columns
        .iter()
        .filter(|column| rows[0].contains_key(&column.name))
        .map(|column| column.name.clone())
        .collect::<Vec<_>>();
    let insert = insert_sql(table, &column_names);

    for row in rows {
        backup::validate_restore_row(table, &row, validation_report);
        let mut query = sqlx::query(&insert);
        for column in &column_names {
            let value = row.get(column).ok_or_else(|| {
                BackupError::InvalidBackup(format!("table {table} row is missing column {column}"))
            })?;
            query = bind_restore_value(query, RestoreBindValue::from_json(value));
        }
        query
            .execute(&mut *connection)
            .await
            .map_err(map_restore_error)?;
    }

    Ok(())
}

fn insert_sql(table: &str, columns: &[String]) -> String {
    let column_list = columns
        .iter()
        .map(|column| sql::quote_identifier(column))
        .collect::<Vec<_>>()
        .join(", ");
    let placeholders = (1..=columns.len())
        .map(|index| format!("${index}"))
        .collect::<Vec<_>>()
        .join(", ");
    format!(
        "INSERT INTO {} ({column_list}) VALUES ({placeholders})",
        sql::quote_identifier(table)
    )
}

/// Binds the closed set of backup NDJSON cell roles for `SQLite` restore.
fn bind_restore_value<'q>(
    query: sqlx::query::Query<'q, sqlx::Sqlite, sqlx::sqlite::SqliteArguments<'q>>,
    value: RestoreBindValue,
) -> sqlx::query::Query<'q, sqlx::Sqlite, sqlx::sqlite::SqliteArguments<'q>> {
    match value {
        RestoreBindValue::Null => query.bind_storage(Option::<RestoreText>::None),
        RestoreBindValue::Boolean(value) => query.bind_storage(value),
        RestoreBindValue::Integer(value) => query.bind_storage(value),
        RestoreBindValue::Real { value, .. } => query.bind_storage(value),
        RestoreBindValue::Text(value) => query.bind_storage(value),
        RestoreBindValue::Json(value) => query.bind_storage(value),
    }
}

async fn validate_foreign_keys(connection: &mut SqliteConnection) -> Result<(), BackupError> {
    let rows = sqlx::query("PRAGMA foreign_key_check")
        .fetch_all(&mut *connection)
        .await?;
    if rows.is_empty() {
        Ok(())
    } else {
        Err(BackupError::ConstraintViolation(format!(
            "sqlite foreign_key_check returned {} violation(s)",
            rows.len()
        )))
    }
}

async fn columns(
    connection: &mut SqliteConnection,
    table: &CatalogTableName,
) -> Result<Vec<ColumnInfo>, BackupError> {
    let sql = format!(
        "PRAGMA table_info({})",
        sql::quote_identifier(table.as_str())
    );
    let rows = sqlx::query(&sql).fetch_all(&mut *connection).await?;
    rows.into_iter()
        .map(|row| {
            // `ColumnInfo` is a plain struct, so its field types police nothing — the
            // turbofish is what makes this decode visible to `sqlx-newtype-decode`. (The
            // `type` read below already carried one, and its `.to_ascii_lowercase()` puts
            // it outside field position anyway.)
            Ok(ColumnInfo {
                name: row.try_get::<CatalogColumnName, _>("name")?.into_inner(),
                type_name: row
                    .try_get::<CatalogTypeName, _>("type")?
                    .into_inner()
                    .to_ascii_lowercase(),
            })
        })
        .collect()
}

async fn export_table(
    connection: &mut SqliteConnection,
    destination_path: &Path,
    table: &str,
    columns: &[ColumnInfo],
) -> Result<(), BackupError> {
    let file = File::create(destination_path.join("db").join(format!("{table}.ndjson")))?;
    let mut writer = BufWriter::new(file);
    let select = json_select(table, columns);
    let mut rows = sqlx::query(&select).fetch(&mut *connection);

    while let Some(row) = rows.try_next().await? {
        let json: BackupRowJson = row.try_get(0)?;
        writer.write_all(json.as_bytes())?;
        writer.write_all(b"\n")?;
    }
    writer.flush()?;
    Ok(())
}

fn json_select(table: &str, columns: &[ColumnInfo]) -> String {
    let json_args = columns
        .iter()
        .map(|column| {
            let name = sql::quote_literal(&column.name);
            let value = if is_bool_column(column) {
                format!(
                    "CASE WHEN {column_name} IS NULL THEN NULL WHEN {column_name} THEN json('true') ELSE json('false') END",
                    column_name = sql::quote_identifier(&column.name)
                )
            } else {
                sql::quote_identifier(&column.name)
            };
            format!("{name}, {value}")
        })
        .collect::<Vec<_>>()
        .join(", ");
    format!(
        "SELECT json_object({json_args}) FROM {} ORDER BY {}",
        sql::quote_identifier(table),
        backup::order_by_clause(columns, sql::quote_identifier)
    )
}

fn is_bool_column(column: &ColumnInfo) -> bool {
    column.type_name.contains("bool")
}

async fn schema_version(connection: &mut SqliteConnection) -> Result<i64, BackupError> {
    Ok(sqlx::query_scalar::<_, Option<MigrationVersion>>(
        "SELECT MAX(version) FROM _sqlx_migrations",
    )
    .fetch_one(&mut *connection)
    .await?
    .map_or(0, MigrationVersion::into_i64))
}

async fn schema_checksum(connection: &mut SqliteConnection) -> Result<String, BackupError> {
    let rows = sqlx::query(
        "SELECT name, sql
         FROM sqlite_master
         WHERE type = 'table' AND name NOT LIKE 'sqlite_%' AND name <> '_sqlx_migrations'
         ORDER BY name",
    )
    .fetch_all(&mut *connection)
    .await?;
    let mut hasher = Sha256::new();
    for row in rows {
        let name = row.try_get::<CatalogTableName, _>("name")?.into_inner();
        let sql: CatalogDefinition = row.try_get("sql")?;
        hasher.update(name.as_bytes());
        hasher.update(b"\0");
        hasher.update(sql.as_bytes());
        hasher.update(b"\0");
    }
    Ok(format!("{:x}", hasher.finalize()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::{Backend, CloseablePool, sqlite_only};
    use rstest::*;
    use rstest_reuse::*;

    fn primary_io_error() -> BackupError {
        BackupError::Io(std::io::Error::new(
            std::io::ErrorKind::PermissionDenied,
            "primary backup failure",
        ))
    }

    fn assert_primary_io_error(result: Result<(), BackupError>) {
        assert!(matches!(
            result,
            Err(BackupError::Io(error))
                if error.kind() == std::io::ErrorKind::PermissionDenied
                    && error.to_string() == "primary backup failure"
        ));
    }

    #[test]
    fn map_restore_error_preserves_non_database_failures() {
        assert!(matches!(
            map_restore_error(sqlx::Error::RowNotFound),
            BackupError::Sqlx(sqlx::Error::RowNotFound)
        ));
    }

    #[test]
    fn continuation_reporting_secondary_failures_preserve_backup_primary_sources_and_report_once() {
        let (export, trace) = crate::helpers::swallowed_test::capture(|| {
            finish_export_rollback(Err(primary_io_error()), Err(sqlx::Error::PoolClosed))
        });
        assert!(matches!(
            export,
            Err(BackupError::Io(error))
                if error.kind() == std::io::ErrorKind::PermissionDenied
                    && error.to_string() == "primary backup failure"
        ));
        crate::helpers::swallowed_test::assert_one_report(
            &trace,
            "storage.sqlite.backup.export.rollback",
        );

        let (restore, trace) = crate::helpers::swallowed_test::capture(|| {
            finish_restore_rollback::<()>(Err(primary_io_error()), Err(sqlx::Error::PoolClosed))
        });
        assert_primary_io_error(restore);
        crate::helpers::swallowed_test::assert_one_report(
            &trace,
            "storage.sqlite.backup.restore.rollback",
        );

        let (restore, trace) = crate::helpers::swallowed_test::capture(|| {
            finish_foreign_key_restore(Err(primary_io_error()), Err(sqlx::Error::PoolClosed))
        });
        assert_primary_io_error(restore);
        crate::helpers::swallowed_test::assert_one_report(
            &trace,
            "storage.sqlite.backup.restore.foreign_keys_restore",
        );
    }

    // reason: drives the SQLite dialect's `export_database` directly with a
    // destination that lacks the `db/` subdir the public API always creates, so
    // the first `export_table` File::create fails mid-transaction and the export
    // ROLLBACK arm runs. The Postgres analog lives in postgres/backup.rs.
    #[apply(sqlite_only)]
    #[tokio::test]
    async fn export_database_rolls_back_on_write_failure(#[case] backend: Backend) {
        let env = backend.setup().await;
        let CloseablePool::Sqlite(pool) = env.base.pool() else {
            unreachable!("sqlite_only yields a SQLite pool")
        };

        // A fresh temp dir with no `db/` subdir: `export_table`'s
        // File::create(destination/db/<table>.ndjson) fails, forcing the
        // transaction into the ROLLBACK arm.
        let destination = tempfile::TempDir::new().expect("tempdir");

        let error = export_database(pool, destination.path(), BackupMode::Directory).await;
        assert!(
            error.is_err(),
            "export into a missing db/ dir must fail and roll back"
        );
    }

    #[test]
    fn json_select_marks_boolean_values_as_json_booleans() {
        let sql = json_select(
            "users",
            &[
                ColumnInfo {
                    name: "user_id".to_owned(),
                    type_name: "integer".to_owned(),
                },
                ColumnInfo {
                    name: "is_operator".to_owned(),
                    type_name: "boolean".to_owned(),
                },
            ],
        );

        assert!(sql.contains("json('true')"));
        assert!(sql.contains("ORDER BY \"user_id\""));
    }

    #[test]
    fn insert_sql_uses_numbered_placeholders() {
        let sql = insert_sql(
            "users",
            &[
                "user_id".to_owned(),
                "username".to_owned(),
                "is_operator".to_owned(),
            ],
        );

        assert_eq!(
            sql,
            "INSERT INTO \"users\" (\"user_id\", \"username\", \"is_operator\") VALUES ($1, $2, $3)"
        );
    }

    #[test]
    fn restore_bind_value_accepts_all_json_shapes() {
        for value in [
            serde_json::Value::Null,
            serde_json::json!(true),
            serde_json::json!(42),
            serde_json::json!(42_u64),
            serde_json::json!(3.5),
            serde_json::json!("text"),
            serde_json::json!(["a"]),
            serde_json::json!({"key": "value"}),
        ] {
            let query = sqlx::query("SELECT ?1");
            let _query = bind_restore_value(query, RestoreBindValue::from_json(&value));
        }
    }
}
