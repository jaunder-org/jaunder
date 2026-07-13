//! `SQLite` pool home: pins the connection-level property the schema relies on —
//! `foreign_keys` enforcement — which is per-connection on `SQLite` and defaulted
//! ON by sqlx's `SqliteConnectOptions`.

#[cfg(test)]
mod tests {
    use crate::test_support::{sqlite_only, sqlite_url, Backend};
    use crate::DbConnectOptions;
    use sqlx::SqlitePool;
    use tempfile::TempDir;

    use rstest::*;
    use rstest_reuse::*;

    /// A migrated `SQLite` pool over the `test.db` under `base`, with `foreign_keys`
    /// left at sqlx's ON default — the FK-enforcing pool this regression guard needs.
    async fn fk_enforcing_pool(base: &TempDir) -> SqlitePool {
        let DbConnectOptions::Sqlite(opts) = sqlite_url(base) else {
            unreachable!("sqlite_url always yields Sqlite");
        };
        let pool = SqlitePool::connect_with(opts.create_if_missing(true))
            .await
            .unwrap();
        sqlx::migrate!("../storage/migrations/sqlite")
            .run(&pool)
            .await
            .unwrap();
        pool
    }

    // reason: Foreign-key enforcement is per-connection in SQLite. sqlx's
    // `SqliteConnectOptions` defaults `foreign_keys` to ON, so every pooled
    // connection (app and test) enforces FKs. The composite same-owner FKs added in
    // later content-visibility phases depend on that, so this is a regression guard:
    // a child-row insert referencing a non-existent parent must be rejected. It would
    // fail if anyone disabled `foreign_keys` on the pool or a sqlx change dropped the
    // default.
    #[apply(sqlite_only)]
    #[tokio::test]
    async fn sqlite_pool_enforces_foreign_keys(#[case] backend: Backend) {
        let env = backend.setup().await;
        let pool = fk_enforcing_pool(&env.base).await; // FK-enforcing pool (sqlx default)
        let result = sqlx::query(
            "INSERT INTO post_revisions (post_id, user_id, title, slug, body, format, rendered_html)
             VALUES (999999, 999999, 't', 's', 'b', 'markdown', '<p>b</p>')",
        )
        .execute(&pool)
        .await;
        assert!(
            result.is_err(),
            "FK violation must be rejected when foreign_keys is ON"
        );
    }
}
