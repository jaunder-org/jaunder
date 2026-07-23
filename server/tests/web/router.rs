//! Router-level smoke tests relocated from `server/src/lib.rs` (#426): they
//! exercise the public `create_router` end to end, so they belong in the
//! integration suite where the single server-fn registrar
//! (`helpers::ensure_server_fns_registered`) lives — rather than carrying a
//! second, independently-rotting registrar in the library crate.
//!
//! They need an `AppState`, so they run over the standard `backends` fixture
//! (temp `SQLite` + Postgres) like every other `server/tests/web` test, satisfying
//! the `test-backend-pattern` guard honestly. Their assertions are
//! backend-agnostic (routing / SSR), so running on both backends is redundant but
//! consistent and cheap. `base: _base` keeps the `TempDir` alive for the test
//! body (dropping it unlinks the `SQLite` file; ADR-0053 / #136).

use axum::{
    body::Body,
    http::{header::CONTENT_TYPE, Request, StatusCode},
};
use leptos::prelude::LeptosOptions;
use tower::ServiceExt;

use rstest::*;
use rstest_reuse::*;

use crate::helpers::{ensure_server_fns_registered, test_options, tmp_storage_path};
use storage::test_support::{backends, noop_mailer, Backend, TestEnv};

#[apply(backends)]
#[tokio::test]
async fn home_route_returns_ok(#[case] backend: Backend) {
    let TestEnv { state, base: _base } = backend.setup().await;
    let local = tokio::task::LocalSet::new();
    local
        .run_until(async {
            ensure_server_fns_registered();
            let app = jaunder::create_router(
                test_options(),
                state,
                noop_mailer(),
                true,
                tmp_storage_path(),
            );
            let response = app
                .oneshot(Request::builder().uri("/").body(Body::empty()).unwrap())
                .await
                .unwrap();
            assert_eq!(response.status(), StatusCode::OK);
        })
        .await;
}

#[apply(backends)]
#[tokio::test]
async fn spa_fallback_serves_embedded_shell_without_disk_index_html(#[case] backend: Backend) {
    let TestEnv { state, base: _base } = backend.setup().await;
    // A site_root with no index.html on disk (the host reality, #239). The SPA
    // fallback must still serve the embedded shell — 200, text/html, boots wasm.
    let options = LeptosOptions::builder()
        .output_name("test")
        .site_root("/tmp/jaunder-nonexistent-site-239")
        .build();
    let local = tokio::task::LocalSet::new();
    local
        .run_until(async {
            ensure_server_fns_registered();
            let app =
                jaunder::create_router(options, state, noop_mailer(), true, tmp_storage_path());
            // `/login` is a client route → not a projector route → SPA fallback.
            let response = app
                .oneshot(
                    Request::builder()
                        .uri("/login")
                        .body(Body::empty())
                        .unwrap(),
                )
                .await
                .unwrap();
            assert_eq!(response.status(), StatusCode::OK);
            assert_eq!(
                response.headers().get(CONTENT_TYPE).unwrap(),
                "text/html; charset=utf-8"
            );
            let body = axum::body::to_bytes(response.into_body(), usize::MAX)
                .await
                .unwrap();
            let body = String::from_utf8(body.to_vec()).unwrap();
            assert!(
                body.contains(r#"init("/pkg/jaunder.wasm")"#),
                "SPA fallback serves the embedded shell that boots the wasm: {body}"
            );
        })
        .await;
}

#[apply(backends)]
#[tokio::test]
async fn home_response_contains_app_content(#[case] backend: Backend) {
    let TestEnv { state, base: _base } = backend.setup().await;
    let local = tokio::task::LocalSet::new();
    local
        .run_until(async {
            let app = jaunder::create_router(
                test_options(),
                state,
                noop_mailer(),
                true,
                tmp_storage_path(),
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

#[apply(backends)]
#[tokio::test]
async fn session_api_route_returns_ok(#[case] backend: Backend) {
    let TestEnv { state, base: _base } = backend.setup().await;
    let local = tokio::task::LocalSet::new();
    local
        .run_until(async {
            ensure_server_fns_registered();
            let app = jaunder::create_router(
                test_options(),
                state,
                noop_mailer(),
                true,
                tmp_storage_path(),
            );
            let response = app
                .oneshot(
                    Request::builder()
                        .method("POST")
                        .uri("/api/session")
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
