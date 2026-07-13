use std::sync::Arc;

use axum::http::StatusCode;
use common::username::Username;

use rstest::*;
use rstest_reuse::*;

use crate::helpers::{backends, post_form, Backend, TestEnv};

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
        state.audiences.list_members(author, aud_id).await.unwrap(),
        vec![sub_id]
    );

    // Adding the same subscriber again is idempotent through the boundary.
    let (status, body) = post_form(
        Arc::clone(&state),
        "/api/add_subscriber_to_audience",
        &format!("audience_id={aud_id}&subscription_id={sub_id}"),
        Some(&cookie),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "idempotent add failed: {body}");
    assert_eq!(
        state.audiences.list_members(author, aud_id).await.unwrap(),
        vec![sub_id],
        "a duplicate add must not duplicate the membership"
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
        .list_members(author, aud_id)
        .await
        .unwrap()
        .is_empty());

    // Removing a subscriber who is no longer a member is a no-op, not an error.
    let (status, body) = post_form(
        Arc::clone(&state),
        "/api/remove_subscriber_from_audience",
        &format!("audience_id={aud_id}&subscription_id={sub_id}"),
        Some(&cookie),
    )
    .await;
    assert_eq!(
        status,
        StatusCode::OK,
        "redundant remove should be a no-op: {body}"
    );
    assert!(state
        .audiences
        .list_members(author, aud_id)
        .await
        .unwrap()
        .is_empty());
}

