#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::too_many_lines,
    clippy::similar_names,
    clippy::items_after_statements,
    clippy::unused_async
)]
#![allow(unused_macros)]

mod helpers;

use std::sync::Arc;

use axum::{
    body::Body,
    http::{header, Request, StatusCode},
};
use chrono::Utc;
use common::mailer::test_utils::CapturingMailSender;
use common::username::Username;
use storage::AppState;
use tower::ServiceExt;

use helpers::{backends, ensure_server_fns_registered, test_options, Backend, TestEnv};

use rstest::*;
#[allow(clippy::single_component_path_imports)]
use rstest_reuse;
use rstest_reuse::*;

async fn post_form(
    state: Arc<AppState>,
    mailer: Arc<dyn common::mailer::MailSender>,
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
        mailer,
        true,
        helpers::tmp_storage_path(),
    );
    let response = app.oneshot(request).await.unwrap();
    let status = response.status();
    let bytes = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .unwrap();
    (status, String::from_utf8(bytes.to_vec()).unwrap())
}

// M3.10.7: request_email_verification creates a row and sends an email via CapturingMailSender.
#[apply(backends)]
#[tokio::test]
async fn request_email_verification_creates_row_and_sends_email(#[case] backend: Backend) {
    let TestEnv { state, base: _base } = backend.setup().await;
    let mailer = Arc::new(CapturingMailSender::new());

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
    let raw_token = state
        .sessions
        .create_session(user_id, "test session")
        .await
        .unwrap();
    let cookie = format!("session={raw_token}");

    let (status, _body) = post_form(
        Arc::clone(&state),
        mailer.clone() as Arc<dyn common::mailer::MailSender>,
        "/api/request_email_verification",
        "email=alice%40example.com",
        Some(&cookie),
    )
    .await;

    assert_eq!(status, StatusCode::OK);

    let sent = mailer.sent();
    assert_eq!(sent.len(), 1, "expected one email to be sent");
    assert_eq!(sent[0].to.len(), 1);
    assert_eq!(sent[0].to[0].as_str(), "alice@example.com");
    assert!(
        sent[0].body_text.contains("/verify-email?token="),
        "email body should contain verification link, got: {}",
        sent[0].body_text
    );
}

// M3.10.8: verify_email with a valid token sets the email as verified.
#[apply(backends)]
#[tokio::test]
async fn verify_email_with_valid_token_sets_email_verified(#[case] backend: Backend) {
    let TestEnv { state, base: _base } = backend.setup().await;
    let mailer = Arc::new(CapturingMailSender::new());

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

    let expires_at = Utc::now() + chrono::Duration::hours(24);
    let raw_token = state
        .email_verifications
        .create_email_verification(user_id, &"bob@example.com".parse().unwrap(), expires_at)
        .await
        .unwrap();

    let (status, _body) = post_form(
        Arc::clone(&state),
        mailer.clone() as Arc<dyn common::mailer::MailSender>,
        "/api/verify_email",
        format!("token={raw_token}"),
        None,
    )
    .await;

    assert_eq!(status, StatusCode::OK);

    let user = state.users.get_user(user_id).await.unwrap().unwrap();
    assert_eq!(
        user.email.as_ref().map(email_address::EmailAddress::as_str),
        Some("bob@example.com")
    );
    assert!(user.email_verified, "email should be marked as verified");
}

// M3.10.9: verify_email with an expired token returns an error.
#[apply(backends)]
#[tokio::test]
async fn verify_email_with_expired_token_returns_error(#[case] backend: Backend) {
    let TestEnv { state, base: _base } = backend.setup().await;
    let mailer = Arc::new(CapturingMailSender::new());

    let user_id = state
        .users
        .create_user(
            &"carol".parse::<Username>().unwrap(),
            &"password123".parse().unwrap(),
            None,
            false,
        )
        .await
        .unwrap();

    let expires_at = Utc::now() - chrono::Duration::hours(1);
    let raw_token = state
        .email_verifications
        .create_email_verification(user_id, &"carol@example.com".parse().unwrap(), expires_at)
        .await
        .unwrap();

    let (status, _body) = post_form(
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

    let (status, _body) = post_form(
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
async fn request_email_verification_unauthorized_returns_error(#[case] backend: Backend) {
    let TestEnv { state, base: _base } = backend.setup().await;
    let mailer = Arc::new(CapturingMailSender::new());

    // No cookie provided
    let (status, _) = post_form(
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

    // Create user and session
    let username: Username = "alice".parse().unwrap();
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

    let (status, _) = post_form(
        state,
        mailer.clone() as Arc<dyn common::mailer::MailSender>,
        "/api/request_email_verification",
        "email=invalid",
        Some(&cookie_header),
    )
    .await;
    assert_eq!(status, StatusCode::INTERNAL_SERVER_ERROR);
}
