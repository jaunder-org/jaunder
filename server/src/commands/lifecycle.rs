use std::{
    fs, io,
    net::SocketAddr,
    path::{Path, PathBuf},
    sync::{Arc, RwLock},
    time::Duration,
};

use anyhow::Context;
use axum::Router;
use host::{
    error,
    metrics::{SaturationObservableGuard, SaturationSnapshot},
    telemetry::TelemetryConfig,
    theme_operations::ThemeOperationCoordinator,
};
use tokio::{net::TcpListener, sync::oneshot::Receiver, task::JoinHandle};

use super::support;
use crate::cli::StorageArgs;
use crate::feed::worker::FeedWorker;
use crate::maintenance::{self, DatabaseMaintenance};
use crate::metrics::{self, SaturationSources};
use crate::publisher::PublisherService;
use crate::runtime_file::{self, RuntimeGuard, StartupCheck, StartupLockGuard};
use crate::scheduled_worker::ScheduledWorkerGuard;
#[cfg(test)]
use host::config_key::SiteConfigKey;
use storage::{
    AudienceStorage, DbConnectOptions, DbPoolObserver, EmailVerificationStorage, FeedCacheStorage,
    FeedEventStorage, InstanceId, InviteStorage, MediaContentLocks, MediaManager, MediaStorage,
    PasswordResetStorage, PostMediaOwnership, PostStorage, PublisherStorage, SessionStorage,
    SiteConfigStorage, StorageRuntimeConfig, SubscriptionStorage, ThemeAssetManager, ThemeManager,
    ThemeStorage, UserConfigStorage, UserStorage, WriteScope,
};

/// Focused storage handles minted once by the serve composition root.
///
/// This root-only assembly is never injected into runtime subsystems; each
/// consumer receives only the handles and services it directly needs.
struct ServeStorage {
    site_config: Arc<dyn SiteConfigStorage>,
    users: Arc<dyn UserStorage>,
    sessions: Arc<dyn SessionStorage>,
    invites: Arc<dyn InviteStorage>,
    email_verifications: Arc<dyn EmailVerificationStorage>,
    password_resets: Arc<dyn PasswordResetStorage>,
    posts: Arc<dyn PostStorage>,
    subscriptions: Arc<dyn SubscriptionStorage>,
    audiences: Arc<dyn AudienceStorage>,
    media: Arc<dyn MediaStorage>,
    user_config: Arc<dyn UserConfigStorage>,
    feed_cache: Arc<dyn FeedCacheStorage>,
    feed_events: Arc<dyn FeedEventStorage>,
    publisher: Arc<dyn PublisherStorage>,
    themes: Arc<dyn ThemeStorage>,
    write_scope: WriteScope,
}

impl ServeStorage {
    fn from_factory(factory: &storage::StorageFactory) -> Self {
        Self {
            site_config: factory.site_config(),
            users: factory.users(),
            sessions: factory.sessions(),
            invites: factory.invites(),
            email_verifications: factory.email_verifications(),
            password_resets: factory.password_resets(),
            posts: factory.posts(),
            subscriptions: factory.subscriptions(),
            audiences: factory.audiences(),
            media: factory.media(),
            user_config: factory.user_config(),
            feed_cache: factory.feed_cache(),
            feed_events: factory.feed_events(),
            publisher: factory.publisher(),
            themes: factory.themes(),
            write_scope: factory.write_scope(),
        }
    }
}
const CAPTURE_FEED_INTERVAL: Duration = Duration::from_millis(250);
const PRODUCTION_FEED_INTERVAL: Duration = Duration::from_secs(10);

fn feed_worker_interval(capture_enabled: bool) -> Duration {
    if capture_enabled {
        CAPTURE_FEED_INTERVAL
    } else {
        PRODUCTION_FEED_INTERVAL
    }
}

/// Capture leaf paths resolved by the serve composition root.
pub struct ServeCapturePaths {
    pub mail: PathBuf,
    pub websub: PathBuf,
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
    factory: storage::StorageFactory,
    instance_id: InstanceId,
    pool_observer: DbPoolObserver,
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
            factory: opened.factory,
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
        support::INIT_FIRST_CONTEXT
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

/// A bound listener and router ready to serve, plus the live background workers
/// that must outlive the serve loop. Produced by [`prepare_server`].
pub struct PreparedServer {
    /// The bound TCP listener.
    pub listener: TcpListener,
    /// The fully wired application router.
    pub router: Router,
    workers: BackgroundWorkers,
    /// Owns the OS-backed startup lock and removes the runtime-info file on drop.
    runtime_guard: RuntimeGuard,
    pub saturation_metrics: Option<PreparedSaturationMetrics>,
}

struct BackgroundWorkerSetup {
    maintenance: DatabaseMaintenance,
    backup_site_config: Arc<dyn SiteConfigStorage>,
    database: DbConnectOptions,
    runtime: StorageRuntimeConfig,
    storage_path: PathBuf,
    feed_worker: FeedWorker,
    feed_interval: Duration,
}

struct BackgroundWorkers {
    backup: Option<ScheduledWorkerGuard>,
    maintenance: ScheduledWorkerGuard,
    feed: ScheduledWorkerGuard,
}
struct SaturationMetricsSetup {
    observables: SaturationObservableGuard,
    sources: SaturationSources,
    snapshot: Arc<RwLock<SaturationSnapshot>>,
}

impl SaturationMetricsSetup {
    fn start(self) -> PreparedSaturationMetrics {
        PreparedSaturationMetrics {
            _observables: self.observables,
            sampler: metrics::spawn_saturation_sampler(self.sources, self.snapshot),
        }
    }
}

impl BackgroundWorkers {
    async fn start(setup: BackgroundWorkerSetup) -> anyhow::Result<Self> {
        let mut maintenance = setup
            .maintenance
            .start(maintenance::DATABASE_MAINTENANCE_INTERVAL)
            .await?;
        let mut backup = match crate::backup::start_backup_worker(
            setup.backup_site_config,
            setup.database,
            setup.runtime,
            setup.storage_path,
        )
        .await
        {
            Ok(scheduler) => scheduler,
            Err(error) => {
                maintenance.stop();
                stop_worker_after_start_failure(
                    &mut maintenance,
                    "server.maintenance.start_rollback",
                )
                .await;
                return Err(error);
            }
        };
        let feed = match setup.feed_worker.start(setup.feed_interval).await {
            Ok(scheduler) => scheduler,
            Err(error) => {
                maintenance.stop();
                if let Some(scheduler) = backup.as_ref() {
                    scheduler.stop();
                }
                if let Some(scheduler) = backup.as_mut() {
                    stop_worker_after_start_failure(scheduler, "server.backup.start_rollback")
                        .await;
                    // The await body is covered; LLVM assigns this closing edge a zero count.
                } // cov:ignore
                stop_worker_after_start_failure(
                    &mut maintenance,
                    "server.maintenance.start_rollback",
                )
                .await;
                return Err(error);
            }
        };
        Ok(Self {
            backup,
            maintenance,
            feed,
        })
    }

