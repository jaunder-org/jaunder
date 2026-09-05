use host::telemetry::{TelemetryConfig, TelemetryRawConfig};

use jaunder::cli::StorageArgs;
use jaunder::commands::{ServeCapturePaths, cmd_create_pg_db, cmd_init, prepare_server};
use sqlx::{AssertSqlSafe, Connection};
use tempfile::TempDir;

use storage::test_support::{PostgresTestConfig, nonexistent_postgres_url};

// These test-generated database and role names become PostgreSQL identifiers,
// not values. Double-quote them at the construction seam before approving the
// resulting structural utility statement.
fn quote_postgres_identifier(identifier: &str) -> String {
    format!("\"{}\"", identifier.replace('"', "\"\""))
}

#[test]
fn quote_postgres_identifier_doubles_delimiters() {
    assert_eq!(
        quote_postgres_identifier("jaunder\"role"),
        "\"jaunder\"\"role\""
    );
}

fn test_host_config() -> (TelemetryConfig, Option<ServeCapturePaths>) {
    let telemetry = TelemetryConfig::from_raw(
        false,
        TelemetryRawConfig {
            log_filter: Ok(None),
            rust_log: Ok(None),
            log_format: Ok(None),
            jaunder_otlp_endpoint: Ok(None),
            otlp_endpoint: Ok(None),
            slow_op_ms: Ok(None),
            e2e_seed_process: Ok(None),
        },
    );
    (telemetry, None)
}

// guard:low-level-db — provisions a Postgres role/database via bootstrap admin; no standard backend fixture
#[tokio::test]
async fn cmd_create_pg_db_provisions_role_and_database() {
    let suffix = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let role_name = format!("jaunder_role_{suffix}");
    let db_name = format!("jaunder_db_{suffix}");

    let config = PostgresTestConfig::from_env();
    let bootstrap = config.bootstrap_url();
    let authority = config.bootstrap_authority();
    let app = format!("postgres://{role_name}@{authority}/{db_name}");

    let mut admin_conn = sqlx::PgConnection::connect(bootstrap).await.unwrap();
    sqlx::query(AssertSqlSafe(format!(
        "DROP DATABASE IF EXISTS {}",
        quote_postgres_identifier(&db_name)
    )))
    .execute(&mut admin_conn)
    .await
    .unwrap();
    sqlx::query(AssertSqlSafe(format!(
        "DROP ROLE IF EXISTS {}",
        quote_postgres_identifier(&role_name)
    )))
    .execute(&mut admin_conn)
    .await
    .unwrap();

    cmd_create_pg_db(
        &bootstrap.parse().unwrap(),
        &app.parse().unwrap(),
        &"bootstrap-secret".parse().unwrap(),
    )
    .await
    .unwrap();

    let role_exists =
        sqlx::query_scalar::<_, bool>("SELECT EXISTS(SELECT 1 FROM pg_roles WHERE rolname = $1)")
            .bind(&role_name)
            .fetch_one(&mut admin_conn)
            .await
            .unwrap();
    assert!(role_exists);

    let owner = sqlx::query_scalar::<_, Option<String>>(
        "SELECT owner.rolname
         FROM pg_database db
         JOIN pg_roles owner ON owner.oid = db.datdba
         WHERE db.datname = $1",
    )
    .bind(&db_name)
    .fetch_optional(&mut admin_conn)
    .await
    .unwrap()
    .flatten();
    assert_eq!(owner.as_deref(), Some(role_name.as_str()));

    let storage_path = TempDir::new().unwrap();
    let args = StorageArgs {
        storage_path: storage_path.path().join("storage"),
        db: app.parse().unwrap(),
    };
    cmd_init(&args, false).await.unwrap();

    sqlx::query(AssertSqlSafe(format!(
        "DROP DATABASE IF EXISTS {}",
        quote_postgres_identifier(&db_name)
    )))
    .execute(&mut admin_conn)
    .await
    .unwrap();
    sqlx::query(AssertSqlSafe(format!(
        "DROP ROLE IF EXISTS {}",
        quote_postgres_identifier(&role_name)
    )))
    .execute(&mut admin_conn)
    .await
    .unwrap();
}

