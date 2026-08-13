use axum::http::StatusCode;
use common::ids::AudienceId;
use common::test_support::parse_audience_name;
use server_fn::ServerFn;

use rstest::*;
use rstest_reuse::*;

use crate::helpers::{create_user_and_session, post_form, post_server_fn};
use storage::test_support::{Backend, SeedUser, TestEnv, backends};

/// Parses the JSON-encoded `i64` that `create_audience` returns.
fn parse_id(body: &str) -> i64 {
    body.trim().parse::<i64>().unwrap()
}

// create → list → rename → delete happy path.
#[apply(backends)]
#[tokio::test]
async fn rename_nested_request_maps_id_and_name(#[case] backend: Backend) {
    let TestEnv { state, base: _base } = backend.setup().await;
    let author = create_user_and_session(&state).await;
    let cookie = author.cookie();

    let (status, body) = post_form(
        &state,
        <web::audiences::Create as ServerFn>::PATH,
        "name=Friends",
        Some(&cookie),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "create failed: {body}");
    let id = parse_id(&body);

    let (status, body) = post_form(
        &state,
        <web::audiences::ListMine as ServerFn>::PATH,
        "",
        Some(&cookie),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert!(
        body.contains("Friends"),
        "audience missing from list: {body}"
    );

    let (status, body) = post_server_fn(
        &state,
        &web::audiences::Rename {
            request: web::audiences::RenameAudienceRequest {
                audience_id: AudienceId::from(id),
                name: parse_audience_name("BestFriends"),
            },
        },
        Some(&cookie),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "rename failed: {body}");
    let (_status, body) = post_form(
        &state,
        <web::audiences::ListMine as ServerFn>::PATH,
        "",
        Some(&cookie),
    )
    .await;
    assert!(body.contains("BestFriends"), "rename not reflected: {body}");

    let (status, body) = post_form(
        &state,
        <web::audiences::Delete as ServerFn>::PATH,
        &format!("audience_id={id}"),
        Some(&cookie),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "delete failed: {body}");
    let audiences = state
        .audiences
        .list_audiences(author.user_id)
        .await
        .unwrap();
    assert!(audiences.is_empty(), "audience should be gone");
}

// Duplicate name surfaces as a user-facing (non-500-masked) error.
#[apply(backends)]
#[tokio::test]
async fn duplicate_audience_name_is_user_error(#[case] backend: Backend) {
    let TestEnv { state, base: _base } = backend.setup().await;
    let cookie = create_user_and_session(&state).await.cookie();

    let (status, _) = post_form(
        &state,
        <web::audiences::Create as ServerFn>::PATH,
        "name=Friends",
        Some(&cookie),
    )
    .await;
    assert_eq!(status, StatusCode::OK);

    let (status, body) = post_form(
        &state,
        <web::audiences::Create as ServerFn>::PATH,
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

// An empty / whitespace-only name is rejected at arg-decode (the typed
// `AudienceName` wire arg), so no audience is created.
#[apply(backends)]
#[tokio::test]
async fn create_audience_empty_name_is_rejected(#[case] backend: Backend) {
    let TestEnv { state, base: _base } = backend.setup().await;
    let author = create_user_and_session(&state).await;
    let cookie = author.cookie();

    let (status, _body) = post_form(
        &state,
        <web::audiences::Create as ServerFn>::PATH,
        "name=%20%20",
        Some(&cookie),
    )
    .await;
    assert_ne!(status, StatusCode::OK, "empty name must be rejected");
    assert!(
        state
            .audiences
            .list_audiences(author.user_id)
            .await
            .unwrap()
            .is_empty(),
        "no audience should have been created"
    );
}

// An empty / whitespace-only name is rejected at arg-decode (the typed
// `AudienceName` wire arg), so the name is unchanged.
#[apply(backends)]
#[tokio::test]
async fn rename_audience_empty_name_is_rejected(#[case] backend: Backend) {
    let TestEnv { state, base: _base } = backend.setup().await;
    let author = create_user_and_session(&state).await;
    let cookie = author.cookie();

    let (_status, body) = post_form(
        &state,
        <web::audiences::Create as ServerFn>::PATH,
        "name=Friends",
        Some(&cookie),
    )
    .await;
    let aud_id = parse_id(&body);

    let (status, _body) = post_form(
        &state,
        <web::audiences::Rename as ServerFn>::PATH,
        &format!("request%5Baudience_id%5D={aud_id}&request%5Bname%5D=%20%20"),
        Some(&cookie),
    )
    .await;
    assert_ne!(status, StatusCode::OK, "empty rename must be rejected");
    // Original name is unchanged.
    let audiences = state
        .audiences
        .list_audiences(author.user_id)
        .await
        .unwrap();
    assert_eq!(audiences.len(), 1);
    assert_eq!(audiences[0].name, "Friends", "name should be unchanged");
}

// list_audience_members returns the audience's subscription members.
#[apply(backends)]
#[tokio::test]
async fn list_audience_members_returns_members(#[case] backend: Backend) {
    let TestEnv { state, base: _base } = backend.setup().await;
    let author = create_user_and_session(&state).await;
    let subscriber = SeedUser::new().seed(&state).await.user_id;
    let cookie = author.cookie();
    let channel = state.subscriptions.local_channel_id().await.unwrap();
    let sub_id = state
        .subscriptions
        .subscribe(author.user_id, channel, &i64::from(subscriber).to_string())
        .await
        .unwrap();

    let aud_id = state
        .audiences
        .create_audience(author.user_id, &parse_audience_name("Friends"))
        .await
        .unwrap();
    state
        .audiences
        .add_member(author.user_id, aud_id, sub_id)
        .await
        .unwrap();

    let (status, body) = post_form(
        &state,
        <web::audiences::ListMembers as ServerFn>::PATH,
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
async fn add_subscriber_nested_request_maps_both_ids(#[case] backend: Backend) {
    let TestEnv { state, base: _base } = backend.setup().await;
    let author = create_user_and_session(&state).await;
    let subscriber = SeedUser::new().seed(&state).await.user_id;
    let cookie = author.cookie();
    let channel = state.subscriptions.local_channel_id().await.unwrap();
    let sub_id = state
        .subscriptions
        .subscribe(author.user_id, channel, &i64::from(subscriber).to_string())
        .await
        .unwrap();
    state
        .audiences
        .create_audience(author.user_id, &parse_audience_name("Decoy"))
        .await
        .unwrap();

    let (_s, body) = post_form(
        &state,
        <web::audiences::Create as ServerFn>::PATH,
        "name=Friends",
        Some(&cookie),
    )
    .await;
    let aud_id = AudienceId::from(parse_id(&body));
    assert_ne!(
        i64::from(aud_id),
        i64::from(sub_id),
        "sentinel ids must differ so a transposition cannot pass"
    );

    let request = web::audiences::AudienceMembershipRequest {
        audience_id: aud_id,
        subscription_id: sub_id,
    };
    let (status, body) = post_server_fn(
        &state,
        &web::audiences::AddSubscriber {
            request: request.clone(),
        },
        Some(&cookie),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "add_member failed: {body}");
    assert_eq!(
        state
            .audiences
            .list_members(author.user_id, aud_id)
            .await
            .unwrap(),
        vec![sub_id]
    );

    // Adding the same subscriber again is idempotent through the boundary.
    let (status, body) = post_server_fn(
        &state,
        &web::audiences::AddSubscriber {
            request: request.clone(),
        },
        Some(&cookie),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "idempotent add failed: {body}");
    assert_eq!(
        state
            .audiences
            .list_members(author.user_id, aud_id)
            .await
            .unwrap(),
        vec![sub_id],
        "a duplicate add must not duplicate the membership"
    );

    let (status, body) = post_server_fn(
        &state,
        &web::audiences::RemoveSubscriber {
            request: request.clone(),
        },
        Some(&cookie),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "remove_member failed: {body}");
    assert!(
        state
            .audiences
            .list_members(author.user_id, aud_id)
            .await
            .unwrap()
            .is_empty()
    );

    // Removing a subscriber who is no longer a member is a no-op, not an error.
    let (status, body) = post_server_fn(
        &state,
        &web::audiences::RemoveSubscriber { request },
        Some(&cookie),
    )
    .await;
    assert_eq!(
        status,
        StatusCode::OK,
        "redundant remove should be a no-op: {body}"
    );
    assert!(
        state
            .audiences
            .list_members(author.user_id, aud_id)
            .await
            .unwrap()
            .is_empty()
    );
}

#[apply(backends)]
#[tokio::test]
async fn remove_subscriber_nested_request_maps_both_ids(#[case] backend: Backend) {
    let TestEnv { state, base: _base } = backend.setup().await;
    let author = create_user_and_session(&state).await;
    let subscriber = SeedUser::new().seed(&state).await.user_id;
    let cookie = author.cookie();
    let channel = state.subscriptions.local_channel_id().await.unwrap();
    let subscription_id = state
        .subscriptions
        .subscribe(author.user_id, channel, &i64::from(subscriber).to_string())
        .await
        .unwrap();
    state
        .audiences
        .create_audience(author.user_id, &parse_audience_name("Decoy"))
        .await
        .unwrap();
    let audience_id = state
        .audiences
        .create_audience(author.user_id, &parse_audience_name("Remove target"))
        .await
        .unwrap();
    assert_ne!(
        i64::from(audience_id),
        i64::from(subscription_id),
        "sentinel ids must differ so a transposition cannot pass"
    );
    state
        .audiences
        .add_member(author.user_id, audience_id, subscription_id)
        .await
        .unwrap();

    let (status, body) = post_server_fn(
        &state,
        &web::audiences::RemoveSubscriber {
            request: web::audiences::AudienceMembershipRequest {
                audience_id,
                subscription_id,
            },
        },
        Some(&cookie),
    )
    .await;

    assert_eq!(status, StatusCode::OK, "remove_member failed: {body}");
    assert!(
        state
            .audiences
            .list_members(author.user_id, audience_id)
            .await
            .unwrap()
            .is_empty()
    );
}

// AUTHORIZATION: every store method is author-scoped, so a client-supplied
// audience_id owned by another author matches nothing — the request succeeds but
// sees/changes none of the other author's data. (The storage-layer guarantee is
// covered by `audience_members_are_author_scoped`.)
#[apply(backends)]
#[tokio::test]
async fn cross_author_audience_id_is_scoped_away(#[case] backend: Backend) {
    let TestEnv { state, base: _base } = backend.setup().await;
    let alice = SeedUser::new().seed(&state).await.user_id;
    let subscriber = SeedUser::new().seed(&state).await.user_id;
    let channel = state.subscriptions.local_channel_id().await.unwrap();
    // Alice owns an audience with a member.
    let alice_sub = state
        .subscriptions
        .subscribe(alice, channel, &i64::from(subscriber).to_string())
        .await
        .unwrap();
    let alice_aud = state
        .audiences
        .create_audience(alice, &parse_audience_name("Secret"))
        .await
        .unwrap();
    state
        .audiences
        .add_member(alice, alice_aud, alice_sub)
        .await
        .unwrap();
    let bob_cookie = create_user_and_session(&state).await.cookie();

    // Bob lists Alice's audience members → succeeds, but sees nothing of hers.
    let (status, body) = post_form(
        &state,
        <web::audiences::ListMembers as ServerFn>::PATH,
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
    let (status, body) = post_server_fn(
        &state,
        &web::audiences::RemoveSubscriber {
            request: web::audiences::AudienceMembershipRequest {
                audience_id: alice_aud,
                subscription_id: alice_sub,
            },
        },
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
    let author = create_user_and_session(&state).await;
    let subscriber = SeedUser::new().seed(&state).await;
    let cookie = author.cookie();
    let channel = state.subscriptions.local_channel_id().await.unwrap();
    state
        .subscriptions
        .subscribe(
            author.user_id,
            channel,
            &i64::from(subscriber.user_id).to_string(),
        )
        .await
        .unwrap();

    let (status, body) = post_form(
        &state,
        <web::audiences::ListMySubscribers as ServerFn>::PATH,
        "",
        Some(&cookie),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert!(
        body.contains(&*subscriber.username),
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
        (<web::audiences::Create as ServerFn>::PATH, "name=Friends"),
        (
            <web::audiences::Rename as ServerFn>::PATH,
            "audience_id=1&name=X",
        ),
        (<web::audiences::Delete as ServerFn>::PATH, "audience_id=1"),
        (<web::audiences::ListMine as ServerFn>::PATH, ""),
        (<web::audiences::ListMySubscribers as ServerFn>::PATH, ""),
        (
            <web::audiences::AddSubscriber as ServerFn>::PATH,
            "audience_id=1&subscription_id=1",
        ),
        (
            <web::audiences::RemoveSubscriber as ServerFn>::PATH,
            "audience_id=1&subscription_id=1",
        ),
        (
            <web::audiences::ListMembers as ServerFn>::PATH,
            "audience_id=1",
        ),
    ];
    for (uri, body) in endpoints {
        let (status, _body) = post_form(&state, uri, body, None).await;
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
// a silent no-op. A cross-author write must be refused at the boundary, not merely
// scoped away.
#[apply(backends)]
#[tokio::test]
async fn cross_author_add_member_is_rejected(#[case] backend: Backend) {
    let TestEnv { state, base: _base } = backend.setup().await;
    let alice = SeedUser::new().seed(&state).await.user_id;
    let subscriber = SeedUser::new().seed(&state).await.user_id;
    let channel = state.subscriptions.local_channel_id().await.unwrap();
    // Alice owns a subscription and an audience (no members yet).
    let alice_sub = state
        .subscriptions
        .subscribe(alice, channel, &i64::from(subscriber).to_string())
        .await
        .unwrap();
    let alice_aud = state
        .audiences
        .create_audience(alice, &parse_audience_name("Secret"))
        .await
        .unwrap();
    let bob_cookie = create_user_and_session(&state).await.cookie();

    // Bob tries to inject Alice's subscription into Alice's audience.
    let (status, body) = post_server_fn(
        &state,
        &web::audiences::AddSubscriber {
            request: web::audiences::AudienceMembershipRequest {
                audience_id: alice_aud,
                subscription_id: alice_sub,
            },
        },
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
// path is pinned.
#[apply(backends)]
#[tokio::test]
async fn cross_author_rename_and_delete_are_scoped(#[case] backend: Backend) {
    let TestEnv { state, base: _base } = backend.setup().await;
    let alice = SeedUser::new().seed(&state).await.user_id;
    let alice_aud = state
        .audiences
        .create_audience(alice, &parse_audience_name("Secret"))
        .await
        .unwrap();
    let bob_cookie = create_user_and_session(&state).await.cookie();

    // Bob renames Alice's audience → refused (store NotFound); name unchanged.
    let (status, body) = post_server_fn(
        &state,
        &web::audiences::Rename {
            request: web::audiences::RenameAudienceRequest {
                audience_id: alice_aud,
                name: parse_audience_name("Hijacked"),
            },
        },
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
        &state,
        <web::audiences::Delete as ServerFn>::PATH,
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
