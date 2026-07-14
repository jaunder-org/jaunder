use std::sync::Arc;

use axum::{
    body::Body,
    extract::Extension,
    http::{header, Request, StatusCode},
    Router,
};
use chrono::{Datelike, Utc};
use common::password::Password;
use common::slug::Slug;
use common::username::Username;
use common::visibility::AudienceTarget;
use storage::{CreatePostInput, PostFormat};
use tower::ServiceExt;

use rstest::*;
use rstest_reuse::*;

use storage::test_support::{backends, Backend, TestEnv};

/// A recognizable stand-in for the real `index.html`, so tests can tell a
/// shell-fallback response apart from a projected one.
const TEST_SHELL: &str = "<!DOCTYPE html><!--test-shell--><html><body></body></html>";

/// A router carrying only the public projector routes plus the posts store.
///
/// The projector is feature-independent (mounted into the live router only under
/// `csr`, but `register` itself always compiles), so registering it onto a bare
/// router exercises it directly under the default test build — no `csr` feature,
/// no full `create_router`.
fn projector_app(state: &Arc<storage::AppState>) -> Router {
    let shell = jaunder::projector::Shell(TEST_SHELL.into());
    jaunder::projector::register(Router::new(), shell)
        .layer(Extension(state.posts.clone()))
        .layer(Extension(state.users.clone()))
}

/// Seed a published, `rust`-tagged post for `alice`.
async fn seed_tagged_post(state: &Arc<storage::AppState>) {
    let username: Username = "alice".parse().unwrap();
    let password: Password = "password123".parse().unwrap();
    let user_id = state
        .users
        .create_user(&username, &password, None, false)
        .await
        .unwrap();
    let post_id = state
        .posts
        .create_post(&CreatePostInput {
            user_id,
            title: Some("Tagged Post".to_string()),
            slug: "tagged".parse::<Slug>().unwrap(),
            body: "b".to_string(),
            format: PostFormat::Markdown,
            rendered_html: "<p>tag body</p>".to_string(),
            published_at: Some(Utc::now()),
            summary: None,
            audiences: vec![AudienceTarget::Public],
            idempotency_key: None,
        })
        .await
        .unwrap();
    state.posts.tag_post(post_id, "rust").await.unwrap();
}

/// Seed a published post for `alice` and return the permalink components.
async fn seed_published_post(state: &Arc<storage::AppState>) -> (String, i32, u32, u32, String) {
    let username: Username = "alice".parse().unwrap();
    let password: Password = "password123".parse().unwrap();
    let user_id = state
        .users
        .create_user(&username, &password, None, false)
        .await
        .unwrap();
    let now = Utc::now();
    state
        .posts
        .create_post(&CreatePostInput {
            user_id,
            title: Some("Hello World".to_string()),
            slug: "hello".parse::<Slug>().unwrap(),
            body: "Body".to_string(),
            format: PostFormat::Markdown,
            rendered_html: "<p>Body here</p>".to_string(),
            published_at: Some(now),
            summary: None,
            audiences: vec![AudienceTarget::Public],
            idempotency_key: None,
        })
        .await
        .unwrap();
    (
        "alice".to_string(),
        now.year(),
        now.month(),
        now.day(),
        "hello".to_string(),
    )
}

fn get(uri: &str) -> Request<Body> {
    Request::builder()
        .method("GET")
        .uri(uri)
        .body(Body::empty())
        .unwrap()
}

#[apply(backends)]
#[tokio::test]
async fn permalink_projects_cacheable_crawlable_html(#[case] backend: Backend) {
    let TestEnv { state, base: _base } = backend.setup().await;
    let (u, y, m, d, slug) = seed_published_post(&state).await;
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
    assert!(html.contains("Hello World"), "title present: {html}");
    assert!(
        html.contains("<p>Body here</p>"),
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
async fn permalink_stale_if_none_match_serves_full_200(#[case] backend: Backend) {
    // A non-matching `If-None-Match` must not 304 — the client's cached copy is
    // stale, so serve the full document.
    let TestEnv { state, base: _base } = backend.setup().await;
    let (u, y, m, d, slug) = seed_published_post(&state).await;
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
    let (u, y, m, d, slug) = seed_published_post(&state).await;
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
async fn profile_projects_user_timeline(#[case] backend: Backend) {
    let TestEnv { state, base: _base } = backend.setup().await;
    let (u, ..) = seed_published_post(&state).await;
    let resp = projector_app(&state)
        .oneshot(get(&format!("/~{u}")))
        .await
        .expect("request");
    assert_eq!(resp.status(), StatusCode::OK, "profile → 200");
    let body = axum::body::to_bytes(resp.into_body(), usize::MAX)
        .await
        .unwrap();
    let html = String::from_utf8_lossy(&body);
    assert!(html.contains("Posts by alice"), "profile heading: {html}");
    assert!(html.contains("Hello World"), "post title present");
    assert!(html.contains(r#"id="jaunder-seed""#), "data blob present");
}

#[apply(backends)]
#[tokio::test]
async fn site_timeline_projects_local_posts(#[case] backend: Backend) {
    let TestEnv { state, base: _base } = backend.setup().await;
    seed_published_post(&state).await;
    let resp = projector_app(&state)
        .oneshot(get("/"))
        .await
        .expect("request");
    assert_eq!(resp.status(), StatusCode::OK, "root site timeline → 200");
    let body = axum::body::to_bytes(resp.into_body(), usize::MAX)
        .await
        .unwrap();
    let html = String::from_utf8_lossy(&body);
    assert!(html.contains("Hello World"), "post present: {html}");
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
async fn site_tag_projects_tagged_posts(#[case] backend: Backend) {
    let TestEnv { state, base: _base } = backend.setup().await;
    seed_tagged_post(&state).await;
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
    assert!(html.contains("Tagged Post"), "tagged post present");
    assert!(html.contains(r#"id="jaunder-seed""#), "data blob present");
}

#[apply(backends)]
#[tokio::test]
async fn user_tag_projects_tagged_posts(#[case] backend: Backend) {
    let TestEnv { state, base: _base } = backend.setup().await;
    seed_tagged_post(&state).await;
    let resp = projector_app(&state)
        .oneshot(get("/~alice/tags/rust"))
        .await
        .expect("request");
    assert_eq!(resp.status(), StatusCode::OK, "user tag → 200");
    let body = axum::body::to_bytes(resp.into_body(), usize::MAX)
        .await
        .unwrap();
    let html = String::from_utf8_lossy(&body);
    assert!(html.contains("Tagged Post"), "tagged post present: {html}");
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

#[apply(backends)]
#[tokio::test]
async fn projected_bytes_ignore_request_auth(#[case] backend: Backend) {
    // Cacheability invariant: the projector never branches on the viewer, so a
    // request carrying a session cookie yields byte-identical output to an
    // anonymous one — one cacheable response for every visitor.
    let TestEnv { state, base: _base } = backend.setup().await;
    let (u, y, m, d, slug) = seed_published_post(&state).await;
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
    let (u, y, m, d, slug) = seed_published_post(&state).await;
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
