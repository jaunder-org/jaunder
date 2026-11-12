//! Offline, identity-first conversion of the explicitly selected `SQLite` cells.
//!
//! This operation runs only inside the queue-row transaction, before admission
//! is installed. It never rewrites semantic Post state or creates Revisions.

use common::time::UtcInstant;
use sqlx::{Row, SqliteConnection};

use super::types::{LegacyInstantRowId, LegacyInstantTupleId};
use crate::backup::{CatalogConstraintValidated, CatalogDefinition};
use crate::sql::{QueryBuilderStorageExt, RowCount};

/// Fetch raw rows first: a broken timestamp must never obscure its row identity.
async fn read_legacy_instant_cells(
    conn: &mut SqliteConnection,
    table: &str,
    columns: &[(&str, bool)],
) -> sqlx::Result<Vec<(LegacyInstantRowId, Vec<Option<UtcInstant>>)>> {
    let names = columns
        .iter()
        .map(|(name, _)| *name)
        .collect::<Vec<_>>()
        .join(", ");
    let sql = format!("SELECT rowid, {names} FROM {table} ORDER BY rowid");
    let rows = sqlx::query(sqlx::AssertSqlSafe(sql))
        .fetch_all(conn)
        .await?;
    rows.into_iter()
        .map(|row| {
            let id: LegacyInstantRowId = row.try_get(0)?;
            let mut cells = Vec::with_capacity(columns.len());
            for (index, (column, nullable)) in columns.iter().enumerate() {
                let raw = row.try_get_raw(index + 1)?;
                let class =
                    sqlx::TypeInfo::name(sqlx::ValueRef::type_info(&raw).as_ref()).to_owned();
                let result = if sqlx::ValueRef::is_null(&raw) && *nullable {
                    Ok(None)
                } else if sqlx::ValueRef::is_null(&raw)
                    || matches!(class.as_str(), "TEXT" | "INTEGER" | "REAL")
                {
                    // Public decode retains the established civil-UTC / Julian-day
                    // interpretation. Identity has already been obtained above.
                    row.try_get_unchecked::<UtcInstant, _>(index + 1).map(Some)
                } else {
                    Err(sqlx::Error::Protocol(format!(
                        "unsupported storage class {class}"
                    )))
                };
                cells.push(result.map_err(|error| {
                    sqlx::Error::Protocol(format!(
                        "legacy instant {table}.{column} rowid={id}: {error}"
                    ))
                })?);
            }
            Ok((id, cells))
        })
        .collect()
}

async fn normalize_table(
    conn: &mut SqliteConnection,
    table: &str,
    columns: &[(&str, bool)],
) -> sqlx::Result<()> {
    for (id, cells) in read_legacy_instant_cells(conn, table, columns).await? {
        let mut query = sqlx::QueryBuilder::<sqlx::Sqlite>::new(format!("UPDATE {table} SET "));
        for (index, ((column, _), instant)) in columns.iter().zip(cells).enumerate() {
            if index != 0 {
                query.push(", ");
            }
            query.push(*column).push(" = ").push_storage_bind(instant);
        }
        query.push(" WHERE rowid = ").push_storage_bind(id);
        query.build().execute(&mut *conn).await?;
    }
    Ok(())
}

/// These are deliberately explicit application tables, not schema discovery.
/// All selected columns of a row are updated together, including nullable ones.
pub(super) async fn normalize_sqlite(conn: &mut SqliteConnection) -> sqlx::Result<()> {
    check_sqlite_enforcement(conn, true).await?;
    normalize_table(
        conn,
        "posts",
        &[
            ("created_at", false),
            ("updated_at", false),
            ("published_at", true),
            ("deleted_at", true),
        ],
    )
    .await?;
    normalize_table(
        conn,
        "post_revisions",
        &[
            ("created_at", false),
            ("updated_at", false),
            ("published_at", true),
            ("deleted_at", true),
            ("captured_at", false),
        ],
    )
    .await?;
    normalize_table(
        conn,
        "sessions",
        &[("created_at", false), ("last_used_at", false)],
    )
    .await?;
    normalize_table(
        conn,
        "invites",
        &[
            ("created_at", false),
            ("expires_at", false),
            ("used_at", true),
        ],
    )
    .await?;
    normalize_table(
        conn,
        "email_verifications",
        &[
            ("created_at", false),
            ("expires_at", false),
            ("used_at", true),
        ],
    )
    .await?;
    normalize_table(
        conn,
        "password_resets",
        &[
            ("created_at", false),
            ("expires_at", false),
            ("used_at", true),
        ],
    )
    .await?;
    normalize_table(conn, "media", &[("created_at", false)]).await?;
    normalize_table(conn, "idempotency_keys", &[("created_at", false)]).await?;
    normalize_table(
        conn,
        "feed_events",
        &[
            ("next_attempt_at", false),
            ("claimed_at", true),
            ("terminal_at", true),
            ("created_at", false),
            ("regenerated_at", true),
            ("pinged_at", true),
        ],
    )
    .await?;
    normalize_table(
        conn,
        "feed_cache",
        &[
            ("representation_modified_at", false),
            ("generated_at", false),
        ],
    )
    .await?;
    normalize_sqlite_passkeys(conn).await?;
    // No enforcement DDL precedes verification of the entire population.
    sqlite_enforcement(conn, SqliteEnforcement::Verify).await?;
    sqlite_enforcement(conn, SqliteEnforcement::Install).await?;
    Ok(())
}

