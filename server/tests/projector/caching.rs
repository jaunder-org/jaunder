use axum::{
    body::Body,
    http::{Request, StatusCode, header},
};
use tower::ServiceExt;

use rstest::*;
use rstest_reuse::*;

use common::test_support::parse_post_body;
use jiff::tz::Offset;
use storage::test_support::{Backend, SeedRawPost, SeedUser, backends};

use super::fixtures::{get, projector_app, seed_published_post};

#[apply(backends)]
#[tokio::test]
async fn refreshed_post_changes_public_permalink_etag_but_keeps_cache_age_bound(
    #[case] backend: Backend,
) {
    let env = backend.setup().await;
    let user = SeedUser::new().seed(env.users(), env.write_scope()).await;
    let post = SeedRawPost::new(user.user_id)
        .body(parse_post_body("```elisp\n(message \"refreshed\")\n```"))
        .seed(env.posts(), env.write_scope())
        .await;
    let date = Offset::UTC
        .to_datetime(post.published_at.expect("published Post").value())
        .date();
    let uri = format!(
        "/~{}/{:04}/{:02}/{:02}/{}",
        user.username,
        date.year(),
        date.month(),
        date.day(),
        post.slug
    );
    env.base
        .pool()
        .execute("UPDATE posts SET rendered_html = '<p>old presentation</p>'")
        .await
        .expect("stale historical projection");
    let before = projector_app(env.posts(), env.users(), env.themes())
        .oneshot(get(&uri))
        .await
        .expect("public permalink before refresh");
    assert_eq!(before.status(), StatusCode::OK);
    assert_eq!(
        before.headers()[header::CACHE_CONTROL],
        "public, max-age=300"
    );
    let before_etag = before.headers()[header::ETAG].clone();

    env.refresh_current_post_projections()
        .await
        .expect("startup refresh");
    let conditional = Request::builder()
        .method("GET")
        .uri(&uri)
        .header(header::IF_NONE_MATCH, &before_etag)
        .body(Body::empty())
        .unwrap();
    let after = projector_app(env.posts(), env.users(), env.themes())
        .oneshot(conditional)
        .await
        .expect("public permalink after refresh");
    assert_eq!(after.status(), StatusCode::OK, "old ETag is stale");
    assert_eq!(
        after.headers()[header::CACHE_CONTROL],
        "public, max-age=300"
    );
    assert_ne!(after.headers()[header::ETAG], before_etag);
    let html = axum::body::to_bytes(after.into_body(), usize::MAX)
        .await
        .expect("projected HTML");
    assert!(String::from_utf8_lossy(&html).contains("j-syn-"));
}

#[apply(backends)]
#[tokio::test]
async fn permalink_stale_if_none_match_serves_full_200(#[case] backend: Backend) {
    // A non-matching `If-None-Match` must not 304 — the client's cached copy is
    // stale, so serve the full document.
    let env = backend.setup().await;
    let (u, y, m, d, slug, ..) =
        seed_published_post(env.users(), env.posts(), env.write_scope()).await;
    let uri = format!("/~{u}/{y}/{m}/{d}/{slug}");
    let req = Request::builder()
        .method("GET")
        .uri(&uri)
        .header(header::IF_NONE_MATCH, "\"sha256-stale\"")
        .body(Body::empty())
        .unwrap();
    let resp = projector_app(env.posts(), env.users(), env.themes())
        .oneshot(req)
        .await
        .expect("request");
    assert_eq!(resp.status(), StatusCode::OK, "stale ETag → full 200");
}

#[apply(backends)]
#[tokio::test]
async fn permalink_if_none_match_returns_304(#[case] backend: Backend) {
    let env = backend.setup().await;
    let (u, y, m, d, slug, ..) =
        seed_published_post(env.users(), env.posts(), env.write_scope()).await;
    let uri = format!("/~{u}/{y}/{m}/{d}/{slug}");

    let resp = projector_app(env.posts(), env.users(), env.themes())
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
    let resp = projector_app(env.posts(), env.users(), env.themes())
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
    let env = backend.setup().await;
    let (u, y, m, d, slug, ..) =
        seed_published_post(env.users(), env.posts(), env.write_scope()).await;
    let uri = format!("/~{u}/{y}/{m}/{d}/{slug}");
    let anon = axum::body::to_bytes(
        projector_app(env.posts(), env.users(), env.themes())
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
        projector_app(env.posts(), env.users(), env.themes())
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
    let env = backend.setup().await;
    let (u, y, m, d, slug, ..) =
        seed_published_post(env.users(), env.posts(), env.write_scope()).await;
    let uri = format!("/~{u}/{y}/{m}/{d}/{slug}");
    let resp = projector_app(env.posts(), env.users(), env.themes())
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

#[apply(backends)]
#[tokio::test]
async fn timeline_order_urls_produce_distinct_cacheable_representations(#[case] backend: Backend) {
    let env = backend.setup().await;
    seed_published_post(env.users(), env.posts(), env.write_scope()).await;

    let newest = projector_app(env.posts(), env.users(), env.themes())
        .oneshot(get("/"))
        .await
        .expect("newest request");
    let newest_etag = newest.headers().get(header::ETAG).cloned();
    let newest_body = axum::body::to_bytes(newest.into_body(), usize::MAX)
        .await
        .expect("newest body");

    let oldest = projector_app(env.posts(), env.users(), env.themes())
        .oneshot(get("/?order=oldest"))
        .await
        .expect("oldest request");
    let oldest_etag = oldest.headers().get(header::ETAG).cloned();
    let oldest_body = axum::body::to_bytes(oldest.into_body(), usize::MAX)
        .await
        .expect("oldest body");

    assert_ne!(
        newest_body, oldest_body,
        "the seed binds the complete URL order"
    );
    assert_ne!(
        newest_etag, oldest_etag,
        "ordered variants must not share an ETag"
    );
}
