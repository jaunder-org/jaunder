use std::{
    fs, io,
    net::SocketAddr,
    path::{Path, PathBuf},
    sync::{Arc, RwLock},
    time::Duration,
};

use anyhow::Context;

use crate::cli::{AppTarget, BootstrapDb, Commands, SiteConfigAction, StorageArgs};
use crate::mailer::LettreMailSender;
use crate::runtime_file;
use common::backup::BackupMode;
use common::display_name::DisplayName;
use common::email::Email;
use common::invite::InviteTtlHours;
use common::mailer::{EmailMessage, MailSender};
use common::pg_role_password::PgRolePassword;
use common::session_label::SessionLabel;
use common::tagged_url::{self, MailConfirmUrl};
use common::token::RawToken;
use common::username::Username;
use host::config_key::SiteConfigKey;
use host::metrics;
use host::password::Password;
use host::smtp_config::SmtpConfig;
use storage::{
    BackupExportOptions, BackupRestoreOptions, BackupRestoreOutcome, RestoreValidationReport,
    StorageRuntimeConfig,
};

const INIT_FIRST_CONTEXT: &str = "database could not be opened; run `jaunder init` first";
const CAPTURE_FEED_INTERVAL: Duration = Duration::from_millis(250);
const PRODUCTION_FEED_INTERVAL: Duration = Duration::from_secs(10);

fn feed_worker_interval(capture_enabled: bool) -> Duration {
    if capture_enabled {
        CAPTURE_FEED_INTERVAL
    } else {
        PRODUCTION_FEED_INTERVAL
    }
}

fn inherited(name: &str) -> Result<Option<String>, std::env::VarError> {
    match std::env::var(name) {
        Ok(value) => Ok(Some(value)),
        Err(std::env::VarError::NotPresent) => Ok(None),
        Err(error) => Err(error),
    }
}

/// Resolves the application connection snapshot at the command boundary.
///
/// Bootstrap commands intentionally do not call this: their credentials are
/// explicit administrative inputs, never application password overrides.
/// `SQLite` has no `PostgreSQL` credential path, so it must not observe a broken
/// `PostgreSQL` password file or variable.
fn storage_runtime_config_from_raw(
    database: &storage::DbConnectOptions,
    sql_slow_ms: Result<Option<String>, std::env::VarError>,
    password_file: Result<Option<io::Result<String>>, std::env::VarError>,
    password: Result<Option<String>, std::env::VarError>,
) -> Result<StorageRuntimeConfig, storage::PostgresPasswordError> {
    match database {
        storage::DbConnectOptions::Sqlite(_) => {
            StorageRuntimeConfig::from_raw(sql_slow_ms, Ok(None), Ok(None))
        }
        storage::DbConnectOptions::Postgres { .. } => {
            StorageRuntimeConfig::from_raw(sql_slow_ms, password_file, password)
        }
    }
}

fn storage_runtime_config(
    database: &storage::DbConnectOptions,
) -> Result<StorageRuntimeConfig, storage::PostgresPasswordError> {
    let sql_slow_ms = inherited("JAUNDER_SQL_SLOW_MS");
    match database {
        storage::DbConnectOptions::Sqlite(_) => {
            storage_runtime_config_from_raw(database, sql_slow_ms, Ok(None), Ok(None))
        }
        storage::DbConnectOptions::Postgres { .. } => {
            let password_file = match std::env::var("JAUNDER_DB_PASSWORD_FILE") {
                Ok(path) => Ok(Some(fs::read_to_string(path))),
                Err(std::env::VarError::NotPresent) => Ok(None),
                Err(error) => Err(error),
            };
            storage_runtime_config_from_raw(
                database,
                sql_slow_ms,
                password_file,
                inherited("JAUNDER_DB_PASSWORD"),
            )
        }
    }
}

pub enum CommandOutput {
    None,
    Backup(PathBuf),
    Restore(BackupRestoreOutcome),
}

/// Capture leaf paths resolved by the serve composition root.
pub struct ServeCapturePaths {
    pub mail: PathBuf,
    pub websub: PathBuf,
}

impl Commands {
    /// Dispatch this parsed subcommand to its handler. A flat match-expression:
    /// each arm evaluates to the command's `Result<CommandOutput>`, so there is no `?` on
    /// the dispatch call and no trailing `Ok(())` — keeping any single function's
    /// cyclomatic complexity (and thus CRAP) low as subcommands are added (#147).
    ///
    /// # Errors
    ///
    /// Propagates the selected command's failure.
    pub async fn execute(
        self,
        telemetry: &host::telemetry::TelemetryConfig,
        capture: Option<ServeCapturePaths>,
    ) -> anyhow::Result<CommandOutput> {
        match self {
            Commands::Init {
                storage,
                skip_if_exists,
            } => cmd_init(&storage, skip_if_exists)
                .await
                .map(|()| CommandOutput::None),
            Commands::CreatePgDb { pg } => {
                cmd_create_pg_db(&pg.bootstrap_db, &pg.app_db, &pg.app_role_password)
                    .await
                    .map(|()| CommandOutput::None)
            }
            Commands::Serve {
                storage,
                bind,
                environment,
                runtime_file,
            } => cmd_serve(
                &storage,
                bind,
                environment.is_prod(),
                runtime_file,
                telemetry,
                capture.as_ref(),
            )
            .await
            .map(|()| CommandOutput::None),
            Commands::UserCreate {
                storage,
                username,
                password,
                display_name,
                operator,
            } => cmd_user_create(
                &storage,
                &username,
                password,
                display_name.as_ref(),
                operator,
            )
            .await
            .map(|()| CommandOutput::None),
            Commands::AppPasswordCreate {
                storage,
                username,
                label,
            } => cmd_app_password_create(&storage, &username, &label)
                .await
                .map(|()| CommandOutput::None),
            Commands::UserInvite {
                storage,
                expires_in,
            } => cmd_user_invite(&storage, expires_in)
                .await
                .map(|()| CommandOutput::None),
            Commands::SmtpTest { storage, to } => cmd_smtp_test(&storage, &to)
                .await
                .map(|()| CommandOutput::None),
            Commands::Backup {
                storage,
                mode,
                path,
            } => cmd_backup(&storage, mode.into(), path)
                .await
                .map(CommandOutput::Backup),
            Commands::Restore { storage, path } => cmd_restore(&storage, &path)
                .await
                .map(CommandOutput::Restore),
            // First nested subcommand group: the arm stays a thin delegation to
            // SiteConfigAction::execute (a sibling match), preserving the low-CRAP
            // one-arm-per-command dispatch shape. Copy this pattern for future groups.
            Commands::SiteConfig { action } => action.execute().await.map(|()| CommandOutput::None),
        }
    }
}

