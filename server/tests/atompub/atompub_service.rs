use axum::{
    body::Body,
    http::{Method, Request, StatusCode, header},
    response::Response,
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
};
use storage::test_support::{Backend, TestEnv, backends};

fn assert_basic_challenge(response: &Response) {
    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
    assert_eq!(
        response
            .headers()
            .get(header::WWW_AUTHENTICATE)
            .and_then(|value| value.to_str().ok()),
        Some(r#"Basic realm="Jaunder AtomPub""#)
    );
    assert_eq!(
        response
            .headers()
            .get_all(header::WWW_AUTHENTICATE)
            .iter()
            .count(),
        1
    );
}

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
        email_verifications: state.email_verifications.clone(),
        password_resets: state.password_resets.clone(),
        posts: state.posts.clone(),
        subscriptions: state.subscriptions.clone(),
        audiences: state.audiences.clone(),
        media: state.media.clone(),
        user_config: state.user_config.clone(),
        feed_cache: state.feed_cache.clone(),
        feed_events: state.feed_events.clone(),
        publisher: state.publisher.clone(),
        write_scope: state.write_scope.clone(),
    })
}

fn with_sessions(
    state: &Arc<storage::AppState>,
    sessions: Arc<dyn storage::SessionStorage>,
) -> Arc<storage::AppState> {
    Arc::new(storage::AppState {
        site_config: state.site_config.clone(),
        users: state.users.clone(),
        sessions,
        invites: state.invites.clone(),
        email_verifications: state.email_verifications.clone(),
        password_resets: state.password_resets.clone(),
        posts: state.posts.clone(),
        subscriptions: state.subscriptions.clone(),
        audiences: state.audiences.clone(),
        media: state.media.clone(),
        user_config: state.user_config.clone(),
        feed_cache: state.feed_cache.clone(),
        feed_events: state.feed_events.clone(),
        publisher: state.publisher.clone(),
        write_scope: state.write_scope.clone(),
    })
}

#[apply(backends)]
#[tokio::test]
async fn service_document_returns_200_with_app_password(#[case] backend: Backend) {
    let TestEnv { state, base } = backend.setup().await;
    let session = create_user_and_session(&state).await;
    let name: &str = &session.username;
    // Give the user a tagged post so the service document's category list is
    // non-empty (exercises the tag-collection path in `service_document`).
    let post = session.seed_post().seed(&state).await;
    storage::test_support::set_post_tags_confirmed(
        &state.write_scope,
        std::sync::Arc::clone(&state.posts),
        post.post_id,
        session.user_id,
        &["rust".parse::<TagLabel>().unwrap()],
    )
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
    assert!(response.headers().get(header::WWW_AUTHENTICATE).is_none());
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
async fn service_document_omits_media_when_uploads_are_disabled(#[case] backend: Backend) {
    let TestEnv { state, base } = backend.setup().media_uploads_enabled(false).await;
    let session = create_user_and_session(&state).await;
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
    let body = body_string(response).await;
    assert!(
        !body.contains(&format!(
            "https://example.com/atompub/{}/media",
            session.username
        )),
        "disabled media collection leaked into service document: {body}"
    );
    assert!(
        body.contains(&format!(
            "https://example.com/atompub/{}/posts",
            session.username
        )),
        "posts collection missing from service document: {body}"
    );
}
#[apply(backends)]
#[tokio::test]
async fn explicit_basic_identity_wins_and_expires_simultaneous_cookie(#[case] backend: Backend) {
    let TestEnv { state, base } = backend.setup().await;
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

    assert_basic_challenge(&response);
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

    assert_basic_challenge(&response);
}

#[apply(backends)]
#[tokio::test]
async fn service_document_requires_basic_challenge_without_authentication(
    #[case] backend: Backend,
) {
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

    assert_basic_challenge(&response);
    assert!(
        body_string(response).await.is_empty(),
        "authentication rejection body stays empty"
    );
}

#[apply(backends)]
#[tokio::test]
async fn service_document_challenges_explicit_authentication_failures_without_cookie_fallback(
    #[case] backend: Backend,
) {
    let TestEnv { state, base } = backend.setup().await;
    let cookie_session = create_user_and_session(&state).await;
    let credential_session = create_user_and_session(&state).await;
    let app = make_app(&state, &base);
    let uri = parse_root_relative_url("/atompub/service");

    for authorization in ["Basic not-base64", "Digest credentials"] {
        let response = app
            .clone()
            .oneshot(
                Request::builder()
                    .method(Method::GET)
                    .uri(uri.as_ref())
                    .header(header::AUTHORIZATION, authorization)
                    .header(header::COOKIE, cookie_session.cookie())
                    .body(Body::empty())
                    .expect("build explicit authentication rejection request"),
            )
            .await
            .expect("request");
        assert_basic_challenge(&response);
    }

    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .method(Method::GET)
                .uri(uri.as_ref())
                .header(
                    header::AUTHORIZATION,
                    format!("Bearer {}", host::token::generate()),
                )
                .header(header::COOKIE, cookie_session.cookie())
                .body(Body::empty())
                .expect("build unknown credential request"),
        )
        .await
        .expect("request");
    assert_basic_challenge(&response);

    let token_hash = host::token::hash(&credential_session.token).expect("hash credential token");
    let sessions = Arc::clone(&state.sessions);
    let outcome = state
        .write_scope
        .run(|transaction| {
            Box::pin(async move { sessions.revoke_session(transaction, &token_hash).await })
        })
        .await
        .expect("revoke credential");
    storage::test_support::confirmed_for(outcome, "credential revocation");
    let response = app
        .oneshot(
            atompub_at(&credential_session, Method::GET, &uri)
                .header(header::COOKIE, cookie_session.cookie())
                .body(Body::empty())
                .expect("build revoked credential request"),
        )
        .await
        .expect("request");
    assert_basic_challenge(&response);
}

