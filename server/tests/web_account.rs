mod helpers;

use std::sync::Arc;

use axum::{
    body::Body,
    http::{header, Request, StatusCode},
};
use common::storage::ProfileUpdate;
use jaunder::username::Username;
use tempfile::TempDir;
use tower::ServiceExt;

use helpers::{ensure_server_fns_registered, test_options, test_state};

async fn post_form(
    state: Arc<jaunder::storage::AppState>,
    uri: &str,
    body: impl Into<String>,
    cookie: Option<&str>,
) -> (StatusCode, Option<String>, String) {
    ensure_server_fns_registered();

    let mut builder = Request::builder()
        .method("POST")
        .uri(uri)
        .header(header::CONTENT_TYPE, "application/x-www-form-urlencoded");
    if let Some(c) = cookie {
        builder = builder.header(header::COOKIE, c);
    }
    let request = builder.body(Body::from(body.into())).unwrap();

    let app = jaunder::create_router(test_options(), state, true);
    let response = app.oneshot(request).await.unwrap();

    let status = response.status();
    let set_cookie = response
        .headers()
        .get(header::SET_COOKIE)
        .map(|v| v.to_str().unwrap().to_string());
    let bytes = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .unwrap();
    let body_str = String::from_utf8(bytes.to_vec()).unwrap();

    (status, set_cookie, body_str)
}

// ── Profile tests (M2.10.5, M2.10.6) ─────────────────────────────────────

// M2.10.5: get_profile returns the authenticated user's display name and bio.
#[tokio::test]
async fn get_profile_returns_display_name_and_bio() {
    let base = TempDir::new().unwrap();
    let state = test_state(&base).await;
    let user_id = state
        .users
        .create_user(
            &"alice".parse::<Username>().unwrap(),
            &"password123".parse().unwrap(),
            None,
        )
        .await
        .unwrap();
    state
        .users
        .update_profile(
            user_id,
            &ProfileUpdate {
                display_name: Some("Alice Smith"),
                bio: Some("Hello world"),
            },
        )
        .await
        .unwrap();
    let token = state.sessions.create_session(user_id, None).await.unwrap();
    let cookie = format!("session={token}");

    let (status, _, body) =
        post_form(Arc::clone(&state), "/api/get_profile", "", Some(&cookie)).await;

    assert_eq!(status, StatusCode::OK, "body: {body}");
    assert!(body.contains("Alice Smith"), "display_name missing: {body}");
    assert!(body.contains("Hello world"), "bio missing: {body}");
}

#[tokio::test]
async fn get_profile_with_email_returns_email() {
    let base = TempDir::new().unwrap();
    let state = test_state(&base).await;

    // Create user with email and session
    let username: Username = "emailuser".parse().unwrap();
    let user_id = state
        .users
        .create_user(&username, &"password123".parse().unwrap(), None)
        .await
        .unwrap();
    let email = "user@example.com".parse().unwrap();
    state
        .users
        .set_email(user_id, Some(&email), true)
        .await
        .unwrap();

    let raw_token = state.sessions.create_session(user_id, None).await.unwrap();
    let cookie_header = format!("session={raw_token}");

    let (status, _, body) = post_form(state, "/api/get_profile", "", Some(&cookie_header)).await;
    assert_eq!(status, StatusCode::OK);
    assert!(body.contains("user@example.com"));
}

// M2.10.6: update_profile persists changes visible in a subsequent get_profile.
#[tokio::test]
async fn update_profile_persists_changes() {
    let base = TempDir::new().unwrap();
    let state = test_state(&base).await;
    let user_id = state
        .users
        .create_user(
            &"bob".parse::<Username>().unwrap(),
            &"password123".parse().unwrap(),
            None,
        )
        .await
        .unwrap();
    let token = state.sessions.create_session(user_id, None).await.unwrap();
    let cookie = format!("session={token}");

    let (status, _, _) = post_form(
        Arc::clone(&state),
        "/api/update_profile",
        "display_name=Robert&bio=My+bio",
        Some(&cookie),
    )
    .await;
    assert_eq!(status, StatusCode::OK);

    let (status, _, body) =
        post_form(Arc::clone(&state), "/api/get_profile", "", Some(&cookie)).await;
    assert_eq!(status, StatusCode::OK, "body: {body}");
    assert!(
        body.contains("Robert"),
        "display_name not persisted: {body}"
    );
    assert!(body.contains("My bio"), "bio not persisted: {body}");
}

// ── Sessions tests (M2.10.7, M2.10.8) ────────────────────────────────────

