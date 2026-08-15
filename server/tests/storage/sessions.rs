use super::*;
use common::test_support::parse_session_label;
use storage::SessionAuthError;
use storage::test_support::seed_users;
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
async fn authenticate_updates_last_used_at(#[case] backend: Backend) {
    let env = backend.setup().await;
    let state = &env.state;

    let user_id = SeedUser::new().seed(state).await.user_id;

    let raw_token = create_session_for(state, user_id).await.token;
    let first = state.sessions.authenticate(&raw_token).await.unwrap();
    let second = state.sessions.authenticate(&raw_token).await.unwrap();

    assert!(second.last_used_at >= first.last_used_at);
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
