use std::sync::Arc;

use super::fixtures::password;
use common::MutationOutcome;
use common::test_support::parse_raw_token;
use common::time::UtcInstant;
use common::token::RawToken;
use host::password::Password;
use rstest::*;
use rstest_reuse::*;
use storage::test_support::{Backend, SeedUser, backends};
use storage::{
    AppState, UsePasswordResetError, WriteScopeError,
    account_mutations::{self, ConfirmPasswordResetError},
};
#[apply(backends)]
#[tokio::test]
async fn confirm_password_reset_hash_failure_preserves_password_error_source_and_token(
    #[case] backend: Backend,
) {
    let env = backend.setup().await;
    let state = &env.state;
    let user_id = SeedUser::new().seed(state).await.user_id;
    let reset_token = create_password_reset(
        state,
        user_id,
        "2099-01-02T03:04:05.123456Z".parse().unwrap(),
    )
    .await;

    let error = confirm_password_reset_result(
        state,
        reset_token.clone(),
        password("force-hash-error-for-test-coverage"),
    )
    .await
    .expect_err("a forced password hash failure must reject the reset");
    let WriteScopeError::Operation(ConfirmPasswordResetError::Internal(sqlx::Error::Io(source))) =
        error
    else {
        panic!("expected PasswordError wrapped by sqlx::Error::Io");
    };
    assert!(
        source
            .get_ref()
            .and_then(|source| source.downcast_ref::<host::password::PasswordError>())
            .is_some(),
        "the password error must remain downcastable through sqlx::Error::Io"
    );

    assert_eq!(use_password_reset(state, reset_token).await, user_id);
}

#[apply(backends)]
#[tokio::test]
async fn confirm_password_reset_changes_credentials(#[case] backend: Backend) {
    let env = backend.setup().await;
    let state = &env.state;
    let user = SeedUser::new().seed(state).await;
    let reset_token = create_password_reset(
        state,
        user.user_id,
        "2099-01-02T03:04:05.123456Z".parse().unwrap(),
    )
    .await;

    let sessions = Arc::clone(&state.sessions);
    let label = common::test_support::parse_session_label("Existing device");
    let outcome = state
        .write_scope
        .run(|transaction| {
            Box::pin(async move {
                sessions
                    .create_session(transaction, user.user_id, &label)
                    .await
            })
        })
        .await
        .expect("session fixture setup should succeed");
    storage::test_support::confirmed_for(outcome, "session fixture setup");
    assert_eq!(
        state
            .sessions
            .list_sessions(user.user_id)
            .await
            .unwrap()
            .len(),
        1
    );

    confirm_password_reset(state, reset_token, password("new_password123")).await;

    let users = Arc::clone(&state.users);
    let username = user.username.clone();
    let password = password("new_password123");
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
    let authenticated = storage::test_support::confirmed_for(outcome, "authentication");
    assert_eq!(authenticated.user_id, user.user_id);
    assert!(
        state
            .sessions
            .list_sessions(user.user_id)
            .await
            .unwrap()
            .is_empty()
    );
}

#[apply(backends)]
#[tokio::test]
async fn confirm_password_reset_bogus_token_returns_not_found_without_hashing(
    #[case] backend: Backend,
) {
    let env = backend.setup().await;
    let state = &env.state;
    // No password_resets row matches this token. A hash-failing new password proves the
    // hash is NOT attempted: the claim rejects the token first -> NotFound, not Internal
    // (ADR-0022).
    let result = confirm_password_reset_result(
        state,
        parse_raw_token("dGVzdA"),
        password("force-hash-error-for-test-coverage"),
    )
    .await;
    assert!(matches!(
        result,
        Err(WriteScopeError::Operation(
            ConfirmPasswordResetError::NotFound
        ))
    ));
}
#[apply(backends)]
#[tokio::test]
async fn create_password_reset_and_use_returns_user_id(#[case] backend: Backend) {
    let env = backend.setup().await;
    let state = &env.state;

    let user_id = SeedUser::new().seed(state).await.user_id;

    let expires_at: UtcInstant = "2099-01-02T03:04:05.123456Z".parse().unwrap();
    let raw_token = create_password_reset(state, user_id, expires_at).await;

    let consumption = use_password_reset(state, raw_token).await;
    assert_eq!(consumption.user_id, user_id);
}

