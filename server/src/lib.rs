// The ParentRoute wrapping all routes in web::App generates a wide tuple of
// route types; the compiler needs a higher recursion limit to monomorphize it,
// particularly under llvm-cov instrumentation. Root cause under investigation.
#![recursion_limit = "512"]

pub mod assets;
pub mod auth;
pub mod cli;
pub mod commands;
pub mod context;
pub mod mailer;
pub mod media;
pub mod observability;
pub mod password;
pub mod tag;
pub mod username;

use std::{
    fs,
    path::{Path, PathBuf},
    sync::Arc,
};

use axum::http::HeaderName;
use axum::Router;
use axum_embed::ServeEmbed;
use croner::Cron;
use leptos::prelude::*;
use leptos_axum::{generate_route_list, LeptosRoutes};
use opentelemetry::propagation::Extractor;
use tokio_cron_scheduler::{Job, JobScheduler};
use tower::ServiceBuilder;
use tower_http::request_id::{MakeRequestUuid, PropagateRequestIdLayer, SetRequestIdLayer};
use tower_http::trace::{DefaultOnResponse, TraceLayer};
use tracing::Level;
use tracing_opentelemetry::OpenTelemetrySpanExt;
use web::{shell, App};

use crate::assets::StaticAssets;
use ::storage::{
    AppState, SiteConfigStorage, BACKUP_DESTINATION_PATH_KEY, BACKUP_MODE_KEY,
    BACKUP_RETENTION_COUNT_KEY, BACKUP_SCHEDULE_KEY,
};
use storage::{export_backup, BackupExportOptions, BackupMode, DbConnectOptions};

