use std::{
    fs, io,
    net::SocketAddr,
    path::{Path, PathBuf},
};

use sqlx::postgres::PgConnectOptions;

use crate::cli::{Commands, StorageArgs};
use crate::mailer::LettreMailSender;
use crate::runtime_file;
use common::backup::BackupMode;
use common::display_name::DisplayName;
use common::email::Email;
use common::mailer::{EmailMessage, MailSender};
use common::password::Password;
use common::username::Username;
use host::capture;
use leptos::prelude::{Env, LeptosOptions};
use storage::load_smtp_config;
use storage::{
    export_backup, restore_backup, BackupExportOptions, BackupRestoreOptions, DbConnectOptions,
};
use storage::{init_storage, open_database, open_existing_database};

/// Parse an optional CLI password string into `Option<Password>` (`None` stays
/// `None`), surfacing the validation error as an `anyhow` message.
fn parse_password(p: Option<String>) -> anyhow::Result<Option<Password>> {
    p.map(|p| p.parse::<Password>())
        .transpose()
        .map_err(|e| anyhow::anyhow!("{e}"))
}

impl Commands {
    /// Dispatch this parsed subcommand to its handler. A flat match-expression:
    /// each arm evaluates to the command's `Result<()>`, so there is no `?` on the
    /// dispatch call and no trailing `Ok(())` (the two arms that parse a CLI
    /// newtype still `?` on that parse) — keeping any single function's cyclomatic
    /// complexity (and thus CRAP) low as subcommands are added (#147).
    ///
    /// # Errors
    ///
    /// Propagates the selected command's failure.
    pub async fn execute(self) -> anyhow::Result<()> {
        match self {
            Commands::Init {
                storage,
                skip_if_exists,
            } => cmd_init(&storage, skip_if_exists).await,
            Commands::CreatePgDb { pg } => {
                cmd_create_pg_db(&pg.bootstrap_db, &pg.app_db, &pg.app_role_password).await
            }
            Commands::Serve {
                storage,
                bind,
                environment,
                runtime_file,
            } => cmd_serve(&storage, bind, environment.is_prod(), runtime_file).await,
            Commands::UserCreate {
                storage,
                username,
                password,
                display_name,
                operator,
            } => {
                cmd_user_create(
                    &storage,
                    &username,
                    parse_password(password)?,
                    display_name.as_ref(),
                    operator,
                )
                .await
            }
            Commands::AppPasswordCreate {
                storage,
                username,
                label,
            } => cmd_app_password_create(&storage, &username, &label).await,
            Commands::UserInvite {
                storage,
                expires_in,
            } => cmd_user_invite(&storage, expires_in).await,
            Commands::SmtpTest { storage, to } => cmd_smtp_test(&storage, &to).await,
            Commands::Backup {
                storage,
                mode,
                path,
            } => cmd_backup(&storage, mode.into(), path).await.map(drop),
            Commands::Restore { storage, path } => cmd_restore(&storage, &path).await,
        }
    }
}

/// Initializes the application's storage directory and database.
///
/// # Errors
///
/// Returns an error if the storage directory cannot be created, or if the
/// database cannot be initialized.
pub async fn cmd_init(storage: &StorageArgs, skip_if_exists: bool) -> anyhow::Result<()> {
    match init_storage(&storage.storage_path) {
        Ok(()) => {}
        Err(e) if skip_if_exists && e.kind() == io::ErrorKind::AlreadyExists => {}
        Err(e) => return Err(e.into()),
    }
    open_database(&storage.db).await?;
    println!(
        "Initialized: storage={} db={}",
        storage.storage_path.display(),
        storage.db,
    );
    Ok(())
}

fn require_postgres_options(
    opts: &DbConnectOptions,
    label: &str,
) -> anyhow::Result<PgConnectOptions> {
    match opts {
        DbConnectOptions::Postgres { options, .. } => Ok(options.clone()),
        DbConnectOptions::Sqlite(_) => Err(anyhow::anyhow!("{label} must be a PostgreSQL URL")),
    }
}

/// Maps a [`storage::PgBootstrapError`] to a user-facing CLI error.
fn describe_bootstrap_error(err: storage::PgBootstrapError) -> anyhow::Error {
    match err {
        storage::PgBootstrapError::RoleExists(role) => anyhow::anyhow!(
            "application role '{role}' already exists; refusing to modify existing role state"
        ),
        storage::PgBootstrapError::DatabaseExists(name) => anyhow::anyhow!(
            "database '{name}' already exists; refusing to modify existing database state"
        ),
        storage::PgBootstrapError::Sqlx(err) => err.into(),
    }
}

