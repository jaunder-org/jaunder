//! Both-backend test environment provisioning, rstest templates, and test-held
//! pool/transaction primitives. Postgres clone URL lifecycle lives in [`super::postgres`];
//! this leaf selects and wires that lifecycle into a uniform harness surface.
use super::postgres::{PG_URL_FILE, PostgresDbGuard, PostgresTestConfig, template_postgres_url};
use crate::posts::{INSERT_POST_TAG, UPSERT_TAG_RETURNING_ID};
use crate::sql::QueryStorageExt;
use crate::{
    AppState, DbConnectOptions, PostStorage, StorageRuntimeConfig, TaggingError, WriteScope,
    WriteScopeError,
};

use common::MutationOutcome;
use common::ids::{PostId, TagId, UserId};
use common::media::MediaRef;
use common::tag::TagLabel;
use sqlx::pool::PoolConnection;
use sqlx::{PgPool, Postgres, Sqlite, SqlitePool, Transaction};
use std::sync::Arc;
use tempfile::TempDir;

// This crate only defines the templates, so it needs just the `template`
// attribute. `#[export]` is consumed by `#[template]` (no import needed), and the
// `rstest`/`case` attributes the expansion emits are resolved at the *apply* site
// in consumer crates, not here.
use rstest_reuse::template;

#[cfg(any(test, feature = "test-utils"))]
/// Creates the mock write scope used by downstream storage-trait unit tests.
///
/// The scope is minted here at the storage test composition root so test callers
/// cannot choose a backend or construct its transaction capability.
#[must_use]
pub fn mock_write_scope() -> WriteScope {
    WriteScope::mock()
}

/// Mints a SQLite-backed write scope for a test fixture that owns its pool.
#[must_use]
pub fn sqlite_write_scope(pool: SqlitePool) -> WriteScope {
    WriteScope::sqlite(pool)
}

/// Persists a site-config fixture through a confirmed caller-owned write scope.
///
/// # Errors
///
/// Returns an error when the storage operation fails.
pub async fn set_site_config(
    env: &TestEnv,
    key: host::config_key::SiteConfigKey,
    value: &str,
) -> anyhow::Result<()> {
    let site_config = Arc::clone(&env.state.site_config);
    let value = value.to_owned();
    confirmed(
        env.state
            .write_scope
            .run(move |transaction| {
                Box::pin(async move { site_config.set(transaction, key, &value).await })
            })
            .await?,
    );
    Ok(())
}

/// Extracts a mutation value, requiring that commit acknowledgement is confirmed.
///
/// # Panics
///
/// Panics when commit acknowledgement is indeterminate.
pub fn confirmed<T>(outcome: MutationOutcome<T>) -> T {
    confirmed_for(outcome, "fixture mutation")
}

/// Extracts a mutation value, requiring a confirmed commit for `action`.
///
/// # Panics
///
/// Panics when commit acknowledgement is indeterminate.
pub fn confirmed_for<T>(outcome: MutationOutcome<T>, action: &str) -> T {
    match outcome {
        MutationOutcome::Confirmed(value) => value,
        MutationOutcome::CommitIndeterminate(_) => panic!("{action} requires a confirmed commit"),
    }
}

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
                // IMMEDIATE, mirroring `SqlitePostStorage::set_post_tags`: takes
                // the write lock up front rather than upgrading a shared lock,
                // which `busy_timeout` cannot rescue (ADR-0021). SQLx tracks
                // this custom begin, so drop schedules rollback before pool reuse.
                HeldWrite::Sqlite(pool.begin_with("BEGIN IMMEDIATE").await?)
            }
            CloseablePool::Postgres(pool) => {
                let mut tx = pool.begin().await?;
                // Mirrors `PostgresPostStorage::set_post_tags`.
                sqlx::query_scalar::<_, PostId>(
                    "SELECT post_id FROM posts WHERE post_id = $1 FOR UPDATE",
                )
                .bind_storage(post_id)
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
                    .bind_storage(key)
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
/// Both arms own a tracked `SQLx` [`Transaction`]. Dropping an unfinished guard
/// starts rollback before its connection can be reused, so neither uncommitted
/// tags nor `SQLite`'s `BEGIN IMMEDIATE` write lock escape the guard.
pub struct PostWriteLock<'a> {
    /// The post the lock was taken for. Held so [`add_tag`](PostWriteLock::add_tag)
    /// cannot be aimed at a post other than the one that is locked.
    post_id: PostId,
    held: HeldWrite<'a>,
}

