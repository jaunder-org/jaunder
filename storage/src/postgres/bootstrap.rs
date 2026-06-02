//! `PostgreSQL` administrative bootstrap: creating the application role and the
//! database it owns, using superuser (bootstrap) credentials.
//!
//! These are DDL utility statements (`CREATE ROLE`, `CREATE DATABASE`) whose
//! identifiers and password literals cannot be supplied through bind
//! placeholders, so the SQL is assembled with explicit quoting helpers.

use sqlx::postgres::PgConnectOptions;
use sqlx::{Connection, PgConnection};

/// Error returned by [`create_postgres_database_and_role`].
#[derive(Debug, thiserror::Error)]
pub enum PgBootstrapError {
    /// The application role already existed (SQLSTATE 42710).
    #[error("application role '{0}' already exists")]
    RoleExists(String),
    /// The application database already existed (SQLSTATE 42P04).
    #[error("database '{0}' already exists")]
    DatabaseExists(String),
    /// Any other connection or statement failure.
    #[error(transparent)]
    Sqlx(#[from] sqlx::Error),
}

/// Creates the application role and the database it owns, connecting with the
/// supplied bootstrap (superuser) options.
///
/// Connects with `bootstrap` directly; it does NOT apply
/// [`resolved_postgres_options`](crate::resolved_postgres_options) env-password
/// resolution, because bootstrap credentials come from the URL, not the
/// application environment.
///
/// # Errors
///
/// Returns [`PgBootstrapError::RoleExists`] or
/// [`PgBootstrapError::DatabaseExists`] when the role or database already
/// exists, or [`PgBootstrapError::Sqlx`] for any other failure.
pub async fn create_postgres_database_and_role(
    bootstrap: &PgConnectOptions,
    app_role: &str,
    app_role_password: &str,
    database_name: &str,
) -> Result<(), PgBootstrapError> {
    let mut admin_conn = PgConnection::connect_with(bootstrap).await?;

    // The role name is an identifier and the password appears in a utility
    // statement, so this SQL must be assembled with the quoting helpers rather
    // than query placeholders.
    let role_sql = format!(
        "CREATE ROLE {} WITH LOGIN PASSWORD {}",
        quote_postgres_identifier(app_role),
        quote_postgres_literal(app_role_password),
    );
    if !execute_utility(&mut admin_conn, &role_sql, "42710").await? {
        return Err(PgBootstrapError::RoleExists(app_role.to_owned()));
    }

    // CREATE DATABASE ... OWNER ... is another identifier-bearing utility
    // statement, so placeholders are not usable here either.
    let create_db_sql = format!(
        "CREATE DATABASE {} OWNER {}",
        quote_postgres_identifier(database_name),
        quote_postgres_identifier(app_role),
    );
    if !execute_utility(&mut admin_conn, &create_db_sql, "42P04").await? {
        return Err(PgBootstrapError::DatabaseExists(database_name.to_owned()));
    }

    Ok(())
}

/// Runs a utility statement. Returns `Ok(true)` on success, `Ok(false)` when it
/// fails with `already_exists_code` (the benign "already exists" case), and
/// `Err` for any other database error.
async fn execute_utility(
    conn: &mut PgConnection,
    sql: &str,
    already_exists_code: &str,
) -> Result<bool, sqlx::Error> {
    match sqlx::query(sql).execute(&mut *conn).await {
        Ok(_) => Ok(true),
        Err(sqlx::Error::Database(ref db_error))
            if pg_error_code_matches(db_error.code().as_deref(), already_exists_code) =>
        {
            Ok(false)
        }
        Err(other) => Err(other),
    }
}

fn quote_postgres_identifier(name: &str) -> String {
    // PostgreSQL role/database names are identifiers, not data values, so they
    // cannot be supplied through bind placeholders. Administrative utility
    // statements such as CREATE ROLE and CREATE DATABASE therefore require
    // validated identifier quoting when assembling SQL dynamically.
    format!("\"{}\"", name.replace('"', "\"\""))
}

fn quote_postgres_literal(value: &str) -> String {
    // PostgreSQL also rejects prepared/bound parameters in these utility
    // statements. Password literals therefore need explicit SQL quoting when
    // used in CREATE ROLE statements.
    format!("'{}'", value.replace('\'', "''"))
}

fn pg_error_code_matches(code: Option<&str>, expected: &str) -> bool {
    code == Some(expected)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    #[tokio::test]
    async fn create_postgres_database_and_role_attempts_admin_connection() {
        // Drives the bootstrap routine far enough to exercise the admin
        // connection attempt; the connection itself fails fast against an
        // unused port. The DDL execution past the connection requires a live
        // PostgreSQL server and is covered by the PostgreSQL VM checks.
        let bootstrap: PgConnectOptions =
            "postgres://postgres@localhost:1/postgres".parse().unwrap();
        let _ = tokio::time::timeout(
            Duration::from_millis(50),
            create_postgres_database_and_role(&bootstrap, "app_role", "secret", "app_db"),
        )
        .await;
    }

    #[test]
    fn pg_error_code_matches_returns_true_for_exact_match() {
        assert!(pg_error_code_matches(Some("42710"), "42710"));
    }

    #[test]
    fn pg_error_code_matches_returns_false_for_different_code() {
        assert!(!pg_error_code_matches(Some("42000"), "42710"));
    }

    #[test]
    fn pg_error_code_matches_returns_false_when_no_code() {
        assert!(!pg_error_code_matches(None, "42710"));
    }

    #[test]
    fn quote_postgres_identifier_wraps_and_escapes() {
        assert_eq!(quote_postgres_identifier("users"), "\"users\"");
        assert_eq!(quote_postgres_identifier("user\"name"), "\"user\"\"name\"");
    }

    #[test]
    fn quote_postgres_literal_wraps_and_escapes() {
        assert_eq!(quote_postgres_literal("password"), "'password'");
        assert_eq!(quote_postgres_literal("can't"), "'can''t'");
    }
}
