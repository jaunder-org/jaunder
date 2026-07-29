//! Router-level smoke tests relocated from `server/src/lib.rs` (#426): they
//! exercise the public `create_router` end to end, so they belong in the
//! integration suite where the single server-fn registrar
//! (`helpers::ensure_server_fns_registered`) lives — rather than carrying a
//! second, independently-rotting registrar in the library crate.
//!
//! They need an `AppState`, so they run over the standard `backends` fixture
//! (temp `SQLite` + Postgres) like every other `server/tests/web` test, satisfying
//! the `test-backend-pattern` guard honestly. Their assertions are
//! backend-agnostic (routing only), so running on both backends is redundant but
//! consistent and cheap. `base: _base` keeps the `TempDir` alive for the test
//! body (dropping it unlinks the `SQLite` file; ADR-0053 / #136).

use axum::{
    body::Body,
    http::{header::CONTENT_TYPE, Request, StatusCode},
};
use server_fn::ServerFn;
use tower::ServiceExt;

use rstest::*;
use rstest_reuse::*;

use crate::helpers::{ensure_server_fns_registered, tmp_storage_path};
use storage::test_support::{backends, noop_mailer, Backend, TestEnv};

#[apply(backends)]
#[tokio::test]
async fn home_route_returns_ok(#[case] backend: Backend) {
    let TestEnv { state, base: _base } = backend.setup().await;
    ensure_server_fns_registered();
    let app = jaunder::create_router(state, noop_mailer(), true, tmp_storage_path());
    let response = app
        .oneshot(Request::builder().uri("/").body(Body::empty()).unwrap())
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
}

#[apply(backends)]
#[tokio::test]
async fn spa_fallback_serves_embedded_shell_without_disk_index_html(#[case] backend: Backend) {
    let TestEnv { state, base: _base } = backend.setup().await;
    // No index.html exists on disk (the host reality, #239); the server owns the
    // embedded shell. The SPA fallback must still serve it — 200, text/html,
    // boots wasm.
    ensure_server_fns_registered();
    let app = jaunder::create_router(state, noop_mailer(), true, tmp_storage_path());
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
}

#[apply(backends)]
#[tokio::test]
async fn session_api_route_returns_ok(#[case] backend: Backend) {
    let TestEnv { state, base: _base } = backend.setup().await;
    ensure_server_fns_registered();
    let app = jaunder::create_router(state, noop_mailer(), true, tmp_storage_path());
    let response = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri(<web::auth::Session as ServerFn>::PATH)
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
}