/// Bootstraps a `PostgreSQL` database and application role.
///
/// # Errors
///
/// Returns an error if the bootstrap connection fails, or if the role or
/// database already exists.
pub async fn cmd_create_pg_db(
    bootstrap_db: &str,
    app_db_url: &str,
    app_role_password: &str,
) -> anyhow::Result<()> {
    let bootstrap_options = require_postgres_options(&bootstrap_db.parse()?, "--bootstrap-db")?;
    let app_options = require_postgres_options(&app_db_url.parse()?, "--app-db")?;
    let app_role = app_options.get_username().to_owned();
    let database_name = app_options
        .get_database()
        .ok_or_else(|| anyhow::anyhow!("--app-db must include a PostgreSQL database name"))?
        .to_owned();

    storage::create_postgres_database_and_role(
        &bootstrap_options,
        &app_role,
        app_role_password,
        &database_name,
    )
    .await
    .map_err(describe_bootstrap_error)?;

    println!("PostgreSQL ready: role='{app_role}' database='{database_name}' owner='{app_role}'");
    Ok(())
}

/// Creates a new user in the database.
///
/// # Errors
///
/// Returns an error if the database cannot be opened, or if the user creation
/// fails (e.g., duplicate username).
pub async fn cmd_user_create(
    storage: &StorageArgs,
    username: &Username,
    password: Option<Password>,
    display_name: Option<&DisplayName>,
    is_operator: bool,
) -> anyhow::Result<()> {
    let state = open_existing_database(&storage.db)
        .await
        .map_err(|e| anyhow::anyhow!("{e}; run `jaunder init` first"))?;

    let password = if let Some(p) = password {
        p
    } else {
        // cov:ignore-start
        let p1 = rpassword::prompt_password("Password: ")?;
        let p2 = rpassword::prompt_password("Confirm password: ")?;
        if p1 != p2 {
            return Err(anyhow::anyhow!("passwords do not match"));
        }
        p1.parse::<Password>().map_err(|e| anyhow::anyhow!("{e}"))?
        // cov:ignore-stop
    };

    let user_id = state
        .users
        .create_user(username, &password, display_name, is_operator)
        .await
        .map_err(|e| anyhow::anyhow!("{e}"))?;

    // CLI user creation bypasses the site registration policy entirely.
    host::metrics::registration(
        host::metrics::RegistrationSource::Cli,
        host::metrics::RegistrationPolicy::CliBypass,
        host::metrics::RegistrationResult::Ok,
    );

    println!("Created user '{username}' with id {user_id}");
    Ok(())
}

/// Mints an app password (a labelled session token) for an existing user and
/// returns the raw token. This is the only out-of-process minter (see ADR-0035).
///
/// # Errors
///
/// Returns an error if the user does not exist or the session cannot be created.
pub async fn app_password_create(
    state: &storage::AppState,
    username: &Username,
    label: &str,
) -> anyhow::Result<String> {
    let user = state
        .users
        .get_user_by_username(username)
        .await
        .map_err(|e| anyhow::anyhow!("{e}"))?
        .ok_or_else(|| anyhow::anyhow!("no such user '{username}'"))?;
    let token = state
        .sessions
        .create_session(user.user_id, label)
        .await
        .map_err(|e| anyhow::anyhow!("{e}"))?;
    Ok(token)
}

/// CLI wrapper: opens the database, mints an app password, prints it to stdout.
///
/// # Errors
///
/// Returns an error if the database cannot be opened or minting fails.
pub async fn cmd_app_password_create(
    storage: &StorageArgs,
    username: &Username,
    label: &str,
) -> anyhow::Result<()> {
    let state = open_existing_database(&storage.db)
        .await
        .map_err(|e| anyhow::anyhow!("{e}; run `jaunder init` first"))?;
    let token = app_password_create(&state, username, label).await?;
    println!("{token}");
    Ok(())
}