/// Keep the credential/ceremony group explicit and in its original queue transaction.
async fn normalize_sqlite_passkeys(conn: &mut SqliteConnection) -> sqlx::Result<()> {
    normalize_table(
        conn,
        "passkey_credentials",
        &[("created_at", false), ("last_used_at", true)],
    )
    .await?;
    normalize_table(
        conn,
        "passkey_registration_ceremonies",
        &[("expires_at", false), ("claimed_at", true)],
    )
    .await?;
    normalize_table(
        conn,
        "passkey_authentication_ceremonies",
        &[("expires_at", false), ("claimed_at", true)],
    )
    .await?;
    Ok(())
}

#[derive(Clone, Copy)]
enum SqliteEnforcement {
    Check,
    Verify,
    Install,
}

fn sqlite_predicate(column: &str, nullable: bool) -> String {
    let c = column;
    let d = "[0-9]";
    let shape =
        format!("{d}{d}{d}{d}-{d}{d}-{d}{d}T{d}{d}:{d}{d}:{d}{d}.{d}{d}{d}{d}{d}{d}{d}{d}{d}Z");
    let year = format!("CAST(substr({c},1,4) AS INTEGER)");
    let valid = format!(
        "typeof({c}) = 'text' AND length(CAST({c} AS BLOB)) = 30 AND {c} GLOB '{shape}'
        AND substr({c},6,2) BETWEEN '01' AND '12'
        AND CAST(substr({c},9,2) AS INTEGER) BETWEEN 1 AND CASE substr({c},6,2)
          WHEN '02' THEN 28 + ({year} % 4 = 0 AND ({year} % 100 != 0 OR {year} % 400 = 0))
          WHEN '04' THEN 30 WHEN '06' THEN 30 WHEN '09' THEN 30 WHEN '11' THEN 30 ELSE 31 END
        AND substr({c},12,2) BETWEEN '00' AND '23'
        AND substr({c},15,2) BETWEEN '00' AND '59'
        AND substr({c},18,2) BETWEEN '00' AND '59'
        AND {c} COLLATE BINARY >= '0001-01-01T00:00:00.000000000Z'
        AND {c} COLLATE BINARY <= '9999-12-30T22:00:00.999999999Z'"
    );
    if nullable {
        format!("({c} IS NULL OR COALESCE(({valid}), 0))")
    } else {
        format!("COALESCE(({valid}), 0)")
    }
}

async fn sqlite_table_enforcement(
    conn: &mut SqliteConnection,
    table: &str,
    columns: &[(&str, bool)],
    mode: SqliteEnforcement,
) -> sqlx::Result<u64> {
    if matches!(mode, SqliteEnforcement::Verify) {
        let predicate = columns
            .iter()
            .map(|(c, n)| sqlite_predicate(c, *n))
            .collect::<Vec<_>>()
            .join(" AND ");
        let sql = format!("SELECT rowid FROM {table} WHERE NOT ({predicate}) LIMIT 1");
        if let Some((id,)) = sqlx::query_as::<_, (LegacyInstantRowId,)>(sqlx::AssertSqlSafe(sql))
            .fetch_optional(&mut *conn)
            .await?
        {
            return Err(sqlx::Error::Protocol(format!(
                "noncanonical comparable instant in {table} rowid={id}"
            )));
        }
        return Ok(0);
    }
    let predicate = columns
        .iter()
        .map(|(c, n)| sqlite_predicate(&format!("NEW.{c}"), *n))
        .collect::<Vec<_>>()
        .join(" AND ");
    let mut present = 0;
    for (suffix, event) in [("insert", "INSERT"), ("update", "UPDATE")] {
        let name = format!("canonical_instants_v1_{table}_{suffix}");
        let sql = format!(
            "CREATE TRIGGER {name} BEFORE {event} ON {table} WHEN NOT ({predicate}) BEGIN SELECT RAISE(ABORT, 'noncanonical comparable instant'); END"
        );
        // Names and definitions are application-owned literals, not data.
        let lookup =
            format!("SELECT sql FROM sqlite_master WHERE type = 'trigger' AND name = '{name}'");
        let existing: Option<(CatalogDefinition,)> = sqlx::query_as(sqlx::AssertSqlSafe(lookup))
            .fetch_optional(&mut *conn)
            .await?;
        if let Some((existing,)) = existing {
            if existing.as_str() != sql {
                return Err(sqlx::Error::Protocol(format!(
                    "different comparable-instant trigger: {name}"
                )));
            }
            present += 1;
        } else if matches!(mode, SqliteEnforcement::Install) {
            sqlx::query(sqlx::AssertSqlSafe(sql))
                .execute(&mut *conn)
                .await
                .map_err(|error| {
                    sqlx::Error::Protocol(format!(
                        "installing comparable-instant trigger {name}: {error}"
                    ))
                })?;
        }
    }
    Ok(present)
}

