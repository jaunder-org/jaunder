use axum::{
    body::Body,
    http::{HeaderValue, Method, StatusCode, header},
};
use rstest::*;
use rstest_reuse::*;
use tower::ServiceExt;

use crate::helpers::{
    SeededSession, atompub, body_string, create_user_and_session, make_app, setup_with_base_url,
};
use storage::test_support::{Backend, TestEnv, backends};

use super::fixtures::{entry_xml, etag_of};

/// POST a create as `session`, optionally with an `Idempotency-Key`.
async fn create_post_keyed(
    app: axum::Router,
    session: &SeededSession,
    xml: &str,
    idempotency_key: Option<&str>,
) -> axum::response::Response {
    create_post_with_idempotency_header(
        app,
        session,
        xml,
        idempotency_key.map(|key| HeaderValue::try_from(key).unwrap()),
    )
    .await
}

/// POST a create with a raw idempotency header value so boundary tests exercise
/// `HeaderValue::to_str`, rather than pre-validating through request strings.
async fn create_post_with_idempotency_header(
    app: axum::Router,
    session: &SeededSession,
    xml: &str,
    idempotency_key: Option<HeaderValue>,
) -> axum::response::Response {
    let mut builder = atompub(session, Method::POST, "posts")
        .header(header::CONTENT_TYPE, "application/atom+xml");
    if let Some(key) = idempotency_key {
        builder = builder.header("Idempotency-Key", key);
    }
    app.oneshot(builder.body(Body::from(xml.to_string())).unwrap())
        .await
        .unwrap()
}

fn location_of(response: &axum::response::Response) -> String {
    response
        .headers()
        .get(header::LOCATION)
        .and_then(|v| v.to_str().ok())
        .expect("response has a Location header")
        .to_string()
}

#[apply(backends)]
#[tokio::test]
async fn create_with_same_idempotency_key_dedups(#[case] backend: Backend) {
    // AC-S1: the same key creates one post; the retry returns it as 200.
    let TestEnv { state, base } = setup_with_base_url(backend).await;
    let session = create_user_and_session(&state).await;
    let app = make_app(&state, &base);
    let xml = entry_xml("Hello", "text", "the body");

    let first = create_post_keyed(app.clone(), &session, &xml, Some("idem-1")).await;
    assert_eq!(first.status(), StatusCode::CREATED);
    let loc1 = location_of(&first);
    let etag1 = etag_of(&first);
    let body1 = body_string(first).await;

    let retry_xml = entry_xml("Changed", "text", "different retry content");
    let second = create_post_keyed(app, &session, &retry_xml, Some("idem-1")).await;
    assert_eq!(second.status(), StatusCode::OK);
    assert_eq!(
        location_of(&second),
        loc1,
        "retry returns the original post"
    );
    assert_eq!(etag_of(&second), etag1, "retry returns the same ETag");
    assert_eq!(
        body_string(second).await,
        body1,
        "retry returns the same body"
    );
}

#[apply(backends)]
#[tokio::test]
async fn create_with_expired_idempotency_key_creates_a_replacement(#[case] backend: Backend) {
    let TestEnv { state, base } = setup_with_base_url(backend).await;
    let session = create_user_and_session(&state).await;
    let app = make_app(&state, &base);
    let first_xml = entry_xml("Original", "text", "original body");

    let first = create_post_keyed(app.clone(), &session, &first_xml, Some("expired-key")).await;
    assert_eq!(first.status(), StatusCode::CREATED);
    let first_location = location_of(&first);
    base.pool()
        .execute(
            "UPDATE idempotency_keys \
             SET created_at = '2000-01-01 00:00:00+00' \
             WHERE key = 'expired-key'",
        )
        .await
        .expect("age the retained mapping as a restored backup may");

    let replacement_xml = entry_xml("Replacement", "text", "replacement body");
    let replacement = create_post_keyed(app, &session, &replacement_xml, Some("expired-key")).await;
    assert_eq!(replacement.status(), StatusCode::CREATED);
    assert_ne!(location_of(&replacement), first_location);
    assert_eq!(
        base.pool()
            .scalar_i64("SELECT COUNT(*) FROM posts")
            .await
            .expect("count durable Posts"),
        2
    );
}

#[apply(backends)]
#[tokio::test]
async fn create_with_fresh_idempotency_key_is_201(#[case] backend: Backend) {
    // AC-S2: distinct keys create distinct posts.
    let TestEnv { state, base } = setup_with_base_url(backend).await;
    let session = create_user_and_session(&state).await;
    let app = make_app(&state, &base);
    let xml = entry_xml("Hello", "text", "the body");

    let first = create_post_keyed(app.clone(), &session, &xml, Some("k-a")).await;
    assert_eq!(first.status(), StatusCode::CREATED);
    let second = create_post_keyed(app, &session, &xml, Some("k-b")).await;
    assert_eq!(second.status(), StatusCode::CREATED);
    assert_ne!(location_of(&first), location_of(&second));
}

#[apply(backends)]
#[tokio::test]
async fn create_without_idempotency_key_is_201(#[case] backend: Backend) {
    // AC-S3: no header → create as today.
    let TestEnv { state, base } = setup_with_base_url(backend).await;
    let session = create_user_and_session(&state).await;
    let app = make_app(&state, &base);
    let xml = entry_xml("Hello", "text", "the body");

    let response = create_post_keyed(app, &session, &xml, None).await;
    assert_eq!(response.status(), StatusCode::CREATED);
}

#[apply(backends)]
#[tokio::test]
async fn unreadable_or_blank_idempotency_keys_do_not_dedup(#[case] backend: Backend) {
    let TestEnv { state, base } = setup_with_base_url(backend).await;
    let session = create_user_and_session(&state).await;
    let app = make_app(&state, &base);
    let xml = entry_xml("Hello", "text", "the body");

    for header_value in [
        HeaderValue::from_static(" \t "),
        HeaderValue::from_bytes("rétry".as_bytes()).unwrap(),
        HeaderValue::from_bytes(&[0xff]).unwrap(),
    ] {
        let first = create_post_with_idempotency_header(
            app.clone(),
            &session,
            &xml,
            Some(header_value.clone()),
        )
        .await;
        let second =
            create_post_with_idempotency_header(app.clone(), &session, &xml, Some(header_value))
                .await;
        assert_eq!(first.status(), StatusCode::CREATED);
        assert_eq!(second.status(), StatusCode::CREATED);
        assert_ne!(location_of(&first), location_of(&second));
    }
}

#[apply(backends)]
#[tokio::test]
async fn idempotency_key_is_scoped_to_the_authenticated_user(#[case] backend: Backend) {
    let TestEnv { state, base } = setup_with_base_url(backend).await;
    let alice = create_user_and_session(&state).await;
    let bob = create_user_and_session(&state).await;
    let app = make_app(&state, &base);
    let xml = entry_xml("Hello", "text", "the body");

    let alice_response = create_post_keyed(app.clone(), &alice, &xml, Some("shared-key")).await;
    let bob_response = create_post_keyed(app, &bob, &xml, Some("shared-key")).await;

    assert_eq!(alice_response.status(), StatusCode::CREATED);
    assert_eq!(bob_response.status(), StatusCode::CREATED);
    assert_ne!(location_of(&alice_response), location_of(&bob_response));
}