pub fn create_router(
    leptos_options: LeptosOptions,
    state: Arc<AppState>,
    mailer: Arc<dyn common::mailer::MailSender>,
    secure_cookies: bool,
    storage_path: PathBuf,
) -> Router {
    let request_id_header = HeaderName::from_static("x-request-id");
    let http_observability = ServiceBuilder::new()
        .layer(axum::middleware::from_fn(extract_trace_context))
        .layer(SetRequestIdLayer::new(
            request_id_header.clone(),
            MakeRequestUuid,
        ))
        .layer(PropagateRequestIdLayer::new(request_id_header))
        .layer(
            TraceLayer::new_for_http()
                .make_span_with(|request: &axum::extract::Request| {
                    let span = tracing::span!(
                        Level::INFO,
                        "request",
                        method = %request.method(),
                        uri = %request.uri(),
                        version = ?request.version(),
                        headers = ?request.headers(),
                    );
                    if let Some(parent) = request.extensions().get::<ExtractedTraceContext>() {
                        span.set_parent(parent.0.clone());
                    }
                    span
                })
                .on_response(DefaultOnResponse::new().level(Level::INFO)),
        );

    let routes = generate_route_list(App);
    let extension_state = state.clone();
    let server_fn_state = state.clone();
    let server_fn_mailer = mailer.clone();
    let leptos_mailer = mailer;
    let serve_assets = ServeEmbed::<StaticAssets>::new();
    let storage_path_ext = Arc::new(storage_path);
    Router::new()
        .nest_service("/style", serve_assets)
        .route(
            "/media/upload",
            axum::routing::post(crate::media::upload_handler),
        )
        .route(
            "/media/{source}/{p1}/{p2}/{hash}/{filename}",
            axum::routing::get(crate::media::serve_handler),
        )
        .route(
            "/media/proxy",
            axum::routing::get(crate::media::proxy_handler),
        )
        .route(
            "/api/{*fn_name}",
            axum::routing::post(move |req: axum::extract::Request| {
                let state = server_fn_state.clone();
                let mailer = server_fn_mailer.clone();
                leptos_axum::handle_server_fns_with_context(
                    move || {
                        crate::context::provide_app_state_contexts(&state);
                        crate::context::provide_mailer_context(&mailer);
                        provide_context(web::auth::CookieSettings {
                            secure: secure_cookies,
                        });
                    },
                    req,
                )
            }),
        )
        .leptos_routes_with_context(
            &leptos_options,
            routes,
            move || {
                crate::context::provide_app_state_contexts(&state);
                crate::context::provide_mailer_context(&leptos_mailer);
                provide_context(web::auth::CookieSettings {
                    secure: secure_cookies,
                });
            },
            {
                let leptos_options = leptos_options.clone();
                move || shell(leptos_options.clone())
            },
        )
        .fallback(leptos_axum::file_and_error_handler(shell))
        .layer(axum::Extension(storage_path_ext))
        .layer(axum::Extension(extension_state))
        .layer(http_observability)
        .with_state(leptos_options)
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct BackupWorkerConfig {
    destination_path: Option<PathBuf>,
    schedule: String,
    retention_count: usize,
    mode: BackupMode,
    invalid_keys: Vec<&'static str>,
}

impl BackupWorkerConfig {
    async fn load(site_config: &dyn SiteConfigStorage) -> anyhow::Result<Self> {
        let mut invalid_keys = Vec::new();
        let destination_path = site_config
            .get(BACKUP_DESTINATION_PATH_KEY)
            .await?
            .and_then(|path| non_empty_path(&path));
        let schedule = match site_config.get(BACKUP_SCHEDULE_KEY).await? {
            Some(value) if backup_schedule_valid(value.trim()) => value.trim().to_owned(),
            Some(_) => {
                invalid_keys.push(BACKUP_SCHEDULE_KEY);
                default_backup_schedule()
            }
            None => default_backup_schedule(),
        };
        let retention_count = match site_config.get(BACKUP_RETENTION_COUNT_KEY).await? {
            Some(value) => {
                if let Ok(value) = value.parse::<usize>() {
                    value
                } else {
                    invalid_keys.push(BACKUP_RETENTION_COUNT_KEY);
                    default_backup_retention_count()
                }
            }
            None => default_backup_retention_count(),
        };
        let mode = match site_config.get(BACKUP_MODE_KEY).await? {
            Some(value) => {
                if let Some(mode) = parse_backup_mode(&value) {
                    mode
                } else {
                    invalid_keys.push(BACKUP_MODE_KEY);
                    default_backup_mode()
                }
            }
            None => default_backup_mode(),
        };

        Ok(Self {
            destination_path,
            schedule,
            retention_count,
            mode,
            invalid_keys,
        })
    }

    fn has_invalid_values(&self) -> bool {
        !self.invalid_keys.is_empty()
    }
}

/// Starts the background backup worker if configured.
///
/// # Errors
///
/// Returns an error if the site configuration cannot be loaded, or if the
/// job scheduler fails to start.
pub async fn start_backup_worker(
    state: Arc<AppState>,
    database: DbConnectOptions,
    storage_path: PathBuf,
) -> anyhow::Result<Option<JobScheduler>> {
    let config = BackupWorkerConfig::load(state.site_config.as_ref()).await?;
    let Some(destination_root) = config.destination_path.clone() else {
        tracing::warn!("backup worker disabled: backup.destination_path is not configured");
        return Ok(None);
    };
    if config.has_invalid_values() {
        tracing::error!(
            invalid_keys = ?config.invalid_keys,
            "scheduled backup worker disabled: backup configuration is invalid and needs urgent operator attention"
        );
        return Ok(None);
    }

    let scheduler = JobScheduler::new().await?;
    let schedule = config.schedule.clone();
    let job = Job::new_async(schedule.as_str(), move |_uuid, _lock| {
        let database = database.clone();
        let media_path = storage_path.join("media");
        let destination_root = destination_root.clone();
        let config = config.clone();
        Box::pin(async move {
            if let Err(error) =
                run_scheduled_backup(&database, &media_path, &destination_root, &config).await
            {
                tracing::error!(error = %error, "scheduled backup failed");
            }
        })
    })?;
    scheduler.add(job).await?;
    scheduler.start().await?;
    Ok(Some(scheduler))
}

async fn run_scheduled_backup(
    database: &DbConnectOptions,
    media_path: &Path,
    destination_root: &Path,
    config: &BackupWorkerConfig,
) -> anyhow::Result<PathBuf> {
    fs::create_dir_all(destination_root)?;
    let destination_path = backup_path_for_mode(destination_root, config.mode);
    export_backup(BackupExportOptions {
        database,
        media_path,
        destination_path: &destination_path,
        mode: config.mode,
    })
    .await?;
    prune_backups(destination_root, config.retention_count)?;
    tracing::info!(path = %destination_path.display(), "scheduled backup complete");
    Ok(destination_path)
}

fn prune_backups(destination_root: &Path, retention_count: usize) -> std::io::Result<()> {
    let mut backups = Vec::new();
    if !destination_root.exists() {
        return Ok(());
    }
    for entry in fs::read_dir(destination_root)? {
        let entry = entry?;
        let path = entry.path();
        if path.join("manifest.json").is_file()
            || path
                .file_name()
                .and_then(|name| name.to_str())
                .is_some_and(|name| name.ends_with(".tar.gz"))
        {
            backups.push(path);
        }
    }
    backups.sort();
    let prune_count = backups.len().saturating_sub(retention_count);
    for path in backups.into_iter().take(prune_count) {
        if path.is_dir() {
            fs::remove_dir_all(path)?;
        } else {
            fs::remove_file(path)?;
        }
    }
    Ok(())
}

fn timestamped_backup_name() -> String {
    format!("backup-{}", chrono::Utc::now().format("%Y%m%dT%H%M%SZ"))
}

fn backup_path_for_mode(destination_root: &Path, mode: BackupMode) -> PathBuf {
    let name = timestamped_backup_name();
    match mode {
        BackupMode::Directory => destination_root.join(name),
        BackupMode::Archive => destination_root.join(format!("{name}.tar.gz")),
    }
}

fn default_backup_schedule() -> String {
    "0 0 0 * * *".to_owned()
}

fn default_backup_retention_count() -> usize {
    7
}

fn default_backup_mode() -> BackupMode {
    BackupMode::Directory
}

fn backup_schedule_valid(schedule: &str) -> bool {
    Cron::new(schedule).with_seconds_required().parse().is_ok()
}

fn parse_backup_mode(value: &str) -> Option<BackupMode> {
    match value.trim() {
        "directory" => Some(BackupMode::Directory),
        "archive" => Some(BackupMode::Archive),
        _ => None,
    }
}

fn non_empty_path(value: &str) -> Option<PathBuf> {
    let value = value.trim();
    if value.is_empty() {
        None
    } else {
        Some(PathBuf::from(value))
    }
}

#[derive(Clone)]
struct ExtractedTraceContext(opentelemetry::Context);

struct HeaderExtractor<'a>(&'a axum::http::HeaderMap);

