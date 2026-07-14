use std::{
    fs::File,
    io::{BufWriter, Write},
    path::Path,
};

use futures_util::TryStreamExt;
use sha2::{Digest, Sha256};
use sqlx::{Row, SqliteConnection, SqlitePool};

use crate::backup::{
    backup_table_set, build_manifest, ensure_schema_version, order_by_clause, read_table_rows,
    BackupError, BackupManifest, BackupMode, ColumnInfo,
};
use crate::sql::{quote_identifier, quote_literal};

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
            let columns = columns(&mut connection, table).await?;
            export_table(&mut connection, destination_path, table, &columns).await?;
        }

        Ok(build_manifest(
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
        Err(error) => {
            let _ = sqlx::query("ROLLBACK").execute(&mut *connection).await;
            Err(error)
        }
    }
}

pub(crate) async fn restore_database(
    pool: &SqlitePool,
    source_path: &Path,
    manifest: &BackupManifest,
) -> Result<(), BackupError> {
    let mut connection = pool.acquire().await?;
    let schema_version = schema_version(&mut connection).await?;
    ensure_schema_version(manifest, schema_version)?;

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
        // Clear every table before loading any (authoritative replace), keeping the
        // two backends' restore shape identical. FK enforcement is off here, so a
        // DELETE never cascades; the clear-then-load split matches Postgres, where
        // deferral does not suppress cascade actions.
        for table in &manifest.tables {
            sqlx::query(&format!("DELETE FROM {}", quote_identifier(table)))
                .execute(&mut *connection)
                .await?;
        }
        for table in &manifest.tables {
            let columns = columns(&mut connection, table).await?;
            import_table(&mut connection, source_path, table, &columns).await?;
        }
        // Validate FKs *before* committing so a violation rolls the whole restore
        // back rather than leaving invalid data committed. `foreign_key_check`
        // scans for violations and works with `foreign_keys = OFF`, so it runs
        // correctly here inside the transaction.
        validate_foreign_keys(&mut connection).await
    }
    .await;

    match result {
        Ok(()) => {
            sqlx::query("COMMIT").execute(&mut *connection).await?;
            sqlx::query("PRAGMA foreign_keys = ON")
                .execute(&mut *connection)
                .await?;
            Ok(())
        }
        Err(error) => {
            let _ = sqlx::query("ROLLBACK").execute(&mut *connection).await;
            let _ = sqlx::query("PRAGMA foreign_keys = ON")
                .execute(&mut *connection)
                .await;
            Err(error)
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
        .map(|row| row.try_get::<String, _>("name"))
        .collect::<Result<Vec<_>, _>>()?;
    Ok(backup_table_set(names))
}

async fn import_table(
    connection: &mut SqliteConnection,
    source_path: &Path,
    table: &str,
    columns: &[ColumnInfo],
) -> Result<(), BackupError> {
    let rows = read_table_rows(source_path, table)?;
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
        let mut query = sqlx::query(&insert);
        for column in &column_names {
            let value = row.get(column).ok_or_else(|| {
                BackupError::InvalidBackup(format!("table {table} row is missing column {column}"))
            })?;
            query = bind_json_value(query, value);
        }
        query.execute(&mut *connection).await?;
    }

    Ok(())
}

fn insert_sql(table: &str, columns: &[String]) -> String {
    let column_list = columns
        .iter()
        .map(|column| quote_identifier(column))
        .collect::<Vec<_>>()
        .join(", ");
    let placeholders = (1..=columns.len())
        .map(|index| format!("${index}"))
        .collect::<Vec<_>>()
        .join(", ");
    format!(
        "INSERT INTO {} ({column_list}) VALUES ({placeholders})",
        quote_identifier(table)
    )
}

fn bind_json_value<'q>(
    query: sqlx::query::Query<'q, sqlx::Sqlite, sqlx::sqlite::SqliteArguments<'q>>,
    value: &serde_json::Value,
) -> sqlx::query::Query<'q, sqlx::Sqlite, sqlx::sqlite::SqliteArguments<'q>> {
    match value {
        serde_json::Value::Null => query.bind(Option::<String>::None),
        serde_json::Value::Bool(value) => query.bind(*value),
        serde_json::Value::Number(value) => {
            // Preserve integer affinity: bind as i64 (including u64 that fits) so
            // large ids round-trip exactly, falling back to f64 only for
            // genuinely non-integral numbers.
            if let Some(value) = value.as_i64() {
                query.bind(value)
            } else if value.as_u64().and_then(|v| i64::try_from(v).ok()).is_some() {
                unreachable!("as_i64 already claims every u64 that fits in i64")
            } else {
                query.bind(value.as_f64())
            }
        }
        serde_json::Value::String(value) => query.bind(value.clone()),
        serde_json::Value::Array(_) | serde_json::Value::Object(_) => query.bind(value.to_string()),
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
    table: &str,
) -> Result<Vec<ColumnInfo>, BackupError> {
    let sql = format!("PRAGMA table_info({})", quote_identifier(table));
    let rows = sqlx::query(&sql).fetch_all(&mut *connection).await?;
    rows.into_iter()
        .map(|row| {
            Ok(ColumnInfo {
                name: row.try_get("name")?,
                type_name: row.try_get::<String, _>("type")?.to_ascii_lowercase(),
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
        let json: String = row.try_get(0)?;
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
            let name = quote_literal(&column.name);
            let value = if is_bool_column(column) {
                format!(
                    "CASE WHEN {column_name} IS NULL THEN NULL WHEN {column_name} THEN json('true') ELSE json('false') END",
                    column_name = quote_identifier(&column.name)
                )
            } else {
                quote_identifier(&column.name)
            };
            format!("{name}, {value}")
        })
        .collect::<Vec<_>>()
        .join(", ");
    format!(
        "SELECT json_object({json_args}) FROM {} ORDER BY {}",
        quote_identifier(table),
        order_by_clause(columns, quote_identifier)
    )
}

fn is_bool_column(column: &ColumnInfo) -> bool {
    column.type_name.contains("bool")
}

async fn schema_version(connection: &mut SqliteConnection) -> Result<i64, BackupError> {
    Ok(
        sqlx::query_scalar::<_, Option<i64>>("SELECT MAX(version) FROM _sqlx_migrations")
            .fetch_one(&mut *connection)
            .await?
            .unwrap_or_default(),
    )
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
        let name: String = row.try_get("name")?;
        let sql: String = row.try_get("sql")?;
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
    use crate::test_support::{sqlite_only, Backend, CloseablePool};
    use rstest::*;
    use rstest_reuse::*;

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
    fn bind_json_value_accepts_all_json_shapes() {
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
            let _query = bind_json_value(query, &value);
        }
    }
}
