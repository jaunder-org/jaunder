use std::sync::{Arc, OnceLock};

use axum::{
    body::Body,
    http::{header, Request, StatusCode},
};
use chrono::Utc;
use common::mailer::{test_utils::CapturingMailSender, MailSender};
use jaunder::storage::{
    AppState, SqliteAtomicOps, SqliteEmailVerificationStorage, SqliteInviteStorage,
    SqlitePasswordResetStorage, SqliteSessionStorage, SqliteSiteConfigStorage, SqliteUserStorage,
};
use jaunder::username::Username;
use leptos::prelude::LeptosOptions;
use sqlx::SqlitePool;
use tempfile::TempDir;
use tower::ServiceExt;

fn ensure_server_fns_registered() {
    static ONCE: OnceLock<()> = OnceLock::new();
    ONCE.get_or_init(|| {
        server_fn::axum::register_explicit::<web::password_reset::RequestPasswordReset>();
        server_fn::axum::register_explicit::<web::password_reset::ConfirmPasswordReset>();
    });
}

async fn open_pool(base: &TempDir) -> SqlitePool {
    let opts: sqlx::sqlite::SqliteConnectOptions =
        format!("sqlite:{}", base.path().join("test.db").display())
            .parse()
            .unwrap();
    let pool = SqlitePool::connect_with(opts.create_if_missing(true))
        .await
        .unwrap();
    sqlx::migrate!("./migrations").run(&pool).await.unwrap();
    pool
}

async fn test_state_with_mailer(base: &TempDir) -> (Arc<AppState>, Arc<CapturingMailSender>) {
    let pool = open_pool(base).await;
    let mailer = Arc::new(CapturingMailSender::new());
    let state = Arc::new(AppState {
        site_config: Arc::new(SqliteSiteConfigStorage::new(pool.clone())),
        users: Arc::new(SqliteUserStorage::new(pool.clone())),
        sessions: Arc::new(SqliteSessionStorage::new(pool.clone())),
        invites: Arc::new(SqliteInviteStorage::new(pool.clone())),
        atomic: Arc::new(SqliteAtomicOps::new(pool.clone())),
        email_verifications: Arc::new(SqliteEmailVerificationStorage::new(pool.clone())),
        password_resets: Arc::new(SqlitePasswordResetStorage::new(pool)),
        mailer: Arc::clone(&mailer) as Arc<dyn MailSender>,
    });
    (state, mailer)
}

fn test_options() -> LeptosOptions {
    LeptosOptions::builder().output_name("test").build()
}

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

/// Creates a user with a verified email address. Returns (user_id, raw_session_token).
async fn create_user_with_verified_email(
    state: &Arc<AppState>,
    username: &str,
    email: &str,
) -> (i64, String) {
    let user_id = state
        .users
        .create_user(
            &username.parse::<Username>().unwrap(),
            &"password123".parse().unwrap(),
            None,
        )
        .await
        .unwrap();
    let email_addr: email_address::EmailAddress = email.parse().unwrap();
    state
        .users
        .set_email(user_id, Some(&email_addr), true)
        .await
        .unwrap();
    let raw_token = state.sessions.create_session(user_id, None).await.unwrap();
    (user_id, raw_token)
}

// M3.11.7: request_password_reset for a user with a verified email sends a reset email.
#[tokio::test]
async fn request_password_reset_sends_email_for_verified_user() {
    let base = TempDir::new().unwrap();
    let (state, mailer) = test_state_with_mailer(&base).await;

    create_user_with_verified_email(&state, "alice", "alice@example.com").await;

    let (status, _body) = post_form(
        Arc::clone(&state),
        "/api/request_password_reset",
        "username=alice",
        None,
    )
    .await;

    assert_eq!(status, StatusCode::OK);

    let sent = mailer.sent();
    assert_eq!(sent.len(), 1, "expected one reset email to be sent");
    assert_eq!(sent[0].to.len(), 1);
    assert_eq!(sent[0].to[0].as_str(), "alice@example.com");
    assert!(
        sent[0].body_text.contains("/reset-password?token="),
        "email body should contain reset link, got: {}",
        sent[0].body_text
    );
}

