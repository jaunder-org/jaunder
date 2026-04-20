pub mod auth;
pub mod cli;
pub mod commands;
pub mod mailer;
pub mod observability;
pub mod password;
pub mod render {
    pub use common::render::*;
}
pub mod storage;
pub mod tag;
pub mod username;

use std::sync::Arc;

use axum::http::HeaderName;
use axum::Router;
use leptos::prelude::*;
use leptos_axum::{generate_route_list, LeptosRoutes};
use opentelemetry::propagation::Extractor;
use tower::ServiceBuilder;
use tower_http::request_id::{MakeRequestUuid, PropagateRequestIdLayer, SetRequestIdLayer};
use tower_http::trace::{DefaultOnResponse, TraceLayer};
use tracing::Level;
use tracing_opentelemetry::OpenTelemetrySpanExt;
use web::{shell, App};

use crate::storage::AppState;

pub fn create_router(
    leptos_options: LeptosOptions,
    state: Arc<AppState>,
    secure_cookies: bool,
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
    Router::new()
        .route(
            "/api/{*fn_name}",
            axum::routing::post(move |req: axum::extract::Request| {
                let state = server_fn_state.clone();
                leptos_axum::handle_server_fns_with_context(
                    move || {
                        provide_context(state.clone());
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
                provide_context(state.clone());
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
        .layer(axum::Extension(extension_state))
        .layer(http_observability)
        .with_state(leptos_options)
}

#[derive(Clone)]
struct ExtractedTraceContext(opentelemetry::Context);

struct HeaderExtractor<'a>(&'a axum::http::HeaderMap);

impl Extractor for HeaderExtractor<'_> {
    fn get(&self, key: &str) -> Option<&str> {
        self.0.get(key).and_then(|value| value.to_str().ok())
    }

    fn keys(&self) -> Vec<&str> {
        self.0.keys().map(|name| name.as_str()).collect()
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
    use tower::ServiceExt;

    fn ensure_server_fns_registered() {
        server_fn::axum::register_explicit::<web::auth::CurrentUser>();
        server_fn::axum::register_explicit::<web::auth::GetRegistrationPolicy>();
        server_fn::axum::register_explicit::<web::auth::Register>();
        server_fn::axum::register_explicit::<web::auth::Login>();
        server_fn::axum::register_explicit::<web::auth::Logout>();
    }

    fn test_options() -> LeptosOptions {
        LeptosOptions::builder().output_name("test").build()
    }

    async fn test_state() -> Arc<AppState> {
        crate::storage::open_database(&"sqlite::memory:".parse().unwrap())
            .await
            .unwrap()
    }

    #[tokio::test]
    async fn home_route_returns_ok() {
        let local = tokio::task::LocalSet::new();
        local
            .run_until(async {
                ensure_server_fns_registered();
                let app = create_router(test_options(), test_state().await, true);
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
                let app = create_router(test_options(), test_state().await, true);
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
                let app = create_router(test_options(), test_state().await, true);
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
                let app = create_router(test_options(), test_state().await, true);
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
                let app = create_router(test_options(), test_state().await, true);
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
                let app = create_router(test_options(), test_state().await, true);
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
                let app = create_router(test_options(), test_state().await, true);
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
                let app = create_router(test_options(), state, true);
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
                let app = create_router(test_options(), test_state().await, true);
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
                let app = create_router(test_options(), test_state().await, true);
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
                let app = create_router(test_options(), test_state().await, true);
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