/// The backend-specific half of a [`PostWriteLock`].
enum HeldWrite<'a> {
    Sqlite(Transaction<'a, Sqlite>),
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
                    .bind_storage(&slug)
                    .fetch_one(&mut **conn)
                    .await?;
                sqlx::query(INSERT_POST_TAG)
                    .bind_storage(post_id)
                    .bind_storage(tag_id)
                    .bind_storage(label)
                    .execute(&mut **conn)
                    .await?;
            }
            HeldWrite::Postgres(tx) => {
                let tag_id = sqlx::query_scalar::<_, TagId>(UPSERT_TAG_RETURNING_ID)
                    .bind_storage(&slug)
                    .fetch_one(&mut **tx)
                    .await?;
                sqlx::query(INSERT_POST_TAG)
                    .bind_storage(post_id)
                    .bind_storage(tag_id)
                    .bind_storage(label)
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
            HeldWrite::Sqlite(tx) => tx.commit().await?,
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

impl TestEnv {
    #[must_use]
    pub fn media_content_locks(&self) -> crate::MediaContentLocks {
        crate::MediaContentLocks::new(Arc::new(self.base.path().to_path_buf()))
    }
}

/// Creates the shared lock seam for fixture Post writers that receive only an
/// [`AppState`], not their enclosing [`TestEnv`].
#[must_use]
pub fn fixture_media_content_locks() -> crate::MediaContentLocks {
    crate::MediaContentLocks::new(Arc::new(
        std::env::temp_dir().join("jaunder-test-media-content-locks"),
    ))
}

/// Reconciles post tags through a scope and requires a confirmed setup write.
///
/// Fixture setup needs a durable row before its assertions run, so an
/// unacknowledged commit is not usable as a successful setup result.
///
/// # Errors
///
/// Returns the scope's begin or tagging error when the setup write fails before
/// commit.
///
/// # Panics
///
/// Panics when commit acknowledgement is indeterminate because fixture setup
/// requires a confirmed durable write.
pub async fn set_post_tags_confirmed(
    write_scope: &WriteScope,
    posts: Arc<dyn PostStorage>,
    post_id: PostId,
    user_id: UserId,
    desired: &[TagLabel],
) -> Result<(), WriteScopeError<TaggingError>> {
    let desired = desired.to_vec();
    let outcome = write_scope
        .run(move |transaction| {
            Box::pin(async move {
                posts
                    .set_post_tags(transaction, post_id, user_id, &desired)
                    .await
            })
        })
        .await?;
    confirmed_for(outcome, "tag fixture setup");
    Ok(())
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

#[cfg(test)]
mod tests {
    use super::{CloseablePool, confirmed_for};

    // guard:no-backend — harness type-guard on the SQLite CloseablePool variant; no database ops
    #[tokio::test]
    #[should_panic(expected = "postgres() on a SQLite CloseablePool")]
    async fn postgres_accessor_rejects_a_sqlite_pool() {
        let pool = sqlx::SqlitePool::connect("sqlite::memory:").await.unwrap();
        let _ = CloseablePool::Sqlite(pool).postgres();
    }

    #[test]
    #[should_panic(expected = "fixture action requires a confirmed commit")]
    fn confirmed_for_rejects_indeterminate_commit() {
        confirmed_for(
            common::MutationOutcome::CommitIndeterminate(()),
            "fixture action",
        );
    }
}