#[apply(backends)]
#[tokio::test]
async fn service_document_accepts_bearer_and_cookie_without_basic_challenge(
    #[case] backend: Backend,
) {
    let TestEnv { state, base } = backend.setup().await;
    let session = create_user_and_session(&state).await;
    let uri = parse_root_relative_url("/atompub/service");

    let bearer_response = make_app(&state, &base)
        .oneshot(
            Request::builder()
                .method(Method::GET)
                .uri(uri.as_ref())
                .header(header::AUTHORIZATION, format!("Bearer {}", session.token))
                .body(Body::empty())
                .expect("build bearer request"),
        )
        .await
        .expect("bearer request");
    assert_eq!(bearer_response.status(), StatusCode::OK);
    assert!(
        bearer_response
            .headers()
            .get(header::WWW_AUTHENTICATE)
            .is_none()
    );

    let cookie_response = make_app(&state, &base)
        .oneshot(
            Request::builder()
                .method(Method::GET)
                .uri(uri.as_ref())
                .header(header::COOKIE, session.cookie())
                .body(Body::empty())
                .expect("build cookie request"),
        )
        .await
        .expect("cookie request");
    assert_eq!(cookie_response.status(), StatusCode::OK);

    assert!(
        cookie_response
            .headers()
            .get(header::WWW_AUTHENTICATE)
            .is_none()
    );
}
// guard:no-backend — injected authentication storage failure before HTTP projection
#[tokio::test]
async fn service_document_authentication_storage_error_keeps_500_without_basic_challenge() {
    let TestEnv { state, base } = Backend::Sqlite.setup().await;
    let mut sessions = storage::MockSessionStorage::new();
    sessions
        .expect_authenticate()
        .times(1)
        .return_once(|_, _| Err(storage::SessionAuthError::Internal(sqlx::Error::PoolClosed)));
    let state = with_sessions(&state, Arc::new(sessions));

    let response = make_app(&state, &base)
        .oneshot(
            Request::builder()
                .uri("/atompub/service")
                .header(
                    header::AUTHORIZATION,
                    format!("Bearer {}", host::token::generate()),
                )
                .body(Body::empty())
                .expect("build request"),
        )
        .await
        .expect("request");

    assert_eq!(response.status(), StatusCode::INTERNAL_SERVER_ERROR);
    assert!(response.headers().get(header::WWW_AUTHENTICATE).is_none());
    assert!(body_string(response).await.is_empty());
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
    let TestEnv { state, base } = backend.setup().base_url(None).await;
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
    assert!(response.headers().get(header::WWW_AUTHENTICATE).is_none());
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
    assert!(response.headers().get(header::WWW_AUTHENTICATE).is_none());
    assert!(
        body_string(response).await.is_empty(),
        "500 body stays masked"
    );
}