/// Generates a new invitation code.
///
/// # Errors
///
/// Returns an error if the database cannot be opened, or if the invitation
/// cannot be saved.
pub async fn cmd_user_invite(storage: &StorageArgs, expires_in: Option<u64>) -> anyhow::Result<()> {
    let state = open_existing_database(&storage.db)
        .await
        .map_err(|e| anyhow::anyhow!("{e}; run `jaunder init` first"))?;

    let hours_u64 = expires_in.unwrap_or(168);
    let hours = i64::try_from(hours_u64)
        .map_err(|_| anyhow::anyhow!("--expires-in value {hours_u64} is too large"))?;
    let expires_at = chrono::Utc::now() + chrono::Duration::hours(hours);

    let code = state.invites.create_invite(expires_at).await?;
    host::metrics::invite(host::metrics::InviteEvent::Created);
    // Deliberate operator-facing reveal via `AsRef` (InviteCode has no Display/serde). With a
    // configured base URL, print a ready-to-send invitation link; otherwise the bare code.
    match state.site_config.get_identity().await?.base_url {
        Some(base_url) => println!("{base_url}/register?invite_code={}", code.as_ref()),
        None => println!("{}", code.as_ref()),
    }
    Ok(())
}

/// Sends a test email using the configured SMTP settings.
///
/// # Errors
///
/// Returns an error if SMTP is not configured, or if the test email cannot be
/// sent.
pub async fn cmd_smtp_test(storage: &StorageArgs, to: &str) -> anyhow::Result<()> {
    let state = open_existing_database(&storage.db)
        .await
        .map_err(|e| anyhow::anyhow!("{e}; run `jaunder init` first"))?;

    let smtp_config = load_smtp_config(state.site_config.as_ref())
        .await
        .map_err(|e| anyhow::anyhow!("SMTP misconfigured: {e}"))?
        .ok_or_else(|| anyhow::anyhow!("SMTP is not configured"))?;

    let mailer = LettreMailSender::from_config(&smtp_config)
        .map_err(|e| anyhow::anyhow!("Failed to build SMTP transport: {e}"))?;

    let to_addr: Email = to
        .parse()
        .map_err(|e| anyhow::anyhow!("Invalid recipient '{to}': {e}"))?;

    let message = EmailMessage {
        from: None,
        to: vec![to_addr],
        subject: "Jaunder SMTP test".to_owned(),
        body_text:
            "This is a test message from Jaunder. If you received it, SMTP is working correctly."
                .to_owned(),
    };

    mailer
        .send_email(&message)
        .await
        .map_err(|e| anyhow::anyhow!("Failed to send test email: {e}"))?;

    println!("Test email sent successfully to {to}");
    Ok(())
}

/// Performs a full backup of the application database and media.
///
/// # Errors
///
/// Returns an error if the backup process fails.
pub async fn cmd_backup(
    storage: &StorageArgs,
    mode: BackupMode,
    path: Option<PathBuf>,
) -> anyhow::Result<PathBuf> {
    let destination_path = path.unwrap_or_else(|| default_backup_path(storage, mode));
    let manifest = export_backup(BackupExportOptions {
        database: &storage.db,
        media_path: &storage.storage_path.join("media"),
        destination_path: &destination_path,
        mode,
    })
    .await?;

    println!(
        "Backup complete: path={} tables={}",
        destination_path.display(),
        manifest.tables.len()
    );
    Ok(destination_path)
}

/// Restores the application state from a backup.
///
/// # Errors
///
/// Returns an error if the backup does not exist, or if the target database or
/// media directory is not empty.
pub async fn cmd_restore(storage: &StorageArgs, path: &Path) -> anyhow::Result<()> {
    if !path.exists() {
        return Err(anyhow::anyhow!(
            "backup path does not exist: {}",
            path.display()
        ));
    }
    ensure_restore_target_empty(storage).await?;
    let manifest = restore_backup(BackupRestoreOptions {
        database: &storage.db,
        media_path: &storage.storage_path.join("media"),
        source_path: path,
    })
    .await?;

    println!(
        "Restore complete: path={} tables={}",
        path.display(),
        manifest.tables.len()
    );
    Ok(())
}

fn default_backup_path(storage: &StorageArgs, mode: BackupMode) -> PathBuf {
    let timestamp = chrono::Utc::now().format("%Y%m%dT%H%M%SZ");
    let name = match mode {
        BackupMode::Directory => format!("backup-{timestamp}"),
        BackupMode::Archive => format!("backup-{timestamp}.tar.gz"),
    };
    storage.storage_path.join("backups").join(name)
}

