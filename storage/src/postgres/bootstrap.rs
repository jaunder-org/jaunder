//! `PostgreSQL` administrative bootstrap: creating the application role and the
//! database it owns, using superuser (bootstrap) credentials.
//!
//! These are DDL utility statements (`CREATE ROLE`, `CREATE DATABASE`) whose
//! identifiers and password literal cannot be supplied through bind
//! placeholders. `PostgreSQL` escape-string syntax makes the password literal
//! independent of the session's `standard_conforming_strings` setting.

use common::pg_identifier::{PgDatabaseName, PgRoleName};
use common::pg_role_password::PgRolePassword;
use sqlx::postgres::PgConnectOptions;
use sqlx::{AssertSqlSafe, Connection, PgConnection};

use crate::sql;

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
/// The three trailing parameters were once three adjacent `&str`, with a credential in
/// the middle — every permutation compiled (#693). They are three distinct types now, so
/// a transposition is a compile error rather than a silent mis-provision.
///
/// The positive companion shows the identical fixture compiles when the arguments are in
/// the right slots, so the `compile_fail` below fails for the transposition rather than
/// for a moved path or a changed signature (#763):
///
/// ```
/// use common::pg_identifier::{PgDatabaseName, PgRoleName};
/// use common::pg_role_password::PgRolePassword;
/// use sqlx::postgres::PgConnectOptions;
/// use storage::create_postgres_database_and_role;
/// # async fn f(bootstrap: &PgConnectOptions) {
/// let pw: PgRolePassword = "p".parse().unwrap();
/// let role: PgRoleName = "r".parse().unwrap();
/// let db: PgDatabaseName = "d".parse().unwrap();
/// let _ = create_postgres_database_and_role(bootstrap, &role, &pw, &db).await;
/// # }
/// ```
///
/// …and the transposed call does not:
///
/// ```compile_fail
/// use common::pg_identifier::{PgDatabaseName, PgRoleName};
/// use common::pg_role_password::PgRolePassword;
/// use sqlx::postgres::PgConnectOptions;
/// use storage::create_postgres_database_and_role;
/// # async fn f(bootstrap: &PgConnectOptions) {
/// let pw: PgRolePassword = "p".parse().unwrap();
/// let db: PgDatabaseName = "d".parse().unwrap();
/// // `db` is a PgDatabaseName, but the role slot needs a PgRoleName
/// let _ = create_postgres_database_and_role(bootstrap, &db, &pw, &db).await;
/// # }
/// ```
///
/// # Errors
///
/// Returns [`PgBootstrapError::RoleExists`] or
/// [`PgBootstrapError::DatabaseExists`] when the role or database already
/// exists, or [`PgBootstrapError::Sqlx`] for any other failure.
pub async fn create_postgres_database_and_role(
    bootstrap: &PgConnectOptions,
    app_role: &PgRoleName,
    app_role_password: &PgRolePassword,
    database_name: &PgDatabaseName,
) -> Result<(), PgBootstrapError> {
    let mut admin_conn = PgConnection::connect_with(bootstrap).await?;

    // The role name is an identifier and the password appears in a utility
    // statement, so this SQL must be assembled with backend-owned quoting
    // rather than query placeholders.
    //
    // `as_ref()` is the *only* place the password leaves its newtype: the `secret`
    // surface has no `Display`/serde/`Deref`/owned-`String`, so any other use of it
    // here would fail to compile.
    // These utility statements cannot bind identifiers or the password literal.
    // Their only dynamic fragments are backend-quoted role/database identifiers and password.
    let role_sql = AssertSqlSafe(format!(
        "CREATE ROLE {} WITH LOGIN PASSWORD {}",
        sql::quote_identifier(app_role),
        quote_postgres_utility_literal(app_role_password.as_ref()),
    ));
    if !execute_utility(&mut admin_conn, role_sql, "42710").await? {
        return Err(PgBootstrapError::RoleExists(app_role.to_string()));
    }

    // CREATE DATABASE ... OWNER ... is another identifier-bearing utility
    // statement, so placeholders are not usable here either.
    let create_db_sql = AssertSqlSafe(format!(
        "CREATE DATABASE {} OWNER {}",
        sql::quote_identifier(database_name),
        sql::quote_identifier(app_role),
    ));
    if !execute_utility(&mut admin_conn, create_db_sql, "42P04").await? {
        return Err(PgBootstrapError::DatabaseExists(database_name.to_string()));
    }

    Ok(())
}

