// The ParentRoute wrapping all routes in web::App generates a wide tuple of
// route types; the compiler needs a higher recursion limit to monomorphize it,
// particularly under llvm-cov instrumentation. Root cause under investigation.
#![recursion_limit = "512"]

pub mod assets;
pub mod atompub;
pub mod cli;
pub mod commands;
pub mod context;
pub mod feed;
pub mod mailer;
pub mod media;
pub mod media_manager;
pub mod observability;

use std::{
    fs,
    path::{Path, PathBuf},
    sync::Arc,
};

use axum::http::HeaderName;
use axum::Router;
use axum_embed::ServeEmbed;
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
use ::storage::AppState;
use common::backup::BackupConfig;
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
        .merge(crate::media::router())
        .merge(crate::atompub::router())
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
        .route(
            "/feed.{ext}",
            axum::routing::get(crate::feed::handlers::feed_site),
        )
        .route(
            "/tags/{tag}/feed.{ext}",
            axum::routing::get(crate::feed::handlers::feed_site_tag),
        )
        .route(
            "/~{username}/feed.{ext}",
            axum::routing::get(crate::feed::handlers::feed_user),
        )
        .route(
            "/~{username}/tags/{tag}/feed.{ext}",
            axum::routing::get(crate::feed::handlers::feed_user_tag),
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
    let config = state.site_config.get_backup_config().await?;
    let Some(destination_root) = config.destination_path.as_deref().map(PathBuf::from) else {
        tracing::warn!("backup worker disabled: backup.destination_path is not configured");
        return Ok(None);
    };

    let scheduler = JobScheduler::new().await?;
    let schedule = config.schedule.as_str().to_owned();
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
    config: &BackupConfig,
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
    use storage::{BACKUP_DESTINATION_PATH_KEY, BACKUP_SCHEDULE_KEY};
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

    #[test]
    fn timestamped_backup_name_has_expected_format() {
        let name = timestamped_backup_name();
        assert!(
            name.starts_with("backup-"),
            "name must start with 'backup-', got: {name}"
        );
        let suffix = name.strip_prefix("backup-").unwrap();
        // Format: YYYYMMDDTHHMMSSz (16 chars)
        assert_eq!(
            suffix.len(),
            16,
            "timestamp suffix must be 16 chars, got: {suffix}"
        );
        assert!(suffix.ends_with('Z'), "timestamp must end with 'Z'");
        assert!(suffix.contains('T'), "timestamp must contain 'T'");
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

        let config = BackupConfig {
            destination_path: Some(destination_root.to_string_lossy().into_owned()),
            schedule: common::backup::BackupSchedule::parse("0 0 0 1 1 *").expect("valid schedule"),
            retention_count: 1,
            mode: BackupMode::Directory,
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
    fn backup_path_for_mode_returns_tar_gz_for_archive_mode() {
        let root = std::path::Path::new("/backups");
        let path = backup_path_for_mode(root, BackupMode::Archive);
        let name = path.file_name().unwrap().to_string_lossy();
        assert!(
            name.ends_with(".tar.gz"),
            "expected .tar.gz extension, got: {name}"
        );
        assert!(
            name.starts_with("backup-"),
            "expected backup- prefix, got: {name}"
        );
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
