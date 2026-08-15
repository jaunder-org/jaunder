use super::*;
#[apply(backends)]
#[tokio::test]
async fn confirm_password_reset_hash_failure_returns_internal(#[case] backend: Backend) {
    let env = backend.setup().await;
    let state = &env.state;
    let user_id = SeedUser::new().seed(state).await.user_id;
    let reset_token = state
        .password_resets
        .create_password_reset(user_id, Utc::now() + chrono::Duration::hours(1))
        .await
        .unwrap();
    // Valid token → the claim succeeds, then hashing the new password fails → Internal
    // (success-path hash failure; the failed hash rolls the claim back).
    let result = state
        .atomic
        .confirm_password_reset(
            &reset_token,
            &password("force-hash-error-for-test-coverage"),
        )
        .await;
    assert!(matches!(
        result,
        Err(ConfirmPasswordResetError::Internal(_))
    ));
}

#[apply(backends)]
#[tokio::test]
async fn confirm_password_reset_changes_credentials(#[case] backend: Backend) {
    let env = backend.setup().await;
    let state = &env.state;
    let user = SeedUser::new().seed(state).await;
    let reset_token = state
        .password_resets
        .create_password_reset(user.user_id, Utc::now() + chrono::Duration::hours(1))
        .await
        .unwrap();

    state
        .atomic
        .confirm_password_reset(&reset_token, &password("new_password123"))
        .await
        .unwrap();

    let authenticated = state
        .users
        .authenticate(&user.username, &password("new_password123"))
        .await
        .unwrap();
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
    let result = state
        .atomic
        .confirm_password_reset(
            &parse_raw_token("dGVzdA"),
            &password("force-hash-error-for-test-coverage"),
        )
        .await;
    assert!(matches!(result, Err(ConfirmPasswordResetError::NotFound)));
}
#[apply(backends)]
#[tokio::test]
async fn create_password_reset_and_use_returns_user_id(#[case] backend: Backend) {
    let env = backend.setup().await;
    let state = &env.state;

    let user_id = SeedUser::new().seed(state).await.user_id;

    let expires_at = Utc::now() + chrono::Duration::hours(24);
    let raw_token = state
        .password_resets
        .create_password_reset(user_id, expires_at)
        .await
        .unwrap();

    let returned_user_id = state
        .password_resets
        .use_password_reset(&raw_token)
        .await
        .unwrap();
    assert_eq!(returned_user_id, user_id);
}

#[apply(backends)]
#[tokio::test]
async fn use_password_reset_already_used_returns_already_used(#[case] backend: Backend) {
    let env = backend.setup().await;
    let state = &env.state;

    let user_id = SeedUser::new().seed(state).await.user_id;

    let expires_at = Utc::now() + chrono::Duration::hours(24);
    let raw_token = state
        .password_resets
        .create_password_reset(user_id, expires_at)
        .await
        .unwrap();

    state
        .password_resets
        .use_password_reset(&raw_token)
        .await
        .unwrap();

    let err = state
        .password_resets
        .use_password_reset(&raw_token)
        .await
        .unwrap_err();
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

    let expires_at = Utc::now() - chrono::Duration::hours(1);
    let raw_token = state
        .password_resets
        .create_password_reset(user_id, expires_at)
        .await
        .unwrap();

    let err = state
        .password_resets
        .use_password_reset(&raw_token)
        .await
        .unwrap_err();
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

    let err = state
        .password_resets
        .use_password_reset(&parse_raw_token("not-a-real-token"))
        .await
        .unwrap_err();
    assert!(
        matches!(err, UsePasswordResetError::NotFound),
        "expected NotFound, got {err:?}"
    );
}
