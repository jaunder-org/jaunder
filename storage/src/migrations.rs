//! Portable migration-contract coverage for upgrades that need to inspect both
//! the pre-migration and current schema on fresh databases.

#[cfg(test)]
mod tests {
    use std::borrow::Cow;

    use crate::DbConnectOptions;
    use crate::test_support::{
        Backend, CloseablePool, PostgresDbGuard, backends, sqlite_url, unique_postgres_url,
    };

    use rstest::*;
    use rstest_reuse::*;
    use sqlx::migrate::{MigrateError, Migrator};
    use sqlx::{PgPool, SqlitePool};
    use tempfile::TempDir;

    static SQLITE_MIGRATOR: Migrator = sqlx::migrate!("./migrations/sqlite");
    static POSTGRES_MIGRATOR: Migrator = sqlx::migrate!("./migrations/postgres");

    fn migrator_through(source: &Migrator, version: i64) -> Migrator {
        Migrator {
            migrations: Cow::Owned(
                source
                    .iter()
                    .filter(|migration| migration.version <= version)
                    .cloned()
                    .collect(),
            ),
            ..Migrator::DEFAULT
        }
    }

    struct MigrationDatabase {
        pool: CloseablePool,
        _sqlite: Option<TempDir>,
        _postgres: Option<PostgresDbGuard>,
    }

    impl MigrationDatabase {
        async fn new(backend: Backend) -> Self {
            match backend {
                Backend::Sqlite => {
                    let base = TempDir::new().unwrap();
                    let DbConnectOptions::Sqlite(options) = sqlite_url(&base) else {
                        unreachable!("sqlite_url always yields SQLite options")
                    };
                    let pool = SqlitePool::connect_with(options.create_if_missing(true))
                        .await
                        .unwrap();
                    Self {
                        pool: CloseablePool::Sqlite(pool),
                        _sqlite: Some(base),
                        _postgres: None,
                    }
                }
                Backend::Postgres => {
                    let (options, guard) = unique_postgres_url().await;
                    let DbConnectOptions::Postgres { options, .. } = options else {
                        unreachable!("unique_postgres_url always yields PostgreSQL options")
                    };
                    let pool = PgPool::connect_with(options).await.unwrap();
                    Self {
                        pool: CloseablePool::Postgres(pool),
                        _sqlite: None,
                        _postgres: Some(guard),
                    }
                }
            }
        }

        async fn migrate_to(&self, version: i64) -> Result<(), MigrateError> {
            match &self.pool {
                CloseablePool::Sqlite(pool) => {
                    migrator_through(&SQLITE_MIGRATOR, version).run(pool).await
                }
                CloseablePool::Postgres(pool) => {
                    migrator_through(&POSTGRES_MIGRATOR, version)
                        .run(pool)
                        .await
                }
            }
        }
        async fn migrate_current(&self) -> Result<(), MigrateError> {
            match &self.pool {
                CloseablePool::Sqlite(pool) => SQLITE_MIGRATOR.run(pool).await,
                CloseablePool::Postgres(pool) => POSTGRES_MIGRATOR.run(pool).await,
            }
        }

        async fn backfill_post_media_references(&self) {
            match &self.pool {
                CloseablePool::Sqlite(pool) => crate::posts::backfill_post_media_references(pool)
                    .await
                    .expect("startup backfill succeeds"),
                CloseablePool::Postgres(pool) => crate::posts::backfill_post_media_references(pool)
                    .await
                    .expect("startup backfill succeeds"),
            }
        }