async fn sqlite_enforcement(
    conn: &mut SqliteConnection,
    mode: SqliteEnforcement,
) -> sqlx::Result<u64> {
    let mut present = 0;
    macro_rules! table {
        ($name:literal, $($column:literal => $nullable:literal),+ $(,)?) => {
            present += sqlite_table_enforcement(conn, $name, &[$(($column, $nullable)),+], mode).await?;
        };
    }
    table!("posts", "created_at" => false, "updated_at" => false, "published_at" => true, "deleted_at" => true);
    table!("post_revisions", "created_at" => false, "updated_at" => false, "published_at" => true, "deleted_at" => true, "captured_at" => false);
    table!("sessions", "created_at" => false, "last_used_at" => false);
    table!("invites", "created_at" => false, "expires_at" => false, "used_at" => true);
    table!("email_verifications", "created_at" => false, "expires_at" => false, "used_at" => true);
    table!("password_resets", "created_at" => false, "expires_at" => false, "used_at" => true);
    table!("media", "created_at" => false);
    table!("idempotency_keys", "created_at" => false);
    table!("feed_events", "next_attempt_at" => false, "claimed_at" => true, "terminal_at" => true, "created_at" => false, "regenerated_at" => true, "pinged_at" => true);
    table!("feed_cache", "representation_modified_at" => false, "generated_at" => false);
    table!("passkey_credentials", "created_at" => false, "last_used_at" => true);
    table!("passkey_registration_ceremonies", "expires_at" => false, "claimed_at" => true);
    table!("passkey_authentication_ceremonies", "expires_at" => false, "claimed_at" => true);
    Ok(present)
}

/// Native PG cells keep their existing microsecond precision. Validate every
/// selected table before installing any finite-bound constraint.
pub(super) async fn normalize_postgres(conn: &mut sqlx::PgConnection) -> sqlx::Result<()> {
    check_postgres_enforcement(conn, true).await?;
    postgres_enforcement(conn, PostgresEnforcement::Verify).await?;
    postgres_enforcement(conn, PostgresEnforcement::Install).await?;
    Ok(())
}

#[derive(Clone, Copy)]
enum PostgresEnforcement {
    Check,
    Verify,
    Install,
}

async fn postgres_enforcement(
    conn: &mut sqlx::PgConnection,
    mode: PostgresEnforcement,
) -> sqlx::Result<u64> {
    let mut present = 0;
    macro_rules! table {
            ($table:literal, $($column:literal),+ $(,)?) => {
                present += postgres_table_bounds(conn, $table, &[$($column),+], mode).await?;
            };
        }
    table!(
        "posts",
        "created_at",
        "updated_at",
        "published_at",
        "deleted_at"
    );
    table!(
        "post_revisions",
        "created_at",
        "updated_at",
        "published_at",
        "deleted_at",
        "captured_at"
    );
    table!("sessions", "created_at", "last_used_at");
    table!("invites", "created_at", "expires_at", "used_at");
    table!("email_verifications", "created_at", "expires_at", "used_at");
    table!("password_resets", "created_at", "expires_at", "used_at");
    table!("media", "created_at");
    table!("idempotency_keys", "created_at");
    table!(
        "feed_events",
        "next_attempt_at",
        "claimed_at",
        "terminal_at",
        "created_at",
        "regenerated_at",
        "pinged_at"
    );
    table!("feed_cache", "representation_modified_at", "generated_at");
    table!("passkey_credentials", "created_at", "last_used_at");
    table!(
        "passkey_registration_ceremonies",
        "expires_at",
        "claimed_at"
    );
    table!(
        "passkey_authentication_ceremonies",
        "expires_at",
        "claimed_at"
    );
    Ok(present)
}