    fn stop(&self) {
        self.feed.stop();
        self.maintenance.stop();
        if let Some(worker) = self.backup.as_ref() {
            worker.stop();
        }
    }

    async fn shutdown(&mut self) -> (anyhow::Result<()>, anyhow::Result<()>, anyhow::Result<()>) {
        self.stop();
        let feed = self.feed.shutdown().await;
        let maintenance = self.maintenance.shutdown().await;
        let backup = match self.backup.as_mut() {
            Some(worker) => worker.shutdown().await,
            None => Ok(()),
        };
        (feed, maintenance, backup)
    }
}

pub struct PreparedSaturationMetrics {
    _observables: SaturationObservableGuard,
    sampler: metrics::SaturationSampler,
}

impl PreparedSaturationMetrics {
    async fn shutdown(self) -> anyhow::Result<()> {
        self.sampler.shutdown().await?;
        Ok(())
    }
}

async fn prepare_saturation_metrics(
    site_config: Arc<dyn SiteConfigStorage>,
    feed_events: Arc<dyn FeedEventStorage>,
    media: Arc<dyn MediaStorage>,
    pool_observer: DbPoolObserver,
    media_root: PathBuf,
    telemetry: &TelemetryConfig,
) -> anyhow::Result<Option<SaturationMetricsSetup>> {
    if !telemetry.otlp_endpoint_configured() {
        return Ok(None);
    }
    let backup_config = site_config
        .get_backup_config()
        .await
        .context("failed to load backup configuration for saturation metrics")?;
    let backup_destination_root = backup_config.destination_path.as_deref().map(PathBuf::from);
    let snapshot = Arc::new(RwLock::new(SaturationSnapshot::default()));
    let observables = host::metrics::register_saturation_observables(snapshot.clone());
    let sources = SaturationSources::real(
        feed_events,
        media,
        media_root,
        backup_destination_root,
        pool_observer,
    );
    Ok(Some(SaturationMetricsSetup {
        observables,
        sources,
        snapshot,
    }))
}

async fn stop_worker_after_start_failure(worker: &mut ScheduledWorkerGuard, context: &'static str) {
    worker.stop();
    if let Err(error) = worker.shutdown().await {
        // cov:ignore-start -- tokio-cron-scheduler 0.13 shutdown always returns Ok;
        // retain reporting so a future fallible implementation does not hide cleanup failure.
        error::report_swallowed(
            error::ErrorKind::Internal,
            error::ErrorClass::Transient,
            context,
            error::SwallowedSource::Error(error.root_cause()),
        );
        // cov:ignore-stop
    }
}

fn merge_worker_shutdown(
    primary: &mut anyhow::Result<()>,
    shutdown: anyhow::Result<()>,
    context: &'static str,
) {
    let Err(error) = shutdown else {
        return;
    };
    let error = error.context(context);
    if primary.is_ok() {
        *primary = Err(error);
    } else {
        error::report_swallowed(
            error::ErrorKind::Internal,
            error::ErrorClass::Transient,
            context,
            error::SwallowedSource::Error(error.root_cause()),
        );
    }
}

fn combine_context_providers<A, P, M, T, O, U, S>(
    accounts: A,
    publication: P,
    media_configuration: M,
    themes: T,
    ownership: O,
    publisher: U,
    services: S,
) -> impl Fn() + Clone + Send + Sync + 'static
where
    A: Fn() + Clone + Send + Sync + 'static,
    P: Fn() + Clone + Send + Sync + 'static,
    M: Fn() + Clone + Send + Sync + 'static,
    T: Fn() + Clone + Send + Sync + 'static,
    O: Fn() + Clone + Send + Sync + 'static,
    U: Fn() + Clone + Send + Sync + 'static,
    S: Fn() + Clone + Send + Sync + 'static,
{
    move || {
        accounts();
        publication();
        media_configuration();
        themes();
        ownership();
        publisher();
        services();
    }
}

fn prepare_runtime_identity(
    storage_path: &Path,
    bind: SocketAddr,
) -> anyhow::Result<(RuntimeGuard, u64)> {
    // Establish our own start-time up front (before opening the DB): if `/proc` is
    // unusable we cannot preserve live runtime-file detection, so refuse rather
    // than serve with a silently-broken guard (#141).
    let start_time = runtime_file::require_start_time_at(Path::new("/proc/self/stat"))?;
    let runtime_path = runtime_file::canonical_runtime_path(storage_path);
    let startup_lock = StartupLockGuard::acquire(storage_path)?;
    match runtime_file::check_startup_mutex(&runtime_path)? {
        StartupCheck::Refuse { pid } => anyhow::bail!(
            "another jaunder instance is already running on data dir {} (pid {pid}); \
             refusing to start",
            storage_path.display()
        ),
        StartupCheck::Stale | StartupCheck::Proceed => {}
    }
    Ok((
        startup_lock.reserve(SocketAddr::new(bind.ip(), 0), start_time)?,
        start_time,
    ))
}

fn publisher_service(
    storage_path: PathBuf,
    publisher: Arc<dyn PublisherStorage>,
    write_scope: WriteScope,
) -> Arc<PublisherService> {
    Arc::new(PublisherService::new(storage_path, publisher, write_scope))
}
fn media_ownership(
    instance_id: InstanceId,
    site_config: Arc<dyn SiteConfigStorage>,
) -> (
    Arc<dyn storage::MediaReferenceOwnershipResolver>,
    PostMediaOwnership,
) {
    let resolver = Arc::new(crate::media_ownership::LiveMediaReferenceOwnershipResolver::new());
    let ownership = PostMediaOwnership::new(resolver.clone(), instance_id, site_config);
    (resolver, ownership)
}

fn database_maintenance(
    posts: Arc<dyn PostStorage>,
    invites: Arc<dyn storage::InviteStorage>,
    email_verifications: Arc<dyn storage::EmailVerificationStorage>,
    password_resets: Arc<dyn storage::PasswordResetStorage>,
    feed_events: Arc<dyn FeedEventStorage>,
) -> DatabaseMaintenance {
    DatabaseMaintenance::new(
        posts,
        invites,
        email_verifications,
        password_resets,
        feed_events,
    )
}

fn feed_worker(
    posts: Arc<dyn PostStorage>,
    feed_cache: Arc<dyn FeedCacheStorage>,
    write_scope: WriteScope,
    publisher: Arc<PublisherService>,
    feed_events: Arc<dyn FeedEventStorage>,
    websub: Arc<dyn crate::websub::WebSubClient>,
) -> FeedWorker {
    FeedWorker::new(
        posts,
        feed_cache,
        Arc::new(write_scope),
        publisher,
        feed_events,
        websub,
    )
}

fn media_manager(
    media: Arc<dyn MediaStorage>,
    posts: Arc<dyn PostStorage>,
    site_config: Arc<dyn SiteConfigStorage>,
    write_scope: WriteScope,
    content_locks: Arc<MediaContentLocks>,
    instance_id: InstanceId,
    ownership_resolver: Arc<dyn storage::MediaReferenceOwnershipResolver>,
) -> Arc<MediaManager> {
    Arc::new(MediaManager::new(
        media,
        posts,
        site_config,
        write_scope,
        content_locks,
        instance_id,
        ownership_resolver,
    ))
}
fn theme_asset_manager(
    themes: Arc<dyn storage::ThemeStorage>,
    write_scope: WriteScope,
    storage_path: Arc<PathBuf>,
) -> Arc<ThemeAssetManager> {
    Arc::new(ThemeAssetManager::new(themes, write_scope, storage_path))
}

fn theme_manager(
    themes: Arc<dyn storage::ThemeStorage>,
    media: Arc<dyn MediaStorage>,
    write_scope: WriteScope,
    content_locks: Arc<MediaContentLocks>,
) -> Arc<ThemeManager> {
    Arc::new(ThemeManager::new(themes, media, write_scope, content_locks))
}

fn compose_server_router(
    dependencies: &ServeStorage,
    storage_path: PathBuf,
    instance_id: &InstanceId,
    mailer: Arc<dyn common::mailer::MailSender>,
    prod: bool,
) -> Router {
    let storage_path = Arc::new(storage_path);
    let locks = Arc::new(MediaContentLocks::new(Arc::clone(&storage_path)));
    let (resolver, ownership) =
        media_ownership(instance_id.clone(), Arc::clone(&dependencies.site_config));
    let publisher = publisher_service(
        (*storage_path).clone(),
        Arc::clone(&dependencies.publisher),
        dependencies.write_scope.clone(),
    );
    let manager = media_manager(
        Arc::clone(&dependencies.media),
        Arc::clone(&dependencies.posts),
        Arc::clone(&dependencies.site_config),
        dependencies.write_scope.clone(),
        Arc::clone(&locks),
        instance_id.clone(),
        resolver,
    );
    let asset_manager = theme_asset_manager(
        Arc::clone(&dependencies.themes),
        dependencies.write_scope.clone(),
        Arc::clone(&storage_path),
    );
    let manager_themes = theme_manager(
        Arc::clone(&dependencies.themes),
        Arc::clone(&dependencies.media),
        dependencies.write_scope.clone(),
        Arc::clone(&locks),
    );
    let contexts = combine_context_providers(
        crate::context::account_context_provider(
            Arc::clone(&dependencies.users),
            Arc::clone(&dependencies.sessions),
            Arc::clone(&dependencies.invites),
            Arc::clone(&dependencies.email_verifications),
            Arc::clone(&dependencies.password_resets),
        ),
        crate::context::publication_context_provider(
            Arc::clone(&dependencies.posts),
            dependencies.write_scope.clone(),
            Arc::clone(&dependencies.subscriptions),
            Arc::clone(&dependencies.audiences),
            Arc::clone(&dependencies.feed_events),
        ),
        crate::context::media_configuration_context_provider(
            Arc::clone(&dependencies.media),
            Arc::clone(&dependencies.user_config),
            Arc::clone(&dependencies.site_config),
        ),
        crate::context::theme_context_provider(Arc::clone(&dependencies.themes)),
        crate::context::post_media_ownership_context_provider(ownership.clone()),
        crate::context::publisher_context_provider(Arc::clone(&publisher)),
        crate::context::service_context_provider(
            mailer,
            Arc::clone(&locks),
            Arc::clone(&manager),
            asset_manager,
            Arc::new(ThemeOperationCoordinator::new()),
            manager_themes,
            prod,
        ),
    );
    let public_projector = crate::projector::PublicProjector::new(
        Arc::clone(&dependencies.posts),
        Arc::clone(&dependencies.users),
        Arc::clone(&dependencies.themes),
        crate::projector::Shell(crate::site::shell_html()),
    );
    let app = crate::application_routes(
        crate::client_telemetry_routes(
            Arc::clone(&dependencies.sessions),
            dependencies.write_scope.clone(),
        ),
        contexts,
        public_projector,
    );
    let app = crate::context::with_media_extensions(app, ownership, manager, locks, storage_path);
    let app = crate::context::with_post_account_extensions(
        app,
        Arc::clone(&dependencies.posts),
        Arc::clone(&dependencies.audiences),
        Arc::clone(&dependencies.users),
        Arc::clone(&dependencies.user_config),
    );
    let app = crate::context::with_theme_media_extensions(
        app,
        Arc::clone(&dependencies.themes),
        Arc::clone(&dependencies.site_config),
        Arc::clone(&dependencies.media),
        Arc::clone(&dependencies.feed_cache),
    );
    let app = crate::context::with_publisher_extensions(
        app,
        publisher,
        Arc::clone(&dependencies.feed_events),
        Arc::clone(&dependencies.sessions),
        dependencies.write_scope.clone(),
    );
    crate::create_router(app, instance_id, prod)
}

async fn reconcile_theme_assets(
    themes: Arc<dyn storage::ThemeStorage>,
    write_scope: WriteScope,
    storage_path: Arc<PathBuf>,
) -> anyhow::Result<()> {
    ThemeAssetManager::new(themes, write_scope, storage_path)
        .reconcile_startup()
        .await
        .context("theme immutable-content reconciliation failed")
        .map(|_| ())
}

fn prepare_background_worker_setup(
    maintenance: DatabaseMaintenance,
    backup_site_config: Arc<dyn SiteConfigStorage>,
    database: DbConnectOptions,
    runtime: StorageRuntimeConfig,
    storage_path: PathBuf,
    feed_worker: FeedWorker,
    feed_interval: Duration,
) -> BackgroundWorkerSetup {
    BackgroundWorkerSetup {
        maintenance,
        backup_site_config,
        database,
        runtime,
        storage_path,
        feed_worker,
        feed_interval,
    }
}

fn prepare_background_worker_setup_from_dependencies(
    dependencies: &ServeStorage,
    storage: &StorageArgs,
    runtime: StorageRuntimeConfig,
    capture: Option<&ServeCapturePaths>,
) -> BackgroundWorkerSetup {
    let maintenance = database_maintenance(
        Arc::clone(&dependencies.posts),
        Arc::clone(&dependencies.invites),
        Arc::clone(&dependencies.email_verifications),
        Arc::clone(&dependencies.password_resets),
        Arc::clone(&dependencies.feed_events),
    );
    let websub_capture = capture.map(|paths| paths.websub.clone());
    let feed_interval = feed_worker_interval(websub_capture.is_some());
    let feed_worker = feed_worker(
        Arc::clone(&dependencies.posts),
        Arc::clone(&dependencies.feed_cache),
        dependencies.write_scope.clone(),
        publisher_service(
            storage.storage_path.clone(),
            Arc::clone(&dependencies.publisher),
            dependencies.write_scope.clone(),
        ),
        Arc::clone(&dependencies.feed_events),
        crate::websub::default_client(websub_capture),
    );
    prepare_background_worker_setup(
        maintenance,
        Arc::clone(&dependencies.site_config),
        storage.db.clone(),
        runtime,
        storage.storage_path.clone(),
        feed_worker,
        feed_interval,
    )
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
/// Returns an error if the runtime mutex refuses startup, the temporary upload
/// directory cannot be prepared, the database cannot be opened/initialized, a
/// worker fails to start, or the listener cannot bind.
pub async fn prepare_server(
    storage: &StorageArgs,
    bind: SocketAddr,
    prod: bool,
    telemetry: &TelemetryConfig,
    capture: Option<&ServeCapturePaths>,
) -> anyhow::Result<PreparedServer> {
    let (runtime_guard, start_time) = prepare_runtime_identity(&storage.storage_path, bind)?;
    // The exclusive OS lock and live reservation above prove no valid upload can
    // be active. Establish a clean transient area before any upload-capable
    // server state is prepared.
    MediaManager::prepare_temporary_upload_directory(&storage.storage_path)
        .await
        .context("failed to prepare media temporary upload directory")?;
    let runtime = support::storage_runtime_config(&storage.db)?;
    let StartupDatabase {
        factory,
        instance_id,
        pool_observer,
    } = open_server_database(storage, &runtime, prod).await?;
    let dependencies = ServeStorage::from_factory(&factory);
    reconcile_theme_assets(
        Arc::clone(&dependencies.themes),
        dependencies.write_scope.clone(),
        Arc::new(storage.storage_path.clone()),
    )
    .await?;
    let worker_setup =
        prepare_background_worker_setup_from_dependencies(&dependencies, storage, runtime, capture);
    let saturation_metrics = prepare_saturation_metrics(
        Arc::clone(&dependencies.site_config),
        Arc::clone(&dependencies.feed_events),
        Arc::clone(&dependencies.media),
        pool_observer,
        storage.storage_path.join("media"),
        telemetry,
    )
    .await?;
    let mailer = crate::mailer::build_mailer(
        dependencies.site_config.as_ref(),
        capture.map(|paths| paths.mail.clone()),
    )
    .await?;
    let router = compose_server_router(
        &dependencies,
        storage.storage_path.clone(),
        &instance_id,
        mailer,
        prod,
    );

    let listener = tokio::net::TcpListener::bind(bind).await?;
    let workers = BackgroundWorkers::start(worker_setup).await?;
    let saturation_metrics = saturation_metrics.map(SaturationMetricsSetup::start);

    // `local_addr` cannot fail on a just-bound listener; fall back to the
    // requested `bind` rather than add a never-taken error branch.
    let addr = listener.local_addr().unwrap_or(bind);
    // Update the live identity after binding, while the OS lock is held.
    runtime_guard.update_address(addr, start_time);

    Ok(PreparedServer {
        listener,
        router,
        workers,
        runtime_guard,
        saturation_metrics,
    })
}

/// Serves `router` on `listener`, draining in-flight requests when `shutdown`
/// resolves, then returns. Runtime ownership remains with the caller so it can
/// stop every background worker before releasing the storage-directory lock.
///
/// # Errors
///
/// Returns an error if the server exits with an error.
async fn serve_with_shutdown(
    listener: TcpListener,
    router: axum::Router,
    shutdown: impl Future<Output = ()> + Send + 'static,
) -> anyhow::Result<()> {
    axum::serve(listener, router)
        .with_graceful_shutdown(shutdown)
        .await?;
    Ok(())
}

/// Models the shutdown supervisor's signal policy without process-global handler
/// installation or process termination.
#[cfg(any(test, unix))]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum ShutdownSignal {
    Sigint,
    Sigterm,
}

#[cfg(any(test, unix))]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum ShutdownState {
    AwaitingSignal,
    Draining,
}

