mod helpers;

use std::sync::Arc;

use axum::{
    body::Body,
    http::{header, Request, StatusCode},
};
use chrono::Utc;
use jaunder::storage::AppState;
use jaunder::username::Username;
use tempfile::TempDir;
use tower::ServiceExt;

use helpers::{ensure_server_fns_registered, test_options, test_state_with_mailer};

async fn post_form(
    state: Arc<AppState>,
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

    let app = jaunder::create_router(test_options(), state, true);
    let response = app.oneshot(request).await.unwrap();
    let status = response.status();
    let bytes = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .unwrap();
    (status, String::from_utf8(bytes.to_vec()).unwrap())
}

// M3.10.7: request_email_verification creates a row and sends an email via CapturingMailSender.
#[tokio::test]
async fn request_email_verification_creates_row_and_sends_email() {
    let base = TempDir::new().unwrap();
    let (state, mailer) = test_state_with_mailer(&base).await;

    let user_id = state
        .users
        .create_user(
            &"alice".parse::<Username>().unwrap(),
            &"password123".parse().unwrap(),
            None,
        )
        .await
        .unwrap();
    let raw_token = state.sessions.create_session(user_id, None).await.unwrap();
    let cookie = format!("session={raw_token}");

    let (status, _body) = post_form(
        Arc::clone(&state),
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
#[tokio::test]
async fn verify_email_with_valid_token_sets_email_verified() {
    let base = TempDir::new().unwrap();
    let (state, _mailer) = test_state_with_mailer(&base).await;

    let user_id = state
        .users
        .create_user(
            &"bob".parse::<Username>().unwrap(),
            &"password123".parse().unwrap(),
            None,
        )
        .await
        .unwrap();

    let expires_at = Utc::now() + chrono::Duration::hours(24);
    let raw_token = state
        .email_verifications
        .create_email_verification(user_id, "bob@example.com", expires_at)
        .await
        .unwrap();

    let (status, _body) = post_form(
        Arc::clone(&state),
        "/api/verify_email",
        format!("token={raw_token}"),
        None,
    )
    .await;

    assert_eq!(status, StatusCode::OK);

    let user = state.users.get_user(user_id).await.unwrap().unwrap();
    assert_eq!(
        user.email.as_ref().map(|e| e.as_str()),
        Some("bob@example.com")
    );
    assert!(user.email_verified, "email should be marked as verified");
}

// M3.10.9: verify_email with an expired token returns an error.
#[tokio::test]
async fn verify_email_with_expired_token_returns_error() {
    let base = TempDir::new().unwrap();
    let (state, _mailer) = test_state_with_mailer(&base).await;

    let user_id = state
        .users
        .create_user(
            &"carol".parse::<Username>().unwrap(),
            &"password123".parse().unwrap(),
            None,
        )
        .await
        .unwrap();

    let expires_at = Utc::now() - chrono::Duration::hours(1);
    let raw_token = state
        .email_verifications
        .create_email_verification(user_id, "carol@example.com", expires_at)
        .await
        .unwrap();

    let (status, _body) = post_form(
        Arc::clone(&state),
        "/api/verify_email",
        format!("token={raw_token}"),
        None,
    )
    .await;

    assert_ne!(status, StatusCode::OK);
}

// M3.10.10: verify_email with an unknown token returns an error.
#[tokio::test]
async fn verify_email_with_unknown_token_returns_error() {
    let base = TempDir::new().unwrap();
    let (state, _mailer) = test_state_with_mailer(&base).await;

    let (status, _body) = post_form(
        Arc::clone(&state),
        "/api/verify_email",
        "token=this_token_does_not_exist",
        None,
    )
    .await;

    assert_ne!(status, StatusCode::OK);
}
