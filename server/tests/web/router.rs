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
    http::{Request, StatusCode, header::CONTENT_TYPE},
};
use server_fn::ServerFn;
use tower::ServiceExt;

use rstest::*;
use rstest_reuse::*;

use crate::helpers::{ensure_server_fns_registered, tmp_storage_path};
use storage::test_support::{Backend, TestEnv, backends, noop_mailer};

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
    let init = format!(
        r#"initMeasured(window.__jaunderWasmFetch ?? "{}")"#,
        web::app::WASM_URL
    );
    assert!(
        body.contains(&init),
        "SPA fallback consumes the early request with an explicit wasm fallback: {body}"
    );
    let prepaint = body
        .find(web::app::PREPAINT_SCRIPT)
        .expect("SPA fallback pre-paint script");
    let starter = body
        .find(web::app::EARLY_WASM_FETCH_SCRIPT)
        .expect("SPA fallback early wasm starter");
    let stylesheet = body
        .find(r#"<link rel="stylesheet" href="/style/jaunder.css" />"#)
        .expect("SPA fallback stylesheet");
    assert!(
        prepaint < starter && starter < stylesheet,
        "SPA fallback must keep prepaint → starter → stylesheet order: {body}"
    );
    assert!(!body.contains("modulepreload"), "{body}");
    assert!(!body.contains(r#"rel="preload""#), "{body}");
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
                .uri(<web::auth::GetSession as ServerFn>::PATH)
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

/// `server/src/lib.rs:65` mounts every server fn under one wildcard,
/// `"/api/{*fn_name}"`. The #684 endpoint scheme (`/api/<vertical>/<op>`) is only
/// viable if that wildcard captures multi-segment remainders — matchit's own
/// doctest says it does (`matchit-0.8.4/src/lib.rs:47-48`); this pins it so an
/// axum upgrade cannot silently 404 every server-fn route at once.
#[apply(backends)]
#[tokio::test]
async fn multi_segment_server_fn_route_is_reachable(#[case] backend: Backend) {
    let TestEnv { state, base: _base } = backend.setup().await;
    ensure_server_fns_registered();
    let app = jaunder::create_router(state, noop_mailer(), true, tmp_storage_path());
    let path = <web::auth::GetSession as ServerFn>::PATH;
    assert_eq!(path, "/api/auth/get_session", "the #684 scheme under test");
    let response = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri(path)
                .header("content-type", "application/x-www-form-urlencoded")
                .body(Body::empty())
                .expect("failed to build request"),
        )
        .await
        .expect("failed to get response");
    assert_ne!(
        response.status(),
        StatusCode::NOT_FOUND,
        "`/api/{{*fn_name}}` must capture a multi-segment server-fn path"
    );
}
