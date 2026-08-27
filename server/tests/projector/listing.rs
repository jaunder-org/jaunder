use axum::http::{StatusCode, header};
use tower::ServiceExt;

use common::seed::{Page, PageSeed};
use common::time::{PermalinkDate, UtcInstant};
use common::visibility::ViewerIdentity;
use rstest::*;
use rstest_reuse::*;

use storage::test_support::{Backend, TestEnv, backends};

use super::fixtures::{TEST_SHELL, get, projector_app, seed_published_post};

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

#[apply(backends)]
#[tokio::test]
async fn profile_unknown_valid_username_is_cacheable_projection(#[case] backend: Backend) {
    let TestEnv { state, base: _base } = backend.setup().await;
    let resp = projector_app(&state)
        .oneshot(get("/~ghost"))
        .await
        .expect("request");
    assert_eq!(resp.status(), StatusCode::OK, "unknown profile → 200");
    assert_eq!(
        resp.headers()
            .get(header::CACHE_CONTROL)
            .and_then(|value| value.to_str().ok()),
        Some("public, max-age=300"),
        "valid unknown username projects an empty cacheable profile"
    );
    let body = axum::body::to_bytes(resp.into_body(), usize::MAX)
        .await
        .unwrap();
    let html = String::from_utf8_lossy(&body);
    assert!(html.contains("Posts by ghost"), "profile heading: {html}");
    assert!(html.contains(r#"id="jaunder-seed""#), "data blob present");
}

#[apply(backends)]
#[tokio::test]
async fn site_timeline_storage_failure_keeps_500_and_reports_boundary_once(
    #[case] backend: Backend,
) {
    let TestEnv { state, base } = backend.setup().await;
    seed_published_post(&state).await;
    let app = projector_app(&state);
    base.close_pool().await;

    let (response, event) = crate::assert_error_signal!(
        async { app.oneshot(get("/")).await.expect("request") },
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

#[apply(backends)]
#[tokio::test]
async fn profile_storage_failure_keeps_no_store_shell_and_reports_once(#[case] backend: Backend) {
    let TestEnv { state, base } = backend.setup().await;
    let (username, ..) = seed_published_post(&state).await;
    let app = projector_app(&state);
    base.close_pool().await;

    let (response, event) = crate::assert_error_signal!(
        async {
            app.oneshot(get(&format!("/~{username}")))
                .await
                .expect("request")
        },
        event = "error swallowed after reporting",
        event_kind = "storage",
        event_class = "bug",
        metric_kind = "storage",
        metric_class = "bug",
        disposition = "swallowed",
        context = "server.projector.profile"
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
async fn every_page_seed_variant_serializes_without_null_fallback(#[case] backend: Backend) {
    let TestEnv { state, base: _base } = backend.setup().await;
    let (username, year, month, day, slug, ..) = seed_published_post(&state).await;
    let username = username.parse().expect("seeded username");
    let slug = slug.parse().expect("seeded slug");
    let date = PermalinkDate::from_ymd(year, month, day).expect("seeded date");
    let record = storage::fetch_post_record(
        state.posts.as_ref(),
        &ViewerIdentity::Anonymous,
        &username,
        date,
        &slug,
        UtcInstant::now(),
    )
    .await
    .expect("permalink lookup")
    .expect("seeded post");
    let page = Page {
        posts: Vec::new(),
        next_cursor: None,
        has_more: false,
    };
    let tag: common::tag::Tag = "rust".parse().expect("representative tag");
    let seeds = [
        PageSeed::SiteTimeline(page.clone()),
        PageSeed::Profile {
            username: username.clone(),
            page: page.clone(),
        },
        PageSeed::SiteTag {
            tag: tag.clone(),
            page: page.clone(),
        },
        PageSeed::UserTag {
            username,
            tag,
            page,
        },
        PageSeed::Permalink(web::posts::authored_post(record, false)),
    ];

    for seed in seeds {
        // No wildcard: adding a PageSeed variant makes this proof fail to compile
        // until a representative is added. Every closed field is a derived
        // string/integer/sequence/newtype serializer; none is a fallible map key
        // or custom serializer, so `null` remains defensive only.
        let variant = match &seed {
            PageSeed::SiteTimeline(_) => "site timeline",
            PageSeed::Profile { .. } => "profile",
            PageSeed::SiteTag { .. } => "site tag",
            PageSeed::UserTag { .. } => "user tag",
            PageSeed::Permalink(_) => "permalink",
        };
        let json = serde_json::to_string(&seed)
            .unwrap_or_else(|error| panic!("{variant} must serialize: {error}"));
        assert_ne!(json, "null", "{variant}");
        let document = jaunder::projector::document(&seed);
        assert!(
            !document.contains(r#"id="jaunder-seed">null</script>"#),
            "{variant} selected the defensive null fallback"
        );
    }
}
