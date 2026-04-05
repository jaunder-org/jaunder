pub mod auth;
pub mod cli;
pub mod commands;
pub mod password;
pub mod storage;
pub mod username;

use std::sync::Arc;

use axum::Router;
use leptos::prelude::*;
use leptos_axum::{generate_route_list, LeptosRoutes};
use web::{shell, App};

use crate::storage::AppState;

pub fn create_router(leptos_options: LeptosOptions, state: Arc<AppState>) -> Router {
    let routes = generate_route_list(App);
    let extension_state = state.clone();
    Router::new()
        .leptos_routes_with_context(
            &leptos_options,
            routes,
            move || {
                provide_context(state.clone());
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
        let app = create_router(test_options(), test_state().await);
        let response = app
            .oneshot(Request::builder().uri("/").body(Body::empty()).unwrap())
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
    }

    #[tokio::test]
    async fn register_route_returns_ok() {
        let app = create_router(test_options(), test_state().await);
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
    }

    #[tokio::test]
    async fn register_route_with_invite_only_policy_returns_ok() {
        let state = test_state().await;
        state
            .site_config
            .set("site.registration_policy", "invite_only")
            .await
            .expect("failed to set registration policy");
        let app = create_router(test_options(), state);
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
    }

    #[tokio::test]
    async fn login_route_returns_ok() {
        let app = create_router(test_options(), test_state().await);
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
    }

    #[tokio::test]
    async fn unknown_route_returns_not_found_with_message() {
        let app = create_router(test_options(), test_state().await);
        let response = app
            .oneshot(
                Request::builder()
                    .uri("/nonexistent")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::NOT_FOUND);
        let body = axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .unwrap();
        let html = String::from_utf8(body.to_vec()).unwrap();
        assert!(html.contains("Page not found."));
    }

    #[tokio::test]
    async fn profile_route_returns_ok() {
        let app = create_router(test_options(), test_state().await);
        let response = app
            .oneshot(
                Request::builder()
                    .uri("/profile")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
    }

    #[tokio::test]
    async fn sessions_route_returns_ok() {
        let app = create_router(test_options(), test_state().await);
        let response = app
            .oneshot(
                Request::builder()
                    .uri("/sessions")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
    }

    #[tokio::test]
    async fn invites_route_returns_not_found_when_policy_is_closed() {
        // Default policy is Closed; InvitesPage sets 404 via SSR ResponseOptions.
        // Leptos SSR resolves Suspense asynchronously after headers are sent,
        // so the status is 200 but the body contains "Page not found."
        let app = create_router(test_options(), test_state().await);
        let response = app
            .oneshot(
                Request::builder()
                    .uri("/invites")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        let body = axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .unwrap();
        let html = String::from_utf8(body.to_vec()).unwrap();
        assert!(html.contains("Page not found."));
    }

    #[tokio::test]
    async fn home_response_contains_app_content() {
        let app = create_router(test_options(), test_state().await);
        let response = app
            .oneshot(Request::builder().uri("/").body(Body::empty()).unwrap())
            .await
            .unwrap();
        let body = axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .unwrap();
        let html = String::from_utf8(body.to_vec()).unwrap();
        assert!(html.contains("Welcome to Leptos"));
    }
}
