use axum::{
    body::Body,
    http::{Method, Request, StatusCode, header},
};
use jiff::tz::Offset;
use tower::ServiceExt;

use rstest::*;
use rstest_reuse::*;

use crate::helpers::body_string;

use common::time::UtcInstant;
use storage::test_support::{Backend, SeedRawPost, SeedUser, backends};

use super::fixtures::{
    assert_sanitized_internal_server_error, assert_shell_miss, failing_author_theme_selection,
    failing_site_theme_selection, get, projector_app, projector_app_with_dependencies,
    seed_published_post,
};

#[apply(backends)]
#[tokio::test]
async fn permalink_projects_cacheable_crawlable_html(#[case] backend: Backend) {
    let env = backend.setup().await;
    let (u, y, m, d, slug, title, rendered_html) =
        seed_published_post(env.users(), env.posts(), env.write_scope()).await;
    let uri = format!("/~{u}/{y}/{m}/{d}/{slug}");

    let resp = projector_app(env.posts(), env.users(), env.themes())
        .oneshot(get(&uri))
        .await
        .expect("request");
    assert_eq!(resp.status(), StatusCode::OK, "published permalink → 200");
    assert!(
        resp.headers().get(header::ETAG).is_some(),
        "ETag header present"
    );
    let html = body_string(resp).await;

    // Crawlable, JS-off: real content is in the served HTML.
    assert!(html.contains(title.as_ref()), "title present: {html}");
    assert!(
        html.contains(rendered_html.as_ref()),
        "rendered post body injected raw"
    );
    // The seed blob remains embedded for client adoption; final CSR URLs are
    // build-generated and intentionally absent from this host no-bundle test.
    assert!(html.contains(r#"id="jaunder-seed""#), "data blob present");

    // Byte-identical on repeat — no per-request variation, so CDN-cacheable.
    let body2 = axum::body::to_bytes(
        projector_app(env.posts(), env.users(), env.themes())
            .oneshot(get(&uri))
            .await
            .unwrap()
            .into_body(),
        usize::MAX,
    )
    .await
    .unwrap();
    assert_eq!(html.as_bytes(), body2.as_ref(), "identical bytes per URL");
}

#[apply(backends)]
#[tokio::test]
async fn permalink_unknown_serves_spa_shell(#[case] backend: Backend) {
    // A URL with no anonymous-public post (nonexistent, or a draft only its
    // author may see) must serve the SPA shell — not a hard 404 — so the CSR
    // client resolves it with the session (draft view, or a client-side 404).
    let env = backend.setup().await;
    let resp = projector_app(env.posts(), env.users(), env.themes())
        .oneshot(get("/~ghost/2026/1/2/missing"))
        .await
        .expect("request");
    assert_eq!(resp.status(), StatusCode::OK, "no public post → SPA shell");
    let html = body_string(resp).await;
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
    let env = backend.setup().await;
    let resp = projector_app(env.posts(), env.users(), env.themes())
        .oneshot(get("/~ghost/not-a-year/1/2/missing"))
        .await
        .expect("request");
    assert_eq!(
        resp.status(),
        StatusCode::OK,
        "non-numeric date → SPA shell"
    );
    let html = body_string(resp).await;
    assert!(html.contains("test-shell"), "served the SPA shell: {html}");
}

#[apply(backends)]
#[tokio::test]
async fn permalink_overflowing_date_serves_shell(#[case] backend: Backend) {
    let env = backend.setup().await;
    let resp = projector_app(env.posts(), env.users(), env.themes())
        .oneshot(get("/~ghost/2147483648/1/2/missing"))
        .await
        .expect("request");
    assert_eq!(
        resp.status(),
        StatusCode::OK,
        "overflowing date → SPA shell"
    );
    let html = body_string(resp).await;
    assert!(html.contains("test-shell"), "served the SPA shell: {html}");
}

#[apply(backends)]
#[tokio::test]
async fn permalink_impossible_date_serves_shell(#[case] backend: Backend) {
    let env = backend.setup().await;
    let resp = projector_app(env.posts(), env.users(), env.themes())
        .oneshot(get("/~ghost/2026/13/40/missing"))
        .await
        .expect("request");
    assert_eq!(resp.status(), StatusCode::OK, "impossible date → SPA shell");
    let html = body_string(resp).await;
    assert!(html.contains("test-shell"), "served the SPA shell: {html}");
}

#[apply(backends)]
#[tokio::test]
async fn permalink_invalid_segment_serves_shell(#[case] backend: Backend) {
    // An unparseable username segment (a dot is not allowed) is never public
    // content — serve the shell and let the client route it.
    let env = backend.setup().await;
    let resp = projector_app(env.posts(), env.users(), env.themes())
        .oneshot(get("/~in.valid/2026/1/2/slug"))
        .await
        .expect("request");
    assert_eq!(
        resp.status(),
        StatusCode::OK,
        "unparseable segment → SPA shell"
    );
    let body = body_string(resp).await;
    assert!(body.contains("test-shell"));
}

#[apply(backends)]
#[tokio::test]
async fn permalink_storage_failure_keeps_500_and_reports_boundary_once(#[case] backend: Backend) {
    let env = backend.setup().await;
    let (u, y, m, d, slug, ..) =
        seed_published_post(env.users(), env.posts(), env.write_scope()).await;
    let uri = format!("/~{u}/{y}/{m}/{d}/{slug}");
    let app = projector_app(env.posts(), env.users(), env.themes());
    env.base.close_pool().await;

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

#[apply(backends)]
#[tokio::test]
async fn permalink_site_theme_failure_keeps_500_and_reports_boundary_once(
    #[case] backend: Backend,
) {
    let env = backend.setup().await;
    let (u, y, m, d, slug, ..) =
        seed_published_post(env.users(), env.posts(), env.write_scope()).await;
    let uri = format!("/~{u}/{y}/{m}/{d}/{slug}");
    let app = projector_app_with_dependencies(
        env.posts(),
        env.users(),
        failing_site_theme_selection("injected permalink site selection failure"),
    );

    let (response, event) = crate::assert_error_signal!(
        async { app.oneshot(get(&uri)).await.expect("request") },
        event = "server function failed",
        event_kind = "Storage",
        event_class = "Bug",
        metric_kind = "storage",
        metric_class = "bug",
        disposition = "boundary",
        context = "server.projector.permalink"
    );

    assert_sanitized_internal_server_error(response).await;
    assert!(event.contains("injected permalink site selection failure"));
}

#[apply(backends)]
#[tokio::test]
async fn permalink_author_theme_failure_keeps_500_and_reports_boundary_once(
    #[case] backend: Backend,
) {
    let env = backend.setup().await;
    let (u, y, m, d, slug, ..) =
        seed_published_post(env.users(), env.posts(), env.write_scope()).await;
    let uri = format!("/~{u}/{y}/{m}/{d}/{slug}");
    let app = projector_app_with_dependencies(
        env.posts(),
        env.users(),
        failing_author_theme_selection("injected permalink author selection failure"),
    );

    let (response, event) = crate::assert_error_signal!(
        async { app.oneshot(get(&uri)).await.expect("request") },
        event = "server function failed",
        event_kind = "Storage",
        event_class = "Bug",
        metric_kind = "storage",
        metric_class = "bug",
        disposition = "boundary",
        context = "server.projector.permalink"
    );

    assert_sanitized_internal_server_error(response).await;
    assert!(event.contains("injected permalink author selection failure"));
}

#[apply(backends)]
#[tokio::test]
async fn permalink_alias_redirects_with_raw_query_and_no_store(#[case] backend: Backend) {
    let env = backend.setup().await;
    let (username, year, month, day, slug, ..) =
        seed_published_post(env.users(), env.posts(), env.write_scope()).await;
    let alias = format!("/{year:04}/{month:02}/{day:02}/{slug}?utm=%2f&utm=&tag=one&tag=two");

    let response = projector_app(env.posts(), env.users(), env.themes())
        .oneshot(get(&alias))
        .await
        .expect("request");

    assert_eq!(response.status(), StatusCode::FOUND);
    let expected_location =
        format!("/~{username}/{year:04}/{month:02}/{day:02}/{slug}?utm=%2f&utm=&tag=one&tag=two");
    assert_eq!(
        response
            .headers()
            .get(header::LOCATION)
            .and_then(|value| value.to_str().ok()),
        Some(expected_location.as_str())
    );
    assert_eq!(
        response
            .headers()
            .get(header::CACHE_CONTROL)
            .and_then(|value| value.to_str().ok()),
        Some("no-store")
    );
}

#[apply(backends)]
#[tokio::test]
async fn permalink_alias_redirect_without_query_has_no_delimiter(#[case] backend: Backend) {
    let env = backend.setup().await;
    let (username, year, month, day, slug, ..) =
        seed_published_post(env.users(), env.posts(), env.write_scope()).await;
    let alias = format!("/{year:04}/{month:02}/{day:02}/{slug}");

    let response = projector_app(env.posts(), env.users(), env.themes())
        .oneshot(get(&alias))
        .await
        .expect("request");

    let location = response
        .headers()
        .get(header::LOCATION)
        .and_then(|value| value.to_str().ok())
        .expect("redirect location");
    assert_eq!(
        location,
        format!("/~{username}/{year:04}/{month:02}/{day:02}/{slug}")
    );
    assert!(!location.contains('?'), "absent query stays absent");
}

#[apply(backends)]
#[tokio::test]
async fn permalink_alias_head_is_not_redirect_or_resolution(#[case] backend: Backend) {
    let env = backend.setup().await;
    let (_, year, month, day, slug, ..) =
        seed_published_post(env.users(), env.posts(), env.write_scope()).await;
    let alias = format!("/{year:04}/{month:02}/{day:02}/{slug}");
    let app = projector_app(env.posts(), env.users(), env.themes());
    env.base.close_pool().await;

    for method in [Method::HEAD, Method::POST] {
        let response = app
            .clone()
            .oneshot(
                Request::builder()
                    .method(method.clone())
                    .uri(&alias)
                    .body(Body::empty())
                    .expect("non-GET request"),
            )
            .await
            .expect("response");

        assert_eq!(response.status(), StatusCode::METHOD_NOT_ALLOWED);
        assert!(
            response.headers().get(header::LOCATION).is_none(),
            "{method} never redirects"
        );
        let body = axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .expect("read response body");
        assert!(body.is_empty(), "{method} rejection has no body");
    }
}

#[apply(backends)]
#[tokio::test]
async fn permalink_alias_strict_dates_and_invalid_utf8_serve_shell(#[case] backend: Backend) {
    let env = backend.setup().await;

    for path in ["/2026/7/12/slug", "/+2026/07/12/slug", "/2026/07/12/%FF"] {
        let response = projector_app(env.posts(), env.users(), env.themes())
            .oneshot(get(path))
            .await
            .expect("request");
        assert_shell_miss(response).await;
    }
}

#[apply(backends)]
#[tokio::test]
async fn permalink_alias_misses_remain_indistinguishable(#[case] backend: Backend) {
    let env = backend.setup().await;
    let now = UtcInstant::now();
    let author = SeedUser::new().seed(env.users(), env.write_scope()).await;
    let private = SeedRawPost::new(author.user_id)
        .slug("hidden-alias")
        .published_at(now)
        .audiences(vec![])
        .seed(env.posts(), env.write_scope())
        .await;
    let other = SeedUser::new().seed(env.users(), env.write_scope()).await;
    SeedRawPost::new(other.user_id)
        .slug("ambiguous-alias")
        .published_at(now)
        .seed(env.posts(), env.write_scope())
        .await;
    SeedRawPost::new(author.user_id)
        .slug("ambiguous-alias")
        .published_at(now)
        .seed(env.posts(), env.write_scope())
        .await;
    let date = Offset::UTC
        .to_datetime(private.published_at.expect("published").value())
        .date();
    let year = i32::from(date.year());
    let month = u32::try_from(date.month()).expect("month fits u32");
    let day = u32::try_from(date.day()).expect("day fits u32");

    for path in [
        format!("/{year:04}/{month:02}/{day:02}/missing-alias"),
        format!("/{year:04}/{month:02}/{day:02}/ambiguous-alias"),
        format!("/{year:04}/{month:02}/{day:02}/hidden-alias"),
        format!("/{year:04}/{month:02}/{day:02}/-malformed"),
        "/not-a-year/01/02/slug".to_owned(),
    ] {
        let response = projector_app(env.posts(), env.users(), env.themes())
            .oneshot(get(&path))
            .await
            .expect("request");
        assert_shell_miss(response).await;
    }
}

#[apply(backends)]
#[tokio::test]
async fn permalink_alias_inactive_post_serves_shell(#[case] backend: Backend) {
    let env = backend.setup().await;
    let author = SeedUser::new().seed(env.users(), env.write_scope()).await;
    let scheduled_at: UtcInstant = "2099-01-02T03:04:05Z".parse().expect("valid instant");
    let scheduled = SeedRawPost::new(author.user_id)
        .slug("scheduled-alias")
        .published_at(scheduled_at)
        .seed(env.posts(), env.write_scope())
        .await;
    let date = Offset::UTC
        .to_datetime(scheduled.published_at.expect("scheduled").value())
        .date();
    let path = format!(
        "/{:04}/{:02}/{:02}/scheduled-alias",
        i32::from(date.year()),
        u32::try_from(date.month()).expect("month fits u32"),
        u32::try_from(date.day()).expect("day fits u32")
    );

    let response = projector_app(env.posts(), env.users(), env.themes())
        .oneshot(get(&path))
        .await
        .expect("request");
    assert_shell_miss(response).await;
}

#[apply(backends)]
#[tokio::test]
async fn permalink_alias_encodes_unicode_slug_in_location(#[case] backend: Backend) {
    let env = backend.setup().await;
    let author = SeedUser::new().seed(env.users(), env.write_scope()).await;
    let post = SeedRawPost::new(author.user_id)
        .slug("café")
        .seed(env.posts(), env.write_scope())
        .await;
    let date = Offset::UTC
        .to_datetime(post.published_at.expect("published").value())
        .date();
    let path = format!(
        "/{:04}/{:02}/{:02}/café",
        i32::from(date.year()),
        u32::try_from(date.month()).expect("month fits u32"),
        u32::try_from(date.day()).expect("day fits u32")
    );

    let response = projector_app(env.posts(), env.users(), env.themes())
        .oneshot(get(&path))
        .await
        .expect("request");
    assert_eq!(response.status(), StatusCode::FOUND);
    let expected_location = format!(
        "/~{}/{:04}/{:02}/{:02}/caf%C3%A9",
        author.username,
        date.year(),
        date.month(),
        date.day()
    );
    assert_eq!(
        response
            .headers()
            .get(header::LOCATION)
            .and_then(|value| value.to_str().ok()),
        Some(expected_location.as_str())
    );
}

#[apply(backends)]
#[tokio::test]
async fn permalink_alias_storage_failure_reports_boundary_once(#[case] backend: Backend) {
    let env = backend.setup().await;
    let (_, year, month, day, slug, ..) =
        seed_published_post(env.users(), env.posts(), env.write_scope()).await;
    let app = projector_app(env.posts(), env.users(), env.themes());
    env.base.close_pool().await;

    let (response, event) = crate::assert_error_signal!(
        async {
            app.oneshot(get(&format!("/{year:04}/{month:02}/{day:02}/{slug}")))
                .await
                .expect("request")
        },
        event = "server function failed",
        event_kind = "Storage",
        event_class = "Bug",
        metric_kind = "storage",
        metric_class = "bug",
        disposition = "boundary",
        context = "server.projector.permalink_alias"
    );

    assert_sanitized_internal_server_error(response).await;
    assert!(event.contains("pool"), "typed storage source: {event}");
}
