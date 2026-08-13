use axum::{
    body::Body,
    http::{Request, StatusCode, header},
};
use tower::ServiceExt;

use rstest::*;
use rstest_reuse::*;

use storage::test_support::{Backend, TestEnv, backends};

use super::fixtures::{get, projector_app, seed_published_post};

#[apply(backends)]
#[tokio::test]
async fn permalink_stale_if_none_match_serves_full_200(#[case] backend: Backend) {
    // A non-matching `If-None-Match` must not 304 — the client's cached copy is
    // stale, so serve the full document.
    let TestEnv { state, base: _base } = backend.setup().await;
    let (u, y, m, d, slug, ..) = seed_published_post(&state).await;
    let uri = format!("/~{u}/{y}/{m}/{d}/{slug}");
    let req = Request::builder()
        .method("GET")
        .uri(&uri)
        .header(header::IF_NONE_MATCH, "\"sha256-stale\"")
        .body(Body::empty())
        .unwrap();
    let resp = projector_app(&state).oneshot(req).await.expect("request");
    assert_eq!(resp.status(), StatusCode::OK, "stale ETag → full 200");
}

#[apply(backends)]
#[tokio::test]
async fn permalink_if_none_match_returns_304(#[case] backend: Backend) {
    let TestEnv { state, base: _base } = backend.setup().await;
    let (u, y, m, d, slug, ..) = seed_published_post(&state).await;
    let uri = format!("/~{u}/{y}/{m}/{d}/{slug}");

    let resp = projector_app(&state)
        .oneshot(get(&uri))
        .await
        .expect("request");
    let etag = resp
        .headers()
        .get(header::ETAG)
        .unwrap()
        .to_str()
        .unwrap()
        .to_string();

    let conditional = Request::builder()
        .method("GET")
        .uri(&uri)
        .header(header::IF_NONE_MATCH, &etag)
        .body(Body::empty())
        .unwrap();
    let resp = projector_app(&state)
        .oneshot(conditional)
        .await
        .expect("request");
    assert_eq!(
        resp.status(),
        StatusCode::NOT_MODIFIED,
        "matching If-None-Match → 304"
    );
}

#[apply(backends)]
#[tokio::test]
async fn projected_bytes_ignore_request_auth(#[case] backend: Backend) {
    // Cacheability invariant: the projector never branches on the viewer, so a
    // request carrying a session cookie yields byte-identical output to an
    // anonymous one — one cacheable response for every visitor.
    let TestEnv { state, base: _base } = backend.setup().await;
    let (u, y, m, d, slug, ..) = seed_published_post(&state).await;
    let uri = format!("/~{u}/{y}/{m}/{d}/{slug}");
    let anon = axum::body::to_bytes(
        projector_app(&state)
            .oneshot(get(&uri))
            .await
            .unwrap()
            .into_body(),
        usize::MAX,
    )
    .await
    .unwrap();
    let with_cookie = Request::builder()
        .method("GET")
        .uri(&uri)
        .header(header::COOKIE, "session=whatever")
        .body(Body::empty())
        .unwrap();
    let authed = axum::body::to_bytes(
        projector_app(&state)
            .oneshot(with_cookie)
            .await
            .unwrap()
            .into_body(),
        usize::MAX,
    )
    .await
    .unwrap();
    assert_eq!(
        anon, authed,
        "projector output must not vary with request auth"
    );
}

#[apply(backends)]
#[tokio::test]
async fn projected_response_is_publicly_cacheable(#[case] backend: Backend) {
    let TestEnv { state, base: _base } = backend.setup().await;
    let (u, y, m, d, slug, ..) = seed_published_post(&state).await;
    let uri = format!("/~{u}/{y}/{m}/{d}/{slug}");
    let resp = projector_app(&state)
        .oneshot(get(&uri))
        .await
        .expect("request");
    let cache_control = resp
        .headers()
        .get(header::CACHE_CONTROL)
        .and_then(|v| v.to_str().ok())
        .unwrap_or_default();
    assert!(
        cache_control.contains("public"),
        "projected response must be publicly cacheable, got: {cache_control}"
    );
}
