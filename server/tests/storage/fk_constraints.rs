use rstest::*;
use rstest_reuse::*;
use storage::test_support::{Backend, TestEnv, backends, seed_users};

use super::fixtures::{open_pool, raw_exec};

// Scheduled publishing (#70) relies on a standalone `published_at` index for the
// `published_at <= now` reads and the worker's go-live range scans. This asserts
// the migration created it; a backend `match` is legitimate here because we are
// querying each engine's schema catalog, not exercising divergent product
// behavior.
#[apply(backends)]
#[tokio::test]
async fn posts_published_at_index_exists(#[case] backend: Backend) {
    let env = backend.setup().await;
    let names: Vec<String> = match backend {
        Backend::Sqlite => sqlx::query_scalar(
            "SELECT name FROM sqlite_master \
             WHERE type = 'index' AND name = 'idx_posts_published_at'",
        )
        .fetch_all(&open_pool(&env.base).await)
        .await
        .unwrap(),
        Backend::Postgres => {
            let pool = env.base.pool().postgres();
            sqlx::query_scalar(
                "SELECT indexname FROM pg_indexes WHERE indexname = 'idx_posts_published_at'",
            )
            .fetch_all(pool)
            .await
            .unwrap()
        }
    };
    assert_eq!(names, vec!["idx_posts_published_at".to_string()]);
}

async fn raw_try_exec(backend: Backend, env: &TestEnv, sql: &str) -> Result<(), sqlx::Error> {
    match backend {
        Backend::Sqlite => sqlx::query(sql)
            .execute(&open_pool(&env.base).await)
            .await
            .map(|_| ()),
        Backend::Postgres => {
            // Reuse the pool behind the per-test `AppState` (the same database
            // the state seeded), rather than reconnecting a fresh pool via
            // `recorded_postgres_url`.
            let pool = env.base.pool().postgres();
            sqlx::query(sql).execute(pool).await.map(|_| ())
        }
    }
}

// The same-owner invariant (an audience and a subscription paired in
// `audience_members` must belong to the same author) is enforced by the
// database via two composite FKs that both point at the same `author_user_id`
// column — never by application code. This raw-SQL test isolates the FK as the
// enforcer: `audience_members` has no trait insert that bypasses the owner
// column. With `author_user_id = A` the `(subscription_id, author_user_id)` FK
// fails (the subscription is B's); with `B` the `(audience_id, author_user_id)`
// FK fails (the audience is A's) — either way the DB must reject it.
#[apply(backends)]
#[tokio::test]
async fn composite_fks_reject_cross_author_membership(#[case] backend: Backend) {
    let env = backend.setup().await;
    let state = &env.state;
    // Users via the already-wired UserStore; audience + subscription via raw SQL.
    let [a, b] = seed_users(state).await;

    raw_exec(
        backend,
        &env,
        &format!("INSERT INTO audiences (author_user_id, name) VALUES ({a}, 'Friends')"),
    )
    .await;
    raw_exec(
        backend,
        &env,
        &format!(
            "INSERT INTO subscriptions (author_user_id, channel_id, subscriber_ref, status_id) \
             VALUES ({b}, (SELECT channel_id FROM channels WHERE name='local'), '{b}', \
                     (SELECT status_id FROM subscription_statuses WHERE name='active'))"
        ),
    )
    .await;

    for owner in [a, b] {
        let res = raw_try_exec(
            backend,
            &env,
            &format!(
                "INSERT INTO audience_members (audience_id, subscription_id, author_user_id) VALUES (\
                  (SELECT audience_id FROM audiences WHERE author_user_id={a} AND name='Friends'), \
                  (SELECT subscription_id FROM subscriptions WHERE author_user_id={b} AND subscriber_ref='{b}'), \
                  {owner})"
            ),
        )
        .await;
        assert!(
            res.is_err(),
            "cross-author membership must be rejected by the DB (owner={owner})"
        );
    }
}
