pub mod assets;
pub mod atompub;
pub mod backup;
pub mod cli;
pub mod client_telemetry;
pub mod commands;
pub mod context;
pub mod feed;
pub mod mailer;
mod maintenance;
pub mod media;
pub mod media_ownership;
pub mod metrics;
pub mod observability;
pub mod projector;
pub mod publisher;
pub mod runtime_file;
mod scheduled_worker;
mod server_fn_response;
pub mod site;
mod soft_path;
pub mod websub;

#[cfg(test)]
mod test_support;

use std::{path::PathBuf, sync::Arc};

use axum::{
    Router,
    http::{HeaderName, HeaderValue},
    routing,
};
use axum_embed::ServeEmbed;
use common::mailer::MailSender;
use leptos::prelude::*;

use crate::{
    assets::StaticAssets, feed::handlers, media_ownership::LiveMediaReferenceOwnershipResolver,
    publisher::PublisherService,
};
use ::storage::{
    AppState, InstanceId, MediaContentLocks, MediaManager, MediaReferenceOwnershipResolver,
    PasswordResetStorage, SessionStorage, SiteConfigStorage, UserStorage, WriteScope,
};

#[derive(Clone)]
struct PasswordResetRequestDependencies {
    users: Arc<dyn UserStorage>,
    password_resets: Arc<dyn PasswordResetStorage>,
    write_scope: WriteScope,
    site_config: Arc<dyn SiteConfigStorage>,
    mailer: Arc<dyn MailSender>,
}

async fn retire_session_cookie(
    axum::extract::State(secure): axum::extract::State<bool>,
    mut request: axum::extract::Request,
    next: axum::middleware::Next,
) -> axum::response::Response {
    let retirement = web::auth::SessionCookieRetirement::default();
    request.extensions_mut().insert(retirement.clone());
    let mut response = next.run(request).await;

    if retirement.requested() {
        let Ok(value) = host::auth::clear_session_cookie_header(secure).parse() else {
            unreachable!("generated session cookie header must be valid");
        };
        response
            .headers_mut()
            .append(axum::http::header::SET_COOKIE, value);
    }

    response
}

const INSTANCE_HEADER: HeaderName = HeaderName::from_static("x-jaunder-instance");

async fn set_instance_header(
    axum::extract::State(instance_id): axum::extract::State<HeaderValue>,
    request: axum::extract::Request,
    next: axum::middleware::Next,
) -> axum::response::Response {
    let mut response = next.run(request).await;
    response.headers_mut().insert(INSTANCE_HEADER, instance_id);
    response
}

/// Builds the production router with the live foreign-reference ownership resolver.
///
/// # Errors
///
/// Returns an error when the persisted instance identity cannot form an HTTP header.
pub fn create_router(
    state: Arc<AppState>,
    instance_id: InstanceId,
    mailer: Arc<dyn MailSender>,
    secure_cookies: bool,
    storage_path: PathBuf,
) -> Result<Router, axum::http::header::InvalidHeaderValue> {
    create_router_with_media_reference_ownership_resolver(
        state,
        instance_id,
        mailer,
        secure_cookies,
        storage_path,
        Arc::new(LiveMediaReferenceOwnershipResolver::new()),
    )
}

fn build_application_routes<F>(
    sessions: Arc<dyn SessionStorage>,
    write_scope: WriteScope,
    provide_server_function_contexts: F,
) -> Router
where
    F: Fn() + Clone + Send + Sync + 'static,
{
    let client_telemetry = client_telemetry::router(
        sessions,
        write_scope,
        Arc::new(client_telemetry::ClientTelemetryLimiter::new()),
    );

    Router::new()
        .nest_service("/style", ServeEmbed::<StaticAssets>::new())
        .merge(crate::media::router())
        .merge(crate::atompub::router())
        .merge(client_telemetry)
        .route(
            "/api/{*fn_name}",
            routing::post(move |req: axum::extract::Request| {
                let provide_server_function_contexts = provide_server_function_contexts.clone();
                server_fn_response::handle_with_context(provide_server_function_contexts, req)
            }),
        )
        .route("/feed.{ext}", routing::get(handlers::feed_site))
        .route(
            "/tags/{tag}/feed.{ext}",
            routing::get(handlers::feed_site_tag),
        )
        .route("/~{username}/feed.{ext}", routing::get(handlers::feed_user))
        .route(
            "/~{username}/tags/{tag}/feed.{ext}",
            routing::get(handlers::feed_user_tag),
        )
}