impl Extractor for HeaderExtractor<'_> {
    fn get(&self, key: &str) -> Option<&str> {
        self.0.get(key).and_then(|value| value.to_str().ok())
    }

    fn keys(&self) -> Vec<&str> {
        self.0.keys().map(axum::http::HeaderName::as_str).collect()
    }
}

async fn extract_trace_context(
    mut request: axum::extract::Request,
    next: axum::middleware::Next,
) -> axum::response::Response {
    let context = opentelemetry::global::get_text_map_propagator(|propagator| {
        propagator.extract(&HeaderExtractor(request.headers()))
    });
    request
        .extensions_mut()
        .insert(ExtractedTraceContext(context));
    next.run(request).await
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::{
        body::Body,
        http::{HeaderMap, Request, StatusCode},
    };
    use leptos::prelude::LeptosOptions;
    use tempfile::TempDir;
    use tower::ServiceExt;

    fn ensure_server_fns_registered() {
        server_fn::axum::register_explicit::<web::auth::CurrentUser>();
        server_fn::axum::register_explicit::<web::backup::BackupWarningVisible>();
        server_fn::axum::register_explicit::<web::auth::GetRegistrationPolicy>();
        server_fn::axum::register_explicit::<web::auth::Register>();
        server_fn::axum::register_explicit::<web::auth::Login>();
        server_fn::axum::register_explicit::<web::auth::Logout>();
    }

    fn test_options() -> LeptosOptions {
        LeptosOptions::builder().output_name("test").build()
    }

    fn test_storage_path() -> PathBuf {
        // Return a non-existent path; media routes are not exercised by lib.rs tests.
        PathBuf::from("/tmp/jaunder-test-storage")
    }

    async fn test_state() -> Arc<AppState> {
        storage::open_database(&"sqlite::memory:".parse().unwrap())
            .await
            .unwrap()
    }

    fn test_mailer() -> Arc<dyn common::mailer::MailSender> {
        Arc::new(common::mailer::NoopMailSender)
    }

    #[tokio::test]
    async fn backup_worker_disabled_without_destination_path() {
        let state = test_state().await;
        let storage = TempDir::new().expect("temp dir");
        let scheduler = start_backup_worker(
            state,
            "sqlite::memory:".parse().expect("sqlite options"),
            storage.path().to_path_buf(),
        )
        .await
        .expect("worker start");

        assert!(scheduler.is_none());
    }

    #[tokio::test]
    async fn backup_worker_starts_when_destination_is_configured() {
        let state = test_state().await;
        let storage = TempDir::new().expect("temp dir");
        state
            .site_config
            .set(
                BACKUP_DESTINATION_PATH_KEY,
                storage.path().join("backups").to_str().expect("utf-8 path"),
            )
            .await
            .expect("set destination");
        state
            .site_config
            .set(BACKUP_SCHEDULE_KEY, "0 0 0 1 1 *")
            .await
            .expect("set schedule");

        let scheduler = start_backup_worker(
            state,
            "sqlite::memory:".parse().expect("sqlite options"),
            storage.path().to_path_buf(),
        )
        .await
        .expect("worker start");

        assert!(scheduler.is_some());
    }

    #[tokio::test]
    async fn backup_worker_executes_scheduled_backup() {
        let temp = TempDir::new().expect("temp dir");
        let db_options: DbConnectOptions =
            format!("sqlite:{}", temp.path().join("jaunder.db").display())
                .parse()
                .expect("db options");
        let state = storage::open_database(&db_options).await.expect("open db");
        let storage_path = temp.path().join("storage");
        let media_path = storage_path.join("media");
        std::fs::create_dir_all(&media_path).expect("media dir");
        std::fs::write(media_path.join("file.txt"), "media").expect("media file");
        let destination_path = temp.path().join("scheduled-backups");
        state
            .site_config
            .set(
                BACKUP_DESTINATION_PATH_KEY,
                destination_path.to_str().expect("utf-8 path"),
            )
            .await
            .expect("set destination");
        state
            .site_config
            .set(BACKUP_SCHEDULE_KEY, "*/1 * * * * *")
            .await
            .expect("set schedule");

        let mut scheduler = start_backup_worker(state, db_options, storage_path)
            .await
            .expect("worker start")
            .expect("scheduler enabled");

        let mut found_manifest = false;
        for _ in 0..30 {
            found_manifest = std::fs::read_dir(&destination_path)
                .ok()
                .into_iter()
                .flatten()
                .filter_map(Result::ok)
                .any(|entry| entry.path().join("manifest.json").is_file());
            if found_manifest {
                break;
            }
            tokio::time::sleep(std::time::Duration::from_millis(100)).await;
        }
        scheduler.shutdown().await.expect("shutdown scheduler");

        assert!(found_manifest, "scheduled backup did not run");
    }

    #[tokio::test]
    async fn backup_worker_config_loads_site_config_values() {
        let state = test_state().await;
        state
            .site_config
            .set(BACKUP_DESTINATION_PATH_KEY, "/tmp/jaunder-backups")
            .await
            .expect("set destination");
        state
            .site_config
            .set(BACKUP_SCHEDULE_KEY, "0 15 2 * * *")
            .await
            .expect("set schedule");
        state
            .site_config
            .set(BACKUP_RETENTION_COUNT_KEY, "3")
            .await
            .expect("set retention");
        state
            .site_config
            .set(BACKUP_MODE_KEY, "directory")
            .await
            .expect("set mode");

        let config = BackupWorkerConfig::load(state.site_config.as_ref())
            .await
            .expect("load config");

        assert_eq!(
            config.destination_path,
            Some(PathBuf::from("/tmp/jaunder-backups"))
        );
        assert_eq!(config.schedule, "0 15 2 * * *");
        assert_eq!(config.retention_count, 3);
        assert_eq!(config.mode, BackupMode::Directory);
    }

    #[tokio::test]
    async fn backup_worker_config_accepts_archive_mode() {
        let state = test_state().await;
        state
            .site_config
            .set(BACKUP_MODE_KEY, "archive")
            .await
            .expect("set mode");

        let config = BackupWorkerConfig::load(state.site_config.as_ref())
            .await
            .expect("load config");

        assert_eq!(config.mode, BackupMode::Archive);
    }

    #[tokio::test]
    async fn backup_worker_config_rejects_unknown_mode() {
        let state = test_state().await;
        state
            .site_config
            .set(BACKUP_MODE_KEY, "surprise")
            .await
            .expect("set mode");

        let config = BackupWorkerConfig::load(state.site_config.as_ref())
            .await
            .expect("load config");

        assert_eq!(config.mode, BackupMode::Directory);
        assert_eq!(config.invalid_keys, vec![BACKUP_MODE_KEY]);
    }

    #[tokio::test]
    async fn backup_worker_disabled_when_schedule_is_invalid() {
        let state = test_state().await;
        let storage = TempDir::new().expect("temp dir");
        state
            .site_config
            .set(
                BACKUP_DESTINATION_PATH_KEY,
                storage.path().join("backups").to_str().expect("utf-8 path"),
            )
            .await
            .expect("set destination");
        state
            .site_config
            .set(BACKUP_SCHEDULE_KEY, "not-a-schedule")
            .await
            .expect("set schedule");

        let scheduler = start_backup_worker(
            state,
            "sqlite::memory:".parse().expect("sqlite options"),
            storage.path().to_path_buf(),
        )
        .await
        .expect("worker start");

        assert!(scheduler.is_none());
    }

    #[tokio::test]
    async fn run_scheduled_backup_writes_backup_and_prunes_old_ones() {
        let temp = TempDir::new().expect("temp dir");
        let db_url = format!("sqlite:{}", temp.path().join("jaunder.db").display());
        storage::open_database(&db_url.parse().expect("db options"))
            .await
            .expect("open db");

        let media = temp.path().join("media");
        std::fs::create_dir(&media).expect("media dir");
        std::fs::write(media.join("file.txt"), "media").expect("media file");

        let destination_root = temp.path().join("backups");
        for name in ["backup-0001", "backup-0002"] {
            let backup = destination_root.join(name);
            std::fs::create_dir_all(&backup).expect("old backup dir");
            std::fs::write(backup.join("manifest.json"), "{}").expect("manifest");
        }

        let config = BackupWorkerConfig {
            destination_path: Some(destination_root.clone()),
            schedule: "0 0 0 1 1 *".to_owned(),
            retention_count: 1,
            mode: BackupMode::Directory,
            invalid_keys: Vec::new(),
        };
        let written = run_scheduled_backup(
            &db_url.parse().expect("db options"),
            &media,
            &destination_root,
            &config,
        )
        .await
        .expect("scheduled backup");

        assert!(written.join("manifest.json").is_file());
        assert!(written.join("media").join("file.txt").is_file());
        assert!(!destination_root.join("backup-0001").exists());
        assert!(!destination_root.join("backup-0002").exists());
    }

    #[test]
    fn prune_backups_keeps_newest_manifest_directories() {
        let temp = TempDir::new().expect("temp dir");
        for name in ["backup-1", "backup-2", "backup-3"] {
            let path = temp.path().join(name);
            std::fs::create_dir(&path).expect("backup dir");
            std::fs::write(path.join("manifest.json"), "{}").expect("manifest");
        }
        let ignored = temp.path().join("not-a-backup");
        std::fs::create_dir(&ignored).expect("ignored dir");

        prune_backups(temp.path(), 2).expect("prune");

        assert!(!temp.path().join("backup-1").exists());
        assert!(temp.path().join("backup-2").exists());
        assert!(temp.path().join("backup-3").exists());
        assert!(ignored.exists());
    }

    #[test]
    fn prune_backups_keeps_newest_archives() {
        let temp = TempDir::new().expect("temp dir");
        for name in ["backup-1.tar.gz", "backup-2.tar.gz", "backup-3.tar.gz"] {
            std::fs::write(temp.path().join(name), "archive").expect("archive");
        }

        prune_backups(temp.path(), 2).expect("prune");

        assert!(!temp.path().join("backup-1.tar.gz").exists());
        assert!(temp.path().join("backup-2.tar.gz").exists());
        assert!(temp.path().join("backup-3.tar.gz").exists());
    }

    #[test]
    fn prune_backups_accepts_missing_destination_root() {
        let temp = TempDir::new().expect("temp dir");
        prune_backups(&temp.path().join("missing"), 1).expect("prune missing root");
    }

    #[test]
    fn backup_path_helpers_parse_empty_and_nonempty_values() {
        assert_eq!(non_empty_path("   "), None);
        assert_eq!(
            non_empty_path(" /tmp/backups "),
            Some(PathBuf::from("/tmp/backups"))
        );
        assert_eq!(parse_backup_mode(""), None);
        assert_eq!(parse_backup_mode("directory"), Some(BackupMode::Directory));
        assert_eq!(parse_backup_mode("archive"), Some(BackupMode::Archive));
    }

    #[tokio::test]
    async fn home_route_returns_ok() {
        let local = tokio::task::LocalSet::new();
        local
            .run_until(async {
                ensure_server_fns_registered();
                let app = create_router(
                    test_options(),
                    test_state().await,
                    test_mailer(),
                    true,
                    test_storage_path(),
                );
                let response = app
                    .oneshot(Request::builder().uri("/").body(Body::empty()).unwrap())
                    .await
                    .unwrap();
                assert_eq!(response.status(), StatusCode::OK);
            })
            .await;
    }

    #[tokio::test]
    async fn profile_route_returns_ok() {
        let local = tokio::task::LocalSet::new();
        local
            .run_until(async {
                ensure_server_fns_registered();
                let app = create_router(
                    test_options(),
                    test_state().await,
                    test_mailer(),
                    true,
                    test_storage_path(),
                );
                let response = app
                    .oneshot(
                        Request::builder()
                            .uri("/profile")
                            .body(Body::empty())
                            .expect("failed to build request"),
                    )
                    .await
                    .expect("failed to get response");
                assert_eq!(response.status(), StatusCode::OK);
            })
            .await;
    }

    #[tokio::test]
    async fn sessions_route_returns_ok() {
        let local = tokio::task::LocalSet::new();
        local
            .run_until(async {
                ensure_server_fns_registered();
                let app = create_router(
                    test_options(),
                    test_state().await,
                    test_mailer(),
                    true,
                    test_storage_path(),
                );
                let response = app
                    .oneshot(
                        Request::builder()
                            .uri("/sessions")
                            .body(Body::empty())
                            .expect("failed to build request"),
                    )
                    .await
                    .expect("failed to get response");
                assert_eq!(response.status(), StatusCode::OK);
            })
            .await;
    }

    #[tokio::test]
    async fn create_post_route_returns_ok() {
        let local = tokio::task::LocalSet::new();
        local
            .run_until(async {
                ensure_server_fns_registered();
                let app = create_router(
                    test_options(),
                    test_state().await,
                    test_mailer(),
                    true,
                    test_storage_path(),
                );
                let response = app
                    .oneshot(
                        Request::builder()
                            .uri("/posts/new")
                            .body(Body::empty())
                            .expect("failed to build request"),
                    )
                    .await
                    .expect("failed to get response");
                assert_eq!(response.status(), StatusCode::OK);
            })
            .await;
    }

    #[tokio::test]
    async fn register_route_returns_ok() {
        let local = tokio::task::LocalSet::new();
        local
            .run_until(async {
                ensure_server_fns_registered();
                let app = create_router(
                    test_options(),
                    test_state().await,
                    test_mailer(),
                    true,
                    test_storage_path(),
                );
                let response = app
                    .oneshot(
                        Request::builder()
                            .uri("/register")
                            .body(Body::empty())
                            .expect("failed to build request"),
                    )
                    .await
                    .expect("failed to get response");
                assert_eq!(response.status(), StatusCode::OK);
                let body = axum::body::to_bytes(response.into_body(), usize::MAX)
                    .await
                    .unwrap();
                let html = String::from_utf8(body.to_vec()).unwrap();
                assert!(html.contains("Register"), "body: {html}");
            })
            .await;
    }

    #[tokio::test]
    async fn login_route_returns_ok() {
        let local = tokio::task::LocalSet::new();
        local
            .run_until(async {
                ensure_server_fns_registered();
                let app = create_router(
                    test_options(),
                    test_state().await,
                    test_mailer(),
                    true,
                    test_storage_path(),
                );
                let response = app
                    .oneshot(
                        Request::builder()
                            .uri("/login")
                            .body(Body::empty())
                            .expect("failed to build request"),
                    )
                    .await
                    .expect("failed to get response");
                assert_eq!(response.status(), StatusCode::OK);
                let body = axum::body::to_bytes(response.into_body(), usize::MAX)
                    .await
                    .unwrap();
                let html = String::from_utf8(body.to_vec()).unwrap();
                assert!(html.contains("Login"), "body: {html}");
            })
            .await;
    }

    #[tokio::test]
    async fn logout_route_returns_ok() {
        ensure_server_fns_registered();
        let local = tokio::task::LocalSet::new();
        local
            .run_until(async {
                let app = create_router(
                    test_options(),
                    test_state().await,
                    test_mailer(),
                    true,
                    test_storage_path(),
                );
                let response = app
                    .oneshot(
                        Request::builder()
                            .uri("/logout")
                            .body(Body::empty())
                            .expect("failed to build request"),
                    )
                    .await
                    .expect("failed to get response");
                assert_eq!(response.status(), StatusCode::OK);
                let body = axum::body::to_bytes(response.into_body(), usize::MAX)
                    .await
                    .unwrap();
                let html = String::from_utf8(body.to_vec()).unwrap();
                assert!(html.contains("Logging out"), "body: {html}");
            })
            .await;
    }

    #[tokio::test]
    async fn register_route_with_invite_only_policy_returns_ok() {
        let local = tokio::task::LocalSet::new();
        local
            .run_until(async {
                ensure_server_fns_registered();
                let state = test_state().await;
                state
                    .site_config
                    .set("site.registration_policy", "invite_only")
                    .await
                    .unwrap();
                let app = create_router(
                    test_options(),
                    state,
                    test_mailer(),
                    true,
                    test_storage_path(),
                );
                let response = app
                    .oneshot(
                        Request::builder()
                            .uri("/register")
                            .body(Body::empty())
                            .expect("failed to build request"),
                    )
                    .await
                    .expect("failed to get response");
                assert_eq!(response.status(), StatusCode::OK);
                let body = axum::body::to_bytes(response.into_body(), usize::MAX)
                    .await
                    .unwrap();
                let html = String::from_utf8(body.to_vec()).unwrap();
                assert!(html.contains("Invite code"), "body: {html}");
            })
            .await;
    }

    #[tokio::test]
    async fn invites_route_returns_not_found_when_policy_is_closed() {
        let local = tokio::task::LocalSet::new();
        local
            .run_until(async {
                ensure_server_fns_registered();
                let app = create_router(
                    test_options(),
                    test_state().await,
                    test_mailer(),
                    true,
                    test_storage_path(),
                );
                let response = app
                    .oneshot(
                        Request::builder()
                            .uri("/invites")
                            .body(Body::empty())
                            .expect("failed to build request"),
                    )
                    .await
                    .expect("failed to get response");
                let status = response.status();
                assert!(
                    status == StatusCode::OK || status == StatusCode::NOT_FOUND,
                    "expected 200 or 404, got {status}"
                );
                let body = axum::body::to_bytes(response.into_body(), usize::MAX)
                    .await
                    .expect("failed to read body");
                let html = String::from_utf8(body.to_vec()).expect("body is not valid UTF-8");
                assert!(html.contains("Page not found."), "body: {html}");
            })
            .await;
    }

    #[tokio::test]
    async fn home_response_contains_app_content() {
        let local = tokio::task::LocalSet::new();
        local
            .run_until(async {
                let app = create_router(
                    test_options(),
                    test_state().await,
                    test_mailer(),
                    true,
                    test_storage_path(),
                );
                let response = app
                    .oneshot(Request::builder().uri("/").body(Body::empty()).unwrap())
                    .await
                    .unwrap();
                let body = axum::body::to_bytes(response.into_body(), usize::MAX)
                    .await
                    .unwrap();
                let html = String::from_utf8(body.to_vec()).unwrap();
                assert!(html.contains("Jaunder"));
            })
            .await;
    }

    #[test]
    fn header_extractor_reads_known_headers() {
        let mut headers = HeaderMap::new();
        headers.insert(
            "traceparent",
            "00-4bf92f3577b34da6a3ce929d0e0e4736-00f067aa0ba902b7-01"
                .parse()
                .expect("valid traceparent header"),
        );

        let extractor = HeaderExtractor(&headers);
        assert_eq!(
            extractor.get("traceparent"),
            Some("00-4bf92f3577b34da6a3ce929d0e0e4736-00f067aa0ba902b7-01")
        );
        assert!(extractor.keys().contains(&"traceparent"));
    }

    #[tokio::test]
    async fn trace_context_middleware_inserts_extension() {
        let app = Router::new()
            .route(
                "/",
                axum::routing::get(|req: axum::extract::Request| async move {
                    if req.extensions().get::<ExtractedTraceContext>().is_some() {
                        StatusCode::OK
                    } else {
                        StatusCode::INTERNAL_SERVER_ERROR
                    }
                }),
            )
            .layer(axum::middleware::from_fn(extract_trace_context));

        let response = app
            .oneshot(
                Request::builder()
                    .uri("/")
                    .header(
                        "traceparent",
                        "00-4bf92f3577b34da6a3ce929d0e0e4736-00f067aa0ba902b7-01",
                    )
                    .body(Body::empty())
                    .expect("failed to build request"),
            )
            .await
            .expect("failed to get response");

        assert_eq!(response.status(), StatusCode::OK);
    }

    #[tokio::test]
    async fn current_user_api_route_returns_ok() {
        let local = tokio::task::LocalSet::new();
        local
            .run_until(async {
                ensure_server_fns_registered();
                let app = create_router(
                    test_options(),
                    test_state().await,
                    test_mailer(),
                    true,
                    test_storage_path(),
                );
                let response = app
                    .oneshot(
                        Request::builder()
                            .method("POST")
                            .uri("/api/current_user")
                            .header("content-type", "application/x-www-form-urlencoded")
                            .header(
                                "traceparent",
                                "00-4bf92f3577b34da6a3ce929d0e0e4736-00f067aa0ba902b7-01",
                            )
                            .body(Body::empty())
                            .expect("failed to build request"),
                    )
                    .await
                    .expect("failed to get response");

                assert_eq!(response.status(), StatusCode::OK);
            })
            .await;
    }
}
