use axum::http::{StatusCode, header};
use tower::ServiceExt;

use rstest::*;
use rstest_reuse::*;

use storage::test_support::{Backend, TestEnv, backends};

use super::fixtures::{get, projector_app, seed_published_post};

#[apply(backends)]
#[tokio::test]
async fn permalink_projects_cacheable_crawlable_html(#[case] backend: Backend) {
    let TestEnv { state, base: _base } = backend.setup().await;
    let (u, y, m, d, slug, title, rendered_html) = seed_published_post(&state).await;
    let uri = format!("/~{u}/{y}/{m}/{d}/{slug}");

    let resp = projector_app(&state)
        .oneshot(get(&uri))
        .await
        .expect("request");
    assert_eq!(resp.status(), StatusCode::OK, "published permalink → 200");
    assert!(
        resp.headers().get(header::ETAG).is_some(),
        "ETag header present"
    );
    let body1 = axum::body::to_bytes(resp.into_body(), usize::MAX)
        .await
        .unwrap();
    let html = String::from_utf8_lossy(&body1);

    // Crawlable, JS-off: real content is in the served HTML.
    assert!(html.contains(title.as_ref()), "title present: {html}");
    assert!(
        html.contains(rendered_html.as_ref()),
        "rendered post body injected raw"
    );
    // The seed blob + CSR boot are embedded for the client to adopt.
    assert!(html.contains(r#"id="jaunder-seed""#), "data blob present");
    assert!(html.contains("/pkg/jaunder.js"), "CSR boot script present");

    // Byte-identical on repeat — no per-request variation, so CDN-cacheable.
    let body2 = axum::body::to_bytes(
        projector_app(&state)
            .oneshot(get(&uri))
            .await
            .unwrap()
            .into_body(),
        usize::MAX,
    )
    .await
    .unwrap();
    assert_eq!(body1, body2, "identical bytes per URL");
}

#[apply(backends)]
#[tokio::test]
async fn permalink_unknown_serves_spa_shell(#[case] backend: Backend) {
    // A URL with no anonymous-public post (nonexistent, or a draft only its
    // author may see) must serve the SPA shell — not a hard 404 — so the CSR
    // client resolves it with the session (draft view, or a client-side 404).
    let TestEnv { state, base: _base } = backend.setup().await;
    let resp = projector_app(&state)
        .oneshot(get("/~ghost/2026/1/2/missing"))
        .await
        .expect("request");
    assert_eq!(resp.status(), StatusCode::OK, "no public post → SPA shell");
    let body = axum::body::to_bytes(resp.into_body(), usize::MAX)
        .await
        .unwrap();
    let html = String::from_utf8_lossy(&body);
    assert!(html.contains("test-shell"), "served the SPA shell: {html}");
    assert!(
        !html.contains("jaunder-seed"),
        "no projected content for a nonexistent post"
    );
}

#[apply(backends)]
#[tokio::test]
async fn permalink_non_numeric_date_serves_shell(#[case] backend: Backend) {
    // A decoded five-segment permalink with a non-numeric date remains a projector soft miss:
    // the shell, never axum's pre-handler 400 (#697, ADR-0063 §4).
    let TestEnv { state, base: _base } = backend.setup().await;
    let resp = projector_app(&state)
        .oneshot(get("/~ghost/not-a-year/1/2/missing"))
        .await
        .expect("request");
    assert_eq!(
        resp.status(),
        StatusCode::OK,
        "non-numeric date → SPA shell"
    );
    let body = axum::body::to_bytes(resp.into_body(), usize::MAX)
        .await
        .unwrap();
    let html = String::from_utf8_lossy(&body);
    assert!(html.contains("test-shell"), "served the SPA shell: {html}");
}

#[apply(backends)]
#[tokio::test]
async fn permalink_overflowing_date_serves_shell(#[case] backend: Backend) {
    let TestEnv { state, base: _base } = backend.setup().await;
    let resp = projector_app(&state)
        .oneshot(get("/~ghost/2147483648/1/2/missing"))
        .await
        .expect("request");
    assert_eq!(
        resp.status(),
        StatusCode::OK,
        "overflowing date → SPA shell"
    );
    let body = axum::body::to_bytes(resp.into_body(), usize::MAX)
        .await
        .unwrap();
    let html = String::from_utf8_lossy(&body);
    assert!(html.contains("test-shell"), "served the SPA shell: {html}");
}

#[apply(backends)]
#[tokio::test]
async fn permalink_impossible_date_serves_shell(#[case] backend: Backend) {
    let TestEnv { state, base: _base } = backend.setup().await;
    let resp = projector_app(&state)
        .oneshot(get("/~ghost/2026/13/40/missing"))
        .await
        .expect("request");
    assert_eq!(resp.status(), StatusCode::OK, "impossible date → SPA shell");
    let body = axum::body::to_bytes(resp.into_body(), usize::MAX)
        .await
        .unwrap();
    let html = String::from_utf8_lossy(&body);
    assert!(html.contains("test-shell"), "served the SPA shell: {html}");
}

#[apply(backends)]
#[tokio::test]
async fn permalink_invalid_segment_serves_shell(#[case] backend: Backend) {
    // An unparseable username segment (a dot is not allowed) is never public
    // content — serve the shell and let the client route it.
    let TestEnv { state, base: _base } = backend.setup().await;
    let resp = projector_app(&state)
        .oneshot(get("/~in.valid/2026/1/2/slug"))
        .await
        .expect("request");
    assert_eq!(
        resp.status(),
        StatusCode::OK,
        "unparseable segment → SPA shell"
    );
    let body = axum::body::to_bytes(resp.into_body(), usize::MAX)
        .await
        .unwrap();
    assert!(String::from_utf8_lossy(&body).contains("test-shell"));
}

#[apply(backends)]
#[tokio::test]
async fn permalink_storage_failure_keeps_500_and_reports_boundary_once(#[case] backend: Backend) {
    let TestEnv { state, base } = backend.setup().await;
    let (u, y, m, d, slug, ..) = seed_published_post(&state).await;
    let uri = format!("/~{u}/{y}/{m}/{d}/{slug}");
    let app = projector_app(&state);
    base.close_pool().await;

    let (response, event) = crate::assert_error_signal!(
        async { app.oneshot(get(&uri)).await.expect("request") },
        event = "server function failed",
        event_kind = "Storage",
        event_class = "Bug",
        metric_kind = "storage",
        metric_class = "bug",
        disposition = "boundary",
        context = ""
    );

    assert_eq!(response.status(), StatusCode::INTERNAL_SERVER_ERROR);
    assert!(
        response.headers().get(header::CACHE_CONTROL).is_none(),
        "500 is not cached"
    );
    let body = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .expect("read body");
    assert!(body.is_empty(), "500 body remains sanitized");
    assert!(event.contains("pool"), "typed storage source: {event}");
}
