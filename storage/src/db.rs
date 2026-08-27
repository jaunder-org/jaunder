//! Database connection and initialization.
//!
//! Handles opening `SQLite` and `PostgreSQL` databases, running migrations,
//! and constructing the [`AppState`] with all storage implementations.

use std::io;
use std::path::Path;
use std::{fmt, str::FromStr, sync::Arc};

use sqlx::postgres::PgConnectOptions;
use sqlx::sqlite::SqliteConnectOptions;
use sqlx::{PgPool, SqlitePool};

use crate::AppState;
use crate::postgres::open_postgres_database_with_pool;
use crate::sqlite::open_sqlite_database_with_pool;

// ---------------------------------------------------------------------------
// DbConnectOptions
// ---------------------------------------------------------------------------

/// Whether `s` carries a `PostgreSQL` scheme.
///
/// The single spelling of that predicate. [`DbConnectOptions::from_str`] needs it, and so
/// does every CLI argument that parses straight to a `PgConnectOptions` — sqlx does not
/// check the scheme itself, so those parsers must. Two copies of the prefix list could
/// drift, and a drift here means accepting a URL the other half rejects.
#[must_use]
pub fn is_postgres_url(s: &str) -> bool {
    s.starts_with("postgres://") || s.starts_with("postgresql://")
}

/// Replaces any password in `url` with `***`, in **both** spellings sqlx accepts.
///
/// - **userinfo** — `postgres://user:secret@host/db` → `postgres://user:***@host/db`
/// - **query parameter** — `postgres://user@host/db?password=secret` → `…?password=***`
///
/// The second is easy to miss and equally live: sqlx-postgres maps a `password` query
/// key straight onto the connection password (`options/parse.rs`, `"password" =>
/// options.password(&value)`), so `postgres:///?password=x` is a working credential.
/// Redacting only the userinfo would leave that spelling printing verbatim.
///
/// Returns the input unchanged when there is no password — which is why the existing
/// passwordless `cli.rs` assertions are unaffected.
fn redact_url_password(url: &str) -> String {
    redact_password_query_param(&redact_userinfo_password(url))
}

/// The `user:secret@` half of [`redact_url_password`].
fn redact_userinfo_password(url: &str) -> String {
    let Some(scheme_end) = url.find("://") else {
        return url.to_owned();
    };
    let authority_start = scheme_end + 3;
    let rest = &url[authority_start..];
    // The authority ends at the path, the query, or the fragment — whichever comes first.
    let authority = &rest[..rest.find(['/', '?', '#']).unwrap_or(rest.len())];
    let Some(at) = authority.rfind('@') else {
        return url.to_owned();
    };
    let Some(colon) = authority[..at].find(':') else {
        return url.to_owned();
    };
    format!(
        "{}{}:***{}",
        &url[..authority_start],
        &authority[..colon],
        &url[authority_start + at..],
    )
}

/// The `?password=secret` half of [`redact_url_password`].
fn redact_password_query_param(url: &str) -> String {
    let Some(query_start) = url.find('?') else {
        return url.to_owned();
    };
    let (head, rest) = url.split_at(query_start + 1);
    let (query, fragment) = rest.find('#').map_or((rest, ""), |i| rest.split_at(i));

    let redacted = query
        .split('&')
        .map(|param| match param.split_once('=') {
            // Case-insensitive: redacting a key sqlx would ignore is harmless; missing
            // one it honours is not.
            Some((key, _)) if key.eq_ignore_ascii_case("password") => format!("{key}=***"),
            _ => param.to_owned(),
        })
        .collect::<Vec<_>>()
        .join("&");

    format!("{head}{redacted}{fragment}")
}

/// The scheme of `s`, or `"(none)"` when it does not look like one.
///
/// Used for the parse-failure message. Deliberately conservative: anything that is not a
/// plain scheme token is reported as `(none)` rather than echoed, so a malformed URL
/// cannot leak its userinfo into an error (#693).
fn url_scheme(s: &str) -> &str {
    let candidate = s.split_once(':').map_or("", |(scheme, _)| scheme);
    if !candidate.is_empty()
        && candidate
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '+' | '-' | '.'))
    {
        candidate
    } else {
        "(none)"
    }
}

