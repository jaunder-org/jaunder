use chrono::{DateTime, Duration, Utc};
use common::test_support::{parse_raw_token, parse_session_label};
use common::token::TokenHash;
use rstest::*;
use rstest_reuse::*;
use storage::SessionAuthError;
use storage::test_support::{Backend, CloseablePool, SeedUser, TestEnv, backends, seed_users};

use crate::helpers::create_session_for;
#[apply(backends)]
#[tokio::test]
async fn create_session_then_authenticate_returns_correct_record(#[case] backend: Backend) {
    let env = backend.setup().await;
    let state = &env.state;

    let user = SeedUser::new().seed(state).await;

    let raw_token = state
        .sessions
        .create_session(user.user_id, &parse_session_label("test"))
        .await
        .unwrap();
    let record = state.sessions.authenticate(&raw_token).await.unwrap();

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
    let record = state.sessions.authenticate(&raw_token).await.unwrap();

    assert_eq!(record.user_id, user_id);
}

#[apply(backends)]
#[tokio::test]
async fn fresh_authenticate_returns_the_persisted_last_used_at(#[case] backend: Backend) {
    let TestEnv { state, base } = backend.setup().await;
    let user_id = SeedUser::new().seed(&state).await.user_id;
    let raw_token = state
        .sessions
        .create_session(user_id, &parse_session_label("test session"))
        .await
        .unwrap();

    let token_hash = host::token::hash(&raw_token).unwrap();
    let first_record = state.sessions.authenticate(&raw_token).await.unwrap();
    let stored = first_record.last_used_at;

    let record = state.sessions.authenticate(&raw_token).await.unwrap();
    let persisted_after_auth = load_last_used_at(base.pool(), &token_hash).await;

    assert_eq!(record.last_used_at, stored);
    assert_eq!(persisted_after_auth, stored);
}

#[apply(backends)]
#[tokio::test]
async fn stale_authenticate_refreshes_the_persisted_last_used_at(#[case] backend: Backend) {
    let TestEnv { state, base } = backend.setup().await;
    let user_id = SeedUser::new().seed(&state).await.user_id;
    let raw_token = state
        .sessions
        .create_session(user_id, &parse_session_label("test session"))
        .await
        .unwrap();

    let token_hash = host::token::hash(&raw_token).unwrap();
    let stale = Utc::now() - Duration::seconds(120);
    set_last_used_at(base.pool(), &token_hash, stale).await;

    let record = state.sessions.authenticate(&raw_token).await.unwrap();
    let persisted_after_auth = load_last_used_at(base.pool(), &token_hash).await;
    let freshness_cutoff_after_auth = Utc::now() - Duration::seconds(60);

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
    let record = state.sessions.authenticate(&raw_token).await.unwrap();

    state
        .sessions
        .revoke_session(&record.token_hash)
        .await
        .unwrap();

    let err = state.sessions.authenticate(&raw_token).await.unwrap_err();
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
    let err = state
        .sessions
        .authenticate(&parse_raw_token("a"))
        .await
        .unwrap_err();
    assert!(matches!(err, SessionAuthError::InvalidToken));
}

#[apply(backends)]
#[tokio::test]
async fn list_sessions_returns_only_sessions_for_given_user(#[case] backend: Backend) {
    let env = backend.setup().await;
    let state = &env.state;

    let [alice_id, bob_id] = seed_users(state).await;

    state
        .sessions
        .create_session(alice_id, &parse_session_label("alice-1"))
        .await
        .unwrap();
    state
        .sessions
        .create_session(alice_id, &parse_session_label("alice-2"))
        .await
        .unwrap();
    state
        .sessions
        .create_session(bob_id, &parse_session_label("bob-1"))
        .await
        .unwrap();

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

    let session1 = state
        .sessions
        .create_session(user, &parse_session_label("session 1"))
        .await
        .expect("create_session 1 failed");

    let _session2 = state
        .sessions
        .create_session(user, &parse_session_label("session 2"))
        .await
        .expect("create_session 2 failed");

    let _session3 = state
        .sessions
        .create_session(user, &parse_session_label("test session"))
        .await
        .expect("create_session 3 failed");

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

    let record = state
        .sessions
        .authenticate(&session1)
        .await
        .expect("authenticate failed");
    assert_eq!(record.user_id, user);
}

async fn set_last_used_at(
    pool: &CloseablePool,
    token_hash: &TokenHash,
    last_used_at: DateTime<Utc>,
) {
    storage::with_closeable_pool!(pool, pool, {
        sqlx::query("UPDATE sessions SET last_used_at = $1 WHERE token_hash = $2")
            .bind(last_used_at)
            .bind(token_hash)
            .execute(pool)
            .await
            .unwrap();
    });
}

async fn load_last_used_at(pool: &CloseablePool, token_hash: &TokenHash) -> DateTime<Utc> {
    storage::with_closeable_pool!(pool, pool, {
        sqlx::query_scalar("SELECT last_used_at FROM sessions WHERE token_hash = $1")
            .bind(token_hash)
            .fetch_one(pool)
            .await
            .unwrap()
    })
}
