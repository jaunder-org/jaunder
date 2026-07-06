use crate::helpers::{
    postgres_bootstrap_url, postgres_only, recorded_postgres_url, unique_postgres_url, Backend,
};
use sqlx::Connection;

use rstest::*;
#[expect(
    clippy::single_component_path_imports,
    reason = "rstest_reuse needs the bare `use rstest_reuse;` import in scope for its #[template]/#[apply] macros; a glob import would trip wildcard_imports instead"
)]
use rstest_reuse;
use rstest_reuse::*;

/// Database name (last path segment, query stripped) from a Postgres test URL.
fn db_name_from_url(url: &str) -> String {
    let without_query = url.split('?').next().unwrap_or(url);
    without_query
        .rsplit('/')
        .next()
        .expect("URL has a database segment")
        .to_owned()
}

/// True if `db_name` currently exists in the ephemeral cluster.
async fn database_exists(db_name: &str) -> bool {
    let options: sqlx::postgres::PgConnectOptions = postgres_bootstrap_url()
        .parse()
        .expect("bootstrap URL parses");
    let mut conn = sqlx::PgConnection::connect_with(&options)
        .await
        .expect("connect to bootstrap database");
    let exists: bool =
        sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM pg_database WHERE datname = $1)")
            .bind(db_name)
            .fetch_one(&mut conn)
            .await
            .expect("query pg_database");
    conn.close().await.ok();
    exists
}

#[apply(postgres_only)]
// reason: asserts Postgres per-test-database teardown (the ephemeral DB is
// dropped when the TestEnv is gone) via pg_database — SQLite has no such cluster.
#[tokio::test]
async fn per_test_database_is_dropped_on_teardown(#[case] backend: Backend) {
    let _ = backend;

    let env = Backend::Postgres.setup().await;
    let db_name = db_name_from_url(&recorded_postgres_url(&env.base));

    assert!(
        database_exists(&db_name).await,
        "per-test database {db_name} should exist while the TestEnv is alive"
    );

    drop(env);

    assert!(
        !database_exists(&db_name).await,
        "per-test database {db_name} should be dropped once the TestEnv is gone"
    );
}

#[apply(postgres_only)]
// reason: asserts unique_postgres_url()'s per-test database is dropped when its
// PostgresDbGuard is dropped — via pg_database; SQLite has no such cluster.
#[tokio::test]
async fn unique_postgres_database_is_dropped_on_guard_drop(#[case] backend: Backend) {
    let _ = backend;

    let (options, guard) = unique_postgres_url().await;
    let db_name = db_name_from_url(&options.to_string());

    assert!(
        database_exists(&db_name).await,
        "unique_postgres_url() database {db_name} should exist while its guard is held"
    );

    drop(guard);

    assert!(
        !database_exists(&db_name).await,
        "unique_postgres_url() database {db_name} should be dropped once its guard is gone"
    );
}
