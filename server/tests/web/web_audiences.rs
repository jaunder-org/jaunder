use std::sync::Arc;

use axum::{
    body::Body,
    http::{header, Request, StatusCode},
};
use common::username::Username;
use tower::ServiceExt;

use rstest::*;
#[expect(
    clippy::single_component_path_imports,
    reason = "rstest_reuse needs the bare `use rstest_reuse;` import in scope for its #[template]/#[apply] macros; a glob import would trip wildcard_imports instead"
)]
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

/// Parses the JSON-encoded `i64` that `create_audience` returns.
fn parse_id(body: &str) -> i64 {
    body.trim().parse::<i64>().unwrap()
}

// create → list → rename → delete happy path.
#[apply(backends)]
#[tokio::test]
async fn audience_crud_round_trips(#[case] backend: Backend) {
    let TestEnv { state, base: _base } = backend.setup().await;
    let author = make_user(&state, "author").await;
    let cookie = cookie_for(&state, author).await;

    let (status, body) = post_form(
        Arc::clone(&state),
        "/api/create_audience",
        "name=Friends",
        Some(&cookie),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "create failed: {body}");
    let id = parse_id(&body);

    let (status, body) = post_form(
        Arc::clone(&state),
        "/api/list_my_audiences",
        "",
        Some(&cookie),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert!(
        body.contains("Friends"),
        "audience missing from list: {body}"
    );

    let (status, body) = post_form(
        Arc::clone(&state),
        "/api/rename_audience",
        &format!("audience_id={id}&name=BestFriends"),
        Some(&cookie),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "rename failed: {body}");
    let (_status, body) = post_form(
        Arc::clone(&state),
        "/api/list_my_audiences",
        "",
        Some(&cookie),
    )
    .await;
    assert!(body.contains("BestFriends"), "rename not reflected: {body}");

    let (status, body) = post_form(
        Arc::clone(&state),
        "/api/delete_audience",
        &format!("audience_id={id}"),
        Some(&cookie),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "delete failed: {body}");
    let audiences = state.audiences.list_audiences(author).await.unwrap();
    assert!(audiences.is_empty(), "audience should be gone");
}

// Duplicate name surfaces as a user-facing (non-500-masked) error.
#[apply(backends)]
#[tokio::test]
async fn duplicate_audience_name_is_user_error(#[case] backend: Backend) {
    let TestEnv { state, base: _base } = backend.setup().await;
    let author = make_user(&state, "author").await;
    let cookie = cookie_for(&state, author).await;

    let (status, _) = post_form(
        Arc::clone(&state),
        "/api/create_audience",
        "name=Friends",
        Some(&cookie),
    )
    .await;
    assert_eq!(status, StatusCode::OK);

    let (status, body) = post_form(
        Arc::clone(&state),
        "/api/create_audience",
        "name=Friends",
        Some(&cookie),
    )
    .await;
    assert_ne!(status, StatusCode::OK, "duplicate name must be rejected");
    assert!(
        body.contains("already exists"),
        "duplicate-name error should be user-facing: {body}"
    );
}

// An empty / whitespace-only name is rejected by create_audience with a
// user-facing validation error (not a 500-masked failure).
#[apply(backends)]
#[tokio::test]
async fn create_audience_empty_name_is_validation_error(#[case] backend: Backend) {
    let TestEnv { state, base: _base } = backend.setup().await;
    let author = make_user(&state, "author").await;
    let cookie = cookie_for(&state, author).await;

    let (status, body) = post_form(
        Arc::clone(&state),
        "/api/create_audience",
        "name=%20%20",
        Some(&cookie),
    )
    .await;
    assert_ne!(status, StatusCode::OK, "empty name must be rejected");
    assert!(
        body.contains("audience name must not be empty"),
        "empty-name error should be user-facing: {body}"
    );
    assert!(
        state
            .audiences
            .list_audiences(author)
            .await
            .unwrap()
            .is_empty(),
        "no audience should have been created"
    );
}

// An empty / whitespace-only name is rejected by rename_audience with a
// user-facing validation error.
#[apply(backends)]
#[tokio::test]
async fn rename_audience_empty_name_is_validation_error(#[case] backend: Backend) {
    let TestEnv { state, base: _base } = backend.setup().await;
    let author = make_user(&state, "author").await;
    let cookie = cookie_for(&state, author).await;

    let (_status, body) = post_form(
        Arc::clone(&state),
        "/api/create_audience",
        "name=Friends",
        Some(&cookie),
    )
    .await;
    let aud_id = parse_id(&body);

    let (status, body) = post_form(
        Arc::clone(&state),
        "/api/rename_audience",
        &format!("audience_id={aud_id}&name=%20%20"),
        Some(&cookie),
    )
    .await;
    assert_ne!(status, StatusCode::OK, "empty rename must be rejected");
    assert!(
        body.contains("audience name must not be empty"),
        "empty-name error should be user-facing: {body}"
    );
    // Original name is unchanged.
    let audiences = state.audiences.list_audiences(author).await.unwrap();
    assert_eq!(audiences.len(), 1);
    assert_eq!(audiences[0].name, "Friends", "name should be unchanged");
}

// list_audience_members returns the audience's subscription members.
#[apply(backends)]
#[tokio::test]
async fn list_audience_members_returns_members(#[case] backend: Backend) {
    let TestEnv { state, base: _base } = backend.setup().await;
    let author = make_user(&state, "author").await;
    let subscriber = make_user(&state, "subscriber").await;
    let cookie = cookie_for(&state, author).await;
    let channel = state.subscriptions.local_channel_id().await.unwrap();
    let sub_id = state
        .subscriptions
        .subscribe(author, channel, &subscriber.to_string())
        .await
        .unwrap();

    let aud_id = state
        .audiences
        .create_audience(author, "Friends")
        .await
        .unwrap();
    state
        .audiences
        .add_member(author, aud_id, sub_id)
        .await
        .unwrap();

    let (status, body) = post_form(
        Arc::clone(&state),
        "/api/list_audience_members",
        &format!("audience_id={aud_id}"),
        Some(&cookie),
    )
    .await;
    assert_eq!(
        status,
        StatusCode::OK,
        "list_audience_members failed: {body}"
    );
    assert!(
        body.contains(&sub_id.to_string()),
        "member subscription_id should appear in list: {body}"
    );
}

// add member → list members → remove member happy path.
#[apply(backends)]
#[tokio::test]
async fn audience_membership_round_trips(#[case] backend: Backend) {
    let TestEnv { state, base: _base } = backend.setup().await;
    let author = make_user(&state, "author").await;
    let subscriber = make_user(&state, "subscriber").await;
    let cookie = cookie_for(&state, author).await;
    let channel = state.subscriptions.local_channel_id().await.unwrap();
    let sub_id = state
        .subscriptions
        .subscribe(author, channel, &subscriber.to_string())
        .await
        .unwrap();

    let (_s, body) = post_form(
        Arc::clone(&state),
        "/api/create_audience",
        "name=Friends",
        Some(&cookie),
    )
    .await;
    let aud_id = parse_id(&body);

    let (status, body) = post_form(
        Arc::clone(&state),
        "/api/add_subscriber_to_audience",
        &format!("audience_id={aud_id}&subscription_id={sub_id}"),
        Some(&cookie),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "add_member failed: {body}");
    assert_eq!(
        state.audiences.list_members(aud_id).await.unwrap(),
        vec![sub_id]
    );

    let (status, body) = post_form(
        Arc::clone(&state),
        "/api/remove_subscriber_from_audience",
        &format!("audience_id={aud_id}&subscription_id={sub_id}"),
        Some(&cookie),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "remove_member failed: {body}");
    assert!(state
        .audiences
        .list_members(aud_id)
        .await
        .unwrap()
        .is_empty());
}

// AUTHORIZATION: a client-supplied audience_id owned by another author must be
// rejected by remove_member / list_members (not author-scoped in the store).
#[apply(backends)]
#[tokio::test]
async fn cross_author_audience_id_is_rejected(#[case] backend: Backend) {
    let TestEnv { state, base: _base } = backend.setup().await;
    let alice = make_user(&state, "alice").await;
    let bob = make_user(&state, "bob").await;
    // Alice owns the audience.
    let alice_aud = state
        .audiences
        .create_audience(alice, "Secret")
        .await
        .unwrap();
    let bob_cookie = cookie_for(&state, bob).await;

    // Bob tries to list Alice's audience members → must be rejected.
    let (status, _body) = post_form(
        Arc::clone(&state),
        "/api/list_audience_members",
        &format!("audience_id={alice_aud}"),
        Some(&bob_cookie),
    )
    .await;
    assert_ne!(
        status,
        StatusCode::OK,
        "listing another author's audience members must be rejected"
    );

    // Bob tries to remove a member from Alice's audience → must be rejected.
    let (status, _body) = post_form(
        Arc::clone(&state),
        "/api/remove_subscriber_from_audience",
        &format!("audience_id={alice_aud}&subscription_id=1"),
        Some(&bob_cookie),
    )
    .await;
    assert_ne!(
        status,
        StatusCode::OK,
        "removing a member from another author's audience must be rejected"
    );
}

// list_my_subscribers returns the author's active subscribers by username.
#[apply(backends)]
#[tokio::test]
async fn list_my_subscribers_resolves_usernames(#[case] backend: Backend) {
    let TestEnv { state, base: _base } = backend.setup().await;
    let author = make_user(&state, "author").await;
    let subscriber = make_user(&state, "subscriber").await;
    let cookie = cookie_for(&state, author).await;
    let channel = state.subscriptions.local_channel_id().await.unwrap();
    state
        .subscriptions
        .subscribe(author, channel, &subscriber.to_string())
        .await
        .unwrap();

    let (status, body) = post_form(
        Arc::clone(&state),
        "/api/list_my_subscribers",
        "",
        Some(&cookie),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert!(
        body.contains("subscriber"),
        "subscriber username should appear: {body}"
    );
}

// Audience management requires authentication.
#[apply(backends)]
#[tokio::test]
async fn audience_create_unauthenticated_is_rejected(#[case] backend: Backend) {
    let TestEnv { state, base: _base } = backend.setup().await;

    let (status, _body) = post_form(
        Arc::clone(&state),
        "/api/create_audience",
        "name=Friends",
        None,
    )
    .await;
    assert_eq!(status, StatusCode::INTERNAL_SERVER_ERROR);
}