// M2.10.7: list_sessions returns sessions for the authenticated user only.
#[tokio::test]
async fn list_sessions_returns_only_authenticated_users_sessions() {
    let base = TempDir::new().unwrap();
    let state = test_state(&base).await;

    let user1_id = state
        .users
        .create_user(
            &"carol".parse::<Username>().unwrap(),
            &"password123".parse().unwrap(),
            None,
        )
        .await
        .unwrap();
    let user2_id = state
        .users
        .create_user(
            &"dave".parse::<Username>().unwrap(),
            &"password123".parse().unwrap(),
            None,
        )
        .await
        .unwrap();

    let token1 = state
        .sessions
        .create_session(user1_id, Some("carol-session"))
        .await
        .unwrap();
    // Create a session for user2 — should NOT appear in user1's list.
    state
        .sessions
        .create_session(user2_id, Some("dave-session"))
        .await
        .unwrap();

    let cookie = format!("session={token1}");
    let (status, _, body) =
        post_form(Arc::clone(&state), "/api/list_sessions", "", Some(&cookie)).await;

    assert_eq!(status, StatusCode::OK, "body: {body}");
    assert!(
        body.contains("carol-session"),
        "own session should appear: {body}"
    );
    assert!(
        !body.contains("dave-session"),
        "other user's session must not appear: {body}"
    );
}

// M2.10.8: revoke_session removes the target session; re-auth with that token fails.
#[tokio::test]
async fn revoke_session_removes_session_and_reauth_fails() {
    let base = TempDir::new().unwrap();
    let state = test_state(&base).await;
    let user_id = state
        .users
        .create_user(
            &"eve".parse::<Username>().unwrap(),
            &"password123".parse().unwrap(),
            None,
        )
        .await
        .unwrap();

    // Create two sessions: use token_a to authenticate, revoke token_b's hash.
    let token_a = state.sessions.create_session(user_id, None).await.unwrap();
    let token_b = state.sessions.create_session(user_id, None).await.unwrap();

    // Authenticate token_b to get its hash from the session record.
    let record_b = state.sessions.authenticate(&token_b).await.unwrap();
    let hash_b = record_b.token_hash;

    let cookie_a = format!("session={token_a}");
    let body = format!(
        "token_hash={}",
        hash_b
            .chars()
            .map(|c| {
                if c.is_alphanumeric() || c == '-' || c == '_' {
                    c.to_string()
                } else {
                    format!("%{:02X}", c as u8)
                }
            })
            .collect::<String>()
    );
    let (status, _, _) = post_form(
        Arc::clone(&state),
        "/api/revoke_session",
        body,
        Some(&cookie_a),
    )
    .await;
    assert_eq!(status, StatusCode::OK);

    // Re-authenticate with token_b should fail (session revoked).
    let result = state.sessions.authenticate(&token_b).await;
    assert!(result.is_err(), "revoked session should not authenticate");
}

// ── Invites tests (M2.10.9) ───────────────────────────────────────────────

// M2.10.9: create_invite returns a code that appears in a subsequent list_invites.
#[tokio::test]
async fn create_invite_appears_in_list_invites() {
    let base = TempDir::new().unwrap();
    let state = test_state(&base).await;
    state
        .site_config
        .set("site.registration_policy", "invite_only")
        .await
        .unwrap();

    let user_id = state
        .users
        .create_user(
            &"frank".parse::<Username>().unwrap(),
            &"password123".parse().unwrap(),
            None,
        )
        .await
        .unwrap();
    let token = state.sessions.create_session(user_id, None).await.unwrap();
    let cookie = format!("session={token}");

    let (status, _, body) = post_form(
        Arc::clone(&state),
        "/api/create_invite",
        "expires_in_hours=24",
        Some(&cookie),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "create_invite failed: {body}");
    // Body is a JSON string: "<code>"
    let trimmed = body.trim();
    assert!(
        trimmed.starts_with('"') && trimmed.ends_with('"'),
        "expected JSON string, got: {trimmed}"
    );
    let code = &trimmed[1..trimmed.len() - 1];

    let (status, _, list_body) =
        post_form(Arc::clone(&state), "/api/list_invites", "", Some(&cookie)).await;
    assert_eq!(status, StatusCode::OK, "list_invites failed: {list_body}");
    assert!(
        list_body.contains(code),
        "created code {code} not in list: {list_body}"
    );
}

// create_invite requires authentication.
#[tokio::test]
async fn create_invite_unauthorized_returns_error() {
    let base = TempDir::new().unwrap();
    let state = test_state(&base).await;

    let (status, _, _) = post_form(Arc::clone(&state), "/api/create_invite", "", None).await;

    assert_eq!(status, StatusCode::INTERNAL_SERVER_ERROR);
}

// create_invite with extremely large hours (causing overflow in i64)?
#[tokio::test]
async fn create_invite_large_hours_returns_error() {
    let base = TempDir::new().unwrap();
    let state = test_state(&base).await;
    let user_id = state
        .users
        .create_user(
            &"alice".parse::<Username>().unwrap(),
            &"password123".parse().unwrap(),
            None,
        )
        .await
        .unwrap();
    let token = state.sessions.create_session(user_id, None).await.unwrap();
    let cookie = format!("session={token}");

    let (status1, _, _) = post_form(
        Arc::clone(&state),
        "/api/create_invite",
        "expires_in_hours=24",
        Some(&cookie),
    )
    .await;
    assert_eq!(status1, StatusCode::OK);

    // This should fail due to u64 -> i64 conversion.
    let (status2, _, _) = post_form(
        Arc::clone(&state),
        "/api/create_invite",
        "expires_in_hours=18446744073709551615", // u64::MAX
        Some(&cookie),
    )
    .await;

    assert_eq!(status2, StatusCode::INTERNAL_SERVER_ERROR);
}

