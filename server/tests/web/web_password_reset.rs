use std::sync::Arc;

use axum::http::StatusCode;
use chrono::Utc;
use common::mailer::test_utils::CapturingMailSender;
use common::test_support::parse_email;
use common::username::Username;
use storage::AppState;

use crate::helpers::{
    assert_no_email, assert_one_absolute_link_email, create_session_for, create_user_and_session,
    post_form_with_mailer, setup_with_base_url, SeededSession,
};
use storage::test_support::{backends, Backend, SeedUser, TestEnv};

use rstest::*;
use rstest_reuse::*;

/// Creates a user with a verified email address and an authenticated session.
async fn create_user_with_verified_email(
    state: &Arc<AppState>,
    username: &str,
    email: &str,
) -> SeededSession {
    let session = create_user_and_session(state, username).await;
    state
        .users
        .set_email(session.user_id, Some(&parse_email(email)), true)
        .await
        .expect("set verified email");
    session
}

// M3.11.7: request_password_reset for a user with a verified email sends a reset email.
#[apply(backends)]
#[tokio::test]
async fn request_password_reset_sends_email_for_verified_user(#[case] backend: Backend) {
    // The reset email now composes an absolute link, so the flow requires a seeded
    // `site.base_url` (canonicalized to `https://example.com/`).
    let TestEnv { state, base: _base } = setup_with_base_url(backend).await;
    let mailer = Arc::new(CapturingMailSender::new());

    create_user_with_verified_email(&state, "alice", "alice@example.com").await;

    let (status, _body) = post_form_with_mailer(
        Arc::clone(&state),
        mailer.clone() as Arc<dyn common::mailer::MailSender>,
        "/api/request_password_reset",
        "username=alice",
        None,
    )
    .await;

    assert_eq!(status, StatusCode::OK);
    assert_one_absolute_link_email(&mailer, "alice@example.com", "/reset-password");
}

// The reset email now composes an absolute link, so an eligible request still
// fails (after confirming the user) without a seeded `site.base_url`, rather than
// emailing a dead relative link.
#[apply(backends)]
#[tokio::test]
async fn request_password_reset_without_base_url_returns_error(#[case] backend: Backend) {
    let TestEnv { state, base: _base } = backend.setup().await; // no base_url seeded
    let mailer = Arc::new(CapturingMailSender::new());

    create_user_with_verified_email(&state, "alice", "alice@example.com").await;

    let (status, _body) = post_form_with_mailer(
        Arc::clone(&state),
        mailer.clone() as Arc<dyn common::mailer::MailSender>,
        "/api/request_password_reset",
        "username=alice",
        None,
    )
    .await;

    assert_ne!(status, StatusCode::OK, "should fail without a base URL");
    assert_no_email(&mailer);
}

// M3.11.8: request_password_reset for a user without a verified email returns an error.
#[apply(backends)]
#[tokio::test]
async fn request_password_reset_returns_error_for_user_without_verified_email(
    #[case] backend: Backend,
) {
    let TestEnv { state, base: _base } = backend.setup().await;
    let mailer = Arc::new(CapturingMailSender::new());

    SeedUser::new("bob").seed(&state).await;

    let (status, _body) = post_form_with_mailer(
        Arc::clone(&state),
        mailer.clone() as Arc<dyn common::mailer::MailSender>,
        "/api/request_password_reset",
        "username=bob",
        None,
    )
    .await;

    assert_ne!(status, StatusCode::OK);
}

#[apply(backends)]
#[tokio::test]
async fn request_password_reset_invalid_username_returns_error(#[case] backend: Backend) {
    let TestEnv { state, base: _base } = backend.setup().await;
    let mailer = Arc::new(CapturingMailSender::new());

    let (status, _) = post_form_with_mailer(
        state,
        mailer.clone() as Arc<dyn common::mailer::MailSender>,
        "/api/request_password_reset",
        "username=invalid username",
        None,
    )
    .await;
    assert_eq!(status, StatusCode::INTERNAL_SERVER_ERROR);
}