/// Builds the production-shaped router with an injected foreign-reference
/// ownership resolver. Tests needing only that seam use this constructor.
///
/// # Errors
///
/// Returns an error when the persisted instance identity cannot form an HTTP header.
pub fn create_router_with_media_reference_ownership_resolver(
    state: Arc<AppState>,
    instance_id: InstanceId,
    mailer: Arc<dyn MailSender>,
    secure_cookies: bool,
    storage_path: PathBuf,
    media_ownership_resolver: Arc<dyn MediaReferenceOwnershipResolver>,
) -> Result<Router, axum::http::header::InvalidHeaderValue> {
    create_router_with_dependencies(
        state,
        instance_id,
        mailer,
        secure_cookies,
        storage_path,
        media_ownership_resolver,
        None,
    )
}

/// Builds a router whose password-reset server-function dependencies are
/// explicitly supplied for deterministic integration tests.
///
/// # Errors
///
/// Returns an error when the persisted instance identity cannot form an HTTP header.
pub fn create_router_with_password_reset_dependencies_for_test(
    state: Arc<AppState>,
    mailer: Arc<dyn MailSender>,
    storage_path: PathBuf,
    users: Arc<dyn UserStorage>,
    password_resets: Arc<dyn PasswordResetStorage>,
    write_scope: WriteScope,
    site_config: Arc<dyn SiteConfigStorage>,
) -> Result<Router, axum::http::header::InvalidHeaderValue> {
    create_router_with_dependencies(
        state,
        InstanceId::new(),
        Arc::clone(&mailer),
        false,
        storage_path,
        Arc::new(LiveMediaReferenceOwnershipResolver::new()),
        Some(PasswordResetRequestDependencies {
            users,
            password_resets,
            write_scope,
            site_config,
            mailer,
        }),
    )
}