#[apply(backends)]
#[tokio::test]
async fn use_password_reset_already_used_returns_already_used(#[case] backend: Backend) {
    let env = backend.setup().await;
    let state = &env.state;

    let user_id = SeedUser::new().seed(state).await.user_id;

    let expires_at: UtcInstant = "2099-01-02T03:04:05.123456Z".parse().unwrap();
    let raw_token = create_password_reset(state, user_id, expires_at).await;

    use_password_reset(state, raw_token.clone()).await;

    let err = use_password_reset_result(state, raw_token)
        .await
        .unwrap_err();
    let WriteScopeError::Operation(err) = err else {
        unreachable!("expected password reset operation error, got {err:?}");
    };
    assert!(
        matches!(err, UsePasswordResetError::AlreadyUsed),
        "expected AlreadyUsed, got {err:?}"
    );
}

#[apply(backends)]
#[tokio::test]
async fn use_password_reset_expired_returns_expired(#[case] backend: Backend) {
    let env = backend.setup().await;
    let state = &env.state;

    let user_id = SeedUser::new().seed(state).await.user_id;

    let expires_at: UtcInstant = "2000-01-02T03:04:05.123456Z".parse().unwrap();
    let raw_token = create_password_reset(state, user_id, expires_at).await;

    let err = use_password_reset_result(state, raw_token)
        .await
        .unwrap_err();
    let WriteScopeError::Operation(err) = err else {
        unreachable!("expected password reset operation error, got {err:?}");
    };
    assert!(
        matches!(err, UsePasswordResetError::Expired),
        "expected Expired, got {err:?}"
    );
}

#[apply(backends)]
#[tokio::test]
async fn use_password_reset_unknown_token_returns_not_found(#[case] backend: Backend) {
    let env = backend.setup().await;
    let state = &env.state;

    let err = use_password_reset_result(state, parse_raw_token("not-a-real-token"))
        .await
        .unwrap_err();
    let WriteScopeError::Operation(err) = err else {
        unreachable!("expected password reset operation error, got {err:?}");
    };
    assert!(
        matches!(err, UsePasswordResetError::NotFound),
        "expected NotFound, got {err:?}"
    );
}

async fn create_password_reset(
    state: &AppState,
    user_id: common::ids::UserId,
    expires_at: UtcInstant,
) -> common::token::RawToken {
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
        .expect("password-reset fixture setup should succeed");
    storage::test_support::confirmed_for(outcome, "password-reset fixture setup")
}

async fn use_password_reset(state: &AppState, raw_token: RawToken) -> PasswordResetConsumption {
    let outcome = use_password_reset_result(state, raw_token)
        .await
        .expect("password reset should succeed");
    test_support::confirmed_for(outcome, "password reset")
}

async fn use_password_reset_result(
    state: &AppState,
    raw_token: RawToken,
) -> Result<MutationOutcome<PasswordResetConsumption>, WriteScopeError<UsePasswordResetError>> {
    let password_resets = Arc::clone(&state.password_resets);
    state
        .write_scope
        .run(|transaction| {
            Box::pin(async move {
                password_resets
                    .use_password_reset(transaction, &raw_token)
                    .await
            })
        })
        .await
}

async fn confirm_password_reset(state: &AppState, raw_token: RawToken, password: Password) {
    let outcome = confirm_password_reset_result(state, raw_token, password)
        .await
        .expect("password reset confirmation should succeed");
    test_support::confirmed_for(outcome, "password reset confirmation");
}

async fn confirm_password_reset_result(
    state: &AppState,
    raw_token: RawToken,
    password: Password,
) -> Result<MutationOutcome<()>, WriteScopeError<ConfirmPasswordResetError>> {
    let password_resets = Arc::clone(&state.password_resets);
    let users = Arc::clone(&state.users);
    let sessions = Arc::clone(&state.sessions);
    state
        .write_scope
        .run(|transaction| {
            Box::pin(async move {
                account_mutations::confirm_password_reset(
                    transaction,
                    password_resets.as_ref(),
                    users.as_ref(),
                    sessions.as_ref(),
                    &raw_token,
                    &password,
                )
                .await
            })
        })
        .await
}