async fn postgres_table_bounds(
    conn: &mut sqlx::PgConnection,
    table: &str,
    columns: &[&str],
    mode: PostgresEnforcement,
) -> sqlx::Result<u64> {
    let terms = columns.iter().map(|c| format!("(({c} IS NULL) OR (isfinite({c}) AND ({c} >= ('0001-01-01T00:00:00.000000Z'::text)::timestamp with time zone) AND ({c} <= ('9999-12-30T22:00:00.999999Z'::text)::timestamp with time zone)))")).collect::<Vec<_>>();
    let predicate = terms.join(" AND ");
    if matches!(mode, PostgresEnforcement::Verify) {
        let sql = format!("SELECT ctid::text FROM {table} WHERE NOT ({predicate}) LIMIT 1");
        if let Some((id,)) = sqlx::query_as::<_, (LegacyInstantTupleId,)>(sqlx::AssertSqlSafe(sql))
            .fetch_optional(&mut *conn)
            .await?
        {
            return Err(sqlx::Error::Protocol(format!(
                "out-of-range comparable instant in {table} row={id}"
            )));
        }
        return Ok(0);
    }
    let expected = if columns.len() == 1 {
        format!("CHECK ({predicate})")
    } else {
        format!("CHECK (({predicate}))")
    };
    let name = format!("canonical_instants_v1_{table}");
    let lookup = format!(
        "SELECT pg_get_constraintdef(c.oid, false), c.convalidated FROM pg_constraint c JOIN pg_class r ON r.oid = c.conrelid JOIN pg_namespace n ON n.oid = r.relnamespace WHERE n.nspname = 'public' AND r.relname = '{table}' AND c.conname = '{name}'"
    );
    let existing: Option<(CatalogDefinition, CatalogConstraintValidated)> =
        sqlx::query_as(sqlx::AssertSqlSafe(lookup))
            .fetch_optional(&mut *conn)
            .await?;
    if let Some((definition, validated)) = existing {
        let definition = definition.as_str();
        if definition != expected || !validated.into_bool() {
            return Err(sqlx::Error::Protocol(format!(
                "different comparable-instant constraint: {name}; expected {expected}, found {definition}"
            )));
        }
        return Ok(1);
    }
    if matches!(mode, PostgresEnforcement::Install) {
        let sql = format!("ALTER TABLE {table} ADD CONSTRAINT {name} CHECK ({predicate})");
        sqlx::query(sqlx::AssertSqlSafe(sql))
            .execute(conn)
            .await
            .map_err(|error| {
                sqlx::Error::Protocol(format!(
                    "installing comparable-instant constraint {name}: {error}"
                ))
            })?;
    }
    Ok(0)
}

pub(crate) async fn check_postgres_enforcement(
    conn: &mut sqlx::PgConnection,
    pending: bool,
) -> sqlx::Result<()> {
    let expected = postgres_enforcement(conn, PostgresEnforcement::Check).await?;
    let (actual,): (RowCount,) = sqlx::query_as("SELECT COUNT(*) FROM pg_constraint c JOIN pg_class r ON r.oid = c.conrelid JOIN pg_namespace n ON n.oid = r.relnamespace WHERE n.nspname = 'public' AND left(c.conname,22) = 'canonical_instants_v1_'").fetch_one(conn).await?;
    if actual.into_u64() != expected || !(expected == 13 || (pending && expected == 0)) {
        return Err(sqlx::Error::Protocol(
            "incomplete comparable-instant enforcement".into(),
        ));
    }
    Ok(())
}

pub(crate) async fn validate_sqlite_population(conn: &mut SqliteConnection) -> sqlx::Result<()> {
    sqlite_enforcement(conn, SqliteEnforcement::Verify)
        .await
        .map(|_| ())
}

pub(crate) async fn validate_postgres_population(
    conn: &mut sqlx::PgConnection,
) -> sqlx::Result<()> {
    postgres_enforcement(conn, PostgresEnforcement::Verify)
        .await
        .map(|_| ())
}

pub(crate) async fn check_sqlite_enforcement(
    conn: &mut SqliteConnection,
    pending: bool,
) -> sqlx::Result<()> {
    let expected = sqlite_enforcement(conn, SqliteEnforcement::Check).await?;
    let (actual,): (RowCount,) = sqlx::query_as("SELECT COUNT(*) FROM sqlite_master WHERE type = 'trigger' AND name GLOB 'canonical_instants_v1_*'").fetch_one(conn).await?;
    if actual.into_u64() != expected || !(expected == 26 || (pending && expected == 0)) {
        return Err(sqlx::Error::Protocol(
            "incomplete comparable-instant enforcement".into(),
        ));
    }
    Ok(())
}
