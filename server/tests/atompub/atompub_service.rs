use axum::{
    body::Body,
    http::{Method, Request, StatusCode, header},
};
use common::tag::TagLabel;
use common::test_support::{parse_root_relative_url, parse_username};
use rstest::*;
use rstest_reuse::*;
use std::error::Error;
use std::sync::Arc;
use tower::ServiceExt;

use crate::helpers::{
    atompub_at, atompub_authed, atompub_xml, body_string, create_user_and_session, make_app,
    setup_with_base_url,
};
use storage::test_support::{Backend, TestEnv, backends};

fn collection_xml<'a>(body: &'a str, href: &str) -> &'a str {
    let opening = format!(r#"<app:collection href="{href}">"#);
    body.split_once(&opening)
        .unwrap()
        .1
        .split_once("</app:collection>")
        .unwrap()
        .0
}

fn accept_values(collection: &str) -> Vec<&str> {
    collection
        .split("<app:accept>")
        .skip(1)
        .map(|rest| rest.split_once("</app:accept>").unwrap().0)
        .collect()
}

fn with_site_config(
    state: &Arc<storage::AppState>,
    site_config: Arc<dyn storage::SiteConfigStorage>,
) -> Arc<storage::AppState> {
    Arc::new(storage::AppState {
        site_config,
        users: state.users.clone(),
        sessions: state.sessions.clone(),
        invites: state.invites.clone(),
        atomic: state.atomic.clone(),
        email_verifications: state.email_verifications.clone(),
        password_resets: state.password_resets.clone(),
        posts: state.posts.clone(),
        subscriptions: state.subscriptions.clone(),
        audiences: state.audiences.clone(),
        media: state.media.clone(),
        user_config: state.user_config.clone(),
        feed_cache: state.feed_cache.clone(),
        feed_events: state.feed_events.clone(),
    })
}