#[cfg(any(test, unix))]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum ShutdownTransition {
    BeginGracefulDrain,
    ContinueGracefulDrain,
    ForceExitAndRemoveRuntimeIdentity,
}

#[cfg(any(test, unix))]
fn shutdown_transition(state: ShutdownState, signal: ShutdownSignal) -> ShutdownTransition {
    match (state, signal) {
        (ShutdownState::AwaitingSignal, _) => ShutdownTransition::BeginGracefulDrain,
        (ShutdownState::Draining, ShutdownSignal::Sigterm) => {
            ShutdownTransition::ContinueGracefulDrain
        }
        (ShutdownState::Draining, ShutdownSignal::Sigint) => {
            ShutdownTransition::ForceExitAndRemoveRuntimeIdentity
        }
    }
}

/// Owns the shutdown supervisor for the complete lifetime of a serve command.
///
/// Dropping this guard aborts the supervisor, so no setup failure can detach its
/// task after the signal handlers have been installed.
#[cfg(unix)]
struct ShutdownSupervisor {
    task: Option<JoinHandle<()>>,
}

#[cfg(unix)]
impl ShutdownSupervisor {
    /// Installs the signal handlers synchronously, before returning the receiver
    /// that begins graceful shutdown on the first signal.
    fn install(runtime_path: PathBuf) -> io::Result<(Receiver<()>, Self)> {
        use tokio::signal::unix::{SignalKind, signal};

        let mut sigint = signal(SignalKind::interrupt())?;
        let mut sigterm = signal(SignalKind::terminate())?;
        let (tx, rx) = tokio::sync::oneshot::channel();
        let task = tokio::spawn(async move {
            // cov:ignore-start -- async signal wait-loop; the forced branch ends in
            // process::exit and is unreachable by a survivable test. The synchronous
            // setup above and serve_with_shutdown are host-covered by the signal tests.
            let mut state = ShutdownState::AwaitingSignal;
            let mut graceful_shutdown = Some(tx);
            loop {
                let (signal, signal_name) = tokio::select! {
                    _ = sigint.recv() => (ShutdownSignal::Sigint, "SIGINT"),
                    _ = sigterm.recv() => (ShutdownSignal::Sigterm, "SIGTERM"),
                };
                match shutdown_transition(state, signal) {
                    ShutdownTransition::BeginGracefulDrain => {
                        tracing::info!(
                            signal = signal_name,
                            "received shutdown signal; draining in-flight requests"
                        );
                        if let Some(tx) = graceful_shutdown.take() {
                            let _ = tx.send(());
                        }
                        state = ShutdownState::Draining;
                    }
                    ShutdownTransition::ContinueGracefulDrain => {
                        tracing::info!(
                            "received SIGTERM while draining; continuing graceful shutdown"
                        );
                    }
                    ShutdownTransition::ForceExitAndRemoveRuntimeIdentity => {
                        tracing::warn!("received SIGINT while draining; forcing immediate exit");
                        runtime_file::remove_runtime_file(&runtime_path);
                        std::process::exit(0);
                    }
                }
            }
            // cov:ignore-stop
        });
        Ok((rx, Self { task: Some(task) }))
    }