// M3.11.8: request_password_reset for a user without a verified email returns an error.
#[tokio::test]
async fn request_password_reset_returns_error_for_user_without_verified_email() {
    let base = TempDir::new().unwrap();
    let (state, _mailer) = test_state_with_mailer(&base).await;

    state
        .users
        .create_user(
            &"bob".parse::<Username>().unwrap(),
            &"password123".parse().unwrap(),
            None,
        )
        .await
        .unwrap();

    let (status, _body) = post_form(
        Arc::clone(&state),
        "/api/request_password_reset",
        "username=bob",
        None,
    )
    .await;

    assert_ne!(status, StatusCode::OK);
}

// M3.11.9: request_password_reset for an unknown username returns an error.
#[tokio::test]
async fn request_password_reset_returns_error_for_unknown_username() {
    let base = TempDir::new().unwrap();
    let (state, _mailer) = test_state_with_mailer(&base).await;

    let (status, _body) = post_form(
        Arc::clone(&state),
        "/api/request_password_reset",
        "username=nobody",
        None,
    )
    .await;

    assert_ne!(status, StatusCode::OK);
}

// M3.11.10: confirm_password_reset with a valid token sets the new password and revokes sessions.
#[tokio::test]
async fn confirm_password_reset_sets_password_and_revokes_sessions() {
    let base = TempDir::new().unwrap();
    let (state, _mailer) = test_state_with_mailer(&base).await;

    let (user_id, _session) =
        create_user_with_verified_email(&state, "carol", "carol@example.com").await;
    // Create a second session to ensure all are revoked
    state.sessions.create_session(user_id, None).await.unwrap();

    let expires_at = Utc::now() + chrono::Duration::hours(1);
    let raw_token = state
        .password_resets
        .create_password_reset(user_id, expires_at)
        .await
        .unwrap();

    let body = format!("token={raw_token}&new_password=newpassword456");
    let (status, _body) = post_form(
        Arc::clone(&state),
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
#[tokio::test]
async fn confirm_password_reset_with_expired_token_returns_error() {
    let base = TempDir::new().unwrap();
    let (state, _mailer) = test_state_with_mailer(&base).await;

    let (user_id, _) = create_user_with_verified_email(&state, "dave", "dave@example.com").await;

    let expires_at = Utc::now() - chrono::Duration::hours(1);
    let raw_token = state
        .password_resets
        .create_password_reset(user_id, expires_at)
        .await
        .unwrap();

    let body = format!("token={raw_token}&new_password=newpassword456");
    let (status, _body) = post_form(
        Arc::clone(&state),
        "/api/confirm_password_reset",
        body,
        None,
    )
    .await;

    assert_ne!(status, StatusCode::OK);
}

// M3.11.12: confirm_password_reset with an invalid token returns an error.
#[tokio::test]
async fn confirm_password_reset_with_invalid_token_returns_error() {
    let base = TempDir::new().unwrap();
    let (state, _mailer) = test_state_with_mailer(&base).await;

    let (status, _body) = post_form(
        Arc::clone(&state),
        "/api/confirm_password_reset",
        "token=not-a-real-token&new_password=newpassword456",
        None,
    )
    .await;

    assert_ne!(status, StatusCode::OK);
}

// M3.11.13: confirm_password_reset with an already-used token returns an error.
#[tokio::test]
async fn confirm_password_reset_with_used_token_returns_error() {
    let base = TempDir::new().unwrap();
    let (state, _mailer) = test_state_with_mailer(&base).await;

    let (user_id, _) = create_user_with_verified_email(&state, "eve", "eve@example.com").await;

    let expires_at = Utc::now() + chrono::Duration::hours(1);
    let raw_token = state
        .password_resets
        .create_password_reset(user_id, expires_at)
        .await
        .unwrap();

    let body = format!("token={raw_token}&new_password=newpassword456");

    // Use it once — should succeed
    let (status, _) = post_form(
        Arc::clone(&state),
        "/api/confirm_password_reset",
        body.clone(),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK);

    // Use it again — should fail
    let (status, _) = post_form(
        Arc::clone(&state),
        "/api/confirm_password_reset",
        body,
        None,
    )
    .await;
    assert_ne!(status, StatusCode::OK);
}