// guard:low-level-db — provisions a Postgres role/database via bootstrap admin; no standard backend fixture
#[tokio::test]
async fn cmd_create_pg_db_fails_if_role_already_exists() {
    let suffix = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let role_name = format!("jaunder_role_{suffix}");
    let db_name = format!("jaunder_db_{suffix}");

    let config = PostgresTestConfig::from_env();
    let bootstrap = config.bootstrap_url();
    let authority = config.bootstrap_authority();
    let app = format!("postgres://{role_name}@{authority}/{db_name}");

    let mut admin_conn = sqlx::PgConnection::connect(bootstrap).await.unwrap();
    sqlx::query(AssertSqlSafe(format!(
        "DROP DATABASE IF EXISTS {}",
        quote_postgres_identifier(&db_name)
    )))
    .execute(&mut admin_conn)
    .await
    .unwrap();
    sqlx::query(AssertSqlSafe(format!(
        "DROP ROLE IF EXISTS {}",
        quote_postgres_identifier(&role_name)
    )))
    .execute(&mut admin_conn)
    .await
    .unwrap();
    sqlx::query(AssertSqlSafe(format!(
        "CREATE ROLE {} LOGIN",
        quote_postgres_identifier(&role_name)
    )))
    .execute(&mut admin_conn)
    .await
    .unwrap();

    let err = cmd_create_pg_db(
        &bootstrap.parse().unwrap(),
        &app.parse().unwrap(),
        &"bootstrap-secret".parse().unwrap(),
    )
    .await
    .unwrap_err();
    assert!(err.to_string().contains("already exists"));

    let db_exists = sqlx::query_scalar::<_, bool>(
        "SELECT EXISTS(SELECT 1 FROM pg_database WHERE datname = $1)",
    )
    .bind(&db_name)
    .fetch_one(&mut admin_conn)
    .await
    .unwrap();
    assert!(!db_exists);

    sqlx::query(AssertSqlSafe(format!(
        "DROP ROLE IF EXISTS {}",
        quote_postgres_identifier(&role_name)
    )))
    .execute(&mut admin_conn)
    .await
    .unwrap();
}

// guard:low-level-db — verifies PostgreSQL startup classification against a real missing database
#[tokio::test]
async fn prepare_server_postgres_missing_database_preserves_3d000_guidance() {
    let base = TempDir::new().expect("temp dir");
    let storage_path = base.path().join("storage");
    let args = StorageArgs {
        storage_path: storage_path.clone(),
        db: {
            let config = PostgresTestConfig::from_env();
            nonexistent_postgres_url(&config)
        },
    };
    let bind = "127.0.0.1:0".parse().expect("bind address");

    let (telemetry, capture) = test_host_config();
    let error = prepare_server(&args, bind, false, &telemetry, capture.as_ref())
        .await
        .err()
        .expect("missing PostgreSQL database must fail");

    assert!(
        error
            .to_string()
            .contains("run `jaunder create-pg-db` first"),
        "SQLSTATE 3D000 must carry PostgreSQL bootstrap guidance: {error:#}"
    );
    let source = error
        .chain()
        .find_map(|source| source.downcast_ref::<sqlx::Error>())
        .expect("typed SQLx source");
    assert!(
        matches!(
            source,
            sqlx::Error::Database(database)
                if database.code().as_deref() == Some("3D000")
        ),
        "expected SQLSTATE 3D000, got {source:?}"
    );
    assert!(
        storage_path.join("media/tmp").is_dir(),
        "transient upload cleanup must precede the database check"
    );
    assert!(
        !storage_path.join("runtime.json").exists(),
        "failed PostgreSQL startup must not publish a live runtime file"
    );
}
