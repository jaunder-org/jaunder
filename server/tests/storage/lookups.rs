use common::visibility::{Channel, SubscriptionStatus, TargetKind};
use rstest::*;
use rstest_reuse::*;
use sqlx::{AssertSqlSafe, PgPool};
use storage::test_support::{
    Backend, PostgresDbGuard, PostgresTestConfig, TestEnv, backends, template_postgres_url,
};

use super::fixtures::open_pool;

async fn open_pg_pool() -> (PgPool, PostgresDbGuard) {
    let config = PostgresTestConfig::from_env();
    let (url, guard) = template_postgres_url(&config).await;
    // `expose_url`, not `to_string`: we are connecting, so the password must survive.
    let pool = PgPool::connect(&url.expose_url()).await.unwrap();
    (pool, guard)
}

// `table` is selected only by the fixed lookup-table call sites below. Quote it
// before assembling the structural SQL so an identifier delimiter cannot alter
// the statement.
fn quote_identifier(identifier: &str) -> String {
    format!("\"{}\"", identifier.replace('"', "\"\""))
}

async fn lookup_names(backend: Backend, env: &TestEnv, table: &str) -> Vec<String> {
    let sql = AssertSqlSafe(format!(
        "SELECT name FROM {} ORDER BY name",
        quote_identifier(table)
    ));
    match backend {
        Backend::Sqlite => sqlx::query_scalar(sql)
            .fetch_all(&open_pool(&env.base).await)
            .await
            .unwrap(),
        Backend::Postgres => {
            let (pool, _pg) = open_pg_pool().await;
            sqlx::query_scalar(sql).fetch_all(&pool).await.unwrap()
        }
    }
}

#[test]
fn quote_identifier_doubles_delimiters() {
    assert_eq!(quote_identifier("lookup\"table"), "\"lookup\"\"table\"");
}

#[apply(backends)]
#[tokio::test]
async fn channels_bijection(#[case] backend: Backend) {
    let env = backend.setup().await;
    let names = lookup_names(backend, &env, "channels").await;
    for n in &names {
        assert!(
            Channel::try_from(n.as_str()).is_ok(),
            "unseeded enum for channel {n}"
        );
    }
    let c = Channel::Local;
    assert!(
        names.iter().any(|n| n == c.as_ref()),
        "missing seed for {c}"
    );
}

#[apply(backends)]
#[tokio::test]
async fn target_kinds_bijection(#[case] backend: Backend) {
    let env = backend.setup().await;
    let names = lookup_names(backend, &env, "target_kinds").await;
    for n in &names {
        assert!(
            TargetKind::try_from(n.as_str()).is_ok(),
            "unseeded enum for target kind {n}"
        );
    }
    for k in [
        TargetKind::Public,
        TargetKind::Subscribers,
        TargetKind::Named,
    ] {
        assert!(
            names.iter().any(|n| n == k.as_ref()),
            "missing seed for {k}"
        );
    }
}

#[apply(backends)]
#[tokio::test]
async fn statuses_seed_maps_to_enum(#[case] backend: Backend) {
    let env = backend.setup().await;
    let names = lookup_names(backend, &env, "subscription_statuses").await;
    // Seeded names must each map to a variant (no orphan seed)...
    for n in &names {
        assert!(
            SubscriptionStatus::try_from(n.as_str()).is_ok(),
            "unseeded enum for subscription status {n}"
        );
    }
    // ...and the one status seeded this milestone must be present. `Pending`
    // and `Blocked` variants exist (reserved for M13/M15) but have no rows yet,
    // so this is the subset direction only — not exact bijection.
    assert!(
        names
            .iter()
            .any(|n| n == SubscriptionStatus::Active.as_ref()),
        "missing seed for {}",
        SubscriptionStatus::Active
    );
}