// AUTHORIZATION: every store method is author-scoped, so a client-supplied
// audience_id owned by another author matches nothing — the request succeeds but
// sees/changes none of the other author's data. (This replaced web's
// `assert_owns_audience` NotFound gate; the storage-layer guarantee is covered by
// `audience_members_are_author_scoped`.)
#[apply(backends)]
#[tokio::test]
async fn cross_author_audience_id_is_scoped_away(#[case] backend: Backend) {
    let TestEnv { state, base: _base } = backend.setup().await;
    let alice = make_user(&state, "alice").await;
    let bob = make_user(&state, "bob").await;
    let subscriber = make_user(&state, "subscriber").await;
    let channel = state.subscriptions.local_channel_id().await.unwrap();
    // Alice owns an audience with a member.
    let alice_sub = state
        .subscriptions
        .subscribe(alice, channel, &subscriber.to_string())
        .await
        .unwrap();
    let alice_aud = state
        .audiences
        .create_audience(alice, "Secret")
        .await
        .unwrap();
    state
        .audiences
        .add_member(alice, alice_aud, alice_sub)
        .await
        .unwrap();
    let bob_cookie = cookie_for(&state, bob).await;

    // Bob lists Alice's audience members → succeeds, but sees nothing of hers.
    let (status, body) = post_form(
        Arc::clone(&state),
        "/api/list_audience_members",
        &format!("audience_id={alice_aud}"),
        Some(&bob_cookie),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(
        body.trim(),
        "[]",
        "another author's audience must list as empty, leaking no member id: {body}"
    );

    // Bob removes from Alice's audience → succeeds, but changes nothing.
    let (status, body) = post_form(
        Arc::clone(&state),
        "/api/remove_subscriber_from_audience",
        &format!("audience_id={alice_aud}&subscription_id={alice_sub}"),
        Some(&bob_cookie),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    // Alice's membership is intact.
    assert_eq!(
        state
            .audiences
            .list_members(alice, alice_aud)
            .await
            .unwrap(),
        vec![alice_sub]
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

// Every audience endpoint independently calls `require_auth`, so each must reject
// an unauthenticated request (a dropped guard on any one would otherwise slip
// through). One table covers all of them.
#[apply(backends)]
#[tokio::test]
async fn audience_endpoints_require_authentication(#[case] backend: Backend) {
    let TestEnv { state, base: _base } = backend.setup().await;

    let endpoints = [
        ("/api/create_audience", "name=Friends"),
        ("/api/rename_audience", "audience_id=1&name=X"),
        ("/api/delete_audience", "audience_id=1"),
        ("/api/list_my_audiences", ""),
        ("/api/list_my_subscribers", ""),
        (
            "/api/add_subscriber_to_audience",
            "audience_id=1&subscription_id=1",
        ),
        (
            "/api/remove_subscriber_from_audience",
            "audience_id=1&subscription_id=1",
        ),
        ("/api/list_audience_members", "audience_id=1"),
    ];
    for (uri, body) in endpoints {
        let (status, _body) = post_form(Arc::clone(&state), uri, body, None).await;
        assert_eq!(
            status,
            StatusCode::INTERNAL_SERVER_ERROR,
            "{uri} must require authentication"
        );
    }
}

// A cross-author ADD is asymmetric with the scoped-away reads/removes: `add_member`
// writes `author_user_id = bob`, so the composite FK `(audience_id, author_user_id)`
// rejects a pairing with Alice's audience and it surfaces as a Storage error — NOT
// a silent no-op. This is the write path the deleted `assert_owns_audience` used to
// guard, so it must be refused at the boundary.
#[apply(backends)]
#[tokio::test]
async fn cross_author_add_member_is_rejected(#[case] backend: Backend) {
    let TestEnv { state, base: _base } = backend.setup().await;
    let alice = make_user(&state, "alice").await;
    let bob = make_user(&state, "bob").await;
    let subscriber = make_user(&state, "subscriber").await;
    let channel = state.subscriptions.local_channel_id().await.unwrap();
    // Alice owns a subscription and an audience (no members yet).
    let alice_sub = state
        .subscriptions
        .subscribe(alice, channel, &subscriber.to_string())
        .await
        .unwrap();
    let alice_aud = state
        .audiences
        .create_audience(alice, "Secret")
        .await
        .unwrap();
    let bob_cookie = cookie_for(&state, bob).await;

    // Bob tries to inject Alice's subscription into Alice's audience.
    let (status, body) = post_form(
        Arc::clone(&state),
        "/api/add_subscriber_to_audience",
        &format!("audience_id={alice_aud}&subscription_id={alice_sub}"),
        Some(&bob_cookie),
    )
    .await;
    assert_ne!(
        status,
        StatusCode::OK,
        "cross-author add must be rejected: {body}"
    );
    // Alice's audience is still empty — nothing was added on her behalf.
    assert!(
        state
            .audiences
            .list_members(alice, alice_aud)
            .await
            .unwrap()
            .is_empty(),
        "no member should have been added to another author's audience"
    );
}

// Cross-author rename and delete at the boundary: rename surfaces the store's
// NotFound (a non-OK error, name untouched); delete is a silent author-scoped
// no-op (OK, audience intact). Complements `cross_author_audience_id_is_scoped_away`
// (reads/removes) and `cross_author_add_member_is_rejected` (add) so every mutation
// path is pinned now that `assert_owns_audience` is gone.
#[apply(backends)]
#[tokio::test]
async fn cross_author_rename_and_delete_are_scoped(#[case] backend: Backend) {
    let TestEnv { state, base: _base } = backend.setup().await;
    let alice = make_user(&state, "alice").await;
    let bob = make_user(&state, "bob").await;
    let alice_aud = state
        .audiences
        .create_audience(alice, "Secret")
        .await
        .unwrap();
    let bob_cookie = cookie_for(&state, bob).await;

    // Bob renames Alice's audience → refused (store NotFound); name unchanged.
    let (status, body) = post_form(
        Arc::clone(&state),
        "/api/rename_audience",
        &format!("audience_id={alice_aud}&name=Hijacked"),
        Some(&bob_cookie),
    )
    .await;
    assert_ne!(
        status,
        StatusCode::OK,
        "cross-author rename must be refused: {body}"
    );

    // Bob deletes Alice's audience → author-scoped no-op (OK), still present.
    let (status, body) = post_form(
        Arc::clone(&state),
        "/api/delete_audience",
        &format!("audience_id={alice_aud}"),
        Some(&bob_cookie),
    )
    .await;
    assert_eq!(
        status,
        StatusCode::OK,
        "cross-author delete is a scoped no-op: {body}"
    );

    // Alice's audience is intact under its original name.
    let audiences = state.audiences.list_audiences(alice).await.unwrap();
    assert_eq!(audiences.len(), 1);
    assert_eq!(audiences[0].name, "Secret", "name must be unchanged");
}