        async fn seed_legacy_post_media(&self) {
            let insert_user = match &self.pool {
                CloseablePool::Sqlite(_) => {
                    "INSERT INTO users \
                     (user_id, username, password_hash, created_at) \
                     VALUES (404, 'migration-media-author', 'hash', CURRENT_TIMESTAMP)"
                }
                CloseablePool::Postgres(_) => {
                    "INSERT INTO users \
                     (user_id, username, password_hash, created_at) \
                     OVERRIDING SYSTEM VALUE \
                     VALUES (404, 'migration-media-author', 'hash', CURRENT_TIMESTAMP)"
                }
            };
            let insert_post = match &self.pool {
                CloseablePool::Sqlite(_) => {
                    r#"
                    INSERT INTO posts
                    (post_id, user_id, title, slug, body, format, rendered_html, created_at, updated_at)
                    VALUES (505, 404, NULL, 'migration-media', 'body', 'html',
                    '<img src="/media/upload/e3/b0/e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855/relative.jpg">
                     <img src="https://example.com/media/upload/e3/b0/e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855/same.jpg">
                     <img src="https://foreign.example/media/upload/e3/b0/e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855/foreign.jpg">
                     <img src="//example.com/media/upload/e3/b0/e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855/scheme.jpg">
                     <img src="http://example.com/media/upload/e3/b0/e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855/overlap.jpg">
                     <img src="//example.com/media/upload/e3/b0/e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855/overlap.jpg">',
                     CURRENT_TIMESTAMP, CURRENT_TIMESTAMP)
                "#
                }
                CloseablePool::Postgres(_) => {
                    r#"
                    INSERT INTO posts
                    (post_id, user_id, title, slug, body, format, rendered_html, created_at, updated_at)
                    OVERRIDING SYSTEM VALUE
                    VALUES (505, 404, NULL, 'migration-media', 'body', 'html',
                    '<img src="/media/upload/e3/b0/e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855/relative.jpg">
                     <img src="https://example.com/media/upload/e3/b0/e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855/same.jpg">
                     <img src="https://foreign.example/media/upload/e3/b0/e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855/foreign.jpg">
                     <img src="//example.com/media/upload/e3/b0/e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855/scheme.jpg">
                     <img src="http://example.com/media/upload/e3/b0/e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855/overlap.jpg">
                     <img src="//example.com/media/upload/e3/b0/e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855/overlap.jpg">',
                     CURRENT_TIMESTAMP, CURRENT_TIMESTAMP)
                "#
                }
            };
            self.pool.execute(insert_user).await.unwrap();
            self.pool.execute(insert_post).await.unwrap();
            self.pool
                .execute(
                    "INSERT INTO post_media (post_id, source, sha256, filename) VALUES \
                     (505, 'upload', 'e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855', 'relative.jpg'), \
                     (505, 'upload', 'e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855', 'same.jpg'), \
                     (505, 'upload', 'e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855', 'foreign.jpg'), \
                     (505, 'upload', 'e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855', 'scheme.jpg'), \
                     (505, 'upload', 'e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855', 'overlap.jpg')",
                )
                .await
                .unwrap();
        }

        async fn post_media_references(&self) -> Vec<(String, String, String, String, String)> {
            self.pool
                .string_quintuples(
                    "SELECT source, sha256, filename, reference_kind, reference_form FROM post_media \
                     ORDER BY source, sha256, filename, reference_kind, reference_form",
                )
                .await
                .expect("post media rows query succeeds")
        }

        async fn seed_subscription_graph(&self, subscriber_ref: &str) {
            let insert_user = match &self.pool {
                CloseablePool::Sqlite(_) => {
                    "INSERT INTO users \
                     (user_id, username, password_hash, created_at) \
                     VALUES (101, 'migration-author', 'hash', CURRENT_TIMESTAMP)"
                }
                CloseablePool::Postgres(_) => {
                    "INSERT INTO users \
                     (user_id, username, password_hash, created_at) \
                     OVERRIDING SYSTEM VALUE \
                     VALUES (101, 'migration-author', 'hash', CURRENT_TIMESTAMP)"
                }
            };
            self.pool.execute(insert_user).await.unwrap();
            self.pool
                .execute(&format!(
                    "INSERT INTO subscriptions \
                     (subscription_id, author_user_id, channel_id, subscriber_ref, status_id, created_at) \
                     SELECT 202, 101, channels.channel_id, '{subscriber_ref}', \
                            subscription_statuses.status_id, CURRENT_TIMESTAMP \
                     FROM channels CROSS JOIN subscription_statuses \
                     WHERE channels.name = 'local' AND subscription_statuses.name = 'active'"
                ))
                .await
                .unwrap();
            self.pool
                .execute(
                    "INSERT INTO audiences (audience_id, author_user_id, name, created_at) \
                     VALUES (303, 101, 'migration-audience', CURRENT_TIMESTAMP)",
                )
                .await
                .unwrap();
            self.pool
                .execute(
                    "INSERT INTO audience_members (audience_id, subscription_id, author_user_id) \
                     VALUES (303, 202, 101)",
                )
                .await
                .unwrap();
        }
    }

