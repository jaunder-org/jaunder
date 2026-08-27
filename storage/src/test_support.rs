//! Both-backend test harness for the `storage` crate's own tests and `server`'s
//! integration tests: the `Backend` enum, per-test database provisioning
//! (`SQLite` tempdir; Postgres clone-from-template via `JAUNDER_PG_TEST_URL`), the
//! `AppState`-level `TestEnv`, and the `backends`/`sqlite_only`/`postgres_only`
//! rstest templates. Lives in `storage` (gated by the `test-support` feature) so
//! `storage`'s in-file tests use it from the same crate instance — avoiding the
//! two-`storage`-instances problem a separate crate would create (see ADR-0033).
//! `server` reaches it via `storage`'s `test-support` feature.

// Deliberately unwrap/expect-heavy test scaffolding (test-support feature, ADR-0033),
// so the workspace's `unwrap_used`/`expect_used = deny` lints are expected off for this
// module; `#[expect]` self-removes if the scaffolding ever stops unwrapping. Everything
// else clippy-pedantic flags is fixed in place rather than suppressed. (#94)
// lint-suppression:allow approved in #294; existing expectation documents intentional test-scaffolding or naming exception
#![expect(clippy::unwrap_used, clippy::expect_used)]

use crate::media::MediaRecord;
use crate::posts::{
    CreatePostError, CreatePostInput, INSERT_POST_TAG, PostBookkeepingExpectation, PublishUpdate,
    UPSERT_TAG_RETURNING_ID, UpdatePostInput,
};
use crate::sql::quote_identifier;
use crate::{
    AppState, DbConnectOptions, PostFormat, PostRecord, StorageRuntimeConfig,
    resolved_postgres_options,
};

use common::feed::FeedPath;
use common::ids::{PostId, TagId, UserId};
use common::mailer::{MailSender, NoopMailSender};
use common::media::{
    Filename, MediaRef, MediaReferenceForm, MediaReferenceKind, MediaSource, detect_content_type,
    media_url,
};
use common::post_body::PostBody;
use common::post_summary::PostSummary;
use common::post_title::PostTitle;
use common::render::{RenderOutput, RenderedHtml};
use common::slug::Slug;
use common::tag::TagLabel;
use common::test_support::{
    parse_byte_size, parse_content_hash, parse_display_name, parse_post_body, parse_post_title,
    parse_slug, parse_tag_label, parse_username,
};
use common::time::UtcInstant;
use common::username::Username;
use common::visibility::AudienceTarget;
use host::invite::InviteCode;
use sqlx::pool::PoolConnection;
use sqlx::{Connection, PgPool, Postgres, Sqlite, SqlitePool, Transaction};
use std::{
    fmt::Write as _,
    path::Path,
    sync::{
        Arc,
        atomic::{AtomicU64, Ordering},
    },
};
use tempfile::TempDir;

// This crate only *defines* the templates, so it needs just the `template`
// attribute. `#[export]` is consumed by `#[template]` (no import needed), and the
// `rstest`/`case` attributes the expansion emits are resolved at the *apply* site
// in consumer crates, not here.
use rstest_reuse::template;

/// The storage backend a test runs against. Backend-parametrized tests take a
/// `#[case] backend: Backend` and call [`Backend::setup`].
#[derive(Copy, Clone)]
pub enum Backend {
    Sqlite,
    Postgres,
}

/// A backend-tagged handle to the connection pool behind a test's [`AppState`].
///
/// The pool isn't otherwise reachable from `AppState`, so tests hold this to
/// inject a storage fault by [`close`](CloseablePool::close)-ing it (the next
/// query through any storage handle then errors) or to run raw SQL against the
/// per-test database ([`postgres`](CloseablePool::postgres)).
pub enum CloseablePool {
    Sqlite(SqlitePool),
    Postgres(PgPool),
}

/// Runs a block once for whichever concrete pool variant a [`CloseablePool`]
/// holds.
///
/// This keeps both-backend tests from copy-pasting identical raw SQL bodies just
/// to give `sqlx` a concrete `SqlitePool` or `PgPool` at the call site.
#[macro_export]
macro_rules! with_closeable_pool {
    ($pool:expr, $backend_pool:ident, $body:block) => {
        match $pool {
            $crate::test_support::CloseablePool::Sqlite($backend_pool) => $body,
            $crate::test_support::CloseablePool::Postgres($backend_pool) => $body,
        }
    };
}

/// Rewrites every row in a directory backup's `media.ndjson` to use `filename`.
///
/// # Panics
///
/// If the backup file cannot be read, parsed, serialized, or written.
pub fn rewrite_media_filename_in_backup(backup_path: &Path, filename: &str) {
    let media_ndjson = backup_path.join("db").join("media.ndjson");
    let mut rows: Vec<serde_json::Map<String, serde_json::Value>> =
        std::fs::read_to_string(&media_ndjson)
            .expect("read media backup")
            .lines()
            .filter(|line| !line.trim().is_empty())
            .map(serde_json::from_str)
            .collect::<Result<_, _>>()
            .expect("parse media backup rows");
    for row in &mut rows {
        row.insert("filename".to_owned(), serde_json::json!(filename));
    }

    let mut rewritten = String::new();
    for row in rows {
        writeln!(
            rewritten,
            "{}",
            serde_json::to_string(&row).expect("serialize media row")
        )
        .expect("append media row");
    }
    std::fs::write(media_ndjson, rewritten).expect("write media backup");
}

/// Returns whether the live `media` table contains `filename` as its raw stored value.
///
/// # Panics
///
/// If connecting to the configured test database or querying `media` fails.
pub async fn raw_media_filename_exists(db: &DbConnectOptions, filename: &str) -> bool {
    match db {
        DbConnectOptions::Sqlite(options) => {
            let pool = SqlitePool::connect_with(options.clone())
                .await
                .expect("connect sqlite");
            let exists: i64 =
                sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM media WHERE filename = $1)")
                    // sqlx-newtype-bind:allow permanent-primitive — intentionally invalid backup filename fixture may not parse as Filename.
                    .bind(filename)
                    .fetch_one(&pool)
                    .await
                    .expect("query sqlite media");
            exists != 0
        }
        DbConnectOptions::Postgres { options, .. } => {
            let options = resolved_postgres_options(options, &StorageRuntimeConfig::default());
            let pool = PgPool::connect_with(options)
                .await
                .expect("connect postgres");
            sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM media WHERE filename = $1)")
                // sqlx-newtype-bind:allow permanent-primitive — intentionally invalid backup filename fixture may not parse as Filename.
                .bind(filename)
                .fetch_one(&pool)
                .await
                .expect("query postgres media")
        }
    }
}

impl CloseablePool {
    /// Closes the pool. Afterwards the next query through any storage handle
    /// backed by it returns `sqlx::Error::PoolClosed`, which the storage layer
    /// maps to its `Internal` error variant — the backend-agnostic
    /// storage-error-propagation fault. `sqlx::Pool::close` is generic over the
    /// backend, so the behavior is identical on `SQLite` and Postgres.
    pub async fn close(&self) {
        crate::with_closeable_pool!(self, pool, { pool.close().await });
    }

    /// Runs a raw statement against whichever backend this env uses — the seed
    /// counterpart to [`close`](CloseablePool::close), dispatched internally so
    /// callers stay backend-agnostic. (The SQL string may still be dialect-specific.)
    ///
    /// # Errors
    ///
    /// Returns the `sqlx::Error` if the statement fails to execute.
    pub async fn execute(&self, sql: &str) -> Result<(), sqlx::Error> {
        crate::with_closeable_pool!(self, pool, {
            sqlx::query(sql).execute(pool).await?;
        });
        Ok(())
    }

    /// Fetches a single `i64` scalar (e.g. a `COUNT(*)`) — the inspect
    /// counterpart to [`execute`](CloseablePool::execute), dispatched per backend
    /// so callers stay backend-agnostic.
    ///
    /// # Errors
    ///
    /// Returns the `sqlx::Error` if the query fails.
    pub async fn scalar_i64(&self, sql: &str) -> Result<i64, sqlx::Error> {
        crate::with_closeable_pool!(self, pool, {
            sqlx::query_scalar(sql).fetch_one(pool).await
        })
    }

    /// Fetches every row of a five-`TEXT`-column query.
    ///
    /// # Errors
    ///
    /// Returns the `sqlx::Error` if the query fails.
    pub async fn string_quintuples(
        &self,
        sql: &str,
    ) -> Result<Vec<(String, String, String, String, String)>, sqlx::Error> {
        crate::with_closeable_pool!(self, pool, { sqlx::query_as(sql).fetch_all(pool).await })
    }