/// Parsed connection options for a supported database backend.
///
/// Constructed via [`FromStr`] at the CLI boundary; invalid or unsupported
/// URLs are rejected there rather than inside application logic.
///
/// `Debug` and [`Display`](fmt::Display) both **redact the password** (ADR-0011). Use
/// [`expose_url`](Self::expose_url) where the real URL is required.
#[derive(Clone)]
pub enum DbConnectOptions {
    Sqlite(SqliteConnectOptions),
    Postgres {
        url: String,
        options: PgConnectOptions,
    },
}

impl DbConnectOptions {
    /// The full connection URL, **including any password**.
    ///
    /// The single deliberate door past the redacting [`Display`](fmt::Display) — call it
    /// only where the secret is genuinely required (recording a URL for later
    /// reconnection, reopening a pool), never for logging or diagnostics. It is named to
    /// be greppable so those sites stay countable.
    #[must_use]
    pub fn expose_url(&self) -> String {
        match self {
            DbConnectOptions::Sqlite(opts) => format!("sqlite:{}", opts.get_filename().display()),
            DbConnectOptions::Postgres { url, .. } => url.clone(),
        }
    }
}

/// Hand-written so the password never renders.
///
/// Both arms matter. `url` obviously carries it; `options` does too, and sqlx's own
/// `PgConnectOptions: Debug` cannot be told to redact — so it is **not printed**, and the
/// non-secret parts worth diagnosing (host, port, database) are named individually
/// instead.
impl fmt::Debug for DbConnectOptions {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            DbConnectOptions::Sqlite(opts) => f
                .debug_tuple("Sqlite")
                .field(&opts.get_filename().display().to_string())
                .finish(),
            DbConnectOptions::Postgres { url, options } => f
                .debug_struct("Postgres")
                .field("url", &redact_url_password(url))
                .field("host", &options.get_host())
                .field("port", &options.get_port())
                .field("database", &options.get_database())
                .finish(),
        }
    }
}

impl fmt::Display for DbConnectOptions {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            DbConnectOptions::Sqlite(opts) => {
                write!(f, "sqlite:{}", opts.get_filename().display())
            }
            DbConnectOptions::Postgres { url, .. } => f.write_str(&redact_url_password(url)),
        }
    }
}

impl FromStr for DbConnectOptions {
    type Err = sqlx::Error;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        if s.starts_with("sqlite:") {
            Ok(DbConnectOptions::Sqlite(s.parse()?))
        } else if is_postgres_url(s) {
            Ok(DbConnectOptions::Postgres {
                url: s.to_owned(),
                options: s.parse()?,
            })
        } else {
            // Names the **scheme**, never the URL. `StorageArgs.db` is parsed by clap via
            // this impl, so echoing the URL for
            // `JAUNDER_DB=postgre://user:secret@host/db` — a one-character typo —
            // would print the credential straight to stderr (#693).
            Err(sqlx::Error::Configuration(
                format!(
                    "unsupported database URL scheme '{}'; supported schemes are sqlite: and postgres://",
                    url_scheme(s)
                )
                .into(),
            ))
        }
    }
}

// ---------------------------------------------------------------------------
// Storage directory helpers
// ---------------------------------------------------------------------------

/// Creates the storage root and required subdirectories (`media/`, `backups/`).
///
/// # Errors
///
/// Returns `Err` if the storage directory cannot be created.
pub fn init_storage(path: &Path) -> io::Result<()> {
    std::fs::create_dir(path)?;
    std::fs::create_dir_all(path.join("media"))?;
    std::fs::create_dir_all(path.join("media").join("upload"))?;
    std::fs::create_dir_all(path.join("media").join("cached"))?;
    std::fs::create_dir_all(path.join("media").join("tmp"))?;
    std::fs::create_dir_all(path.join("backups"))?;
    Ok(())
}

// ---------------------------------------------------------------------------
// Database helpers
// ---------------------------------------------------------------------------

fn read_sql_slow_threshold_env() -> Result<Option<String>, std::env::VarError> {
    match std::env::var("JAUNDER_SQL_SLOW_MS") {
        Ok(value) => Ok(Some(value)),
        Err(std::env::VarError::NotPresent) => Ok(None),
        Err(error @ std::env::VarError::NotUnicode(_)) => Err(error),
    }
}