async fn ensure_restore_target_empty(storage: &StorageArgs) -> anyhow::Result<()> {
    if !storage::database_is_empty(&storage.db).await? {
        return Err(anyhow::anyhow!(
            "refusing to restore into a non-empty database"
        ));
    }
    let media_path = storage.storage_path.join("media");
    if directory_has_entries(&media_path)? {
        return Err(anyhow::anyhow!(
            "refusing to restore into a non-empty media directory"
        ));
    }
    Ok(())
}

fn directory_has_entries(path: &Path) -> io::Result<bool> {
    if !path.exists() {
        return Ok(false);
    }
    for entry in fs::read_dir(path)? {
        let entry = entry?;
        let metadata = entry.metadata()?;
        if metadata.is_dir() {
            if directory_has_entries(&entry.path())? {
                return Ok(true);
            }
        } else {
            return Ok(true);
        }
    }
    Ok(false)
}

/// A bound listener and router ready to serve, plus the live background-worker
/// schedulers that must outlive the serve loop. Produced by [`prepare_server`].
pub struct PreparedServer {
    /// The bound TCP listener.
    pub listener: tokio::net::TcpListener,
    /// The fully wired application router.
    pub router: axum::Router,
    // Held only to keep the workers running for the server's lifetime.
    backup_scheduler: Option<tokio_cron_scheduler::JobScheduler>,
    feed_scheduler: tokio_cron_scheduler::JobScheduler,
    /// Removes the runtime-info file on drop (see ADR-0035).
    runtime_guard: runtime_file::RuntimeFileGuard,
}

/// Performs all of [`cmd_serve`]'s setup — open the database (auto-initializing
/// in dev), start the backup and feed workers, build the router, and bind the
/// listener — returning it ready to serve.
///
/// Split out from [`cmd_serve`] so the whole setup path is covered by a
/// deterministic test: the blocking `axum::serve` loop can only be exercised by
/// an abort-racing test, whose async-region coverage is nondeterministic
/// (jaunder-uox1).
///
/// # Errors
///
/// Returns an error if the database cannot be opened/initialized, a worker fails
/// to start, or the listener cannot bind.
pub async fn prepare_server(
    storage: &StorageArgs,
    bind: SocketAddr,
    prod: bool,
    runtime_file: Option<std::path::PathBuf>,
) -> anyhow::Result<PreparedServer> {
    // Establish our own start-time up front (before opening the DB): if `/proc` is
    // unusable we cannot enforce the start-up mutex, so refuse rather than serve with
    // a silently-broken guard (#141). Threaded into the post-bind runtime-file write.
    let start_time = runtime_file::require_start_time_at(Path::new("/proc/self/stat"))?;

    // Start-up mutex: if the runtime file names a live writer process, refuse before
    // opening the DB / touching a data dir another instance owns (#141).
    let runtime_path = runtime_file::resolve_runtime_path(runtime_file, &storage.storage_path);
    match runtime_file::check_startup_mutex(&runtime_path)? {
        runtime_file::StartupCheck::Refuse { pid } => anyhow::bail!(
            "another jaunder instance is already running on data dir {} (pid {pid}); \
             refusing to start",
            storage.storage_path.display()
        ),
        runtime_file::StartupCheck::Stale | runtime_file::StartupCheck::Proceed => {}
    }

    let db = match open_existing_database(&storage.db).await {
        Ok(db) => db,
        Err(_) if !prod => {
            // Dev mode: auto-initialize on first `jaunder serve` so the host e2e
            // loop (and any dev run) works without a manual `jaunder init`.
            let storage_path = storage.storage_path.display();
            tracing::warn!(
                storage_path = %storage_path,
                db = %storage.db,
                "Database not found — auto-initializing (dev mode): storage={} db={}",
                storage_path,
                storage.db,
            );
            cmd_init(storage, true).await?;
            open_existing_database(&storage.db).await.map_err(|e| {
                // cov:ignore-start -- unreachable in practice: reopening cannot
                // fail immediately after `cmd_init` just created the database.
                anyhow::anyhow!("{e}; auto-init failed")
            })?
            // cov:ignore-stop
        }
        Err(e) => return Err(anyhow::anyhow!("{e}; run `jaunder init` first")),
    };

    let leptos_options = LeptosOptions::builder()
        .output_name("jaunder")
        .site_root("target/site")
        .site_pkg_dir("pkg")
        .env(if prod { Env::PROD } else { Env::DEV })
        .site_addr(bind)
        .build();

    let backup_scheduler = crate::backup::start_backup_worker(
        db.site_config.clone(),
        storage.db.clone(),
        storage.storage_path.clone(),
    )
    .await?;
    // The `WebSub` publisher is a service, not storage: it is constructed at the
    // composition root and injected into the feed worker (ADR-0016).
    let websub = crate::websub::default_client(capture::file(capture::Stream::WebSub));
    let feed_scheduler = crate::feed::worker::FeedWorker::new(
        db.site_config.clone(),
        db.posts.clone(),
        db.feed_cache.clone(),
        db.feed_events.clone(),
        websub,
    )
    .start()
    .await?;
    let mailer = crate::mailer::build_mailer(
        db.site_config.as_ref(),
        capture::file(capture::Stream::Mail),
    )
    .await;
    let router = crate::create_router(
        leptos_options,
        db,
        mailer,
        prod,
        storage.storage_path.clone(),
    );
    let listener = tokio::net::TcpListener::bind(bind).await?;
    // `local_addr` cannot fail on a just-bound listener; fall back to the
    // requested `bind` rather than add a never-taken error branch.
    let addr = listener.local_addr().unwrap_or(bind);
    // Reuse the path already resolved for the mutex check (no re-resolve / clone).
    let runtime_guard = runtime_file::RuntimeFileGuard::for_serve(
        Some(runtime_path),
        &storage.storage_path,
        addr,
        start_time,
    );

    Ok(PreparedServer {
        listener,
        router,
        backup_scheduler,
        feed_scheduler,
        runtime_guard,
    })
}

