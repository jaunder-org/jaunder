use std::sync::Arc;

use common::MutationOutcome;
use common::test_support::parse_raw_token;
use common::time::UtcInstant;
use rstest::*;
use rstest_reuse::*;
use storage::test_support::{Backend, SeedUser, backends};
use storage::{AppState, ConfirmPasswordResetError, UsePasswordResetError, WriteScopeError};

use super::fixtures::password;
#[apply(backends)]
#[tokio::test]
async fn confirm_password_reset_hash_failure_returns_internal(#[case] backend: Backend) {
    let env = backend.setup().await;
    let state = &env.state;
    let user_id = SeedUser::new().seed(state).await.user_id;
    let reset_token = create_password_reset(
        state,
        user_id,
        "2099-01-02T03:04:05.123456Z".parse().unwrap(),
    )
    .await;
    // Valid token → the claim succeeds, then hashing the new password fails → Internal
    // (success-path hash failure; the failed hash rolls the claim back).
    let result = confirm_password_reset_result(
        state,
        reset_token,
        password("force-hash-error-for-test-coverage"),
    )
    .await;
    assert!(matches!(
        result,
        Err(WriteScopeError::Operation(
            ConfirmPasswordResetError::Internal(_)
        ))
    ));
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

    let returned_user_id = use_password_reset(state, raw_token).await;
    assert_eq!(returned_user_id, user_id);
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
        panic!("expected password reset operation error, got {err:?}");
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
        panic!("expected password reset operation error, got {err:?}");
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
        panic!("expected password reset operation error, got {err:?}");
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

async fn use_password_reset(
    state: &AppState,
    raw_token: common::token::RawToken,
) -> common::ids::UserId {
    let outcome = use_password_reset_result(state, raw_token)
        .await
        .expect("password reset should succeed");
    storage::test_support::confirmed_for(outcome, "password reset")
}

async fn use_password_reset_result(
    state: &AppState,
    raw_token: common::token::RawToken,
) -> Result<MutationOutcome<common::ids::UserId>, WriteScopeError<UsePasswordResetError>> {
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

async fn confirm_password_reset(
    state: &AppState,
    raw_token: common::token::RawToken,
    password: host::password::Password,
) {
    let outcome = confirm_password_reset_result(state, raw_token, password)
        .await
        .expect("password reset confirmation should succeed");
    storage::test_support::confirmed_for(outcome, "password reset confirmation");
}

async fn confirm_password_reset_result(
    state: &AppState,
    raw_token: common::token::RawToken,
    password: host::password::Password,
) -> Result<MutationOutcome<()>, WriteScopeError<ConfirmPasswordResetError>> {
    let atomic = Arc::clone(&state.atomic);
    state
        .write_scope
        .run(|transaction| {
            Box::pin(async move {
                atomic
                    .confirm_password_reset(transaction, &raw_token, &password)
                    .await
            })
        })
        .await
}
