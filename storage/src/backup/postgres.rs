use std::{
    collections::HashSet,
    fs::File,
    io::{BufWriter, Write},
    path::Path,
};

use futures_util::TryStreamExt;
use sha2::{Digest, Sha256};
use sqlx::{PgConnection, PgPool, Row};

use super::{
    build_manifest, ensure_schema_version, json_value_as_restore_text, order_by_clause,
    read_table_rows, BackupError, BackupManifest, BackupMode, ColumnInfo, TABLES_IN_EXPORT_ORDER,
};

pub(super) async fn export_database(
    pool: &PgPool,
    destination_path: &Path,
    mode: BackupMode,
) -> Result<BackupManifest, BackupError> {
    let mut connection = pool.acquire().await?;
    sqlx::query("BEGIN ISOLATION LEVEL REPEATABLE READ READ ONLY")
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
    pool: &PgPool,
    source_path: &Path,
    manifest: &BackupManifest,
) -> Result<(), BackupError> {
    let mut connection = pool.acquire().await?;
    let schema_version = schema_version(&mut connection).await?;
    ensure_schema_version(manifest, schema_version)?;

    sqlx::query("BEGIN").execute(&mut *connection).await?;
    let result = async {
        for table in &manifest.tables {
            let columns = columns(&mut connection, table).await?;
            import_table(&mut connection, source_path, table, &columns).await?;
        }
        repair_sequences(&mut connection).await
    }
    .await;

    match result {
        Ok(()) => {
            sqlx::query("COMMIT").execute(&mut *connection).await?;
            Ok(())
        }
        Err(error) => {
            let _ = sqlx::query("ROLLBACK").execute(&mut *connection).await;
            Err(error)
        }
    }
}

async fn existing_export_tables(connection: &mut PgConnection) -> Result<Vec<String>, BackupError> {
    let rows = sqlx::query(
        "SELECT table_name
         FROM information_schema.tables
         WHERE table_schema = 'public' AND table_type = 'BASE TABLE'",
    )
    .fetch_all(&mut *connection)
    .await?;
    let existing = rows
        .into_iter()
        .map(|row| row.try_get::<String, _>("table_name"))
        .collect::<Result<HashSet<_>, _>>()?;
    Ok(TABLES_IN_EXPORT_ORDER
        .iter()
        .filter(|table| existing.contains(**table))
        .map(|table| (*table).to_owned())
        .collect())
}

async fn import_table(
    connection: &mut PgConnection,
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
        .cloned()
        .collect::<Vec<_>>();
    let insert = insert_sql(table, &column_names);

    for row in rows {
        let mut query = sqlx::query(&insert);
        for column in &column_names {
            let value = row.get(&column.name).ok_or_else(|| {
                BackupError::InvalidBackup(format!(
                    "table {table} row is missing column {}",
                    column.name
                ))
            })?;
            query = query.bind(json_value_as_restore_text(value));
        }
        query.execute(&mut *connection).await?;
    }

    Ok(())
}

fn insert_sql(table: &str, columns: &[ColumnInfo]) -> String {
    let column_list = columns
        .iter()
        .map(|column| quote_identifier(&column.name))
        .collect::<Vec<_>>()
        .join(", ");
    let placeholders = columns
        .iter()
        .enumerate()
        .map(|(index, column)| format!("CAST(${} AS {})", index + 1, restore_type(column)))
        .collect::<Vec<_>>()
        .join(", ");
    if table == "users" {
        format!(
            "INSERT INTO {} ({column_list}) OVERRIDING SYSTEM VALUE VALUES ({placeholders})",
            quote_identifier(table)
        )
    } else {
        format!(
            "INSERT INTO {} ({column_list}) VALUES ({placeholders})",
            quote_identifier(table)
        )
    }
}

fn restore_type(column: &ColumnInfo) -> &'static str {
    match column.type_name.as_str() {
        "bool" => "BOOLEAN",
        "int8" => "BIGINT",
        "timestamptz" => "TIMESTAMPTZ",
        _ => "TEXT",
    }
}

