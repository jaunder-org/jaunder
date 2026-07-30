use axum::http::StatusCode;
use common::visibility::ViewerIdentity;
use server_fn::ServerFn;

use rstest::*;
use rstest_reuse::*;

use crate::helpers::{create_user_and_session, post_form};
use storage::test_support::{backends, Backend, SeedUser, TestEnv};

// Authed subscribe makes `is_subscriber` true; unsubscribe reverses it.
#[apply(backends)]
#[tokio::test]
async fn subscribe_then_unsubscribe_round_trips(#[case] backend: Backend) {
    let TestEnv { state, base: _base } = backend.setup().await;
    let author = SeedUser::new().seed(&state).await;
    let subscriber = create_user_and_session(&state).await;
    let cookie = subscriber.cookie();
    let channel = state.subscriptions.local_channel_id().await.unwrap();
    let viewer = ViewerIdentity::local(subscriber.user_id, channel);

    let (status, body) = post_form(
        &state,
        <web::subscriptions::Subscribe as ServerFn>::PATH,
        format!("author_username={}", author.username),
        Some(&cookie),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "subscribe failed: {body}");
    assert!(
        state
            .subscriptions
            .is_subscriber(author.user_id, &viewer)
            .await
            .unwrap(),
        "is_subscriber should be true after subscribe"
    );

    let (status, body) = post_form(
        &state,
        <web::subscriptions::Unsubscribe as ServerFn>::PATH,
        format!("author_username={}", author.username),
        Some(&cookie),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "unsubscribe failed: {body}");
    assert!(
        !state
            .subscriptions
            .is_subscriber(author.user_id, &viewer)
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
    let me = create_user_and_session(&state).await;
    let cookie = me.cookie();
    let channel = state.subscriptions.local_channel_id().await.unwrap();

    let (status, _body) = post_form(
        &state,
        <web::subscriptions::Subscribe as ServerFn>::PATH,
        format!("author_username={}", me.username),
        Some(&cookie),
    )
    .await;
    assert_ne!(status, StatusCode::OK, "self-subscribe must be rejected");
    assert!(
        !state
            .subscriptions
            .is_subscriber(me.user_id, &ViewerIdentity::local(me.user_id, channel))
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
    let author = SeedUser::new().seed(&state).await;

    let (status, _body) = post_form(
        &state,
        <web::subscriptions::Subscribe as ServerFn>::PATH,
        format!("author_username={}", author.username),
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
    let author = SeedUser::new().seed(&state).await;
    let cookie = create_user_and_session(&state).await.cookie();

    let (status, body) = post_form(
        &state,
        <web::subscriptions::IsSubscribed as ServerFn>::PATH,
        format!("author_username={}", author.username),
        Some(&cookie),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert!(
        body.contains("false"),
        "should not be subscribed yet: {body}"
    );

    post_form(
        &state,
        <web::subscriptions::Subscribe as ServerFn>::PATH,
        format!("author_username={}", author.username),
        Some(&cookie),
    )
    .await;

    let (status, body) = post_form(
        &state,
        <web::subscriptions::IsSubscribed as ServerFn>::PATH,
        format!("author_username={}", author.username),
        Some(&cookie),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert!(body.contains("true"), "should be subscribed now: {body}");
}
