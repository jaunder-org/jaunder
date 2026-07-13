//! Postgres migration home: coverage of the real migration sequence running
//! against a from-scratch Postgres database via the public `open_database`.

#[cfg(test)]
mod tests {
    use crate::open_database;
    use crate::test_support::unique_postgres_url;

    // guard:low-level-db — Postgres per-test DBs are template clones (setup bypasses migration); this is the sole test of the real migration run against a from-scratch DB via the public open_database. SQLite has no template, so every SQLite test covers its path.
    #[tokio::test]
    async fn open_database_migrates_a_from_scratch_database() {
        let (url, _pg) = unique_postgres_url().await;
        let state = open_database(&url).await.unwrap();
        // A migrated-but-empty database resolves a missing config key to None.
        assert_eq!(state.site_config.get("missing").await.unwrap(), None);
    }
}
