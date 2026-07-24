use std::sync::Arc;

use axum::http::StatusCode;

use rstest::*;
use rstest_reuse::*;

use crate::helpers::{create_user_and_session, post_form};
use storage::test_support::{backends, Backend, TestEnv};

#[apply(backends)]
#[tokio::test]
async fn list_sessions_returns_sessions_for_authenticated_user(#[case] backend: Backend) {
    let TestEnv { state, base: _base } = backend.setup().await;
    let session = create_user_and_session(&state, "alice").await;
    let cookie = session.cookie();
    // Create a second session with a label.
    state
        .sessions
        .create_session(session.user_id, "mobile")
        .await
        .unwrap();

    let (status, body) =
        post_form(Arc::clone(&state), "/api/list_sessions", "", Some(&cookie)).await;

    assert_eq!(status, StatusCode::OK);
    // The body is a JSON array of SessionInfo objects; verify both sessions are present.
    // Count occurrences of "token_hash" to confirm both sessions are returned.
    let session_count = body.matches("\"token_hash\"").count();
    assert_eq!(session_count, 2, "expected 2 sessions, body: {body}");
    assert!(
        body.contains("mobile"),
        "label should appear in body: {body}"
    );
}

#[apply(backends)]
#[tokio::test]
async fn list_sessions_marks_current_session(#[case] backend: Backend) {
    let TestEnv { state, base: _base } = backend.setup().await;
    let cookie = create_user_and_session(&state, "alice").await.cookie();

    let (status, body) =
        post_form(Arc::clone(&state), "/api/list_sessions", "", Some(&cookie)).await;

    assert_eq!(status, StatusCode::OK);
    assert!(
        body.contains("\"is_current\":true"),
        "current session should be marked: {body}"
    );
}

#[apply(backends)]
#[tokio::test]
async fn list_sessions_requires_authentication(#[case] backend: Backend) {
    let TestEnv { state, base: _base } = backend.setup().await;

    let (status, _) = post_form(Arc::clone(&state), "/api/list_sessions", "", None).await;

    assert_eq!(status, StatusCode::INTERNAL_SERVER_ERROR);
}

#[apply(backends)]
#[tokio::test]
async fn revoke_session_removes_session_for_authenticated_user(#[case] backend: Backend) {
    let TestEnv { state, base: _base } = backend.setup().await;
    let session = create_user_and_session(&state, "alice").await;
    let cookie1 = session.cookie();
    // Create a second session to revoke.
    let raw_token2 = state
        .sessions
        .create_session(session.user_id, "test session")
        .await
        .unwrap();
    let token_hash2 = state
        .sessions
        .authenticate(&raw_token2)
        .await
        .unwrap()
        .token_hash;

    let body = format!("token_hash={token_hash2}");
    let (status, _) = post_form(
        Arc::clone(&state),
        "/api/revoke_session",
        body,
        Some(&cookie1),
    )
    .await;

    assert_eq!(status, StatusCode::OK);

    // Verify the revoked session is gone but the requester's session remains.
    let sessions = state.sessions.list_sessions(session.user_id).await.unwrap();
    assert_eq!(sessions.len(), 1, "only one session should remain");
    assert!(
        !sessions.iter().any(|s| s.token_hash == token_hash2),
        "revoked session should not appear"
    );
    // The requesting session should still be valid.
    assert!(
        state.sessions.authenticate(&session.token).await.is_ok(),
        "requesting session should still be valid"
    );
}

#[apply(backends)]
#[tokio::test]
async fn revoke_session_rejects_session_belonging_to_another_user(#[case] backend: Backend) {
    let TestEnv { state, base: _base } = backend.setup().await;
    let alice_cookie = create_user_and_session(&state, "alice").await.cookie();
    let bob = create_user_and_session(&state, "bob").await;
    let bob_token_hash = state
        .sessions
        .authenticate(&bob.token)
        .await
        .unwrap()
        .token_hash;

    // Alice tries to revoke Bob's session.
    let body = format!("token_hash={bob_token_hash}");
    let (status, _) = post_form(
        Arc::clone(&state),
        "/api/revoke_session",
        body,
        Some(&alice_cookie),
    )
    .await;

    assert_eq!(status, StatusCode::INTERNAL_SERVER_ERROR);

    // Bob's session should still exist.
    let bob_sessions = state.sessions.list_sessions(bob.user_id).await.unwrap();
    assert!(
        !bob_sessions.is_empty(),
        "Bob's session should not be revoked"
    );
}

#[apply(backends)]
#[tokio::test]
async fn revoke_session_requires_authentication(#[case] backend: Backend) {
    let TestEnv { state, base: _base } = backend.setup().await;

    let (status, _) = post_form(
        Arc::clone(&state),
        "/api/revoke_session",
        "token_hash=somehash",
        None,
    )
    .await;

    assert_eq!(status, StatusCode::INTERNAL_SERVER_ERROR);
}

#[apply(backends)]
#[tokio::test]
async fn create_app_password_mints_labelled_session(#[case] backend: Backend) {
    let TestEnv { state, base: _base } = backend.setup().await;
    let session = create_user_and_session(&state, "alice").await;
    let cookie = session.cookie();

    let (status, body) = post_form(
        Arc::clone(&state),
        "/api/create_app_password",
        "label=MarsEdit",
        Some(&cookie),
    )
    .await;

    assert_eq!(status, StatusCode::OK);
    assert!(body.contains("\"token\""), "token missing: {body}");
    assert!(body.contains("MarsEdit"), "label missing: {body}");

    // The new app password appears as a session with its label.
    let sessions = state.sessions.list_sessions(session.user_id).await.unwrap();
    assert!(sessions.iter().any(|s| s.label == "MarsEdit"));
}

#[apply(backends)]
#[tokio::test]
async fn create_app_password_rejects_blank_label(#[case] backend: Backend) {
    let TestEnv { state, base: _base } = backend.setup().await;
    let cookie = create_user_and_session(&state, "alice").await.cookie();

    let (status, _body) = post_form(
        Arc::clone(&state),
        "/api/create_app_password",
        "label=%20%20",
        Some(&cookie),
    )
    .await;

    // Server-fn errors surface as 500 (the existing session-fn convention).
    assert_eq!(status, StatusCode::INTERNAL_SERVER_ERROR);
}

#[apply(backends)]
#[tokio::test]
async fn create_app_password_requires_authentication(#[case] backend: Backend) {
    let TestEnv { state, base: _base } = backend.setup().await;

    let (status, _body) = post_form(
        Arc::clone(&state),
        "/api/create_app_password",
        "label=MarsEdit",
        None,
    )
    .await;

    assert_eq!(status, StatusCode::INTERNAL_SERVER_ERROR);
}