fn create_router_with_dependencies(
    state: Arc<AppState>,
    instance_id: InstanceId,
    mailer: Arc<dyn MailSender>,
    secure_cookies: bool,
    storage_path: PathBuf,
    media_ownership_resolver: Arc<dyn MediaReferenceOwnershipResolver>,
    password_reset_dependencies: Option<PasswordResetRequestDependencies>,
) -> Result<Router, axum::http::header::InvalidHeaderValue> {
    let instance_header = instance_id.to_string().parse::<HeaderValue>()?;
    let storage_path = Arc::new(storage_path);
    let media_content_locks = Arc::new(MediaContentLocks::new(Arc::clone(&storage_path)));
    let publisher_service = Arc::new(PublisherService::new(
        (*storage_path).clone(),
        Arc::clone(&state.publisher),
        state.write_scope.clone(),
    ));
    let media_manager = Arc::new(MediaManager::new(
        state.media.clone(),
        state.posts.clone(),
        state.site_config.clone(),
        state.write_scope.clone(),
        Arc::clone(&media_content_locks),
        instance_id,
        media_ownership_resolver,
    ));
    let sessions = state.sessions.clone();
    let write_scope = state.write_scope.clone();
    let posts = state.posts.clone();
    let audiences = state.audiences.clone();
    let users = state.users.clone();
    let user_config = state.user_config.clone();
    let site_config = state.site_config.clone();
    let media = state.media.clone();
    let feed_cache = state.feed_cache.clone();
    let feed_events = state.feed_events.clone();

    let provide_server_function_contexts = {
        let publisher_service = Arc::clone(&publisher_service);
        let media_content_locks = Arc::clone(&media_content_locks);
        let media_manager = Arc::clone(&media_manager);
        let password_reset_dependencies = password_reset_dependencies;

        move || {
            context::provide_app_state_contexts(&state, &publisher_service);
            context::provide_media_content_locks_context(&media_content_locks);
            context::provide_mailer_context(&mailer);
            if let Some(dependencies) = &password_reset_dependencies {
                provide_context::<Arc<dyn UserStorage>>(Arc::clone(&dependencies.users));
                provide_context::<Arc<dyn PasswordResetStorage>>(Arc::clone(
                    &dependencies.password_resets,
                ));
                provide_context::<WriteScope>(dependencies.write_scope.clone());
                provide_context::<Arc<dyn SiteConfigStorage>>(Arc::clone(
                    &dependencies.site_config,
                ));
                context::provide_mailer_context(&dependencies.mailer);
            }
            context::provide_media_manager_context(&media_manager);
            provide_context(web::auth::CookieSettings {
                secure: secure_cookies,
            });
        }
    };
    let app = build_application_routes(
        sessions.clone(),
        write_scope.clone(),
        provide_server_function_contexts,
    );

    // --- The page path: no reactive render (#180, closes #173). Serve the
    //     embedded CSR site tree (pkg/*, public/*) plus the public projector's
    //     cacheable anonymous HTML. The /api server fns and the raw HTTP routes
    //     (feed, media, atompub, style) above are untouched, so server fns remain
    //     the data API; only the page render leaves the request path. ---
    let app = {
        // The CSR bundle + public assets are embedded (#237, ADR-0003/0008): the
        // server owns them, the same way the SPA shell (#239) and CSS
        // (`StaticAssets`) are embedded. `site::serve_site` negotiates the
        // precompressed (.br/.gz) variants and falls through to the SPA shell for
        // any path with no embedded file — exactly as the old
        // `ServeDir(...).fallback(spa_shell)` did (the build never writes
        // index.html to disk; the server owns it). Non-reactive HTML for the
        // public discoverability routes (the projector, #178) sits ahead of this
        // fallback; everything else boots the CSR client via the shell.
        crate::projector::register(app, crate::projector::Shell(web::app::SPA_SHELL.into()))
            .fallback(site::serve_site)
    };
    // Raw Axum handlers receive only the storage traits they declare
    // (ADR-0016); server functions receive their separate Leptos contexts.
    let app = app
        .layer(axum::Extension(media_manager))
        .layer(axum::Extension(media_content_locks))
        .layer(axum::Extension(storage_path))
        .layer(axum::Extension(posts))
        .layer(axum::Extension(audiences))
        .layer(axum::Extension(users))
        .layer(axum::Extension(user_config))
        .layer(axum::Extension(site_config))
        .layer(axum::Extension(media))
        .layer(axum::Extension(feed_cache))
        .layer(axum::Extension(publisher_service))
        .layer(axum::Extension(feed_events))
        .layer(axum::Extension(sessions))
        .layer(axum::Extension(write_scope))
        .layer(axum::middleware::from_fn_with_state(
            secure_cookies,
            retire_session_cookie,
        ));

    Ok(crate::observability::with_http_observability(app).layer(
        axum::middleware::from_fn_with_state(instance_header, set_instance_header),
    ))
}

#[cfg(test)]
mod tests {
    use axum::{
        Router,
        http::{HeaderValue, header},
        response::IntoResponse,
        routing::get,
    };
    use tower::ServiceExt;

    use super::{INSTANCE_HEADER, set_instance_header};

    async fn conflicting_instance_header() -> axum::response::Response {
        let mut response = ().into_response();
        response
            .headers_mut()
            .append(INSTANCE_HEADER, HeaderValue::from_static("foreign"));
        response
            .headers_mut()
            .append(INSTANCE_HEADER, HeaderValue::from_static("duplicate"));
        response
    }

    #[tokio::test]
    async fn instance_header_replaces_inner_duplicate_values() {
        let app = Router::new()
            .route("/conflict", get(conflicting_instance_header))
            .layer(axum::middleware::from_fn_with_state(
                HeaderValue::from_static("canonical"),
                set_instance_header,
            ));

        let response = app
            .oneshot(
                axum::http::Request::builder()
                    .uri("/conflict")
                    .body(axum::body::Body::empty())
                    .expect("request"),
            )
            .await
            .expect("router response");

        let values = response
            .headers()
            .get_all(header::HeaderName::from_static("x-jaunder-instance"));
        assert_eq!(values.iter().count(), 1);
        assert_eq!(
            values.iter().next(),
            Some(&HeaderValue::from_static("canonical"))
        );
    }
}
