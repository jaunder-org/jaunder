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
    build_manifest, order_by_clause, BackupError, BackupManifest, BackupMode, ColumnInfo,
    TABLES_IN_EXPORT_ORDER,
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
}
