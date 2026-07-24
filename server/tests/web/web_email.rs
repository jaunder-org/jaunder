use std::sync::Arc;

use axum::http::StatusCode;
use chrono::Utc;
use common::mailer::test_utils::CapturingMailSender;
use common::test_support::parse_email;

use crate::helpers::{
    assert_no_email, assert_one_absolute_link_email, create_user_and_session,
    post_form_with_mailer, setup_with_base_url,
};
use storage::test_support::{backends, Backend, SeedUser, TestEnv};

use rstest::*;
use rstest_reuse::*;

// M3.10.7: request_email_verification creates a row and sends an email via CapturingMailSender.
#[apply(backends)]
#[tokio::test]
async fn request_email_verification_creates_row_and_sends_email(#[case] backend: Backend) {
    // The verification email now composes an absolute link, so the flow requires a
    // seeded `site.base_url` (canonicalized to `https://example.com/`).
    let TestEnv { state, base: _base } = setup_with_base_url(backend).await;
    let mailer = Arc::new(CapturingMailSender::new());

    let cookie = create_user_and_session(&state, "alice").await.cookie();

    let (status, _body) = post_form_with_mailer(
        Arc::clone(&state),
        mailer.clone() as Arc<dyn common::mailer::MailSender>,
        "/api/request_email_verification",
        "email=alice%40example.com",
        Some(&cookie),
    )
    .await;

    assert_eq!(status, StatusCode::OK);
    assert_one_absolute_link_email(&mailer, "alice@example.com", "/verify-email");
}

// The verification email now composes an absolute link, so without a seeded
// `site.base_url` the request fails rather than emailing a dead relative link.
#[apply(backends)]
#[tokio::test]
async fn request_email_verification_without_base_url_returns_error(#[case] backend: Backend) {
    let TestEnv { state, base: _base } = backend.setup().await; // no base_url seeded
    let mailer = Arc::new(CapturingMailSender::new());

    let cookie = create_user_and_session(&state, "alice").await.cookie();

    let (status, _body) = post_form_with_mailer(
        Arc::clone(&state),
        mailer.clone() as Arc<dyn common::mailer::MailSender>,
        "/api/request_email_verification",
        "email=alice%40example.com",
        Some(&cookie),
    )
    .await;

    assert_ne!(status, StatusCode::OK, "should fail without a base URL");
    assert_no_email(&mailer);
}

// M3.10.8: verify_email with a valid token sets the email as verified.
#[apply(backends)]
#[tokio::test]
async fn verify_email_with_valid_token_sets_email_verified(#[case] backend: Backend) {
    let TestEnv { state, base: _base } = backend.setup().await;
    let mailer = Arc::new(CapturingMailSender::new());

    let user_id = SeedUser::new("bob").seed(&state).await;

    let email = parse_email("bob@example.com");
    let expires_at = Utc::now() + chrono::Duration::hours(24);
    let raw_token = state
        .email_verifications
        .create_email_verification(user_id, &email, expires_at)
        .await
        .unwrap();

    let (status, _body) = post_form_with_mailer(
        Arc::clone(&state),
        mailer.clone() as Arc<dyn common::mailer::MailSender>,
        "/api/verify_email",
        format!("token={raw_token}"),
        None,
    )
    .await;

    assert_eq!(status, StatusCode::OK);

    let user = state.users.get_user(user_id).await.unwrap().unwrap();
    assert_eq!(user.email, Some(email));
    assert!(user.email_verified, "email should be marked as verified");
}

// M3.10.9: verify_email with an expired token returns an error.
#[apply(backends)]
#[tokio::test]
async fn verify_email_with_expired_token_returns_error(#[case] backend: Backend) {
    let TestEnv { state, base: _base } = backend.setup().await;
    let mailer = Arc::new(CapturingMailSender::new());

    let user_id = SeedUser::new("carol").seed(&state).await;

    let expires_at = Utc::now() - chrono::Duration::hours(1);
    let raw_token = state
        .email_verifications
        .create_email_verification(user_id, &"carol@example.com".parse().unwrap(), expires_at)
        .await
        .unwrap();

    let (status, _body) = post_form_with_mailer(
        Arc::clone(&state),
        mailer.clone() as Arc<dyn common::mailer::MailSender>,
        "/api/verify_email",
        format!("token={raw_token}"),
        None,
    )
    .await;

    assert_ne!(status, StatusCode::OK);
}

// M3.10.10: verify_email with an unknown token returns an error.
#[apply(backends)]
#[tokio::test]
async fn verify_email_with_unknown_token_returns_error(#[case] backend: Backend) {
    let TestEnv { state, base: _base } = backend.setup().await;
    let mailer = Arc::new(CapturingMailSender::new());

    let (status, _body) = post_form_with_mailer(
        Arc::clone(&state),
        mailer.clone() as Arc<dyn common::mailer::MailSender>,
        "/api/verify_email",
        "token=this_token_does_not_exist",
        None,
    )
    .await;

    assert_ne!(status, StatusCode::OK);
}

#[apply(backends)]
#[tokio::test]
async fn verify_email_with_malformed_token_returns_error(#[case] backend: Backend) {
    let TestEnv { state, base: _base } = backend.setup().await;
    let mailer = Arc::new(CapturingMailSender::new());

    // `bad!token` is not valid base64url shape, so `RawToken` rejects it — in-body today,
    // at wire-decode once `token` is typed. Either way a non-OK response.
    let (status, _body) = post_form_with_mailer(
        Arc::clone(&state),
        mailer.clone() as Arc<dyn common::mailer::MailSender>,
        "/api/verify_email",
        "token=bad!token",
        None,
    )
    .await;

    assert_ne!(
        status,
        StatusCode::OK,
        "a malformed verification token must be rejected"
    );
}

#[apply(backends)]
#[tokio::test]
async fn request_email_verification_unauthorized_returns_error(#[case] backend: Backend) {
    let TestEnv { state, base: _base } = backend.setup().await;
    let mailer = Arc::new(CapturingMailSender::new());

    let (status, _) = post_form_with_mailer(
        state,
        mailer.clone() as Arc<dyn common::mailer::MailSender>,
        "/api/request_email_verification",
        "email=alice@example.com",
        None,
    )
    .await;

    // Leptos server functions return 500 for ServerFnError (which require_auth returns).
    assert_eq!(status, StatusCode::INTERNAL_SERVER_ERROR);
}

#[apply(backends)]
#[tokio::test]
async fn request_email_verification_invalid_email_returns_error(#[case] backend: Backend) {
    let TestEnv { state, base: _base } = backend.setup().await;
    let mailer = Arc::new(CapturingMailSender::new());

    let cookie_header = create_user_and_session(&state, "alice").await.cookie();

    let (status, _) = post_form_with_mailer(
        state,
        mailer.clone() as Arc<dyn common::mailer::MailSender>,
        "/api/request_email_verification",
        "email=invalid",
        Some(&cookie_header),
    )
    .await;
    assert_eq!(status, StatusCode::INTERNAL_SERVER_ERROR);
}
