use std::sync::Arc;

use common::MutationOutcome;
use common::test_support::parse_raw_token;
use common::time::UtcInstant;
use rstest::*;
use rstest_reuse::*;
use storage::test_support::{Backend, SeedUser, backends, confirmed_for};
use storage::{AppState, UseEmailVerificationError, WriteScopeError};

use super::fixtures::raw_exec;
#[apply(backends)]
#[tokio::test]
async fn create_email_verification_and_use_returns_user_id_and_email(#[case] backend: Backend) {
    let env = backend.setup().await;
    let state = &env.state;

    let user_id = SeedUser::new().seed(state).await.user_id;

    let expires_at: UtcInstant = "2099-01-02T03:04:05.123456Z".parse().unwrap();
    let raw_token = create_email_verification(
        state,
        user_id,
        "alice@example.com".parse().unwrap(),
        expires_at,
    )
    .await;

    let consumption = use_email_verification(state, raw_token.clone()).await;

    assert_eq!(consumption.user_id, user_id);
    assert_eq!(consumption.email, "alice@example.com");
}

#[apply(backends)]
#[tokio::test]
async fn use_email_verification_already_used_returns_already_used(#[case] backend: Backend) {
    let env = backend.setup().await;
    let state = &env.state;

    let user_id = SeedUser::new().seed(state).await.user_id;

    let expires_at: UtcInstant = "2099-01-02T03:04:05.123456Z".parse().unwrap();
    let raw_token = create_email_verification(
        state,
        user_id,
        "alice@example.com".parse().unwrap(),
        expires_at,
    )
    .await;

    use_email_verification(state, raw_token.clone()).await;

    let err = use_email_verification_result(state, raw_token)
        .await
        .unwrap_err();
    let WriteScopeError::Operation(err) = err else {
        unreachable!("expected email verification operation error, got {err:?}");
    };
    assert!(
        matches!(err, UseEmailVerificationError::AlreadyUsed),
        "expected AlreadyUsed, got {err:?}"
    );
}

#[apply(backends)]
#[tokio::test]
async fn use_email_verification_expired_returns_expired(#[case] backend: Backend) {
    let env = backend.setup().await;
    let state = &env.state;

    let user_id = SeedUser::new().seed(state).await.user_id;

    let expires_at: UtcInstant = "2000-01-02T03:04:05.123456Z".parse().unwrap();
    let raw_token = create_email_verification(
        state,
        user_id,
        "alice@example.com".parse().unwrap(),
        expires_at,
    )
    .await;

    let err = use_email_verification_result(state, raw_token)
        .await
        .unwrap_err();
    let WriteScopeError::Operation(err) = err else {
        unreachable!("expected email verification operation error, got {err:?}");
    };
    assert!(
        matches!(err, UseEmailVerificationError::Expired),
        "expected Expired, got {err:?}"
    );
}

#[apply(backends)]
#[tokio::test]
async fn use_email_verification_unknown_token_returns_not_found(#[case] backend: Backend) {
    let env = backend.setup().await;
    let state = &env.state;

    let err = use_email_verification_result(state, parse_raw_token("not-a-real-token"))
        .await
        .unwrap_err();
    let WriteScopeError::Operation(err) = err else {
        unreachable!("expected email verification operation error, got {err:?}");
    };
    assert!(
        matches!(err, UseEmailVerificationError::NotFound),
        "expected NotFound, got {err:?}"
    );
}

#[apply(backends)]
#[tokio::test]
async fn second_email_verification_supersedes_first(#[case] backend: Backend) {
    let env = backend.setup().await;
    let state = &env.state;

    let user_id = SeedUser::new().seed(state).await.user_id;

    let expires_at: UtcInstant = "2099-01-02T03:04:05.123456Z".parse().unwrap();
    let first_token = create_email_verification(
        state,
        user_id,
        "alice@example.com".parse().unwrap(),
        expires_at,
    )
    .await;

    // Create a second verification; the first should be superseded.
    let second_token = create_email_verification(
        state,
        user_id,
        "alice2@example.com".parse().unwrap(),
        expires_at,
    )
    .await;

    // Second token works normally.
    let consumption = use_email_verification(state, second_token).await;
    assert_eq!(consumption.user_id, user_id);
    assert_eq!(consumption.email, "alice2@example.com");

    // First token is now either NotFound or Expired.
    let err = use_email_verification_result(state, first_token)
        .await
        .unwrap_err();
    let WriteScopeError::Operation(err) = err else {
        unreachable!("expected email verification operation error, got {err:?}");
    };
    assert!(
        matches!(
            err,
            UseEmailVerificationError::NotFound | UseEmailVerificationError::Expired
        ),
        "expected NotFound or Expired for superseded token, got {err:?}"
    );
}

#[apply(backends)]
#[tokio::test]
async fn use_email_verification_with_corrupt_stored_email_returns_internal(
    #[case] backend: Backend,
) {
    let env = backend.setup().await;
    let state = &env.state;

    let user_id = SeedUser::new().seed(state).await.user_id;

    let expires_at: UtcInstant = "2099-01-02T03:04:05.123456Z".parse().unwrap();
    let raw_token = create_email_verification(
        state,
        user_id,
        "alice@example.com".parse().unwrap(),
        expires_at,
    )
    .await;

    // Corrupt the stored address out-of-band so claiming the token yields a
    // value that no longer parses as an email. The `email` column is plain
    // TEXT on both backends, so the same UPDATE is portable.
    raw_exec(
        backend,
        &env,
        "UPDATE email_verifications SET email = 'not-an-email'",
    )
    .await;

    let err = use_email_verification_result(state, raw_token)
        .await
        .unwrap_err();
    let WriteScopeError::Operation(err) = err else {
        unreachable!("expected email verification operation error, got {err:?}");
    };
    assert!(
        matches!(err, UseEmailVerificationError::Internal(_)),
        "expected Internal for unparseable stored email, got {err:?}"
    );
}

async fn create_email_verification(
    state: &AppState,
    user_id: common::ids::UserId,
    email: common::email::Email,
    expires_at: UtcInstant,
) -> common::token::RawToken {
    let email_verifications = Arc::clone(&state.email_verifications);
    let outcome = state
        .write_scope
        .run(|transaction| {
            Box::pin(async move {
                email_verifications
                    .create_email_verification(transaction, user_id, &email, expires_at)
                    .await
            })
        })
        .await
        .expect("email-verification fixture setup should succeed");
    confirmed_for(outcome, "email-verification fixture setup")
}

async fn use_email_verification(
    state: &AppState,
    raw_token: common::token::RawToken,
) -> storage::EmailVerificationConsumption {
    let outcome = use_email_verification_result(state, raw_token)
        .await
        .expect("email verification should succeed");
    confirmed_for(outcome, "email verification")
}

async fn use_email_verification_result(
    state: &AppState,
    raw_token: common::token::RawToken,
) -> Result<
    MutationOutcome<storage::EmailVerificationConsumption>,
    WriteScopeError<UseEmailVerificationError>,
> {
    let email_verifications = Arc::clone(&state.email_verifications);
    state
        .write_scope
        .run(|transaction| {
            Box::pin(async move {
                email_verifications
                    .use_email_verification(transaction, &raw_token)
                    .await
            })
        })
        .await
}
