use rstest::*;
use rstest_reuse::*;
use storage::test_support::{Backend, backends, recorded_postgres_url, sqlite_url};
use storage::{DbConnectOptions, open_database};

#[apply(backends)]
#[tokio::test]
async fn second_open_on_migrated_database_succeeds(#[case] backend: Backend) {
    let env = backend.setup().await;

    // Re-open the *same* per-test database the setup just migrated, addressed by
    // its backend URL, to prove a second `open_database` (re-running migrations)
    // is idempotent on both backends.
    let opts = match backend {
        Backend::Sqlite => sqlite_url(&env.base),
        Backend::Postgres => recorded_postgres_url(&env.base).parse().unwrap(),
    };
    open_database(&opts, &storage::StorageRuntimeConfig::default())
        .await
        .unwrap();
}

#[test]
fn postgres_url_is_accepted_at_parse_time() {
    let result = "postgres://localhost/test".parse::<DbConnectOptions>();
    assert!(result.is_ok());
}

#[test]
fn unsupported_url_is_rejected_at_parse_time() {
    let result = "mysql://localhost/test".parse::<DbConnectOptions>();
    assert!(result.is_err());
}
