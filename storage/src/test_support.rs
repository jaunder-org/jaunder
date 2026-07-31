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
#![expect(clippy::unwrap_used, clippy::expect_used)]

use crate::media::MediaRecord;
use crate::posts::{CreatePostError, CreatePostInput};
use crate::sql::quote_identifier;
use crate::{AppState, DbConnectOptions, PostFormat, PostRecord, SiteConfigStorage};
use async_trait::async_trait;
use chrono::{DateTime, Utc};
use common::feed::FeedPath;
use common::ids::{PostId, UserId};
use common::mailer::{MailSender, NoopMailSender};
use common::media::{detect_content_type, media_url, Filename, MediaRef, MediaSource};
use common::post_body::PostBody;
use common::post_summary::PostSummary;
use common::post_title::PostTitle;
use common::render::{RenderOutput, RenderedHtml};
use common::slug::Slug;
use common::tag::TagLabel;
use common::test_support::{
    parse_byte_size, parse_content_hash, parse_display_name, parse_password, parse_post_title,
    parse_slug, parse_tag_label, parse_username,
};
use common::username::Username;
use common::visibility::AudienceTarget;
use host::invite::InviteCode;
use sqlx::{Connection, PgPool, SqlitePool};
use std::collections::BTreeMap;
use std::sync::{
    atomic::{AtomicU64, Ordering},
    Arc, Mutex,
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

    /// Fetches a single `i64` scalar (e.g. a `COUNT(*)`) — the inspect
    /// counterpart to [`execute`](CloseablePool::execute), dispatched per backend
    /// so callers stay backend-agnostic.
    ///
    /// # Errors
    ///
    /// Returns the `sqlx::Error` if the query fails.
    pub async fn scalar_i64(&self, sql: &str) -> Result<i64, sqlx::Error> {
        match self {
            CloseablePool::Sqlite(pool) => sqlx::query_scalar(sql).fetch_one(pool).await,
            CloseablePool::Postgres(pool) => sqlx::query_scalar(sql).fetch_one(pool).await,
        }
    }

    /// Fetches every row of a three-`TEXT`-column query — the multi-row sibling of
    /// [`scalar_i64`](CloseablePool::scalar_i64), for inspecting a child table whose
    /// identity is a string triple (`post_media`).
    ///
    /// # Errors
    ///
    /// Returns the `sqlx::Error` if the query fails.
    pub async fn string_triples(
        &self,
        sql: &str,
    ) -> Result<Vec<(String, String, String)>, sqlx::Error> {
        match self {
            CloseablePool::Sqlite(pool) => sqlx::query_as(sql).fetch_all(pool).await,
            CloseablePool::Postgres(pool) => sqlx::query_as(sql).fetch_all(pool).await,
        }
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

/// Owns a test's temp dir and, on Postgres, a [`PostgresDbGuard`] that drops the
/// per-test database on teardown so the ephemeral cluster's data dir does not
/// grow with the suite (the disk-exhaustion fix for issue #28). `Deref`s to the
/// inner `TempDir`, so existing `base.path()` and `&base` uses keep compiling
/// unchanged.
pub struct TestBase {
    dir: TempDir,
    /// A clone of the pool behind [`TestEnv::state`], so tests can fault it
    /// ([`close_pool`](TestBase::close_pool)) or run raw SQL through it
    /// ([`pool`](TestBase::pool)). Held here (a private field) rather than on
    /// `TestEnv` so the many `let TestEnv { state, base } = …` destructures keep
    /// compiling. A live clone when the guard below drops is safe because
    /// [`drop_test_database`] issues `DROP DATABASE … WITH (FORCE)`.
    pool: CloseablePool,
    /// `Some` on Postgres (drops the per-test database on teardown); `None` on
    /// `SQLite`. Declared after `pool` so the pool drops first (fields drop in
    /// declaration order); with `WITH (FORCE)` the order is not critical.
    _pg: Option<PostgresDbGuard>,
}

impl TestBase {
    fn sqlite(dir: TempDir, pool: SqlitePool) -> Self {
        Self {
            dir,
            pool: CloseablePool::Sqlite(pool),
            _pg: None,
        }
    }

    fn postgres(dir: TempDir, pg: PostgresDbGuard, pool: PgPool) -> Self {
        Self {
            dir,
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
        let (state, base) = match self {
            Backend::Sqlite => {
                let DbConnectOptions::Sqlite(options) = sqlite_url(&dir) else {
                    unreachable!("sqlite_url always yields Sqlite")
                };
                let (state, pool) = crate::sqlite::open_sqlite_database_with_pool(&options, true)
                    .await
                    .unwrap();
                (state, TestBase::sqlite(dir, pool))
            }
            Backend::Postgres => {
                let (url, guard) = template_postgres_url().await;
                // template_postgres_url() always yields Postgres, so unreachable.
                let DbConnectOptions::Postgres { options, .. } = &url else {
                    unreachable!("template_postgres_url always yields Postgres")
                };
                let (state, pool) = crate::postgres::open_postgres_database_with_pool(options)
                    .await
                    .unwrap();
                // Record the per-test DB URL so raw-SQL helpers reuse this exact
                // database rather than minting a fresh (empty) template clone.
                std::fs::write(dir.path().join(PG_URL_FILE), url.to_string())
                    .expect("write recorded Postgres URL");
                (state, TestBase::postgres(dir, guard, pool))
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
}

impl Drop for PostgresDbGuard {
    fn drop(&mut self) {
        drop_test_database(&self.db_name);
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
pub async fn unique_postgres_url() -> (DbConnectOptions, PostgresDbGuard) {
    let db_name = unique_postgres_db_name();

    let bootstrap: sqlx::postgres::PgConnectOptions = postgres_bootstrap_url().parse().unwrap();
    let DbConnectOptions::Postgres { options, .. } = postgres_url() else {
        unreachable!("postgres_url always yields Postgres")
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

    let options = postgres_url_with_db_name(&db_name).parse().unwrap();
    (options, PostgresDbGuard { db_name })
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
            unreachable!("postgres_url always yields Postgres")
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
pub async fn template_postgres_url() -> (DbConnectOptions, PostgresDbGuard) {
    ensure_template_db().await;

    let DbConnectOptions::Postgres { options, .. } = postgres_url() else {
        unreachable!("postgres_url always yields Postgres")
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

    let options = postgres_url_with_db_name(&db_name).parse().unwrap();
    (options, PostgresDbGuard { db_name })
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
                format!("# Post {i}\n\nbody").into(),
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
                &parse_password(self.password),
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
pub struct SeedPost<'a> {
    user_id: UserId,
    title: Option<&'a str>,
    body: PostBody,
    audiences: Vec<AudienceTarget>,
}

impl<'a> SeedPost<'a> {
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
            body: PostBody::from("Seeded post body"),
            audiences: vec![AudienceTarget::Public],
        }
    }

    /// Set an explicit title — for the permalink/listing tests that assert on it.
    #[must_use]
    pub fn title(mut self, title: &'a str) -> Self {
        self.title = Some(title);
        self
    }

    /// Override the default body.
    #[must_use]
    pub fn body(mut self, body: impl Into<PostBody>) -> Self {
        self.body = body.into();
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
                title: self.title,
                format: PostFormat::Markdown,
                slug_override: None,
                published_at: Some(Utc::now()),
                max_attempts: 100,
                summary: None,
                audiences: self.audiences,
                idempotency_key: None,
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
    pub published_at: Option<DateTime<Utc>>,
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
    published_at: Option<DateTime<Utc>>,
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
            body: "seed body".to_owned().into(),
            format: PostFormat::Markdown,
            published_at: Some(Utc::now()),
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
    pub fn body(mut self, body: impl Into<PostBody>) -> Self {
        self.body = body.into();
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
    pub fn published_at(mut self, at: DateTime<Utc>) -> Self {
        self.published_at = Some(at);
        self
    }

    /// Tags applied via `tag_post` right after the insert by `.seed`/`.create`. Ignored by
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
    /// If applying a `.tags()` label via `tag_post` fails (tagging is happy-path setup).
    pub async fn create(mut self, state: &Arc<AppState>) -> Result<SeededPost, CreatePostError> {
        let tags = std::mem::take(&mut self.tags);
        let input = self.into_input();
        let post_id = state.posts.create_post(&input).await?;
        for tag in &tags {
            state
                .posts
                .tag_post(post_id, tag)
                .await
                .expect("seed tag_post should succeed");
        }
        Ok(SeededPost {
            post_id,
            slug: input.slug,
            title: input
                .title
                .expect("SeedRawPost always autogenerates a title"),
            published_at: input.published_at,
            rendered_html: input.rendered.html,
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

/// The content hash every media fixture is stored under: the SHA-256 of the empty
/// input, so the digest is a real one rather than an invented hex string. Public
/// because a test spelling the `AtomPub` member layout
/// (`/atompub/<user>/media/<sha>/<name>`) needs the digest itself, not a serve URL.
pub const MEDIA_TEST_SHA256: &str =
    "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855";

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
            created_at: Utc::now(),
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

/// A post's `post_media` rows as `(source, sha256, filename)`, ascending — the
/// persisted form of what its rendered HTML points a reader at.
///
/// # Panics
///
/// If the query fails.
pub async fn fetch_post_media(base: &TestBase, post_id: PostId) -> Vec<(String, String, String)> {
    base.pool()
        .string_triples(&format!(
            "SELECT source, sha256, filename FROM post_media \
             WHERE post_id = {post_id} ORDER BY source, sha256, filename"
        ))
        .await
        .expect("post_media query should succeed")
}

/// Creates a post through [`perform_post_creation`](crate::perform_post_creation) —
/// the same entry point `web::posts::create` uses — so a test exercises the product's
/// own path (render, extract, write) rather than a synthetic [`CreatePostInput`].
///
/// # Panics
///
/// If the post cannot be created.
pub async fn create_post_via_service(state: &Arc<AppState>, user_id: UserId, body: &str) -> PostId {
    create_via_service(state, user_id, body, Some(Utc::now())).await
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
    body: &str,
) -> PostId {
    create_via_service(state, user_id, body, None).await
}

/// Shared body of the two service-layer creators: everything but `published_at` is
/// fixed (public, Markdown, title derived from the body), as the two differ in exactly
/// that one field.
async fn create_via_service(
    state: &Arc<AppState>,
    user_id: UserId,
    body: &str,
    published_at: Option<DateTime<Utc>>,
) -> PostId {
    crate::perform_post_creation(
        state.posts.as_ref(),
        crate::PostCreation {
            user_id,
            body: PostBody::from(body),
            title: None,
            format: PostFormat::Markdown,
            slug_override: None,
            published_at,
            max_attempts: 100,
            summary: None,
            audiences: vec![AudienceTarget::Public],
            idempotency_key: None,
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
    body: &str,
) {
    crate::perform_post_update(
        state.posts.as_ref(),
        crate::PostUpdate {
            post_id,
            editor_user_id,
            body: PostBody::from(body),
            title: None,
            format: PostFormat::Markdown,
            slug_override: None,
            publish: crate::PublishUpdate::Publish { at: None },
            summary: None,
            audiences: vec![AudienceTarget::Public],
        },
    )
    .await
    .expect("post update via the service path should succeed");
}

/// An in-memory [`SiteConfigStorage`] for tests that need a facade over site
/// config without a database. A real key/value store: `set`/`delete` mutate a
/// `BTreeMap` (so `list` is naturally key-ordered) and `get` reads it. Shared by
/// every module that stubs `SiteConfigStorage` (SMTP loading, mailer building).
#[derive(Default)]
pub struct InMemorySiteConfig(Mutex<BTreeMap<String, String>>);

impl InMemorySiteConfig {
    /// An empty store.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// A store preloaded with `pairs`.
    pub fn from_pairs<K, V>(pairs: impl IntoIterator<Item = (K, V)>) -> Self
    where
        K: Into<String>,
        V: Into<String>,
    {
        Self(Mutex::new(
            pairs
                .into_iter()
                .map(|(k, v)| (k.into(), v.into()))
                .collect(),
        ))
    }
}

#[async_trait]
impl SiteConfigStorage for InMemorySiteConfig {
    async fn get(&self, key: &str) -> sqlx::Result<Option<String>> {
        Ok(self.0.lock().unwrap().get(key).cloned())
    }

    async fn set(&self, key: &str, value: &str) -> sqlx::Result<()> {
        self.0
            .lock()
            .unwrap()
            .insert(key.to_owned(), value.to_owned());
        Ok(())
    }

    async fn list(&self) -> sqlx::Result<Vec<(String, String)>> {
        Ok(self
            .0
            .lock()
            .unwrap()
            .iter()
            .map(|(k, v)| (k.clone(), v.clone()))
            .collect())
    }

    async fn delete(&self, key: &str) -> sqlx::Result<bool> {
        Ok(self.0.lock().unwrap().remove(key).is_some())
    }

    async fn get_smtp_credentials(&self) -> sqlx::Result<crate::smtp::SmtpCredentials> {
        // Mirror the real backend's bridge decode: parse each stored value and
        // surface a reject (empty) as a decode error.
        let username = self
            .get("smtp.username")
            .await?
            .map(|v| v.parse::<common::smtp_username::SmtpUsername>())
            .transpose()
            .map_err(|e| sqlx::Error::Decode(Box::new(e)))?;
        let password = self
            .get("smtp.password")
            .await?
            .map(|v| v.parse::<common::smtp_password::SmtpPassword>())
            .transpose()
            .map_err(|e| sqlx::Error::Decode(Box::new(e)))?;
        Ok(crate::smtp::SmtpCredentials { username, password })
    }
}

#[cfg(test)]
mod tests {
    use super::{
        backends, bootstrap_url, parse_password, report_drop_outcome, splice_db_name,
        AudienceTarget, Backend, CreatePostError, PostFormat, PostSummary, SeedPost, SeedRawPost,
        SeedUser,
    };
    use chrono::Utc;
    // The free renderer, to pin that the builder's HTML is exactly `render(body)` — the
    // half of `RenderOutput` the seeded record carries.
    use common::render::render;
    use common::test_support::parse_row_limit;
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
            .authenticate(&user.username, &parse_password("password123"))
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
            .authenticate(&user.username, &parse_password("hunter2xyz"))
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
            .title("Custom Title")
            .body("Custom body text")
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
            .summary(PostSummary::truncated("excerpt"))
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
            .body("custom body")
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
                Utc::now(),
            )
            .await
            .unwrap();
        assert!(published.is_empty(), "build() does not persist");
    }
}
