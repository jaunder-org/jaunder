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

use crate::storage::SiteConfigStorage;

pub fn create_router(leptos_options: LeptosOptions, db: Arc<dyn SiteConfigStorage>) -> Router {
    let routes = generate_route_list(App);
    Router::new()
        .leptos_routes_with_context(
            &leptos_options,
            routes,
            move || {
                provide_context(db.clone());
            },
            {
                let leptos_options = leptos_options.clone();
                move || shell(leptos_options.clone())
            },
        )
        .fallback(leptos_axum::file_and_error_handler(shell))
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

    async fn test_db() -> Arc<dyn SiteConfigStorage> {
        crate::storage::open_database(&"sqlite::memory:".parse().unwrap())
            .await
            .unwrap()
    }

    #[tokio::test]
    async fn home_route_returns_ok() {
        let app = create_router(test_options(), test_db().await);
        let response = app
            .oneshot(Request::builder().uri("/").body(Body::empty()).unwrap())
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
    }

    #[tokio::test]
    async fn unknown_route_returns_not_found() {
        let app = create_router(test_options(), test_db().await);
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
    }

    #[tokio::test]
    async fn home_response_contains_app_content() {
        let app = create_router(test_options(), test_db().await);
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
