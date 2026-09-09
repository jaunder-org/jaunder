pub mod assets;
pub mod atompub;
pub mod backup;
mod bundle;
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
pub mod theme_content;

pub mod websub;

#[doc(hidden)]
pub mod test_support;

use std::{path::PathBuf, sync::Arc};

use axum::{
    Router,
    http::{HeaderName, HeaderValue},
    routing,
};
use axum_embed::ServeEmbed;
use common::mailer::MailSender;
use leptos::prelude;

use crate::{
    assets::StaticAssets,
    feed::handlers,
    media_ownership::LiveMediaReferenceOwnershipResolver,
    projector::{PublicProjector, Shell},
    publisher::PublisherService,
};
use ::storage::{
    AppState, InstanceId, MediaContentLocks, MediaManager, MediaReferenceOwnershipResolver,
    PostMediaOwnership, PostStorage, SessionStorage, ThemeAssetManager, ThemeManager, ThemeStorage,
    UserStorage, WriteScope,
};
use host::theme_operations::ThemeOperationCoordinator;

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
            unreachable!("generated session cookie header must be valid"); // cov:ignore -- host constructs this fixed header from validated literals.
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
        .merge(crate::theme_content::router())
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
        || {},
    )
}

/// Places cacheable anonymous projection ahead of the embedded CSR fallback.
fn build_page_routes(
    app: Router,
    public_projector: PublicProjector,
    post_media_ownership: PostMediaOwnership,
) -> Router {
    let app = crate::projector::register(app, public_projector);
    app.fallback(site::serve_site)
        .layer(axum::Extension(post_media_ownership))
}

fn build_public_projector(
    posts: &Arc<dyn PostStorage>,
    users: &Arc<dyn UserStorage>,
    themes: &Arc<dyn ThemeStorage>,
) -> PublicProjector {
    PublicProjector::new(
        Arc::clone(posts),
        Arc::clone(users),
        Arc::clone(themes),
        Shell(site::shell_html()),
    )
}

fn create_router_with_dependencies<F>(
    state: Arc<AppState>,
    instance_id: InstanceId,
    mailer: Arc<dyn MailSender>,
    secure_cookies: bool,
    storage_path: PathBuf,
    media_ownership_resolver: Arc<dyn MediaReferenceOwnershipResolver>,
    provide_additional_contexts: F,
) -> Result<Router, axum::http::header::InvalidHeaderValue>
where
    F: Fn() + Clone + Send + Sync + 'static,
{
    let instance_header = instance_id.to_string().parse::<HeaderValue>()?;
    let storage_path = Arc::new(storage_path);
    let media_content_locks = Arc::new(MediaContentLocks::new(Arc::clone(&storage_path)));
    let post_media_ownership = PostMediaOwnership::new(
        Arc::clone(&media_ownership_resolver),
        instance_id.clone(),
        Arc::clone(&state.site_config),
    );
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
    let theme_asset_manager = Arc::new(ThemeAssetManager::new(
        state.themes.clone(),
        state.write_scope.clone(),
        Arc::clone(&storage_path),
    ));
    let theme_operation_coordinator = Arc::new(ThemeOperationCoordinator::new());
    let theme_manager = Arc::new(ThemeManager::new(
        state.themes.clone(),
        state.media.clone(),
        state.write_scope.clone(),
        Arc::clone(&media_content_locks),
    ));
    let sessions = state.sessions.clone();
    let write_scope = state.write_scope.clone();
    let posts = state.posts.clone();
    let audiences = state.audiences.clone();
    let users = state.users.clone();
    let user_config = state.user_config.clone();
    let site_config = state.site_config.clone();
    let themes = state.themes.clone();
    let media = state.media.clone();
    let feed_cache = state.feed_cache.clone();
    let feed_events = state.feed_events.clone();
    let public_projector = build_public_projector(&posts, &users, &themes);

    let provide_server_function_contexts = {
        let publisher_service = Arc::clone(&publisher_service);
        let media_content_locks = Arc::clone(&media_content_locks);
        let media_manager = Arc::clone(&media_manager);
        let post_media_ownership = post_media_ownership.clone();
        let theme_operation_coordinator = Arc::clone(&theme_operation_coordinator);
        let theme_manager = Arc::clone(&theme_manager);
        move || {
            prelude::provide_context(post_media_ownership.clone());
            context::provide_app_state_contexts(&state, &publisher_service);
            context::provide_media_content_locks_context(&media_content_locks);
            context::provide_mailer_context(&mailer);
            provide_additional_contexts();
            context::provide_media_manager_context(&media_manager);
            context::provide_theme_asset_manager_context(&theme_asset_manager);
            context::provide_theme_operation_coordinator_context(&theme_operation_coordinator);
            context::provide_theme_manager_context(&theme_manager);
            prelude::provide_context(web::auth::CookieSettings {
                secure: secure_cookies,
            });
        }
    };
    let app = build_application_routes(
        sessions.clone(),
        write_scope.clone(),
        provide_server_function_contexts,
    );

    let app = build_page_routes(app, public_projector, post_media_ownership);
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
        .layer(axum::Extension(themes))
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
