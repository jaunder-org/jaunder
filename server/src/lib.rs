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

#[cfg(test)]
#[path = "build_staging.rs"]
mod build_staging;

#[doc(hidden)]
pub mod test_support;

use std::sync::Arc;

use ::storage::{InstanceId, SessionStorage, WriteScope};
use axum::{
    Router,
    http::{HeaderName, HeaderValue},
    routing,
};
use axum_embed::ServeEmbed;

use crate::{assets::StaticAssets, feed::handlers, projector::PublicProjector};

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

/// Adds every application route that needs an HTTP router. The caller supplies
/// the independently composed telemetry route and the request-context behavior.
pub fn application_routes<F>(
    client_telemetry: Router,
    provide_server_function_contexts: F,
    public_projector: PublicProjector,
) -> Router
where
    F: Fn() + Clone + Send + Sync + 'static,
{
    let app = Router::new()
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
        );

    crate::projector::register(app, public_projector).fallback(site::serve_site)
}

/// Completes a fully composed application router with the common page,
/// observability, cookie, and instance-identity middleware.
pub fn create_router(app: Router, instance_id: &InstanceId, secure_cookies: bool) -> Router {
    // A non-header-safe value would violate `InstanceId`'s canonical UUID invariant.
    let instance_header = instance_id
        .to_string()
        .parse::<HeaderValue>()
        .unwrap_or_else(|_| std::process::abort());
    let app = app.layer(axum::middleware::from_fn_with_state(
        secure_cookies,
        retire_session_cookie,
    ));

    crate::observability::with_http_observability(app).layer(axum::middleware::from_fn_with_state(
        instance_header,
        set_instance_header,
    ))
}

/// Builds client-telemetry routes from their exact storage dependencies.
pub fn client_telemetry_routes(
    sessions: Arc<dyn SessionStorage>,
    write_scope: WriteScope,
) -> Router {
    client_telemetry::router(
        sessions,
        write_scope,
        Arc::new(client_telemetry::ClientTelemetryLimiter::new()),
    )
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
