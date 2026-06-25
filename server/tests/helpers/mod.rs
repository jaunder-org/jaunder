#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::too_many_lines,
    clippy::similar_names,
    clippy::items_after_statements,
    clippy::unused_async
)]
#![allow(dead_code)]
// A `#[template]` expands to a name-mangled `macro_rules!`, so a per-item
// `#[allow(unused_macros)]` can't reach it — this crate-level allow suppresses
// the resulting dead-template lint in test binaries that import an unused
// template.
#![allow(unused_macros)]

use common::mailer::{MailSender, NoopMailSender};
use leptos::prelude::LeptosOptions;
use sqlx::Connection;
use std::sync::{
    atomic::{AtomicU64, Ordering},
    Arc, OnceLock,
};
use storage::{
    open_database, open_existing_database, AppState, DbConnectOptions, SqliteAtomicOps,
    SqliteAudienceStorage, SqliteEmailVerificationStorage, SqliteFeedCacheStorage,
    SqliteFeedEventStorage, SqliteInviteStorage, SqliteMediaStorage, SqlitePasswordResetStorage,
    SqlitePostStorage, SqliteSessionStorage, SqliteSiteConfigStorage, SqliteSubscriptionStorage,
    SqliteUserConfigStorage, SqliteUserStorage,
};
use tempfile::TempDir;

// `rstest::*` brings the `rstest` attribute the `#[template]` bodies use. It
// reads as unused here because the templates expand to macros rather than
// fixture functions, so the import needs an explicit allow.
#[allow(unused_imports)]
use rstest::*;
// `#[template]`/`#[apply]` come from the `rstest_reuse` companion crate (rstest
// itself only exports `rstest`/`fixture`). The bare `use rstest_reuse;` is
// required because `rstest_reuse::template` expands to code that names the
// `rstest_reuse` crate; `use rstest_reuse::*;` alone is not enough.
#[allow(clippy::single_component_path_imports)]
use rstest_reuse;
use rstest_reuse::*;

mod websub_capturing;
// Re-exported for `feed_worker.rs`; `helpers` is included into every test binary
// and most don't use it, so the re-export reads as unused in those.
#[allow(unused_imports)]
pub use websub_capturing::CapturingWebSubClient;

/// The storage backend a test runs against. Backend-parametrized tests take a
/// `#[case] backend: Backend` and call [`Backend::setup`].
#[derive(Copy, Clone)]
pub enum Backend {
    Sqlite,
    Postgres,
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
}

impl TestBase {
    fn sqlite(dir: TempDir) -> Self {
        Self {
            dir,
            postgres_db: None,
        }
    }