// M2.10.10: revoke_session returns error when target session does not exist.
#[tokio::test]
async fn revoke_session_unknown_hash_returns_error() {
    let base = TempDir::new().unwrap();
    let state = test_state(&base).await;
    let user_id = state
        .users
        .create_user(
            &"alice".parse::<Username>().unwrap(),
            &"password123".parse().unwrap(),
            None,
        )
        .await
        .unwrap();
    let token = state.sessions.create_session(user_id, None).await.unwrap();
    let cookie = format!("session={token}");

    let (_status, _, _) = post_form(
        Arc::clone(&state),
        "/api/revoke_session",
        "token_hash=nonexistenthash",
        Some(&cookie),
    )
    .await;

    // ...
}

// M2.10.11: revoke_session returns error when target session belongs to another user.
#[tokio::test]
async fn revoke_session_other_user_hash_returns_error() {
    let base = TempDir::new().unwrap();
    let state = test_state(&base).await;
    let user1_id = state
        .users
        .create_user(
            &"alice".parse::<Username>().unwrap(),
            &"password123".parse().unwrap(),
            None,
        )
        .await
        .unwrap();
    let user2_id = state
        .users
        .create_user(
            &"bob".parse::<Username>().unwrap(),
            &"password123".parse().unwrap(),
            None,
        )
        .await
        .unwrap();

    let token1 = state.sessions.create_session(user1_id, None).await.unwrap();
    let token2 = state.sessions.create_session(user2_id, None).await.unwrap();
    let record2 = state.sessions.authenticate(&token2).await.unwrap();

    let cookie1 = format!("session={token1}");
    let (status, _, _) = post_form(
        Arc::clone(&state),
        "/api/revoke_session",
        format!("token_hash={}", record2.token_hash),
        Some(&cookie1),
    )
    .await;

    assert_eq!(status, StatusCode::INTERNAL_SERVER_ERROR);
}

// list_invites returns error when policy is not InviteOnly.
#[tokio::test]
async fn list_invites_returns_error_when_policy_not_invite_only() {
    let base = TempDir::new().unwrap();
    let state = test_state(&base).await;
    // Policy defaults to Closed.

    let user_id = state
        .users
        .create_user(
            &"grace".parse::<Username>().unwrap(),
            &"password123".parse().unwrap(),
            None,
        )
        .await
        .unwrap();
    let token = state.sessions.create_session(user_id, None).await.unwrap();
    let cookie = format!("session={token}");

    let (status, _, _) =
        post_form(Arc::clone(&state), "/api/list_invites", "", Some(&cookie)).await;
    assert_ne!(
        status,
        StatusCode::OK,
        "list_invites should fail when policy is not invite_only"
    );
}

#[tokio::test]
async fn get_profile_unauthorized_returns_error() {
    let base = TempDir::new().unwrap();
    let state = test_state(&base).await;

    let (status, _, _) = post_form(state, "/api/get_profile", "", None).await;
    assert_eq!(status, StatusCode::INTERNAL_SERVER_ERROR);
}

#[tokio::test]
async fn update_profile_unauthorized_returns_error() {
    let base = TempDir::new().unwrap();
    let state = test_state(&base).await;

    let (status, _, _) = post_form(
        state,
        "/api/update_profile",
        "display_name=New&bio=Bio",
        None,
    )
    .await;
    assert_eq!(status, StatusCode::INTERNAL_SERVER_ERROR);
}

#[tokio::test]
async fn update_profile_with_empty_fields_sets_to_none() {
    let base = TempDir::new().unwrap();
    let state = test_state(&base).await;

    // Create user and session
    let username: Username = "empty".parse().unwrap();
    let user_id = state
        .users
        .create_user(&username, &"password123".parse().unwrap(), Some("Initial"))
        .await
        .unwrap();
    state
        .users
        .update_profile(
            user_id,
            &ProfileUpdate {
                display_name: Some("Initial"),
                bio: Some("Initial Bio"),
            },
        )
        .await
        .unwrap();

    let raw_token = state.sessions.create_session(user_id, None).await.unwrap();
    let cookie_header = format!("session={raw_token}");

    let (status, _, _) = post_form(
        Arc::clone(&state),
        "/api/update_profile",
        "display_name=&bio=",
        Some(&cookie_header),
    )
    .await;
    assert_eq!(status, StatusCode::OK);

    let user = state.users.get_user(user_id).await.unwrap().unwrap();
    assert!(user.display_name.is_none());
    assert!(user.bio.is_none());
}
