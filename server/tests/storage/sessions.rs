use std::sync::Arc;

use chrono::{Duration, Utc};
use common::MutationOutcome;
use common::test_support::{parse_raw_token, parse_session_label};
use common::time::UtcInstant;
use common::token::TokenHash;
use rstest::*;
use rstest_reuse::*;
use storage::test_support::{Backend, CloseablePool, SeedUser, TestEnv, backends, seed_users};
use storage::{AppState, SessionAuthError, WriteScopeError};

use crate::helpers::create_session_for;
#[apply(backends)]
#[tokio::test]
async fn create_session_then_authenticate_returns_correct_record(#[case] backend: Backend) {
    let env = backend.setup().await;
    let state = &env.state;

    let user = SeedUser::new().seed(state).await;

    let raw_token = create_session(state, user.user_id, parse_session_label("test")).await;
    let record = authenticate(state, raw_token.clone()).await;

    assert_eq!(record.user_id, user.user_id);
    assert_eq!(record.username, user.username);
    assert_eq!(record.label, "test");
    assert!(!record.token_hash.is_empty());
}

#[apply(backends)]
#[tokio::test]
async fn authenticate_returns_session_record_for_valid_token(#[case] backend: Backend) {
    let env = backend.setup().await;
    let state = &env.state;

    let user_id = SeedUser::new().seed(state).await.user_id;

    let raw_token = create_session_for(state, user_id).await.token;
    let record = authenticate(state, raw_token).await;

    assert_eq!(record.user_id, user_id);
}

#[apply(backends)]
#[tokio::test]
async fn fresh_authenticate_returns_the_persisted_last_used_at(#[case] backend: Backend) {
    let TestEnv { state, base } = backend.setup().await;
    let user_id = SeedUser::new().seed(&state).await.user_id;
    let raw_token =
        create_session(state.as_ref(), user_id, parse_session_label("test session")).await;

    let token_hash = host::token::hash(&raw_token).unwrap();
    let first_record = authenticate(state.as_ref(), raw_token.clone()).await;
    let stored = first_record.last_used_at;

    let record = authenticate(state.as_ref(), raw_token).await;
    let persisted_after_auth = load_last_used_at(base.pool(), &token_hash).await;

    assert_eq!(record.last_used_at, stored);
    assert_eq!(persisted_after_auth, stored);
}

#[apply(backends)]
#[tokio::test]
async fn stale_authenticate_refreshes_the_persisted_last_used_at(#[case] backend: Backend) {
    let TestEnv { state, base } = backend.setup().await;
    let user_id = SeedUser::new().seed(&state).await.user_id;
    let raw_token =
        create_session(state.as_ref(), user_id, parse_session_label("test session")).await;

    let token_hash = host::token::hash(&raw_token).unwrap();
    let stale = UtcInstant::from(Utc::now() - Duration::seconds(120));
    set_last_used_at(base.pool(), &token_hash, stale).await;

    let record = authenticate(state.as_ref(), raw_token).await;
    let persisted_after_auth = load_last_used_at(base.pool(), &token_hash).await;
    let freshness_cutoff_after_auth = UtcInstant::from(Utc::now() - Duration::seconds(60));

    assert!(record.last_used_at > stale);
    assert_eq!(record.last_used_at, persisted_after_auth);
    assert!(persisted_after_auth >= freshness_cutoff_after_auth);
}

#[apply(backends)]
#[tokio::test]
async fn revoke_session_then_authenticate_returns_session_not_found(#[case] backend: Backend) {
    let env = backend.setup().await;
    let state = &env.state;

    let user_id = SeedUser::new().seed(state).await.user_id;

    let raw_token = create_session_for(state, user_id).await.token;
    let record = authenticate(state, raw_token.clone()).await;

    revoke_session(state, record.token_hash).await;

    let err = authenticate_result(state, raw_token).await.unwrap_err();
    let WriteScopeError::Operation(err) = err else {
        panic!("expected session authentication operation error, got {err:?}");
    };
    assert!(matches!(err, SessionAuthError::SessionNotFound));
}

#[apply(backends)]
#[tokio::test]
async fn authenticate_with_invalid_base64_token_returns_invalid_token(#[case] backend: Backend) {
    let env = backend.setup().await;
    let state = &env.state;

    // In-charset (base64url) but an invalid length that cannot decode, so hashing
    // fails and `authenticate` reports InvalidToken. (A non-charset string like
    // "not-base64!" can no longer be constructed as a `RawToken`.)
    let err = authenticate_result(state, parse_raw_token("a"))
        .await
        .unwrap_err();
    let WriteScopeError::Operation(err) = err else {
        panic!("expected session authentication operation error, got {err:?}");
    };
    assert!(matches!(err, SessionAuthError::InvalidToken));
}

