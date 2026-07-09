use std::{
    fs, io,
    net::SocketAddr,
    path::{Path, PathBuf},
};

use sqlx::postgres::PgConnectOptions;

use crate::cli::StorageArgs;
use crate::mailer::LettreMailSender;
use common::backup::BackupMode;
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
    display_name: Option<&str>,
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
    println!("{code}");
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

    let to_addr: email_address::EmailAddress = to
        .parse()
        .map_err(|e| anyhow::anyhow!("Invalid email address '{to}': {e}"))?;

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
    if storage::database_has_users(&storage.db).await? {
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
    runtime_guard: crate::runtime_file::RuntimeFileGuard,
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
    let runtime_guard =
        crate::runtime_file::RuntimeFileGuard::for_serve(runtime_file, &storage.storage_path, addr);

    Ok(PreparedServer {
        listener,
        router,
        backup_scheduler,
        feed_scheduler,
        runtime_guard,
    })
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
    // Keep the worker schedulers alive for the lifetime of the serve loop.
    let _backup_scheduler = backup_scheduler;
    let _feed_scheduler = feed_scheduler;
    // Kept alive until the serve loop returns; removes runtime.json on drop.
    let _runtime_guard = runtime_guard;
    axum::serve(listener, router).await?;
    Ok(()) // cov:ignore
}

#[cfg(test)]
mod tests {
    use super::*;
    use storage::DbConnectOptions;
    use tempfile::TempDir;

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
}