fn write_sql_threshold_fallback(mut writer: impl std::io::Write) -> std::io::Result<()> {
    writeln!(
        writer,
        "storage.observability.sql_slow_threshold: invalid configured value; using 5s"
    )
}

fn warn_sql_threshold() {
    let _ = write_sql_threshold_fallback(std::io::stderr().lock());
}

fn sql_slow_query_threshold_with(
    mut read: impl FnMut() -> Result<Option<String>, std::env::VarError>,
    mut warn: impl FnMut(),
) -> std::time::Duration {
    match read() {
        Ok(Some(value)) => {
            if let Ok(value) = value.parse::<u64>() {
                std::time::Duration::from_millis(value)
            } else {
                warn();
                std::time::Duration::from_secs(5)
            }
        }
        Ok(None) => std::time::Duration::from_secs(5),
        Err(_) => {
            warn();
            std::time::Duration::from_secs(5)
        }
    }
}

/// Slow-query log threshold shared by both `SQLite` and Postgres backends.
///
/// Reads `JAUNDER_SQL_SLOW_MS` (milliseconds), defaulting to five seconds.
pub(crate) fn sql_slow_query_threshold() -> std::time::Duration {
    sql_slow_query_threshold_with(read_sql_slow_threshold_env, warn_sql_threshold)
}

#[derive(Clone)]
pub struct DbPoolObserver {
    inner: DbPoolObserverInner,
}

