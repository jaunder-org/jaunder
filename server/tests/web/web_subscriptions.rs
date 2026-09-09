use axum::http::StatusCode;
use common::visibility::ViewerIdentity;
use server_fn::ServerFn;

use rstest::*;
use rstest_reuse::*;

use crate::helpers::{create_user_and_session, make_app, post_form};
use storage::test_support::{Backend, SeedUser, backends};

// Authed subscribe makes `is_subscriber` true; unsubscribe reverses it.
#[apply(backends)]
#[tokio::test]
async fn subscribe_then_unsubscribe_round_trips(#[case] backend: Backend) {
    let env = backend.setup().await;
    let app = make_app!(&env, &env.base);
    let author = SeedUser::new()
        .seed(std::sync::Arc::clone(&env.users()), env.write_scope())
        .await;
    let subscriber = create_user_and_session(
        std::sync::Arc::clone(&env.users()),
        std::sync::Arc::clone(&env.sessions()),
        env.write_scope(),
    )
    .await;
    let cookie = subscriber.cookie();
    let viewer = ViewerIdentity::local(subscriber.user_id);

    let (status, body) = post_form(
        app.clone(),
        <web::subscriptions::Subscribe as ServerFn>::PATH,
        format!("author_username={}", author.username),
        Some(&cookie),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "subscribe failed: {body}");
    assert!(
        env.subscriptions()
            .is_subscriber(author.user_id, &viewer)
            .await
            .unwrap(),
        "is_subscriber should be true after subscribe"
    );

    let (status, body) = post_form(
        app.clone(),
        <web::subscriptions::Unsubscribe as ServerFn>::PATH,
        format!("author_username={}", author.username),
        Some(&cookie),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "unsubscribe failed: {body}");
    assert!(
        !env.subscriptions()
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
    let env = backend.setup().await;
    let app = make_app!(&env, &env.base);
    let me = create_user_and_session(
        std::sync::Arc::clone(&env.users()),
        std::sync::Arc::clone(&env.sessions()),
        env.write_scope(),
    )
    .await;
    let cookie = me.cookie();

    let (status, _body) = post_form(
        app.clone(),
        <web::subscriptions::Subscribe as ServerFn>::PATH,
        format!("author_username={}", me.username),
        Some(&cookie),
    )
    .await;
    assert_ne!(status, StatusCode::OK, "self-subscribe must be rejected");
    assert!(
        !env.subscriptions()
            .is_subscriber(me.user_id, &ViewerIdentity::local(me.user_id))
            .await
            .unwrap(),
        "no self-subscription row may be created"
    );
}

// Subscribe requires authentication.
#[apply(backends)]
#[tokio::test]
async fn subscribe_unauthenticated_is_rejected(#[case] backend: Backend) {
    let env = backend.setup().await;
    let app = make_app!(&env, &env.base);
    let author = SeedUser::new()
        .seed(std::sync::Arc::clone(&env.users()), env.write_scope())
        .await;

    let (status, _body) = post_form(
        app.clone(),
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
    let env = backend.setup().await;
    let app = make_app!(&env, &env.base);
    let author = SeedUser::new()
        .seed(std::sync::Arc::clone(&env.users()), env.write_scope())
        .await;
    let cookie = create_user_and_session(
        std::sync::Arc::clone(&env.users()),
        std::sync::Arc::clone(&env.sessions()),
        env.write_scope(),
    )
    .await
    .cookie();

    let (status, body) = post_form(
        app.clone(),
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
        app.clone(),
        <web::subscriptions::Subscribe as ServerFn>::PATH,
        format!("author_username={}", author.username),
        Some(&cookie),
    )
    .await;

    let (status, body) = post_form(
        app.clone(),
        <web::subscriptions::IsSubscribed as ServerFn>::PATH,
        format!("author_username={}", author.username),
        Some(&cookie),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert!(body.contains("true"), "should be subscribed now: {body}");
}
