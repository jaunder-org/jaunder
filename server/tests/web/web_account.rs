use std::sync::Arc;

use axum::http::StatusCode;
use common::mailer::test_utils::CapturingMailSender;
use common::mailer::MailSender;
use common::username::Username;
use storage::{AppState, ProfileUpdate};

use rstest::*;
use rstest_reuse::*;

use crate::helpers::{post_form, post_form_with_mailer};
use storage::test_support::{backends, Backend, TestEnv};

// ── Profile tests (M2.10.5, M2.10.6) ─────────────────────────────────────

// M2.10.5: get_profile returns the authenticated user's display name and bio.
#[apply(backends)]
#[tokio::test]
async fn get_profile_returns_display_name_and_bio(#[case] backend: Backend) {
    let TestEnv { state, base: _base } = backend.setup().await;
    let user_id = state
        .users
        .create_user(
            &"alice".parse::<Username>().unwrap(),
            &"password123".parse().unwrap(),
            None,
            false,
        )
        .await
        .unwrap();
    state
        .users
        .update_profile(
            user_id,
            &ProfileUpdate {
                display_name: Some(&"Alice Smith".parse().unwrap()),
                bio: Some("Hello world"),
            },
        )
        .await
        .unwrap();
    let token = state
        .sessions
        .create_session(user_id, "test session")
        .await
        .unwrap();
    let cookie = format!("session={token}");

    let (status, body) = post_form(Arc::clone(&state), "/api/get_profile", "", Some(&cookie)).await;

    assert_eq!(status, StatusCode::OK, "body: {body}");
    assert!(body.contains("Alice Smith"), "display_name missing: {body}");
    assert!(body.contains("Hello world"), "bio missing: {body}");
}

#[apply(backends)]
#[tokio::test]
async fn get_profile_with_email_returns_email(#[case] backend: Backend) {
    let TestEnv { state, base: _base } = backend.setup().await;

    // Create user with email and session
    let username: Username = "emailuser".parse().unwrap();
    let user_id = state
        .users
        .create_user(&username, &"password123".parse().unwrap(), None, false)
        .await
        .unwrap();
    let email = "user@example.com".parse().unwrap();
    state
        .users
        .set_email(user_id, Some(&email), true)
        .await
        .unwrap();

    let raw_token = state
        .sessions
        .create_session(user_id, "test session")
        .await
        .unwrap();
    let cookie_header = format!("session={raw_token}");

    let (status, body) = post_form(state, "/api/get_profile", "", Some(&cookie_header)).await;
    assert_eq!(status, StatusCode::OK);
    assert!(body.contains("user@example.com"));
}

// M2.10.6: update_profile persists changes visible in a subsequent get_profile.
#[apply(backends)]
#[tokio::test]
async fn update_profile_persists_changes(#[case] backend: Backend) {
    let TestEnv { state, base: _base } = backend.setup().await;
    let user_id = state
        .users
        .create_user(
            &"bob".parse::<Username>().unwrap(),
            &"password123".parse().unwrap(),
            None,
            false,
        )
        .await
        .unwrap();
    let token = state
        .sessions
        .create_session(user_id, "test session")
        .await
        .unwrap();
    let cookie = format!("session={token}");

    let (status, _) = post_form(
        Arc::clone(&state),
        "/api/update_profile",
        "display_name=Robert&bio=My+bio",
        Some(&cookie),
    )
    .await;
    assert_eq!(status, StatusCode::OK);

    let (status, body) = post_form(Arc::clone(&state), "/api/get_profile", "", Some(&cookie)).await;
    assert_eq!(status, StatusCode::OK, "body: {body}");
    assert!(
        body.contains("Robert"),
        "display_name not persisted: {body}"
    );
    assert!(body.contains("My bio"), "bio not persisted: {body}");
}

// ── Sessions tests (M2.10.7, M2.10.8) ────────────────────────────────────