/// Serves `router` on `listener`, draining in-flight requests when `shutdown`
/// resolves, then returns. Owns `runtime_guard`, so a normal return drops it and
/// removes the runtime file — the covered removal path. The forced-exit path (see
/// [`spawn_shutdown_supervisor`]) removes the file explicitly instead.
///
/// # Errors
///
/// Returns an error if the server exits with an error.
async fn serve_with_shutdown(
    listener: tokio::net::TcpListener,
    router: axum::Router,
    // Held only for its `Drop`, which removes runtime.json when this function
    // returns (the graceful path). Underscore-named so it lives to scope end
    // rather than dropping immediately.
    _runtime_guard: runtime_file::RuntimeFileGuard,
    shutdown: impl std::future::Future<Output = ()> + Send + 'static,
) -> anyhow::Result<()> {
    axum::serve(listener, router)
        .with_graceful_shutdown(shutdown)
        .await?;
    Ok(())
    // `_runtime_guard` drops here → removes runtime.json on the graceful path.
}

/// Installs `SIGINT`/`SIGTERM` handlers and returns a receiver that fires when the
/// first arrives (the graceful-shutdown trigger). A second signal forces an
/// immediate exit, best-effort removing the runtime file first — necessary because
/// `process::exit` skips `Drop`. `runtime_path` is cloned from the guard before it
/// is moved into [`serve_with_shutdown`].
///
/// The streams are created synchronously (before returning), so a caller can rely
/// on the handlers being active the moment this returns.
///
/// # Errors
///
/// Returns an error if a signal handler cannot be installed.
#[cfg(unix)]
fn spawn_shutdown_supervisor(
    runtime_path: Option<std::path::PathBuf>,
) -> std::io::Result<tokio::sync::oneshot::Receiver<()>> {
    use tokio::signal::unix::{signal, SignalKind};
    let mut sigint = signal(SignalKind::interrupt())?;
    let mut sigterm = signal(SignalKind::terminate())?;
    let (tx, rx) = tokio::sync::oneshot::channel();
    tokio::spawn(async move {
        // cov:ignore-start -- async signal wait-loop; the forced branch ends in
        // process::exit and is unreachable by a survivable test. The synchronous
        // setup above and serve_with_shutdown are host-covered by the signal tests.
        let signal = tokio::select! {
            _ = sigint.recv() => "SIGINT",
            _ = sigterm.recv() => "SIGTERM",
        };
        tracing::info!(
            signal,
            "received shutdown signal; draining in-flight requests"
        );
        let _ = tx.send(());
        tokio::select! { _ = sigint.recv() => {}, _ = sigterm.recv() => {} }
        tracing::warn!("second shutdown signal; forcing immediate exit");
        if let Some(p) = &runtime_path {
            runtime_file::remove_runtime_file(p);
        }
        std::process::exit(0);
        // cov:ignore-stop
    });
    Ok(rx)
}