#[apply(backends)]
#[tokio::test]
async fn service_document_returns_200_with_app_password(#[case] backend: Backend) {
    let TestEnv { state, base } = setup_with_base_url(backend).await;
    let session = create_user_and_session(&state).await;
    let name: &str = &session.username;
    // Give the user a tagged post so the service document's category list is
    // non-empty (exercises the tag-collection path in `service_document`).
    let post = session.seed_post().seed(&state).await;
    state
        .posts
        .set_post_tags(post.post_id, &["rust".parse::<TagLabel>().unwrap()])
        .await
        .unwrap();
    let app = make_app(&state, &base);
    let uri = parse_root_relative_url("/atompub/service");

    let response = app
        .oneshot(
            atompub_at(&session, Method::GET, &uri)
                .body(Body::empty())
                .expect("failed to build atompub GET request"),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);
    let ctype = response
        .headers()
        .get(header::CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .unwrap_or_default()
        .to_string();
    assert!(
        ctype.contains("application/atomsvc+xml"),
        "content-type was {ctype}"
    );
    let body = body_string(response).await;
    assert!(body.contains("app:service"));
    let posts = collection_xml(&body, &format!("https://example.com/atompub/{name}/posts"));
    let media = collection_xml(&body, &format!("https://example.com/atompub/{name}/media"));
    assert_eq!(
        accept_values(posts),
        vec!["application/atom+xml;type=entry"]
    );
    assert_eq!(accept_values(media), vec!["*/*"]);
    assert!(!media.contains("image/"), "media collection: {media}");
    // The tagged post surfaces as an inline category in the posts collection.
    assert!(
        posts.contains("term=\"rust\""),
        "categories missing: {posts}"
    );
    // Capability discovery (ADR-0023): the service document advertises the
    // Jaunder wire extensions this server understands.
    assert!(body.contains("j:extension"), "j:extension missing: {body}");
    assert!(
        body.contains("features=\"format-media-type slug\""),
        "extension features missing: {body}"
    );
}
#[apply(backends)]
#[tokio::test]
async fn explicit_basic_identity_wins_and_expires_simultaneous_cookie(#[case] backend: Backend) {
    let TestEnv { state, base } = setup_with_base_url(backend).await;
    let alice = create_user_and_session(&state).await;
    let bob = create_user_and_session(&state).await;
    let app = make_app(&state, &base);
    let uri = parse_root_relative_url("/atompub/service");

    let response = app
        .oneshot(
            atompub_at(&bob, Method::GET, &uri)
                .header(header::COOKIE, alice.cookie())
                .body(Body::empty())
                .expect("failed to build atompub GET request"),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(
        response
            .headers()
            .get(header::SET_COOKIE)
            .and_then(|value| value.to_str().ok()),
        Some("session=; HttpOnly; SameSite=Lax; Path=/; Max-Age=0")
    );
    let body = body_string(response).await;
    assert!(body.contains(&format!(
        "https://example.com/atompub/{}/posts",
        bob.username
    )));
    assert!(!body.contains(&format!(
        "https://example.com/atompub/{}/posts",
        alice.username
    )));
}

#[apply(backends)]
#[tokio::test]
async fn explicit_basic_identity_mismatch_does_not_expire_valid_cookie(#[case] backend: Backend) {
    let TestEnv { state, base } = backend.setup().await;
    let alice = create_user_and_session(&state).await;
    let bob = create_user_and_session(&state).await;
    let app = make_app(&state, &base);
    let uri = parse_root_relative_url("/atompub/service");
    let username = parse_username("mallory");

    let response = app
        .oneshot(
            atompub_authed(Method::GET, &uri, &username, &bob.token)
                .header(header::COOKIE, alice.cookie())
                .body(Body::empty())
                .expect("failed to build atompub GET request"),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
    assert!(response.headers().get(header::SET_COOKIE).is_none());
}

#[apply(backends)]
#[tokio::test]
async fn service_document_rejects_basic_username_mismatch(#[case] backend: Backend) {
    let TestEnv { state, base } = backend.setup().await;
    let session = create_user_and_session(&state).await;
    let app = make_app(&state, &base);
    let uri = parse_root_relative_url("/atompub/service");
    let username = parse_username("mallory");

    // Correct token, but the Basic username does not match the session's user.
    let response = app
        .oneshot(atompub_xml(
            Method::GET,
            &uri,
            &username,
            &session.token,
            None,
        ))
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
}

#[apply(backends)]
#[tokio::test]
async fn service_document_requires_authentication(#[case] backend: Backend) {
    let TestEnv { state, base } = backend.setup().await;
    let app = make_app(&state, &base);

    let response = app
        .oneshot(
            Request::builder()
                .uri("/atompub/service")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
}

// guard:no-backend — injected storage failure before HTTP projection
#[tokio::test]
async fn required_base_url_preserves_storage_error_source() {
    let mut site_config = storage::MockSiteConfigStorage::new();
    site_config
        .expect_get_identity()
        .times(1)
        .return_once(|| Err(sqlx::Error::PoolClosed));

    let error = jaunder::atompub::required_base_url(&site_config)
        .await
        .expect_err("storage failure is not an unconfigured base URL");

    let source = error
        .source()
        .and_then(|source| source.downcast_ref::<sqlx::Error>())
        .expect("typed sqlx source");
    assert!(matches!(source, sqlx::Error::PoolClosed));
}

#[apply(backends)]
#[tokio::test]
async fn service_document_unconfigured_base_url_keeps_documented_500(#[case] backend: Backend) {
    let TestEnv { state, base } = backend.setup().await;
    let session = create_user_and_session(&state).await;
    let uri = parse_root_relative_url("/atompub/service");
    let response = make_app(&state, &base)
        .oneshot(
            atompub_at(&session, Method::GET, &uri)
                .body(Body::empty())
                .expect("build request"),
        )
        .await
        .expect("request");

    assert_eq!(response.status(), StatusCode::INTERNAL_SERVER_ERROR);
    assert!(
        body_string(response).await.is_empty(),
        "500 body stays masked"
    );
}

#[apply(backends)]
#[tokio::test]
async fn service_document_identity_storage_error_keeps_500_and_is_not_absence(
    #[case] backend: Backend,
) {
    let TestEnv { state, base } = backend.setup().await;
    let session = create_user_and_session(&state).await;
    let mut failing = storage::MockSiteConfigStorage::new();
    failing
        .expect_get_identity()
        .times(1)
        .return_once(|| Err(sqlx::Error::PoolClosed));
    let state = with_site_config(&state, Arc::new(failing));
    let uri = parse_root_relative_url("/atompub/service");

    let response = make_app(&state, &base)
        .oneshot(
            atompub_at(&session, Method::GET, &uri)
                .body(Body::empty())
                .expect("build request"),
        )
        .await
        .expect("request");

    assert_eq!(response.status(), StatusCode::INTERNAL_SERVER_ERROR);
    assert!(
        body_string(response).await.is_empty(),
        "500 body stays masked"
    );
}
