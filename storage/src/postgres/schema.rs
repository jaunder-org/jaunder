//! Postgres schema-property home: catalog-level invariants the Postgres restore
//! path relies on, read straight from `pg_catalog`.

#[cfg(test)]
mod tests {
    use crate::test_support::{postgres_only, Backend};

    use rstest::*;
    use rstest_reuse::*;

    // Every foreign key must be DEFERRABLE so a restore can `SET CONSTRAINTS ALL
    // DEFERRED` and bulk-load rows in any order, with integrity verified once at
    // COMMIT. This pins that migration 0024 left no NOT DEFERRABLE foreign key
    // behind — the invariant the order-independent Postgres restore relies on.
    #[apply(postgres_only)]
    // reason: FK deferrability is a Postgres catalog property (pg_constraint.condeferrable);
    // SQLite enforces FKs per-connection and has no equivalent, so this is Postgres-only.
    #[tokio::test]
    async fn every_foreign_key_is_deferrable(#[case] backend: Backend) {
        let env = backend.setup().await;
        let pool = env.base.pool().postgres();
        let non_deferrable: i64 = sqlx::query_scalar(
            "SELECT COUNT(*) FROM pg_constraint \
             WHERE contype = 'f' AND connamespace = 'public'::regnamespace \
               AND NOT condeferrable",
        )
        .fetch_one(pool)
        .await
        .unwrap();
        assert_eq!(
            non_deferrable, 0,
            "every foreign key must be DEFERRABLE so restore can SET CONSTRAINTS ALL DEFERRED"
        );
    }
}