    async fn abort_and_join(mut self) {
        let Some(task) = self.task.take() else {
            unreachable!("shutdown supervisor owns one task until joined")
        };
        task.abort();
        match task.await {
            Err(error) if error.is_cancelled() => {}
            Ok(()) => {} // cov:ignore
            // cov:ignore-start
            Err(error) => error::report_swallowed(
                error::ErrorKind::Internal,
                error::ErrorClass::Transient,
                "server.shutdown_supervisor.join",
                error::SwallowedSource::Error(&error),
            ),
            // cov:ignore-stop
        }
    }
}

#[cfg(unix)]
impl Drop for ShutdownSupervisor {
    fn drop(&mut self) {
        if let Some(task) = &self.task {
            task.abort();
        }
    }
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
    telemetry: &TelemetryConfig,
    capture: Option<&ServeCapturePaths>,
) -> anyhow::Result<()> {
    // Telemetry is owned by `run`, which holds the TelemetryGuard across this
    // call (see `server/src/main.rs`); `cmd_serve` does not init it, matching
    // every other `cmd_*`.
    // Install handlers before preparation can publish a nonzero runtime port.
    // The guard aborts its task if preparation returns early.
    #[cfg(unix)]
    let (shutdown_rx, shutdown_supervisor) =
        ShutdownSupervisor::install(runtime_file::canonical_runtime_path(&storage.storage_path))?;

    let PreparedServer {
        listener,
        router,
        mut workers,
        runtime_guard,
        saturation_metrics,
    } = prepare_server(storage, bind, prod, telemetry, capture).await?;

    tracing::info!(bind = %bind, prod, "starting HTTP server");
    #[cfg(unix)]
    let mut serve_result = serve_with_shutdown(listener, router, async move {
        let _ = shutdown_rx.await;
    })
    .await;
    #[cfg(not(unix))]
    let mut serve_result = {
        // No signal handling off unix (jaunder targets Linux/NixOS): serve until
        // the process is otherwise terminated.
        serve_with_shutdown(listener, router, std::future::pending::<()>()).await
    };
    // Refuse every new background activation together. Admitted work and the
    // current saturation sample must finish while both the storage lock and the
    // SIGINT forced-exit supervisor remain live.
    workers.stop();
    if let Some(metrics) = saturation_metrics {
        let saturation_shutdown = metrics.shutdown().await;
        merge_worker_shutdown(
            &mut serve_result,
            saturation_shutdown,
            "server.metrics.shutdown",
        );
    } // cov:ignore — LLVM assigns this completed shutdown branch's closing edge a zero count.
    let (feed_shutdown, maintenance_shutdown, backup_shutdown) = workers.shutdown().await;
    merge_worker_shutdown(&mut serve_result, feed_shutdown, "server.feed.shutdown");
    merge_worker_shutdown(
        &mut serve_result,
        maintenance_shutdown,
        "server.maintenance.shutdown",
    );
    merge_worker_shutdown(&mut serve_result, backup_shutdown, "server.backup.shutdown");
    #[cfg(unix)]
    shutdown_supervisor.abort_and_join().await;
    drop(workers);
    drop(runtime_guard);
    serve_result
}