/// Quotes text for a `PostgreSQL` utility-statement literal.
///
/// `CREATE ROLE ... PASSWORD` does not accept a bind parameter. The `E` prefix
/// selects `PostgreSQL`'s escape-string grammar regardless of
/// `standard_conforming_strings`, so both a literal backslash and an apostrophe
/// are represented unambiguously.
fn quote_postgres_utility_literal(value: &str) -> String {
    format!("E'{}'", value.replace('\\', "\\\\").replace('\'', "''"))
}

/// Runs a utility statement. Returns `Ok(true)` on success, `Ok(false)` when it
/// fails with `already_exists_code` (the benign "already exists" case), and
/// `Err` for any other database error.
async fn execute_utility(
    conn: &mut PgConnection,
    sql: sqlx::AssertSqlSafe<String>,
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

fn pg_error_code_matches(code: Option<&str>, expected: &str) -> bool {
    code == Some(expected)
}

/// The database role reported by `PostgreSQL` after a bootstrap-created login.
#[cfg(test)]
#[derive(Debug, macros::SqlxBridge)]
struct BootstrapCurrentRole(String);

#[cfg(test)]
impl BootstrapCurrentRole {
    fn as_str(&self) -> &str {
        &self.0
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::PostgresTestConfig;
    use std::time::Duration;

    /// A process-unique suffix for admin-created role/database identifiers, so
    /// the bootstrap tests don't collide with each other or a prior run.
    fn unique_suffix() -> String {
        use std::sync::atomic::{AtomicU64, Ordering};
        static COUNTER: AtomicU64 = AtomicU64::new(0);
        let n = COUNTER.fetch_add(1, Ordering::Relaxed);
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .expect("clock after unix epoch")
            .as_nanos();
        format!("{nanos}_{n}")
    }

    // guard:low-level-db — exercises CREATE ROLE/CREATE DATABASE admin DDL via a
    // bootstrap admin connection, below the backend fixture; no SQLite analog.
    #[tokio::test]
    async fn create_postgres_database_and_role_attempts_admin_connection() {
        // Drives the bootstrap routine far enough to exercise the admin
        // connection attempt; the connection itself fails fast against an
        // unused port. The DDL execution past the connection requires a live
        // PostgreSQL server and is covered by the PostgreSQL VM checks.
        let bootstrap: PgConnectOptions =
            "postgres://postgres@localhost:1/postgres".parse().unwrap();
        let app_role: PgRoleName = "app_role".parse().unwrap();
        let password: PgRolePassword = "secret".parse().unwrap();
        let app_db: PgDatabaseName = "app_db".parse().unwrap();
        let _ = tokio::time::timeout(
            Duration::from_millis(50),
            create_postgres_database_and_role(&bootstrap, &app_role, &password, &app_db),
        )
        .await;
    }

    // guard:low-level-db — verifies PostgreSQL utility-literal escaping against
    // a live server with standard_conforming_strings disabled; no SQLite analog.
    #[tokio::test]
    async fn create_postgres_database_and_role_preserves_delimiters_with_scs_off() {
        let config = PostgresTestConfig::from_env();
        let bootstrap: PgConnectOptions = config.bootstrap_url().parse().expect("bootstrap url");
        let scs_off_bootstrap = bootstrap
            .clone()
            .options([("standard_conforming_strings", "off")]);
        let suffix = unique_suffix();
        let db_name = format!("cov_bootstrap_db_'\\_{suffix}");
        let role_name = format!("cov_bootstrap_role_'\\_{suffix}");
        let password = "secret'\\password";
        let app_role: PgRoleName = role_name.parse().expect("role name");
        let app_password: PgRolePassword = password.parse().expect("password");
        let app_db: PgDatabaseName = db_name.parse().expect("database name");

        create_postgres_database_and_role(&scs_off_bootstrap, &app_role, &app_password, &app_db)
            .await
            .expect("bootstrap succeeds with delimiter-bearing values and SCS off");

        let app_options = bootstrap
            .clone()
            .username(&role_name)
            .password(password)
            .database(&db_name);
        let mut app_connection = PgConnection::connect_with(&app_options)
            .await
            .expect("created role accepts the exact password");
        let current_user: BootstrapCurrentRole = sqlx::query_scalar("SELECT current_user")
            .fetch_one(&mut app_connection)
            .await
            .expect("created role can connect to its database");
        assert_eq!(current_user.as_str(), role_name);
        drop(app_connection);

        let mut admin = PgConnection::connect_with(&bootstrap)
            .await
            .expect("admin connect");
        sqlx::query(AssertSqlSafe(format!(
            "DROP DATABASE {}",
            sql::quote_identifier(&db_name)
        )))
        .execute(&mut admin)
        .await
        .expect("drop test database");
        sqlx::query(AssertSqlSafe(format!(
            "DROP ROLE {}",
            sql::quote_identifier(&role_name)
        )))
        .execute(&mut admin)
        .await
        .expect("drop test role");
    }

    // guard:low-level-db — exercises the DatabaseExists arm of admin CREATE DATABASE
    // DDL via a bootstrap admin connection, below the backend fixture; no SQLite analog.
    #[tokio::test]
    async fn create_postgres_database_and_role_reports_existing_database() {
        let config = PostgresTestConfig::from_env();
        let bootstrap: PgConnectOptions = config.bootstrap_url().parse().expect("bootstrap url");
        let mut admin = PgConnection::connect_with(&bootstrap)
            .await
            .expect("admin connect");

        let suffix = unique_suffix();
        let db_name = format!("cov_bootstrap_db_\"quote_{suffix}");
        let role_name = format!("cov_bootstrap_role_\"quote_{suffix}");
        // Pre-create the target database so the bootstrap's CREATE DATABASE hits
        // the benign already-exists (42P04) path and reports DatabaseExists.
        sqlx::query(AssertSqlSafe(format!(
            "CREATE DATABASE {}",
            sql::quote_identifier(&db_name)
        )))
        .execute(&mut admin)
        .await
        .expect("pre-create database");

        let app_role: PgRoleName = role_name.parse().expect("role name");
        let password: PgRolePassword = "secret'quote".parse().expect("password");
        let app_db: PgDatabaseName = db_name.parse().expect("database name");

        let error = create_postgres_database_and_role(&bootstrap, &app_role, &password, &app_db)
            .await
            .expect_err("database already exists");
        assert!(matches!(error, PgBootstrapError::DatabaseExists(_)));

        // The role is created before the DB step fails, so drop both to leave the
        // shared cluster clean.
        let _ = sqlx::query(AssertSqlSafe(format!(
            "DROP DATABASE {}",
            sql::quote_identifier(&db_name)
        )))
        .execute(&mut admin)
        .await;
        let _ = sqlx::query(AssertSqlSafe(format!(
            "DROP ROLE {}",
            sql::quote_identifier(&role_name)
        )))
        .execute(&mut admin)
        .await;
    }

    // guard:low-level-db — exercises execute_utility's non-benign error passthrough
    // over admin DDL via a bootstrap admin connection, below the backend fixture; no SQLite analog.
    #[tokio::test]
    async fn execute_utility_propagates_unexpected_errors() {
        let config = PostgresTestConfig::from_env();
        let bootstrap: PgConnectOptions = config.bootstrap_url().parse().expect("bootstrap url");
        let mut admin = PgConnection::connect_with(&bootstrap)
            .await
            .expect("admin connect");

        // A syntax error's SQLSTATE (42601) never matches the already-exists code,
        // so execute_utility surfaces it as Err rather than the benign Ok(false).
        let result = execute_utility(
            &mut admin,
            AssertSqlSafe("NOT A VALID STATEMENT".to_owned()),
            "42710",
        )
        .await;
        assert!(result.is_err());
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
}
