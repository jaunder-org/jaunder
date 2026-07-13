use jaunder::cli::StorageArgs;
use jaunder::commands::{cmd_create_pg_db, cmd_init};
use sqlx::Connection;
use tempfile::TempDir;

use crate::helpers::{postgres_bootstrap_url, postgres_test_authority};

// guard:low-level-db — provisions a Postgres role/database via bootstrap admin; no standard backend fixture
#[tokio::test]
async fn cmd_create_pg_db_provisions_role_and_database() {
    let suffix = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let role_name = format!("jaunder_role_{suffix}");
    let db_name = format!("jaunder_db_{suffix}");

    let bootstrap = postgres_bootstrap_url();
    let authority = postgres_test_authority();
    let app = format!("postgres://{role_name}@{authority}/{db_name}");

    let mut admin_conn = sqlx::PgConnection::connect(&bootstrap).await.unwrap();
    sqlx::query(&format!("DROP DATABASE IF EXISTS \"{db_name}\""))
        .execute(&mut admin_conn)
        .await
        .unwrap();
    sqlx::query(&format!("DROP ROLE IF EXISTS \"{role_name}\""))
        .execute(&mut admin_conn)
        .await
        .unwrap();

    cmd_create_pg_db(&bootstrap, &app, "bootstrap-secret")
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

    sqlx::query(&format!("DROP DATABASE IF EXISTS \"{db_name}\""))
        .execute(&mut admin_conn)
        .await
        .unwrap();
    sqlx::query(&format!("DROP ROLE IF EXISTS \"{role_name}\""))
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

    let bootstrap = postgres_bootstrap_url();
    let authority = postgres_test_authority();
    let app = format!("postgres://{role_name}@{authority}/{db_name}");

    let mut admin_conn = sqlx::PgConnection::connect(&bootstrap).await.unwrap();
    sqlx::query(&format!("DROP DATABASE IF EXISTS \"{db_name}\""))
        .execute(&mut admin_conn)
        .await
        .unwrap();
    sqlx::query(&format!("DROP ROLE IF EXISTS \"{role_name}\""))
        .execute(&mut admin_conn)
        .await
        .unwrap();
    sqlx::query(&format!("CREATE ROLE \"{role_name}\" LOGIN"))
        .execute(&mut admin_conn)
        .await
        .unwrap();

    let err = cmd_create_pg_db(&bootstrap, &app, "bootstrap-secret")
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

    sqlx::query(&format!("DROP ROLE IF EXISTS \"{role_name}\""))
        .execute(&mut admin_conn)
        .await
        .unwrap();
}