// M2.10.7: list_sessions returns sessions for the authenticated user only.
#[apply(backends)]
#[tokio::test]
async fn list_sessions_returns_only_authenticated_users_sessions(#[case] backend: Backend) {
    let TestEnv { state, base: _base } = backend.setup().await;

    let user1_id = state
        .users
        .create_user(
            &"carol".parse::<Username>().unwrap(),
            &"password123".parse().unwrap(),
            None,
            false,
        )
        .await
        .unwrap();
    let user2_id = state
        .users
        .create_user(
            &"dave".parse::<Username>().unwrap(),
            &"password123".parse().unwrap(),
            None,
            false,
        )
        .await
        .unwrap();

    let token1 = state
        .sessions
        .create_session(user1_id, "carol-session")
        .await
        .unwrap();
    // Create a session for user2 — should NOT appear in user1's list.
    state
        .sessions
        .create_session(user2_id, "dave-session")
        .await
        .unwrap();

    let cookie = format!("session={token1}");
    let (status, body) =
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
#[apply(backends)]
#[tokio::test]
async fn revoke_session_removes_session_and_reauth_fails(#[case] backend: Backend) {
    let TestEnv { state, base: _base } = backend.setup().await;
    let user_id = state
        .users
        .create_user(
            &"eve".parse::<Username>().unwrap(),
            &"password123".parse().unwrap(),
            None,
            false,
        )
        .await
        .unwrap();

    // Create two sessions: use token_a to authenticate, revoke token_b's hash.
    let token_a = state
        .sessions
        .create_session(user_id, "test session")
        .await
        .unwrap();
    let token_b = state
        .sessions
        .create_session(user_id, "test session")
        .await
        .unwrap();

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
    let (status, _) = post_form(
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

// ── Invites tests (M2.10.9, #433) ─────────────────────────────────────────

/// Create an authenticated operator and return its `session=` cookie header.
async fn operator_cookie(state: &Arc<AppState>, username: &str) -> String {
    let user_id = state
        .users
        .create_user(
            &username.parse::<Username>().unwrap(),
            &"password123".parse().unwrap(),
            None,
            false,
        )
        .await
        .unwrap();
    let token = state
        .sessions
        .create_session(user_id, "test session")
        .await
        .unwrap();
    format!("session={token}")
}

// #433: create_invite emails the invitation link to the recipient and records the invite.
#[apply(backends)]
#[tokio::test]
async fn create_invite_emails_link_and_appears_in_list(#[case] backend: Backend) {
    let TestEnv { state, base: _base } = backend.setup().await;
    state
        .site_config
        .set("site.registration_policy", "invite_only")
        .await
        .unwrap();
    state
        .site_config
        .set("site.base_url", "https://example.com")
        .await
        .unwrap();
    let cookie = operator_cookie(&state, "frank").await;
    let mailer = Arc::new(CapturingMailSender::new());

    let (status, body) = post_form_with_mailer(
        Arc::clone(&state),
        mailer.clone() as Arc<dyn MailSender>,
        "/api/create_invite",
        "expires_in_hours=24&recipient_email=invitee@example.com",
        Some(&cookie),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "create_invite failed: {body}");

    let sent = mailer.sent();
    assert_eq!(sent.len(), 1, "expected one invite email");
    assert_eq!(sent[0].to.len(), 1);
    assert_eq!(sent[0].to[0], "invitee@example.com");
    assert!(
        sent[0]
            .body_text
            .contains("https://example.com/register?invite_code="),
        "email should contain the invite link, got: {}",
        sent[0].body_text
    );

    // The invite is tracked — as metadata only, never the raw code.
    let (status, list_body) =
        post_form(Arc::clone(&state), "/api/list_invites", "", Some(&cookie)).await;
    assert_eq!(status, StatusCode::OK, "list_invites failed: {list_body}");
    assert!(
        list_body.contains("expires_at"),
        "created invite not in list: {list_body}"
    );
    assert!(
        !list_body.contains("\"code\""),
        "list_invites must not expose an invite code: {list_body}"
    );
}

// create_invite requires authentication (valid body, but no session cookie).
#[apply(backends)]
#[tokio::test]
async fn create_invite_unauthorized_returns_error(#[case] backend: Backend) {
    let TestEnv { state, base: _base } = backend.setup().await;

    let (status, _) = post_form(
        Arc::clone(&state),
        "/api/create_invite",
        "recipient_email=invitee@example.com",
        None,
    )
    .await;

    assert_eq!(status, StatusCode::INTERNAL_SERVER_ERROR);
}

// create_invite errors when the site base URL is unset — and sends no email (no orphan).
#[apply(backends)]
#[tokio::test]
async fn create_invite_without_base_url_errors_and_sends_nothing(#[case] backend: Backend) {
    let TestEnv { state, base: _base } = backend.setup().await;
    let cookie = operator_cookie(&state, "frank").await;
    let mailer = Arc::new(CapturingMailSender::new());

    let (status, _) = post_form_with_mailer(
        Arc::clone(&state),
        mailer.clone() as Arc<dyn MailSender>,
        "/api/create_invite",
        "expires_in_hours=24&recipient_email=invitee@example.com",
        Some(&cookie),
    )
    .await;

    assert_ne!(status, StatusCode::OK, "a missing base URL must error");
    assert!(
        mailer.sent().is_empty(),
        "no email must be sent when the base URL is unset"
    );
}

// create_invite rejects a malformed recipient address before creating any invite.
#[apply(backends)]
#[tokio::test]
async fn create_invite_invalid_recipient_returns_error(#[case] backend: Backend) {
    let TestEnv { state, base: _base } = backend.setup().await;
    state
        .site_config
        .set("site.base_url", "https://example.com")
        .await
        .unwrap();
    let cookie = operator_cookie(&state, "frank").await;
    let mailer = Arc::new(CapturingMailSender::new());

    let (status, _) = post_form_with_mailer(
        Arc::clone(&state),
        mailer.clone() as Arc<dyn MailSender>,
        "/api/create_invite",
        "expires_in_hours=24&recipient_email=not-an-email",
        Some(&cookie),
    )
    .await;

    assert_ne!(status, StatusCode::OK, "a malformed recipient must error");
    assert!(
        mailer.sent().is_empty(),
        "no email must be sent for a malformed recipient"
    );
}

// create_invite propagates a mail-send failure (the noop mailer reports NotConfigured).
#[apply(backends)]
#[tokio::test]
async fn create_invite_send_failure_returns_error(#[case] backend: Backend) {
    let TestEnv { state, base: _base } = backend.setup().await;
    state
        .site_config
        .set("site.base_url", "https://example.com")
        .await
        .unwrap();
    let cookie = operator_cookie(&state, "frank").await;

    // `post_form` uses the noop mailer, whose `send_email` fails with NotConfigured.
    let (status, _) = post_form(
        Arc::clone(&state),
        "/api/create_invite",
        "expires_in_hours=24&recipient_email=invitee@example.com",
        Some(&cookie),
    )
    .await;

    assert_ne!(status, StatusCode::OK, "a mail-send failure must error");
}

// create_invite with an out-of-range expiry (u64::MAX hours) errors before emailing.
#[apply(backends)]
#[tokio::test]
async fn create_invite_large_hours_returns_error(#[case] backend: Backend) {
    let TestEnv { state, base: _base } = backend.setup().await;
    state
        .site_config
        .set("site.base_url", "https://example.com")
        .await
        .unwrap();
    let cookie = operator_cookie(&state, "alice").await;
    let mailer = Arc::new(CapturingMailSender::new());

    let (status, _) = post_form_with_mailer(
        Arc::clone(&state),
        mailer.clone() as Arc<dyn MailSender>,
        "/api/create_invite",
        "expires_in_hours=18446744073709551615&recipient_email=invitee@example.com", // u64::MAX
        Some(&cookie),
    )
    .await;

    assert_eq!(status, StatusCode::INTERNAL_SERVER_ERROR);
    // The overflow must be caught before any email is attempted (proves ordering).
    assert!(
        mailer.sent().is_empty(),
        "an out-of-range expiry must error before emailing"
    );
}

// M2.10.10: revoke_session returns error when target session does not exist.
#[apply(backends)]
#[tokio::test]
async fn revoke_session_unknown_hash_returns_error(#[case] backend: Backend) {
    let TestEnv { state, base: _base } = backend.setup().await;
    let user_id = state
        .users
        .create_user(
            &"alice".parse::<Username>().unwrap(),
            &"password123".parse().unwrap(),
            None,
            false,
        )
        .await
        .unwrap();
    let token = state
        .sessions
        .create_session(user_id, "test session")
        .await
        .unwrap();
    let cookie = format!("session={token}");

    let (_status, _) = post_form(
        Arc::clone(&state),
        "/api/revoke_session",
        "token_hash=nonexistenthash",
        Some(&cookie),
    )
    .await;

    // ...
}

// M2.10.11: revoke_session returns error when target session belongs to another user.
#[apply(backends)]
#[tokio::test]
async fn revoke_session_other_user_hash_returns_error(#[case] backend: Backend) {
    let TestEnv { state, base: _base } = backend.setup().await;
    let user1_id = state
        .users
        .create_user(
            &"alice".parse::<Username>().unwrap(),
            &"password123".parse().unwrap(),
            None,
            false,
        )
        .await
        .unwrap();
    let user2_id = state
        .users
        .create_user(
            &"bob".parse::<Username>().unwrap(),
            &"password123".parse().unwrap(),
            None,
            false,
        )
        .await
        .unwrap();

    let token1 = state
        .sessions
        .create_session(user1_id, "test session")
        .await
        .unwrap();
    let token2 = state
        .sessions
        .create_session(user2_id, "test session")
        .await
        .unwrap();
    let record2 = state.sessions.authenticate(&token2).await.unwrap();

    let cookie1 = format!("session={token1}");
    let (status, _) = post_form(
        Arc::clone(&state),
        "/api/revoke_session",
        format!("token_hash={}", record2.token_hash),
        Some(&cookie1),
    )
    .await;

    assert_eq!(status, StatusCode::INTERNAL_SERVER_ERROR);
}

// list_invites returns error when policy is not InviteOnly.
#[apply(backends)]
#[tokio::test]
async fn list_invites_returns_error_when_policy_not_invite_only(#[case] backend: Backend) {
    let TestEnv { state, base: _base } = backend.setup().await;
    // Policy defaults to Closed.

    let user_id = state
        .users
        .create_user(
            &"grace".parse::<Username>().unwrap(),
            &"password123".parse().unwrap(),
            None,
            false,
        )
        .await
        .unwrap();
    let token = state
        .sessions
        .create_session(user_id, "test session")
        .await
        .unwrap();
    let cookie = format!("session={token}");

    let (status, _) = post_form(Arc::clone(&state), "/api/list_invites", "", Some(&cookie)).await;
    assert_ne!(
        status,
        StatusCode::OK,
        "list_invites should fail when policy is not invite_only"
    );
}

#[apply(backends)]
#[tokio::test]
async fn get_profile_unauthorized_returns_error(#[case] backend: Backend) {
    let TestEnv { state, base: _base } = backend.setup().await;

    let (status, _) = post_form(state, "/api/get_profile", "", None).await;
    assert_eq!(status, StatusCode::INTERNAL_SERVER_ERROR);
}

#[apply(backends)]
#[tokio::test]
async fn update_profile_unauthorized_returns_error(#[case] backend: Backend) {
    let TestEnv { state, base: _base } = backend.setup().await;

    let (status, _) = post_form(
        state,
        "/api/update_profile",
        "display_name=New&bio=Bio",
        None,
    )
    .await;
    assert_eq!(status, StatusCode::INTERNAL_SERVER_ERROR);
}

#[apply(backends)]
#[tokio::test]
async fn update_profile_with_empty_fields_sets_to_none(#[case] backend: Backend) {
    let TestEnv { state, base: _base } = backend.setup().await;

    let username: Username = "empty".parse().unwrap();
    let user_id = state
        .users
        .create_user(
            &username,
            &"password123".parse().unwrap(),
            Some(&"Initial".parse().unwrap()),
            false,
        )
        .await
        .unwrap();
    state
        .users
        .update_profile(
            user_id,
            &ProfileUpdate {
                display_name: Some(&"Initial".parse().unwrap()),
                bio: Some("Initial Bio"),
            },
        )
        .await
        .unwrap();

    let raw_token = state
        .sessions
        .create_session(user_id, "test session")
        .await
        .unwrap();
    let cookie_header = format!("session={raw_token}");

    // Clearing the display name is the dispatch-`None` path: the typed
    // `Option<DisplayName>` wire arg is *omitted* (serde decodes a missing
    // Option field to `None`); an empty `display_name=` would instead fail to
    // parse. Empty `bio=` clears via `non_empty`.
    let (status, _) = post_form(
        Arc::clone(&state),
        "/api/update_profile",
        "bio=",
        Some(&cookie_header),
    )
    .await;
    assert_eq!(status, StatusCode::OK);

    let user = state.users.get_user(user_id).await.unwrap().unwrap();
    assert!(user.display_name.is_none());
    assert!(user.bio.is_none());
}

#[apply(backends)]
#[tokio::test]
async fn update_profile_rejects_invalid_display_name(#[case] backend: Backend) {
    // A malformed display_name (over the 255-char bound) fails at typed-arg
    // decode — a non-OK server-function error, not a Validation message
    // (ADR-0065). The client's disable-until-valid gate keeps a real browser
    // from reaching this; a raw POST is the malformed-client path. Mirrors
    // web_auth::register_invalid_username_returns_error.
    let TestEnv { state, base: _base } = backend.setup().await;
    let username: Username = "invalid_dn".parse().unwrap();
    let user_id = state
        .users
        .create_user(&username, &"password123".parse().unwrap(), None, false)
        .await
        .unwrap();
    let raw_token = state
        .sessions
        .create_session(user_id, "test session")
        .await
        .unwrap();
    let cookie_header = format!("session={raw_token}");

    let overlong = "a".repeat(common::display_name::MAX_DISPLAY_NAME_CHARS + 1);
    let (status, _) = post_form(
        Arc::clone(&state),
        "/api/update_profile",
        &format!("display_name={overlong}&bio=x"),
        Some(&cookie_header),
    )
    .await;

    assert_ne!(status, StatusCode::OK, "over-long display_name should fail");
    // Store side-effect did not happen.
    let user = state.users.get_user(user_id).await.unwrap().unwrap();
    assert!(
        user.display_name.is_none(),
        "invalid display_name must not persist"
    );
}
