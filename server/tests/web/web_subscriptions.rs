use std::sync::Arc;

use axum::{
    body::Body,
    http::{header, Request, StatusCode},
};
use common::username::Username;
use common::visibility::ViewerIdentity;
use tower::ServiceExt;

use rstest::*;
#[allow(clippy::single_component_path_imports)]
use rstest_reuse;
use rstest_reuse::*;

use crate::helpers::{backends, ensure_server_fns_registered, test_options, Backend, TestEnv};

async fn post_form(
    state: Arc<storage::AppState>,
    uri: &str,
    body: impl Into<String>,
    cookie: Option<&str>,
) -> (StatusCode, String) {
    ensure_server_fns_registered();

    let mut builder = Request::builder()
        .method("POST")
        .uri(uri)
        .header(header::CONTENT_TYPE, "application/x-www-form-urlencoded");
    if let Some(c) = cookie {
        builder = builder.header(header::COOKIE, c);
    }
    let request = builder.body(Body::from(body.into())).unwrap();

    let app = jaunder::create_router(
        test_options(),
        state,
        crate::helpers::noop_mailer(),
        true,
        crate::helpers::tmp_storage_path(),
    );
    let response = app.oneshot(request).await.unwrap();
    let status = response.status();
    let bytes = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .unwrap();
    (status, String::from_utf8(bytes.to_vec()).unwrap())
}

async fn make_user(state: &Arc<storage::AppState>, name: &str) -> i64 {
    state
        .users
        .create_user(
            &name.parse::<Username>().unwrap(),
            &"password123".parse().unwrap(),
            None,
            false,
        )
        .await
        .unwrap()
}

async fn cookie_for(state: &Arc<storage::AppState>, user_id: i64) -> String {
    let token = state
        .sessions
        .create_session(user_id, "test session")
        .await
        .unwrap();
    format!("session={token}")
}

// Authed subscribe makes `is_subscriber` true; unsubscribe reverses it.
#[apply(backends)]
#[tokio::test]
async fn subscribe_then_unsubscribe_round_trips(#[case] backend: Backend) {
    let TestEnv { state, base: _base } = backend.setup().await;
    let author = make_user(&state, "author").await;
    let subscriber = make_user(&state, "subscriber").await;
    let cookie = cookie_for(&state, subscriber).await;
    let channel = state.subscriptions.local_channel_id().await.unwrap();
    let viewer = ViewerIdentity::local(subscriber, channel);

    let (status, body) = post_form(
        Arc::clone(&state),
        "/api/subscribe_to",
        "author_username=author",
        Some(&cookie),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "subscribe failed: {body}");
    assert!(
        state
            .subscriptions
            .is_subscriber(author, &viewer)
            .await
            .unwrap(),
        "is_subscriber should be true after subscribe"
    );

    let (status, body) = post_form(
        Arc::clone(&state),
        "/api/unsubscribe_from",
        "author_username=author",
        Some(&cookie),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "unsubscribe failed: {body}");
    assert!(
        !state
            .subscriptions
            .is_subscriber(author, &viewer)
            .await
            .unwrap(),
        "is_subscriber should be false after unsubscribe"
    );
}

// Self-subscribe is rejected (and creates no subscription).
#[apply(backends)]
#[tokio::test]
async fn self_subscribe_is_rejected(#[case] backend: Backend) {
    let TestEnv { state, base: _base } = backend.setup().await;
    let me = make_user(&state, "narcissus").await;
    let cookie = cookie_for(&state, me).await;
    let channel = state.subscriptions.local_channel_id().await.unwrap();

    let (status, _body) = post_form(
        Arc::clone(&state),
        "/api/subscribe_to",
        "author_username=narcissus",
        Some(&cookie),
    )
    .await;
    assert_ne!(status, StatusCode::OK, "self-subscribe must be rejected");
    assert!(
        !state
            .subscriptions
            .is_subscriber(me, &ViewerIdentity::local(me, channel))
            .await
            .unwrap(),
        "no self-subscription row may be created"
    );
}

// Subscribe requires authentication.
#[apply(backends)]
#[tokio::test]
async fn subscribe_unauthenticated_is_rejected(#[case] backend: Backend) {
    let TestEnv { state, base: _base } = backend.setup().await;
    make_user(&state, "author").await;

    let (status, _body) = post_form(
        Arc::clone(&state),
        "/api/subscribe_to",
        "author_username=author",
        None,
    )
    .await;
    assert_eq!(status, StatusCode::INTERNAL_SERVER_ERROR);
}

// is_subscribed_to reflects the current subscription state.
#[apply(backends)]
#[tokio::test]
async fn is_subscribed_to_reports_state(#[case] backend: Backend) {
    let TestEnv { state, base: _base } = backend.setup().await;
    let _author = make_user(&state, "author").await;
    let subscriber = make_user(&state, "subscriber").await;
    let cookie = cookie_for(&state, subscriber).await;

    let (status, body) = post_form(
        Arc::clone(&state),
        "/api/is_subscribed_to",
        "author_username=author",
        Some(&cookie),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert!(
        body.contains("false"),
        "should not be subscribed yet: {body}"
    );

    post_form(
        Arc::clone(&state),
        "/api/subscribe_to",
        "author_username=author",
        Some(&cookie),
    )
    .await;

    let (status, body) = post_form(
        Arc::clone(&state),
        "/api/is_subscribed_to",
        "author_username=author",
        Some(&cookie),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert!(body.contains("true"), "should be subscribed now: {body}");
}
