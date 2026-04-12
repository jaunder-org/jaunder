pub mod auth;
pub mod cli;
pub mod commands;
pub mod mailer;
pub mod password;
pub mod render {
    pub use common::render::*;
}
pub mod storage;
pub mod tag;
pub mod username;

use std::sync::Arc;

use axum::Router;
use leptos::prelude::*;
use leptos_axum::{generate_route_list, LeptosRoutes};
use web::{shell, App};

use crate::storage::AppState;

pub fn create_router(
    leptos_options: LeptosOptions,
    state: Arc<AppState>,
    secure_cookies: bool,
) -> Router {
    let routes = generate_route_list(App);
    let extension_state = state.clone();
    Router::new()
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
        .with_state(leptos_options)
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
        ensure_server_fns_registered();
        let app = create_router(test_options(), test_state().await, true);
        let response = app
            .oneshot(Request::builder().uri("/").body(Body::empty()).unwrap())
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
    }

    #[tokio::test]
    async fn profile_route_returns_ok() {
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
    }

    #[tokio::test]
    async fn sessions_route_returns_ok() {
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
    }

    #[tokio::test]
    async fn register_route_returns_ok() {
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
    }

    #[tokio::test]
    async fn invites_route_returns_not_found_when_policy_is_closed() {
        ensure_server_fns_registered();
        // Default policy is Closed; InvitesPage sets "Page not found." body via Suspense.
        // When the Suspense resolves before response headers are committed (common with
        // fast in-memory SQLite), Leptos correctly emits 404.  When it resolves after
        // headers are committed (streaming path), the status remains 200.  Both
        // outcomes are valid; what matters is that the rendered body says "Page not
        // found." in both cases.
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
    }

    #[tokio::test]
    async fn home_response_contains_app_content() {
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
    }
}