/// Starts the HTTP server and the background workers.
///
/// # Errors
///
/// Returns an error if setup fails (see [`prepare_server`]) or the server exits
/// with an error.
pub async fn cmd_serve(
    storage: &StorageArgs,
    bind: SocketAddr,
    prod: bool,
    runtime_file: Option<std::path::PathBuf>,
) -> anyhow::Result<()> {
    // Telemetry is owned by `run`, which holds the TelemetryGuard across this
    // call (see `server/src/main.rs`); `cmd_serve` does not init it, matching
    // every other `cmd_*`.
    let PreparedServer {
        listener,
        router,
        backup_scheduler,
        feed_scheduler,
        runtime_guard,
    } = prepare_server(storage, bind, prod, runtime_file).await?;

    tracing::info!(bind = %bind, prod, "starting HTTP server");
    // cov:ignore-start -- live serve glue: unreachable by host tests (the sole
    // cmd_serve test returns early at prepare_server). The covered pieces live in
    // serve_with_shutdown + spawn_shutdown_supervisor, exercised by the signal
    // tests; this only wires them to the prepared server. Mirrors jaunder-uox1.
    // Keep the worker schedulers alive for the lifetime of the serve loop.
    let _backup_scheduler = backup_scheduler;
    let _feed_scheduler = feed_scheduler;
    #[cfg(unix)]
    {
        // Clone the runtime-file path for the forced-exit removal before the guard
        // moves into serve_with_shutdown (whose Drop handles the graceful path).
        let runtime_path = runtime_guard.path().map(std::path::Path::to_path_buf);
        let shutdown_rx = spawn_shutdown_supervisor(runtime_path)?;
        serve_with_shutdown(listener, router, runtime_guard, async move {
            let _ = shutdown_rx.await;
        })
        .await
    }
    #[cfg(not(unix))]
    {
        // No signal handling off unix (jaunder targets Linux/NixOS): serve until
        // the process is otherwise terminated, matching prior behavior.
        serve_with_shutdown(
            listener,
            router,
            runtime_guard,
            std::future::pending::<()>(),
        )
        .await
    }
    // cov:ignore-stop
}

#[cfg(test)]
mod tests {
    use super::*;
    use storage::DbConnectOptions;
    use tempfile::TempDir;

    #[test]
    fn parse_password_none_is_ok_none() {
        assert!(parse_password(None).unwrap().is_none());
    }

    #[test]
    fn parse_password_validates_some() {
        assert!(parse_password(Some("password123".to_owned()))
            .unwrap()
            .is_some());
        let err = parse_password(Some("short".to_owned()))
            .unwrap_err()
            .to_string();
        assert!(err.contains("at least 8 characters"), "got: {err}");
    }

    #[test]
    fn describe_bootstrap_error_role_exists_message() {
        let msg =
            describe_bootstrap_error(storage::PgBootstrapError::RoleExists("alice".to_owned()))
                .to_string();
        assert!(msg.contains("application role 'alice' already exists"));
        assert!(msg.contains("refusing to modify existing role state"));
    }

    #[test]
    fn describe_bootstrap_error_database_exists_message() {
        let msg =
            describe_bootstrap_error(storage::PgBootstrapError::DatabaseExists("blog".to_owned()))
                .to_string();
        assert!(msg.contains("database 'blog' already exists"));
        assert!(msg.contains("refusing to modify existing database state"));
    }

    #[test]
    fn describe_bootstrap_error_sqlx_passes_through_source_message() {
        let expected = sqlx::Error::PoolClosed.to_string();
        let err =
            describe_bootstrap_error(storage::PgBootstrapError::Sqlx(sqlx::Error::PoolClosed));
        assert_eq!(err.to_string(), expected);
    }

    #[test]
    fn test_require_postgres_options() {
        let pg_url = "postgres://user:pass@localhost/db";
        let opts: DbConnectOptions = pg_url.parse().unwrap();
        assert!(require_postgres_options(&opts, "test").is_ok());

        let sqlite_url = "sqlite:test.db";
        let opts: DbConnectOptions = sqlite_url.parse().unwrap();
        let err = require_postgres_options(&opts, "test").unwrap_err();
        assert!(err.to_string().contains("test must be a PostgreSQL URL"));
    }

