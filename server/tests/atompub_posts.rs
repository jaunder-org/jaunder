mod helpers;

use std::sync::Arc;

use axum::{
    body::Body,
    http::{header, Request, StatusCode},
};
use base64::Engine as _;
use tempfile::TempDir;
use tower::ServiceExt;

use helpers::{ensure_server_fns_registered, noop_mailer, test_options, test_state};

async fn make_app(state: Arc<storage::AppState>, storage: &TempDir) -> axum::Router {
    ensure_server_fns_registered();
    let storage_path = storage.path().to_path_buf();
    jaunder::create_router(test_options(), state, noop_mailer(), false, storage_path)
}

fn basic_header(username: &str, password: &str) -> String {
    let raw = format!("{username}:{password}");
    let encoded = base64::engine::general_purpose::STANDARD.encode(raw);
    format!("Basic {encoded}")
}

async fn body_string(response: axum::response::Response) -> String {
    let bytes = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .unwrap();
    String::from_utf8(bytes.to_vec()).unwrap()
}

#[tokio::test]
async fn collection_lists_user_posts() {
    let base = TempDir::new().unwrap();
    let state = test_state(&base).await;
    let user_id = state
        .users
        .create_user(
            &"alice".parse().unwrap(),
            &"password123".parse().unwrap(),
            None,
            false,
        )
        .await
        .unwrap();
    let token = state
        .sessions
        .create_session(user_id, "MarsEdit")
        .await
        .unwrap();

    // Create two published posts
    let _post1 = storage::perform_post_creation(
        state.posts.as_ref(),
        user_id,
        "Hello body one".to_string(),
        Some("Hello Title One"),
        storage::PostFormat::Markdown,
        None,
        Some(chrono::Utc::now()),
        100,
        None,
    )
    .await
    .unwrap();

    let _post2 = storage::perform_post_creation(
        state.posts.as_ref(),
        user_id,
        "Hello body two".to_string(),
        Some("Hello Title Two"),
        storage::PostFormat::Markdown,
        None,
        Some(chrono::Utc::now()),
        100,
        None,
    )
    .await
    .unwrap();

    let app = make_app(state, &base).await;

    let response = app
        .oneshot(
            Request::builder()
                .uri("/atompub/alice/posts")
                .header(header::AUTHORIZATION, basic_header("alice", &token))
                .body(Body::empty())
                .unwrap(),
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
        ctype.contains("type=feed"),
        "content-type was {ctype}, should contain type=feed"
    );
    let body = body_string(response).await;
    assert!(body.contains("<feed"), "body should contain <feed");
    assert!(
        body.contains("Hello Title One"),
        "body should contain first post title"
    );
    assert!(
        body.contains("Hello Title Two"),
        "body should contain second post title"
    );
    assert!(
        body.contains("rel=\"edit\""),
        "body should contain rel=edit link"
    );
}

#[tokio::test]
async fn member_returns_native_source_with_etag() {
    let base = TempDir::new().unwrap();
    let state = test_state(&base).await;
    let user_id = state
        .users
        .create_user(
            &"alice".parse().unwrap(),
            &"password123".parse().unwrap(),
            None,
            false,
        )
        .await
        .unwrap();
    let token = state
        .sessions
        .create_session(user_id, "MarsEdit")
        .await
        .unwrap();

    let post = storage::perform_post_creation(
        state.posts.as_ref(),
        user_id,
        "# Markdown body".to_string(),
        Some("My Post"),
        storage::PostFormat::Markdown,
        None,
        Some(chrono::Utc::now()),
        100,
        None,
    )
    .await
    .unwrap();

    let app = make_app(state, &base).await;

    let response = app
        .oneshot(
            Request::builder()
                .uri(&format!("/atompub/alice/posts/{}", post.post_id))
                .header(header::AUTHORIZATION, basic_header("alice", &token))
                .body(Body::empty())
                .unwrap(),
        )
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
        body.contains("type=\"text\""),
        "body should contain type=text (native source)"
    );
    assert!(
        body.contains("# Markdown body"),
        "body should contain markdown"
    );
}