    /// Takes the same write lock `set_post_tags` takes and holds it until the
    /// returned guard commits or drops — which [`execute`](CloseablePool::execute)
    /// cannot do, since it returns its connection to the pool as soon as the
    /// statement finishes.
    ///
    /// The lock's granularity differs per backend, deliberately:
    ///
    /// * `SQLite` — `BEGIN IMMEDIATE` takes a **database-wide** write lock, so the
    ///   guard excludes any concurrent writer, not just one on `post_id`.
    /// * Postgres — `SELECT … FOR UPDATE` locks the **post row**, so exclusion is
    ///   per-post.
    ///
    /// Both serialize two writers on the same post, which is the invariant tests
    /// built on this assert. `post_id` is taken on both arms even though only the
    /// Postgres arm needs it: the guard remembers it, so a caller cannot lock one
    /// post and then write to another.
    ///
    /// # Errors
    ///
    /// Returns the `sqlx::Error` if the connection cannot be acquired or the lock
    /// cannot be taken (including when `post_id` does not exist on Postgres).
    pub async fn lock_post_for_write(
        &self,
        post_id: PostId,
    ) -> Result<PostWriteLock<'_>, sqlx::Error> {
        let held = match self {
            CloseablePool::Sqlite(pool) => {
                let mut conn = pool.acquire().await?;
                // IMMEDIATE, mirroring `SqlitePostStorage::set_post_tags`: takes
                // the write lock up front rather than upgrading a shared lock,
                // which `busy_timeout` cannot rescue (ADR-0021).
                sqlx::query("BEGIN IMMEDIATE").execute(&mut *conn).await?;
                HeldWrite::Sqlite(conn)
            }
            CloseablePool::Postgres(pool) => {
                let mut tx = pool.begin().await?;
                // Mirrors `PostgresPostStorage::set_post_tags`.
                sqlx::query_scalar::<_, PostId>(
                    "SELECT post_id FROM posts WHERE post_id = $1 FOR UPDATE",
                )
                .bind(post_id)
                .fetch_one(&mut *tx)
                .await?;
                HeldWrite::Postgres(tx)
            }
        };
        Ok(PostWriteLock { post_id, held })
    }

    /// The Postgres pool, for raw-SQL seed/inspect against the per-test database
    /// (avoids reconnecting a fresh pool via [`recorded_postgres_url`]).
    ///
    /// # Panics
    ///
    /// If called on a `SQLite` environment.
    #[must_use]
    pub fn postgres(&self) -> &PgPool {
        match self {
            CloseablePool::Postgres(pool) => pool,
            CloseablePool::Sqlite(_) => panic!("postgres() on a SQLite CloseablePool"),
        }
    }
    /// Acquires the same transaction-scoped media-reference lock that Post writes,
    /// guarded deletes, and reclamation use.
    ///
    /// `PostgreSQL` takes the target's advisory lock; `SQLite` takes its sole writer lock
    /// up front. The returned guard must be explicitly committed or rolled back.
    ///
    /// # Errors
    ///
    /// Returns an error if acquiring the database connection, beginning the transaction, or
    /// taking its media-reference lock fails.
    pub async fn lock_media_reference_for_write(
        &self,
        media: &MediaRef,
    ) -> Result<MediaReferenceWriteLock<'_>, sqlx::Error> {
        let held = match self {
            CloseablePool::Sqlite(pool) => {
                let mut conn = pool.acquire().await?;
                sqlx::query("BEGIN IMMEDIATE").execute(&mut *conn).await?;
                HeldMediaReferenceWrite::Sqlite(conn)
            }
            CloseablePool::Postgres(pool) => {
                let mut tx = pool.begin().await?;
                let key = crate::posts::media_advisory_lock_key(media);
                sqlx::query("SELECT pg_advisory_xact_lock($1)")
                    .bind(key)
                    .execute(&mut *tx)
                    .await?;
                HeldMediaReferenceWrite::Postgres(tx)
            }
        };
        Ok(MediaReferenceWriteLock { held })
    }
}

/// A test-held media-reference write lock, using the production lock namespace.
pub struct MediaReferenceWriteLock<'a> {
    held: HeldMediaReferenceWrite<'a>,
}

enum HeldMediaReferenceWrite<'a> {
    Sqlite(PoolConnection<Sqlite>),
    Postgres(Transaction<'a, Postgres>),
}

impl MediaReferenceWriteLock<'_> {
    /// Commits the held transaction and releases its media-reference lock.
    ///
    /// # Errors
    ///
    /// Returns an error if committing the held transaction fails.
    pub async fn commit(self) -> Result<(), sqlx::Error> {
        match self.held {
            HeldMediaReferenceWrite::Sqlite(mut conn) => {
                sqlx::query("COMMIT").execute(&mut *conn).await?;
            }
            HeldMediaReferenceWrite::Postgres(tx) => tx.commit().await?,
        }
        Ok(())
    }

    /// Rolls back the held transaction and releases its media-reference lock.
    ///
    /// # Errors
    ///
    /// Returns an error if rolling back the held transaction fails.
    pub async fn rollback(self) -> Result<(), sqlx::Error> {
        match self.held {
            HeldMediaReferenceWrite::Sqlite(mut conn) => {
                sqlx::query("ROLLBACK").execute(&mut *conn).await?;
            }
            HeldMediaReferenceWrite::Postgres(tx) => tx.rollback().await?,
        }
        Ok(())
    }
}

/// A held post write lock, from [`CloseablePool::lock_post_for_write`].
///
/// **The two arms do not behave the same on drop.** The Postgres arm is a real
/// `Transaction`, which rolls back when dropped. The `SQLite` arm's
/// `BEGIN IMMEDIATE` was issued as a raw statement, so sqlx's transaction-depth
/// tracking never saw it: dropping the guard returns the connection to the pool
/// **with the write transaction still open**, holding a database-wide write lock.
/// A test that panics between `lock_post_for_write` and
/// [`commit`](PostWriteLock::commit) therefore wedges the rest of that test's
/// writes rather than failing cleanly. Commit (or end the test) promptly.
pub struct PostWriteLock<'a> {
    /// The post the lock was taken for. Held so [`add_tag`](PostWriteLock::add_tag)
    /// cannot be aimed at a post other than the one that is locked.
    post_id: PostId,
    held: HeldWrite<'a>,
}

/// The backend-specific half of a [`PostWriteLock`].
enum HeldWrite<'a> {
    Sqlite(PoolConnection<Sqlite>),
    Postgres(Transaction<'a, Postgres>),
}

impl PostWriteLock<'_> {
    /// Adds one tag to the locked post from inside the held lock — a rival
    /// writer, for tests that must interleave a competing write with a storage
    /// method.
    ///
    /// Two statements, not one: `post_tags` carries a foreign key to
    /// `tags(tag_id)`, so the tag row must exist before the join row can be
    /// inserted. Both statements are the shared constants the production
    /// reconcile uses, so this carries no SQL of its own — the arms differ only
    /// in which executor the guard is holding.
    ///
    /// # Errors
    ///
    /// Returns the `sqlx::Error` if either statement fails.
    pub async fn add_tag(&mut self, label: &TagLabel) -> Result<(), sqlx::Error> {
        let slug = label.slug();
        let post_id = self.post_id;
        match &mut self.held {
            HeldWrite::Sqlite(conn) => {
                let tag_id = sqlx::query_scalar::<_, TagId>(UPSERT_TAG_RETURNING_ID)
                    .bind(&slug)
                    .fetch_one(&mut **conn)
                    .await?;
                sqlx::query(INSERT_POST_TAG)
                    .bind(post_id)
                    .bind(tag_id)
                    .bind(label)
                    .execute(&mut **conn)
                    .await?;
            }
            HeldWrite::Postgres(tx) => {
                let tag_id = sqlx::query_scalar::<_, TagId>(UPSERT_TAG_RETURNING_ID)
                    .bind(&slug)
                    .fetch_one(&mut **tx)
                    .await?;
                sqlx::query(INSERT_POST_TAG)
                    .bind(post_id)
                    .bind(tag_id)
                    .bind(label)
                    .execute(&mut **tx)
                    .await?;
            }
        }
        Ok(())
    }

    /// Commits the held transaction, releasing the lock and persisting whatever
    /// was written through it.
    ///
    /// # Errors
    ///
    /// Returns the `sqlx::Error` if the commit fails.
    pub async fn commit(self) -> Result<(), sqlx::Error> {
        match self.held {
            HeldWrite::Sqlite(mut conn) => {
                sqlx::query("COMMIT").execute(&mut *conn).await?;
            }
            HeldWrite::Postgres(tx) => tx.commit().await?,
        }
        Ok(())
    }
}

/// A ready-to-use [`AppState`] plus the temp dir backing it. `base` doubles as
/// the media-storage root HTTP tests need on both backends, and on `SQLite` it
/// also holds the database file alive for the lifetime of the test.
pub struct TestEnv {
    pub state: Arc<AppState>,
    pub base: TestBase,
}

/// Owns a test's temp dir and, on Postgres, a [`PostgresDbGuard`] that drops the
/// per-test database on teardown so the ephemeral cluster's data dir does not
/// grow with the suite (the disk-exhaustion fix for issue #28). `Deref`s to the
/// inner `TempDir`, so existing `base.path()` and `&base` uses keep compiling
/// unchanged.
pub struct TestBase {
    dir: TempDir,
    /// The persistent identity returned by the opening path that built this harness.
    instance_id: crate::InstanceId,
    /// A clone of the pool behind [`TestEnv::state`], so tests can fault it
    /// ([`close_pool`](TestBase::close_pool)) or run raw SQL through it
    /// ([`pool`](TestBase::pool)).
    pool: CloseablePool,
    /// `Some` on Postgres (drops the per-test database on teardown); `None` on
    /// `SQLite`. Declared after `pool` so the pool drops first.
    _pg: Option<PostgresDbGuard>,
}

impl TestBase {
    fn sqlite(dir: TempDir, pool: SqlitePool, instance_id: crate::InstanceId) -> Self {
        Self {
            dir,
            instance_id,
            pool: CloseablePool::Sqlite(pool),
            _pg: None,
        }
    }

    fn postgres(
        dir: TempDir,
        pg: PostgresDbGuard,
        pool: PgPool,
        instance_id: crate::InstanceId,
    ) -> Self {
        Self {
            dir,
            instance_id,
            pool: CloseablePool::Postgres(pool),
            _pg: Some(pg),
        }
    }

    /// Injects a storage fault: closes the pool behind this env's [`AppState`],
    /// so the next query through any storage handle returns an `Internal` error.
    pub async fn close_pool(&self) {
        self.pool.close().await;
    }

    /// The pool behind this env's [`AppState`], for raw-SQL seed/inspect.
    #[must_use]
    pub fn pool(&self) -> &CloseablePool {
        &self.pool
    }