// M3.11.9: request_password_reset for an unknown username returns an error.
#[apply(backends)]
#[tokio::test]
async fn request_password_reset_returns_error_for_unknown_username(#[case] backend: Backend) {
    let TestEnv { state, base: _base } = backend.setup().await;
    let mailer = Arc::new(CapturingMailSender::new());

    let (status, _body) = post_form_with_mailer(
        Arc::clone(&state),
        mailer.clone() as Arc<dyn common::mailer::MailSender>,
        "/api/request_password_reset",
        "username=nobody",
        None,
    )
    .await;

    assert_ne!(status, StatusCode::OK);
}

// M3.11.10: confirm_password_reset with a valid token sets the new password and revokes sessions.
#[apply(backends)]
#[tokio::test]
async fn confirm_password_reset_sets_password_and_revokes_sessions(#[case] backend: Backend) {
    let TestEnv { state, base: _base } = backend.setup().await;
    let mailer = Arc::new(CapturingMailSender::new());

    let session = create_user_with_verified_email(&state, "carol", "carol@example.com").await;
    let user_id = session.user_id;
    // Create a second session to ensure all are revoked
    create_session_for(&state, user_id).await;

    let expires_at = Utc::now() + chrono::Duration::hours(1);
    let raw_token = state
        .password_resets
        .create_password_reset(user_id, expires_at)
        .await
        .unwrap();

    let body = format!("token={raw_token}&new_password=newpassword456");
    let (status, _body) = post_form_with_mailer(
        Arc::clone(&state),
        mailer.clone() as Arc<dyn common::mailer::MailSender>,
        "/api/confirm_password_reset",
        body,
        None,
    )
    .await;

    assert_eq!(status, StatusCode::OK);

    // Old password should fail authentication
    let old_auth = state
        .users
        .authenticate(
            &"carol".parse::<Username>().unwrap(),
            &"password123".parse().unwrap(),
        )
        .await;
    assert!(old_auth.is_err(), "old password should no longer work");

    // New password should succeed
    let new_auth = state
        .users
        .authenticate(
            &"carol".parse::<Username>().unwrap(),
            &"newpassword456".parse().unwrap(),
        )
        .await;
    assert!(new_auth.is_ok(), "new password should work");

    // All sessions should be revoked
    let sessions = state.sessions.list_sessions(user_id).await.unwrap();
    assert!(sessions.is_empty(), "all sessions should be revoked");
}

// M3.11.11: confirm_password_reset with an expired token returns an error.
#[apply(backends)]
#[tokio::test]
async fn confirm_password_reset_with_expired_token_returns_error(#[case] backend: Backend) {
    let TestEnv { state, base: _base } = backend.setup().await;
    let mailer = Arc::new(CapturingMailSender::new());

    let user_id = create_user_with_verified_email(&state, "dave", "dave@example.com")
        .await
        .user_id;

    let expires_at = Utc::now() - chrono::Duration::hours(1);
    let raw_token = state
        .password_resets
        .create_password_reset(user_id, expires_at)
        .await
        .unwrap();

    let body = format!("token={raw_token}&new_password=newpassword456");
    let (status, _body) = post_form_with_mailer(
        Arc::clone(&state),
        mailer.clone() as Arc<dyn common::mailer::MailSender>,
        "/api/confirm_password_reset",
        body,
        None,
    )
    .await;

    assert_ne!(status, StatusCode::OK);
}