    #[apply(backends)]
    #[tokio::test]
    async fn migration_0027_backfills_legacy_post_media_origins_from_rendered_html(
        #[case] backend: Backend,
    ) {
        let db = MigrationDatabase::new(backend).await;
        db.migrate_to(26).await.unwrap();
        db.seed_legacy_post_media().await;

        db.migrate_current().await.unwrap();
        db.backfill_post_media_references().await;

        assert_eq!(
            db.pool
                .scalar_i64("SELECT MAX(version) FROM _sqlx_migrations")
                .await
                .unwrap(),
            27
        );
        assert_eq!(
            db.pool
                .scalar_i64("SELECT COUNT(*) FROM post_media WHERE reference_kind = 'legacy'")
                .await
                .unwrap(),
            0,
            "startup backfill must replace every pre-provenance row"
        );
        let references = db.post_media_references().await;
        assert_eq!(references.len(), 6);
        assert!(references.iter().any(|(_, _, filename, kind, form)| {
            filename == "relative.jpg" && kind == "local" && form.ends_with("/relative.jpg")
        }));
        assert!(references.iter().any(|(_, _, filename, kind, form)| {
            filename == "same.jpg" && kind == "absolute" && form.starts_with("https://example.com/")
        }));
        assert!(references.iter().any(|(_, _, filename, kind, form)| {
            filename == "scheme.jpg"
                && kind == "scheme_relative"
                && form.starts_with("//example.com/")
        }));
        assert_eq!(
            references
                .iter()
                .filter(|(_, _, filename, _, _)| filename == "overlap.jpg")
                .count(),
            2,
            "absolute and scheme-relative spellings remain distinct exact rows"
        );
    }

    #[apply(backends)]
    #[tokio::test]
    async fn migration_0026_upgrades_valid_subscription_graph_without_losing_schema_contracts(
        #[case] backend: Backend,
    ) {
        let db = MigrationDatabase::new(backend).await;
        db.migrate_to(25).await.unwrap();
        db.seed_subscription_graph("opaque-ref").await;

        db.migrate_to(26).await.unwrap();

        assert_eq!(
            db.pool
                .scalar_i64("SELECT MAX(version) FROM _sqlx_migrations")
                .await
                .unwrap(),
            26
        );
        assert_eq!(
            db.pool
                .scalar_i64(
                    "SELECT COUNT(*) FROM subscriptions \
                     WHERE subscription_id = 202 AND author_user_id = 101 \
                       AND subscriber_ref = 'opaque-ref' AND created_at IS NOT NULL",
                )
                .await
                .unwrap(),
            1,
            "the rebuild must preserve the subscription ID and stored values"
        );
        assert_eq!(
            db.pool
                .scalar_i64(
                    "SELECT COUNT(*) FROM audience_members \
                     WHERE audience_id = 303 AND subscription_id = 202 AND author_user_id = 101",
                )
                .await
                .unwrap(),
            1,
            "the rebuild must preserve dependent audience membership"
        );

        let empty_error = db
            .pool
            .execute(
                "INSERT INTO subscriptions \
                 (subscription_id, author_user_id, channel_id, subscriber_ref, status_id) \
                 SELECT 203, 101, channels.channel_id, '', subscription_statuses.status_id \
                 FROM channels CROSS JOIN subscription_statuses \
                 WHERE channels.name = 'local' AND subscription_statuses.name = 'active'",
            )
            .await
            .expect_err("migration 0026 must reject a zero-length subscriber reference");
        assert!(
            empty_error
                .as_database_error()
                .is_some_and(sqlx::error::DatabaseError::is_check_violation)
        );
        let null_error = db
            .pool
            .execute(
                "INSERT INTO subscriptions \
                 (subscription_id, author_user_id, channel_id, subscriber_ref, status_id) \
                 SELECT 203, 101, channels.channel_id, NULL, subscription_statuses.status_id \
                 FROM channels CROSS JOIN subscription_statuses \
                 WHERE channels.name = 'local' AND subscription_statuses.name = 'active'",
            )
            .await
            .expect_err("migration 0026 must retain subscriber_ref NOT NULL");
        assert!(matches!(
            null_error
                .as_database_error()
                .map(sqlx::error::DatabaseError::kind),
            Some(sqlx::error::ErrorKind::NotNullViolation)
        ));

        db.pool
            .execute(
                "INSERT INTO subscriptions \
                 (subscription_id, author_user_id, channel_id, subscriber_ref, status_id) \
                 SELECT 203, 101, channels.channel_id, '   ', subscription_statuses.status_id \
                 FROM channels CROSS JOIN subscription_statuses \
                 WHERE channels.name = 'local' AND subscription_statuses.name = 'active'",
            )
            .await
            .unwrap();
        assert_eq!(
            db.pool
                .scalar_i64(
                    "SELECT COUNT(*) FROM subscriptions \
                     WHERE subscription_id = 203 AND subscriber_ref = '   ' \
                       AND created_at IS NOT NULL",
                )
                .await
                .unwrap(),
            1,
            "the portable schema subset rejects only zero length and retains the timestamp default"
        );

        let duplicate_error = db
            .pool
            .execute(
                "INSERT INTO subscriptions \
                 (subscription_id, author_user_id, channel_id, subscriber_ref, status_id) \
                 SELECT 204, 101, channels.channel_id, 'opaque-ref', subscription_statuses.status_id \
                 FROM channels CROSS JOIN subscription_statuses \
                 WHERE channels.name = 'local' AND subscription_statuses.name = 'active'",
            )
            .await
            .expect_err("the identity UNIQUE constraint must survive migration 0026");
        assert!(
            duplicate_error
                .as_database_error()
                .is_some_and(sqlx::error::DatabaseError::is_unique_violation)
        );

        let foreign_key_error = db
            .pool
            .execute(
                "INSERT INTO subscriptions \
                 (subscription_id, author_user_id, channel_id, subscriber_ref, status_id) \
                 VALUES (204, 999, 999, 'missing-parents', 999)",
            )
            .await
            .expect_err("subscription foreign keys must survive migration 0026");
        assert!(
            foreign_key_error
                .as_database_error()
                .is_some_and(sqlx::error::DatabaseError::is_foreign_key_violation)
        );

        match backend {
            Backend::Sqlite => {
                assert_eq!(
                    db.pool
                        .scalar_i64(
                            "SELECT COUNT(*) FROM pragma_index_list('subscriptions') \
                             WHERE name = 'idx_subscriptions_author_status'",
                        )
                        .await
                        .unwrap(),
                    1
                );
                assert_eq!(
                    db.pool
                        .scalar_i64(
                            "SELECT COUNT(*) FROM pragma_index_list('subscriptions') \
                             WHERE origin = 'u'",
                        )
                        .await
                        .unwrap(),
                    2,
                    "both subscription UNIQUE constraints must survive the rebuild"
                );
                assert_eq!(
                    db.pool
                        .scalar_i64("SELECT COUNT(*) FROM pragma_foreign_key_list('subscriptions')")
                        .await
                        .unwrap(),
                    3
                );
            }
            Backend::Postgres => {
                assert_eq!(
                    db.pool
                        .scalar_i64(
                            "SELECT COUNT(*) FROM pg_indexes \
                             WHERE schemaname = 'public' AND tablename = 'subscriptions' \
                               AND indexname = 'idx_subscriptions_author_status'",
                        )
                        .await
                        .unwrap(),
                    1
                );
                assert_eq!(
                    db.pool
                        .scalar_i64(
                            "SELECT COUNT(*) FROM pg_constraint \
                             WHERE conrelid = 'subscriptions'::regclass AND contype = 'u'",
                        )
                        .await
                        .unwrap(),
                    2
                );
                assert_eq!(
                    db.pool
                        .scalar_i64(
                            "SELECT COUNT(*) FROM pg_constraint \
                             WHERE conrelid = 'subscriptions'::regclass AND contype = 'f'",
                        )
                        .await
                        .unwrap(),
                    3
                );
            }
        }
    }