    /// The immutable identity created by the production opening path.
    #[must_use]
    pub fn instance_id(&self) -> &crate::InstanceId {
        &self.instance_id
    }
}

impl std::ops::Deref for TestBase {
    type Target = TempDir;

    fn deref(&self) -> &Self::Target {
        &self.dir
    }
}

/// File name (under `TestEnv::base`) holding the Postgres connection string for
/// the *per-test* database that [`AppState`] was migrated into. Raw-SQL tests
/// need this because `template_postgres_url` mints a *fresh* clone on every
/// call, so re-calling it would connect to a different (empty) database than
/// the one the state seeded. Recorded here (instead of a new `TestEnv` field)
/// to avoid breaking the many `let TestEnv { state, base } = ...` destructures.
/// Absent on `SQLite`, where raw access goes through the `base` temp dir directly.
pub const PG_URL_FILE: &str = "pg_test_url";

/// Returns the Postgres connection string recorded by [`Backend::setup`] for a
/// test's per-test database. Reuse this for raw-SQL pools so they see rows the
/// state already inserted.
///
/// # Panics
///
/// If called on a `SQLite` `TestEnv`, where no URL was recorded.
#[must_use]
pub fn recorded_postgres_url(base: &TempDir) -> String {
    std::fs::read_to_string(base.path().join(PG_URL_FILE))
        .expect("Postgres test URL not recorded; recorded_postgres_url is Postgres-only")
}

impl Backend {
    /// Builds a fresh [`TestEnv`] (an `AppState` plus its backing temp dir) for
    /// this backend: a `SQLite` file under a tempdir, or a per-test Postgres
    /// database cloned from the migrated template.
    ///
    /// # Panics
    ///
    /// If the database cannot be opened/migrated (e.g. Postgres is unreachable
    /// or `JAUNDER_PG_TEST_URL` is misconfigured) — a setup failure fails the test.
    pub async fn setup(self) -> TestEnv {
        let dir = TempDir::new().unwrap();
        let runtime = StorageRuntimeConfig::default();
        let (state, base) = match self {
            Backend::Sqlite => {
                let DbConnectOptions::Sqlite(options) = sqlite_url(&dir) else {
                    unreachable!("sqlite_url always yields Sqlite")
                };
                let (state, pool, instance_id) =
                    crate::sqlite::open_sqlite_database_with_pool(&options, true, &runtime)
                        .await
                        .unwrap();
                (state, TestBase::sqlite(dir, pool, instance_id))
            }
            Backend::Postgres => {
                let config = PostgresTestConfig::from_env();
                let (url, guard) = template_postgres_url(&config).await;
                // template_postgres_url() always yields Postgres, so unreachable.
                let DbConnectOptions::Postgres { options, .. } = &url else {
                    unreachable!("template_postgres_url always yields Postgres")
                };
                let (state, pool, instance_id) =
                    crate::postgres::open_postgres_database_with_pool(options, &runtime)
                        .await
                        .unwrap();
                // Record the per-test DB URL so raw-SQL helpers reuse this exact
                // database rather than minting a fresh (empty) template clone.
                // `expose_url`, not `to_string`: this URL is read back by
                // `recorded_postgres_url` and reconnected with, so it must keep any
                // password. `Display` redacts.
                std::fs::write(dir.path().join(PG_URL_FILE), url.expose_url())
                    .expect("write recorded Postgres URL");
                (state, TestBase::postgres(dir, guard, pool, instance_id))
            }
        };
        TestEnv { state, base }
    }
}

/// Resolved `PostgreSQL` test-harness inputs. Construct this once at a test setup
/// boundary and pass it through provisioning and teardown.
#[derive(Clone)]
pub struct PostgresTestConfig {
    test_url: String,
    bootstrap_url: String,
}

impl PostgresTestConfig {
    /// Resolves the inherited `PostgreSQL` test URLs before asynchronous setup.
    #[must_use]
    pub fn from_env() -> Self {
        let test_url = std::env::var("JAUNDER_PG_TEST_URL")
            .unwrap_or_else(|_| "postgres://jaunder@127.0.0.1:55432/jaunder".to_owned());
        Self::from_raw(
            test_url,
            std::env::var("JAUNDER_PG_BOOTSTRAP_TEST_URL").ok(),
        )
    }

    fn from_raw(test_url: String, explicit_bootstrap_url: Option<String>) -> Self {
        let bootstrap_url = bootstrap_url(explicit_bootstrap_url, &test_url);
        Self {
            test_url,
            bootstrap_url,
        }
    }

    /// The application-role URL used to create per-test databases.
    #[must_use]
    pub fn test_url(&self) -> &str {
        &self.test_url
    }

    /// The superuser URL used to create and remove per-test databases.
    #[must_use]
    pub fn bootstrap_url(&self) -> &str {
        &self.bootstrap_url
    }

    /// The bootstrap connection's `host:port` authority.
    #[must_use]
    pub fn bootstrap_authority(&self) -> String {
        postgres_url_authority(&self.bootstrap_url)
    }
}