// M3.11.12: confirm_password_reset with an invalid token returns an error.
#[apply(backends)]
#[tokio::test]
async fn confirm_password_reset_with_invalid_token_returns_error(#[case] backend: Backend) {
    let TestEnv { state, base: _base } = backend.setup().await;
    let mailer = Arc::new(CapturingMailSender::new());

    let (status, _body) = post_form_with_mailer(
        Arc::clone(&state),
        mailer.clone() as Arc<dyn common::mailer::MailSender>,
        "/api/confirm_password_reset",
        "token=not-a-real-token&new_password=newpassword456",
        None,
    )
    .await;

    assert_ne!(status, StatusCode::OK);
}

#[apply(backends)]
#[tokio::test]
async fn confirm_password_reset_with_malformed_token_returns_error(#[case] backend: Backend) {
    let TestEnv { state, base: _base } = backend.setup().await;
    let mailer = Arc::new(CapturingMailSender::new());

    // `bad!token` is outside base64url, so `RawToken` rejects it (at wire-decode once
    // `token` is typed). `new_password` is valid-length, so the failure isolates to the
    // token.
    let (status, _body) = post_form_with_mailer(
        Arc::clone(&state),
        mailer.clone() as Arc<dyn common::mailer::MailSender>,
        "/api/confirm_password_reset",
        "token=bad!token&new_password=newpassword456",
        None,
    )
    .await;

    assert_ne!(
        status,
        StatusCode::OK,
        "a malformed reset token must be rejected"
    );
}

// M3.11.13: confirm_password_reset with an already-used token returns an error.
#[apply(backends)]
#[tokio::test]
async fn confirm_password_reset_with_used_token_returns_error(#[case] backend: Backend) {
    let TestEnv { state, base: _base } = backend.setup().await;
    let mailer = Arc::new(CapturingMailSender::new());

    let user_id = create_user_with_verified_email(&state, "eve", "eve@example.com")
        .await
        .user_id;

    let expires_at = Utc::now() + chrono::Duration::hours(1);
    let raw_token = state
        .password_resets
        .create_password_reset(user_id, expires_at)
        .await
        .unwrap();

    let body = format!("token={raw_token}&new_password=newpassword456");

    // Use it once — should succeed
    let (status, _) = post_form_with_mailer(
        Arc::clone(&state),
        mailer.clone() as Arc<dyn common::mailer::MailSender>,
        "/api/confirm_password_reset",
        body.clone(),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK);

    // Use it again — should fail
    let (status, _) = post_form_with_mailer(
        Arc::clone(&state),
        mailer.clone() as Arc<dyn common::mailer::MailSender>,
        "/api/confirm_password_reset",
        body,
        None,
    )
    .await;
    assert_ne!(status, StatusCode::OK);
}

// A too-short `new_password` is rejected at the wire (the `ProfferedPassword` arg
// fails to deserialize via `validate_password_shape`) before the reset is applied —
// the parallel of `register_short_password_returns_error`, for the reset endpoint.
#[apply(backends)]
#[tokio::test]
async fn confirm_password_reset_with_short_password_returns_error(#[case] backend: Backend) {
    let TestEnv { state, base: _base } = backend.setup().await;
    let mailer = Arc::new(CapturingMailSender::new());

    let user_id = create_user_with_verified_email(&state, "frank", "frank@example.com")
        .await
        .user_id;

    let expires_at = Utc::now() + chrono::Duration::hours(1);
    let raw_token = state
        .password_resets
        .create_password_reset(user_id, expires_at)
        .await
        .unwrap();

    let body = format!("token={raw_token}&new_password=short");
    let (status, _body) = post_form_with_mailer(
        Arc::clone(&state),
        mailer.clone() as Arc<dyn common::mailer::MailSender>,
        "/api/confirm_password_reset",
        body,
        None,
    )
    .await;

    assert_ne!(status, StatusCode::OK);

    // The reset must not have been applied: the original password still authenticates.
    let auth = state
        .users
        .authenticate(
            &"frank".parse::<Username>().unwrap(),
            &"password123".parse().unwrap(),
        )
        .await;
    assert!(
        auth.is_ok(),
        "a too-short new password must be rejected without applying the reset"
    );
}