#[cfg(test)]
mod tests {
    use super::super::test_support::{assert_command_source, sqlite_storage_args};
    use super::*;
    use storage::{DbConnectOptions, MediaTemporaryDirectoryError, test_support::confirmed};
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
    fn feed_worker_interval_is_250_ms_for_capture() {
        assert_eq!(feed_worker_interval(true), Duration::from_millis(250));
    }

    #[test]
    fn feed_worker_interval_is_10_seconds_without_capture() {
        assert_eq!(feed_worker_interval(false), Duration::from_secs(10));
    }

    #[test]
    fn combined_context_provider_runs_each_provider_in_order() {
        let calls = Arc::new(std::sync::Mutex::new(Vec::new()));
        let provider = |name| {
            let calls = Arc::clone(&calls);
            move || calls.lock().expect("record context provider").push(name)
        };
        let combined = combine_context_providers(
            provider("accounts"),
            provider("publication"),
            provider("media configuration"),
            provider("themes"),
            provider("ownership"),
            provider("publisher"),
            provider("services"),
        );

        combined();

        assert_eq!(
            *calls.lock().expect("read context provider order"),
            [
                "accounts",
                "publication",
                "media configuration",
                "themes",
                "ownership",
                "publisher",
                "services",
            ]
        );
    }

    async fn background_worker_setup(
        storage: &StorageArgs,
        backup_site_config: Arc<dyn SiteConfigStorage>,
        feed_interval: Duration,
    ) -> BackgroundWorkerSetup {
        let factory =
            storage::open_existing_database(&storage.db, &StorageRuntimeConfig::default())
                .await
                .expect("open test database");
        let posts = factory.posts();
        let invites = factory.invites();
        let email_verifications = factory.email_verifications();
        let password_resets = factory.password_resets();
        let feed_events = factory.feed_events();
        let feed_cache = factory.feed_cache();
        let publisher = factory.publisher();
        let write_scope = factory.write_scope();
        BackgroundWorkerSetup {
            maintenance: DatabaseMaintenance::new(
                Arc::clone(&posts),
                invites,
                email_verifications,
                password_resets,
                Arc::clone(&feed_events),
            ),
            backup_site_config,
            database: storage.db.clone(),
            runtime: StorageRuntimeConfig::default(),
            storage_path: storage.storage_path.clone(),
            feed_worker: FeedWorker::new(
                posts,
                feed_cache,
                Arc::new(write_scope.clone()),
                Arc::new(PublisherService::new(
                    storage.storage_path.clone(),
                    publisher,
                    write_scope,
                )),
                feed_events,
                crate::websub::default_client(None),
            ),
            feed_interval,
        }
    }

