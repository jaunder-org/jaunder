use std::sync::Arc;

use axum::http::StatusCode;
use common::MutationOutcome;
use common::mailer::test_utils::CapturingMailSender;
use common::test_support::parse_email;
use common::time::UtcInstant;
use server_fn::ServerFn;
use storage::AppState;

use crate::helpers::{
    SeededSession, assert_no_email, assert_one_absolute_link_email, create_session_for,
    create_user_and_session, post_form_with_mailer, post_server_fn_request_fixture_with_mailer,
    post_server_fn_with_mailer, setup_with_base_url,
};
use storage::test_support::{Backend, SeedUser, TestEnv, backends};

#[derive(serde::Serialize)]
struct ConfirmPasswordResetDecodeFixture<'a> {
    token: &'a str,
    new_password: &'a str,
}

use rstest::*;
use rstest_reuse::*;

/// Creates a user with a verified email address and an authenticated session.
async fn create_user_with_verified_email(state: &Arc<AppState>, email: &str) -> SeededSession {
    let session = create_user_and_session(state).await;
    let email = parse_email(email);
    let users = Arc::clone(&state.users);
    let outcome = state
        .write_scope
        .run(|transaction| {
            Box::pin(async move {
                users
                    .set_email(transaction, session.user_id, Some(&email), true)
                    .await
            })
        })
        .await
        .expect("set verified email");
    assert!(matches!(outcome, MutationOutcome::Confirmed(())));
    session
}

// M3.11.7: request_password_reset for a user with a verified email sends a reset email.
#[apply(backends)]
#[tokio::test]
async fn request_password_reset_sends_email_for_verified_user(#[case] backend: Backend) {
    // The reset email composes an absolute link, so the flow requires a seeded
    // `site.base_url` (canonicalized to `https://example.com/`).
    let TestEnv { state, base: _base } = setup_with_base_url(backend).await;
    let mailer = Arc::new(CapturingMailSender::new());

    let session = create_user_with_verified_email(&state, "alice@example.com").await;

    let (status, _body) = post_form_with_mailer(
        &state,
        &mailer,
        <web::password_reset::Request as ServerFn>::PATH,
        format!("username={}", session.username),
        None,
    )
    .await;

    assert_eq!(status, StatusCode::OK);
    assert_one_absolute_link_email(&mailer, "alice@example.com", "/reset-password");
}