    #[apply(backends)]
    #[tokio::test]
    async fn migration_0026_rejects_existing_empty_ref_without_mutating_dependency_graph(
        #[case] backend: Backend,
    ) {
        let db = MigrationDatabase::new(backend).await;
        db.migrate_to(25).await.unwrap();
        db.seed_subscription_graph("").await;

        db.migrate_to(26)
            .await
            .expect_err("an existing empty subscriber reference must abort the migration");

        assert_eq!(
            db.pool
                .scalar_i64("SELECT MAX(version) FROM _sqlx_migrations")
                .await
                .unwrap(),
            25,
            "the failed migration must not be recorded"
        );
        assert_eq!(
            db.pool
                .scalar_i64(
                    "SELECT COUNT(*) FROM subscriptions \
                     WHERE subscription_id = 202 AND author_user_id = 101 AND subscriber_ref = ''",
                )
                .await
                .unwrap(),
            1,
            "the invalid pre-upgrade subscription must remain untouched for operator repair"
        );
        assert_eq!(
            db.pool
                .scalar_i64(
                    "SELECT COUNT(*) FROM audience_members \
                     WHERE audience_id = 303 AND subscription_id = 202 AND author_user_id = 101",
                )
                .await
                .unwrap(),
            1,
            "a failed upgrade must not silently remove dependent audience membership"
        );
        let check_count = match backend {
            Backend::Sqlite => db
                .pool
                .scalar_i64(
                    "SELECT COUNT(*) FROM sqlite_master \
                         WHERE type = 'table' AND name = 'subscriptions' \
                           AND instr(sql, 'subscriptions_subscriber_ref_nonempty') > 0",
                )
                .await
                .unwrap(),
            Backend::Postgres => db
                .pool
                .scalar_i64(
                    "SELECT COUNT(*) FROM pg_constraint \
                         WHERE conrelid = 'subscriptions'::regclass \
                           AND conname = 'subscriptions_subscriber_ref_nonempty'",
                )
                .await
                .unwrap(),
        };
        assert_eq!(
            check_count, 0,
            "the failed migration must leave the version-25 schema in place"
        );
    }
}
