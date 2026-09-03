use std::sync::Arc;

use axum::http::{StatusCode, header};
use tower::ServiceExt;

use common::{MutationOutcome, theme::Theme};
use rstest::*;
use rstest_reuse::*;

use crate::helpers::body_string;

use storage::test_support::{Backend, TestEnv, backends};

use super::fixtures::{
    TEST_SHELL, assert_sanitized_internal_server_error, failing_site_config, get, projector_app,
    projector_app_with_dependencies, seed_tagged_post,
};

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
    let html = body_string(resp).await;
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
    let html = body_string(resp).await;
    assert!(html.contains(title.as_ref()), "tagged post present: {html}");
    assert!(html.contains(r#"id="jaunder-seed""#), "data blob present");
}
#[apply(backends)]
#[tokio::test]
async fn user_tag_projects_the_authors_override_into_initial_markup(#[case] backend: Backend) {
    let TestEnv { state, base: _base } = backend.setup().await;
    let (username, title) = seed_tagged_post(&state).await;
    let parsed_username = username.parse().expect("seeded username");
    let author = state
        .users
        .get_user_by_username(&parsed_username)
        .await
        .expect("author lookup")
        .expect("seeded author");
    let site_config = Arc::clone(&state.site_config);
    let user_config = Arc::clone(&state.user_config);
    let outcome = state
        .write_scope
        .run(|transaction| {
            Box::pin(async move {
                site_config.set_theme(transaction, Theme::Terminal).await?;
                storage::set_theme_override(
                    user_config.as_ref(),
                    transaction,
                    author.user_id,
                    Theme::Reader,
                )
                .await
            })
        })
        .await
        .expect("theme write");
    assert!(matches!(outcome, MutationOutcome::Confirmed(())));

    let response = projector_app(&state)
        .oneshot(get(&format!("/~{username}/tags/rust")))
        .await
        .expect("request");
    assert_eq!(response.status(), StatusCode::OK);
    let html = body_string(response).await;
    assert!(html.contains(title.as_ref()), "tagged post present: {html}");
    assert!(
        html.contains(r#"data-theme="reader""#),
        "author override reaches initial markup: {html}"
    );
    assert!(
        html.contains(r#""theme":"reader""#),
        "author override reaches projector seed: {html}"
    );
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
    let body = body_string(resp).await;
    assert!(body.contains("test-shell"));
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
    let body = body_string(resp).await;
    assert!(body.contains("test-shell"));
}

#[apply(backends)]
#[tokio::test]
async fn user_tag_invalid_tag_serves_shell(#[case] backend: Backend) {
    let TestEnv { state, base: _base } = backend.setup().await;
    let resp = projector_app(&state)
        .oneshot(get("/~alice/tags/-rust"))
        .await
        .expect("request");
    assert_eq!(resp.status(), StatusCode::OK, "unparseable tag → shell");
    let body = body_string(resp).await;
    assert!(body.contains("test-shell"));
}

#[apply(backends)]
#[tokio::test]
async fn user_tag_unknown_valid_username_serves_shell(#[case] backend: Backend) {
    let TestEnv { state, base: _base } = backend.setup().await;
    let resp = projector_app(&state)
        .oneshot(get("/~ghost/tags/rust"))
        .await
        .expect("request");
    assert_eq!(resp.status(), StatusCode::OK, "unknown user tag → shell");
    assert_eq!(
        resp.headers()
            .get(header::CACHE_CONTROL)
            .and_then(|value| value.to_str().ok()),
        Some("no-store"),
        "unknown user tag is a shell fallback, not a projected page"
    );
    let body = axum::body::to_bytes(resp.into_body(), usize::MAX)
        .await
        .unwrap();
    assert_eq!(body.as_ref(), TEST_SHELL.as_bytes(), "exact CSR shell body");
}

#[apply(backends)]
#[tokio::test]
async fn site_tag_storage_failure_keeps_no_store_shell_and_reports_once(#[case] backend: Backend) {
    let TestEnv { state, base } = backend.setup().await;
    seed_tagged_post(&state).await;
    let app = projector_app(&state);
    base.close_pool().await;

    let (response, event) = crate::assert_error_signal!(
        async { app.oneshot(get("/tags/rust")).await.expect("request") },
        event = "error swallowed after reporting",
        event_kind = "storage",
        event_class = "bug",
        metric_kind = "storage",
        metric_class = "bug",
        disposition = "swallowed",
        context = "server.projector.site_tag"
    );

    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(
        response
            .headers()
            .get(header::CACHE_CONTROL)
            .and_then(|value| value.to_str().ok()),
        Some("no-store")
    );
    let body = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .expect("read body");
    assert_eq!(body.as_ref(), TEST_SHELL.as_bytes(), "exact CSR shell body");
    assert!(event.contains("pool"), "typed storage source: {event}");
}

#[apply(backends)]
#[tokio::test]
async fn site_tag_theme_failure_keeps_500_and_reports_boundary_once(#[case] backend: Backend) {
    let TestEnv { state, base: _base } = backend.setup().await;
    seed_tagged_post(&state).await;
    let app = projector_app_with_dependencies(
        Arc::clone(&state.posts),
        Arc::clone(&state.users),
        failing_site_config("injected site tag theme failure"),
        Arc::clone(&state.user_config),
    );

    let (response, event) = crate::assert_error_signal!(
        async { app.oneshot(get("/tags/rust")).await.expect("request") },
        event = "server function failed",
        event_kind = "Storage",
        event_class = "Bug",
        metric_kind = "storage",
        metric_class = "bug",
        disposition = "boundary",
        context = "server.projector.site_tag"
    );

    assert_sanitized_internal_server_error(response).await;
    assert!(event.contains("injected site tag theme failure"));
}
