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
pub mod runtime_file;
mod scheduled_worker;
pub mod site;
mod soft_path;
pub mod websub;

#[cfg(test)]
mod test_support;

use std::{path::PathBuf, sync::Arc};

use axum::{
    Router,
    http::{HeaderName, HeaderValue},
};
use axum_embed::ServeEmbed;
use leptos::prelude::*;

use crate::{assets::StaticAssets, media_ownership::LiveMediaReferenceOwnershipResolver};
use ::storage::{AppState, InstanceId, MediaReferenceOwnershipResolver};

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
    mailer: Arc<dyn common::mailer::MailSender>,
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

/// Builds a router with an injected foreign-reference ownership resolver.
///
/// This is the narrow test composition seam; production callers use
/// [`create_router`], which installs the live resolver.
///
/// # Errors
///
/// Returns an error when the persisted instance identity cannot form an HTTP header.
pub fn create_router_with_media_reference_ownership_resolver(
    state: Arc<AppState>,
    instance_id: InstanceId,
    mailer: Arc<dyn common::mailer::MailSender>,
    secure_cookies: bool,
    storage_path: PathBuf,
    media_ownership_resolver: Arc<dyn MediaReferenceOwnershipResolver>,
) -> Result<Router, axum::http::header::InvalidHeaderValue> {
    // Per-trait extensions for the raw axum HTTP handlers (feed, atompub,
    // media). The whole `AppState` is never layered as an `Extension`; each
    // handler receives only the storage traits it declares (ADR-0016). The
    // Leptos `#[server]` functions are wired separately via per-trait contexts
    // in `provide_app_state_contexts`.
    let posts_ext = state.posts.clone();
    let audiences_ext = state.audiences.clone();
    // The projector's user-tag route resolves a username to a user id via the
    // user store (see `crate::projector`).
    let users_ext = state.users.clone();
    let user_config_ext = state.user_config.clone();
    let site_config_ext = state.site_config.clone();
    let media_ext = state.media.clone();
    let feed_cache_ext = state.feed_cache.clone();
    let feed_events_ext = state.feed_events.clone();
    // The `auth::User` extractor (web crate) authenticates the session cookie /
    // bearer token, so the raw HTTP handlers and the Leptos request `Parts`
    // need the session store reachable as a request extension.
    let sessions_ext = state.sessions.clone();
    let write_scope_ext = state.write_scope.clone();
    let instance_header = instance_id.to_string().parse::<HeaderValue>()?;
    let server_fn_instance_id = instance_id.clone();
    let server_fn_media_ownership_resolver = media_ownership_resolver.clone();
    let server_fn_state = state;
    let server_fn_mailer = mailer;
    let serve_assets = ServeEmbed::<StaticAssets>::new();
    let storage_path_ext = Arc::new(storage_path);
    let media_content_locks_ext = Arc::new(storage::MediaContentLocks::new(Arc::clone(
        &storage_path_ext,
    )));
    let server_fn_media_content_locks = Arc::clone(&media_content_locks_ext);
    let client_telemetry = crate::client_telemetry::router(
        sessions_ext.clone(),
        write_scope_ext.clone(),
        Arc::new(crate::client_telemetry::ClientTelemetryLimiter::new()),
    );
    let app = Router::new()
        .nest_service("/style", serve_assets)
        .merge(crate::media::router())
        .merge(crate::atompub::router())
        .merge(client_telemetry)
        .route(
            "/api/{*fn_name}",
            axum::routing::post(move |req: axum::extract::Request| {
                let instance_id = server_fn_instance_id.clone();
                let resolver = server_fn_media_ownership_resolver.clone();
                let state = server_fn_state.clone();
                let mailer = server_fn_mailer.clone();
                let media_content_locks = Arc::clone(&server_fn_media_content_locks);
                leptos_axum::handle_server_fns_with_context(
                    move || {
                        context::provide_app_state_contexts(&state);
                        context::provide_media_content_locks_context(&media_content_locks);
                        context::provide_mailer_context(&mailer);
                        context::provide_media_ownership_context(&resolver, &instance_id);
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
        let app =
            crate::projector::register(app, crate::projector::Shell(web::app::SPA_SHELL.into()));
        app.fallback(site::serve_site)
    };

    let app = app
        .layer(axum::Extension(media_ownership_resolver))
        .layer(axum::Extension(instance_id))
        .layer(axum::Extension(storage_path_ext))
        .layer(axum::Extension(media_content_locks_ext))
        .layer(axum::Extension(posts_ext))
        .layer(axum::Extension(audiences_ext))
        .layer(axum::Extension(users_ext))
        .layer(axum::Extension(user_config_ext))
        .layer(axum::Extension(site_config_ext))
        .layer(axum::Extension(media_ext))
        .layer(axum::Extension(feed_cache_ext))
        .layer(axum::Extension(feed_events_ext))
        .layer(axum::Extension(sessions_ext))
        .layer(axum::Extension(write_scope_ext))
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
