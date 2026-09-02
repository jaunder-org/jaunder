use axum::{
    body::Body,
    http::{Method, StatusCode, header},
};
use common::test_support::{parse_post_body, parse_post_title};
use rstest::*;
use rstest_reuse::*;
use tower::ServiceExt;

use crate::helpers::{atompub, atompub_get, body_string, create_user_and_session, make_app};
use storage::test_support::{Backend, TestEnv, backends};

#[apply(backends)]
#[tokio::test]
async fn member_returns_native_source_with_etag(#[case] backend: Backend) {
    let TestEnv { state, base } = backend.setup().await;
    let session = create_user_and_session(&state).await;

    let post = session
        .seed_post()
        .body(parse_post_body("# Markdown body"))
        .seed(&state)
        .await;

    let app = make_app(&state, &base);

    let response = app
        .oneshot(atompub_get(&session, &format!("posts/{}", post.post_id)))
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);
    let etag = response
        .headers()
        .get(header::ETAG)
        .and_then(|v| v.to_str().ok());
    assert!(etag.is_some(), "response should have ETag header");
    let body = body_string(response).await;
    assert!(
        body.contains("type=\"text/markdown\""),
        "body should carry the text/markdown media type (native source, ADR-0023)"
    );
    assert!(
        body.contains("# Markdown body"),
        "body should contain markdown"
    );
}

// Member responses are the wire contract consumed by pull clients, not merely mapper data.
#[apply(backends)]
#[tokio::test]
async fn member_get_serializes_empty_and_genuine_titles(#[case] backend: Backend) {
    let TestEnv { state, base } = backend.setup().await;
    let session = create_user_and_session(&state).await;

    let untitled = session
        .seed_post()
        .body(parse_post_body("Untitled source"))
        .seed(&state)
        .await;
    let titled = session
        .seed_post()
        .title(parse_post_title("Genuine title"))
        .body(parse_post_body("Titled source"))
        .seed(&state)
        .await;

    let app = make_app(&state, &base);
    let untitled_response = app
        .clone()
        .oneshot(atompub_get(
            &session,
            &format!("posts/{}", untitled.post_id),
        ))
        .await
        .unwrap();
    assert_eq!(untitled_response.status(), StatusCode::OK);
    let untitled_body = body_string(untitled_response).await;
    assert!(
        untitled_body.contains("<title/>") || untitled_body.contains("<title></title>"),
        "untitled Member must serialize a required empty title element: {untitled_body}"
    );
    assert!(untitled_body.contains("Untitled source"));
    let untitled_entry = untitled_body
        .parse::<host::atompub::Entry>()
        .expect("untitled Member response is an Atom Entry");
    assert_eq!(untitled_entry.title().as_str(), "");

    let titled_response = app
        .oneshot(atompub_get(&session, &format!("posts/{}", titled.post_id)))
        .await
        .unwrap();
    assert_eq!(titled_response.status(), StatusCode::OK);
    let titled_body = body_string(titled_response).await;
    assert!(
        titled_body.contains("<title>Genuine title</title>"),
        "titled Member must serialize its exact title: {titled_body}"
    );
    assert!(titled_body.contains("Titled source"));
    let titled_entry = titled_body
        .parse::<host::atompub::Entry>()
        .expect("titled Member response is an Atom Entry");
    assert_eq!(titled_entry.title().as_str(), "Genuine title");
}

#[apply(backends)]
#[tokio::test]
async fn member_get_unknown_returns_404(#[case] backend: Backend) {
    let TestEnv { state, base } = backend.setup().await;
    let session = create_user_and_session(&state).await;

    let app = make_app(&state, &base);

    let response = app
        .oneshot(atompub_get(&session, "posts/999999"))
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::NOT_FOUND);
}

#[apply(backends)]
#[tokio::test]
async fn delete_then_get_is_404(#[case] backend: Backend) {
    let TestEnv { state, base } = backend.setup().await;
    let session = create_user_and_session(&state).await;

    let post = session.seed_post().seed(&state).await;

    let app = make_app(&state, &base);

    // First, delete the post
    let delete_response = app
        .clone()
        .oneshot(
            atompub(&session, Method::DELETE, &format!("posts/{}", post.post_id))
                .body(Body::empty())
                .expect("failed to build atompub DELETE request"),
        )
        .await
        .unwrap();

    assert_eq!(delete_response.status(), StatusCode::NO_CONTENT);

    // Then, try to get it
    let get_response = app
        .oneshot(atompub_get(&session, &format!("posts/{}", post.post_id)))
        .await
        .unwrap();

    assert_eq!(get_response.status(), StatusCode::NOT_FOUND);
}

#[apply(backends)]
#[tokio::test]
async fn member_carries_read_only_j_slug(#[case] backend: Backend) {
    let TestEnv { state, base } = backend.setup().await;
    let session = create_user_and_session(&state).await;

    let post = session
        .seed_post()
        .title(parse_post_title("My Post"))
        .seed(&state)
        .await;

    let app = make_app(&state, &base);

    let response = app
        .oneshot(atompub_get(&session, &format!("posts/{}", post.post_id)))
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);
    let body = body_string(response).await;
    let entry = body
        .parse::<host::atompub::Entry>()
        .expect("Member response is an Atom Entry");
    assert_eq!(
        host::atompub::j_slug(&entry).as_deref(),
        Some(post.slug.as_ref()),
        "Member should carry the Post's slug as j:slug"
    );
}