    #[tokio::test]
    async fn cmd_create_pg_db_rejects_non_postgres_app_db() {
        let err = cmd_create_pg_db(
            "postgres://bootstrap:secret@localhost/postgres",
            "sqlite:/tmp/jaunder.db",
            "secret",
        )
        .await
        .unwrap_err();

        assert!(err
            .to_string()
            .contains("--app-db must be a PostgreSQL URL"));
    }

    #[tokio::test]
    async fn cmd_create_pg_db_requires_database_name() {
        let err = cmd_create_pg_db(
            "postgres://bootstrap:secret@localhost/postgres",
            "postgres://app:secret@localhost",
            "secret",
        )
        .await
        .unwrap_err();

        assert!(err
            .to_string()
            .contains("--app-db must include a PostgreSQL database name"));
    }

    #[test]
    fn default_backup_path_is_under_storage_backups() {
        let storage = StorageArgs {
            storage_path: PathBuf::from("/tmp/jaunder"),
            db: "sqlite:/tmp/jaunder.db".parse().expect("sqlite db"),
        };

        let path = default_backup_path(&storage, BackupMode::Directory);

        assert!(path.starts_with("/tmp/jaunder/backups"));
    }

    #[test]
    fn default_archive_backup_path_ends_with_tar_gz() {
        let storage = StorageArgs {
            storage_path: PathBuf::from("/tmp/jaunder"),
            db: "sqlite:/tmp/jaunder.db".parse().expect("sqlite db"),
        };

        let path = default_backup_path(&storage, BackupMode::Archive);

        assert!(path.starts_with("/tmp/jaunder/backups"));
        assert!(path.to_string_lossy().ends_with(".tar.gz"));
    }

    #[test]
    fn directory_has_entries_handles_missing_empty_and_nested_paths() {
        let temp = tempfile::TempDir::new().expect("temp dir");
        assert!(!directory_has_entries(&temp.path().join("missing")).expect("missing"));

        let empty = temp.path().join("empty");
        std::fs::create_dir(&empty).expect("empty dir");
        assert!(!directory_has_entries(&empty).expect("empty"));

        let nested = temp.path().join("nested");
        std::fs::create_dir(&nested).expect("nested dir");
        std::fs::write(nested.join("file.txt"), "content").expect("nested file");
        assert!(directory_has_entries(temp.path()).expect("nested"));
    }

    #[tokio::test]
    async fn cmd_user_invite_creates_invite_expiring_in_the_future() {
        let temp = TempDir::new().expect("temp dir");
        let db_path = temp.path().join("jaunder.db");
        let db_url = format!("sqlite:{}", db_path.display());
        let opts: DbConnectOptions = db_url.parse().expect("parse sqlite url");

        let state = storage::open_database(&opts).await.expect("open db");

        let storage_args = StorageArgs {
            storage_path: temp.path().to_path_buf(),
            db: opts,
        };

        let before = chrono::Utc::now();
        cmd_user_invite(&storage_args, Some(24))
            .await
            .expect("create invite");

        let invites = state.invites.list_invites().await.expect("list invites");
        assert_eq!(invites.len(), 1, "exactly one invite must be created");
        assert!(
            invites[0].expires_at > before,
            "invite must expire in the future, got: {}",
            invites[0].expires_at
        );
    }

    #[tokio::test]
    async fn cmd_user_invite_with_base_url_configured_prints_link() {
        // Exercises the base-URL branch of the reveal: when a base URL is set, the
        // command prints a ready-to-send invitation link rather than the bare code.
        let temp = TempDir::new().expect("temp dir");
        let db_path = temp.path().join("jaunder.db");
        let db_url = format!("sqlite:{}", db_path.display());
        let opts: DbConnectOptions = db_url.parse().expect("parse sqlite url");

        let state = storage::open_database(&opts).await.expect("open db");
        state
            .site_config
            .set("site.base_url", "https://example.com")
            .await
            .expect("set base_url");

        let storage_args = StorageArgs {
            storage_path: temp.path().to_path_buf(),
            db: opts,
        };

        cmd_user_invite(&storage_args, Some(24))
            .await
            .expect("create invite");

        let invites = state.invites.list_invites().await.expect("list invites");
        assert_eq!(invites.len(), 1, "exactly one invite must be created");
    }