    async fn shutdown_prepared_server(mut prepared: PreparedServer) {
        if let Some(metrics) = prepared.saturation_metrics.take() {
            metrics
                .shutdown()
                .await
                .expect("saturation sampler shutdown");
        }
        let (feed, maintenance, backup) = prepared.workers.shutdown().await;
        feed.expect("feed worker shutdown");
        maintenance.expect("maintenance worker shutdown");
        backup.expect("backup worker shutdown");
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

        assert_command_source::<sqlx::Error>(&error, support::INIT_FIRST_CONTEXT);
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
    }

    #[tokio::test]
    async fn background_workers_roll_back_maintenance_when_backup_config_load_fails() {
        let temp = TempDir::new().expect("temp dir");
        let storage = sqlite_storage_args(&temp);
        storage::open_database(&storage.db, &StorageRuntimeConfig::default())
            .await
            .expect("open db");
        let mut site_config = storage::MockSiteConfigStorage::new();
        site_config.expect_get_backup_config().return_once(|| {
            Err(sqlx::Error::Io(io::Error::other(
                "injected backup configuration read failure",
            )))
        });

        let error = BackgroundWorkers::start(
            background_worker_setup(&storage, Arc::new(site_config), Duration::from_secs(1)).await,
        )
        .await
        .err()
        .expect("a backup configuration read failure must stop worker startup");

        assert!(
            error
                .to_string()
                .contains("injected backup configuration read failure"),
            "startup error must retain the backup configuration failure: {error:#}"
        );
        assert!(
            error
                .chain()
                .any(|source| source.downcast_ref::<sqlx::Error>().is_some()),
            "the backup configuration source remains downcastable: {error:#}"
        );
    }

    #[tokio::test]
    async fn background_workers_roll_back_backup_and_maintenance_when_feed_start_fails() {
        let temp = TempDir::new().expect("temp dir");
        let storage = sqlite_storage_args(&temp);
        let factory = storage::open_database(&storage.db, &StorageRuntimeConfig::default())
            .await
            .expect("open db");
        let site_config = factory.site_config();
        let write_scope = factory.write_scope();
        let destination = temp.path().join("backups");
        let destination_for_config = destination.clone();
        let config_for_update = Arc::clone(&site_config);
        confirmed(
            write_scope
                .run(move |transaction| {
                    Box::pin(async move {
                        config_for_update
                            .set(
                                transaction,
                                SiteConfigKey::BackupDestinationPath,
                                destination_for_config.to_str().expect("utf-8 path"),
                            )
                            .await
                    })
                })
                .await
                .expect("configure backup destination"),
        );

        let error = BackgroundWorkers::start(
            background_worker_setup(&storage, site_config, Duration::ZERO).await,
        )
        .await
        .err()
        .expect("a zero feed interval must stop worker startup");

        assert_eq!(error.to_string(), "feed worker interval must be non-zero");
    }

    #[tokio::test]
    async fn background_workers_shutdown_all_configured_workers() {
        let temp = TempDir::new().expect("temp dir");
        let storage = sqlite_storage_args(&temp);
        let factory = storage::open_database(&storage.db, &StorageRuntimeConfig::default())
            .await
            .expect("open db");
        let site_config = factory.site_config();
        let write_scope = factory.write_scope();
        let destination = temp.path().join("backups");
        let destination_for_config = destination.clone();
        let config_for_update = Arc::clone(&site_config);
        confirmed(
            write_scope
                .run(move |transaction| {
                    Box::pin(async move {
                        config_for_update
                            .set(
                                transaction,
                                SiteConfigKey::BackupDestinationPath,
                                destination_for_config.to_str().expect("utf-8 path"),
                            )
                            .await
                    })
                })
                .await
                .expect("configure backup destination"),
        );
        let telemetry = test_telemetry(None);
        let bind: SocketAddr = "127.0.0.1:0".parse().expect("bind addr");
        let prepared = prepare_server(&storage, bind, false, &telemetry, None)
            .await
            .expect("prepare server");

        assert!(
            prepared.workers.backup.is_some(),
            "configured backup must start"
        );
        shutdown_prepared_server(prepared).await;
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
        let prepared = prepare_server(&storage, bind, false, &telemetry, None)
            .await
            .expect("dev-mode prepare_server must auto-initialize");

        assert!(db_path.exists(), "auto-init must have created the database");
        shutdown_prepared_server(prepared).await;
    }

    #[tokio::test]
    async fn prepare_server_registers_saturation_sampler_when_otel_endpoint_is_set() {
        let temp = TempDir::new().expect("temp dir");
        let storage = sqlite_storage_args(&temp);
        storage::open_database(&storage.db, &StorageRuntimeConfig::default())
            .await
            .expect("open db");
        let bind: SocketAddr = "127.0.0.1:0".parse().expect("bind addr");

        let telemetry = test_telemetry(Some("http://127.0.0.1:4318"));
        let prepared = prepare_server(&storage, bind, false, &telemetry, None)
            .await
            .expect("prepare server");

        assert!(prepared.saturation_metrics.is_some());
        shutdown_prepared_server(prepared).await;
    }

    #[tokio::test]
    async fn prepare_server_does_not_start_saturation_sampler_without_otel_endpoint() {
        let temp = TempDir::new().expect("temp dir");
        let storage = sqlite_storage_args(&temp);
        storage::open_database(&storage.db, &StorageRuntimeConfig::default())
            .await
            .expect("open db");
        let bind: SocketAddr = "127.0.0.1:0".parse().expect("bind addr");

        let telemetry = test_telemetry(None);
        let prepared = prepare_server(&storage, bind, false, &telemetry, None)
            .await
            .expect("prepare server");

        assert!(prepared.saturation_metrics.is_none());
        shutdown_prepared_server(prepared).await;
    }

