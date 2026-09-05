//! Postgres-specific per-test database configuration, clone provisioning, and RAII
//! teardown. Backend-neutral environment setup and template selection live in [`super::backend`].
use crate::DbConnectOptions;
use crate::sql::QueryStorageExt;
use crate::sql::{Exists, quote_identifier};

use sqlx::{Connection, PgPool};
use std::sync::atomic::{AtomicU64, Ordering};
use tempfile::TempDir;

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
                    let dropped = sqlx::query(sqlx::AssertSqlSafe(statement))
                        .execute(&mut conn)
                        .await
                        .map(|_| ());
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
    sqlx::query(sqlx::AssertSqlSafe(format!(
        "CREATE DATABASE {} OWNER {}",
        quote_identifier(&db_name),
        quote_identifier(owner),
    )))
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
#[derive(Debug, macros::SqlxBridge)]
pub(crate) struct TemplateDatabaseName(String);

impl TemplateDatabaseName {
    fn new() -> Self {
        Self("jaunder_test_template".to_owned())
    }

    fn as_str(&self) -> &str {
        &self.0
    }
}

/// Advisory-lock key serialising template creation across nextest's
/// process-per-test workers. The first worker migrates the template; the rest
/// see it already exists and skip straight to cloning.
#[derive(Clone, Copy, Debug, macros::SqlxBridge)]
pub(crate) struct TemplateDatabaseLockKey(i64);

const TEMPLATE_LOCK_KEY: TemplateDatabaseLockKey = TemplateDatabaseLockKey(78_316_621);

/// Ensures `template` exists and is fully migrated. Safe to call concurrently
/// from many processes: creation is guarded by a session-level advisory lock
/// taken on the bootstrap connection.
async fn ensure_template_db(config: &PostgresTestConfig, template: &TemplateDatabaseName) {
    let bootstrap: sqlx::postgres::PgConnectOptions = config.bootstrap_url().parse().unwrap();
    let mut admin = sqlx::PgConnection::connect_with(&bootstrap).await.unwrap();

    sqlx::query("SELECT pg_advisory_lock($1)")
        .bind_storage(TEMPLATE_LOCK_KEY)
        .execute(&mut admin)
        .await
        .unwrap();

    let exists = sqlx::query_scalar::<_, Exists>(
        "SELECT EXISTS(SELECT 1 FROM pg_database WHERE datname = $1)",
    )
    .bind_storage(template)
    .fetch_one(&mut admin)
    .await
    .unwrap()
    .into_bool();
    if !exists {
        let DbConnectOptions::Postgres { options, .. } = config.test_url().parse().unwrap() else {
            unreachable!("PostgreSQL test URL always yields PostgreSQL options")
        };
        let owner = options.get_username();
        sqlx::query(sqlx::AssertSqlSafe(format!(
            "CREATE DATABASE {} OWNER {}",
            quote_identifier(template.as_str()),
            quote_identifier(owner),
        )))
        .execute(&mut admin)
        .await
        .unwrap();

        // Migrate the template through its own pool, then close it: a database
        // can only serve as a CREATE DATABASE template when nobody is connected
        // to it.
        let pool = PgPool::connect(&postgres_url_with_db_name(config, template.as_str()))
            .await
            .unwrap();
        sqlx::migrate!("../storage/migrations/postgres")
            .run(&pool)
            .await
            .unwrap();
        pool.close().await;
    }

    sqlx::query("SELECT pg_advisory_unlock($1)")
        .bind_storage(TEMPLATE_LOCK_KEY)
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
    let template = TemplateDatabaseName::new();
    ensure_template_db(config, &template).await;

    let DbConnectOptions::Postgres { options, .. } = config.test_url().parse().unwrap() else {
        unreachable!("PostgreSQL test URL always yields PostgreSQL options")
    };
    let owner = options.get_username();
    let db_name = unique_postgres_db_name();

    let bootstrap: sqlx::postgres::PgConnectOptions = config.bootstrap_url().parse().unwrap();
    let mut admin = sqlx::PgConnection::connect_with(&bootstrap).await.unwrap();
    sqlx::query(sqlx::AssertSqlSafe(format!(
        "CREATE DATABASE {} OWNER {} TEMPLATE {}",
        quote_identifier(&db_name),
        quote_identifier(owner),
        quote_identifier(template.as_str()),
    )))
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

#[cfg(test)]
mod tests {
    use super::{
        PostgresDbGuard, PostgresTestConfig, bootstrap_url, report_drop_outcome, splice_db_name,
    };

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
}
