use std::{
    collections::HashSet,
    fs::File,
    io::{BufWriter, Write},
    path::Path,
};

use futures_util::TryStreamExt;
use sha2::{Digest, Sha256};
use sqlx::{Row, SqliteConnection, SqlitePool};

use super::{
    build_manifest, ensure_schema_version, order_by_clause, read_table_rows, BackupError,
    BackupManifest, BackupMode, ColumnInfo, TABLES_IN_EXPORT_ORDER,
};

pub(super) async fn export_database(
    pool: &SqlitePool,
    destination_path: &Path,
    mode: BackupMode,
) -> Result<BackupManifest, BackupError> {
    let mut connection = pool.acquire().await?;
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
        Err(error) => {
            let _ = sqlx::query("ROLLBACK").execute(&mut *connection).await;
            Err(error)
        }
    }
}

pub(super) async fn restore_database(
    pool: &SqlitePool,
    source_path: &Path,
    manifest: &BackupManifest,
) -> Result<(), BackupError> {
    let mut connection = pool.acquire().await?;
    let schema_version = schema_version(&mut connection).await?;
    ensure_schema_version(manifest, schema_version)?;

    sqlx::query("PRAGMA foreign_keys = OFF")
        .execute(&mut *connection)
        .await?;
    sqlx::query("BEGIN IMMEDIATE")
        .execute(&mut *connection)
        .await?;

    let result = async {
        for table in &manifest.tables {
            let columns = columns(&mut connection, table).await?;
            import_table(&mut connection, source_path, table, &columns).await?;
        }
        Ok::<(), BackupError>(())
    }
    .await;

    match result {
        Ok(()) => {
            sqlx::query("COMMIT").execute(&mut *connection).await?;
            sqlx::query("PRAGMA foreign_keys = ON")
                .execute(&mut *connection)
                .await?;
            validate_foreign_keys(&mut connection).await
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
    let existing = rows
        .into_iter()
        .map(|row| row.try_get::<String, _>("name"))
        .collect::<Result<HashSet<_>, _>>()?;
    Ok(TABLES_IN_EXPORT_ORDER
        .iter()
        .filter(|table| existing.contains(**table))
        .map(|table| (*table).to_owned())
        .collect())
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
        .map(|index| format!("?{index}"))
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
            if let Some(value) = value.as_i64() {
                query.bind(value)
            } else if let Some(value) = value.as_u64().and_then(|value| i64::try_from(value).ok()) {
                query.bind(value)
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
        order_by_clause(table, quote_identifier)
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

fn quote_identifier(identifier: &str) -> String {
    format!("\"{}\"", identifier.replace('"', "\"\""))
}

fn quote_literal(value: &str) -> String {
    format!("'{}'", value.replace('\'', "''"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use sqlx::Connection;

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
    fn quoting_escapes_identifiers_and_literals() {
        assert_eq!(quote_identifier("a\"b"), "\"a\"\"b\"");
        assert_eq!(quote_literal("a'b"), "'a''b'");
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
            "INSERT INTO \"users\" (\"user_id\", \"username\", \"is_operator\") VALUES (?1, ?2, ?3)"
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

    #[tokio::test]
    async fn validate_foreign_keys_reports_violations() -> Result<(), BackupError> {
        let mut connection = sqlx::SqliteConnection::connect("sqlite::memory:").await?;
        sqlx::query("PRAGMA foreign_keys = OFF")
            .execute(&mut connection)
            .await?;
        sqlx::query("CREATE TABLE parent (id INTEGER PRIMARY KEY)")
            .execute(&mut connection)
            .await?;
        sqlx::query(
            "CREATE TABLE child (id INTEGER PRIMARY KEY, parent_id INTEGER REFERENCES parent(id))",
        )
        .execute(&mut connection)
        .await?;
        sqlx::query("INSERT INTO child (id, parent_id) VALUES (1, 999)")
            .execute(&mut connection)
            .await?;

        let error = validate_foreign_keys(&mut connection)
            .await
            .expect_err("foreign key violation");

        assert!(matches!(error, BackupError::ConstraintViolation(_)));
        Ok(())
    }

    #[tokio::test]
    async fn schema_version_returns_migration_count() -> Result<(), BackupError> {
        let mut connection = sqlx::SqliteConnection::connect("sqlite::memory:").await?;
        sqlx::migrate!("./migrations/sqlite")
            .run(&mut connection)
            .await
            .map_err(|e| BackupError::Io(std::io::Error::other(e.to_string())))?;
        let version = schema_version(&mut connection).await?;
        assert_eq!(version, 13, "expected one entry per migration file");
        Ok(())
    }

    #[tokio::test]
    async fn schema_checksum_returns_nonempty_hex_string() -> Result<(), BackupError> {
        let mut connection = sqlx::SqliteConnection::connect("sqlite::memory:").await?;
        sqlx::migrate!("./migrations/sqlite")
            .run(&mut connection)
            .await
            .map_err(|e| BackupError::Io(std::io::Error::other(e.to_string())))?;
        let checksum = schema_checksum(&mut connection).await?;
        assert_eq!(checksum.len(), 64, "SHA-256 hex string must be 64 chars");
        assert!(
            checksum.chars().all(|c| c.is_ascii_hexdigit()),
            "checksum must be lowercase hex"
        );
        Ok(())
    }
}