    fn postgres(dir: TempDir, db_name: String) -> Self {
        Self {
            dir,
            postgres_db: Some(db_name),
        }
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
/// state already inserted. Panics if called on a `SQLite` `TestEnv`.
pub fn recorded_postgres_url(base: &TempDir) -> String {
    std::fs::read_to_string(base.path().join(PG_URL_FILE))
        .expect("Postgres test URL not recorded; recorded_postgres_url is Postgres-only")
}

impl Backend {
    pub async fn setup(self) -> TestEnv {
        let dir = TempDir::new().unwrap();
        let (state, base) = match self {
            Backend::Sqlite => {
                let state = open_database(&sqlite_url(&dir)).await.unwrap();
                (state, TestBase::sqlite(dir))
            }
            Backend::Postgres => {
                let url = template_postgres_url().await;
                let state = open_existing_database(&url).await.unwrap();
                let DbConnectOptions::Postgres { options, .. } = &url else {
                    unreachable!("template_postgres_url returns Postgres options");
                };
                let db_name = options
                    .get_database()
                    .expect("per-test database URL includes a name")
                    .to_owned();
                // Record the per-test DB URL so raw-SQL helpers reuse this exact
                // database rather than minting a fresh (empty) template clone.
                std::fs::write(dir.path().join(PG_URL_FILE), url.to_string())
                    .expect("write recorded Postgres URL");
                (state, TestBase::postgres(dir, db_name))
            }
        };
        TestEnv { state, base }
    }
}

#[template]
#[rstest]
#[case::sqlite(Backend::Sqlite)]
fn sqlite_only(#[case] backend: Backend) {}

#[template]
#[rstest]
#[case::postgres(Backend::Postgres)]
fn postgres_only(#[case] backend: Backend) {}

#[template]
#[rstest]
#[case::sqlite(Backend::Sqlite)]
#[case::postgres(Backend::Postgres)]
fn backends(#[case] backend: Backend) {}

pub fn ensure_server_fns_registered() {
    static ONCE: OnceLock<()> = OnceLock::new();
    ONCE.get_or_init(|| {
        server_fn::axum::register_explicit::<web::auth::CurrentUser>();
        server_fn::axum::register_explicit::<web::backup::BackupWarningVisible>();
        server_fn::axum::register_explicit::<web::backup::CurrentUserIsOperator>();
        server_fn::axum::register_explicit::<web::backup::GetBackupSettings>();
        server_fn::axum::register_explicit::<web::backup::UpdateBackupSettings>();
        server_fn::axum::register_explicit::<web::auth::GetRegistrationPolicy>();
        server_fn::axum::register_explicit::<web::auth::Register>();
        server_fn::axum::register_explicit::<web::auth::Login>();
        server_fn::axum::register_explicit::<web::auth::Logout>();
        server_fn::axum::register_explicit::<web::email::RequestEmailVerification>();
        server_fn::axum::register_explicit::<web::email::VerifyEmail>();
        server_fn::axum::register_explicit::<web::profile::GetProfile>();
        server_fn::axum::register_explicit::<web::profile::UpdateProfile>();
        server_fn::axum::register_explicit::<web::sessions::ListSessions>();
        server_fn::axum::register_explicit::<web::sessions::RevokeSession>();
        server_fn::axum::register_explicit::<web::invites::CreateInvite>();
        server_fn::axum::register_explicit::<web::invites::ListInvites>();
        server_fn::axum::register_explicit::<web::password_reset::RequestPasswordReset>();
        server_fn::axum::register_explicit::<web::password_reset::ConfirmPasswordReset>();
        server_fn::axum::register_explicit::<web::posts::CreatePost>();
        server_fn::axum::register_explicit::<web::posts::GetPost>();
        server_fn::axum::register_explicit::<web::posts::GetPostPreview>();
        server_fn::axum::register_explicit::<web::posts::UpdatePost>();
        server_fn::axum::register_explicit::<web::posts::ListDrafts>();
        server_fn::axum::register_explicit::<web::posts::PublishPost>();
        server_fn::axum::register_explicit::<web::posts::ListUserPosts>();
        server_fn::axum::register_explicit::<web::posts::ListLocalTimeline>();
        server_fn::axum::register_explicit::<web::posts::ListHomeFeed>();
        server_fn::axum::register_explicit::<web::posts::ListPostsByTag>();
        server_fn::axum::register_explicit::<web::posts::ListUserPostsByTag>();
        server_fn::axum::register_explicit::<web::site::GetSiteIdentity>();
        server_fn::axum::register_explicit::<web::site::UpdateSiteIdentity>();
        server_fn::axum::register_explicit::<web::tags::ListTags>();
        server_fn::axum::register_explicit::<web::subscriptions::SubscribeTo>();
        server_fn::axum::register_explicit::<web::subscriptions::UnsubscribeFrom>();
        server_fn::axum::register_explicit::<web::subscriptions::IsSubscribedTo>();
        server_fn::axum::register_explicit::<web::audiences::CreateAudience>();
        server_fn::axum::register_explicit::<web::audiences::RenameAudience>();
        server_fn::axum::register_explicit::<web::audiences::DeleteAudience>();
        server_fn::axum::register_explicit::<web::audiences::ListMyAudiences>();
        server_fn::axum::register_explicit::<web::audiences::ListMySubscribers>();
        server_fn::axum::register_explicit::<web::audiences::AddSubscriberToAudience>();
        server_fn::axum::register_explicit::<web::audiences::RemoveSubscriberFromAudience>();
        server_fn::axum::register_explicit::<web::audiences::ListAudienceMembers>();
    });
}

pub fn test_options() -> LeptosOptions {
    LeptosOptions::builder().output_name("test").build()
}

/// Returns a `PathBuf` pointing to a temporary directory usable as a storage
/// root.  The caller is responsible for keeping the `TempDir` alive; this
/// function returns the inner path for convenience when lifetime management is
/// not needed (e.g. when storage is never actually written to in the test).
pub fn tmp_storage_path() -> std::path::PathBuf {
    // Return the system temp dir — the media subdirectories are created on
    // demand by the handlers, so the root just needs to exist.
    std::env::temp_dir().join("jaunder-test-storage")
}

pub fn sqlite_url(base: &TempDir) -> DbConnectOptions {
    format!("sqlite:{}", base.path().join("test.db").display())
        .parse()
        .unwrap()
}

pub fn postgres_url() -> DbConnectOptions {
    postgres_url_string().parse().unwrap()
}

pub fn postgres_testing_enabled() -> bool {
    std::env::var("JAUNDER_PG_TEST_URL").is_ok()
}

pub fn postgres_bootstrap_url() -> String {
    std::env::var("JAUNDER_PG_BOOTSTRAP_TEST_URL").unwrap_or_else(|_| {
        let authority = postgres_url_authority(&postgres_url_string());
        format!("postgres://postgres@{authority}/postgres")
    })
}

pub fn postgres_url_string() -> String {
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

pub fn postgres_test_authority() -> String {
    postgres_url_authority(&postgres_bootstrap_url())
}

fn quote_postgres_identifier(name: &str) -> String {
    format!("\"{}\"", name.replace('"', "\"\""))
}

fn postgres_url_with_db_name(db_name: &str) -> String {
    let template = postgres_url_string();
    let (base, query) = template
        .split_once('?')
        .map_or((template.as_str(), None), |(base, query)| {
            (base, Some(query))
        });
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
    let statement = format!(
        "DROP DATABASE {} WITH (FORCE)",
        quote_postgres_identifier(db_name)
    );
    std::thread::scope(|scope| {
        scope.spawn(|| {
            let Ok(runtime) = tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
            else {
                return;
            };
            runtime.block_on(async {
                let Ok(options) = bootstrap.parse::<sqlx::postgres::PgConnectOptions>() else {
                    return;
                };
                let outcome = tokio::time::timeout(std::time::Duration::from_secs(10), async {
                    let mut conn = sqlx::PgConnection::connect_with(&options).await?;
                    let dropped = sqlx::query(&statement).execute(&mut conn).await.map(|_| ());
                    let _ = conn.close().await;
                    dropped
                })
                .await;
                match outcome {
                    Ok(Ok(())) => {}
                    Ok(Err(error)) => {
                        eprintln!("issue #28 teardown: dropping {db_name} failed: {error}");
                    }
                    Err(_elapsed) => {
                        eprintln!("issue #28 teardown: dropping {db_name} timed out after 10s");
                    }
                }
            });
        });
    });
}

pub fn nonexistent_postgres_url() -> DbConnectOptions {
    postgres_url_with_db_name(&unique_postgres_db_name())
        .parse()
        .unwrap()
}

pub async fn unique_postgres_url() -> DbConnectOptions {
    let db_name = unique_postgres_db_name();

    let bootstrap: sqlx::postgres::PgConnectOptions = postgres_bootstrap_url().parse().unwrap();
    let DbConnectOptions::Postgres { options, .. } = postgres_url() else {
        panic!("expected postgres options");
    };
    let owner = options.get_username();
    assert!(
        !owner.is_empty(),
        "PostgreSQL test URL must include a username"
    );

    let mut admin_conn = sqlx::PgConnection::connect_with(&bootstrap).await.unwrap();
    sqlx::query(&format!(
        "CREATE DATABASE {} OWNER {}",
        quote_postgres_identifier(&db_name),
        quote_postgres_identifier(owner),
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
            panic!("expected postgres options");
        };
        let owner = options.get_username();
        sqlx::query(&format!(
            "CREATE DATABASE {} OWNER {}",
            quote_postgres_identifier(TEMPLATE_DB),
            quote_postgres_identifier(owner),
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
pub async fn template_postgres_url() -> DbConnectOptions {
    ensure_template_db().await;

    let DbConnectOptions::Postgres { options, .. } = postgres_url() else {
        panic!("expected postgres options");
    };
    let owner = options.get_username();
    let db_name = unique_postgres_db_name();

    let bootstrap: sqlx::postgres::PgConnectOptions = postgres_bootstrap_url().parse().unwrap();
    let mut admin = sqlx::PgConnection::connect_with(&bootstrap).await.unwrap();
    sqlx::query(&format!(
        "CREATE DATABASE {} OWNER {} TEMPLATE {}",
        quote_postgres_identifier(&db_name),
        quote_postgres_identifier(owner),
        quote_postgres_identifier(TEMPLATE_DB),
    ))
    .execute(&mut admin)
    .await
    .unwrap();

    postgres_url_with_db_name(&db_name).parse().unwrap()
}

/// Default mailer for tests that don't care about email sending. Use with
/// [`create_router`] when you don't have a captured mailer to pass.
pub fn noop_mailer() -> Arc<dyn MailSender> {
    Arc::new(NoopMailSender)
}

/// Like [`test_state`] but also returns the underlying `SQLite` pool for raw SQL access.
/// Only available when Postgres testing is disabled; panics otherwise.
pub async fn test_sqlite_state_with_pool(base: &TempDir) -> (Arc<AppState>, sqlx::SqlitePool) {
    let pool = sqlx::SqlitePool::connect_with(
        format!("sqlite:{}", base.path().join("test.db").display())
            .parse::<sqlx::sqlite::SqliteConnectOptions>()
            .unwrap()
            .create_if_missing(true),
    )
    .await
    .unwrap();
    sqlx::migrate!("../storage/migrations/sqlite")
        .run(&pool)
        .await
        .unwrap();
    let state = Arc::new(AppState {
        site_config: Arc::new(SqliteSiteConfigStorage::new(pool.clone())),
        users: Arc::new(SqliteUserStorage::new(pool.clone())),
        sessions: Arc::new(SqliteSessionStorage::new(pool.clone())),
        invites: Arc::new(SqliteInviteStorage::new(pool.clone())),
        atomic: Arc::new(SqliteAtomicOps::new(pool.clone())),
        email_verifications: Arc::new(SqliteEmailVerificationStorage::new(pool.clone())),
        password_resets: Arc::new(SqlitePasswordResetStorage::new(pool.clone())),
        posts: Arc::new(SqlitePostStorage::new(pool.clone())),
        subscriptions: Arc::new(SqliteSubscriptionStorage::new(
            pool.clone(),
            Arc::new(common::visibility::OpenSubscriptionPolicy),
        )),
        audiences: Arc::new(SqliteAudienceStorage::new(pool.clone())),
        media: Arc::new(SqliteMediaStorage::new(pool.clone())),
        user_config: Arc::new(SqliteUserConfigStorage::new(pool.clone())),
        feed_cache: Arc::new(SqliteFeedCacheStorage::new(pool.clone())),
        feed_events: Arc::new(SqliteFeedEventStorage::new(pool.clone())),
    });
    (state, pool)
}

/// Seeds `count` posts for `user_id` directly through the storage service,
/// bypassing the HTTP/server-fn path (markdown render of trivial bodies is
/// negligible; the cost we avoid is axum routing + `server_fn` per call).
/// `published == true` sets `published_at = now` so list/timeline endpoints
/// return them; `false` leaves them as drafts. Returns ids in creation order.
pub async fn seed_posts(
    state: &Arc<AppState>,
    user_id: i64,
    count: usize,
    published: bool,
) -> Vec<i64> {
    let mut ids = Vec::with_capacity(count);
    for i in 0..count {
        let published_at = if published {
            Some(chrono::Utc::now())
        } else {
            None
        };
        let id = storage::create_rendered_post(
            &*state.posts,
            user_id,
            None,
            format!("seed-{i}").parse().expect("valid slug"),
            format!("# Post {i}\n\nbody"),
            storage::PostFormat::Markdown,
            published_at,
            None,
            vec![common::visibility::AudienceTarget::Public],
        )
        .await
        .expect("seed post should be created");
        ids.push(id);
    }
    ids
}
