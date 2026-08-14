pub mod assets;
pub mod atompub;
pub mod backup;
pub mod cli;
pub mod client_telemetry;
pub mod commands;
pub mod context;
pub mod feed;
pub mod mailer;
pub mod media;
pub mod observability;
pub mod projector;
pub mod runtime_file;
pub mod site;
mod soft_path;
pub mod websub;

#[cfg(test)]
mod test_support;

use std::{path::PathBuf, sync::Arc};

use axum::Router;
use axum_embed::ServeEmbed;
use leptos::prelude::*;

use crate::assets::StaticAssets;
use ::storage::AppState;

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

pub fn create_router(
    state: Arc<AppState>,
    mailer: Arc<dyn common::mailer::MailSender>,
    secure_cookies: bool,
    storage_path: PathBuf,
) -> Router {
    // Per-trait extensions for the raw axum HTTP handlers (feed, atompub,
    // media). The whole `AppState` is never layered as an `Extension`; each
    // handler receives only the storage traits it declares (ADR-0016). The
    // Leptos `#[server]` functions are wired separately via per-trait contexts
    // in `provide_app_state_contexts`.
    let posts_ext = state.posts.clone();
    // The projector's user-tag route resolves a username to a user id via the
    // user store (see `crate::projector`).
    let users_ext = state.users.clone();
    let user_config_ext = state.user_config.clone();
    let site_config_ext = state.site_config.clone();
    let media_ext = state.media.clone();
    let feed_cache_ext = state.feed_cache.clone();
    // The `AuthUser` extractor (web crate) authenticates the session cookie /
    // bearer token, so the raw HTTP handlers and the Leptos request `Parts`
    // need the session store reachable as a request extension.
    let sessions_ext = state.sessions.clone();
    let server_fn_state = state;
    let server_fn_mailer = mailer;
    let serve_assets = ServeEmbed::<StaticAssets>::new();
    let storage_path_ext = Arc::new(storage_path);
    let client_telemetry = crate::client_telemetry::router(
        sessions_ext.clone(),
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
        .layer(axum::Extension(storage_path_ext))
        .layer(axum::Extension(posts_ext))
        .layer(axum::Extension(users_ext))
        .layer(axum::Extension(user_config_ext))
        .layer(axum::Extension(site_config_ext))
        .layer(axum::Extension(media_ext))
        .layer(axum::Extension(feed_cache_ext))
        .layer(axum::Extension(sessions_ext))
        .layer(axum::middleware::from_fn_with_state(
            secure_cookies,
            retire_session_cookie,
        ));
    crate::observability::with_http_observability(app)
}