    #[tokio::test]
    async fn prepare_server_auto_initializes_in_dev_mode() {
        // A fresh storage dir with no database: `open_existing_database` fails,
        // and because `prod == false`, `prepare_server` takes the dev auto-init
        // branch (warn + `cmd_init` + reopen) instead of erroring. Binding to
        // port 0 avoids a fixed-port clash; we never enter the serve loop.
        let temp = TempDir::new().expect("temp dir");
        let db_path = temp.path().join("jaunder.db");
        let db_url = format!("sqlite:{}", db_path.display());
        let opts: DbConnectOptions = db_url.parse().expect("parse sqlite url");
        let storage = StorageArgs {
            storage_path: temp.path().to_path_buf(),
            db: opts,
        };
        assert!(
            !db_path.exists(),
            "database must not exist before prepare_server"
        );

        let bind: std::net::SocketAddr = "127.0.0.1:0".parse().expect("bind addr");
        let prepared = prepare_server(&storage, bind, false, None)
            .await
            .expect("dev-mode prepare_server must auto-initialize");

        assert!(db_path.exists(), "auto-init must have created the database");
        // Drop the prepared server (and its background workers) without serving.
        drop(prepared);
    }

    #[tokio::test]
    async fn prepare_server_refuses_on_live_holder_before_db_open() {
        // A planted runtime.json naming a live writer (our own pid + real
        // start-time) must make prepare_server refuse *before* opening/creating
        // the DB (#141). Uses dev mode (prod == false) so, absent the mutex, it
        // would auto-init — proving the refusal precedes that.
        let temp = TempDir::new().expect("temp dir");
        let db_path = temp.path().join("jaunder.db");
        let db_url = format!("sqlite:{}", db_path.display());
        let opts: DbConnectOptions = db_url.parse().expect("parse sqlite url");
        let storage = StorageArgs {
            storage_path: temp.path().to_path_buf(),
            db: opts,
        };
        let start = runtime_file::require_start_time_at(std::path::Path::new("/proc/self/stat"))
            .expect("read own start-time");
        std::fs::write(
            temp.path().join("runtime.json"),
            serde_json::json!({
                "ip": "127.0.0.1", "port": 1,
                "pid": std::process::id(), "start_time": start,
            })
            .to_string(),
        )
        .expect("plant runtime file");

        let bind: std::net::SocketAddr = "127.0.0.1:0".parse().expect("bind addr");
        // `.err()` discards the Ok(PreparedServer) (which isn't Debug) and keeps the
        // error, so the whole check is one covered assertion (no standalone panic line).
        let err = prepare_server(&storage, bind, false, None).await.err();
        assert!(
            err.is_some_and(|e| e.to_string().contains("already running")),
            "prepare_server must refuse when a live writer holds runtime.json"
        );
        assert!(
            !db_path.exists(),
            "must refuse before creating the database"
        );
    }

    // The two shutdown tests below raise a REAL signal to their own process. This
    // is safe only under `cargo nextest` (one process per test) — the tokio
    // handler, installed synchronously by spawn_shutdown_supervisor *before* we
    // raise, replaces the default terminate disposition so the signal is delivered
    // to the handler instead of killing us. Under a bare `cargo test` (libtest,
    // shared process) two such tests could observe each other's signals; the gate
    // runs nextest.
    #[cfg(unix)]
    async fn assert_signal_removes_runtime_file(signal: nix::sys::signal::Signal) {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("runtime.json");
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let guard = runtime_file::RuntimeFileGuard::write(path.clone(), addr, 0);
        assert!(path.exists(), "guard wrote the runtime file");

        // Installs the SIGINT/SIGTERM handlers synchronously, so the raise below
        // cannot beat handler installation.
        let shutdown_rx = spawn_shutdown_supervisor(Some(path.clone())).unwrap();
        let handle = tokio::spawn(serve_with_shutdown(
            listener,
            axum::Router::new(),
            guard,
            async move {
                let _ = shutdown_rx.await;
            },
        ));

        nix::sys::signal::raise(signal).unwrap();

        // Await serve completion so removal (guard Drop on return) is observed
        // deterministically, not by a timing poll.
        handle
            .await
            .unwrap()
            .expect("serve_with_shutdown returns Ok on graceful shutdown");
        assert!(!path.exists(), "runtime.json removed after {signal:?}");
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn sigterm_drains_and_removes_runtime_file() {
        assert_signal_removes_runtime_file(nix::sys::signal::Signal::SIGTERM).await;
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn sigint_drains_and_removes_runtime_file() {
        assert_signal_removes_runtime_file(nix::sys::signal::Signal::SIGINT).await;
    }
}