    #[tokio::test]
    async fn prepare_server_cleans_temporary_uploads_before_returning_ready() {
        let temp = TempDir::new().expect("temp dir");
        let storage = sqlite_storage_args(&temp);
        storage::open_database(&storage.db, &StorageRuntimeConfig::default())
            .await
            .expect("open db");
        let tmp_dir = temp.path().join("media").join("tmp");
        fs::create_dir_all(tmp_dir.join("interrupted").join("upload"))
            .expect("create stale temporary directory");
        fs::write(
            tmp_dir.join("interrupted").join("upload").join("bytes"),
            b"stale",
        )
        .expect("write stale temporary upload");

        let bind: SocketAddr = "127.0.0.1:0".parse().expect("bind addr");
        let runtime_path = temp.path().join("runtime.json");
        let telemetry = test_telemetry(None);
        let prepared = prepare_server(&storage, bind, false, &telemetry, None)
            .await
            .expect("prepare server after temporary cleanup");

        assert!(
            fs::read_dir(&tmp_dir)
                .expect("read cleaned temporary directory")
                .next()
                .is_none(),
            "uploads may be accepted only after stale temporary artifacts are removed"
        );
        let runtime: serde_json::Value =
            serde_json::from_str(&fs::read_to_string(&runtime_path).expect("runtime file"))
                .expect("valid runtime file");
        let addr = prepared
            .listener
            .local_addr()
            .expect("bound listener address");
        assert_eq!(runtime["pid"], std::process::id());
        assert!(runtime["start_time"].as_u64().is_some());
        assert_eq!(runtime["port"], addr.port());
        shutdown_prepared_server(prepared).await;
        assert!(
            !runtime_path.exists(),
            "dropping the server removes its canonical runtime file"
        );
    }

    #[tokio::test]
    async fn prepare_server_surfaces_temporary_cleanup_failure_as_fatal() {
        let temp = TempDir::new().expect("temp dir");
        let storage = sqlite_storage_args(&temp);
        storage::open_database(&storage.db, &StorageRuntimeConfig::default())
            .await
            .expect("open db");
        let tmp_dir = temp.path().join("media").join("tmp");
        fs::create_dir_all(tmp_dir.parent().expect("temporary parent"))
            .expect("create media directory");
        fs::write(&tmp_dir, b"not a directory").expect("block temporary cleanup");

        let bind: SocketAddr = "127.0.0.1:0".parse().expect("bind addr");
        let telemetry = test_telemetry(None);
        let error = prepare_server(&storage, bind, false, &telemetry, None)
            .await
            .err()
            .expect("temporary cleanup failure must stop startup");

        assert_eq!(
            error.to_string(),
            "failed to prepare media temporary upload directory"
        );
        assert!(
            error.chain().any(|source| source
                .downcast_ref::<MediaTemporaryDirectoryError>()
                .is_some()),
            "fatal startup error must retain the typed cleanup source: {error:#}"
        );
    }

    #[tokio::test]
    async fn prepare_server_refuses_canonical_live_runtime_identity_before_cleanup() {
        let temp = TempDir::new().expect("temp dir");
        let db_path = temp.path().join("jaunder.db");
        let storage = sqlite_storage_args(&temp);
        let tmp_dir = temp.path().join("media").join("tmp");
        fs::create_dir_all(&tmp_dir).expect("create temporary directory");
        let stale_upload = tmp_dir.join("stale-upload");
        fs::write(&stale_upload, b"stale").expect("write stale upload");
        let runtime_path = temp.path().join("runtime.json");
        let start_time = runtime_file::require_start_time_at(Path::new("/proc/self/stat"))
            .expect("current process start time");
        let live_identity = serde_json::json!({
            "ip": "127.0.0.1",
            "port": 1,
            "pid": std::process::id(),
            "start_time": start_time,
        })
        .to_string();
        fs::write(&runtime_path, &live_identity).expect("write live canonical runtime file");

        let bind: SocketAddr = "127.0.0.1:0".parse().expect("bind addr");
        let telemetry = test_telemetry(None);
        let error = prepare_server(&storage, bind, false, &telemetry, None)
            .await
            .err()
            .expect("a live canonical runtime identity must refuse startup");

        assert!(
            error
                .to_string()
                .contains("another jaunder instance is already running"),
            "refusal must identify the live canonical owner: {error:#}"
        );
        assert!(
            !db_path.exists(),
            "must refuse before creating the database"
        );
        assert!(
            stale_upload.exists(),
            "a canonical live-identity refusal must occur before temporary cleanup"
        );
        assert_eq!(
            fs::read_to_string(&runtime_path).expect("read live canonical runtime file"),
            live_identity,
            "the live canonical identity must not be overwritten"
        );
    }

    #[tokio::test]
    async fn prepare_server_propagates_reservation_failure_before_cleanup() {
        let temp = TempDir::new().expect("temp dir");
        let db_path = temp.path().join("jaunder.db");
        let storage = sqlite_storage_args(&temp);
        let tmp_dir = temp.path().join("media").join("tmp");
        fs::create_dir_all(&tmp_dir).expect("create temporary directory");
        let stale_upload = tmp_dir.join("stale-upload");
        fs::write(&stale_upload, b"stale").expect("write stale upload");
        let runtime_path = temp.path().join("runtime.json");
        fs::create_dir(&runtime_path).expect("block canonical runtime reservation");

        let bind: SocketAddr = "127.0.0.1:0".parse().expect("bind addr");
        let telemetry = test_telemetry(None);
        let error = prepare_server(&storage, bind, false, &telemetry, None)
            .await
            .err()
            .expect("a canonical reservation failure must stop startup");

        assert!(
            error
                .to_string()
                .contains("cannot publish live runtime reservation"),
            "reservation failure must propagate with its publication context: {error:#}"
        );
        assert!(
            error
                .chain()
                .any(|source| source.downcast_ref::<std::io::Error>().is_some()),
            "reservation failure must retain its I/O source: {error:#}"
        );
        assert!(!db_path.exists(), "must fail before creating the database");
        assert!(
            stale_upload.exists(),
            "a reservation failure must occur before temporary cleanup"
        );
        assert!(
            runtime_path.is_dir(),
            "the blocking canonical runtime path must not be replaced"
        );
    }

