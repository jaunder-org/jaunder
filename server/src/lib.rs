// The ParentRoute wrapping all routes in web::App generates a wide tuple of
// route types; the compiler needs a higher recursion limit to monomorphize it,
// particularly under llvm-cov instrumentation. Root cause under investigation.
#![recursion_limit = "512"]

pub mod assets;
pub mod atompub;
pub mod backup;
pub mod cli;
pub mod commands;
pub mod context;
pub mod feed;
pub mod mailer;
pub mod media;
pub mod media_manager;
pub mod observability;

use std::{path::PathBuf, sync::Arc};

use axum::Router;
use axum_embed::ServeEmbed;
use leptos::prelude::*;
use leptos_axum::{generate_route_list, LeptosRoutes};
use web::{shell, App};

use crate::assets::StaticAssets;
use ::storage::AppState;

pub fn create_router(
    leptos_options: LeptosOptions,
    state: Arc<AppState>,
    mailer: Arc<dyn common::mailer::MailSender>,
    secure_cookies: bool,
    storage_path: PathBuf,
) -> Router {
    let routes = generate_route_list(App);
    let extension_state = state.clone();
    let server_fn_state = state.clone();
    let server_fn_mailer = mailer.clone();
    let leptos_mailer = mailer;
    let serve_assets = ServeEmbed::<StaticAssets>::new();
    let storage_path_ext = Arc::new(storage_path);
    let app = Router::new()
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
        .layer(axum::Extension(extension_state));
    crate::observability::with_http_observability(app).with_state(leptos_options)
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::{
        body::Body,
        http::{Request, StatusCode},
    };
    use leptos::prelude::LeptosOptions;
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
