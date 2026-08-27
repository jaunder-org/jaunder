//! Postgres migration home: coverage of the real migration sequence running
//! against a from-scratch Postgres database via the public `open_database`.

#[cfg(test)]
mod tests {
    use crate::test_support::{PostgresTestConfig, unique_postgres_url};
    use crate::{StorageRuntimeConfig, open_database};
    use host::config_key::SiteConfigKey;

    // guard:low-level-db — Postgres per-test DBs are template clones (setup bypasses migration); this is the sole test of the real migration run against a from-scratch DB via the public open_database. SQLite has no template, so every SQLite test covers its path.
    #[tokio::test]
    async fn open_database_migrates_a_from_scratch_database() {
        let config = PostgresTestConfig::from_env();
        let (url, _pg) = unique_postgres_url(&config).await;
        let state = open_database(&url, &StorageRuntimeConfig::default())
            .await
            .unwrap();
        // A migrated-but-empty database resolves an unwritten config key to None.
        assert_eq!(
            state
                .site_config
                .get_raw(SiteConfigKey::SiteTitle)
                .await
                .unwrap(),
            None
        );
    }
}