#[template]
#[export]
#[rstest]
#[case::sqlite(Backend::Sqlite)]
pub fn sqlite_only(#[case] backend: Backend) {}

#[template]
#[export]
#[rstest]
#[case::postgres(Backend::Postgres)]
pub fn postgres_only(#[case] backend: Backend) {}

// `#[export]` adds `#[macro_export]` to the generated template macro so it is
// reachable at this crate's root and `#[apply]`-able from *other* crates
// (`server`'s test crate, via the `storage::test_support` re-export). Without it
// the macro is `pub(crate)` and a cross-crate `use storage::test_support::backends`
// fails with "private macro".
#[template]
#[export]
#[rstest]
#[case::sqlite(Backend::Sqlite)]
#[case::postgres(Backend::Postgres)]
pub fn backends(#[case] backend: Backend) {}

/// Dual-backend matrix template: a `#[values]`-based backend axis that composes
/// with a test's own local `#[case]`/`#[values]` matrix (the `#[case]`-based
/// `backends` template cannot — its case rows collide with local case rows).
#[template]
#[export]
#[rstest]
pub fn backends_matrix(#[values(Backend::Sqlite, Backend::Postgres)] backend: Backend) {}

/// The `SQLite` connect options for a `test.db` under `base`.
///
/// # Panics
///
/// If the constructed `sqlite:` URL fails to parse.
#[must_use]
pub fn sqlite_url(base: &TempDir) -> DbConnectOptions {
    format!("sqlite:{}", base.path().join("test.db").display())
        .parse()
        .unwrap()
}

/// Pure core of [`PostgresTestConfig::from_env`]: the `explicit` bootstrap URL
/// when set, else a `postgres` superuser URL on the same authority as `test_url`.
fn bootstrap_url(explicit: Option<String>, test_url: &str) -> String {
    explicit.unwrap_or_else(|| {
        let authority = postgres_url_authority(test_url);
        format!("postgres://postgres@{authority}/postgres")
    })
}

fn postgres_url_authority(url: &str) -> String {
    let without_scheme = url
        .strip_prefix("postgres://")
        .or_else(|| url.strip_prefix("postgresql://"))
        .unwrap_or(url);
    let after_credentials = without_scheme
        .rsplit_once('@')
        .map_or(without_scheme, |(_, authority_and_path)| authority_and_path);
    after_credentials
        .split('/')
        .next()
        .expect("bootstrap URL should include an authority")
        .to_owned()
}

fn postgres_url_with_db_name(config: &PostgresTestConfig, db_name: &str) -> String {
    splice_db_name(config.test_url(), db_name)
}

/// Pure core of [`postgres_url_with_db_name`]: replace the database segment of
/// `template` with `db_name`, preserving any `?query`. Kept separate so the
/// with-query and without-query arms are unit-testable.
fn splice_db_name(template: &str, db_name: &str) -> String {
    let (base, query) = template
        .split_once('?')
        .map_or((template, None), |(base, query)| (base, Some(query)));
    let (prefix, _) = base
        .rsplit_once('/')
        .expect("PostgreSQL test URL should include a database name");
    match query {
        Some(query) => format!("{prefix}/{db_name}?{query}"),
        None => format!("{prefix}/{db_name}"),
    }
}

fn unique_postgres_db_name() -> String {
    static COUNTER: AtomicU64 = AtomicU64::new(0);

    let suffix = COUNTER.fetch_add(1, Ordering::Relaxed);
    let timestamp = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .expect("system clock before unix epoch")
        .as_nanos();
    // nextest runs each test in its own process, so `COUNTER` (and thus
    // `suffix`) restarts at 0 per process; the nanosecond timestamp alone can
    // collide when two parallel test processes start within the same tick. The
    // process id makes the name unique across processes regardless of clock
    // resolution.
    let pid = std::process::id();
    format!("jaunder_test_{timestamp}_{pid}_{suffix}")
}

/// Best-effort `DROP DATABASE <name> WITH (FORCE)` for a per-test clone.
///
/// Runs on a dedicated thread with its own current-thread runtime so it is safe
/// to call from `Drop` regardless of the ambient async context (a fresh thread
/// has no running Tokio runtime, so building one does not panic). The thread is
/// joined before returning, so the clone's disk is reclaimed before the next
/// test allocates. `WITH (FORCE)` (Postgres 13+) terminates any connections
/// still open to the clone, so teardown is robust to drop ordering relative to
/// the `AppState` pool. The drop is bounded by a timeout and never panics (it
/// runs inside `Drop`); a failed or timed-out drop is logged to stderr rather
/// than returned mutely, since a silently leaking clone is the disk-creep
/// regression this guards against.
fn drop_test_database(db_name: &str, bootstrap_url: &str) {
    let statement = format!("DROP DATABASE {} WITH (FORCE)", quote_identifier(db_name));
    std::thread::scope(|scope| {
        scope.spawn(|| {
            let Ok(runtime) = tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
            else {
                return; // cov:ignore — current-thread runtime build only fails under OOM
            };
            runtime.block_on(async {
                let Ok(options) = bootstrap_url.parse::<sqlx::postgres::PgConnectOptions>() else {
                    return; // cov:ignore — bootstrap URL is always a valid Postgres URL
                };
                let outcome = tokio::time::timeout(std::time::Duration::from_secs(10), async {
                    let mut conn = sqlx::PgConnection::connect_with(&options).await?;
                    let dropped = sqlx::query(&statement).execute(&mut conn).await.map(|_| ());
                    let _ = conn.close().await;
                    dropped
                })
                .await;
                report_drop_outcome(db_name, outcome);
            });
        });
    });
}

/// Logs the outcome of the best-effort per-test database drop. Split out of
/// [`drop_test_database`] so its failure/timeout arms — which fire only when a
/// `DROP DATABASE` errors or exceeds the timeout, never in a normal run — can be
/// `// cov:ignore`-marked at an indentation where the marker fits on the line.
fn report_drop_outcome(
    db_name: &str,
    outcome: Result<Result<(), sqlx::Error>, tokio::time::error::Elapsed>,
) {
    match outcome {
        Ok(Ok(())) => {}
        Ok(Err(error)) => eprintln!("test database drop {db_name} failed: {error}"),
        Err(_elapsed) => eprintln!("test database drop {db_name} timed out"),
    }
}

/// RAII owner of a per-test Postgres database created by [`unique_postgres_url`]
/// (or [`template_postgres_url`]). Dropping it removes the database via
/// [`drop_test_database`], so the ephemeral cluster's data dir does not grow with
/// the suite. This is the single teardown primitive; [`TestBase`] composes it.
pub struct PostgresDbGuard {
    db_name: String,
    bootstrap_url: String,
}

impl Drop for PostgresDbGuard {
    fn drop(&mut self) {
        drop_test_database(&self.db_name, &self.bootstrap_url);
    }
}

/// A connect URL naming a per-test database that has **not** been created — for
/// tests that exercise the "database is absent" path.
///
/// # Panics
///
/// If the constructed URL fails to parse.
#[must_use]
pub fn nonexistent_postgres_url(config: &PostgresTestConfig) -> DbConnectOptions {
    postgres_url_with_db_name(config, &unique_postgres_db_name())
        .parse()
        .unwrap()
}

/// Creates a fresh, empty per-test Postgres database and returns its connect URL.
///
/// # Panics
///
/// If the test URL lacks a username, or the admin connection / `CREATE DATABASE`
/// fails.
pub async fn unique_postgres_url(
    config: &PostgresTestConfig,
) -> (DbConnectOptions, PostgresDbGuard) {
    let db_name = unique_postgres_db_name();

    let bootstrap: sqlx::postgres::PgConnectOptions = config.bootstrap_url().parse().unwrap();
    let DbConnectOptions::Postgres { options, .. } = config.test_url().parse().unwrap() else {
        unreachable!("PostgreSQL test URL always yields PostgreSQL options")
    };
    let owner = options.get_username();
    assert!(
        !owner.is_empty(),
        "PostgreSQL test URL must include a username"
    );

    let mut admin_conn = sqlx::PgConnection::connect_with(&bootstrap).await.unwrap();
    sqlx::query(&format!(
        "CREATE DATABASE {} OWNER {}",
        quote_identifier(&db_name),
        quote_identifier(owner),
    ))
    .execute(&mut admin_conn)
    .await
    .unwrap();

    let options = postgres_url_with_db_name(config, &db_name).parse().unwrap();
    (
        options,
        PostgresDbGuard {
            db_name,
            bootstrap_url: config.bootstrap_url().to_owned(),
        },
    )
}

/// Name of the once-migrated template database that per-test databases are
/// cloned from. Cloning via `CREATE DATABASE ... TEMPLATE` block-copies an
/// already-migrated schema, so each test pays a fast copy instead of re-running
/// every migration.
const TEMPLATE_DB: &str = "jaunder_test_template";

/// Advisory-lock key serialising template creation across nextest's
/// process-per-test workers. The first worker migrates the template; the rest
/// see it already exists and skip straight to cloning.
const TEMPLATE_LOCK_KEY: i64 = 78_316_621;

/// Ensures [`TEMPLATE_DB`] exists and is fully migrated. Safe to call
/// concurrently from many processes: creation is guarded by a session-level
/// advisory lock taken on the bootstrap connection.
async fn ensure_template_db(config: &PostgresTestConfig) {
    let bootstrap: sqlx::postgres::PgConnectOptions = config.bootstrap_url().parse().unwrap();
    let mut admin = sqlx::PgConnection::connect_with(&bootstrap).await.unwrap();

    sqlx::query("SELECT pg_advisory_lock($1)")
        .bind(TEMPLATE_LOCK_KEY)
        .execute(&mut admin)
        .await
        .unwrap();

    let exists: bool =
        sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM pg_database WHERE datname = $1)")
            .bind(TEMPLATE_DB)
            .fetch_one(&mut admin)
            .await
            .unwrap();

    if !exists {
        let DbConnectOptions::Postgres { options, .. } = config.test_url().parse().unwrap() else {
            unreachable!("PostgreSQL test URL always yields PostgreSQL options")
        };
        let owner = options.get_username();
        sqlx::query(&format!(
            "CREATE DATABASE {} OWNER {}",
            quote_identifier(TEMPLATE_DB),
            quote_identifier(owner),
        ))
        .execute(&mut admin)
        .await
        .unwrap();

        // Migrate the template through its own pool, then close it: a database
        // can only serve as a CREATE DATABASE template when nobody is connected
        // to it.
        let pool = sqlx::PgPool::connect(&postgres_url_with_db_name(config, TEMPLATE_DB))
            .await
            .unwrap();
        sqlx::migrate!("../storage/migrations/postgres")
            .run(&pool)
            .await
            .unwrap();
        pool.close().await;
    }

    sqlx::query("SELECT pg_advisory_unlock($1)")
        .bind(TEMPLATE_LOCK_KEY)
        .execute(&mut admin)
        .await
        .unwrap();
}

/// Creates a fresh, already-migrated per-test database cloned from the template
/// and returns its connection options. Owned by the same role as the configured
/// test URL so the application user can access every cloned object.
///
/// # Panics
///
/// If template setup, the admin connection, or the `CREATE DATABASE` clone fails.
pub async fn template_postgres_url(
    config: &PostgresTestConfig,
) -> (DbConnectOptions, PostgresDbGuard) {
    ensure_template_db(config).await;

    let DbConnectOptions::Postgres { options, .. } = config.test_url().parse().unwrap() else {
        unreachable!("PostgreSQL test URL always yields PostgreSQL options")
    };
    let owner = options.get_username();
    let db_name = unique_postgres_db_name();

    let bootstrap: sqlx::postgres::PgConnectOptions = config.bootstrap_url().parse().unwrap();
    let mut admin = sqlx::PgConnection::connect_with(&bootstrap).await.unwrap();
    sqlx::query(&format!(
        "CREATE DATABASE {} OWNER {} TEMPLATE {}",
        quote_identifier(&db_name),
        quote_identifier(owner),
        quote_identifier(TEMPLATE_DB),
    ))
    .execute(&mut admin)
    .await
    .unwrap();

    let options = postgres_url_with_db_name(config, &db_name).parse().unwrap();
    (
        options,
        PostgresDbGuard {
            db_name,
            bootstrap_url: config.bootstrap_url().to_owned(),
        },
    )
}

/// Default mailer for tests that don't care about email sending.
#[must_use]
pub fn noop_mailer() -> Arc<dyn MailSender> {
    Arc::new(NoopMailSender)
}

/// Seeds `count` posts for `user_id` directly through the storage service,
/// bypassing the HTTP/server-fn path (markdown render of trivial bodies is
/// negligible; the cost we avoid is axum routing + `server_fn` per call).
/// Parses `s` into the canonical [`FeedPath`] identity key. The one shared
/// feed-path constructor for both the `storage` crate's tests and `server`'s
/// integration tests, so the `"…".parse().expect(…)` shape lives in one place.
///
/// # Panics
///
/// If `s` is not a valid canonical feed path.
#[must_use]
pub fn fp(s: &str) -> FeedPath {
    s.parse().expect("valid feed path")
}

/// Parse `s` into a valid [`InviteCode`] for tests — the single place a test
/// invite-code literal is parsed. Lives here rather than `common::test_support`
/// because `InviteCode` is a `host` type (`common` cannot name it), and `storage`
/// depends on `host`, so this is reachable from every `storage` test module.
///
/// # Panics
///
/// Panics if `s` is not a validly-shaped invite code.
#[must_use]
pub fn parse_invite_code(s: &str) -> InviteCode {
    s.parse().expect("valid test invite code")
}

/// `published == true` sets `published_at = now` so list/timeline endpoints
/// return them; `false` leaves them as drafts. Returns ids in creation order.
///
/// # Panics
///
/// If a slug fails to parse or a post fails to persist.
pub async fn seed_posts(
    state: &Arc<AppState>,
    user_id: UserId,
    count: usize,
    published: bool,
) -> Vec<PostId> {
    let inputs: Vec<_> = (0..count)
        .map(|i| {
            crate::seed_post_input(
                user_id,
                parse_slug(&format!("seed-{i}")),
                parse_post_body(&format!("# Post {i}\n\nbody")),
                published,
            )
        })
        .collect();
    state
        .posts
        .create_posts(&inputs)
        .await
        .expect("seed posts should be created")
}

/// A user seeded by [`SeedUser::seed`] — its id plus its **autogenerated**
/// username. Tests that need the name read `.username` (never a literal).
pub struct SeededUser {
    pub user_id: UserId,
    pub username: Username,
}

/// Monotonic sequence behind [`SeedUser`]'s autogenerated usernames. Private; no
/// test touches it. Correctness rests on the fresh-DB-per-test invariant (see
/// [`SeedUser`]).
static SEED_SEQ: AtomicU64 = AtomicU64::new(0);

/// Fixture for a seeded user, built the real `UserStorage::create_user` way. The
/// username is **autogenerated** (`user{n}`) and returned on [`SeededUser`]; a
/// test never picks or passes a name — it reads `.username` if it needs one.
/// Defaults: password `password123`, no display name, non-operator — chain the
/// setters to override only what a test varies. [`SeedUser::seed`] `expect()`s
/// success, so it is happy-path setup only; error-path tests (duplicate username,
/// hash failure) call `create_user` directly and assert the error.
///
/// **Invariant:** the autogenerated names are unique only *per process*. Because
/// nextest runs process-per-test and every `SeedUser`-using test provisions a
/// fresh per-test database (`TestEnv`/`Backend::setup`, ADR-0033/0053), that is
/// exactly one counter per DB — so names never collide. The e2e suite (the one
/// shared-DB context) seeds through the `test-support` crate, not `SeedUser`. If
/// the invariant were ever broken (two counters-from-0 into one DB), the second
/// `create_user` returns `UsernameTaken` and `seed()` panics — a loud failure,
/// never silent corruption.
pub struct SeedUser<'a> {
    password: &'a str,
    display_name: Option<&'a str>,
    is_operator: bool,
}

impl Default for SeedUser<'_> {
    fn default() -> Self {
        Self::new()
    }
}

impl<'a> SeedUser<'a> {
    /// A non-operator user, password `password123`, no display name, autogenerated
    /// username. (Hand-written, not derived: `&str`'s `Default` is `""`, which
    /// would wipe the password default.)
    #[must_use]
    pub fn new() -> Self {
        Self {
            password: "password123",
            display_name: None,
            is_operator: false,
        }
    }

    /// Override the password (auth/duplicate tests).
    #[must_use]
    pub fn password(mut self, password: &'a str) -> Self {
        self.password = password;
        self
    }

    /// Set a display name.
    #[must_use]
    pub fn display_name(mut self, display_name: &'a str) -> Self {
        self.display_name = Some(display_name);
        self
    }

    /// Mark the user an operator.
    #[must_use]
    pub fn operator(mut self) -> Self {
        self.is_operator = true;
        self
    }

    /// Create the user and return its id + autogenerated username.
    ///
    /// # Panics
    ///
    /// If the password/display name fail to parse or the user cannot be created.
    pub async fn seed(self, state: &Arc<AppState>) -> SeededUser {
        let n = SEED_SEQ.fetch_add(1, Ordering::Relaxed);
        let username = parse_username(&format!("user{n}"));
        let display_name = self.display_name.map(parse_display_name);
        let user_id = state
            .users
            .create_user(
                &username,
                &host::test_support::parse_password(self.password),
                display_name.as_ref(),
                self.is_operator,
            )
            .await
            .expect("seed user should be created");
        SeededUser { user_id, username }
    }
}

/// Seed `N` distinct users (autogenerated names) and return their ids — for tests
/// that need several users and only care about their identities. Destructure it:
/// `let [alice, bob] = seed_users(&state).await;`.
///
/// # Panics
///
/// If any user cannot be created.
pub async fn seed_users<const N: usize>(state: &Arc<AppState>) -> [UserId; N] {
    let mut ids = Vec::with_capacity(N);
    for _ in 0..N {
        ids.push(SeedUser::new().seed(state).await.user_id);
    }
    ids.try_into().expect("seeded exactly N users")
}

/// A single seeded post, built the real [`perform_post_creation`](crate::perform_post_creation)
/// way — the same service-layer path production uses (renders the body, generates a
/// unique slug via collision-retry, re-reads the row). Aggressively defaulted: a
/// **published, public, Markdown** post with a fixed non-empty body, so the
/// overwhelming majority of call sites are the bare `SeedPost::new(user_id).seed(&state)`
/// and a setter appears only where a test asserts on (or requires) that field — the
/// [`SeedUser`] discipline.
///
/// Distinct from [`seed_posts`] (batch, generic `seed-{i}` posts) and from the
/// `create_post`-layer literals the storage-contract tests hand-roll (#656): those seed
/// *below* `perform_post_creation` and control `rendered_html`/slug explicitly, which
/// this builder deliberately does not.
///
/// Repeated bare seeds get **distinct** slugs: a title-less post derives its slug from
/// the fixed body, and `perform_post_creation`'s collision-suffix retry disambiguates
/// (`seeded-post-body`, `seeded-post-body-2`, …).
pub struct SeedPost {
    user_id: UserId,
    title: Option<PostTitle>,
    body: PostBody,
    audiences: Vec<AudienceTarget>,
}

impl SeedPost {
    /// A published, public, Markdown post owned by `user_id`, with a fixed non-empty
    /// body and no explicit title. Deviate from a default only where a test requires
    /// it. Only the three fields real call sites vary — title, body, audiences — are
    /// settable; the rest (Markdown, published-now, no slug/summary/idempotency) are
    /// fixed defaults, mirroring how `SeedUser` exposes only the setters its callers use.
    #[must_use]
    pub fn new(user_id: UserId) -> Self {
        Self {
            user_id,
            title: None,
            body: parse_post_body("Seeded post body"),
            audiences: vec![AudienceTarget::Public],
        }
    }

    /// Set an explicit title — for the permalink/listing tests that assert on it.
    #[must_use]
    pub fn title(mut self, title: PostTitle) -> Self {
        self.title = Some(title);
        self
    }

    /// Override the default body.
    #[must_use]
    pub fn body(mut self, body: PostBody) -> Self {
        self.body = body;
        self
    }

    /// Replace the default `[Public]` audience targeting.
    #[must_use]
    pub fn audiences(mut self, audiences: Vec<AudienceTarget>) -> Self {
        self.audiences = audiences;
        self
    }

    /// Persist via [`perform_post_creation`](crate::perform_post_creation)
    /// (`max_attempts = 100`) and return the re-read [`PostRecord`] (carries `post_id`
    /// and `slug`).
    ///
    /// # Panics
    ///
    /// If the post cannot be created — happy-path setup only, like [`SeedUser::seed`].
    pub async fn seed(self, state: &Arc<AppState>) -> PostRecord {
        crate::perform_post_creation(
            state.posts.as_ref(),
            crate::PostCreation {
                user_id: self.user_id,
                body: self.body,
                title: self.title.as_ref(),
                format: PostFormat::Markdown,
                slug_override: None,
                published_at: Some(UtcInstant::now()),
                max_attempts: 100,
                summary: None,
                audiences: self.audiences,
                idempotency_key: None,
                expectations: PostBookkeepingExpectation::default(),
            },
        )
        .await
        .expect("seed post should be created")
    }
}

/// A post seeded by [`SeedRawPost`] — its id plus the values a test reads back instead of
/// hardcoding a literal (mirrors [`SeededUser`]). `body` is never read back, so it is not
/// carried here; `rendered_html` is the resolved `render(body)` (one page-render assertion
/// site embeds it); `published_at` is `None` for a `.draft()`.
#[derive(Debug)]
pub struct SeededPost {
    pub post_id: PostId,
    pub slug: Slug,
    pub title: PostTitle,
    pub published_at: Option<UtcInstant>,
    // rendered-html-from-trusted:allow over-included test-support seed carries render(body) output for assertions (#701)
    pub rendered_html: RenderedHtml,
}

/// Monotonic sequence behind [`SeedRawPost`]'s autogenerated slug + title. Private and
/// per-process; correctness rests on the same fresh-DB-per-test invariant [`SeedUser`]
/// documents (nextest runs process-per-test, so one counter serves one DB and the
/// `(user, slug, day)` uniqueness never trips on an accidental collision).
static RAW_POST_SEQ: AtomicU64 = AtomicU64::new(0);

/// Builder that seeds a post **directly through the `create_post` storage layer** — no
/// slug-retry, no service-layer massaging — a sibling to the service-layer post seeder
/// and distinct from the batch [`seed_posts`]. It defaults every field (autogenerated
/// unique slug `post-{n}` + title `"Post {n}"`, a fixed non-empty Markdown body,
/// `render(body)` HTML, published-now, Public); a call site overrides only what it varies.
///
/// `.seed`/`.create` return a [`SeededPost`] so a test reads back the autogenerated
/// slug/title (etc.) rather than owning a literal; `.seed` `expect()`s success like
/// [`SeedUser::seed`], while `.create` hands back the `Result` for the conflict/FK tests
/// that assert the `Err`. `.build` yields the raw [`CreatePostInput`] for the batch tests.
///
/// There is deliberately **no** `.title`/`.idempotency_key`/`.rendered_html` setter: no
/// adopting site chooses a title, sets an idempotency key, or supplies rendered HTML — the
/// builder renders `body` with the production [`render`], so the HTML is always derived.
pub struct SeedRawPost {
    user_id: UserId,
    slug: Option<Slug>,
    body: PostBody,
    format: PostFormat,
    published_at: Option<UtcInstant>,
    summary: Option<PostSummary>,
    audiences: Vec<AudienceTarget>,
    tags: Vec<TagLabel>,
}

impl SeedRawPost {
    /// A published, Public, Markdown post owned by `user_id`, with an autogenerated unique
    /// slug + title and a fixed non-empty body.
    #[must_use]
    pub fn new(user_id: UserId) -> Self {
        Self {
            user_id,
            slug: None,
            body: parse_post_body("seed body"),
            format: PostFormat::Markdown,
            published_at: Some(UtcInstant::now()),
            summary: None,
            audiences: vec![AudienceTarget::Public],
            tags: Vec::new(),
        }
    }

    /// Force a specific slug — for conflict *sameness* (`.slug(other.slug.as_ref())`) or a
    /// slug a test lists / looks up by.
    #[must_use]
    pub fn slug(mut self, slug: impl AsRef<str>) -> Self {
        self.slug = Some(parse_slug(slug.as_ref()));
        self
    }

    /// Override the body (e.g. embed a media URL). The rendered HTML re-derives from it.
    #[must_use]
    pub fn body(mut self, body: PostBody) -> Self {
        self.body = body;
        self
    }

    /// Override the markup format (the rendered HTML re-derives accordingly).
    #[must_use]
    pub fn format(mut self, format: PostFormat) -> Self {
        self.format = format;
        self
    }

    /// Attach a summary/excerpt.
    #[must_use]
    pub fn summary(mut self, summary: PostSummary) -> Self {
        self.summary = Some(summary);
        self
    }

    /// Replace the audience targeting (default `[Public]`).
    #[must_use]
    pub fn audiences(mut self, audiences: Vec<AudienceTarget>) -> Self {
        self.audiences = audiences;
        self
    }

    /// Seed as a draft (`published_at = None`).
    #[must_use]
    pub fn draft(mut self) -> Self {
        self.published_at = None;
        self
    }

    /// Seed with an exact publication instant (scheduled / backdated / go-live-window).
    #[must_use]
    pub fn published_at(mut self, at: UtcInstant) -> Self {
        self.published_at = Some(at);
        self
    }

    /// Tags applied via `set_post_tags` right after the insert by `.seed`/`.create`. Ignored by
    /// `.build` (which does not write) — guarded by a `debug_assert!`.
    #[must_use]
    pub fn tags<I, S>(mut self, tags: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: AsRef<str>,
    {
        self.tags = tags
            .into_iter()
            .map(|t| parse_tag_label(t.as_ref()))
            .collect();
        self
    }

    /// Resolve the autogenerated slug/title and rendered HTML into a [`CreatePostInput`]
    /// without writing. The batch tests build a `Vec` from this and read `input.slug` /
    /// `input.title` off it.
    ///
    /// # Panics
    ///
    /// Debug-asserts that no `.tags()` were set (they cannot be applied without a write —
    /// use `.seed()`/`.create()`).
    #[must_use]
    pub fn build(self) -> CreatePostInput {
        debug_assert!(
            self.tags.is_empty(),
            ".tags() is ignored by build(); apply tags via .seed()/.create()"
        );
        self.into_input()
    }

    fn into_input(self) -> CreatePostInput {
        let n = RAW_POST_SEQ.fetch_add(1, Ordering::Relaxed);
        let slug = self
            .slug
            .unwrap_or_else(|| parse_slug(&format!("post-{n}")));
        let title = parse_post_title(&format!("Post {n}"));
        let rendered = RenderOutput::render(&self.body, &self.format);
        CreatePostInput {
            user_id: self.user_id,
            title: Some(title),
            slug,
            body: self.body,
            format: self.format,
            rendered,
            published_at: self.published_at,
            summary: self.summary,
            audiences: self.audiences,
            expectations: PostBookkeepingExpectation::default(),
            idempotency_key: None,
        }
    }

    /// Write via `create_post`, apply any `.tags()`, and return the [`SeededPost`]. The
    /// error-path tests (slug conflict, foreign-key violation) call this and assert the
    /// `Err` rather than `expect`-ing success.
    ///
    /// # Errors
    ///
    /// Propagates the [`CreatePostError`] from `create_post`.
    ///
    /// # Panics
    ///
    /// If applying the `.tags()` labels via `set_post_tags` fails (tagging is happy-path setup).
    pub async fn create(mut self, state: &Arc<AppState>) -> Result<SeededPost, CreatePostError> {
        let tags = std::mem::take(&mut self.tags);
        let input = self.into_input();
        let post_id = state.posts.create_post(&input).await?;
        // The `is_empty` guard is safe only here: the post was just created, so
        // clearing and no-op coincide. `set_post_tags(id, &[])` on an existing
        // post means *clear* (#771 D11).
        if !tags.is_empty() {
            state
                .posts
                .set_post_tags(post_id, &tags)
                .await
                .expect("seed set_post_tags should succeed");
        }
        Ok(SeededPost {
            post_id,
            slug: input.slug,
            title: input
                .title
                .expect("SeedRawPost always autogenerates a title"),
            published_at: input.published_at,
            rendered_html: input.rendered.into_html(),
        })
    }

    /// Happy-path seed: `create` + `expect`, like [`SeedUser::seed`].
    ///
    /// # Panics
    ///
    /// If the post cannot be created.
    pub async fn seed(self, state: &Arc<AppState>) -> SeededPost {
        self.create(state)
            .await
            .expect("seed raw post should be created")
    }
}

/// Builder for an [`UpdatePostInput`] — the edit-side sibling of [`SeedRawPost`], with the
/// same defaults-plus-overrides shape. An update test typically varies one or two fields;
/// this builder defaults the rest so a test overrides only what it means.
///
/// Defaults: title `"Updated Title"`, body `"updated body"`, Markdown, no summary, `[Public]`,
/// and [`PublishUpdate::Publish`] without an explicit timestamp, which keeps an existing
/// publication timestamp or stamps `now` for a previously-unpublished Post. A test that
/// unpublishes says so with [`unpublish`][Self::unpublish]. The slug is the one required
/// argument because an update's slug is what collides (or does not) with a sibling Post, so
/// no default could be right.
///
/// `rendered` has no setter: [`build`][Self::build] derives it from `body`/`format` with the
/// production [`RenderOutput::render`], exactly as `SeedRawPost` does, so no call site
/// re-spells the render and no input can carry a reference set that disagrees with its HTML
/// (#711).
///
/// `Clone` is load-bearing: the audience tests vary one field off a shared base via
/// `..base.clone()` struct-update spreads.
#[derive(Clone)]
pub struct UpdateRawPost {
    title: Option<PostTitle>,
    slug: Slug,
    body: PostBody,
    format: PostFormat,
    publish: PublishUpdate,
    summary: Option<PostSummary>,
    audiences: Vec<AudienceTarget>,
}

impl UpdateRawPost {
    /// A titled, Public, Markdown edit at `slug` that leaves publication alone.
    #[must_use]
    pub fn new(slug: impl AsRef<str>) -> Self {
        Self {
            title: Some(parse_post_title("Updated Title")),
            slug: parse_slug(slug.as_ref()),
            body: parse_post_body("updated body"),
            format: PostFormat::Markdown,
            publish: PublishUpdate::Publish { at: None },
            summary: None,
            audiences: vec![AudienceTarget::Public],
        }
    }

    /// Override the title a test reads back.
    #[must_use]
    pub fn title(mut self, title: &str) -> Self {
        self.title = Some(parse_post_title(title));
        self
    }

    /// Override the body (the rendered HTML and its media references re-derive from it).
    #[must_use]
    pub fn body(mut self, body: PostBody) -> Self {
        self.body = body;
        self
    }

    /// Override the markup format (the rendered HTML re-derives accordingly).
    #[must_use]
    pub fn format(mut self, format: PostFormat) -> Self {
        self.format = format;
        self
    }

    /// Clear `published_at` back to NULL (draft / unschedule).
    #[must_use]
    pub fn unpublish(mut self) -> Self {
        self.publish = PublishUpdate::Unpublish;
        self
    }

    /// Set — or, with `None`, clear — the summary/excerpt. Takes `impl Into<Option<_>>` so a
    /// test that only ever sets one reads like [`SeedRawPost::summary`], while the
    /// set-then-clear test passes its `Option` straight through.
    #[must_use]
    pub fn summary(mut self, summary: impl Into<Option<PostSummary>>) -> Self {
        self.summary = summary.into();
        self
    }

    /// Replace the audience targeting (default `[Public]`).
    #[must_use]
    pub fn audiences(mut self, audiences: Vec<AudienceTarget>) -> Self {
        self.audiences = audiences;
        self
    }

    /// Resolve into the [`UpdatePostInput`] to hand `update_post`, rendering `body` here.
    #[must_use]
    pub fn build(self) -> UpdatePostInput {
        let rendered = RenderOutput::render(&self.body, &self.format);
        UpdatePostInput {
            title: self.title,
            slug: self.slug,
            body: self.body,
            format: self.format,
            rendered,
            publish: self.publish,
            summary: self.summary,
            audiences: self.audiences,
            request_clock: UtcInstant::now(),
            expectations: PostBookkeepingExpectation::default(),
        }
    }
}

/// The content hash every media fixture is stored under, re-exported from
/// [`common::test_support`] so `common`'s media-layout tests and this crate's fixtures
/// share one digest rather than restating it. Re-exported (rather than reached for
/// directly) because it is part of what a fixture caller expects from this module, next
/// to [`media_ref_for`]; public because a test spelling the `AtomPub` member layout
/// (`/atompub/<user>/media/<sha>/<name>`) needs the digest itself, not a serve URL.
pub use common::test_support::MEDIA_TEST_SHA256;

/// The [`MediaRef`] naming the fixture entry called `name`.
///
/// `name` is the **raw** name a person types; it goes through
/// [`Filename::sanitized`] — the upload-intake door — so a fixture spelling
/// `"my photo.jpg"` yields the stored `my%20photo.jpg` and a test never hand-encodes.
///
/// # Panics
///
/// If `name` is not a usable filename leaf.
#[must_use]
pub fn media_ref_for(name: &str) -> MediaRef {
    MediaRef {
        source: MediaSource::Upload,
        sha256: parse_content_hash(MEDIA_TEST_SHA256),
        filename: Filename::sanitized(name).expect("valid test media filename"),
    }
}

/// The canonical serve URL for `name` under the shared test digest — the single place
/// a test spells a media URL, composed by the production [`media_url`] rather than by
/// re-writing the layout.
#[must_use]
pub fn media_url_for(name: &str) -> String {
    let media = media_ref_for(name);
    media_url(&media.source, &media.sha256, &media.filename).to_string()
}

/// Seeds a `media` row owned by `user_id` for the fixture entry called `name`, and
/// returns the [`MediaRef`] naming it — the entry a post's `post_media` row resolves
/// to. Content type is derived from the name, as the real upload path derives it.
///
/// # Panics
///
/// If the row cannot be created — happy-path setup only, like [`SeedUser::seed`].
pub async fn seed_media(state: &Arc<AppState>, user_id: UserId, name: &str) -> MediaRef {
    let media = media_ref_for(name);
    state
        .media
        .create_media(&MediaRecord {
            user_id,
            sha256: media.sha256.clone(),
            filename: media.filename.clone(),
            source: media.source,
            content_type: detect_content_type(&media.filename),
            size_bytes: parse_byte_size("1"),
            source_url: None,
            created_at: UtcInstant::now(),
        })
        .await
        .expect("seed media should be created");
    media
}

/// Whether a `media` row exists for `user_id` and `media` — the ownership-scoped
/// existence question, asked through the real store rather than raw SQL.
///
/// # Panics
///
/// If the lookup fails.
pub async fn media_row_exists(state: &Arc<AppState>, user_id: UserId, media: &MediaRef) -> bool {
    state
        .media
        .get_media(user_id, &media.sha256, &media.filename, &media.source)
        .await
        .expect("media lookup should succeed")
        .is_some()
}
/// A post's `post_media` rows, ascending by media identity then origin.
///
/// # Panics
///
/// If the query fails, or a stored column is not a valid media identity or reference.
pub async fn fetch_post_media(
    base: &TestBase,
    post_id: PostId,
) -> Vec<(MediaRef, MediaReferenceKind, MediaReferenceForm)> {
    base.pool()
        .string_quintuples(&format!(
            "SELECT source, sha256, filename, reference_kind, reference_form FROM post_media \
             WHERE post_id = {post_id} ORDER BY source, sha256, filename, reference_kind, reference_form"
        ))
        .await
        .expect("post_media query should succeed")
        .into_iter()
        .map(|(source, sha256, filename, kind, form)| {
            (
                MediaRef {
                    source: source.parse().expect("valid media source"),
                    sha256: sha256.parse().expect("valid content hash"),
                    filename: filename.parse().expect("valid filename"),
                },
                kind.parse().expect("valid media reference kind"),
                form.parse().expect("valid media reference form"),
            )
        })
        .collect()
}

/// Creates a post through [`perform_post_creation`](crate::perform_post_creation) —
/// the same entry point `web::posts::create` uses — so a test exercises the product's
/// own path (render, extract, write) rather than a synthetic [`CreatePostInput`].
///
/// # Panics
///
/// If the post cannot be created.
pub async fn create_post_via_service(
    state: &Arc<AppState>,
    user_id: UserId,
    body: PostBody,
) -> PostId {
    create_via_service(state, user_id, body, Some(UtcInstant::now())).await
}

/// The unpublished twin of [`create_post_via_service`] — the draft a publication test
/// needs, created the same way.
///
/// # Panics
///
/// If the post cannot be created.
pub async fn create_draft_via_service(
    state: &Arc<AppState>,
    user_id: UserId,
    body: PostBody,
) -> PostId {
    create_via_service(state, user_id, body, None).await
}

/// Shared body of the two service-layer creators: everything but `published_at` is
/// fixed (public, Markdown, title derived from the body), as the two differ in exactly
/// that one field.
async fn create_via_service(
    state: &Arc<AppState>,
    user_id: UserId,
    body: PostBody,
    published_at: Option<UtcInstant>,
) -> PostId {
    crate::perform_post_creation(
        state.posts.as_ref(),
        crate::PostCreation {
            user_id,
            body,
            title: None,
            format: PostFormat::Markdown,
            slug_override: None,
            published_at,
            max_attempts: 100,
            summary: None,
            audiences: vec![AudienceTarget::Public],
            idempotency_key: None,
            expectations: PostBookkeepingExpectation::default(),
        },
    )
    .await
    .expect("post creation via the service path should succeed")
    .post_id
}
/// Edits a post's body through [`perform_post_update`](crate::perform_post_update) —
/// the service-layer twin of [`create_post_via_service`], so an edit's re-render and
/// re-extraction run exactly as the product runs them. Publication state is left
/// as-is.
///
/// # Panics
///
/// If the update fails.
pub async fn update_post_body_via_service(
    state: &Arc<AppState>,
    post_id: PostId,
    editor_user_id: UserId,
    body: PostBody,
) {
    crate::perform_post_update(
        state.posts.as_ref(),
        crate::PostUpdate {
            post_id,
            editor_user_id,
            body,
            title: None,
            format: PostFormat::Markdown,
            slug_override: None,
            publish: crate::PublishUpdate::Publish { at: None },
            summary: None,
            request_clock: UtcInstant::now(),
            expectations: PostBookkeepingExpectation::default(),
            audiences: vec![AudienceTarget::Public],
        },
    )
    .await
    .expect("post update via the service path should succeed");
}

#[cfg(test)]
mod tests {
    use super::{
        AudienceTarget, Backend, CreatePostError, PostFormat, PostSummary, PostgresDbGuard,
        PostgresTestConfig, SeedPost, SeedRawPost, SeedUser, UtcInstant, backends, bootstrap_url,
        parse_post_title, report_drop_outcome, splice_db_name,
    };

    // The free renderer, to pin that the builder's HTML is exactly `render(body)` — the
    // half of `RenderOutput` the seeded record carries.
    use common::render::render;
    use common::test_support::{parse_post_body, parse_row_limit};
    use common::visibility::ViewerIdentity;
    use rstest::*;
    use rstest_reuse::*;

    #[apply(backends)]
    #[tokio::test]
    async fn seed_user_builder_defaults_create_a_plain_non_operator_user(#[case] backend: Backend) {
        let env = backend.setup().await;
        let state = &env.state;
        let user = SeedUser::new().seed(state).await;
        let u = state
            .users
            .get_user(user.user_id)
            .await
            .unwrap()
            .expect("user exists");
        assert_eq!(u.username, user.username);
        assert!(!u.is_operator);
        assert!(u.display_name.is_none());
        // The default password authenticates — proves `seed` used `password123`.
        state
            .users
            .authenticate(
                &user.username,
                &host::test_support::parse_password("password123"),
            )
            .await
            .expect("default password authenticates");
    }

    #[apply(backends)]
    #[tokio::test]
    async fn seed_user_builder_overrides_apply_password_display_name_and_operator(
        #[case] backend: Backend,
    ) {
        let env = backend.setup().await;
        let state = &env.state;
        let user = SeedUser::new()
            .password("hunter2xyz")
            .display_name("Bob B")
            .operator()
            .seed(state)
            .await;
        let u = state
            .users
            .get_user(user.user_id)
            .await
            .unwrap()
            .expect("user exists");
        assert!(u.is_operator);
        assert_eq!(u.display_name.expect("display name set"), "Bob B");
        // The overridden password authenticates — not the default.
        state
            .users
            .authenticate(
                &user.username,
                &host::test_support::parse_password("hunter2xyz"),
            )
            .await
            .expect("overridden password authenticates");
    }

    #[apply(backends)]
    #[tokio::test]
    async fn seed_user_autogenerates_distinct_usernames(#[case] backend: Backend) {
        let env = backend.setup().await;
        // Two seeds against one DB get distinct autogenerated names, each retrievable.
        // (`default()` and `new()` are equivalent — exercise both.)
        let a = SeedUser::new().seed(&env.state).await;
        let b = SeedUser::default().seed(&env.state).await;
        assert_ne!(a.username, b.username, "each seed gets a fresh name");
        for user in [&a, &b] {
            let rec = env
                .state
                .users
                .get_user(user.user_id)
                .await
                .unwrap()
                .expect("exists");
            assert_eq!(rec.username, user.username);
        }
    }

    #[apply(backends)]
    #[tokio::test]
    async fn seed_post_builder_defaults_create_published_public_markdown(#[case] backend: Backend) {
        let env = backend.setup().await;
        let state = &env.state;
        let user = SeedUser::new().seed(state).await;
        let post = SeedPost::new(user.user_id).seed(state).await;
        assert!(
            post.published_at.is_some(),
            "default post should be published"
        );
        assert!(!post.slug.as_ref().is_empty(), "post should have a slug");
        assert!(!post.body.as_ref().is_empty(), "post should have a body");
        assert_eq!(post.format, PostFormat::Markdown);
        let audiences = state.posts.get_post_audiences(post.post_id).await.unwrap();
        assert_eq!(audiences, vec![AudienceTarget::Public]);
    }

    #[apply(backends)]
    #[tokio::test]
    async fn seed_post_builder_setters_apply(#[case] backend: Backend) {
        let env = backend.setup().await;
        let state = &env.state;
        let user = SeedUser::new().seed(state).await;
        // Exercise the three settable fields and assert each lands on the record.
        let post = SeedPost::new(user.user_id)
            .title(parse_post_title("Custom Title"))
            .body(parse_post_body("Custom body text"))
            .audiences(vec![AudienceTarget::Public])
            .seed(state)
            .await;
        assert_eq!(post.title.as_ref().map(AsRef::as_ref), Some("Custom Title"));
        assert!(post.body.as_ref().contains("Custom body text"));
        let audiences = state.posts.get_post_audiences(post.post_id).await.unwrap();
        assert_eq!(audiences, vec![AudienceTarget::Public]);
    }

    #[apply(backends)]
    #[tokio::test]
    async fn seed_post_bare_repeated_seeds_get_distinct_slugs(#[case] backend: Backend) {
        let env = backend.setup().await;
        let state = &env.state;
        let user = SeedUser::new().seed(state).await;
        // Two title-less bare seeds derive the same slug seed from the fixed body;
        // `perform_post_creation`'s collision-suffix retry keeps them distinct.
        let a = SeedPost::new(user.user_id).seed(state).await;
        let b = SeedPost::new(user.user_id).seed(state).await;
        assert_ne!(a.slug, b.slug, "bare seeds should get distinct slugs");
    }

    #[test]
    fn postgres_test_config_preserves_explicit_bootstrap_url() {
        let config = PostgresTestConfig::from_raw(
            "postgres://jaunder@db:5432/jaunder".to_owned(),
            Some("postgres://admin@bootstrap:5432/postgres".to_owned()),
        );

        assert_eq!(config.test_url(), "postgres://jaunder@db:5432/jaunder");
        assert_eq!(
            config.bootstrap_url(),
            "postgres://admin@bootstrap:5432/postgres"
        );
        assert_eq!(config.bootstrap_authority(), "bootstrap:5432");
    }

    #[test]
    fn postgres_test_config_derives_bootstrap_url_from_test_authority() {
        let config =
            PostgresTestConfig::from_raw("postgres://jaunder@db:5432/jaunder".to_owned(), None);

        assert_eq!(
            config.bootstrap_url(),
            "postgres://postgres@db:5432/postgres"
        );
    }

    #[test]
    fn postgres_db_guard_owns_the_resolved_teardown_url() {
        let config = PostgresTestConfig::from_raw(
            "postgres://jaunder@db:5432/jaunder".to_owned(),
            Some("postgres://admin@bootstrap:5432/postgres".to_owned()),
        );
        let guard = std::mem::ManuallyDrop::new(PostgresDbGuard {
            db_name: "test_db".to_owned(),
            bootstrap_url: config.bootstrap_url().to_owned(),
        });

        assert_eq!(
            guard.bootstrap_url,
            "postgres://admin@bootstrap:5432/postgres"
        );
    }

    // guard:no-backend — harness type-guard on the SQLite CloseablePool variant; no database ops
    #[tokio::test]
    #[should_panic(expected = "postgres() on a SQLite CloseablePool")]
    async fn postgres_accessor_rejects_a_sqlite_pool() {
        let pool = sqlx::SqlitePool::connect("sqlite::memory:").await.unwrap();
        let _ = super::CloseablePool::Sqlite(pool).postgres();
    }

    // guard:no-backend — drives the pure `report_drop_outcome` logging arms; no database ops
    #[tokio::test]
    async fn report_drop_outcome_logs_failure_and_timeout() {
        // Failure arm: a DROP DATABASE that returned a database error.
        report_drop_outcome("test_db", Ok(Err(sqlx::Error::RowNotFound)));

        // Timeout arm: a genuine `Elapsed` from a zero-duration timeout over a
        // future that never completes.
        let elapsed = tokio::time::timeout(std::time::Duration::ZERO, std::future::pending::<()>())
            .await
            .unwrap_err();
        report_drop_outcome("test_db", Err(elapsed));
    }

    #[test]
    fn bootstrap_url_prefers_explicit_when_set() {
        assert_eq!(
            bootstrap_url(
                Some("postgres://admin@db:5432/postgres".to_owned()),
                "postgres://jaunder@127.0.0.1:55432/jaunder",
            ),
            "postgres://admin@db:5432/postgres"
        );
    }

    #[test]
    fn bootstrap_url_derives_superuser_url_on_test_authority_when_unset() {
        assert_eq!(
            bootstrap_url(None, "postgres://jaunder@127.0.0.1:55432/jaunder"),
            "postgres://postgres@127.0.0.1:55432/postgres"
        );
    }

    #[test]
    fn splice_db_name_replaces_the_database_segment() {
        assert_eq!(
            splice_db_name("postgres://jaunder@127.0.0.1:55432/jaunder", "clone_1"),
            "postgres://jaunder@127.0.0.1:55432/clone_1"
        );
    }

    #[test]
    fn splice_db_name_preserves_the_query_string() {
        assert_eq!(
            splice_db_name(
                "postgres://jaunder@127.0.0.1:55432/jaunder?sslmode=require",
                "clone_1",
            ),
            "postgres://jaunder@127.0.0.1:55432/clone_1?sslmode=require"
        );
    }

    #[apply(backends)]
    #[tokio::test]
    async fn seed_raw_post_defaults_create_a_published_public_markdown_post(
        #[case] backend: Backend,
    ) {
        let env = backend.setup().await;
        let state = &env.state;
        let author = SeedUser::new().seed(state).await.user_id;

        let post = SeedRawPost::new(author).seed(state).await;

        let record = state
            .posts
            .get_post_by_id(post.post_id, &ViewerIdentity::Anonymous)
            .await
            .unwrap()
            .expect("post exists");
        assert_eq!(record.slug, post.slug);
        assert_eq!(record.title, Some(post.title));
        assert_eq!(record.format, PostFormat::Markdown);
        assert!(record.published_at.is_some(), "default is published");
        assert_eq!(record.rendered_html, post.rendered_html);
        // The default rendered HTML is the production render of the body.
        assert_eq!(
            record.rendered_html,
            render(&record.body, &record.format),
            "default rendered_html equals render(body)"
        );
    }

    #[apply(backends)]
    #[tokio::test]
    async fn seed_raw_post_autogenerates_distinct_slugs_and_titles(#[case] backend: Backend) {
        let env = backend.setup().await;
        let state = &env.state;
        let author = SeedUser::new().seed(state).await.user_id;
        let a = SeedRawPost::new(author).seed(state).await;
        let b = SeedRawPost::new(author).seed(state).await;
        assert_ne!(a.slug, b.slug, "each seed gets a fresh slug");
        assert_ne!(a.title, b.title, "each seed gets a fresh title");
    }

    #[apply(backends)]
    #[tokio::test]
    async fn seed_raw_post_overrides_apply(#[case] backend: Backend) {
        let env = backend.setup().await;
        let state = &env.state;
        let author = SeedUser::new().seed(state).await.user_id;
        let post = SeedRawPost::new(author)
            .draft()
            .format(PostFormat::Org)
            .summary(PostSummary::from_title(&parse_post_title("excerpt")))
            .tags(["rust"])
            .seed(state)
            .await;
        let record = state
            .posts
            .get_post_by_id(post.post_id, &ViewerIdentity::Anonymous)
            .await
            .unwrap()
            .expect("post exists");
        assert!(record.published_at.is_none(), "draft override applies");
        assert_eq!(record.format, PostFormat::Org);
        assert!(record.summary.is_some(), "summary override applies");
        assert_eq!(record.tags.len(), 1, "tag applied after insert");

        // A non-default audience is stored — `get_post_audiences` reads it back raw
        // (no viewer filter), so a Subscribers-only post is checkable directly.
        let targeted = SeedRawPost::new(author)
            .audiences(vec![AudienceTarget::Subscribers])
            .seed(state)
            .await;
        let audiences = state
            .posts
            .get_post_audiences(targeted.post_id)
            .await
            .unwrap();
        assert_eq!(audiences, vec![AudienceTarget::Subscribers]);
    }

    #[apply(backends)]
    #[tokio::test]
    async fn seed_raw_post_create_surfaces_slug_conflict(#[case] backend: Backend) {
        let env = backend.setup().await;
        let state = &env.state;
        let author = SeedUser::new().seed(state).await.user_id;
        let first = SeedRawPost::new(author).seed(state).await;
        // Reuse the first post's slug and instant so the same-day unique index trips.
        let err = SeedRawPost::new(author)
            .slug(first.slug.as_ref())
            .published_at(first.published_at.expect("default is published"))
            .create(state)
            .await
            .unwrap_err();
        assert!(matches!(err, CreatePostError::SlugConflict));
    }

    #[apply(backends)]
    #[tokio::test]
    async fn seed_raw_post_body_override_is_persisted_and_rendered(#[case] backend: Backend) {
        let env = backend.setup().await;
        let state = &env.state;
        let author = SeedUser::new().seed(state).await.user_id;
        let post = SeedRawPost::new(author)
            .body(parse_post_body("custom body"))
            .seed(state)
            .await;
        let record = state
            .posts
            .get_post_by_id(post.post_id, &ViewerIdentity::Anonymous)
            .await
            .unwrap()
            .expect("post exists");
        assert!(
            record.body.contains("custom body"),
            "body override persisted"
        );
        assert!(
            post.rendered_html.as_ref().contains("custom body"),
            "rendered HTML derives from the overridden body"
        );
    }

    #[apply(backends)]
    #[tokio::test]
    async fn seed_raw_post_build_yields_a_distinct_input_without_writing(#[case] backend: Backend) {
        let env = backend.setup().await;
        let state = &env.state;
        let author = SeedUser::new().seed(state).await;
        let a = SeedRawPost::new(author.user_id).build();
        let b = SeedRawPost::new(author.user_id).build();
        assert!(a.title.is_some(), "build autogenerates a title");
        assert_ne!(a.slug, b.slug, "each build autogenerates a distinct slug");
        // build() takes no state, so nothing was written — the author has no posts.
        let published = state
            .posts
            .list_published_by_user(
                &author.username,
                None,
                parse_row_limit("50"),
                &ViewerIdentity::Anonymous,
                UtcInstant::now(),
            )
            .await
            .unwrap();
        assert!(published.is_empty(), "build() does not persist");
    }
}