    #[tokio::test]
    async fn prepare_server_refuses_on_live_lock_before_cleanup() {
        // Holding the OS-backed guard must refuse the contender before it can
        // delete temporary uploads or create the dev-mode database.
        let temp = TempDir::new().expect("temp dir");
        let db_path = temp.path().join("jaunder.db");
        let storage = sqlite_storage_args(&temp);
        let tmp_dir = temp.path().join("media").join("tmp");
        fs::create_dir_all(&tmp_dir).expect("create temporary directory");
        let stale_upload = tmp_dir.join("stale-upload");
        fs::write(&stale_upload, b"stale").expect("write stale upload");
        let _lock = StartupLockGuard::acquire(temp.path()).expect("hold startup lock");
        let bind: SocketAddr = "127.0.0.1:0".parse().expect("bind addr");
        let telemetry = test_telemetry(None);
        let err = prepare_server(&storage, bind, false, &telemetry, None)
            .await
            .err();
        assert!(
            err.is_some_and(|e| e.to_string().contains("exclusive startup lock")),
            "prepare_server must refuse when a live process holds the OS lock"
        );
        assert!(
            !db_path.exists(),
            "must refuse before creating the database"
        );
        assert!(
            stale_upload.exists(),
            "a live-instance refusal must occur before temporary cleanup"
        );
    }

    #[test]
    fn successful_worker_shutdown_preserves_serve_result() {
        let mut primary = Ok(());

        merge_worker_shutdown(&mut primary, Ok(()), "server.test.shutdown");

        assert!(primary.is_ok());
    }

    #[test]
    fn worker_shutdown_error_becomes_primary_after_successful_serve() {
        let mut primary = Ok(());
        merge_worker_shutdown(
            &mut primary,
            Err(anyhow::anyhow!("shutdown failed")),
            "server.test.shutdown",
        );
        assert!(
            primary
                .expect_err("shutdown failure must be returned")
                .to_string()
                .contains("server.test.shutdown")
        );
    }

    #[test]
    fn worker_shutdown_error_preserves_existing_serve_failure() {
        let mut primary = Err(anyhow::anyhow!("serve failed"));
        merge_worker_shutdown(
            &mut primary,
            Err(anyhow::anyhow!("shutdown failed")),
            "server.test.shutdown",
        );
        assert_eq!(
            primary
                .expect_err("serve failure must remain primary")
                .to_string(),
            "serve failed"
        );
    }

    #[test]
    fn first_sigint_starts_graceful_drain() {
        assert_eq!(
            shutdown_transition(ShutdownState::AwaitingSignal, ShutdownSignal::Sigint),
            ShutdownTransition::BeginGracefulDrain
        );
    }

    #[test]
    fn first_sigterm_starts_graceful_drain() {
        assert_eq!(
            shutdown_transition(ShutdownState::AwaitingSignal, ShutdownSignal::Sigterm),
            ShutdownTransition::BeginGracefulDrain
        );
    }

    #[test]
    fn repeated_sigterm_continues_graceful_drain() {
        assert_eq!(
            shutdown_transition(ShutdownState::Draining, ShutdownSignal::Sigterm),
            ShutdownTransition::ContinueGracefulDrain
        );
    }

    #[test]
    fn later_sigint_forces_exit_and_removes_runtime_identity() {
        assert_eq!(
            shutdown_transition(ShutdownState::Draining, ShutdownSignal::Sigint),
            ShutdownTransition::ForceExitAndRemoveRuntimeIdentity
        );
    }

    // The shutdown tests below raise a REAL signal to their own process. This is
    // safe only under `cargo nextest` (one process per test): installation is
    // synchronous, so the signal is delivered to this command's handler rather
    // than its default disposition. Bare `cargo test` shares one process, so
    // signal tests could observe each other's signals.
    #[cfg(unix)]
    async fn assert_signal_removes_runtime_file(signal: nix::sys::signal::Signal) {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("runtime.json");
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let guard = StartupLockGuard::acquire(dir.path())
            .expect("startup lock")
            .reserve(addr, 0)
            .expect("runtime reservation");
        assert!(path.exists(), "guard wrote the runtime file");

        // Installs the SIGINT/SIGTERM handlers synchronously, so the raise below
        // cannot beat handler installation.
        let (shutdown_rx, supervisor) = ShutdownSupervisor::install(guard.path().to_path_buf())
            .expect("install shutdown supervisor");
        let handle = tokio::spawn(serve_with_shutdown(
            listener,
            axum::Router::new(),
            async move {
                let _ = shutdown_rx.await;
            },
        ));

        nix::sys::signal::raise(signal).unwrap();

        handle
            .await
            .unwrap()
            .expect("serve_with_shutdown returns Ok on graceful shutdown");
        supervisor.abort_and_join().await;
        assert!(
            path.exists(),
            "serve completion must not release runtime ownership"
        );
        drop(guard);
        assert!(!path.exists(), "runtime.json removed after {signal:?}");
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn cmd_serve_handles_sigterm_immediately_after_ready_publication() {
        let temp = TempDir::new().expect("temp dir");
        let storage = sqlite_storage_args(&temp);
        storage::open_database(&storage.db, &StorageRuntimeConfig::default())
            .await
            .expect("initialize test database");
        let runtime_path = runtime_file::canonical_runtime_path(&storage.storage_path);
        let telemetry = test_telemetry(Some("http://127.0.0.1:4317"));
        let bind = "127.0.0.1:0".parse().expect("bind address");
        let mut command =
            tokio::spawn(async move { cmd_serve(&storage, bind, false, &telemetry, None).await });

        if tokio::time::timeout(Duration::from_secs(5), async {
            loop {
                let ready = fs::read(&runtime_path)
                    .ok()
                    .and_then(|contents| {
                        serde_json::from_slice::<serde_json::Value>(&contents).ok()
                    })
                    .and_then(|runtime| runtime["port"].as_u64())
                    .is_some_and(|port| port != 0);
                if ready {
                    break;
                }
                tokio::time::sleep(Duration::from_millis(1)).await;
            }
        })
        .await
        .is_err()
        {
            // cov:ignore-start
            command.abort();
            let _ = command.await;
            panic!("cmd_serve must publish a ready runtime identity");
            // cov:ignore-stop
        }

        nix::sys::signal::raise(nix::sys::signal::Signal::SIGTERM).expect("send SIGTERM");

        if let Ok(result) = tokio::time::timeout(Duration::from_secs(5), &mut command).await {
            result
                .expect("cmd_serve task must not panic")
                .expect("cmd_serve must gracefully shut down");
        } else {
            command.abort();
            let _ = command.await;
            panic!("cmd_serve must complete after SIGTERM");
        }
        assert!(
            !runtime_path.exists(),
            "completed cmd_serve must remove its runtime identity"
        );
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
