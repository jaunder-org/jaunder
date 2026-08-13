use axum::http::StatusCode;
use tower::ServiceExt;

use rstest::*;
use rstest_reuse::*;

use storage::test_support::{Backend, TestEnv, backends};

use super::fixtures::{get, projector_app, seed_published_post};

#[apply(backends)]
#[tokio::test]
async fn profile_projects_user_timeline(#[case] backend: Backend) {
    let TestEnv { state, base: _base } = backend.setup().await;
    let (u, .., title, _rendered_html) = seed_published_post(&state).await;
    let resp = projector_app(&state)
        .oneshot(get(&format!("/~{u}")))
        .await
        .expect("request");
    assert_eq!(resp.status(), StatusCode::OK, "profile → 200");
    let body = axum::body::to_bytes(resp.into_body(), usize::MAX)
        .await
        .unwrap();
    let html = String::from_utf8_lossy(&body);
    assert!(
        html.contains(&format!("Posts by {u}")),
        "profile heading: {html}"
    );
    assert!(html.contains(title.as_ref()), "post title present");
    assert!(html.contains(r#"id="jaunder-seed""#), "data blob present");
}

#[apply(backends)]
#[tokio::test]
async fn site_timeline_projects_local_posts(#[case] backend: Backend) {
    let TestEnv { state, base: _base } = backend.setup().await;
    let (.., title, _rendered_html) = seed_published_post(&state).await;
    let resp = projector_app(&state)
        .oneshot(get("/"))
        .await
        .expect("request");
    assert_eq!(resp.status(), StatusCode::OK, "root site timeline → 200");
    let body = axum::body::to_bytes(resp.into_body(), usize::MAX)
        .await
        .unwrap();
    let html = String::from_utf8_lossy(&body);
    assert!(html.contains(title.as_ref()), "post present: {html}");
    assert!(html.contains(r#"id="jaunder-seed""#), "data blob present");
}

#[apply(backends)]
#[tokio::test]
async fn profile_invalid_username_serves_shell(#[case] backend: Backend) {
    let TestEnv { state, base: _base } = backend.setup().await;
    let resp = projector_app(&state)
        .oneshot(get("/~in.valid"))
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