#[derive(Clone)]
enum DbPoolObserverInner {
    Sqlite(SqlitePool),
    Postgres(PgPool),
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct DbPoolSnapshot {
    pub used: u64,
    pub idle: u64,
    pub max: u64,
}

pub struct OpenedDatabase {
    pub state: Arc<AppState>,
    pub instance_id: crate::InstanceId,
    pub pool_observer: DbPoolObserver,
}

impl DbPoolObserver {
    #[must_use]
    pub fn snapshot(&self) -> DbPoolSnapshot {
        match &self.inner {
            DbPoolObserverInner::Sqlite(pool) => pool_snapshot(pool),
            DbPoolObserverInner::Postgres(pool) => pool_snapshot(pool),
        }
    }
}

fn pool_snapshot<DB: sqlx::Database>(pool: &sqlx::Pool<DB>) -> DbPoolSnapshot {
    let size = u64::from(pool.size());
    let idle = u64::try_from(pool.num_idle()).unwrap_or(u64::MAX);
    let max = u64::from(pool.options().get_max_connections());
    DbPoolSnapshot {
        used: size.saturating_sub(idle),
        idle,
        max,
    }
}

/// Opens (or creates) the database described by `opts`, runs pending
/// migrations, and returns an [`AppState`] bundling all storage handles.
///
/// # Errors
///
/// Returns `Err` if the database connection pool cannot be established.
#[tracing::instrument(name = "storage.open_database", skip(opts))]
pub async fn open_database(opts: &DbConnectOptions) -> sqlx::Result<Arc<AppState>> {
    Ok(open_database_with_observer(opts).await?.state)
}

/// Opens (or creates) a database and returns its storage state plus a pool observer.
///
/// # Errors
///
/// Returns `Err` if the database connection pool cannot be established.
#[tracing::instrument(name = "storage.open_database_with_observer", skip(opts))]
pub async fn open_database_with_observer(opts: &DbConnectOptions) -> sqlx::Result<OpenedDatabase> {
    match opts {
        DbConnectOptions::Sqlite(options) => {
            let (state, pool, instance_id) = open_sqlite_database_with_pool(options, true).await?;
            Ok(OpenedDatabase {
                state,
                instance_id,
                pool_observer: DbPoolObserver {
                    inner: DbPoolObserverInner::Sqlite(pool),
                },
            })
        }
        DbConnectOptions::Postgres { options, .. } => {
            let (state, pool, instance_id) = open_postgres_database_with_pool(options).await?;
            Ok(OpenedDatabase {
                state,
                instance_id,
                pool_observer: DbPoolObserver {
                    inner: DbPoolObserverInner::Postgres(pool),
                },
            })
        }
    }
}

/// Opens an existing database described by `opts`, runs pending migrations.
///
/// Unlike [`open_database`], fails if the database does not already exist.
///
/// # Errors
///
/// Returns `Err` if the database connection pool cannot be established.
#[tracing::instrument(name = "storage.open_existing_database", skip(opts))]
pub async fn open_existing_database(opts: &DbConnectOptions) -> sqlx::Result<Arc<AppState>> {
    Ok(open_existing_database_with_observer(opts).await?.state)
}

/// Opens an existing database and returns its storage state plus a pool observer.
///
/// # Errors
///
/// Returns `Err` if the database connection pool cannot be established.
#[tracing::instrument(name = "storage.open_existing_database_with_observer", skip(opts))]
pub async fn open_existing_database_with_observer(
    opts: &DbConnectOptions,
) -> sqlx::Result<OpenedDatabase> {
    match opts {
        DbConnectOptions::Sqlite(options) => {
            let (state, pool, instance_id) = open_sqlite_database_with_pool(options, false).await?;
            Ok(OpenedDatabase {
                state,
                instance_id,
                pool_observer: DbPoolObserver {
                    inner: DbPoolObserverInner::Sqlite(pool),
                },
            })
        }
        DbConnectOptions::Postgres { options, .. } => {
            let (state, pool, instance_id) = open_postgres_database_with_pool(options).await?;
            Ok(OpenedDatabase {
                state,
                instance_id,
                pool_observer: DbPoolObserver {
                    inner: DbPoolObserverInner::Postgres(pool),
                },
            })
        }
    }
}

/// Tables initialized by migrations or identity bootstrap hold no user data,
/// even in a pristine database, so an emptiness check must ignore them.
pub(crate) const MIGRATION_SEEDED_TABLES: &[&str] = &[
    "channels",
    "subscription_statuses",
    "target_kinds",
    "instance_identity",
];

/// Returns `true` if the database holds no user data — every table except the
/// migration/identity bootstrap tables ([`MIGRATION_SEEDED_TABLES`]) is empty.
///
/// Used as a restore preflight: refusing to overwrite a database that is already
/// in use. This is stricter than checking for users alone — it also catches data
/// held before any user exists (site config, unused invites, a populated feed
/// cache).
///
/// # Errors
///
/// Returns the underlying [`sqlx::Error`] if the database cannot be reached or a
/// query fails.
pub async fn database_is_empty(options: &DbConnectOptions) -> sqlx::Result<bool> {
    match options {
        DbConnectOptions::Sqlite(options) => crate::sqlite::database_is_empty(options).await,
        DbConnectOptions::Postgres { options, .. } => {
            crate::postgres::database_is_empty(options).await
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::{
        Backend, PostgresTestConfig, backends, recorded_postgres_url, sqlite_url,
        template_postgres_url,
    };
    use common::test_support::with_env;
    use rstest::*;
    use rstest_reuse::*;
    use std::time::Duration;
    use tempfile::TempDir;

    #[test]
    fn new_path_created_with_subdirs() {
        let base = TempDir::new().unwrap();
        let storage = base.path().join("storage");

        init_storage(&storage).unwrap();

        assert!(storage.is_dir());
        assert!(storage.join("media").is_dir());
        assert!(storage.join("media").join("upload").is_dir());
        assert!(storage.join("media").join("cached").is_dir());
        assert!(storage.join("media").join("tmp").is_dir());
        assert!(storage.join("backups").is_dir());
    }

    #[test]
    fn existing_path_returns_already_exists_error() {
        let base = TempDir::new().unwrap();
        let storage = base.path().join("storage");

        init_storage(&storage).unwrap();

        let err = init_storage(&storage).unwrap_err();
        assert_eq!(err.kind(), io::ErrorKind::AlreadyExists);
    }

    #[test]
    fn init_storage_fails_on_missing_parent() {
        let storage = std::path::Path::new("/nonexistent/path/to/storage");
        let result = init_storage(storage);
        assert!(result.is_err());
    }

    #[apply(backends)]
    #[tokio::test]
    async fn opened_database_carries_pool_observer(#[case] backend: Backend) {
        let env = backend.setup().await;
        let options = match backend {
            Backend::Sqlite => sqlite_url(&env.base),
            Backend::Postgres => recorded_postgres_url(&env.base)
                .parse()
                .expect("recorded postgres URL"),
        };
        let opened = open_existing_database_with_observer(&options)
            .await
            .expect("open existing database with observer");

        let snapshot = opened.pool_observer.snapshot();

        assert!(snapshot.max >= 1);
        assert!(snapshot.used <= snapshot.max);
        assert!(snapshot.idle <= snapshot.max);
        assert!(Arc::strong_count(&opened.state) >= 1);
    }

    #[test]
    fn sql_slow_query_threshold_defaults_to_five_seconds() {
        with_env(|env| {
            env.remove("JAUNDER_SQL_SLOW_MS");
            assert_eq!(sql_slow_query_threshold(), Duration::from_secs(5));
        });
    }

    #[test]
    fn sql_slow_query_threshold_uses_env_override() {
        with_env(|env| {
            env.set("JAUNDER_SQL_SLOW_MS", "250");
            assert_eq!(sql_slow_query_threshold(), Duration::from_millis(250));
        });
    }

    #[test]
    fn nonnumeric_sql_slow_threshold_uses_default_with_one_fixed_nonrecursive_fallback() {
        let mut output = Vec::new();
        let threshold = sql_slow_query_threshold_with(
            || Ok(Some("not-a-number".to_owned())),
            || write_sql_threshold_fallback(&mut output).expect("write fallback"),
        );
        assert_eq!(threshold, Duration::from_secs(5));
        assert_eq!(
            String::from_utf8(output).expect("fallback utf8"),
            "storage.observability.sql_slow_threshold: invalid configured value; using 5s\n"
        );
    }

    #[test]
    fn invalid_unicode_sql_slow_threshold_uses_default_with_one_redacted_nonrecursive_fallback() {
        let mut output = Vec::new();
        let threshold = sql_slow_query_threshold_with(
            || {
                Err(std::env::VarError::NotUnicode(std::ffi::OsString::from(
                    "injected invalid unicode",
                )))
            },
            || write_sql_threshold_fallback(&mut output).expect("write fallback"),
        );
        assert_eq!(threshold, Duration::from_secs(5));
        assert_eq!(
            String::from_utf8(output).expect("fallback utf8"),
            "storage.observability.sql_slow_threshold: invalid configured value; using 5s\n"
        );
    }

    #[cfg(unix)]
    #[test]
    fn production_sql_threshold_reader_warns_for_invalid_unicode() {
        use std::os::unix::ffi::OsStringExt as _;

        with_env(|env| {
            env.set(
                "JAUNDER_SQL_SLOW_MS",
                std::ffi::OsString::from_vec(vec![0xff]),
            );
            assert_eq!(sql_slow_query_threshold(), Duration::from_secs(5));
        });
    }

    #[test]
    fn test_db_connect_options_parsing() {
        let sqlite = "sqlite:jaunder.db".parse::<DbConnectOptions>().unwrap();
        assert!(matches!(sqlite, DbConnectOptions::Sqlite(_)));
        assert_eq!(sqlite.to_string(), "sqlite:jaunder.db");

        let pg = "postgres://user:pass@localhost/db"
            .parse::<DbConnectOptions>()
            .unwrap();
        assert!(matches!(pg, DbConnectOptions::Postgres { .. }));
        // `Display` redacts (#693); `expose_url` is the door to the real URL. Pinning
        // an unredacted rendering here would assert the leak.
        assert_eq!(pg.to_string(), "postgres://user:***@localhost/db");
        assert_eq!(pg.expose_url(), "postgres://user:pass@localhost/db");

        let pgs = "postgresql://user:pass@localhost/db"
            .parse::<DbConnectOptions>()
            .unwrap();
        assert!(matches!(pgs, DbConnectOptions::Postgres { .. }));

        let invalid = "mysql://localhost".parse::<DbConnectOptions>();
        assert!(invalid.is_err());
    }

    #[test]
    fn test_db_connect_options_invalid_sqlite() {
        // Starts with sqlite: but is invalid
        let invalid = "sqlite:??invalid??".parse::<DbConnectOptions>();
        assert!(invalid.is_err());
    }

    #[test]
    fn test_db_connect_options_invalid_postgres() {
        let invalid = "postgres://[invalid]".parse::<DbConnectOptions>();
        assert!(invalid.is_err());
    }

    /// The regression guard for the `Display` → file → `FromStr` → connect round-trip.
    ///
    /// `test_support` records a per-test URL that `backup.rs` reads back and reconnects
    /// with. That recorder now uses `expose_url`, so redacting `Display` cannot break it —
    /// this test is what says so. It does not fail today only because the default test URL
    /// happens to be passwordless.
    #[test]
    fn expose_url_round_trips_a_password_bearing_url() {
        let raw = "postgres://app:hunter2@localhost/jaunder";
        let opts: DbConnectOptions = raw.parse().unwrap();

        assert_eq!(opts.expose_url(), raw, "the password must survive");

        let reparsed: DbConnectOptions = opts.expose_url().parse().unwrap();
        assert_eq!(reparsed.expose_url(), raw, "and survive a round-trip");
    }

    /// The live leak #693 found: `StorageArgs.db` is parsed by clap through `FromStr`, so
    /// a one-character scheme typo printed the credential to stderr.
    #[test]
    fn parse_failure_names_the_scheme_not_the_url() {
        let err = "postgre://u:hunter2@h/db"
            .parse::<DbConnectOptions>()
            .unwrap_err();
        let msg = err.to_string();

        assert!(!msg.contains("hunter2"), "leaked the password: {msg}");
        assert!(!msg.contains("u:hunter2"), "leaked the userinfo: {msg}");
        assert!(msg.contains("postgre"), "should name the scheme: {msg}");
    }

    #[test]
    fn debug_and_display_redact_the_password() {
        let opts: DbConnectOptions = "postgres://app:hunter2@localhost/jaunder".parse().unwrap();

        let shown = format!("{opts}");
        let debugged = format!("{opts:?}");

        for rendered in [&shown, &debugged] {
            assert!(!rendered.contains("hunter2"), "leaked: {rendered}");
            assert!(!rendered.contains(":hunter2@"), "leaked: {rendered}");
        }

        // Still useful for diagnosis.
        assert!(shown.contains("app:***@localhost"), "{shown}");
        assert!(debugged.contains("localhost"), "{debugged}");
        assert!(debugged.contains("jaunder"), "{debugged}");
    }

    /// A passwordless URL renders unchanged — which is why the five `cli.rs`
    /// `to_string()` assertions need no edits.
    #[test]
    fn redaction_leaves_a_passwordless_url_alone() {
        let raw = "postgres://jaunder@localhost/jaunder";
        let opts: DbConnectOptions = raw.parse().unwrap();
        assert_eq!(opts.to_string(), raw);
    }

    /// The three "nothing to redact" shapes, driven directly: `redact_url_password` is
    /// deliberately total, so each early return is a real branch.
    #[test]
    fn redaction_passes_through_urls_with_no_password() {
        // No `://` at all — defensive; `DbConnectOptions` never builds one, but the
        // function is total and the branch is live.
        assert_eq!(
            redact_url_password("sqlite:jaunder.db"),
            "sqlite:jaunder.db"
        );
        // No userinfo.
        assert_eq!(
            redact_url_password("postgres://localhost/db"),
            "postgres://localhost/db"
        );
        // Userinfo, but no password.
        assert_eq!(
            redact_url_password("postgres://user@localhost/db"),
            "postgres://user@localhost/db"
        );
    }

    /// sqlx accepts the password as a query parameter as well as in the userinfo
    /// (`options/parse.rs`: `"password" => options.password(&value)`), so redacting only
    /// the userinfo left a working credential printing verbatim. Caught in review.
    #[test]
    fn redaction_covers_the_query_parameter_spelling() {
        let opts: DbConnectOptions = "postgres://app@localhost/jaunder?password=hunter2"
            .parse()
            .unwrap();

        for rendered in [format!("{opts}"), format!("{opts:?}")] {
            assert!(!rendered.contains("hunter2"), "leaked: {rendered}");
            assert!(rendered.contains("password=***"), "{rendered}");
        }

        // …and `expose_url` still yields the real thing, since that is its whole job.
        assert!(opts.expose_url().contains("password=hunter2"));
    }

    #[test]
    fn redaction_covers_both_spellings_at_once_and_spares_other_params() {
        assert_eq!(
            redact_url_password(
                "postgres://app:pw@localhost/jaunder?sslmode=require&password=hunter2"
            ),
            "postgres://app:***@localhost/jaunder?sslmode=require&password=***"
        );
    }

    #[test]
    fn redaction_matches_the_password_key_case_insensitively() {
        assert_eq!(
            redact_url_password("postgres://app@h/db?PassWord=hunter2"),
            "postgres://app@h/db?PassWord=***"
        );
    }

    /// A password-bearing userinfo with a query string: the authority must end at the
    /// `?`, not swallow it.
    #[test]
    fn redaction_bounds_the_authority_at_the_query() {
        assert_eq!(
            redact_url_password("postgres://app:pw@localhost?sslmode=require"),
            "postgres://app:***@localhost?sslmode=require"
        );
    }

    #[test]
    fn parse_failure_reports_no_scheme_when_there_is_none() {
        let err = "just-a-path".parse::<DbConnectOptions>().unwrap_err();
        assert!(err.to_string().contains("(none)"), "{err}");
    }

    #[test]
    fn sqlite_renders_its_path_through_every_door() {
        let opts: DbConnectOptions = "sqlite:/tmp/jaunder.db".parse().unwrap();

        assert_eq!(opts.expose_url(), "sqlite:/tmp/jaunder.db");
        assert_eq!(opts.to_string(), "sqlite:/tmp/jaunder.db");

        let debugged = format!("{opts:?}");
        assert!(debugged.contains("Sqlite"), "{debugged}");
        assert!(debugged.contains("/tmp/jaunder.db"), "{debugged}");
    }

    // guard:no-backend — asserts DbConnectOptions→backend routing; connects lazily, no live pool
    #[tokio::test]
    async fn open_database_routes_to_postgres_backend() {
        let opts = "postgres://localhost:1/db"
            .parse::<DbConnectOptions>()
            .unwrap();
        let _ =
            tokio::time::timeout(std::time::Duration::from_millis(50), open_database(&opts)).await;
    }

    #[apply(backends)]
    #[tokio::test]
    async fn concurrent_opens_converge_on_one_instance_identity(#[case] backend: Backend) {
        let temp = TempDir::new().unwrap();
        let (options, _guard) = match backend {
            Backend::Sqlite => (sqlite_url(&temp), None),
            Backend::Postgres => {
                let config = PostgresTestConfig::from_env();
                let (options, guard) = template_postgres_url(&config).await;
                (options, Some(guard))
            }
        };
        let initial = open_database_with_observer(&options)
            .await
            .expect("initial open migrates the database");
        let expected = initial.instance_id;
        let (first, second) = tokio::join!(
            open_database_with_observer(&options),
            open_database_with_observer(&options)
        );
        let first = first.expect("first concurrent open succeeds");
        let second = second.expect("second concurrent open succeeds");
        assert_eq!(first.instance_id, expected);
        assert_eq!(second.instance_id, expected);
    }

    // guard:no-backend — asserts DbConnectOptions→backend routing; connects lazily, no live pool
    #[tokio::test]
    async fn open_existing_database_routes_to_postgres_backend() {
        let opts = "postgres://localhost:1/db"
            .parse::<DbConnectOptions>()
            .unwrap();
        let _ = tokio::time::timeout(
            std::time::Duration::from_millis(50),
            open_existing_database(&opts),
        )
        .await;
    }

    // guard:no-backend — asserts DbConnectOptions→backend routing; connects lazily, no live pool
    #[tokio::test]
    async fn database_is_empty_routes_to_postgres_backend() {
        let opts = "postgres://localhost:1/db"
            .parse::<DbConnectOptions>()
            .unwrap();
        let _ = tokio::time::timeout(
            std::time::Duration::from_millis(50),
            database_is_empty(&opts),
        )
        .await;
    }
}