// The reset email composes an absolute link, so an eligible request still
// fails (after confirming the user) without a seeded `site.base_url`, rather than
// emailing a dead relative link.
#[apply(backends)]
#[tokio::test]
async fn request_password_reset_without_base_url_returns_error(#[case] backend: Backend) {
    let TestEnv { state, base: _base } = backend.setup().await; // no base_url seeded
    let mailer = Arc::new(CapturingMailSender::new());

    let session = create_user_with_verified_email(&state, "alice@example.com").await;

    let (status, _body) = post_form_with_mailer(
        &state,
        &mailer,
        <web::password_reset::Request as ServerFn>::PATH,
        format!("username={}", session.username),
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

    let user = SeedUser::new().seed(&state).await;

    let (status, _body) = post_form_with_mailer(
        &state,
        &mailer,
        <web::password_reset::Request as ServerFn>::PATH,
        format!("username={}", user.username),
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
        &state,
        &mailer,
        <web::password_reset::Request as ServerFn>::PATH,
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
        &state,
        &mailer,
        <web::password_reset::Request as ServerFn>::PATH,
        "username=nobody",
        None,
    )
    .await;

    assert_ne!(status, StatusCode::OK);
}

// M3.11.10: the nested request maps its token and password exactly, applies the
// password, consumes the token, and revokes every existing session.
#[apply(backends)]
#[tokio::test]
async fn confirm_nested_request_maps_token_and_password(#[case] backend: Backend) {
    let TestEnv { state, base: _base } = backend.setup().await;
    let mailer = Arc::new(CapturingMailSender::new());

    let session = create_user_with_verified_email(&state, "carol@example.com").await;
    let user_id = session.user_id;
    // Create a second session to ensure all are revoked
    create_session_for(&state, user_id).await;

    let expires_at: UtcInstant = "2099-01-02T03:04:05.123456Z".parse().unwrap();
    let password_resets = Arc::clone(&state.password_resets);
    let outcome = state
        .write_scope
        .run(|transaction| {
            Box::pin(async move {
                password_resets
                    .create_password_reset(transaction, user_id, expires_at)
                    .await
            })
        })
        .await
        .unwrap();
    let raw_token = storage::test_support::confirmed_for(outcome, "password-reset fixture setup");

    let (status, _body) = post_server_fn_with_mailer(
        &state,
        &mailer,
        &web::password_reset::Confirm {
            request: web::password_reset::ConfirmPasswordResetRequest {
                token: raw_token,
                new_password: "newpassword456".parse().unwrap(),
            },
        },
        None,
    )
    .await;

    assert_eq!(status, StatusCode::OK);

    // Old password should fail authentication
    let users = Arc::clone(&state.users);
    let username = session.username.clone();
    let password = "password123".parse().unwrap();
    let old_auth = users.prepare_authentication(&username, &password).await;
    assert!(old_auth.is_err(), "old password should no longer work");

    // New password should succeed
    let users = Arc::clone(&state.users);
    let username = session.username.clone();
    let password = "newpassword456".parse().unwrap();
    let authentication = users
        .prepare_authentication(&username, &password)
        .await
        .unwrap();
    let outcome = state
        .write_scope
        .run(|transaction| {
            Box::pin(async move { users.authenticate(transaction, authentication).await })
        })
        .await
        .unwrap();
    assert!(matches!(outcome, MutationOutcome::Confirmed(_)));

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

    let user_id = create_user_with_verified_email(&state, "dave@example.com")
        .await
        .user_id;

    let expires_at: UtcInstant = "2000-01-02T03:04:05.123456Z".parse().unwrap();
    let password_resets = Arc::clone(&state.password_resets);
    let outcome = state
        .write_scope
        .run(|transaction| {
            Box::pin(async move {
                password_resets
                    .create_password_reset(transaction, user_id, expires_at)
                    .await
            })
        })
        .await
        .unwrap();
    let raw_token = storage::test_support::confirmed_for(outcome, "password-reset fixture setup");

    let (status, response_body) = post_server_fn_with_mailer(
        &state,
        &mailer,
        &web::password_reset::Confirm {
            request: web::password_reset::ConfirmPasswordResetRequest {
                token: raw_token,
                new_password: "newpassword456".parse().unwrap(),
            },
        },
        None,
    )
    .await;

    assert_ne!(status, StatusCode::OK);
    assert!(
        response_body.contains("\"validation\""),
        "expected a validation-class password-reset error; body: {response_body}"
    );
}

// M3.11.12: confirm_password_reset with an invalid token returns an error.
#[apply(backends)]
#[tokio::test]
async fn confirm_password_reset_with_invalid_token_returns_error(#[case] backend: Backend) {
    let TestEnv { state, base: _base } = backend.setup().await;
    let mailer = Arc::new(CapturingMailSender::new());

    let (status, response_body) =
        post_server_fn_request_fixture_with_mailer::<web::password_reset::Confirm, _, _>(
            &state,
            &mailer,
            &ConfirmPasswordResetDecodeFixture {
                token: "not-a-real-token",
                new_password: "newpassword456",
            },
            None,
        )
        .await;

    assert_ne!(status, StatusCode::OK);
    assert!(
        response_body.contains("\"validation\""),
        "expected a validation-class password-reset error; body: {response_body}"
    );
}

#[apply(backends)]
#[tokio::test]
async fn confirm_nested_request_rejects_malformed_token_before_handler(#[case] backend: Backend) {
    let TestEnv { state, base: _base } = backend.setup().await;
    let mailer = Arc::new(CapturingMailSender::new());
    let session = create_user_with_verified_email(&state, "malformed@example.com").await;

    // `bad!token` is outside base64url, so `RawToken` rejects it (at wire-decode once
    // `token` is typed). `new_password` is valid-length, so the failure isolates to the
    // token.
    let (status, response_body) =
        post_server_fn_request_fixture_with_mailer::<web::password_reset::Confirm, _, _>(
            &state,
            &mailer,
            &ConfirmPasswordResetDecodeFixture {
                token: "bad!token",
                new_password: "newpassword456",
            },
            None,
        )
        .await;

    assert_eq!(status, StatusCode::INTERNAL_SERVER_ERROR);
    assert!(
        response_body.contains("server_function"),
        "expected a server-fn decode rejection; body: {response_body}"
    );
    let users = Arc::clone(&state.users);
    let username = session.username.clone();
    let password = "password123".parse().unwrap();
    let authentication = users
        .prepare_authentication(&username, &password)
        .await
        .unwrap();
    let outcome = state
        .write_scope
        .run(|transaction| {
            Box::pin(async move { users.authenticate(transaction, authentication).await })
        })
        .await
        .unwrap();
    assert!(
        matches!(outcome, MutationOutcome::Confirmed(_)),
        "a malformed token must not change the password"
    );
    assert_eq!(
        state
            .sessions
            .list_sessions(session.user_id)
            .await
            .unwrap()
            .len(),
        1,
        "a malformed token must not revoke sessions"
    );
}

// M3.11.13: confirm_password_reset with an already-used token returns an error.
#[apply(backends)]
#[tokio::test]
async fn confirm_password_reset_with_used_token_returns_error(#[case] backend: Backend) {
    let TestEnv { state, base: _base } = backend.setup().await;
    let mailer = Arc::new(CapturingMailSender::new());

    let user_id = create_user_with_verified_email(&state, "eve@example.com")
        .await
        .user_id;

    let expires_at: UtcInstant = "2099-01-02T03:04:05.123456Z".parse().unwrap();
    let password_resets = Arc::clone(&state.password_resets);
    let outcome = state
        .write_scope
        .run(|transaction| {
            Box::pin(async move {
                password_resets
                    .create_password_reset(transaction, user_id, expires_at)
                    .await
            })
        })
        .await
        .unwrap();
    let raw_token = storage::test_support::confirmed_for(outcome, "password-reset fixture setup");

    let request = web::password_reset::Confirm {
        request: web::password_reset::ConfirmPasswordResetRequest {
            token: raw_token,
            new_password: "newpassword456".parse().unwrap(),
        },
    };

    // Use it once — should succeed
    let (status, _) = post_server_fn_with_mailer(&state, &mailer, &request, None).await;
    assert_eq!(status, StatusCode::OK);

    // Use it again — should fail
    let (status, response_body) = post_server_fn_with_mailer(&state, &mailer, &request, None).await;
    assert_ne!(status, StatusCode::OK);
    assert!(
        response_body.contains("\"validation\""),
        "expected a validation-class password-reset error; body: {response_body}"
    );
}

// A too-short `new_password` is rejected while decoding the nested request before
// the reset is applied.
#[apply(backends)]
#[tokio::test]
async fn confirm_nested_request_rejects_short_password_before_handler(#[case] backend: Backend) {
    let TestEnv { state, base: _base } = backend.setup().await;
    let mailer = Arc::new(CapturingMailSender::new());

    let session = create_user_with_verified_email(&state, "frank@example.com").await;

    let password_resets = Arc::clone(&state.password_resets);
    let expires_at: UtcInstant = "2099-01-02T03:04:05.123456Z".parse().unwrap();
    let outcome = state
        .write_scope
        .run(|transaction| {
            Box::pin(async move {
                password_resets
                    .create_password_reset(transaction, session.user_id, expires_at)
                    .await
            })
        })
        .await
        .unwrap();
    let raw_token = storage::test_support::confirmed_for(outcome, "password-reset fixture setup");

    let (status, response_body) =
        post_server_fn_request_fixture_with_mailer::<web::password_reset::Confirm, _, _>(
            &state,
            &mailer,
            &ConfirmPasswordResetDecodeFixture {
                token: raw_token.as_ref(),
                new_password: "short",
            },
            None,
        )
        .await;

    // A decode rejection is HTTP 500 with a body tagged `server_function` — distinct
    // from an in-body failure, which projects to `validation`/`unauthorized`/etc.
    // (`WebError` is externally tagged, snake_case.) This is the wire contract the
    // decode-telemetry path in `web::error` sits behind (#822).
    assert_eq!(status, StatusCode::INTERNAL_SERVER_ERROR);
    assert!(
        response_body.contains("server_function"),
        "expected a server-fn decode rejection; body: {response_body}"
    );

    // The reset must not have been applied: the original password still authenticates.
    let users = Arc::clone(&state.users);
    let username = session.username.clone();
    let password = "password123".parse().unwrap();
    let authentication = users
        .prepare_authentication(&username, &password)
        .await
        .unwrap();
    let outcome = state
        .write_scope
        .run(|transaction| {
            Box::pin(async move { users.authenticate(transaction, authentication).await })
        })
        .await
        .unwrap();
    assert!(
        matches!(outcome, MutationOutcome::Confirmed(_)),
        "a too-short new password must be rejected without applying the reset"
    );
    assert_eq!(
        state
            .sessions
            .list_sessions(session.user_id)
            .await
            .unwrap()
            .len(),
        1,
        "a too-short new password must not revoke sessions"
    );
}