#[tokio::test]
async fn member_get_unknown_returns_404() {
    let base = TempDir::new().unwrap();
    let state = test_state(&base).await;
    let user_id = state
        .users
        .create_user(
            &"alice".parse().unwrap(),
            &"password123".parse().unwrap(),
            None,
            false,
        )
        .await
        .unwrap();
    let token = state
        .sessions
        .create_session(user_id, "MarsEdit")
        .await
        .unwrap();

    let app = make_app(state, &base).await;

    let response = app
        .oneshot(
            Request::builder()
                .uri("/atompub/alice/posts/999999")
                .header(header::AUTHORIZATION, basic_header("alice", &token))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn collection_forbids_other_user() {
    let base = TempDir::new().unwrap();
    let state = test_state(&base).await;
    let alice_id = state
        .users
        .create_user(
            &"alice".parse().unwrap(),
            &"password123".parse().unwrap(),
            None,
            false,
        )
        .await
        .unwrap();
    let token = state
        .sessions
        .create_session(alice_id, "MarsEdit")
        .await
        .unwrap();

    let app = make_app(state, &base).await;

    let response = app
        .oneshot(
            Request::builder()
                .uri("/atompub/bob/posts")
                .header(header::AUTHORIZATION, basic_header("alice", &token))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::FORBIDDEN);
}

#[tokio::test]
async fn delete_then_get_is_404() {
    let base = TempDir::new().unwrap();
    let state = test_state(&base).await;
    let user_id = state
        .users
        .create_user(
            &"alice".parse().unwrap(),
            &"password123".parse().unwrap(),
            None,
            false,
        )
        .await
        .unwrap();
    let token = state
        .sessions
        .create_session(user_id, "MarsEdit")
        .await
        .unwrap();

    let post = storage::perform_post_creation(
        state.posts.as_ref(),
        user_id,
        "Delete me".to_string(),
        Some("Temporary Post"),
        storage::PostFormat::Markdown,
        None,
        Some(chrono::Utc::now()),
        100,
        None,
    )
    .await
    .unwrap();

    let app = make_app(state, &base).await;

    // First, delete the post
    let delete_response = app
        .clone()
        .oneshot(
            Request::builder()
                .method("DELETE")
                .uri(&format!("/atompub/alice/posts/{}", post.post_id))
                .header(header::AUTHORIZATION, basic_header("alice", &token))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(delete_response.status(), StatusCode::NO_CONTENT);

    // Then, try to get it
    let get_response = app
        .oneshot(
            Request::builder()
                .uri(&format!("/atompub/alice/posts/{}", post.post_id))
                .header(header::AUTHORIZATION, basic_header("alice", &token))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(get_response.status(), StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn collection_paging_emits_next_link() {
    let base = TempDir::new().unwrap();
    let state = test_state(&base).await;
    let user_id = state
        .users
        .create_user(
            &"alice".parse().unwrap(),
            &"password123".parse().unwrap(),
            None,
            false,
        )
        .await
        .unwrap();
    let token = state
        .sessions
        .create_session(user_id, "MarsEdit")
        .await
        .unwrap();

    for i in 0..2 {
        storage::perform_post_creation(
            state.posts.as_ref(),
            user_id,
            format!("Body {i}"),
            Some(&format!("Title {i}")),
            storage::PostFormat::Markdown,
            None,
            Some(chrono::Utc::now()),
            100,
            None,
        )
        .await
        .unwrap();
    }

    let app = make_app(state, &base).await;

    // Page size 1 with 2 posts -> a next link must be present.
    let response = app
        .oneshot(
            Request::builder()
                .uri("/atompub/alice/posts?limit=1")
                .header(header::AUTHORIZATION, basic_header("alice", &token))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);
    let body = body_string(response).await;
    assert!(body.contains("rel=\"next\""), "missing next link: {body}");
    assert!(
        body.contains("updated_before="),
        "next link lacks cursor: {body}"
    );
    // Only one entry on this page.
    assert_eq!(
        body.matches("<entry").count(),
        1,
        "expected exactly one entry"
    );
}

/// Seeds a user named `alice` and returns `(user_id, session_token)`.
async fn seed_alice(state: &Arc<storage::AppState>) -> (i64, String) {
    let user_id = state
        .users
        .create_user(
            &"alice".parse().unwrap(),
            &"password123".parse().unwrap(),
            None,
            false,
        )
        .await
        .unwrap();
    let token = state
        .sessions
        .create_session(user_id, "MarsEdit")
        .await
        .unwrap();
    (user_id, token)
}

#[tokio::test]
async fn collection_accepts_valid_cursor() {
    let base = TempDir::new().unwrap();
    let state = test_state(&base).await;
    let (user_id, token) = seed_alice(&state).await;
    storage::perform_post_creation(
        state.posts.as_ref(),
        user_id,
        "Body".to_string(),
        Some("Title"),
        storage::PostFormat::Markdown,
        None,
        Some(chrono::Utc::now()),
        100,
        None,
    )
    .await
    .unwrap();
    let app = make_app(state, &base).await;

    let response = app
        .oneshot(
            Request::builder()
                .uri("/atompub/alice/posts?updated_before=2099-01-01T00:00:00Z&id_before=999999")
                .header(header::AUTHORIZATION, basic_header("alice", &token))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);
}

#[tokio::test]
async fn collection_rejects_invalid_cursor() {
    let base = TempDir::new().unwrap();
    let state = test_state(&base).await;
    let (_user_id, token) = seed_alice(&state).await;
    let app = make_app(state, &base).await;

    let response = app
        .oneshot(
            Request::builder()
                .uri("/atompub/alice/posts?updated_before=not-a-date&id_before=1")
                .header(header::AUTHORIZATION, basic_header("alice", &token))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn collection_empty_returns_feed_without_entries() {
    let base = TempDir::new().unwrap();
    let state = test_state(&base).await;
    let (_user_id, token) = seed_alice(&state).await;
    let app = make_app(state, &base).await;

    let response = app
        .oneshot(
            Request::builder()
                .uri("/atompub/alice/posts")
                .header(header::AUTHORIZATION, basic_header("alice", &token))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);
    let body = body_string(response).await;
    assert!(body.contains("<feed"));
    assert_eq!(body.matches("<entry").count(), 0);
}

#[tokio::test]
async fn member_forbids_other_user() {
    let base = TempDir::new().unwrap();
    let state = test_state(&base).await;
    let (_user_id, token) = seed_alice(&state).await;
    let app = make_app(state, &base).await;

    // alice is authenticated but requests bob's member URL.
    let response = app
        .oneshot(
            Request::builder()
                .uri("/atompub/bob/posts/1")
                .header(header::AUTHORIZATION, basic_header("alice", &token))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::FORBIDDEN);
}
