use rstest::*;
use rstest_reuse::*;
use storage::sql::QueryStorageExt;
use storage::test_support::{Backend, backends, seed_users};

use super::fixtures::open_pool;

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

    let audience_insert = storage::with_closeable_pool!(env.base.pool(), pool, {
        sqlx::query("INSERT INTO audiences (author_user_id, name) VALUES ($1, 'Friends')")
            .bind_storage(a)
            .execute(pool)
            .await
            .map(|_| ())
    });
    audience_insert.expect("audience fixture setup should succeed");

    let subscriber_ref = b.to_string();
    let subscription_insert = storage::with_closeable_pool!(env.base.pool(), pool, {
        sqlx::query(
            "INSERT INTO subscriptions (author_user_id, channel_id, subscriber_ref, status_id) \
             VALUES ($1, (SELECT channel_id FROM channels WHERE name = 'local'), $2, \
                     (SELECT status_id FROM subscription_statuses WHERE name = 'active'))",
        )
        .bind_storage(b)
        .bind(&subscriber_ref)
        .execute(pool)
        .await
        .map(|_| ())
    });
    subscription_insert.expect("subscription fixture setup should succeed");

    for owner in [a, b] {
        let result = storage::with_closeable_pool!(env.base.pool(), pool, {
            sqlx::query(
                "INSERT INTO audience_members (audience_id, subscription_id, author_user_id) \
                 VALUES (\
                   (SELECT audience_id FROM audiences WHERE author_user_id = $1 AND name = 'Friends'), \
                   (SELECT subscription_id FROM subscriptions \
                    WHERE author_user_id = $2 AND subscriber_ref = $3), \
                   $4)",
            )
            .bind_storage(a)
            .bind_storage(b)
            .bind(&subscriber_ref)
            .bind_storage(owner)
            .execute(pool)
            .await
            .map(|_| ())
        });
        assert!(
            result.is_err(),
            "cross-author membership must be rejected by the DB (owner={owner})"
        );
    }
}
