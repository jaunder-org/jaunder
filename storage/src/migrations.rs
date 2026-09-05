//! Portable migration-contract coverage for upgrades that need to inspect both
//! the pre-migration and current schema on fresh databases.

#[cfg(test)]
mod tests {
    use std::borrow::Cow;

    use crate::DbConnectOptions;
    use crate::posts::media;
    use crate::sql::QueryStorageExt;
    use crate::test_support::{
        Backend, CloseablePool, PostgresDbGuard, PostgresTestConfig, backends, sqlite_url,
        unique_postgres_url,
    };
    use common::visibility::SubscriberRef;

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
                    let config = PostgresTestConfig::from_env();
                    let (options, guard) = unique_postgres_url(&config).await;
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
                CloseablePool::Sqlite(pool) => {
                    media::backfill_post_media_references(pool)
                        .await
                        .expect("startup backfill succeeds");
                }
                CloseablePool::Postgres(pool) => {
                    media::backfill_post_media_references(pool)
                        .await
                        .expect("startup backfill succeeds");
                }
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

        async fn seed_subscription_graph(&self, subscriber_ref: Option<&SubscriberRef>) {
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
            if let Some(subscriber_ref) = subscriber_ref {
                crate::with_closeable_pool!(&self.pool, pool, {
                    sqlx::query(
                        "INSERT INTO subscriptions \
                         (subscription_id, author_user_id, channel_id, subscriber_ref, status_id, created_at) \
                         SELECT 202, 101, channels.channel_id, $1, \
                                subscription_statuses.status_id, CURRENT_TIMESTAMP \
                         FROM channels CROSS JOIN subscription_statuses \
                         WHERE channels.name = 'local' AND subscription_statuses.name = 'active'",
                    )
                    .bind_storage(subscriber_ref)
                    .execute(pool)
                    .await
                    .map(|_| ())
                })
                .unwrap();
            } else {
                self.pool
                    .execute(
                        "INSERT INTO subscriptions \
                         (subscription_id, author_user_id, channel_id, subscriber_ref, status_id, created_at) \
                         SELECT 202, 101, channels.channel_id, '', \
                                subscription_statuses.status_id, CURRENT_TIMESTAMP \
                         FROM channels CROSS JOIN subscription_statuses \
                         WHERE channels.name = 'local' AND subscription_statuses.name = 'active'",
                    )
                    .await
                    .unwrap();
            }
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
            32
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
    async fn migration_0032_invalidates_pre_fingerprint_feed_cache_rows(#[case] backend: Backend) {
        let db = MigrationDatabase::new(backend).await;
        db.migrate_to(31).await.unwrap();
        db.pool
            .execute(
                "INSERT INTO feed_cache \
                 (feed_url, body, etag, content_type, updated_at, generated_at) VALUES \
                 ('/feed.rss', '<rss/>', '\"legacy\"', 'application/rss+xml; charset=utf-8', \
                 CURRENT_TIMESTAMP, CURRENT_TIMESTAMP)",
            )
            .await
            .unwrap();

        db.migrate_current().await.unwrap();

        assert_eq!(
            db.pool
                .scalar_i64("SELECT COUNT(*) FROM feed_cache")
                .await
                .unwrap(),
            0,
            "legacy cache rows cannot establish semantic identity"
        );
    }

    #[apply(backends)]
    #[tokio::test]
    async fn migration_0028_preserves_complete_revision_children_and_exact_media_subjects(
        #[case] backend: Backend,
    ) {
        let db = MigrationDatabase::new(backend).await;
        db.migrate_current().await.unwrap();
        db.pool
            .execute(
                "INSERT INTO users (username, password_hash, created_at) \
                 VALUES ('revision-author', 'hash', CURRENT_TIMESTAMP)",
            )
            .await
            .unwrap();
        db.pool
            .execute(
                "INSERT INTO posts \
                 (user_id, title, slug, body, format, rendered_html, created_at, updated_at) \
                 VALUES ((SELECT user_id FROM users WHERE username = 'revision-author'), \
                 NULL, 'revision-post', 'body', 'markdown', '<p>body</p>', \
                 CURRENT_TIMESTAMP, CURRENT_TIMESTAMP)",
            )
            .await
            .unwrap();
        db.pool
            .execute(
                "INSERT INTO post_revisions \
                 (post_id, user_id, title, slug, body, format, rendered_html, summary, \
                  created_at, updated_at, published_at, deleted_at) \
                 SELECT post_id, user_id, NULL, slug, body, format, rendered_html, NULL, \
                        created_at, updated_at, NULL, NULL \
                 FROM posts WHERE slug = 'revision-post'",
            )
            .await
            .unwrap();
        db.pool
            .execute(
                "INSERT INTO post_revision_tags (revision_id, tag_slug, tag_display) \
                 SELECT revision_id, 'immutable-revision-tag', 'Immutable revision tag' \
                 FROM post_revisions",
            )
            .await
            .unwrap();
        db.pool
            .execute(
                "INSERT INTO post_revision_audiences (revision_id, target_kind, audience_id) \
                 SELECT revision_id, 'public', NULL FROM post_revisions",
            )
            .await
            .unwrap();
        db.pool
            .execute(
                "INSERT INTO post_revision_audiences (revision_id, target_kind, audience_id) \
                 SELECT revision_id, 'named', 999999 FROM post_revisions",
            )
            .await
            .unwrap();
        let duplicate_audience_error = db
            .pool
            .execute(
                "INSERT INTO post_revision_audiences (revision_id, target_kind, audience_id) \
                 SELECT revision_id, 'public', NULL FROM post_revisions",
            )
            .await
            .expect_err("a built-in audience target occurs at most once per revision");
        assert!(duplicate_audience_error.as_database_error().is_some());
        db.pool
            .execute(
                "INSERT INTO post_media \
                 (post_id, subject_kind, revision_id, source, sha256, filename, reference_kind, reference_form) \
                 SELECT post_id, 'revision', revision_id, 'upload', \
                 'e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855', \
                 'revision.jpg', 'local', '/media/upload/revision.jpg' \
                 FROM post_revisions",
            )
            .await
            .unwrap();

        assert_eq!(
            db.pool
                .scalar_i64(
                    "SELECT COUNT(*) FROM post_revisions WHERE summary IS NULL \
                 AND published_at IS NULL AND deleted_at IS NULL AND captured_at IS NOT NULL"
                )
                .await
                .unwrap(),
            1
        );
        assert_eq!(
            db.pool
                .scalar_i64("SELECT COUNT(*) FROM post_revision_tags")
                .await
                .unwrap(),
            1
        );
        assert_eq!(
            db.pool
                .scalar_i64("SELECT COUNT(*) FROM post_revision_audiences")
                .await
                .unwrap(),
            2
        );
        let duplicate_current = db.pool.execute(
            "INSERT INTO post_media \
             (post_id, subject_kind, revision_id, source, sha256, filename, reference_kind, reference_form) \
             SELECT post_id, 'current', 0, 'upload', \
             'e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855', \
             'current.jpg', 'local', '/media/upload/current.jpg' FROM posts",
        );
        duplicate_current.await.unwrap();
        let duplicate_error = db
            .pool
            .execute(
                "INSERT INTO post_media \
                 (post_id, subject_kind, revision_id, source, sha256, filename, reference_kind, reference_form) \
                 SELECT post_id, 'current', 0, 'upload', \
                 'e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855', \
                 'current.jpg', 'local', '/media/upload/current.jpg' FROM posts",
            )
            .await
            .expect_err("one exact current media subject is unique");
        assert!(duplicate_error.as_database_error().is_some());
        db.pool
            .execute(
                "INSERT INTO posts \
                 (user_id, title, slug, body, format, rendered_html, created_at, updated_at) \
                 VALUES ((SELECT user_id FROM users WHERE username = 'revision-author'), \
                 NULL, 'revision-other-post', 'body', 'markdown', '<p>body</p>', \
                 CURRENT_TIMESTAMP, CURRENT_TIMESTAMP)",
            )
            .await
            .unwrap();
        let cross_post_error = db
            .pool
            .execute(
                "INSERT INTO post_media \
                 (post_id, subject_kind, revision_id, source, sha256, filename, reference_kind, reference_form) \
                 SELECT p.post_id, 'revision', r.revision_id, 'upload', \
                 'e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855', \
                 'wrong-post.jpg', 'local', '/media/upload/wrong-post.jpg' \
                 FROM post_revisions r CROSS JOIN posts p WHERE p.slug = 'revision-other-post'",
            )
            .await
            .expect_err("a revision media subject must name its revision's post");
        assert!(cross_post_error.as_database_error().is_some());
    }

    #[apply(backends)]
    #[tokio::test]
    async fn migration_0026_upgrades_valid_subscription_graph_without_losing_schema_contracts(
        #[case] backend: Backend,
    ) {
        let db = MigrationDatabase::new(backend).await;
        db.migrate_to(25).await.unwrap();
        let subscriber_ref: SubscriberRef = "opaque-ref".parse().unwrap();
        db.seed_subscription_graph(Some(&subscriber_ref)).await;

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
        db.seed_subscription_graph(None).await;

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

    #[apply(backends)]
    #[tokio::test]
    async fn migration_0029_backfills_feed_terminal_instants_and_adds_retention_indexes(
        #[case] backend: Backend,
    ) {
        let db = MigrationDatabase::new(backend).await;
        db.migrate_to(28).await.unwrap();
        db.pool
            .execute(
                "INSERT INTO feed_events
                    (feed_url, status, created_at, pinged_at, next_attempt_at, claimed_at)
                 VALUES
                 ('/~done-known/feed.rss', 'done', '2026-01-01T00:00:00Z',
                  '2026-01-02T00:00:00Z', '2026-01-01T00:00:00Z', NULL),
                 ('/~done-fallback/feed.rss', 'done', '2026-01-03T00:00:00Z',
                  NULL, '2026-01-03T00:00:00Z', NULL),
                 ('/~failed/feed.rss', 'failed', '2026-01-04T00:00:00Z',
                  NULL, '2026-01-04T00:00:00Z', '2026-01-06T00:00:00Z'),
                 ('/~pending/feed.rss', 'pending', '2026-01-05T00:00:00Z',
                  NULL, '2026-01-05T00:00:00Z', NULL)",
            )
            .await
            .unwrap();

        db.migrate_current().await.unwrap();

        assert_eq!(
            db.pool
                .scalar_i64("SELECT MAX(version) FROM _sqlx_migrations")
                .await
                .unwrap(),
            32
        );
        assert_eq!(
            db.pool
                .scalar_i64(
                    "SELECT COUNT(*) FROM feed_events
                     WHERE status = 'done' AND pinged_at IS NOT NULL
                       AND terminal_at = pinged_at",
                )
                .await
                .unwrap(),
            1,
            "a known completion instant must remain the retention anchor"
        );
        assert_eq!(
            db.pool
                .scalar_i64(
                    "SELECT COUNT(*) FROM feed_events
                     WHERE feed_url = '/~done-fallback/feed.rss'
                       AND terminal_at = created_at",
                )
                .await
                .unwrap(),
            1,
            "a legacy completion without pinged_at must retain its original age"
        );
        assert_eq!(
            db.pool
                .scalar_i64(
                    "SELECT COUNT(*) FROM feed_events
                     WHERE feed_url = '/~failed/feed.rss'
                       AND terminal_at = claimed_at",
                )
                .await
                .unwrap(),
            1,
            "a legacy exhaustion must retain its final-attempt age"
        );
        assert_eq!(
            db.pool
                .scalar_i64(
                    "SELECT COUNT(*) FROM feed_events
                     WHERE status IN ('done', 'failed') AND terminal_at IS NOT NULL",
                )
                .await
                .unwrap(),
            3,
            "every legacy terminal row needs a deterministic retention anchor"
        );
        assert_eq!(
            db.pool
                .scalar_i64(
                    "SELECT COUNT(*) FROM feed_events
                     WHERE status = 'pending' AND terminal_at IS NULL",
                )
                .await
                .unwrap(),
            1,
            "non-terminal rows must not acquire a terminal instant"
        );

        let retention_index_count = match backend {
            Backend::Sqlite => db
                .pool
                .scalar_i64(
                    "SELECT COUNT(*) FROM sqlite_master
                         WHERE type = 'index' AND name IN (
                           'idx_idempotency_keys_created_at',
                           'idx_invites_expires_at',
                           'idx_invites_used_at',
                           'idx_email_verifications_expires_at',
                           'idx_email_verifications_used_at',
                           'idx_password_resets_expires_at',
                           'idx_password_resets_used_at',
                           'idx_feed_events_terminal_retention'
                         )",
                )
                .await
                .unwrap(),
            Backend::Postgres => db
                .pool
                .scalar_i64(
                    "SELECT COUNT(*) FROM pg_indexes
                         WHERE schemaname = 'public' AND indexname IN (
                           'idx_idempotency_keys_created_at',
                           'idx_invites_expires_at',
                           'idx_invites_used_at',
                           'idx_email_verifications_expires_at',
                           'idx_email_verifications_used_at',
                           'idx_password_resets_expires_at',
                           'idx_password_resets_used_at',
                           'idx_feed_events_terminal_retention'
                         )",
                )
                .await
                .unwrap(),
        };
        assert_eq!(retention_index_count, 8);
    }

    #[apply(backends)]
    #[tokio::test]
    async fn migration_0030_maps_legacy_feed_event_attempts_to_one_phase(#[case] backend: Backend) {
        let db = MigrationDatabase::new(backend).await;
        db.migrate_to(29).await.unwrap();
        db.pool.execute(
            "INSERT INTO feed_events \
             (feed_url, status, attempts, last_error, next_attempt_at, created_at, regenerated_at, terminal_at) VALUES \
             ('/pending.rss', 'pending', 3, 'pending error', CURRENT_TIMESTAMP, CURRENT_TIMESTAMP, NULL, NULL), \
             ('/regeneration.rss', 'failed', 4, 'regeneration error', CURRENT_TIMESTAMP, CURRENT_TIMESTAMP, NULL, CURRENT_TIMESTAMP), \
             ('/publication.rss', 'failed', 5, 'publication error', CURRENT_TIMESTAMP, CURRENT_TIMESTAMP, CURRENT_TIMESTAMP, CURRENT_TIMESTAMP)",
        ).await.unwrap();
        db.migrate_current().await.unwrap();

        assert_eq!(
            db.pool
                .scalar_i64(
                    "SELECT COUNT(*) FROM feed_events \
                 WHERE phase = 'regeneration' AND regeneration_attempts = 3 \
                   AND publication_attempts = 0 AND status = 'pending'",
                )
                .await
                .unwrap(),
            1,
        );
        assert_eq!(
            db.pool
                .scalar_i64(
                    "SELECT COUNT(*) FROM feed_events \
                 WHERE phase = 'regeneration' AND regeneration_attempts = 4 \
                   AND regeneration_diagnostic = 'regeneration error' \
                   AND publication_attempts = 0 AND publication_diagnostic IS NULL",
                )
                .await
                .unwrap(),
            1,
        );
        assert_eq!(
            db.pool
                .scalar_i64(
                    "SELECT COUNT(*) FROM feed_events \
                 WHERE phase = 'publication' AND publication_attempts = 5 \
                   AND publication_diagnostic = 'publication error' \
                   AND regeneration_attempts = 0 AND regeneration_diagnostic IS NULL",
                )
                .await
                .unwrap(),
            1,
        );
    }
}
