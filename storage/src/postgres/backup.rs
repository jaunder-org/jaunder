use std::{
    collections::HashSet,
    fs::File,
    io::{BufWriter, Write},
    path::Path,
};

use futures_util::TryStreamExt;
use sha2::{Digest, Sha256};
use sqlx::{PgConnection, PgPool, Row};

use crate::backup::{
    build_manifest, ensure_schema_version, json_value_as_restore_text, order_by_clause,
    read_table_rows, BackupError, BackupManifest, BackupMode, ColumnInfo, TABLES_IN_EXPORT_ORDER,
};
use crate::sql::quote_identifier;

/// Map a Postgres integrity-constraint violation (SQLSTATE class `23`, e.g. `23503`
/// `foreign_key_violation`) to `ConstraintViolation`, so a restore that violates the
/// schema fails uniformly with `SQLite` (which detects it via `foreign_key_check`).
/// Any other database error is a genuine infrastructure failure and passes through as
/// `Sqlx`.
fn map_restore_error(error: sqlx::Error) -> BackupError {
    let constraint_message = error
        .as_database_error()
        .filter(|db| db.code().is_some_and(|code| code.starts_with("23")))
        .map(|db| db.message().to_owned());
    match constraint_message {
        Some(message) => BackupError::ConstraintViolation(message),
        None => BackupError::Sqlx(error),
    }
}

pub(crate) async fn export_database(
    pool: &PgPool,
    destination_path: &Path,
    mode: BackupMode,
) -> Result<BackupManifest, BackupError> {
    let mut connection = pool.acquire().await?;
    // Snapshot the whole database at one instant: REPEATABLE READ gives every
    // table read below the same MVCC snapshot, so a concurrent writer cannot make
    // the multi-table export internally inconsistent. (SQLite achieves this with
    // BEGIN IMMEDIATE.)
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
        // Export rollback is unreachable through the public `export_backup`
        // (`export_directory_backup` always creates the `db/` subdir before the
        // dialect writes). It is exercised directly by pointing the dialect
        // `export_database` at a destination lacking `db/` (see tests). Parity
        // with sqlite/backup.rs.
        Err(error) => {
            let _ = sqlx::query("ROLLBACK").execute(&mut *connection).await;
            Err(error)
        }
    }
}

pub(crate) async fn restore_database(
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
        query
            .execute(&mut *connection)
            .await
            .map_err(map_restore_error)?;
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
    // `users.user_id` is GENERATED ALWAYS AS IDENTITY, so OVERRIDING SYSTEM VALUE
    // is required to restore the original ids. The other tables use serial-style
    // defaults that accept explicit ids without an override.
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

/// Advance each table's identity/serial sequence past the largest id just
/// imported. Restore inserts explicit ids, which does not move the sequence, so
/// without this the next INSERT would reuse an existing id and hit a unique
/// violation. The third `setval` arg (`COUNT(*) > 0`) leaves the sequence
/// "uncalled" for an empty table, so the first inserted row still gets id 1.
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::{postgres_only, Backend, CloseablePool};
    use rstest::*;
    use rstest_reuse::*;

    // reason: drives the Postgres dialect's `export_database` directly with a
    // destination that lacks the `db/` subdir the public API always creates, so
    // the first `export_table` File::create fails mid-transaction and the export
    // ROLLBACK arm runs. The SQLite analog lives in sqlite/backup.rs.
    #[apply(postgres_only)]
    #[tokio::test]
    async fn export_database_rolls_back_on_write_failure(#[case] backend: Backend) {
        let env = backend.setup().await;
        let CloseablePool::Postgres(pool) = env.base.pool() else {
            unreachable!("postgres_only yields a Postgres pool")
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

    // reason: drives the Postgres dialect's restore directly with a backup whose
    // users.ndjson carries a non-numeric `user_id`. `import_table`'s
    // `CAST($n AS BIGINT)` then raises SQLSTATE 22P02 (class 22, a data exception —
    // not a class-23 constraint violation), so `map_restore_error` takes its `None`
    // arm and yields `BackupError::Sqlx`. That arm is unreachable through the public
    // restore interface, so it is exercised at the dialect level here.
    #[apply(postgres_only)]
    #[tokio::test]
    async fn restore_maps_non_constraint_error_to_sqlx(
        #[case] backend: Backend,
    ) -> Result<(), BackupError> {
        let source = backend.setup().await;
        let CloseablePool::Postgres(source_pool) = source.base.pool() else {
            unreachable!("postgres_only yields a Postgres pool")
        };
        source
            .state
            .users
            .create_user(
                &"userone".parse().expect("valid username"),
                &"password123".parse().expect("valid password"),
                None,
                false,
            )
            .await
            .expect("seed user");

        // Export a real backup so its manifest's schema version/checksum match the
        // fresh target, and users.ndjson has a complete row to corrupt.
        let temp = tempfile::TempDir::new().expect("tempdir");
        let backup = temp.path().join("backup");
        std::fs::create_dir_all(backup.join("db"))?;
        let manifest = export_database(source_pool, &backup, BackupMode::Directory).await?;

        // Replace the integer `user_id` with a non-numeric string. Row 0 still
        // carries every column, so `column_names` includes `user_id`; the bind then
        // trips `CAST('abc' AS BIGINT)` -> SQLSTATE 22P02 during the INSERT.
        let users_ndjson = backup.join("db").join("users.ndjson");
        let mut rows: Vec<serde_json::Map<String, serde_json::Value>> =
            std::fs::read_to_string(&users_ndjson)?
                .lines()
                .filter(|line| !line.trim().is_empty())
                .map(serde_json::from_str)
                .collect::<Result<_, _>>()?;
        assert!(!rows.is_empty(), "expected an exported user row");
        rows[0].insert("user_id".to_owned(), serde_json::json!("abc"));
        let mut corrupted = String::new();
        for row in &rows {
            corrupted.push_str(&serde_json::to_string(row)?);
            corrupted.push('\n');
        }
        std::fs::write(&users_ndjson, corrupted)?;

        let target = backend.setup().await;
        let CloseablePool::Postgres(target_pool) = target.base.pool() else {
            unreachable!("postgres_only yields a Postgres pool")
        };
        let error = restore_database(target_pool, &backup, &manifest)
            .await
            .expect_err("restore should fail casting a non-numeric user_id");

        assert!(
            matches!(error, BackupError::Sqlx(_)),
            "non-constraint (class 22) restore error must map to Sqlx, got {error:?}"
        );
        Ok(())
    }

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
}
