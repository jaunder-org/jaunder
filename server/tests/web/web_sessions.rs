use std::sync::Arc;

use axum::http::StatusCode;

use rstest::*;
use rstest_reuse::*;

use common::MutationOutcome;
use common::test_support::parse_session_label;
use server_fn::ServerFn;

use crate::helpers::{
    TestHttpResponse, create_session_for, create_user_and_session, post_form,
    post_form_with_credentials,
};
use storage::test_support::{Backend, TestEnv, backends};

#[apply(backends)]
#[tokio::test]
async fn list_sessions_returns_sessions_for_authenticated_user(#[case] backend: Backend) {
    let TestEnv { state, base: _base } = backend.setup().await;
    let session = create_user_and_session(&state).await;
    let cookie = session.cookie();
    // Create a second session with a label.
    let sessions = Arc::clone(&state.sessions);
    let label = parse_session_label("mobile");
    let outcome = state
        .write_scope
        .run(|transaction| {
            Box::pin(async move {
                sessions
                    .create_session(transaction, session.user_id, &label)
                    .await
            })
        })
        .await
        .unwrap();
    assert!(matches!(outcome, MutationOutcome::Confirmed(_)));

    let (status, body) = post_form(
        &state,
        <web::sessions::List as ServerFn>::PATH,
        "",
        Some(&cookie),
    )
    .await;

    assert_eq!(status, StatusCode::OK);
    // The body is a JSON array of sessions::Info objects; verify both sessions are present.
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
    let cookie = create_user_and_session(&state).await.cookie();

    let (status, body) = post_form(
        &state,
        <web::sessions::List as ServerFn>::PATH,
        "",
        Some(&cookie),
    )
    .await;

    assert_eq!(status, StatusCode::OK);
    assert!(
        body.contains("\"is_current\":true"),
        "current session should be marked: {body}"
    );
}
#[apply(backends)]
#[tokio::test]
async fn bearer_identity_wins_and_expires_simultaneous_cookie(#[case] backend: Backend) {
    let TestEnv { state, base: _base } = backend.setup().await;
    let cookie_user = create_user_and_session(&state).await;
    let bearer_user = create_user_and_session(&state).await;
    let authorization = format!("Bearer {}", bearer_user.token);

    let response = post_form_with_credentials(
        &state,
        <web::sessions::List as ServerFn>::PATH,
        "",
        Some(&cookie_user.cookie()),
        Some(&authorization),
        true,
    )
    .await;

    assert_eq!(response.status, StatusCode::OK);
    let sessions: Vec<web::sessions::Info> = serde_json::from_str(&response.body).unwrap();
    let current = sessions.iter().find(|session| session.is_current).unwrap();
    assert_eq!(
        current.token_hash,
        host::token::hash(&bearer_user.token).unwrap()
    );
    assert!(
        response.set_cookies.iter().any(|value| {
            value == "session=; HttpOnly; SameSite=Lax; Path=/; Secure; Max-Age=0"
        })
    );
}

#[apply(backends)]
#[tokio::test]
async fn bearer_matching_cookie_still_expires_cookie(#[case] backend: Backend) {
    let TestEnv { state, base: _base } = backend.setup().await;
    let session = create_user_and_session(&state).await;
    let authorization = format!("Bearer {}", session.token);

    let response = post_form_with_credentials(
        &state,
        <web::sessions::List as ServerFn>::PATH,
        "",
        Some(&session.cookie()),
        Some(&authorization),
        true,
    )
    .await;

    assert_eq!(response.status, StatusCode::OK);
    assert_eq!(
        response.set_cookies,
        ["session=; HttpOnly; SameSite=Lax; Path=/; Secure; Max-Age=0"]
    );
}

#[apply(backends)]
#[tokio::test]
async fn explicit_auth_failures_do_not_expire_valid_cookie(#[case] backend: Backend) {
    let TestEnv { state, base: _base } = backend.setup().await;
    let cookie = create_user_and_session(&state).await.cookie();

    for authorization in [
        "Bearer unknown-token",
        "Bearer has space",
        "Negotiate unsupported",
    ] {
        let TestHttpResponse {
            status,
            set_cookies,
            ..
        } = post_form_with_credentials(
            &state,
            <web::sessions::List as ServerFn>::PATH,
            "",
            Some(&cookie),
            Some(authorization),
            true,
        )
        .await;

        assert_ne!(status, StatusCode::OK, "{authorization}");
        assert!(set_cookies.is_empty(), "{authorization}: {set_cookies:?}");
    }
}

#[apply(backends)]
#[tokio::test]
async fn bearer_only_success_does_not_emit_cookie_expiry(#[case] backend: Backend) {
    let TestEnv { state, base: _base } = backend.setup().await;
    let session = create_user_and_session(&state).await;
    let authorization = format!("Bearer {}", session.token);

    let response = post_form_with_credentials(
        &state,
        <web::sessions::List as ServerFn>::PATH,
        "",
        None,
        Some(&authorization),
        true,
    )
    .await;

    assert_eq!(response.status, StatusCode::OK);
    assert!(response.set_cookies.is_empty());
}