impl SiteConfigAction {
    /// Dispatch a `site-config` leaf to its handler (mirrors [`Commands::execute`]).
    ///
    /// # Errors
    ///
    /// Propagates the selected leaf's failure.
    pub async fn execute(self) -> anyhow::Result<()> {
        match self {
            SiteConfigAction::Set {
                storage,
                key,
                value,
            } => cmd_site_config_set(&storage, key, &value).await,
            SiteConfigAction::Get { storage, key } => cmd_site_config_get(&storage, key).await,
            SiteConfigAction::List { storage } => cmd_site_config_list(&storage).await,
            SiteConfigAction::Unset { storage, key } => cmd_site_config_unset(&storage, key).await,
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
    match storage::init_storage(&storage.storage_path) {
        Ok(()) => {}
        Err(e) if skip_if_exists && e.kind() == io::ErrorKind::AlreadyExists => {}
        Err(e) => return Err(e.into()),
    }
    let runtime = storage_runtime_config(&storage.db)?;
    storage::open_database(&storage.db, &runtime).await?;
    println!(
        "Initialized: storage={} db={}",
        storage.storage_path.display(),
        storage.db,
    );
    Ok(())
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
/// Every argument is already validated by the time it arrives: the CLI is the parse
/// boundary, so a non-`PostgreSQL` URL, a URL naming no database, and an empty password
/// are all rejected at argument parsing rather than here (#693).
///
/// # Errors
///
/// Returns an error if the bootstrap connection fails, or if the role or
/// database already exists.
pub async fn cmd_create_pg_db(
    bootstrap_db: &BootstrapDb,
    app_db: &AppTarget,
    app_role_password: &PgRolePassword,
) -> anyhow::Result<()> {
    let app_role = app_db.role();
    let database_name = app_db.database();

    storage::create_postgres_database_and_role(
        bootstrap_db.options(),
        app_role,
        app_role_password,
        database_name,
    )
    .await
    .map_err(describe_bootstrap_error)?;

    println!("PostgreSQL ready: role='{app_role}' database='{database_name}' owner='{app_role}'");
    Ok(())
}

async fn create_command_user(
    users: &dyn storage::UserStorage,
    username: &Username,
    password: &Password,
    display_name: Option<&DisplayName>,
    is_operator: bool,
) -> anyhow::Result<common::ids::UserId> {
    users
        .create_user(username, password, display_name, is_operator)
        .await
        .context("failed to create user")
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
    let runtime = storage_runtime_config(&storage.db)?;
    let state = storage::open_existing_database(&storage.db, &runtime)
        .await
        .context(INIT_FIRST_CONTEXT)?;

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

    let user_id = create_command_user(
        state.users(),
        username,
        &password,
        display_name,
        is_operator,
    )
    .await?;

    // CLI user creation bypasses the site registration policy entirely.
    metrics::registration(
        host::metrics::RegistrationSource::Cli,
        host::metrics::RegistrationPolicy::CliBypass,
        host::metrics::RegistrationResult::Ok,
    );

    println!("Created user '{username}' with id {}", i64::from(user_id));
    Ok(())
}

async fn app_password_create_with(
    users: &dyn storage::UserStorage,
    sessions: &dyn storage::SessionStorage,
    username: &Username,
    label: &SessionLabel,
) -> anyhow::Result<RawToken> {
    let user = users
        .get_user_by_username(username)
        .await
        .context("failed to look up user")?
        .ok_or_else(|| anyhow::anyhow!("no such user '{username}'"))?;
    // No validation here: the signature carries it. `SessionLabel` cannot be built from
    // an invalid string, so there is nothing left to check and no step to remember.
    sessions
        .create_session(user.user_id, label)
        .await
        .context("failed to create app password")
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
    label: &SessionLabel,
) -> anyhow::Result<RawToken> {
    app_password_create_with(state.users(), state.sessions(), username, label).await
}

/// CLI wrapper: opens the database, mints an app password, prints it to stdout.
///
/// # Errors
///
/// Returns an error if the database cannot be opened or minting fails.
pub async fn cmd_app_password_create(
    storage: &StorageArgs,
    username: &Username,
    label: &SessionLabel,
) -> anyhow::Result<()> {
    let runtime = storage_runtime_config(&storage.db)?;
    let state = storage::open_existing_database(&storage.db, &runtime)
        .await
        .context(INIT_FIRST_CONTEXT)?;
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
pub async fn cmd_user_invite(
    storage: &StorageArgs,
    expires_in: Option<InviteTtlHours>,
) -> anyhow::Result<()> {
    let runtime = storage_runtime_config(&storage.db)?;
    let state = storage::open_existing_database(&storage.db, &runtime)
        .await
        .context(INIT_FIRST_CONTEXT)?;

    // The 1..=336 bound lives in `InviteTtlHours` (clap rejects an out-of-range `--expires-in`
    // at parse), so no in-body overflow check is needed.
    let expires_at = common::time::UtcInstant::from(
        chrono::Utc::now() + chrono::Duration::hours(expires_in.unwrap_or_default().value()),
    );

    let code = state.invites().create_invite(expires_at).await?;
    metrics::invite(host::metrics::InviteEvent::Created);
    // Deliberate operator-facing reveal via `AsRef` (InviteCode has no Display/serde). With a
    // configured base URL, print a ready-to-send invitation link; otherwise the bare code.
    match state.site_config().get_identity().await?.base_url {
        Some(base_url) => {
            let register_url: MailConfirmUrl = tagged_url::compose(&base_url, "/register");
            println!("{register_url}?invite_code={}", code.as_ref());
        }
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
pub async fn cmd_smtp_test(storage: &StorageArgs, to: &Email) -> anyhow::Result<()> {
    let runtime = storage_runtime_config(&storage.db)?;
    let state = storage::open_existing_database(&storage.db, &runtime)
        .await
        .context(INIT_FIRST_CONTEXT)?;

    smtp_test_with(state.site_config(), to, |config| {
        Ok(Box::new(LettreMailSender::from_config(config)?) as Box<dyn MailSender>)
    })
    .await
}

async fn smtp_test_with(
    site_config: &dyn storage::SiteConfigStorage,
    to: &Email,
    build_smtp: impl FnOnce(&SmtpConfig) -> Result<Box<dyn MailSender>, crate::mailer::BuildMailerError>,
) -> anyhow::Result<()> {
    let smtp_config = storage::load_smtp_config(site_config)
        .await
        .context("SMTP is misconfigured")?
        .ok_or_else(|| anyhow::anyhow!("SMTP is not configured"))?;

    let mailer = build_smtp(&smtp_config).context("failed to build SMTP transport")?;

    let message = EmailMessage {
        from: None,
        to: vec![to.clone()],
        subject: "Jaunder SMTP test".to_owned(),
        body_text:
            "This is a test message from Jaunder. If you received it, SMTP is working correctly."
                .to_owned(),
    };

    mailer
        .send_email(&message)
        .await
        .context("failed to send test email")?;

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
    let runtime = storage_runtime_config(&storage.db)?;
    let destination_path = path.unwrap_or_else(|| default_backup_path(storage, mode));
    let manifest = storage::export_backup(BackupExportOptions {
        database: &storage.db,
        runtime: &runtime,
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
pub async fn cmd_restore(
    storage: &StorageArgs,
    path: &Path,
) -> anyhow::Result<BackupRestoreOutcome> {
    if !path.exists() {
        return Err(anyhow::anyhow!(
            "backup path does not exist: {}",
            path.display()
        ));
    }
    let runtime = storage_runtime_config(&storage.db)?;
    ensure_restore_target_empty(storage, &runtime).await?;
    let outcome = storage::restore_backup(BackupRestoreOptions {
        database: &storage.db,
        runtime: &runtime,
        media_path: &storage.storage_path.join("media"),
        source_path: path,
    })
    .await?;
    println!(
        "Restore complete: path={} tables={}",
        path.display(),
        outcome.manifest.tables.len()
    );
    print_restore_validation_report(&outcome.validation_report);
    Ok(outcome)
}

fn print_restore_validation_report(report: &RestoreValidationReport) {
    if report.is_empty() {
        return;
    }

    println!(
        "Restore validation issues: count={} (data restored; repair may be needed before normal reads)",
        report.len()
    );
    for issue in report.issues() {
        println!("- {issue}");
    }
}

fn default_backup_path(storage: &StorageArgs, mode: BackupMode) -> PathBuf {
    let timestamp = chrono::Utc::now().format("%Y%m%dT%H%M%SZ");
    let name = match mode {
        BackupMode::Directory => format!("backup-{timestamp}"),
        BackupMode::Archive => format!("backup-{timestamp}.tar.gz"),
    };
    storage.storage_path.join("backups").join(name)
}

async fn ensure_restore_target_empty(
    storage: &StorageArgs,
    runtime: &StorageRuntimeConfig,
) -> anyhow::Result<()> {
    if !storage::database_is_empty(&storage.db, runtime).await? {
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

#[async_trait::async_trait]
trait StartupDatabaseOperations: Sync {
    async fn open_existing(
        &self,
        options: &storage::DbConnectOptions,
        runtime: &StorageRuntimeConfig,
    ) -> sqlx::Result<StartupDatabase>;

    async fn init(
        &self,
        storage: &StorageArgs,
        runtime: &StorageRuntimeConfig,
    ) -> anyhow::Result<()>;

    fn metadata(&self, path: &Path) -> io::Result<fs::Metadata>;
}

struct RealStartupDatabaseOperations;

struct StartupDatabase {
    state: Arc<storage::AppState>,
    instance_id: storage::InstanceId,
    pool_observer: storage::DbPoolObserver,
}

#[async_trait::async_trait]
impl StartupDatabaseOperations for RealStartupDatabaseOperations {
    async fn open_existing(
        &self,
        options: &storage::DbConnectOptions,
        runtime: &StorageRuntimeConfig,
    ) -> sqlx::Result<StartupDatabase> {
        let opened = storage::open_existing_database_with_observer(options, runtime).await?;
        Ok(StartupDatabase {
            state: opened.state,
            instance_id: opened.instance_id,
            pool_observer: opened.pool_observer,
        })
    }

    async fn init(
        &self,
        storage: &StorageArgs,
        runtime: &StorageRuntimeConfig,
    ) -> anyhow::Result<()> {
        match storage::init_storage(&storage.storage_path) {
            Ok(()) => {}
            Err(error) if error.kind() == io::ErrorKind::AlreadyExists => {}
            Err(error) => return Err(error.into()),
        }
        storage::open_database(&storage.db, runtime).await?;
        Ok(())
    }

    fn metadata(&self, path: &Path) -> io::Result<fs::Metadata> {
        fs::metadata(path)
    }
}

fn database_error_code(error: &sqlx::Error, expected: &str) -> bool {
    matches!(
        error,
        sqlx::Error::Database(database)
            if database.code().as_deref() == Some(expected)
    )
}

fn is_sqlite_cantopen(error: &sqlx::Error) -> bool {
    database_error_code(error, "14")
}

fn classify_development_auto_init(
    database: &storage::DbConnectOptions,
    open_error: &sqlx::Error,
    sqlite_filename_metadata: Option<io::Result<fs::Metadata>>,
) -> io::Result<bool> {
    if !matches!(database, storage::DbConnectOptions::Sqlite(_)) || !is_sqlite_cantopen(open_error)
    {
        return Ok(false);
    }

    match sqlite_filename_metadata {
        Some(Err(error)) if error.kind() == io::ErrorKind::NotFound => Ok(true),
        Some(Err(error)) => Err(error),
        Some(Ok(_)) | None => Ok(false),
    }
}

fn startup_database_error_context(
    database: &storage::DbConnectOptions,
    error: &sqlx::Error,
) -> &'static str {
    if matches!(database, storage::DbConnectOptions::Postgres { .. })
        && database_error_code(error, "3D000")
    {
        "PostgreSQL database does not exist; run `jaunder create-pg-db` first"
    } else {
        INIT_FIRST_CONTEXT
    }
}

async fn open_server_database_with(
    storage: &StorageArgs,
    runtime: &StorageRuntimeConfig,
    prod: bool,
    operations: &impl StartupDatabaseOperations,
) -> anyhow::Result<StartupDatabase> {
    let open_error = match operations.open_existing(&storage.db, runtime).await {
        Ok(database) => return Ok(database),
        Err(error) => error,
    };

    if prod {
        let context = startup_database_error_context(&storage.db, &open_error);
        return Err(open_error).context(context);
    }

    let metadata = match &storage.db {
        storage::DbConnectOptions::Sqlite(options) if is_sqlite_cantopen(&open_error) => {
            Some(operations.metadata(options.get_filename()))
        }
        storage::DbConnectOptions::Sqlite(_) | storage::DbConnectOptions::Postgres { .. } => None,
    };
    let initialize = classify_development_auto_init(&storage.db, &open_error, metadata)
        .context("failed to inspect SQLite database filename for development auto-init")?;
    if !initialize {
        let context = startup_database_error_context(&storage.db, &open_error);
        return Err(open_error).context(context);
    }

    let storage_path = storage.storage_path.display();
    tracing::warn!(
        storage_path = %storage_path,
        db = %storage.db,
        "Database not found — auto-initializing (dev mode): storage={} db={}",
        storage_path,
        storage.db,
    );
    operations.init(storage, runtime).await?;
    operations
        .open_existing(&storage.db, runtime)
        .await
        .context("auto-init failed while reopening database")
}

async fn open_server_database(
    storage: &StorageArgs,
    runtime: &StorageRuntimeConfig,
    prod: bool,
) -> anyhow::Result<StartupDatabase> {
    open_server_database_with(storage, runtime, prod, &RealStartupDatabaseOperations).await
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
    pub saturation_metrics: Option<PreparedSaturationMetrics>,
}

pub struct PreparedSaturationMetrics {
    _observables: host::metrics::SaturationObservableGuard,
    sampler: tokio::task::JoinHandle<()>,
}

impl Drop for PreparedSaturationMetrics {
    fn drop(&mut self) {
        self.sampler.abort();
    }
}

async fn prepare_saturation_metrics(
    db: Arc<storage::AppState>,
    pool_observer: storage::DbPoolObserver,
    media_root: PathBuf,
    telemetry: &host::telemetry::TelemetryConfig,
) -> anyhow::Result<Option<PreparedSaturationMetrics>> {
    if !telemetry.otlp_endpoint_configured() {
        return Ok(None);
    }
    let backup_config = db
        .site_config
        .get_backup_config()
        .await
        .context("failed to load backup configuration for saturation metrics")?;
    let backup_destination_root = backup_config.destination_path.as_deref().map(PathBuf::from);
    let snapshot = Arc::new(RwLock::new(host::metrics::SaturationSnapshot::default()));
    let observables = metrics::register_saturation_observables(snapshot.clone());
    let sources = crate::metrics::SaturationSources::real(
        db.feed_events.clone(),
        db.media.clone(),
        media_root,
        backup_destination_root,
        pool_observer,
    );
    let sampler = crate::metrics::spawn_saturation_sampler(sources, snapshot);

    Ok(Some(PreparedSaturationMetrics {
        _observables: observables,
        sampler,
    }))
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
    telemetry: &host::telemetry::TelemetryConfig,
    capture: Option<&ServeCapturePaths>,
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
    let runtime = storage_runtime_config(&storage.db)?;
    let StartupDatabase {
        state: db,
        instance_id,
        pool_observer,
    } = open_server_database(storage, &runtime, prod).await?;

    let saturation_metrics = prepare_saturation_metrics(
        db.clone(),
        pool_observer,
        storage.storage_path.join("media"),
        telemetry,
    )
    .await?;
    let backup_scheduler = crate::backup::start_backup_worker(
        db.site_config.clone(),
        storage.db.clone(),
        runtime,
        storage.storage_path.clone(),
    )
    .await?;
    // The `WebSub` publisher is a service, not storage: it is constructed at the
    // composition root and injected into the feed worker (ADR-0016). Capture mode
    // also selects the shorter e2e cadence without changing the production policy.
    let websub_capture = capture.map(|paths| paths.websub.clone());
    let feed_interval = feed_worker_interval(websub_capture.is_some());
    let websub = crate::websub::default_client(websub_capture);
    let feed_scheduler = crate::feed::worker::FeedWorker::new(
        db.site_config.clone(),
        db.posts.clone(),
        db.feed_cache.clone(),
        db.feed_events.clone(),
        websub,
    )
    .start(feed_interval)
    .await?;
    let mailer =
        crate::mailer::build_mailer(db.site_config(), capture.map(|paths| paths.mail.clone()))
            .await?;
    let router = crate::create_router(db, instance_id, mailer, prod, storage.storage_path.clone())?;
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
        saturation_metrics,
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
    use tokio::signal::unix::{SignalKind, signal};
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
    telemetry: &host::telemetry::TelemetryConfig,
    capture: Option<&ServeCapturePaths>,
) -> anyhow::Result<()> {
    // Telemetry is owned by `run`, which holds the TelemetryGuard across this
    // call (see `server/src/main.rs`); `cmd_serve` does not init it, matching
    // every other `cmd_*`.
    // cov:ignore-start -- live serve glue: unreachable by host tests (the sole
    // cmd_serve test returns early at prepare_server). The covered pieces live in
    // serve_with_shutdown + spawn_shutdown_supervisor, exercised by the signal
    // tests; this only wires them to the prepared server. Mirrors jaunder-uox1.
    //
    // The marker starts at the destructuring, not below it: completing this binding
    // *is* the prepare_server-succeeded path, so the same rationale covers it (#693).
    let PreparedServer {
        listener,
        router,
        backup_scheduler,
        feed_scheduler,
        runtime_guard,
        saturation_metrics,
    } = prepare_server(storage, bind, prod, runtime_file, telemetry, capture).await?;

    tracing::info!(bind = %bind, prod, "starting HTTP server");
    // Keep the worker schedulers alive for the lifetime of the serve loop.
    let _backup_scheduler = backup_scheduler;
    let _feed_scheduler = feed_scheduler;
    let _saturation_metrics = saturation_metrics;
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
        // the process is otherwise terminated.
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

/// Upsert a `site_config` key/value through the real storage path.
///
/// The value is checked against the key's own validator *before* the database is
/// opened, so a rejected value never reaches a row.
async fn cmd_site_config_set(
    storage: &StorageArgs,
    key: SiteConfigKey,
    value: &str,
) -> anyhow::Result<()> {
    key.validate(value)?;
    let runtime = storage_runtime_config(&storage.db)?;
    let state = storage::open_existing_database(&storage.db, &runtime).await?;
    state.site_config.set(key, value).await?;
    eprintln!("set site_config {key} = {value}");
    Ok(())
}

/// Print the value for `key` to stdout; error (→ non-zero exit) if it is unset,
/// so a caller can distinguish an unset key from an empty value.
async fn cmd_site_config_get(storage: &StorageArgs, key: SiteConfigKey) -> anyhow::Result<()> {
    let runtime = storage_runtime_config(&storage.db)?;
    let state = storage::open_existing_database(&storage.db, &runtime).await?;
    match state.site_config.get_raw(key).await? {
        Some(value) => {
            println!("{value}");
            Ok(())
        }
        None => Err(anyhow::anyhow!("no site_config value for key {key:?}")),
    }
}

/// Print all `site_config` entries as `key=value`, one per line, ordered by key.
async fn cmd_site_config_list(storage: &StorageArgs) -> anyhow::Result<()> {
    let runtime = storage_runtime_config(&storage.db)?;
    let state = storage::open_existing_database(&storage.db, &runtime).await?;
    let entries = state.site_config.list().await?;
    print!("{}", format_entries(&entries));
    Ok(())
}

/// Delete a `site_config` key. Idempotent (exit 0 whether or not a row existed);
/// stderr notes which happened.
async fn cmd_site_config_unset(storage: &StorageArgs, key: SiteConfigKey) -> anyhow::Result<()> {
    let runtime = storage_runtime_config(&storage.db)?;
    let state = storage::open_existing_database(&storage.db, &runtime).await?;
    if state.site_config.delete(key).await? {
        eprintln!("unset site_config {key}");
    } else {
        eprintln!("site_config {key} was not set (no-op)");
    }
    Ok(())
}

/// Render `site_config` entries as `key=value\n` lines (a human/discovery view;
/// `site-config get` is the lossless scriptable accessor). Pure, unit-tested directly.
///
/// Every stored row is printed — this is a faithful dump of what is physically
/// stored — but rows the registry judges are annotated (spec D4):
///
/// - a key outside [`SiteConfigKey`] is marked `UNKNOWN KEY` (a legacy or
///   hand-written row the typed seam can no longer read or write);
/// - a known key whose value fails its validator is marked `INVALID (<reason>)`.
///
/// An empty value on an optional key is *not* invalid: empty means unset (spec
/// D1b), which `SiteConfigKey::validate` already honours.
fn format_entries(entries: &[(String, String)]) -> String {
    use std::fmt::Write;
    entries.iter().fold(String::new(), |mut out, (k, v)| {
        // writeln! to a String is infallible; the newline gives one entry per line.
        let _ = match k.parse::<SiteConfigKey>() {
            Err(_) => writeln!(out, "{:<40}  UNKNOWN KEY", format!("{k}={v}")),
            Ok(key) => match key.validate(v) {
                Ok(()) => writeln!(out, "{k}={v}"),
                Err(err) => writeln!(out, "{:<40}  INVALID ({err})", format!("{k}={v}")),
            },
        };
        out
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use common::smtp_tls_mode::SmtpTlsMode;
    use common::test_support::{
        parse_email, parse_invite_ttl_hours, parse_session_label, parse_username,
    };
    use rstest::*;
    use rstest_reuse::*;
    #[cfg(unix)]
    use std::os::unix::ffi::OsStringExt as _;
    use storage::{
        DbConnectOptions,
        test_support::{
            Backend, PostgresDbGuard, TestEnv, backends, sqlite_url, unique_postgres_url,
        },
    };
    use tempfile::TempDir;

    fn test_telemetry(otlp_endpoint: Option<&str>) -> host::telemetry::TelemetryConfig {
        host::telemetry::TelemetryConfig::from_raw(
            false,
            host::telemetry::TelemetryRawConfig {
                log_filter: Ok(None),
                rust_log: Ok(None),
                log_format: Ok(None),
                jaunder_otlp_endpoint: Ok(otlp_endpoint.map(str::to_owned)),
                otlp_endpoint: Ok(None),
                slow_op_ms: Ok(None),
                e2e_seed_process: Ok(None),
            },
        )
    }

    #[test]
    fn subprocess_classifies_command_configuration_inputs() {
        const SCENARIO: &str = "JAUNDER_TEST_COMMAND_CONFIG_SCENARIO";
        if let Some(scenario) = std::env::var_os(SCENARIO) {
            let database: DbConnectOptions = "postgres://app@localhost/jaunder"
                .parse()
                .expect("PostgreSQL URL");
            let result = storage_runtime_config(&database);
            match scenario.to_string_lossy().as_ref() {
                "file" | "password" | "invalid-threshold" => {
                    result.expect("valid command configuration");
                }
                "invalid-file-variable" => {
                    assert!(matches!(
                        result,
                        Err(storage::PostgresPasswordError::FileVariable(_))
                    ));
                }
                _ => unreachable!("parent supplies a closed configuration scenario set"),
            }
            return;
        }

        let dir = TempDir::new().expect("password directory");
        let password_file = dir.path().join("password");
        std::fs::write(&password_file, "from-file\n").expect("password fixture");
        for scenario in [
            "file",
            "password",
            "invalid-threshold",
            "invalid-file-variable",
        ] {
            let mut command =
                std::process::Command::new(std::env::current_exe().expect("test executable"));
            command.args([
                "--exact",
                "commands::tests::subprocess_classifies_command_configuration_inputs",
                "--nocapture",
            ]);
            command.env(SCENARIO, scenario);
            for name in [
                "JAUNDER_SQL_SLOW_MS",
                "JAUNDER_DB_PASSWORD_FILE",
                "JAUNDER_DB_PASSWORD",
            ] {
                command.env_remove(name);
            }
            match scenario {
                "file" => {
                    command.env("JAUNDER_DB_PASSWORD_FILE", &password_file);
                }
                "password" => {
                    command.env("JAUNDER_DB_PASSWORD", "from-variable");
                }
                "invalid-threshold" => {
                    command.env(
                        "JAUNDER_SQL_SLOW_MS",
                        std::ffi::OsString::from_vec(vec![0xff]),
                    );
                }
                "invalid-file-variable" => {
                    command.env(
                        "JAUNDER_DB_PASSWORD_FILE",
                        std::ffi::OsString::from_vec(vec![0xff]),
                    );
                }
                _ => unreachable!("closed parent scenario set"),
            }
            assert!(
                command
                    .status()
                    .expect("spawn configuration child")
                    .success(),
                "configuration child scenario {scenario} must succeed"
            );
        }
    }

    #[test]
    fn sqlite_runtime_config_ignores_broken_postgres_credential_inputs() {
        let database: DbConnectOptions = "sqlite:/tmp/jaunder.db".parse().expect("SQLite URL");
        let runtime = storage_runtime_config_from_raw(
            &database,
            Ok(None),
            Err(std::env::VarError::NotUnicode(std::ffi::OsString::from(
                "invalid-password-file-variable",
            ))),
            Err(std::env::VarError::NotUnicode(std::ffi::OsString::from(
                "invalid-password-variable",
            ))),
        )
        .expect("SQLite does not resolve PostgreSQL credentials");

        assert_eq!(
            runtime.sql_slow_query_threshold(),
            Duration::from_secs(5),
            "SQLite retains the shared threshold default"
        );
    }

    #[test]
    fn feed_worker_interval_is_250_ms_for_capture() {
        assert_eq!(feed_worker_interval(true), Duration::from_millis(250));
    }

    #[test]
    fn feed_worker_interval_is_10_seconds_without_capture() {
        assert_eq!(feed_worker_interval(false), Duration::from_secs(10));
    }

    /// A `StorageArgs` for `backend` whose database already exists, since the
    /// `site-config` handlers all go through `open_existing_database`.
    async fn site_config_args(
        backend: Backend,
        base: &TempDir,
    ) -> (StorageArgs, Option<PostgresDbGuard>) {
        let (db, guard) = match backend {
            Backend::Sqlite => (sqlite_url(base), None),
            Backend::Postgres => {
                let config = storage::test_support::PostgresTestConfig::from_env();
                let (db, guard) = unique_postgres_url(&config).await;
                (db, Some(guard))
            }
        };
        storage::open_database(&db, &StorageRuntimeConfig::default())
            .await
            .expect("open db");
        (
            StorageArgs {
                storage_path: base.path().to_path_buf(),
                db,
            },
            guard,
        )
    }

    fn sqlite_storage_args(temp: &TempDir) -> StorageArgs {
        StorageArgs {
            storage_path: temp.path().to_path_buf(),
            db: crate::test_support::sqlite_db_options(temp.path()),
        }
    }

    fn typed_crypto_storage_error() -> sqlx::Error {
        let password = host::test_support::parse_password("password123");
        let error = host::password::verify(
            &password,
            "$argon2id$v=1$m=65536,t=2,p=1$c29tZXNhbHQ$AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA",
        )
        .unwrap_err();
        sqlx::Error::Io(io::Error::other(error))
    }

    fn assert_typed_account_command_source(error: &anyhow::Error, context: &str) {
        assert_eq!(error.to_string(), context);
        let source = error
            .chain()
            .find_map(|source| source.downcast_ref::<argon2::password_hash::Error>());
        assert_eq!(source, Some(&argon2::password_hash::Error::Version));
        assert!(
            format!("{error:#}").contains("verification failed"),
            "human error chain must include the crypto failure: {error:#}"
        );
    }

    fn smtp_config() -> SmtpConfig {
        SmtpConfig {
            host: "mail.example.com".parse().expect("valid host"),
            port: common::smtp_port::SmtpPort::default(),
            tls_mode: SmtpTlsMode::StartTls,
            username: None,
            password: None,
            sender: "Jaunder <noreply@example.com>"
                .parse()
                .expect("valid sender"),
        }
    }

    fn transport_build_error() -> lettre::transport::smtp::Error {
        lettre::transport::smtp::client::TlsParametersBuilder::new("mail.example.com".to_owned())
            .set_min_tls_version(lettre::transport::smtp::client::TlsVersion::Tlsv10)
            .build_rustls()
            .err()
            .expect("rustls rejects TLS 1.0")
    }

    fn assert_command_source<T: std::error::Error + 'static>(error: &anyhow::Error, context: &str) {
        assert_eq!(error.to_string(), context);
        assert!(
            error
                .chain()
                .any(|source| source.downcast_ref::<T>().is_some()),
            "typed source must remain downcastable: {error:#}"
        );
    }

    struct FailingMailSender;

    #[async_trait::async_trait]
    impl MailSender for FailingMailSender {
        async fn send_email(
            &self,
            _message: &EmailMessage,
        ) -> Result<(), common::mailer::MailError> {
            Err(common::mailer::MailError::Send(Box::new(
                transport_build_error(),
            )))
        }
    }

    #[tokio::test]
    async fn command_source_chain_smtp_config_read() {
        let mut store = storage::MockSiteConfigStorage::new();
        store.expect_get_smtp_config().return_once(|| {
            Err(sqlx::Error::Io(io::Error::other(
                "injected SMTP config read failure",
            )))
        });

        let error = smtp_test_with(&store, &parse_email("to@example.com"), |_| unreachable!())
            .await
            .unwrap_err();

        assert_command_source::<sqlx::Error>(&error, "SMTP is misconfigured");
    }

    #[tokio::test]
    async fn command_source_chain_smtp_invalid_sender() {
        let mut store = storage::MockSiteConfigStorage::new();
        store
            .expect_get_smtp_config()
            .return_once(|| Ok(Some(smtp_config())));

        let error = smtp_test_with(&store, &parse_email("to@example.com"), |_| {
            let source = "not a mailbox"
                .parse::<lettre::message::Mailbox>()
                .expect_err("invalid mailbox yields lettre address error");
            Err(crate::mailer::BuildMailerError::InvalidSender(source))
        })
        .await
        .unwrap_err();

        assert_command_source::<lettre::address::AddressError>(
            &error,
            "failed to build SMTP transport",
        );
    }

    #[tokio::test]
    async fn command_source_chain_smtp_transport_build() {
        let mut store = storage::MockSiteConfigStorage::new();
        store
            .expect_get_smtp_config()
            .return_once(|| Ok(Some(smtp_config())));

        let error = smtp_test_with(&store, &parse_email("to@example.com"), |_| {
            Err(crate::mailer::BuildMailerError::Transport(
                transport_build_error(),
            ))
        })
        .await
        .unwrap_err();

        assert_command_source::<lettre::transport::smtp::Error>(
            &error,
            "failed to build SMTP transport",
        );
    }

    #[tokio::test]
    async fn command_source_chain_smtp_send() {
        let mut store = storage::MockSiteConfigStorage::new();
        store
            .expect_get_smtp_config()
            .return_once(|| Ok(Some(smtp_config())));

        let error = smtp_test_with(&store, &parse_email("to@example.com"), |_| {
            Ok(Box::new(FailingMailSender) as Box<dyn MailSender>)
        })
        .await
        .unwrap_err();

        assert_command_source::<lettre::transport::smtp::Error>(
            &error,
            "failed to send test email",
        );
    }

    async fn missing_sqlite_open_error(database: &DbConnectOptions) -> sqlx::Error {
        storage::open_existing_database(database, &StorageRuntimeConfig::default())
            .await
            .err()
            .expect("missing SQLite filename must fail")
    }

    #[tokio::test]
    async fn auto_init_classification_is_sqlite_cantopen_and_not_found_only() {
        let temp = TempDir::new().expect("temp dir");
        let filename = temp.path().join("missing.db");
        let database: DbConnectOptions = format!("sqlite:{}", filename.display())
            .parse()
            .expect("SQLite options");
        let cantopen = missing_sqlite_open_error(&database).await;
        assert!(is_sqlite_cantopen(&cantopen), "fixture must be CANTOPEN");

        assert!(
            classify_development_auto_init(&database, &cantopen, Some(fs::metadata(&filename)),)
                .expect("NotFound is classified"),
            "CANTOPEN plus a missing filename requests auto-init"
        );

        fs::write(&filename, []).expect("create existing filename");
        assert!(
            !classify_development_auto_init(&database, &cantopen, Some(fs::metadata(&filename)),)
                .expect("existing metadata is classified"),
            "an existing SQLite filename must propagate CANTOPEN"
        );

        let metadata_error = classify_development_auto_init(
            &database,
            &cantopen,
            Some(Err(io::Error::from(io::ErrorKind::PermissionDenied))),
        )
        .expect_err("metadata failures must propagate");
        assert_eq!(metadata_error.kind(), io::ErrorKind::PermissionDenied);

        assert!(
            !classify_development_auto_init(&database, &sqlx::Error::PoolClosed, None)
                .expect("other SQLite errors are classified"),
            "non-CANTOPEN SQLite failures must propagate"
        );

        let malformed = sqlx::Error::Configuration(Box::new(io::Error::other("malformed URL")));
        assert!(
            !classify_development_auto_init(&database, &malformed, None)
                .expect("configuration errors are classified"),
            "malformed database URLs must propagate"
        );

        let migration =
            sqlx::Error::Migrate(Box::new(sqlx::migrate::MigrateError::VersionMissing(1)));
        assert!(
            !classify_development_auto_init(&database, &migration, None)
                .expect("migration errors are classified"),
            "migration failures must propagate"
        );

        let postgres: DbConnectOptions = "postgres://user@localhost/database"
            .parse()
            .expect("PostgreSQL options");
        assert!(
            !classify_development_auto_init(
                &postgres,
                &sqlx::Error::PoolTimedOut,
                Some(Err(io::Error::from(io::ErrorKind::NotFound))),
            )
            .expect("PostgreSQL errors are classified"),
            "PostgreSQL never auto-initializes"
        );
    }

    struct FailingStartupDatabaseOperations {
        errors: std::sync::Mutex<std::collections::VecDeque<sqlx::Error>>,
        metadata_error: Option<io::ErrorKind>,
        init_calls: std::sync::atomic::AtomicUsize,
    }

    #[async_trait::async_trait]
    impl StartupDatabaseOperations for FailingStartupDatabaseOperations {
        async fn open_existing(
            &self,
            _options: &DbConnectOptions,
            _runtime: &StorageRuntimeConfig,
        ) -> sqlx::Result<StartupDatabase> {
            Err(self
                .errors
                .lock()
                .expect("error queue lock")
                .pop_front()
                .expect("an injected open error"))
        }

        async fn init(
            &self,
            _storage: &StorageArgs,
            _runtime: &StorageRuntimeConfig,
        ) -> anyhow::Result<()> {
            self.init_calls
                .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            Ok(())
        }

        fn metadata(&self, path: &Path) -> io::Result<fs::Metadata> {
            match self.metadata_error {
                Some(kind) => Err(io::Error::from(kind)),
                None => fs::metadata(path),
            }
        }
    }

    #[tokio::test]
    async fn command_source_chain_prepare_server_reopen_after_auto_init() {
        let temp = TempDir::new().expect("temp dir");
        let filename = temp.path().join("missing.db");
        let database: DbConnectOptions = format!("sqlite:{}", filename.display())
            .parse()
            .expect("SQLite options");
        let cantopen = missing_sqlite_open_error(&database).await;
        let operations = FailingStartupDatabaseOperations {
            errors: std::sync::Mutex::new(
                [cantopen, sqlx::Error::PoolClosed].into_iter().collect(),
            ),
            metadata_error: None,
            init_calls: std::sync::atomic::AtomicUsize::new(0),
        };
        let storage = StorageArgs {
            storage_path: temp.path().join("storage"),
            db: database,
        };

        let error = open_server_database_with(
            &storage,
            &StorageRuntimeConfig::default(),
            false,
            &operations,
        )
        .await
        .err()
        .expect("reopen failure must propagate");

        assert_command_source::<sqlx::Error>(&error, "auto-init failed while reopening database");
        assert_eq!(
            operations
                .init_calls
                .load(std::sync::atomic::Ordering::Relaxed),
            1
        );
    }

    #[tokio::test]
    async fn command_source_chain_prepare_server_metadata_failure() {
        let temp = TempDir::new().expect("temp dir");
        let filename = temp.path().join("missing.db");
        let database: DbConnectOptions = format!("sqlite:{}", filename.display())
            .parse()
            .expect("SQLite options");
        let cantopen = missing_sqlite_open_error(&database).await;
        let operations = FailingStartupDatabaseOperations {
            errors: std::sync::Mutex::new([cantopen].into_iter().collect()),
            metadata_error: Some(io::ErrorKind::PermissionDenied),
            init_calls: std::sync::atomic::AtomicUsize::new(0),
        };
        let storage = StorageArgs {
            storage_path: temp.path().join("storage"),
            db: database,
        };

        let error = open_server_database_with(
            &storage,
            &StorageRuntimeConfig::default(),
            false,
            &operations,
        )
        .await
        .err()
        .expect("metadata failure must propagate");

        assert_command_source::<io::Error>(
            &error,
            "failed to inspect SQLite database filename for development auto-init",
        );
        assert_eq!(
            operations
                .init_calls
                .load(std::sync::atomic::Ordering::Relaxed),
            0
        );
    }

    #[tokio::test]
    async fn command_source_chain_prepare_server_postgres_connection_failure() {
        let temp = TempDir::new().expect("temp dir");
        let database: DbConnectOptions = "postgres://jaunder@localhost/jaunder"
            .parse()
            .expect("PostgreSQL options");
        let operations = FailingStartupDatabaseOperations {
            errors: std::sync::Mutex::new([sqlx::Error::PoolTimedOut].into_iter().collect()),
            metadata_error: None,
            init_calls: std::sync::atomic::AtomicUsize::new(0),
        };
        let storage = StorageArgs {
            storage_path: temp.path().join("storage"),
            db: database,
        };

        let error = open_server_database_with(
            &storage,
            &StorageRuntimeConfig::default(),
            false,
            &operations,
        )
        .await
        .err()
        .expect("connection failure must propagate");

        assert_command_source::<sqlx::Error>(&error, INIT_FIRST_CONTEXT);
        assert!(
            error.chain().any(|source| matches!(
                source.downcast_ref::<sqlx::Error>(),
                Some(sqlx::Error::PoolTimedOut)
            )),
            "representative PostgreSQL connection failure must remain typed: {error:#}"
        );
        assert_eq!(
            operations
                .init_calls
                .load(std::sync::atomic::Ordering::Relaxed),
            0,
            "PostgreSQL connection failures must not trigger auto-init"
        );
    }

    #[tokio::test]
    async fn typed_account_command_source_cmd_user_create() {
        let mut users = storage::MockUserStorage::new();
        users.expect_create_user().return_once(|_, _, _, _| {
            Err(storage::CreateUserError::Internal(
                typed_crypto_storage_error(),
            ))
        });
        let username = parse_username("alice");
        let password = host::test_support::parse_password("password123");

        let error = create_command_user(&users, &username, &password, None, false)
            .await
            .unwrap_err();

        assert_typed_account_command_source(&error, "failed to create user");
    }

    #[tokio::test]
    async fn typed_account_command_source_app_password_lookup() {
        let mut users = storage::MockUserStorage::new();
        users
            .expect_get_user_by_username()
            .return_once(|_| Err(typed_crypto_storage_error()));
        let sessions = storage::MockSessionStorage::new();
        let username = parse_username("alice");
        let label = parse_session_label("CLI");

        let error = app_password_create_with(&users, &sessions, &username, &label)
            .await
            .unwrap_err();

        assert_typed_account_command_source(&error, "failed to look up user");
    }

    #[tokio::test]
    async fn typed_account_command_source_app_password_session_create() {
        let username = parse_username("alice");
        let mut users = storage::MockUserStorage::new();
        let returned_username = username.clone();
        users.expect_get_user_by_username().return_once(move |_| {
            Ok(Some(storage::UserRecord {
                user_id: common::ids::UserId::from(1),
                username: returned_username,
                display_name: None,
                bio: None,
                created_at: common::time::UtcInstant::now(),
                last_authenticated_at: None,
                email: None,
                email_verified: false,
                is_operator: false,
            }))
        });
        let mut sessions = storage::MockSessionStorage::new();
        sessions
            .expect_create_session()
            .return_once(|_, _| Err(typed_crypto_storage_error()));
        let label = parse_session_label("CLI");

        let error = app_password_create_with(&users, &sessions, &username, &label)
            .await
            .unwrap_err();

        assert_typed_account_command_source(&error, "failed to create app password");
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

    // `PgBootstrapArgs` holds a `PgConnectOptions` and an `AppTarget`, so a
    // non-PostgreSQL URL or one naming no database is rejected at argument parsing;
    // those rejections are pinned by `app_target_rejects_*` and
    // `create_pg_db_rejects_a_non_postgres_bootstrap_url` in `cli.rs`.

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

    #[test]
    fn format_entries_renders_sorted_key_value_lines() {
        let entries = vec![
            (
                "site.base_url".to_string(),
                "https://example.com/".to_string(),
            ),
            ("site.title".to_string(), "My Site".to_string()),
        ];
        assert_eq!(
            format_entries(&entries),
            "site.base_url=https://example.com/\nsite.title=My Site\n"
        );
        assert_eq!(format_entries(&[]), "");
    }

    /// A7: a known key with an invalid value is rejected before the write.
    #[apply(backends)]
    #[tokio::test]
    async fn site_config_set_rejects_an_invalid_value(#[case] backend: Backend) {
        let base = TempDir::new().expect("temp dir");
        let (args, _pg) = site_config_args(backend, &base).await;
        let state = storage::open_existing_database(&args.db, &StorageRuntimeConfig::default())
            .await
            .expect("reopen");
        let before = state.site_config.list().await.unwrap().len();

        cmd_site_config_set(&args, SiteConfigKey::SiteBaseUrl, "nonsense://x")
            .await
            .expect_err("an unparseable base URL is refused");

        assert_eq!(
            state.site_config.list().await.unwrap().len(),
            before,
            "no row written",
        );
    }

    /// A8: empty-means-unset survives at the CLI door.
    #[apply(backends)]
    #[tokio::test]
    async fn site_config_set_accepts_empty_for_an_optional_key(#[case] backend: Backend) {
        let base = TempDir::new().expect("temp dir");
        let (args, _pg) = site_config_args(backend, &base).await;

        cmd_site_config_set(&args, SiteConfigKey::SiteBaseUrl, "")
            .await
            .expect("empty means unset on an optional key");

        let state = storage::open_existing_database(&args.db, &StorageRuntimeConfig::default())
            .await
            .expect("reopen");
        assert_eq!(
            state
                .site_config
                .get_raw(SiteConfigKey::SiteBaseUrl)
                .await
                .unwrap(),
            Some(String::new()),
        );
    }

    /// A9: list is a faithful dump that judges without hiding.
    #[apply(backends)]
    #[tokio::test]
    async fn site_config_list_flags_unknown_keys_and_invalid_values(#[case] backend: Backend) {
        let TestEnv { state, base } = backend.setup().await;
        // A row the registry does not know. `set` cannot express it any more, which is
        // exactly the legacy case `list` exists to surface -- so write it as raw SQL
        // through the harness pool.
        base.pool()
            .execute("INSERT INTO site_config (key, value) VALUES ('legacy.orphan', 'x')")
            .await
            .unwrap();
        let cfg = &state.site_config;
        // set() is the typed seam and does not validate; the CLI does. Storing junk
        // here is how a pre-#687 row would look to `list`.
        cfg.set(SiteConfigKey::SiteBaseUrl, "nonsense://x")
            .await
            .unwrap();
        cfg.set(SiteConfigKey::SiteTitle, "My Site").await.unwrap();
        // An empty value on an optional key means unset, not invalid (spec D1b).
        cfg.set(SiteConfigKey::BackupDestinationPath, "")
            .await
            .unwrap();

        let rendered = format_entries(&cfg.list().await.unwrap());

        let line = |prefix: &str| {
            rendered
                .lines()
                .find(|l| l.starts_with(prefix))
                .unwrap_or_else(|| panic!("no line for {prefix} in:\n{rendered}"))
                .to_owned()
        };
        assert!(line("legacy.orphan=x").contains("UNKNOWN KEY"));
        assert!(line("site.base_url=nonsense://x").contains("INVALID"));
        assert_eq!(line("site.title="), "site.title=My Site");
        assert_eq!(
            line("backup.destination_path="),
            "backup.destination_path=",
            "empty on an optional key is unset, not invalid",
        );
    }

    #[tokio::test]
    async fn cmd_site_config_set_upserts_and_get_and_list_read_back() {
        let temp = TempDir::new().expect("temp dir");
        let storage_args = sqlite_storage_args(&temp);
        // Handlers use open_existing_database, so the DB must already exist.
        storage::open_database(&storage_args.db, &StorageRuntimeConfig::default())
            .await
            .expect("open db");

        cmd_site_config_set(
            &storage_args,
            SiteConfigKey::FeedsWebsubHubUrl,
            "https://x/",
        )
        .await
        .expect("set ok");
        // set() is an upsert: a second write on the same key overwrites.
        cmd_site_config_set(
            &storage_args,
            SiteConfigKey::FeedsWebsubHubUrl,
            "https://y/",
        )
        .await
        .expect("upsert ok");

        let state =
            storage::open_existing_database(&storage_args.db, &StorageRuntimeConfig::default())
                .await
                .expect("reopen");
        assert_eq!(
            state
                .site_config
                .get_raw(SiteConfigKey::FeedsWebsubHubUrl)
                .await
                .unwrap(),
            Some("https://y/".to_string()),
            "second set overwrites",
        );

        // get: present key returns Ok (exercises the println! path); an unwritten key
        // errors. A key outside the registry can no longer be named here at all — clap
        // rejects it at parse time (see `cli`'s `site_config_rejects_an_unknown_key`).
        cmd_site_config_get(&storage_args, SiteConfigKey::FeedsWebsubHubUrl)
            .await
            .expect("get present key ok");
        cmd_site_config_get(&storage_args, SiteConfigKey::SiteTitle)
            .await
            .expect_err("get unwritten key errors (→ non-zero exit)");

        // list runs against a populated store (exercises the print path).
        cmd_site_config_list(&storage_args).await.expect("list ok");
    }

    #[tokio::test]
    async fn cmd_user_invite_creates_invite_expiring_in_the_future() {
        let temp = TempDir::new().expect("temp dir");
        let storage_args = sqlite_storage_args(&temp);
        let state = storage::open_database(&storage_args.db, &StorageRuntimeConfig::default())
            .await
            .expect("open db");

        let before = common::time::UtcInstant::now();
        cmd_user_invite(&storage_args, Some(parse_invite_ttl_hours("24")))
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
        let storage_args = sqlite_storage_args(&temp);
        let state = storage::open_database(&storage_args.db, &StorageRuntimeConfig::default())
            .await
            .expect("open db");
        state
            .site_config
            .set(SiteConfigKey::SiteBaseUrl, "https://example.com")
            .await
            .expect("set base_url");

        cmd_user_invite(&storage_args, Some(parse_invite_ttl_hours("24")))
            .await
            .expect("create invite");

        let invites = state.invites.list_invites().await.expect("list invites");
        assert_eq!(invites.len(), 1, "exactly one invite must be created");
    }

    #[tokio::test]
    async fn open_server_database_carries_pool_observer() {
        let temp = TempDir::new().expect("temp dir");
        let storage = sqlite_storage_args(&temp);
        storage::open_database(&storage.db, &StorageRuntimeConfig::default())
            .await
            .expect("open db");

        let database = open_server_database(&storage, &StorageRuntimeConfig::default(), false)
            .await
            .expect("open server database");
        let snapshot = database.pool_observer.snapshot();

        assert!(snapshot.max >= 1);
        assert!(snapshot.used <= snapshot.max);
        assert!(snapshot.idle <= snapshot.max);
        assert!(Arc::strong_count(&database.state) >= 1);
    }

    #[tokio::test]
    async fn prepare_server_auto_initializes_in_dev_mode() {
        // A fresh storage dir with no database: `open_existing_database` fails,
        // and because `prod == false`, `prepare_server` takes the dev auto-init
        // branch (warn + `cmd_init` + reopen) instead of erroring. Binding to
        // port 0 avoids a fixed-port clash; we never enter the serve loop.
        let temp = TempDir::new().expect("temp dir");
        let db_path = temp.path().join("jaunder.db");
        let storage = sqlite_storage_args(&temp);
        assert!(
            !db_path.exists(),
            "database must not exist before prepare_server"
        );

        let bind: std::net::SocketAddr = "127.0.0.1:0".parse().expect("bind addr");
        let telemetry = test_telemetry(None);
        let prepared = prepare_server(&storage, bind, false, None, &telemetry, None)
            .await
            .expect("dev-mode prepare_server must auto-initialize");

        assert!(db_path.exists(), "auto-init must have created the database");
        // Drop the prepared server (and its background workers) without serving.
        drop(prepared);
    }

    #[test]
    fn prepare_server_registers_saturation_sampler_when_otel_endpoint_is_set() {
        let runtime = tokio::runtime::Runtime::new().expect("runtime");
        runtime.block_on(async {
            let temp = TempDir::new().expect("temp dir");
            let storage = sqlite_storage_args(&temp);
            storage::open_database(&storage.db, &StorageRuntimeConfig::default())
                .await
                .expect("open db");
            let bind: std::net::SocketAddr = "127.0.0.1:0".parse().expect("bind addr");

            let telemetry = test_telemetry(Some("http://127.0.0.1:4318"));
            let prepared = prepare_server(&storage, bind, false, None, &telemetry, None)
                .await
                .expect("prepare server");

            assert!(prepared.saturation_metrics.is_some());
        });
    }

    #[test]
    fn prepare_server_does_not_start_saturation_sampler_without_otel_endpoint() {
        let runtime = tokio::runtime::Runtime::new().expect("runtime");
        runtime.block_on(async {
            let temp = TempDir::new().expect("temp dir");
            let storage = sqlite_storage_args(&temp);
            storage::open_database(&storage.db, &StorageRuntimeConfig::default())
                .await
                .expect("open db");
            let bind: std::net::SocketAddr = "127.0.0.1:0".parse().expect("bind addr");

            let telemetry = test_telemetry(None);
            let prepared = prepare_server(&storage, bind, false, None, &telemetry, None)
                .await
                .expect("prepare server");

            assert!(prepared.saturation_metrics.is_none());
        });
    }

    #[tokio::test]
    async fn prepare_server_refuses_on_live_holder_before_db_open() {
        // A planted runtime.json naming a live writer (our own pid + real
        // start-time) must make prepare_server refuse *before* opening/creating
        // the DB (#141). Uses dev mode (prod == false) so, absent the mutex, it
        // would auto-init — proving the refusal precedes that.
        let temp = TempDir::new().expect("temp dir");
        let db_path = temp.path().join("jaunder.db");
        let storage = sqlite_storage_args(&temp);
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
        let telemetry = test_telemetry(None);
        let err = prepare_server(&storage, bind, false, None, &telemetry, None)
            .await
            .err();
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
