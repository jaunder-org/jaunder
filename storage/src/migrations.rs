//! Portable migration-contract coverage for upgrades that need to inspect both
//! the pre-migration and current schema on fresh databases.

#[cfg(test)]
mod tests {
    use std::borrow::Cow;

    use crate::DbConnectOptions;
    use crate::code_migrations;
    use crate::posts::PostDialect;
    use crate::posts::search::{
        PostMutationVersion, PostSearchBackfillCandidate, StoredPostSearchText,
        backfill_post_search_projections,
    };
    use crate::sql::{QueryStorageExt, RowCount};
    use crate::subscriptions::CorruptSubscriberRef;
    use crate::test_support::{
        Backend, CloseablePool, PostgresDbGuard, PostgresTestConfig, backends, sqlite_url,
        unique_postgres_url,
    };
    use common::ids::PostId;
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
        options: DbConnectOptions,
        pool: CloseablePool,
        _sqlite: Option<TempDir>,
        _postgres: Option<PostgresDbGuard>,
    }

    impl MigrationDatabase {
        async fn new(backend: Backend) -> Self {
            match backend {
                Backend::Sqlite => {
                    let base = TempDir::new().unwrap();
                    let options = sqlite_url(&base);
                    let DbConnectOptions::Sqlite(sqlite_options) = &options else {
                        unreachable!("sqlite_url always yields SQLite options")
                    };
                    let pool =
                        SqlitePool::connect_with(sqlite_options.clone().create_if_missing(true))
                            .await
                            .unwrap();
                    Self {
                        options,
                        pool: CloseablePool::Sqlite(pool),
                        _sqlite: Some(base),
                        _postgres: None,
                    }
                }
                Backend::Postgres => {
                    let config = PostgresTestConfig::from_env();
                    let (options, guard) = unique_postgres_url(&config).await;
                    let DbConnectOptions::Postgres {
                        options: pg_options,
                        ..
                    } = &options
                    else {
                        unreachable!("unique_postgres_url always yields PostgreSQL options")
                    };
                    let pool = PgPool::connect_with(pg_options.clone()).await.unwrap();
                    Self {
                        options,
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

        async fn drain_pending_code_migrations(&self) {
            self.drain_pending(&|| Ok(()))
                .await
                .expect("offline migration succeeds");
        }

        async fn drain_pending(
            &self,
            authorize: &(dyn Fn() -> sqlx::Result<()> + Sync),
        ) -> sqlx::Result<()> {
            match &self.pool {
                CloseablePool::Sqlite(pool) => {
                    code_migrations::drain_pending(pool, authorize).await
                }
                CloseablePool::Postgres(pool) => {
                    code_migrations::drain_pending(pool, authorize).await
                }
            }
        }

        async fn backfill_post_search_projections(&self) {
            match &self.pool {
                CloseablePool::Sqlite(pool) => backfill_post_search_projections(pool)
                    .await
                    .expect("startup search backfill succeeds"),
                CloseablePool::Postgres(pool) => backfill_post_search_projections(pool)
                    .await
                    .expect("startup search backfill succeeds"),
            }
        }

        async fn apply_post_search_candidates(&self, candidates: &[PostSearchBackfillCandidate]) {
            match &self.pool {
                CloseablePool::Sqlite(pool) => {
                    <sqlx::Sqlite as PostDialect>::apply_post_search_backfill(pool, candidates)
                        .await
                        .expect("SQLite search batch applies");
                }
                CloseablePool::Postgres(pool) => {
                    <sqlx::Postgres as PostDialect>::apply_post_search_backfill(pool, candidates)
                        .await
                        .expect("PostgreSQL search batch applies");
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
    async fn migration_0044_backfills_legacy_post_media_origins_from_rendered_html(
        #[case] backend: Backend,
    ) {
        let db = MigrationDatabase::new(backend).await;
        db.migrate_to(26).await.unwrap();
        db.seed_legacy_post_media().await;

        db.migrate_to(44).await.unwrap();
        db.drain_pending_code_migrations().await;

        assert_eq!(
            db.pool
                .scalar_i64("SELECT MAX(version) FROM _sqlx_migrations")
                .await
                .unwrap(),
            44
        );
        assert_eq!(
            db.pool
                .scalar_i64("SELECT COUNT(*) FROM pending_code_migrations")
                .await
                .unwrap(),
            0
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
    async fn pending_code_migrations_are_ordered_reusable_and_reject_unknown_operations(
        #[case] backend: Backend,
    ) {
        use std::sync::atomic::{AtomicUsize, Ordering};

        let db = MigrationDatabase::new(backend).await;
        db.migrate_to(44).await.unwrap();
        db.pool
            .execute("INSERT INTO pending_code_migrations (operation) VALUES ('not_registered')")
            .await
            .unwrap();
        db.pool
            .execute("INSERT INTO pending_code_migrations (operation) VALUES ('backfill_post_media_references')")
            .await
            .unwrap();
        let authorized = AtomicUsize::new(0);
        let error = db
            .drain_pending(&|| {
                authorized.fetch_add(1, Ordering::SeqCst);
                Ok(())
            })
            .await
            .expect_err("unknown operation stops before the following operation");
        assert!(error.to_string().contains("not_registered"));
        assert_eq!(authorized.load(Ordering::SeqCst), 2);
        assert_eq!(
            db.pool
                .scalar_i64("SELECT COUNT(*) FROM pending_code_migrations")
                .await
                .unwrap(),
            2,
            "the earlier migration committed and the unknown row remains"
        );
        db.pool
            .execute("DELETE FROM pending_code_migrations WHERE operation = 'not_registered'")
            .await
            .unwrap();
        db.drain_pending_code_migrations().await;
        assert_eq!(
            db.pool
                .scalar_i64("SELECT COUNT(*) FROM pending_code_migrations")
                .await
                .unwrap(),
            0
        );
        db.pool
            .execute("INSERT INTO pending_code_migrations (operation) VALUES ('backfill_post_media_references')")
            .await
            .unwrap();
        db.drain_pending_code_migrations().await;
        assert_eq!(
            db.pool
                .scalar_i64("SELECT COUNT(*) FROM pending_code_migrations")
                .await
                .unwrap(),
            0,
            "a later SQLx migration can enqueue the same operation again"
        );
        let error = db
            .drain_pending(&|| unreachable!("a drained queue must not request authorization"))
            .await;
        assert!(error.is_ok(), "no pending row needs no authorization");
    }

    #[apply(backends)]
    #[tokio::test]
    async fn pending_code_migration_rolls_back_work_if_queue_delete_fails(
        #[case] backend: Backend,
    ) {
        let db = MigrationDatabase::new(backend).await;
        db.migrate_to(26).await.unwrap();
        db.seed_legacy_post_media().await;
        db.migrate_to(44).await.unwrap();
        let legacy_before = db
            .pool
            .scalar_i64("SELECT COUNT(*) FROM post_media WHERE reference_kind = 'legacy'")
            .await
            .unwrap();
        match backend {
            Backend::Sqlite => {
                db.pool.execute(
                    "CREATE TRIGGER reject_queue_delete BEFORE DELETE ON pending_code_migrations
                     BEGIN SELECT RAISE(ABORT, 'injected queue delete failure'); END",
                ).await.unwrap();
            }
            Backend::Postgres => {
                db.pool.execute(
                    "CREATE FUNCTION reject_queue_delete() RETURNS trigger LANGUAGE plpgsql AS $$
                     BEGIN RAISE EXCEPTION 'injected queue delete failure'; END; $$",
                ).await.unwrap();
                db.pool.execute(
                    "CREATE TRIGGER reject_queue_delete BEFORE DELETE ON pending_code_migrations
                     FOR EACH ROW EXECUTE FUNCTION reject_queue_delete()",
                ).await.unwrap();
            }
        }
        let error = db
            .drain_pending(&|| Ok(()))
            .await
            .expect_err("injected delete fails");
        assert!(error.to_string().contains("injected queue delete failure"));
        assert_eq!(
            db.pool
                .scalar_i64("SELECT COUNT(*) FROM pending_code_migrations")
                .await
                .unwrap(),
            1
        );
        assert_eq!(
            db.pool
                .scalar_i64("SELECT COUNT(*) FROM post_media WHERE reference_kind = 'legacy'")
                .await
                .unwrap(),
            legacy_before,
            "the derived changes roll back with the pending row"
        );
        let drop_trigger = match backend {
            Backend::Sqlite => "DROP TRIGGER reject_queue_delete",
            Backend::Postgres => "DROP TRIGGER reject_queue_delete ON pending_code_migrations",
        };
        db.pool.execute(drop_trigger).await.unwrap();
        db.drain_pending_code_migrations().await;
        assert_eq!(
            db.pool
                .scalar_i64("SELECT COUNT(*) FROM pending_code_migrations")
                .await
                .unwrap(),
            0
        );
        assert_eq!(
            db.pool
                .scalar_i64("SELECT COUNT(*) FROM post_media WHERE reference_kind = 'legacy'")
                .await
                .unwrap(),
            0
        );
    }

    #[apply(backends)]
    #[tokio::test]
    async fn sqlx_failure_returns_no_opened_storage_or_rust_operation(#[case] backend: Backend) {
        let db = MigrationDatabase::new(backend).await;
        db.migrate_to(43).await.unwrap();
        match backend {
            Backend::Sqlite => {
                db.pool
                    .execute(
                        "CREATE TRIGGER reject_enqueue BEFORE INSERT ON pending_code_migrations
                     BEGIN SELECT RAISE(ABORT, 'injected SQLx failure'); END",
                    )
                    .await
                    .unwrap();
            }
            Backend::Postgres => {
                db.pool
                    .execute(
                        "CREATE FUNCTION reject_enqueue() RETURNS trigger LANGUAGE plpgsql AS $$
                     BEGIN RAISE EXCEPTION 'injected SQLx failure'; END; $$",
                    )
                    .await
                    .unwrap();
                db.pool
                    .execute(
                        "CREATE TRIGGER reject_enqueue BEFORE INSERT ON pending_code_migrations
                     FOR EACH ROW EXECUTE FUNCTION reject_enqueue()",
                    )
                    .await
                    .unwrap();
            }
        }
        let open =
            crate::open_existing_database(&db.options, &crate::StorageRuntimeConfig::default())
                .await;
        assert!(
            open.is_err(),
            "SQLx failure must prevent handing out a storage factory"
        );
        assert_eq!(
            db.pool
                .scalar_i64("SELECT MAX(version) FROM _sqlx_migrations")
                .await
                .unwrap(),
            43
        );
        assert_eq!(
            db.pool
                .scalar_i64("SELECT COUNT(*) FROM pending_code_migrations")
                .await
                .unwrap(),
            0,
            "the failing SQLx migration did not enqueue any Rust work"
        );
        let drop_trigger = match backend {
            Backend::Sqlite => "DROP TRIGGER reject_enqueue",
            Backend::Postgres => "DROP TRIGGER reject_enqueue ON pending_code_migrations",
        };
        db.pool.execute(drop_trigger).await.unwrap();
        crate::open_existing_database(&db.options, &crate::StorageRuntimeConfig::default())
            .await
            .expect("next open completes SQLx and drains Rust work");
        assert_eq!(
            db.pool
                .scalar_i64("SELECT COUNT(*) FROM pending_code_migrations")
                .await
                .unwrap(),
            0
        );
    }

    #[apply(backends)]
    #[tokio::test]
    async fn migration_0047_rebuilds_existing_code_blocks_without_author_edits(
        #[case] backend: Backend,
    ) {
        let db = MigrationDatabase::new(backend).await;
        db.migrate_to(46).await.unwrap();
        db.drain_pending_code_migrations().await;
        let user = match backend {
            Backend::Sqlite => {
                "INSERT INTO users (user_id, username, password_hash, created_at) VALUES (404, 'code-author', 'hash', CURRENT_TIMESTAMP)"
            }
            Backend::Postgres => {
                "INSERT INTO users (user_id, username, password_hash, created_at) OVERRIDING SYSTEM VALUE VALUES (404, 'code-author', 'hash', CURRENT_TIMESTAMP)"
            }
        };
        db.pool.execute(user).await.unwrap();
        let posts = match backend {
            Backend::Sqlite => {
                r#"INSERT INTO posts (post_id, user_id, slug, body, format, rendered_html, created_at, updated_at, published_at, deleted_at) VALUES
                (505, 404, 'old-org', '#+begin_src emacs-lisp
(message "hello")
#+end_src', 'org', '<p>old Org</p>', '2025-01-01T00:00:00Z', '2025-02-01T00:00:00Z', '2025-02-01T00:00:00Z', NULL),
                (506, 404, 'old-markdown', '```haskell
main = putStrLn "hello"
```', 'markdown', '<p>old Markdown</p>', '2025-01-01T00:00:00Z', '2025-02-01T00:00:00Z', NULL, NULL),
                (507, 404, 'old-deleted', '#+begin_src emacs-lisp
(message "gone")
#+end_src', 'org', '<p>old Deleted</p>', '2025-01-01T00:00:00Z', '2025-02-01T00:00:00Z', '2025-02-01T00:00:00Z', '2025-03-01T00:00:00Z'),
                (508, 404, 'old-html', '<p>authored</p>', 'html', '<p>old HTML</p>', '2025-01-01T00:00:00Z', '2025-02-01T00:00:00Z', NULL, NULL)"#
            }
            Backend::Postgres => {
                r#"INSERT INTO posts (post_id, user_id, slug, body, format, rendered_html, created_at, updated_at, published_at, deleted_at) OVERRIDING SYSTEM VALUE VALUES
                (505, 404, 'old-org', '#+begin_src emacs-lisp
(message "hello")
#+end_src', 'org', '<p>old Org</p>', '2025-01-01T00:00:00Z', '2025-02-01T00:00:00Z', '2025-02-01T00:00:00Z', NULL),
                (506, 404, 'old-markdown', '```haskell
main = putStrLn "hello"
```', 'markdown', '<p>old Markdown</p>', '2025-01-01T00:00:00Z', '2025-02-01T00:00:00Z', NULL, NULL),
                (507, 404, 'old-deleted', '#+begin_src emacs-lisp
(message "gone")
#+end_src', 'org', '<p>old Deleted</p>', '2025-01-01T00:00:00Z', '2025-02-01T00:00:00Z', '2025-02-01T00:00:00Z', '2025-03-01T00:00:00Z'),
                (508, 404, 'old-html', '<p>authored</p>', 'html', '<p>old HTML</p>', '2025-01-01T00:00:00Z', '2025-02-01T00:00:00Z', NULL, NULL)"#
            }
        };
        db.pool.execute(posts).await.unwrap();
        db.pool
            .execute(
                "INSERT INTO post_audiences (post_id, target_kind_id, audience_id)
             SELECT 505, kind_id, NULL FROM target_kinds WHERE name = 'public'",
            )
            .await
            .unwrap();
        db.pool.execute(
            "INSERT INTO post_revisions (post_id, user_id, slug, body, format, rendered_html, created_at, updated_at, published_at)
             SELECT post_id, user_id, slug, body, format, rendered_html, created_at, updated_at, published_at FROM posts WHERE post_id = 505",
        ).await.unwrap();
        let snapshots = "SELECT CAST(post_id AS TEXT), body, rendered_html, CAST(updated_at AS TEXT), COALESCE(CAST(deleted_at AS TEXT), '') FROM posts ORDER BY post_id";
        let before = db.pool.string_quintuples(snapshots).await.unwrap();
        let revisions =
            "SELECT body, rendered_html, CAST(updated_at AS TEXT), '', '' FROM post_revisions";
        let old_revisions = db.pool.string_quintuples(revisions).await.unwrap();

        db.migrate_current().await.unwrap();
        assert_eq!(
            db.pool
                .scalar_i64("SELECT MAX(version) FROM _sqlx_migrations")
                .await
                .unwrap(),
            47
        );
        assert_eq!(
            db.pool
                .string_quintuples("SELECT operation, '', '', '', '' FROM pending_code_migrations")
                .await
                .unwrap()[0]
                .0,
            "rebuild_rendered_posts"
        );
        db.drain_pending_code_migrations().await;

        let after = db.pool.string_quintuples(snapshots).await.unwrap();
        for (old, new) in before.iter().zip(&after) {
            assert_eq!(
                (
                    old.0.as_str(),
                    old.1.as_str(),
                    old.3.as_str(),
                    old.4.as_str()
                ),
                (
                    new.0.as_str(),
                    new.1.as_str(),
                    new.3.as_str(),
                    new.4.as_str()
                ),
                "authored body, identity, edit time and deletion remain unchanged"
            );
        }
        for post in after.iter().take(3) {
            assert!(
                post.2.contains("j-syn-"),
                "code block must be highlighted: {}",
                post.0
            );
        }
        assert_eq!(after[3].2, "<p>authored</p>");
        assert_eq!(
            db.pool.string_quintuples(revisions).await.unwrap(),
            old_revisions
        );
        assert_eq!(
            db.pool
                .scalar_i64("SELECT COUNT(*) FROM pending_code_migrations")
                .await
                .unwrap(),
            0
        );
        assert_eq!(
            db.pool
                .scalar_i64("SELECT COUNT(*) FROM feed_events")
                .await
                .unwrap(),
            6,
            "only the public Org Post queues Site and User Feeds"
        );
        assert_eq!(
            db.pool
                .scalar_i64("SELECT COUNT(*) FROM post_projection_refresh_progress WHERE id = 1 AND completed = FALSE")
                .await
                .unwrap(),
            1,
            "bounded refresh can checkpoint without duplicate changes afterward"
        );
        let factory = match &db.pool {
            CloseablePool::Sqlite(pool) => crate::StorageFactory::sqlite(pool.clone()),
            CloseablePool::Postgres(pool) => crate::StorageFactory::postgres(pool.clone()),
        };
        factory.refresh_current_post_projections().await.unwrap();
        assert_eq!(
            db.pool
                .scalar_i64("SELECT COUNT(*) FROM post_projection_refresh_progress WHERE id = 1 AND completed = TRUE")
                .await
                .unwrap(),
            1,
            "the bounded checkpoint follows the offline rebuild"
        );
        assert_eq!(
            db.pool
                .scalar_i64("SELECT COUNT(*) FROM feed_events")
                .await
                .unwrap(),
            6,
            "unchanged projections must not enqueue duplicate events"
        );
    }

    #[apply(backends)]
    #[tokio::test]
    async fn migration_0045_runs_media_repair_before_render_rebuild(#[case] backend: Backend) {
        let db = MigrationDatabase::new(backend).await;
        db.migrate_to(26).await.unwrap();
        db.seed_legacy_post_media().await;
        db.migrate_to(45).await.unwrap();
        let pending = db
            .pool
            .string_quintuples(
                "SELECT operation, '', '', '', '' FROM pending_code_migrations ORDER BY queue_id",
            )
            .await
            .unwrap();
        assert_eq!(
            pending.iter().map(|row| row.0.as_str()).collect::<Vec<_>>(),
            ["backfill_post_media_references", "rebuild_rendered_posts"]
        );
        db.drain_pending_code_migrations().await;
        assert_eq!(
            db.pool
                .scalar_i64("SELECT COUNT(*) FROM post_media WHERE subject_kind = 'current'")
                .await
                .unwrap(),
            0,
            "rebuild reconciles the backfilled references against the current authored body"
        );
        assert_eq!(
            db.pool
                .scalar_i64("SELECT COUNT(*) FROM pending_code_migrations")
                .await
                .unwrap(),
            0
        );
    }

    #[apply(backends)]
    #[tokio::test]
    async fn migration_0045_rebuilds_only_changed_current_derivatives_and_public_feeds(
        #[case] backend: Backend,
    ) {
        use common::render::PostFormat;

        let db = MigrationDatabase::new(backend).await;
        db.migrate_to(44).await.unwrap();
        db.drain_pending_code_migrations().await;
        let user = match backend {
            Backend::Sqlite => {
                "INSERT INTO users (user_id, username, password_hash, created_at) VALUES (404, 'rebuild-author', 'hash', CURRENT_TIMESTAMP)"
            }
            Backend::Postgres => {
                "INSERT INTO users (user_id, username, password_hash, created_at) OVERRIDING SYSTEM VALUE VALUES (404, 'rebuild-author', 'hash', CURRENT_TIMESTAMP)"
            }
        };
        db.pool.execute(user).await.unwrap();
        let posts = match backend {
            Backend::Sqlite => "INSERT INTO posts (post_id, user_id, title, rendered_title, slug, body, format, rendered_html, created_at, updated_at, published_at, deleted_at) VALUES
                (505, 404, 'A---B...', 'A---B...', 'rebuild-org', 'A---B...', 'org', '<p>A---B...</p>', '2025-01-01T00:00:00Z', '2025-02-01T00:00:00Z', '2025-02-01T00:00:00Z', NULL),
                (506, 404, NULL, NULL, 'rebuild-markdown', 'word', 'markdown', '<p>word</p>', '2025-01-01T00:00:00Z', '2025-02-01T00:00:00Z', NULL, NULL),
                (507, 404, NULL, NULL, 'rebuild-html', '<p>fixed</p>', 'html', '<p>fixed</p>', '2025-01-01T00:00:00Z', '2025-02-01T00:00:00Z', NULL, NULL),
                (508, 404, 'Deleted--', 'Deleted--', 'rebuild-deleted', 'Deleted---...', 'org', '<p>Deleted---...</p>', '2025-01-01T00:00:00Z', '2025-02-01T00:00:00Z', '2025-02-01T00:00:00Z', '2025-03-01T00:00:00Z')",
            Backend::Postgres => "INSERT INTO posts (post_id, user_id, title, rendered_title, slug, body, format, rendered_html, created_at, updated_at, published_at, deleted_at) OVERRIDING SYSTEM VALUE VALUES
                (505, 404, 'A---B...', 'A---B...', 'rebuild-org', 'A---B...', 'org', '<p>A---B...</p>', '2025-01-01T00:00:00Z', '2025-02-01T00:00:00Z', '2025-02-01T00:00:00Z', NULL),
                (506, 404, NULL, NULL, 'rebuild-markdown', 'word', 'markdown', '<p>word</p>', '2025-01-01T00:00:00Z', '2025-02-01T00:00:00Z', NULL, NULL),
                (507, 404, NULL, NULL, 'rebuild-html', '<p>fixed</p>', 'html', '<p>fixed</p>', '2025-01-01T00:00:00Z', '2025-02-01T00:00:00Z', NULL, NULL),
                (508, 404, 'Deleted--', 'Deleted--', 'rebuild-deleted', 'Deleted---...', 'org', '<p>Deleted---...</p>', '2025-01-01T00:00:00Z', '2025-02-01T00:00:00Z', '2025-02-01T00:00:00Z', '2025-03-01T00:00:00Z')",
        };
        db.pool.execute(posts).await.unwrap();
        db.pool
            .execute("UPDATE posts SET rendered_html = '<p>stale</p>' WHERE post_id = 506")
            .await
            .unwrap();
        let title_only = match backend {
            Backend::Sqlite => "INSERT INTO posts (post_id, user_id, title, rendered_title, slug, body, format, rendered_html, created_at, updated_at)
                 VALUES (509, 404, 'title', 'stale', 'title-only', '<p>fixed</p>', 'html', '<p>fixed</p>', '2025-01-01T00:00:00Z', '2025-02-01T00:00:00Z')",
            Backend::Postgres => "INSERT INTO posts (post_id, user_id, title, rendered_title, slug, body, format, rendered_html, created_at, updated_at)
                 OVERRIDING SYSTEM VALUE VALUES (509, 404, 'title', 'stale', 'title-only', '<p>fixed</p>', 'html', '<p>fixed</p>', '2025-01-01T00:00:00Z', '2025-02-01T00:00:00Z')",
        };
        db.pool.execute(title_only).await.unwrap();
        db.pool
            .execute(
                "INSERT INTO post_audiences (post_id, target_kind_id, audience_id)
             SELECT 505, kind_id, NULL FROM target_kinds WHERE name = 'public'",
            )
            .await
            .unwrap();
        db.pool.execute(
            "INSERT INTO post_revisions (post_id, user_id, title, rendered_title, slug, body, format, rendered_html, created_at, updated_at, published_at)
             SELECT post_id, user_id, title, rendered_title, slug, body, format, rendered_html, created_at, updated_at, published_at FROM posts WHERE post_id = 505",
        ).await.unwrap();
        db.pool
            .execute("INSERT INTO tags (tag_slug) VALUES ('topic')")
            .await
            .unwrap();
        db.pool
            .execute(
                "INSERT INTO post_tags (post_id, tag_id, tag_display)
             SELECT 505, tag_id, 'Topic' FROM tags WHERE tag_slug = 'topic'",
            )
            .await
            .unwrap();
        let before =  db.pool.string_quintuples(
            "SELECT CAST(post_id AS TEXT), COALESCE(title, ''), body, rendered_html, CAST(updated_at AS TEXT)
             FROM posts ORDER BY post_id",
        ).await.unwrap();
        let revision_before = db.pool.string_quintuples(
            "SELECT title, body, rendered_html, CAST(updated_at AS TEXT), COALESCE(rendered_title, '') FROM post_revisions",
        ).await.unwrap();
        db.pool.execute(
            "INSERT INTO feed_cache (feed_url, body, etag, content_type, representation_modified_at, generated_at, semantic_fingerprint)
             VALUES ('/feed.rss', 'stale', 'old-etag', 'application/rss+xml', '2025-02-01T00:00:00Z', '2025-02-01T00:00:00Z', 'old-fingerprint')",
        ).await.unwrap();
        db.migrate_current().await.unwrap();
        db.drain_pending_code_migrations().await;
        let after = db.pool.string_quintuples(
            "SELECT CAST(post_id AS TEXT), COALESCE(title, ''), body, rendered_html, CAST(updated_at AS TEXT)
             FROM posts ORDER BY post_id",
        ).await.unwrap();
        let org = host::render::render_post(
            Some("A---B...".parse().unwrap()),
            "A---B...".parse().unwrap(),
            PostFormat::Org,
        )
        .unwrap();
        assert_eq!(
            after[0].2, before[0].2,
            "authored Org source remains unchanged"
        );
        assert_eq!(after[0].3, org.rendered_html().as_ref());
        let current_title = db
            .pool
            .string_quintuples(
                "SELECT rendered_title, '', '', '', '' FROM posts WHERE post_id = 505",
            )
            .await
            .unwrap();
        assert_eq!(current_title[0].0, org.rendered_title().unwrap().as_ref());
        assert_eq!(after[0].4, before[0].4, "semantic edit time is unchanged");
        assert_eq!(after[1].2, before[1].2, "Markdown source is unchanged");
        assert_eq!(
            after[1].3, "<p>word</p>\n",
            "Markdown derivative is rebuilt"
        );
        assert_eq!(after[1].4, before[1].4, "Markdown edit time is unchanged");
        assert_eq!(
            after[2], before[2],
            "HTML with current rendering is untouched"
        );
        assert_ne!(after[3].3, before[3].3, "retained Deleted Post is rebuilt");
        assert_eq!(after[3].4, before[3].4);
        assert_eq!(
            after[4].3, before[4].3,
            "title-only rebuild leaves HTML body unchanged"
        );
        assert_eq!(after[4].4, before[4].4);
        let title_only = db
            .pool
            .string_quintuples(
                "SELECT rendered_title, '', '', '', '' FROM posts WHERE post_id = 509",
            )
            .await
            .unwrap();
        assert_eq!(title_only[0].0, "title");
        assert_eq!(db.pool.string_quintuples(
            "SELECT title, body, rendered_html, CAST(updated_at AS TEXT), COALESCE(rendered_title, '') FROM post_revisions"
        ).await.unwrap(), revision_before, "historical Post Revision bytes are immutable");
        assert_eq!(
            db.pool
                .scalar_i64("SELECT COUNT(*) FROM post_revisions")
                .await
                .unwrap(),
            1
        );
        assert_eq!(
            db.pool
                .scalar_i64("SELECT COUNT(*) FROM feed_cache WHERE feed_url = '/feed.rss'")
                .await
                .unwrap(),
            0
        );
        assert_eq!(
            db.pool
                .scalar_i64("SELECT COUNT(*) FROM feed_events")
                .await
                .unwrap(),
            12,
            "only changed public Post affects site, User, and Tag feeds in three formats"
        );
        let paths = db
            .pool
            .string_quintuples("SELECT feed_url, '', '', '', '' FROM feed_events ORDER BY feed_url")
            .await
            .unwrap();
        assert!(
            paths
                .iter()
                .any(|path| path.0.contains("/tags/topic/feed.rss")),
            "affected Tag Syndication Feeds are queued for regeneration"
        );
        assert_eq!(
            db.pool
                .scalar_i64("SELECT COUNT(*) FROM pending_code_migrations")
                .await
                .unwrap(),
            0
        );
        db.pool
            .execute(
                "INSERT INTO pending_code_migrations (operation) VALUES ('rebuild_rendered_posts')",
            )
            .await
            .unwrap();
        db.drain_pending_code_migrations().await;
        assert_eq!(
            db.pool
                .scalar_i64("SELECT COUNT(*) FROM feed_events")
                .await
                .unwrap(),
            12,
            "byte-identical repeat rebuild produces no publish work"
        );
    }

    #[apply(backends)]
    #[tokio::test]
    async fn migration_0045_reconciles_current_media_and_rolls_back_failed_attempt(
        #[case] backend: Backend,
    ) {
        const HASH: &str = "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855";
        let db = MigrationDatabase::new(backend).await;
        db.migrate_to(44).await.unwrap();
        db.drain_pending_code_migrations().await;
        let user = match backend {
            Backend::Sqlite => {
                "INSERT INTO users (user_id, username, password_hash, created_at) VALUES (404, 'media-rebuild', 'hash', CURRENT_TIMESTAMP)"
            }
            Backend::Postgres => {
                "INSERT INTO users (user_id, username, password_hash, created_at) OVERRIDING SYSTEM VALUE VALUES (404, 'media-rebuild', 'hash', CURRENT_TIMESTAMP)"
            }
        };
        db.pool.execute(user).await.unwrap();
        let post = match backend {
            Backend::Sqlite => {
                r#"INSERT INTO posts (post_id, user_id, slug, body, format, rendered_html, created_at, updated_at)
              VALUES (505, 404, 'changed-media', '<img src="/media/upload/e3/b0/e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855/new.jpg">', 'html',
                 '<img src="/media/upload/e3/b0/e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855/old.jpg">',
                 '2025-01-01T00:00:00Z', '2025-02-01T00:00:00Z')"#
            }
            Backend::Postgres => {
                r#"INSERT INTO posts (post_id, user_id, slug, body, format, rendered_html, created_at, updated_at)
              OVERRIDING SYSTEM VALUE VALUES (505, 404, 'changed-media', '<img src="/media/upload/e3/b0/e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855/new.jpg">', 'html',
                 '<img src="/media/upload/e3/b0/e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855/old.jpg">',
                 '2025-01-01T00:00:00Z', '2025-02-01T00:00:00Z')"#
            }
        };
        db.pool.execute(post).await.unwrap();
        db.pool.execute(
            "INSERT INTO post_revisions (post_id, user_id, slug, body, format, rendered_html, created_at, updated_at)
             SELECT post_id, user_id, slug, body, format, rendered_html, created_at, updated_at FROM posts WHERE post_id = 505",
        ).await.unwrap();
        let insert_media = format!(
            "INSERT INTO media (user_id, sha256, filename, source, content_type, size_bytes) VALUES
             (404, '{HASH}', 'old.jpg', 'upload', 'image/jpeg', 1),
             (404, '{HASH}', 'new.jpg', 'upload', 'image/jpeg', 1)"
        );
        let insert_current = format!(
            "INSERT INTO post_media (post_id, source, sha256, filename, reference_kind, reference_form)
             VALUES (505, 'upload', '{HASH}', 'old.jpg', 'local', '/media/upload/e3/b0/{HASH}/old.jpg')"
        );
        let insert_revision = format!(
            "INSERT INTO post_media (post_id, subject_kind, revision_id, source, sha256, filename, reference_kind, reference_form)
             SELECT 505, 'revision', revision_id, 'upload', '{HASH}', 'old.jpg', 'local', '/media/upload/e3/b0/{HASH}/old.jpg'
             FROM post_revisions WHERE post_id = 505"
        );
        crate::with_closeable_pool!(&db.pool, pool, {
            async {
                // All substitutions use the fixed test-only content hash.
                sqlx::query(sqlx::AssertSqlSafe(insert_media))
                    .execute(pool)
                    .await?;
                sqlx::query(sqlx::AssertSqlSafe(insert_current))
                    .execute(pool)
                    .await?;
                sqlx::query(sqlx::AssertSqlSafe(insert_revision))
                    .execute(pool)
                    .await?;
                Ok::<_, sqlx::Error>(())
            }
            .await
        })
        .unwrap();
        let original_html = db.pool.string_quintuples(
            "SELECT rendered_html, body, CAST(updated_at AS TEXT), '', '' FROM posts WHERE post_id = 505"
        ).await.unwrap();
        db.migrate_to(45).await.unwrap();
        match backend {
            Backend::Sqlite => db
                .pool
                .execute(
                    "CREATE TRIGGER reject_rebuild_delete BEFORE DELETE ON pending_code_migrations
                 BEGIN SELECT RAISE(ABORT, 'injected rebuild failure'); END",
                )
                .await
                .unwrap(),
            Backend::Postgres => {
                db.pool.execute(
                    "CREATE FUNCTION reject_rebuild_delete() RETURNS trigger LANGUAGE plpgsql AS $$
                     BEGIN RAISE EXCEPTION 'injected rebuild failure'; END; $$"
                ).await.unwrap();
                db.pool.execute(
                    "CREATE TRIGGER reject_rebuild_delete BEFORE DELETE ON pending_code_migrations
                     FOR EACH ROW EXECUTE FUNCTION reject_rebuild_delete()"
                ).await.unwrap();
            }
        }
        let error = db
            .drain_pending(&|| Ok(()))
            .await
            .expect_err("failure after rebuild rolls back");
        assert!(error.to_string().contains("injected rebuild failure"));
        assert_eq!(
            db.pool
                .scalar_i64("SELECT COUNT(*) FROM pending_code_migrations")
                .await
                .unwrap(),
            1
        );
        assert_eq!(db.pool.string_quintuples(
            "SELECT rendered_html, body, CAST(updated_at AS TEXT), '', '' FROM posts WHERE post_id = 505"
        ).await.unwrap(), original_html, "rendering rolls back with the queue row");
        assert_eq!(db.pool.scalar_i64("SELECT COUNT(*) FROM post_media WHERE subject_kind = 'current' AND filename = 'old.jpg'").await.unwrap(), 1);
        let drop_trigger = match backend {
            Backend::Sqlite => "DROP TRIGGER reject_rebuild_delete",
            Backend::Postgres => "DROP TRIGGER reject_rebuild_delete ON pending_code_migrations",
        };
        db.pool.execute(drop_trigger).await.unwrap();
        db.drain_pending_code_migrations().await;
        assert_eq!(
            db.pool
                .scalar_i64("SELECT COUNT(*) FROM pending_code_migrations")
                .await
                .unwrap(),
            0
        );
        assert_eq!(db.pool.scalar_i64("SELECT COUNT(*) FROM post_media WHERE subject_kind = 'current' AND filename = 'old.jpg'").await.unwrap(), 0);
        assert_eq!(db.pool.scalar_i64("SELECT COUNT(*) FROM post_media WHERE subject_kind = 'current' AND filename = 'new.jpg'").await.unwrap(), 1);
        assert_eq!(db.pool.scalar_i64("SELECT COUNT(*) FROM post_media WHERE subject_kind = 'revision' AND filename = 'old.jpg'").await.unwrap(), 1);
        assert_eq!(
            db.pool
                .scalar_i64("SELECT COUNT(*) FROM media WHERE user_id = 404")
                .await
                .unwrap(),
            2,
            "Media Record ownership is unchanged"
        );
        assert_eq!(
            db.pool
                .scalar_i64("SELECT COUNT(*) FROM feed_events")
                .await
                .unwrap(),
            0,
            "non-public derivative change does not create publish work"
        );
        assert_eq!(db.pool.string_quintuples(
            "SELECT body, CAST(updated_at AS TEXT), '', '', '' FROM posts WHERE post_id = 505"
        ).await.unwrap()[0].0, original_html[0].1);
    }

    #[apply(backends)]
    #[tokio::test]
    async fn migration_0039_adds_nullable_columns_without_repairing_existing_rows(
        #[case] backend: Backend,
    ) {
        let db = MigrationDatabase::new(backend).await;
        db.migrate_to(38).await.unwrap();
        db.pool
            .execute(
                "INSERT INTO users (username, password_hash, created_at) \
                 VALUES ('rendered-title-author', 'hash', CURRENT_TIMESTAMP)",
            )
            .await
            .unwrap();
        db.pool
            .execute(
                "INSERT INTO posts \
                 (user_id, title, slug, body, format, rendered_html, created_at, updated_at) \
                 VALUES ((SELECT user_id FROM users WHERE username = 'rendered-title-author'), \
                 'legacy title', 'legacy-title', 'body', 'markdown', '<p>body</p>', \
                 CURRENT_TIMESTAMP, CURRENT_TIMESTAMP)",
            )
            .await
            .unwrap();
        db.pool
            .execute(
                "INSERT INTO post_revisions \
                 (post_id, user_id, title, slug, body, format, rendered_html, summary, \
                  created_at, updated_at, published_at, deleted_at) \
                 SELECT post_id, user_id, title, slug, body, format, rendered_html, NULL, \
                        created_at, updated_at, NULL, NULL \
                 FROM posts",
            )
            .await
            .unwrap();

        db.migrate_to(39).await.unwrap();

        let nullable_column_count = match &db.pool {
            CloseablePool::Sqlite(pool) => {
                sqlx::query_scalar::<_, RowCount>(
                    "SELECT COUNT(*) FROM pragma_table_info('posts') \
                     WHERE name = 'rendered_title' AND \"notnull\" = 0",
                )
                .fetch_one(pool)
                .await
                .unwrap()
                .into_u64()
                    + sqlx::query_scalar::<_, RowCount>(
                        "SELECT COUNT(*) FROM pragma_table_info('post_revisions') \
                         WHERE name = 'rendered_title' AND \"notnull\" = 0",
                    )
                    .fetch_one(pool)
                    .await
                    .unwrap()
                    .into_u64()
            }
            CloseablePool::Postgres(pool) => sqlx::query_scalar::<_, RowCount>(
                "SELECT COUNT(*) FROM information_schema.columns \
                 WHERE table_name IN ('posts', 'post_revisions') \
                 AND column_name = 'rendered_title' AND is_nullable = 'YES'",
            )
            .fetch_one(pool)
            .await
            .unwrap()
            .into_u64(),
        };
        assert_eq!(
            nullable_column_count, 2,
            "migration 0039 adds both Rendered Title columns as nullable"
        );
        assert_eq!(
            db.pool
                .scalar_i64("SELECT COUNT(*) FROM posts WHERE rendered_title IS NULL")
                .await
                .unwrap(),
            1,
            "migration 0039 does not repair an existing Post"
        );
        assert_eq!(
            db.pool
                .scalar_i64("SELECT COUNT(*) FROM post_revisions WHERE rendered_title IS NULL")
                .await
                .unwrap(),
            1,
            "migration 0039 does not repair an existing Post Revision"
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
    async fn migration_0040_invalidates_pre_rendered_title_feed_cache_rows(
        #[case] backend: Backend,
    ) {
        let db = MigrationDatabase::new(backend).await;
        db.migrate_to(39).await.unwrap();
        db.pool
            .execute(
                "INSERT INTO feed_cache \
                 (feed_url, body, etag, content_type, representation_modified_at, generated_at, semantic_fingerprint) VALUES \
                 ('/feed.rss', '<rss/>', '\"legacy\"', 'application/rss+xml; charset=utf-8', \
                 CURRENT_TIMESTAMP, CURRENT_TIMESTAMP, \
                 '0000000000000000000000000000000000000000000000000000000000000000')",
            )
            .await
            .unwrap();
        db.pool
            .execute("INSERT INTO site_config (key, value) VALUES ('legacy.unrelated', 'retained')")
            .await
            .unwrap();

        db.migrate_current().await.unwrap();

        assert_eq!(
            db.pool
                .scalar_i64("SELECT COUNT(*) FROM feed_cache")
                .await
                .unwrap(),
            0,
            "pre-Rendered-Title cache bytes cannot be served"
        );
        assert_eq!(
            db.pool
                .scalar_i64("SELECT COUNT(*) FROM site_config WHERE key = 'legacy.unrelated' AND value = 'retained'")
                .await
                .unwrap(),
            1,
            "cache invalidation must not erase unrelated durable state"
        );
    }

    #[apply(backends)]
    #[tokio::test]
    async fn migration_0041_repairs_duplicate_active_slugs_before_enforcing_uniqueness(
        #[case] backend: Backend,
    ) {
        let db = MigrationDatabase::new(backend).await;
        db.migrate_to(40).await.unwrap();
        let insert_user = match &db.pool {
            CloseablePool::Sqlite(_) => {
                "INSERT INTO users (user_id, username, password_hash, created_at) VALUES
                 (6101, 'slug-migration-a', 'hash', CURRENT_TIMESTAMP),
                 (6102, 'slug-migration-b', 'hash', CURRENT_TIMESTAMP)"
            }
            CloseablePool::Postgres(_) => {
                "INSERT INTO users (user_id, username, password_hash, created_at)
                 OVERRIDING SYSTEM VALUE VALUES
                 (6101, 'slug-migration-a', 'hash', CURRENT_TIMESTAMP),
                 (6102, 'slug-migration-b', 'hash', CURRENT_TIMESTAMP)"
            }
        };
        db.pool.execute(insert_user).await.unwrap();
        let insert_posts = match &db.pool {
            CloseablePool::Sqlite(_) => {
                "INSERT INTO posts
                 (post_id, user_id, title, rendered_title, slug, body, format, rendered_html,
                  created_at, updated_at, published_at, summary, deleted_at) VALUES
                 (6110, 6101, 'Oldest', 'Oldest', 'shared', 'oldest', 'html', '<p>oldest</p>',
                  '2026-01-01T00:00:00Z', '2030-01-01T00:00:00Z', '2026-01-01T00:00:00Z', NULL, NULL),
                 (6111, 6101, 'Middle', 'Middle', 'shared', 'middle', 'html', '<p>middle</p>',
                  '2026-01-02T00:00:00Z', '2026-01-02T00:00:00Z', '2026-01-02T00:00:00Z', NULL, NULL),
                 (6112, 6101, 'Newest', 'Newest', 'shared', 'newest', 'html', '<p>newest</p>',
                  '2026-01-03T00:00:00Z', '2026-01-03T00:00:00Z', '2026-01-03T00:00:00Z', NULL, NULL),
                 (6113, 6101, 'Occupied', 'Occupied', 'shared-1', 'occupied', 'html', '<p>occupied</p>',
                  '2026-01-04T00:00:00Z', '2026-01-04T00:00:00Z', '2026-01-04T00:00:00Z', NULL, NULL),
                 (6114, 6101, 'Deleted', 'Deleted', 'shared', 'deleted', 'html', '<p>deleted</p>',
                  '2026-01-05T00:00:00Z', '2026-01-05T00:00:00Z', '2026-01-05T00:00:00Z', NULL, '2026-01-06T00:00:00Z'),
                 (6120, 6102, 'Other User', 'Other User', 'shared', 'other', 'html', '<p>other</p>',
                  '2026-01-01T00:00:00Z', '2026-01-01T00:00:00Z', '2026-01-01T00:00:00Z', NULL, NULL)"
            }
            CloseablePool::Postgres(_) => {
                "INSERT INTO posts
                 (post_id, user_id, title, rendered_title, slug, body, format, rendered_html,
                  created_at, updated_at, published_at, summary, deleted_at)
                 OVERRIDING SYSTEM VALUE VALUES
                 (6110, 6101, 'Oldest', 'Oldest', 'shared', 'oldest', 'html', '<p>oldest</p>',
                  '2026-01-01T00:00:00Z', '2030-01-01T00:00:00Z', '2026-01-01T00:00:00Z', NULL, NULL),
                 (6111, 6101, 'Middle', 'Middle', 'shared', 'middle', 'html', '<p>middle</p>',
                  '2026-01-02T00:00:00Z', '2026-01-02T00:00:00Z', '2026-01-02T00:00:00Z', NULL, NULL),
                 (6112, 6101, 'Newest', 'Newest', 'shared', 'newest', 'html', '<p>newest</p>',
                  '2026-01-03T00:00:00Z', '2026-01-03T00:00:00Z', '2026-01-03T00:00:00Z', NULL, NULL),
                 (6113, 6101, 'Occupied', 'Occupied', 'shared-1', 'occupied', 'html', '<p>occupied</p>',
                  '2026-01-04T00:00:00Z', '2026-01-04T00:00:00Z', '2026-01-04T00:00:00Z', NULL, NULL),
                 (6114, 6101, 'Deleted', 'Deleted', 'shared', 'deleted', 'html', '<p>deleted</p>',
                  '2026-01-05T00:00:00Z', '2026-01-05T00:00:00Z', '2026-01-05T00:00:00Z', NULL, '2026-01-06T00:00:00Z'),
                 (6120, 6102, 'Other User', 'Other User', 'shared', 'other', 'html', '<p>other</p>',
                  '2026-01-01T00:00:00Z', '2026-01-01T00:00:00Z', '2026-01-01T00:00:00Z', NULL, NULL)"
            }
        };
        db.pool.execute(insert_posts).await.unwrap();
        let insert_long_slugs = match &db.pool {
            CloseablePool::Sqlite(_) => {
                "INSERT INTO posts
                 (post_id, user_id, title, rendered_title, slug, body, format, rendered_html,
                  created_at, updated_at, published_at) VALUES
                 (6121, 6102, 'Long Old', 'Long Old', replace(hex(zeroblob(80)), '00', '界'),
                  'old', 'html', '<p>old</p>', '2026-02-01T00:00:00Z',
                  '2026-02-01T00:00:00Z', '2026-02-01T00:00:00Z'),
                 (6122, 6102, 'Long New', 'Long New', replace(hex(zeroblob(80)), '00', '界'),
                  'new', 'html', '<p>new</p>', '2026-02-02T00:00:00Z',
                  '2026-02-02T00:00:00Z', '2026-02-02T00:00:00Z')"
            }
            CloseablePool::Postgres(_) => {
                "INSERT INTO posts
                 (post_id, user_id, title, rendered_title, slug, body, format, rendered_html,
                  created_at, updated_at, published_at)
                 OVERRIDING SYSTEM VALUE VALUES
                 (6121, 6102, 'Long Old', 'Long Old', repeat('界', 80),
                  'old', 'html', '<p>old</p>', '2026-02-01T00:00:00Z',
                  '2026-02-01T00:00:00Z', '2026-02-01T00:00:00Z'),
                 (6122, 6102, 'Long New', 'Long New', repeat('界', 80),
                  'new', 'html', '<p>new</p>', '2026-02-02T00:00:00Z',
                  '2026-02-02T00:00:00Z', '2026-02-02T00:00:00Z')"
            }
        };
        db.pool.execute(insert_long_slugs).await.unwrap();
        let insert_edge_groups = match &db.pool {
            CloseablePool::Sqlite(_) => {
                "INSERT INTO posts
                 (post_id, user_id, title, rendered_title, slug, body, format, rendered_html,
                  created_at, updated_at, published_at) VALUES
                 (6123, 6102, 'Colliding Long Old', 'Colliding Long Old',
                  replace(hex(zeroblob(78)), '00', '界') || '甲乙', 'old', 'html', '<p>old</p>',
                  '2026-02-03T00:00:00Z', '2026-02-03T00:00:00Z', '2026-02-03T00:00:00Z'),
                 (6124, 6102, 'Colliding Long New', 'Colliding Long New',
                  replace(hex(zeroblob(78)), '00', '界') || '甲乙', 'new', 'html', '<p>new</p>',
                  '2026-02-04T00:00:00Z', '2026-02-04T00:00:00Z', '2026-02-04T00:00:00Z'),
                 (6140, 6101, 'Quad One', 'Quad One', 'quad', 'one', 'html', '<p>one</p>',
                  '2026-03-01T00:00:00Z', '2026-03-01T00:00:00Z', '2026-03-01T00:00:00Z'),
                 (6141, 6101, 'Quad Two', 'Quad Two', 'quad', 'two', 'html', '<p>two</p>',
                  '2026-03-02T00:00:00Z', '2026-03-02T00:00:00Z', '2026-03-02T00:00:00Z'),
                 (6142, 6101, 'Quad Three', 'Quad Three', 'quad', 'three', 'html', '<p>three</p>',
                  '2026-03-03T00:00:00Z', '2026-03-03T00:00:00Z', '2026-03-03T00:00:00Z'),
                 (6143, 6101, 'Quad Four', 'Quad Four', 'quad', 'four', 'html', '<p>four</p>',
                  '2026-03-04T00:00:00Z', '2026-03-04T00:00:00Z', '2026-03-04T00:00:00Z'),
                 (6150, 6102, 'Cutoff Old', 'Cutoff Old',
                  replace(hex(zeroblob(77)), '00', 'a') || '-bb', 'old', 'html', '<p>old</p>',
                  '2026-04-01T00:00:00Z', '2026-04-01T00:00:00Z', '2026-04-01T00:00:00Z'),
                 (6151, 6102, 'Cutoff New', 'Cutoff New',
                  replace(hex(zeroblob(77)), '00', 'a') || '-bb', 'new', 'html', '<p>new</p>',
                  '2026-04-02T00:00:00Z', '2026-04-02T00:00:00Z', '2026-04-02T00:00:00Z')"
            }
            CloseablePool::Postgres(_) => {
                "INSERT INTO posts
                 (post_id, user_id, title, rendered_title, slug, body, format, rendered_html,
                  created_at, updated_at, published_at)
                 OVERRIDING SYSTEM VALUE VALUES
                 (6123, 6102, 'Colliding Long Old', 'Colliding Long Old',
                  repeat('界', 78) || '甲乙', 'old', 'html', '<p>old</p>',
                  '2026-02-03T00:00:00Z', '2026-02-03T00:00:00Z', '2026-02-03T00:00:00Z'),
                 (6124, 6102, 'Colliding Long New', 'Colliding Long New',
                  repeat('界', 78) || '甲乙', 'new', 'html', '<p>new</p>',
                  '2026-02-04T00:00:00Z', '2026-02-04T00:00:00Z', '2026-02-04T00:00:00Z'),
                 (6140, 6101, 'Quad One', 'Quad One', 'quad', 'one', 'html', '<p>one</p>',
                  '2026-03-01T00:00:00Z', '2026-03-01T00:00:00Z', '2026-03-01T00:00:00Z'),
                 (6141, 6101, 'Quad Two', 'Quad Two', 'quad', 'two', 'html', '<p>two</p>',
                  '2026-03-02T00:00:00Z', '2026-03-02T00:00:00Z', '2026-03-02T00:00:00Z'),
                 (6142, 6101, 'Quad Three', 'Quad Three', 'quad', 'three', 'html', '<p>three</p>',
                  '2026-03-03T00:00:00Z', '2026-03-03T00:00:00Z', '2026-03-03T00:00:00Z'),
                 (6143, 6101, 'Quad Four', 'Quad Four', 'quad', 'four', 'html', '<p>four</p>',
                  '2026-03-04T00:00:00Z', '2026-03-04T00:00:00Z', '2026-03-04T00:00:00Z'),
                 (6150, 6102, 'Cutoff Old', 'Cutoff Old', repeat('a', 77) || '-bb',
                  'old', 'html', '<p>old</p>', '2026-04-01T00:00:00Z',
                  '2026-04-01T00:00:00Z', '2026-04-01T00:00:00Z'),
                 (6151, 6102, 'Cutoff New', 'Cutoff New', repeat('a', 77) || '-bb',
                  'new', 'html', '<p>new</p>', '2026-04-02T00:00:00Z',
                  '2026-04-02T00:00:00Z', '2026-04-02T00:00:00Z')"
            }
        };
        db.pool.execute(insert_edge_groups).await.unwrap();
        db.pool
            .execute("INSERT INTO tags (tag_id, tag_slug) VALUES (6130, 'migration-tag')")
            .await
            .unwrap();
        db.pool
            .execute("INSERT INTO post_tags (post_id, tag_id, tag_display) VALUES (6110, 6130, 'Migration Tag')")
            .await
            .unwrap();
        db.pool
            .execute(
                "INSERT INTO post_audiences (post_id, target_kind_id, audience_id)
                 SELECT 6110, kind_id, NULL FROM target_kinds WHERE name = 'public'",
            )
            .await
            .unwrap();
        db.pool
            .execute(
                "INSERT INTO post_media
                 (post_id, source, sha256, filename, reference_kind, reference_form)
                 VALUES (6110, 'local', 'hash', 'migration.png', 'legacy', '')",
            )
            .await
            .unwrap();
        db.pool
            .execute(
                "INSERT INTO feed_cache
                 (feed_url, body, etag, content_type, representation_modified_at, generated_at, semantic_fingerprint)
                 VALUES ('/~slug-migration-a/feed.atom', '<feed/>', '\"old\"', 'application/atom+xml',
                         CURRENT_TIMESTAMP, CURRENT_TIMESTAMP,
                         '0000000000000000000000000000000000000000000000000000000000000000')",
            )
            .await
            .unwrap();

        db.migrate_current().await.unwrap();

        assert_eq!(
            db.pool
                .scalar_i64("SELECT COUNT(*) FROM posts WHERE post_id = 6112 AND slug = 'shared'")
                .await
                .unwrap(),
            1,
            "newest duplicate retains the base slug"
        );
        assert_eq!(
            db.pool
                .scalar_i64(
                    "SELECT CAST(substr(slug, 8) AS BIGINT) FROM posts WHERE post_id = 6110"
                )
                .await
                .unwrap(),
            2,
            "oldest duplicate takes the first unoccupied suffix"
        );
        assert_eq!(
            db.pool
                .scalar_i64(
                    "SELECT CAST(substr(slug, 8) AS BIGINT) FROM posts WHERE post_id = 6111"
                )
                .await
                .unwrap(),
            3,
            "later repaired duplicates skip earlier allocations"
        );
        assert_eq!(
            db.pool
                .scalar_i64("SELECT COUNT(*) FROM post_permalink_aliases WHERE user_id = 6101 AND slug = 'shared'")
                .await
                .unwrap(),
            2,
            "each renamed Post retains its old permalink identity"
        );
        assert_eq!(
            db.pool
                .scalar_i64(
                    "SELECT COUNT(*) FROM posts
                     WHERE post_id = 6121 AND length(slug) = 80 AND slug LIKE '%-1'",
                )
                .await
                .unwrap(),
            1,
            "suffix allocation preserves the Unicode scalar length boundary"
        );
        assert_eq!(
            db.pool
                .scalar_i64(
                    "SELECT COUNT(*) FROM posts
                     WHERE post_id = 6123 AND length(slug) = 80 AND slug LIKE '%-2'",
                )
                .await
                .unwrap(),
            1,
            "backend-neutral queue order resolves colliding truncated candidates"
        );
        assert_eq!(
            db.pool
                .scalar_i64(
                    "SELECT COUNT(*) FROM posts
                     WHERE post_id = 6150 AND length(slug) = 79
                       AND slug LIKE '%-1' AND slug NOT LIKE '%--1'",
                )
                .await
                .unwrap(),
            1,
            "migration suffixes trim a hyphen exposed at the truncation boundary"
        );
        assert_eq!(
            db.pool
                .scalar_i64(
                    "SELECT COUNT(*) FROM posts WHERE
                     (post_id = 6140 AND slug = 'quad-1') OR
                     (post_id = 6141 AND slug = 'quad-2') OR
                     (post_id = 6142 AND slug = 'quad-3') OR
                     (post_id = 6143 AND slug = 'quad')",
                )
                .await
                .unwrap(),
            4,
            "four-Post groups keep the newest base and suffix oldest-first"
        );
        assert_eq!(
            db.pool
                .scalar_i64("SELECT COUNT(*) FROM posts WHERE post_id = 6114 AND slug = 'shared'")
                .await
                .unwrap(),
            1,
            "Deleted Posts do not participate in repair"
        );
        assert_eq!(
            db.pool
                .scalar_i64("SELECT COUNT(*) FROM posts WHERE post_id = 6120 AND slug = 'shared'")
                .await
                .unwrap(),
            1,
            "each User has an independent active slug namespace"
        );
        assert_eq!(
            db.pool
                .scalar_i64("SELECT COUNT(*) FROM post_revisions WHERE post_id IN (6110, 6111)")
                .await
                .unwrap(),
            2,
            "each repair captures exactly one prior-state Revision"
        );
        assert_eq!(
            db.pool
                .scalar_i64(
                    "SELECT COUNT(*) FROM post_revisions
                     WHERE post_id = 6110 AND slug = 'shared' AND body = 'oldest'
                       AND updated_at = '2030-01-01T00:00:00Z'",
                )
                .await
                .unwrap(),
            1,
            "the Revision retains the complete pre-repair scalar state"
        );
        assert_eq!(
            db.pool
                .scalar_i64(
                    "SELECT COUNT(*) FROM post_revision_tags prt
                     JOIN post_revisions pr ON pr.revision_id = prt.revision_id
                     WHERE pr.post_id = 6110 AND prt.tag_slug = 'migration-tag'",
                )
                .await
                .unwrap(),
            1,
            "repair preserves Revision tag state"
        );
        assert_eq!(
            db.pool
                .scalar_i64(
                    "SELECT COUNT(*) FROM post_revision_audiences pra
                     JOIN post_revisions pr ON pr.revision_id = pra.revision_id
                     WHERE pr.post_id = 6110 AND pra.target_kind = 'public'",
                )
                .await
                .unwrap(),
            1,
            "repair preserves Revision audience state"
        );
        assert_eq!(
            db.pool
                .scalar_i64(
                    "SELECT COUNT(*) FROM post_media pm
                     JOIN post_revisions pr ON pr.revision_id = pm.revision_id
                     WHERE pr.post_id = 6110 AND pm.subject_kind = 'revision'",
                )
                .await
                .unwrap(),
            1,
            "repair snapshots current media references"
        );
        assert_eq!(
            db.pool
                .scalar_i64("SELECT COUNT(*) FROM posts WHERE post_id = 6110 AND updated_at > '2030-01-01T00:00:00Z'")
                .await
                .unwrap(),
            1,
            "repair clock is strictly later even for restored future timestamps"
        );
        assert_eq!(
            db.pool
                .scalar_i64("SELECT COUNT(*) FROM feed_cache")
                .await
                .unwrap(),
            0,
            "permalink-bearing cache state is invalidated"
        );
        db.pool
            .execute(
                "INSERT INTO posts
                 (user_id, title, rendered_title, slug, body, format, rendered_html,
                  created_at, updated_at, published_at)
                 VALUES (6101, 'Conflict', 'Conflict', 'shared', 'body', 'html', '<p>body</p>',
                         CURRENT_TIMESTAMP, CURRENT_TIMESTAMP, CURRENT_TIMESTAMP)",
            )
            .await
            .expect_err("the active per-User slug constraint must arbitrate writes");
        db.pool
            .execute("UPDATE posts SET deleted_at = CURRENT_TIMESTAMP WHERE post_id = 6112")
            .await
            .unwrap();
        db.pool
            .execute(
                "INSERT INTO posts
                 (user_id, title, rendered_title, slug, body, format, rendered_html,
                  created_at, updated_at, published_at)
                 VALUES (6101, 'Replacement', 'Replacement', 'shared', 'body', 'html', '<p>body</p>',
                         CURRENT_TIMESTAMP, CURRENT_TIMESTAMP, CURRENT_TIMESTAMP)",
            )
            .await
            .expect("soft deletion releases the active slug");

        db.migrate_current().await.unwrap();
        assert_eq!(
            db.pool
                .scalar_i64("SELECT COUNT(*) FROM post_revisions WHERE post_id IN (6110, 6111)")
                .await
                .unwrap(),
            2,
            "an already-applied migration is inert"
        );
    }

    #[apply(backends)]
    #[tokio::test]
    async fn migration_0042_backfills_search_projection_and_rejects_stale_candidates(
        #[case] backend: Backend,
    ) {
        let db = MigrationDatabase::new(backend).await;
        db.migrate_to(41).await.unwrap();
        let (insert_user, insert_post) = match &db.pool {
            CloseablePool::Sqlite(_) => (
                "INSERT INTO users (user_id, username, password_hash, created_at)
                 VALUES (604, 'search-backfill-author', 'hash', CURRENT_TIMESTAMP)",
                "INSERT INTO posts
                 (post_id, user_id, title, rendered_title, slug, body, format, rendered_html,
                  created_at, updated_at)
                 VALUES (605, 604, 'Old Title', 'Old Title', 'legacy-slug', 'body', 'markdown',
                         '<p>body</p>', CURRENT_TIMESTAMP, CURRENT_TIMESTAMP)",
            ),
            CloseablePool::Postgres(_) => (
                "INSERT INTO users (user_id, username, password_hash, created_at)
                 OVERRIDING SYSTEM VALUE
                 VALUES (604, 'search-backfill-author', 'hash', CURRENT_TIMESTAMP)",
                "INSERT INTO posts
                 (post_id, user_id, title, rendered_title, slug, body, format, rendered_html,
                  created_at, updated_at)
                 OVERRIDING SYSTEM VALUE
                 VALUES (605, 604, 'Old Title', 'Old Title', 'legacy-slug', 'body', 'markdown',
                         '<p>body</p>', CURRENT_TIMESTAMP, CURRENT_TIMESTAMP)",
            ),
        };
        db.pool.execute(insert_user).await.unwrap();
        db.pool.execute(insert_post).await.unwrap();

        db.migrate_current().await.unwrap();
        assert_eq!(
            db.pool
                .scalar_i64(
                    "SELECT COUNT(*) FROM posts
                     WHERE post_id = 605 AND search_text IS NULL AND mutation_version = 1",
                )
                .await
                .unwrap(),
            1,
            "migration leaves derivation to bounded startup work"
        );

        let stale = PostSearchBackfillCandidate {
            post_id: PostId::from(605),
            mutation_version: PostMutationVersion::initial(),
            search_text: StoredPostSearchText::from("old title legacy-slug".to_owned()),
        };
        db.pool
            .execute(
                "UPDATE posts SET title = 'New Title', rendered_title = 'New Title',
                 mutation_version = 2 WHERE post_id = 605",
            )
            .await
            .unwrap();
        db.apply_post_search_candidates(&[stale]).await;
        assert_eq!(
            db.pool
                .scalar_i64(
                    "SELECT COUNT(*) FROM posts WHERE post_id = 605 AND search_text IS NULL"
                )
                .await
                .unwrap(),
            1,
            "a stale derivation cannot overwrite newer Post state"
        );

        db.backfill_post_search_projections().await;
        db.backfill_post_search_projections().await;
        assert_eq!(
            db.pool
                .scalar_i64(
                    "SELECT COUNT(*) FROM posts
                     WHERE post_id = 605 AND search_text = 'new title legacy-slug'
                       AND mutation_version = 2",
                )
                .await
                .unwrap(),
            1,
            "startup backfill is idempotent and derives the current normalized value"
        );
    }

    #[apply(backends)]
    #[tokio::test]
    async fn migration_0034_removes_legacy_theme_rows_after_0033_backfill(
        #[case] backend: Backend,
    ) {
        let db = MigrationDatabase::new(backend).await;
        db.migrate_to(32).await.unwrap();
        db.pool
            .execute("INSERT INTO site_config (key, value) VALUES ('site.theme', 'reader')")
            .await
            .unwrap();
        let insert_user = match &db.pool {
            CloseablePool::Sqlite(_) => {
                "INSERT INTO users (user_id, username, password_hash, created_at) \
                 VALUES (101, 'theme-cutover-user', 'hash', CURRENT_TIMESTAMP)"
            }
            CloseablePool::Postgres(_) => {
                "INSERT INTO users (user_id, username, password_hash, created_at) \
                 OVERRIDING SYSTEM VALUE \
                 VALUES (101, 'theme-cutover-user', 'hash', CURRENT_TIMESTAMP)"
            }
        };
        db.pool.execute(insert_user).await.unwrap();
        db.pool
            .execute("INSERT INTO user_config (user_id, key, value) VALUES (101, 'user.theme', 'terminal')")
            .await
            .unwrap();

        db.migrate_current().await.unwrap();

        assert_eq!(
            db.pool
                .scalar_i64(
                    "SELECT COUNT(*) FROM theme_selections \
                     WHERE catalog_owner_key = 'site' AND builtin_theme = 'reader' AND theme_id IS NULL",
                )
                .await
                .unwrap(),
            1,
            "site selection survives the cutover",
        );
        assert_eq!(
            db.pool
                .scalar_i64(
                    "SELECT COUNT(*) FROM theme_selections \
                     WHERE catalog_owner_key = 'user:101' AND builtin_theme = 'terminal' AND theme_id IS NULL",
                )
                .await
                .unwrap(),
            1,
            "author selection survives the cutover",
        );
        assert_eq!(
            db.pool
                .scalar_i64(
                    "SELECT (SELECT COUNT(*) FROM site_config WHERE key = 'site.theme') \
                     + (SELECT COUNT(*) FROM user_config WHERE key = 'user.theme')",
                )
                .await
                .unwrap(),
            0,
            "0034 removes legacy theme rows after their selections are materialized",
        );
        assert_eq!(
            db.pool
                .scalar_i64("SELECT MAX(version) FROM _sqlx_migrations")
                .await
                .unwrap(),
            47,
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
        let preserved_subscriber = crate::with_closeable_pool!(&db.pool, pool, {
            sqlx::query_scalar::<_, i64>(
                "SELECT COUNT(*) FROM subscriptions \
                 WHERE subscription_id = 202 AND author_user_id = 101 \
                   AND subscriber_ref = $1 AND created_at IS NOT NULL",
            )
            .bind_storage(&subscriber_ref)
            .fetch_one(pool)
            .await
            .unwrap()
        });
        assert_eq!(
            preserved_subscriber, 1,
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

        let empty_error = crate::with_closeable_pool!(&db.pool, pool, {
            sqlx::query(
                "INSERT INTO subscriptions \
                 (subscription_id, author_user_id, channel_id, subscriber_ref, status_id) \
                 SELECT 203, 101, channels.channel_id, $1, subscription_statuses.status_id \
                 FROM channels CROSS JOIN subscription_statuses \
                 WHERE channels.name = 'local' AND subscription_statuses.name = 'active'",
            )
            .bind_storage(CorruptSubscriberRef(String::new()))
            .execute(pool)
            .await
            .map(|_| ())
        })
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

        crate::with_closeable_pool!(&db.pool, pool, {
            sqlx::query(
                "INSERT INTO subscriptions \
                 (subscription_id, author_user_id, channel_id, subscriber_ref, status_id) \
                 SELECT 203, 101, channels.channel_id, $1, subscription_statuses.status_id \
                 FROM channels CROSS JOIN subscription_statuses \
                 WHERE channels.name = 'local' AND subscription_statuses.name = 'active'",
            )
            .bind_storage(CorruptSubscriberRef("   ".to_owned()))
            .execute(pool)
            .await
            .unwrap();
        });
        let whitespace_subscriber = crate::with_closeable_pool!(&db.pool, pool, {
            sqlx::query_scalar::<_, i64>(
                "SELECT COUNT(*) FROM subscriptions \
                 WHERE subscription_id = 203 AND subscriber_ref = $1 \
                   AND created_at IS NOT NULL",
            )
            .bind_storage(CorruptSubscriberRef("   ".to_owned()))
            .fetch_one(pool)
            .await
            .unwrap()
        });
        assert_eq!(
            whitespace_subscriber, 1,
            "the portable schema subset rejects only zero length and retains the timestamp default"
        );

        let duplicate_error = crate::with_closeable_pool!(&db.pool, pool, {
            sqlx::query(
                "INSERT INTO subscriptions \
                 (subscription_id, author_user_id, channel_id, subscriber_ref, status_id) \
                 SELECT 204, 101, channels.channel_id, $1, subscription_statuses.status_id \
                 FROM channels CROSS JOIN subscription_statuses \
                 WHERE channels.name = 'local' AND subscription_statuses.name = 'active'",
            )
            .bind_storage(&subscriber_ref)
            .execute(pool)
            .await
            .map(|_| ())
        })
        .expect_err("the identity UNIQUE constraint must survive migration 0026");
        assert!(
            duplicate_error
                .as_database_error()
                .is_some_and(sqlx::error::DatabaseError::is_unique_violation)
        );

        let missing_parents: SubscriberRef = "missing-parents".parse().unwrap();
        let foreign_key_error = crate::with_closeable_pool!(&db.pool, pool, {
            sqlx::query(
                "INSERT INTO subscriptions \
                 (subscription_id, author_user_id, channel_id, subscriber_ref, status_id) \
                 VALUES (204, 999, 999, $1, 999)",
            )
            .bind_storage(missing_parents)
            .execute(pool)
            .await
            .map(|_| ())
        })
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
        let invalid_subscriber = crate::with_closeable_pool!(&db.pool, pool, {
            sqlx::query_scalar::<_, i64>(
                "SELECT COUNT(*) FROM subscriptions \
                 WHERE subscription_id = 202 AND author_user_id = 101 AND subscriber_ref = $1",
            )
            .bind_storage(CorruptSubscriberRef(String::new()))
            .fetch_one(pool)
            .await
            .unwrap()
        });
        assert_eq!(
            invalid_subscriber, 1,
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
            47
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
    #[apply(backends)]
    #[tokio::test]
    async fn migration_0037_backfills_one_valid_unique_handle_for_existing_users(
        #[case] backend: Backend,
    ) {
        let db = MigrationDatabase::new(backend).await;
        db.migrate_to(36).await.unwrap();
        let insert = match backend {
            Backend::Sqlite => {
                "INSERT INTO users (user_id, username, password_hash, created_at) VALUES \
                 (901, 'passkey-migration-a', 'hash', CURRENT_TIMESTAMP), \
                 (902, 'passkey-migration-b', 'hash', CURRENT_TIMESTAMP)"
            }
            Backend::Postgres => {
                "INSERT INTO users (user_id, username, password_hash, created_at) \
                 OVERRIDING SYSTEM VALUE VALUES \
                 (901, 'passkey-migration-a', 'hash', CURRENT_TIMESTAMP), \
                 (902, 'passkey-migration-b', 'hash', CURRENT_TIMESTAMP)"
            }
        };
        db.pool.execute(insert).await.unwrap();
        db.migrate_current().await.unwrap();
        assert_eq!(
            db.pool
                .scalar_i64(
                    "SELECT COUNT(*) FROM passkey_user_handles \
                     WHERE user_id IN (901, 902) AND length(user_handle) = 32",
                )
                .await
                .unwrap(),
            2,
        );
        assert_eq!(
            db.pool
                .scalar_i64(
                    "SELECT COUNT(DISTINCT user_handle) FROM passkey_user_handles \
                     WHERE user_id IN (901, 902)",
                )
                .await
                .unwrap(),
            2,
        );
    }
}