async fn repair_sequences(connection: &mut PgConnection) -> Result<(), BackupError> {
    for (table, column) in [
        ("users", "user_id"),
        ("posts", "post_id"),
        ("post_revisions", "revision_id"),
        ("tags", "tag_id"),
    ] {
        let sql = format!(
            "SELECT setval(
                pg_get_serial_sequence('{table}', '{column}'),
                COALESCE((SELECT MAX({column}) FROM {table}), 1),
                (SELECT COUNT(*) > 0 FROM {table})
            )"
        );
        sqlx::query(&sql).execute(&mut *connection).await?;
    }
    Ok(())
}

async fn columns(
    connection: &mut PgConnection,
    table: &str,
) -> Result<Vec<ColumnInfo>, BackupError> {
    let rows = sqlx::query(
        "SELECT column_name, udt_name
         FROM information_schema.columns
         WHERE table_schema = 'public' AND table_name = $1
         ORDER BY ordinal_position",
    )
    .bind(table)
    .fetch_all(&mut *connection)
    .await?;
    rows.into_iter()
        .map(|row| {
            Ok(ColumnInfo {
                name: row.try_get("column_name")?,
                type_name: row.try_get("udt_name")?,
            })
        })
        .collect()
}

async fn export_table(
    connection: &mut PgConnection,
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
    let column_list = columns
        .iter()
        .map(|column| quote_identifier(&column.name))
        .collect::<Vec<_>>()
        .join(", ");
    format!(
        "SELECT to_jsonb(export_row)::text FROM (SELECT {column_list} FROM {} ORDER BY {}) AS export_row",
        quote_identifier(table),
        order_by_clause(table, quote_identifier)
    )
}

async fn schema_version(connection: &mut PgConnection) -> Result<i64, BackupError> {
    Ok(
        sqlx::query_scalar::<_, Option<i64>>("SELECT MAX(version) FROM _sqlx_migrations")
            .fetch_one(&mut *connection)
            .await?
            .unwrap_or_default(),
    )
}

async fn schema_checksum(connection: &mut PgConnection) -> Result<String, BackupError> {
    let rows = sqlx::query(
        "SELECT table_name, column_name, udt_name, is_nullable, ordinal_position
         FROM information_schema.columns
         WHERE table_schema = 'public' AND table_name <> '_sqlx_migrations'
         ORDER BY table_name, ordinal_position",
    )
    .fetch_all(&mut *connection)
    .await?;
    let mut hasher = Sha256::new();
    for row in rows {
        let table_name: String = row.try_get("table_name")?;
        let column_name: String = row.try_get("column_name")?;
        let type_name: String = row.try_get("udt_name")?;
        let is_nullable: String = row.try_get("is_nullable")?;
        hasher.update(table_name.as_bytes());
        hasher.update(b"\0");
        hasher.update(column_name.as_bytes());
        hasher.update(b"\0");
        hasher.update(type_name.as_bytes());
        hasher.update(b"\0");
        hasher.update(is_nullable.as_bytes());
        hasher.update(b"\0");
    }
    Ok(format!("{:x}", hasher.finalize()))
}

fn quote_identifier(identifier: &str) -> String {
    format!("\"{}\"", identifier.replace('"', "\"\""))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn json_select_orders_by_table_key() {
        let sql = json_select(
            "post_tags",
            &[
                ColumnInfo {
                    name: "post_id".to_owned(),
                    type_name: "int8".to_owned(),
                },
                ColumnInfo {
                    name: "tag_id".to_owned(),
                    type_name: "int8".to_owned(),
                },
            ],
        );

        assert!(sql.contains("to_jsonb(export_row)::text"));
        assert!(sql.contains("ORDER BY \"post_id\", \"tag_id\""));
    }

    #[test]
    fn quote_identifier_escapes_double_quotes() {
        assert_eq!(quote_identifier("a\"b"), "\"a\"\"b\"");
    }
}