#[apply(backends)]
#[tokio::test]
async fn list_sessions_requires_authentication(#[case] backend: Backend) {
    let TestEnv { state, base: _base } = backend.setup().await;

    let (status, _) = post_form(&state, <web::sessions::List as ServerFn>::PATH, "", None).await;

    assert_eq!(status, StatusCode::INTERNAL_SERVER_ERROR);
}

#[apply(backends)]
#[tokio::test]
async fn revoke_session_removes_session_for_authenticated_user(#[case] backend: Backend) {
    let TestEnv { state, base: _base } = backend.setup().await;
    let session = create_user_and_session(&state).await;
    let cookie1 = session.cookie();
    // Create a second session to revoke.
    let raw_token2 = create_session_for(&state, session.user_id).await.token;
    let sessions = Arc::clone(&state.sessions);
    let outcome = state
        .write_scope
        .run(|transaction| {
            Box::pin(async move { sessions.authenticate(transaction, &raw_token2).await })
        })
        .await
        .unwrap();
    let token_hash2 = match outcome {
        MutationOutcome::Confirmed(record) => record.token_hash,
        MutationOutcome::CommitIndeterminate(_) => {
            panic!("session authentication requires a confirmed commit")
        }
    };

    let body = format!("token_hash={token_hash2}");
    let (status, _) = post_form(
        &state,
        <web::sessions::Revoke as ServerFn>::PATH,
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
    let sessions = Arc::clone(&state.sessions);
    let token = session.token.clone();
    let outcome = state
        .write_scope
        .run(|transaction| {
            Box::pin(async move { sessions.authenticate(transaction, &token).await })
        })
        .await
        .unwrap();
    assert!(
        matches!(outcome, MutationOutcome::Confirmed(_)),
        "requesting session should still be valid"
    );
}

#[apply(backends)]
#[tokio::test]
async fn revoke_session_rejects_session_belonging_to_another_user(#[case] backend: Backend) {
    let TestEnv { state, base: _base } = backend.setup().await;
    let alice_cookie = create_user_and_session(&state).await.cookie();
    let bob = create_user_and_session(&state).await;
    let sessions = Arc::clone(&state.sessions);
    let token = bob.token.clone();
    let outcome = state
        .write_scope
        .run(|transaction| {
            Box::pin(async move { sessions.authenticate(transaction, &token).await })
        })
        .await
        .unwrap();
    let bob_token_hash = match outcome {
        MutationOutcome::Confirmed(record) => record.token_hash,
        MutationOutcome::CommitIndeterminate(_) => {
            panic!("session authentication requires a confirmed commit")
        }
    };

    // Alice tries to revoke Bob's session.
    let body = format!("token_hash={bob_token_hash}");
    let (status, _) = post_form(
        &state,
        <web::sessions::Revoke as ServerFn>::PATH,
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
        &state,
        <web::sessions::Revoke as ServerFn>::PATH,
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
    let session = create_user_and_session(&state).await;
    let cookie = session.cookie();

    let (status, body) = post_form(
        &state,
        <web::sessions::CreateAppPassword as ServerFn>::PATH,
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
    let cookie = create_user_and_session(&state).await.cookie();

    let (status, _body) = post_form(
        &state,
        <web::sessions::CreateAppPassword as ServerFn>::PATH,
        "label=%20%20",
        Some(&cookie),
    )
    .await;

    // A blank/whitespace label is rejected at the typed-wire-arg decode
    // (SessionLabel's FromStr), not a server-side check; it surfaces as 500 (the
    // session-fn convention).
    assert_eq!(status, StatusCode::INTERNAL_SERVER_ERROR);
}

#[apply(backends)]
#[tokio::test]
async fn create_app_password_rejects_overlong_label(#[case] backend: Backend) {
    let TestEnv { state, base: _base } = backend.setup().await;
    let cookie = create_user_and_session(&state).await.cookie();

    // A label past MAX_SESSION_LABEL_CHARS (255) is rejected at the SessionLabel
    // decode — coverage the cap makes possible.
    let overlong = "a".repeat(256);
    let (status, _body) = post_form(
        &state,
        <web::sessions::CreateAppPassword as ServerFn>::PATH,
        format!("label={overlong}"),
        Some(&cookie),
    )
    .await;

    assert_eq!(status, StatusCode::INTERNAL_SERVER_ERROR);
}

#[apply(backends)]
#[tokio::test]
async fn create_app_password_requires_authentication(#[case] backend: Backend) {
    let TestEnv { state, base: _base } = backend.setup().await;

    let (status, _body) = post_form(
        &state,
        <web::sessions::CreateAppPassword as ServerFn>::PATH,
        "label=MarsEdit",
        None,
    )
    .await;

    assert_eq!(status, StatusCode::INTERNAL_SERVER_ERROR);
}
