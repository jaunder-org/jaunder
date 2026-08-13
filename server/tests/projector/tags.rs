use axum::http::StatusCode;
use tower::ServiceExt;

use rstest::*;
use rstest_reuse::*;

use storage::test_support::{Backend, TestEnv, backends};

use super::fixtures::{get, projector_app, seed_tagged_post};

#[apply(backends)]
#[tokio::test]
async fn site_tag_projects_tagged_posts(#[case] backend: Backend) {
    let TestEnv { state, base: _base } = backend.setup().await;
    let (_u, title) = seed_tagged_post(&state).await;
    let resp = projector_app(&state)
        .oneshot(get("/tags/rust"))
        .await
        .expect("request");
    assert_eq!(resp.status(), StatusCode::OK, "site tag → 200");
    let body = axum::body::to_bytes(resp.into_body(), usize::MAX)
        .await
        .unwrap();
    let html = String::from_utf8_lossy(&body);
    assert!(html.contains("#rust"), "tag heading: {html}");
    assert!(html.contains(title.as_ref()), "tagged post present");
    assert!(html.contains(r#"id="jaunder-seed""#), "data blob present");
}

#[apply(backends)]
#[tokio::test]
async fn user_tag_projects_tagged_posts(#[case] backend: Backend) {
    let TestEnv { state, base: _base } = backend.setup().await;
    let (u, title) = seed_tagged_post(&state).await;
    let resp = projector_app(&state)
        .oneshot(get(&format!("/~{u}/tags/rust")))
        .await
        .expect("request");
    assert_eq!(resp.status(), StatusCode::OK, "user tag → 200");
    let body = axum::body::to_bytes(resp.into_body(), usize::MAX)
        .await
        .unwrap();
    let html = String::from_utf8_lossy(&body);
    assert!(html.contains(title.as_ref()), "tagged post present: {html}");
    assert!(html.contains(r#"id="jaunder-seed""#), "data blob present");
}

#[apply(backends)]
#[tokio::test]
async fn site_tag_invalid_serves_shell(#[case] backend: Backend) {
    let TestEnv { state, base: _base } = backend.setup().await;
    let resp = projector_app(&state)
        .oneshot(get("/tags/-rust"))
        .await
        .expect("request");
    assert_eq!(resp.status(), StatusCode::OK, "unparseable tag → shell");
    let body = axum::body::to_bytes(resp.into_body(), usize::MAX)
        .await
        .unwrap();
    assert!(String::from_utf8_lossy(&body).contains("test-shell"));
}

#[apply(backends)]
#[tokio::test]
async fn user_tag_invalid_serves_shell(#[case] backend: Backend) {
    let TestEnv { state, base: _base } = backend.setup().await;
    let resp = projector_app(&state)
        .oneshot(get("/~in.valid/tags/rust"))
        .await
        .expect("request");
    assert_eq!(
        resp.status(),
        StatusCode::OK,
        "unparseable username → shell"
    );
    let body = axum::body::to_bytes(resp.into_body(), usize::MAX)
        .await
        .unwrap();
    assert!(String::from_utf8_lossy(&body).contains("test-shell"));
}