#[apply(backends)]
#[tokio::test]
async fn list_sessions_returns_only_sessions_for_given_user(#[case] backend: Backend) {
    let env = backend.setup().await;
    let state = &env.state;

    let [alice_id, bob_id] = seed_users(state).await;

    create_session(state, alice_id, parse_session_label("alice-1")).await;
    create_session(state, alice_id, parse_session_label("alice-2")).await;
    create_session(state, bob_id, parse_session_label("bob-1")).await;

    let alice_sessions = state.sessions.list_sessions(alice_id).await.unwrap();
    assert_eq!(alice_sessions.len(), 2);
    assert!(alice_sessions.iter().all(|s| s.user_id == alice_id));

    let bob_sessions = state.sessions.list_sessions(bob_id).await.unwrap();
    assert_eq!(bob_sessions.len(), 1);
    assert_eq!(bob_sessions[0].user_id, bob_id);
}
#[apply(backends)]
#[tokio::test]
async fn session_list_operations(#[case] backend: Backend) {
    let env = backend.setup().await;
    let state = &env.state;
    let user = SeedUser::new().seed(state).await.user_id;

    let session1 = create_session(state, user, parse_session_label("session 1")).await;

    let _session2 = create_session(state, user, parse_session_label("session 2")).await;

    let _session3 = create_session(state, user, parse_session_label("test session")).await;

    let sessions = state
        .sessions
        .list_sessions(user)
        .await
        .expect("list_sessions failed");

    assert_eq!(sessions.len(), 3);

    let labels: Vec<&str> = sessions.iter().map(|s| s.label.as_ref()).collect();
    assert!(labels.contains(&"session 1"));
    assert!(labels.contains(&"session 2"));
    assert!(labels.contains(&"test session"));

    let record = authenticate(state, session1).await;
    assert_eq!(record.user_id, user);
}

async fn set_last_used_at(pool: &CloseablePool, token_hash: &TokenHash, last_used_at: UtcInstant) {
    storage::with_closeable_pool!(pool, pool, {
        sqlx::query("UPDATE sessions SET last_used_at = $1 WHERE token_hash = $2")
            .bind(last_used_at)
            .bind(token_hash)
            .execute(pool)
            .await
            .unwrap();
    });
}

async fn load_last_used_at(pool: &CloseablePool, token_hash: &TokenHash) -> UtcInstant {
    storage::with_closeable_pool!(pool, pool, {
        sqlx::query_scalar("SELECT last_used_at FROM sessions WHERE token_hash = $1")
            .bind(token_hash)
            .fetch_one(pool)
            .await
            .unwrap()
    })
}

async fn create_session(
    state: &AppState,
    user_id: common::ids::UserId,
    label: common::session_label::SessionLabel,
) -> common::token::RawToken {
    let sessions = Arc::clone(&state.sessions);
    let outcome = state
        .write_scope
        .run(|transaction| {
            Box::pin(async move { sessions.create_session(transaction, user_id, &label).await })
        })
        .await
        .expect("session fixture setup should succeed");
    storage::test_support::confirmed_for(outcome, "session fixture setup")
}

async fn authenticate(
    state: &AppState,
    raw_token: common::token::RawToken,
) -> storage::SessionRecord {
    let outcome = authenticate_result(state, raw_token)
        .await
        .expect("session authentication should succeed");
    storage::test_support::confirmed_for(outcome, "session authentication")
}

async fn authenticate_result(
    state: &AppState,
    raw_token: common::token::RawToken,
) -> Result<MutationOutcome<storage::SessionRecord>, WriteScopeError<SessionAuthError>> {
    let sessions = Arc::clone(&state.sessions);
    state
        .write_scope
        .run(|transaction| {
            Box::pin(async move { sessions.authenticate(transaction, &raw_token).await })
        })
        .await
}

async fn revoke_session(state: &AppState, token_hash: common::token::TokenHash) {
    let sessions = Arc::clone(&state.sessions);
    let outcome = state
        .write_scope
        .run(|transaction| {
            Box::pin(async move { sessions.revoke_session(transaction, &token_hash).await })
        })
        .await
        .expect("session revocation should succeed");
    storage::test_support::confirmed_for(outcome, "session revocation");
}
