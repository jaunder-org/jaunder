//! Both-backend test harness for the `storage` crate's own tests and `server`'s
//! integration tests: the `Backend` enum, per-test database provisioning
//! (`SQLite` tempdir; Postgres clone-from-template via `JAUNDER_PG_TEST_URL`), the
//! `AppState`-level `TestEnv`, and the `backends`/`sqlite_only`/`postgres_only`
//! rstest templates. Lives in `storage` (gated by the `test-support` feature) so
//! `storage`'s in-file tests use it from the same crate instance — avoiding the
//! two-`storage`-instances problem a separate crate would create (see ADR-0033).
//! `server` reaches it via `storage`'s `test-support` feature.

// Deliberately unwrap/expect-heavy test scaffolding, so the workspace's
// `unwrap_used`/`expect_used = deny` lints are allowed off for this module
// (an inner `#![allow]` overrides the crate-level deny); everything else
// clippy-pedantic flags is fixed in place rather than allowed.
#![allow(clippy::unwrap_used, clippy::expect_used)]

use crate::sql::quote_identifier;
use crate::{AppState, DbConnectOptions};
use common::mailer::{MailSender, NoopMailSender};
use sqlx::{Connection, PgPool, SqlitePool};
use std::sync::{
    atomic::{AtomicU64, Ordering},
    Arc,
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

impl CloseablePool {
    /// Closes the pool. Afterwards the next query through any storage handle
    /// backed by it returns `sqlx::Error::PoolClosed`, which the storage layer
    /// maps to its `Internal` error variant — the backend-agnostic
    /// storage-error-propagation fault. `sqlx::Pool::close` is generic over the
    /// backend, so the behavior is identical on `SQLite` and Postgres.
    pub async fn close(&self) {
        match self {
            CloseablePool::Sqlite(pool) => pool.close().await,
            CloseablePool::Postgres(pool) => pool.close().await,
        }
    }

    /// Runs a raw statement against whichever backend this env uses — the seed
    /// counterpart to [`close`](CloseablePool::close), dispatched internally so
    /// callers stay backend-agnostic. (The SQL string may still be dialect-specific.)
    ///
    /// # Errors
    ///
    /// Returns the `sqlx::Error` if the statement fails to execute.
    pub async fn execute(&self, sql: &str) -> Result<(), sqlx::Error> {
        match self {
            CloseablePool::Sqlite(pool) => {
                sqlx::query(sql).execute(pool).await?;
            }
            CloseablePool::Postgres(pool) => {
                sqlx::query(sql).execute(pool).await?;
            }
        }
        Ok(())
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
}

/// A ready-to-use [`AppState`] plus the temp dir backing it. `base` doubles as
/// the media-storage root HTTP tests need on both backends, and on `SQLite` it
/// also holds the database file alive for the lifetime of the test.
pub struct TestEnv {
    pub state: Arc<AppState>,
    pub base: TestBase,
}

/// Owns a test's temp dir and, on Postgres, the name of the per-test database
/// cloned from the template. Dropping it removes that clone so the ephemeral
/// cluster's data dir does not grow with the suite — the disk-exhaustion fix for
/// issue #28. `Deref`s to the inner `TempDir`, so existing `base.path()` and
/// `&base` uses keep compiling unchanged.
pub struct TestBase {
    dir: TempDir,
    /// `Some(name)` on Postgres; `None` on `SQLite`.
    postgres_db: Option<String>,
    /// A clone of the pool behind [`TestEnv::state`], so tests can fault it
    /// ([`close_pool`](TestBase::close_pool)) or run raw SQL through it
    /// ([`pool`](TestBase::pool)). Held here (a private field) rather than on
    /// `TestEnv` so the many `let TestEnv { state, base } = …` destructures keep
    /// compiling. A live clone at [`Drop`] time is safe because
    /// [`drop_test_database`] issues `DROP DATABASE … WITH (FORCE)`.
    pool: CloseablePool,
}

impl TestBase {
    fn sqlite(dir: TempDir, pool: SqlitePool) -> Self {
        Self {
            dir,
            postgres_db: None,
            pool: CloseablePool::Sqlite(pool),
        }
    }

    fn postgres(dir: TempDir, db_name: String, pool: PgPool) -> Self {
        Self {
            dir,
            postgres_db: Some(db_name),
            pool: CloseablePool::Postgres(pool),
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
}

impl std::ops::Deref for TestBase {
    type Target = TempDir;

    fn deref(&self) -> &Self::Target {
        &self.dir
    }
}

impl Drop for TestBase {
    fn drop(&mut self) {
        if let Some(db_name) = self.postgres_db.take() {
            drop_test_database(&db_name);
        }
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
        let (state, base) = match self {
            Backend::Sqlite => {
                let DbConnectOptions::Sqlite(options) = sqlite_url(&dir) else {
                    unreachable!() // cov:ignore — sqlite_url always yields SQLite
                };
                let (state, pool) = crate::sqlite::open_sqlite_database_with_pool(&options, true)
                    .await
                    .unwrap();
                (state, TestBase::sqlite(dir, pool))
            }
            Backend::Postgres => {
                let url = template_postgres_url().await;
                // template_postgres_url() always yields Postgres, so unreachable.
                let DbConnectOptions::Postgres { options, .. } = &url else {
                    unreachable!() // cov:ignore
                };
                let (state, pool) = crate::postgres::open_postgres_database_with_pool(options)
                    .await
                    .unwrap();
                let db_name = options
                    .get_database()
                    .expect("per-test database URL includes a name")
                    .to_owned();
                // Record the per-test DB URL so raw-SQL helpers reuse this exact
                // database rather than minting a fresh (empty) template clone.
                std::fs::write(dir.path().join(PG_URL_FILE), url.to_string())
                    .expect("write recorded Postgres URL");
                (state, TestBase::postgres(dir, db_name, pool))
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

pub(crate) fn postgres_url() -> DbConnectOptions {
    postgres_url_string().parse().unwrap()
}

/// Whether Postgres-backed tests are enabled (i.e. `JAUNDER_PG_TEST_URL` is set).
#[must_use]
pub fn postgres_testing_enabled() -> bool {
    std::env::var("JAUNDER_PG_TEST_URL").is_ok()
}

/// The superuser bootstrap URL used to create/drop per-test databases —
/// `JAUNDER_PG_BOOTSTRAP_TEST_URL` if set, else a `postgres` URL derived from the
/// test URL's authority.
#[must_use]
pub fn postgres_bootstrap_url() -> String {
    bootstrap_url(
        std::env::var("JAUNDER_PG_BOOTSTRAP_TEST_URL").ok(),
        &postgres_url_string(),
    )
}

/// Pure core of [`postgres_bootstrap_url`]: the `explicit` bootstrap URL when set,
/// else a `postgres` superuser URL on the same authority as `test_url`. Split out
/// from the env read so both arms are unit-testable (the env read itself is
/// covered whenever the suite provisions Postgres).
fn bootstrap_url(explicit: Option<String>, test_url: &str) -> String {
    explicit.unwrap_or_else(|| {
        let authority = postgres_url_authority(test_url);
        format!("postgres://postgres@{authority}/postgres")
    })
}

pub(crate) fn postgres_url_string() -> String {
    std::env::var("JAUNDER_PG_TEST_URL")
        .unwrap_or_else(|_| "postgres://jaunder@127.0.0.1:55432/jaunder".to_owned())
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

/// The `host:port` authority of the bootstrap connection (for raw cluster ops).
#[must_use]
pub fn postgres_test_authority() -> String {
    postgres_url_authority(&postgres_bootstrap_url())
}

fn postgres_url_with_db_name(db_name: &str) -> String {
    splice_db_name(&postgres_url_string(), db_name)
}

/// Pure core of [`postgres_url_with_db_name`]: replace the database segment of
/// `template` with `db_name`, preserving any `?query`. Split out from the env read
/// so the with-query and without-query arms are unit-testable.
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
fn drop_test_database(db_name: &str) {
    let bootstrap = postgres_bootstrap_url();
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
                let Ok(options) = bootstrap.parse::<sqlx::postgres::PgConnectOptions>() else {
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
        Ok(Err(error)) => eprintln!("issue #28: drop {db_name} failed: {error}"), // cov:ignore
        Err(_elapsed) => eprintln!("issue #28: drop {db_name} timed out"),        // cov:ignore
    }
}

/// A connect URL naming a per-test database that has **not** been created — for
/// tests that exercise the "database is absent" path.
///
/// # Panics
///
/// If the constructed URL fails to parse.
#[must_use]
pub fn nonexistent_postgres_url() -> DbConnectOptions {
    postgres_url_with_db_name(&unique_postgres_db_name())
        .parse()
        .unwrap()
}

/// Creates a fresh, empty per-test Postgres database and returns its connect URL.
///
/// # Panics
///
/// If the test URL lacks a username, or the admin connection / `CREATE DATABASE`
/// fails.
pub async fn unique_postgres_url() -> DbConnectOptions {
    let db_name = unique_postgres_db_name();

    let bootstrap: sqlx::postgres::PgConnectOptions = postgres_bootstrap_url().parse().unwrap();
    let DbConnectOptions::Postgres { options, .. } = postgres_url() else {
        panic!("expected postgres options"); // cov:ignore — postgres_url() always parses to Postgres
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

    postgres_url_with_db_name(&db_name).parse().unwrap()
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
async fn ensure_template_db() {
    let bootstrap: sqlx::postgres::PgConnectOptions = postgres_bootstrap_url().parse().unwrap();
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
        let DbConnectOptions::Postgres { options, .. } = postgres_url() else {
            panic!("expected postgres options"); // cov:ignore — postgres_url() always parses to Postgres
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
        let pool = sqlx::PgPool::connect(&postgres_url_with_db_name(TEMPLATE_DB))
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
pub async fn template_postgres_url() -> DbConnectOptions {
    ensure_template_db().await;

    let DbConnectOptions::Postgres { options, .. } = postgres_url() else {
        panic!("expected postgres options"); // cov:ignore — postgres_url() always parses to Postgres
    };
    let owner = options.get_username();
    let db_name = unique_postgres_db_name();

    let bootstrap: sqlx::postgres::PgConnectOptions = postgres_bootstrap_url().parse().unwrap();
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

    postgres_url_with_db_name(&db_name).parse().unwrap()
}

/// Default mailer for tests that don't care about email sending.
#[must_use]
pub fn noop_mailer() -> Arc<dyn MailSender> {
    Arc::new(NoopMailSender)
}

/// Seeds `count` posts for `user_id` directly through the storage service,
/// bypassing the HTTP/server-fn path (markdown render of trivial bodies is
/// negligible; the cost we avoid is axum routing + `server_fn` per call).
/// `published == true` sets `published_at = now` so list/timeline endpoints
/// return them; `false` leaves them as drafts. Returns ids in creation order.
///
/// # Panics
///
/// If a slug fails to parse or a post fails to persist.
pub async fn seed_posts(
    state: &Arc<AppState>,
    user_id: i64,
    count: usize,
    published: bool,
) -> Vec<i64> {
    let mut ids = Vec::with_capacity(count);
    for i in 0..count {
        let id = crate::seed_rendered_post(
            &*state.posts,
            user_id,
            format!("seed-{i}").parse().expect("valid slug"),
            format!("# Post {i}\n\nbody"),
            published,
        )
        .await
        .expect("seed post should be created");
        ids.push(id);
    }
    ids
}

/// Creates a throwaway user and returns its id, for tests that need a user to
/// exist before exercising a per-user handle (replaces raw `INSERT INTO users`).
///
/// # Panics
///
/// If the username/password fail to parse or the user cannot be created.
pub async fn seed_user(state: &Arc<AppState>) -> i64 {
    state
        .users
        .create_user(
            &"testuser".parse().expect("valid username"),
            &"password123".parse().expect("valid password"),
            None,
            false,
        )
        .await
        .expect("seed user should be created")
}

#[cfg(test)]
mod tests {
    use super::{backends, bootstrap_url, seed_user, splice_db_name, Backend};
    use rstest::*;
    use rstest_reuse::*;

    #[apply(backends)]
    #[tokio::test]
    async fn seed_user_creates_a_user(#[case] backend: Backend) {
        let env = backend.setup().await;
        let id = seed_user(&env.state).await;
        assert!(id > 0);
    }

    #[tokio::test]
    #[should_panic(expected = "postgres() on a SQLite CloseablePool")]
    async fn postgres_accessor_rejects_a_sqlite_pool() {
        let pool = sqlx::SqlitePool::connect("sqlite::memory:").await.unwrap();
        let _ = super::CloseablePool::Sqlite(pool).postgres();
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
}
