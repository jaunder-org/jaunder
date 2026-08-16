use chrono::Utc;
use common::test_support::parse_raw_token;
use rstest::*;
use rstest_reuse::*;
use storage::UseEmailVerificationError;
use storage::test_support::{Backend, SeedUser, backends};

use super::fixtures::raw_exec;
#[apply(backends)]
#[tokio::test]
async fn create_email_verification_and_use_returns_user_id_and_email(#[case] backend: Backend) {
    let env = backend.setup().await;
    let state = &env.state;

    let user_id = SeedUser::new().seed(state).await.user_id;

    let expires_at = Utc::now() + chrono::Duration::hours(24);
    let raw_token = state
        .email_verifications
        .create_email_verification(user_id, &"alice@example.com".parse().unwrap(), expires_at)
        .await
        .unwrap();

    let (returned_user_id, returned_email) = state
        .email_verifications
        .use_email_verification(&raw_token)
        .await
        .unwrap();

    assert_eq!(returned_user_id, user_id);
    assert_eq!(returned_email, "alice@example.com");
}

#[apply(backends)]
#[tokio::test]
async fn use_email_verification_already_used_returns_already_used(#[case] backend: Backend) {
    let env = backend.setup().await;
    let state = &env.state;

    let user_id = SeedUser::new().seed(state).await.user_id;

    let expires_at = Utc::now() + chrono::Duration::hours(24);
    let raw_token = state
        .email_verifications
        .create_email_verification(user_id, &"alice@example.com".parse().unwrap(), expires_at)
        .await
        .unwrap();

    state
        .email_verifications
        .use_email_verification(&raw_token)
        .await
        .unwrap();

    let err = state
        .email_verifications
        .use_email_verification(&raw_token)
        .await
        .unwrap_err();
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

    let expires_at = Utc::now() - chrono::Duration::hours(1);
    let raw_token = state
        .email_verifications
        .create_email_verification(user_id, &"alice@example.com".parse().unwrap(), expires_at)
        .await
        .unwrap();

    let err = state
        .email_verifications
        .use_email_verification(&raw_token)
        .await
        .unwrap_err();
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

    let err = state
        .email_verifications
        .use_email_verification(&parse_raw_token("not-a-real-token"))
        .await
        .unwrap_err();
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

    let expires_at = Utc::now() + chrono::Duration::hours(24);
    let first_token = state
        .email_verifications
        .create_email_verification(user_id, &"alice@example.com".parse().unwrap(), expires_at)
        .await
        .unwrap();

    // Create a second verification; the first should be superseded.
    let second_token = state
        .email_verifications
        .create_email_verification(user_id, &"alice2@example.com".parse().unwrap(), expires_at)
        .await
        .unwrap();

    // Second token works normally.
    let (uid, email) = state
        .email_verifications
        .use_email_verification(&second_token)
        .await
        .unwrap();
    assert_eq!(uid, user_id);
    assert_eq!(email, "alice2@example.com");

    // First token is now either NotFound or Expired.
    let err = state
        .email_verifications
        .use_email_verification(&first_token)
        .await
        .unwrap_err();
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

    let expires_at = Utc::now() + chrono::Duration::hours(24);
    let raw_token = state
        .email_verifications
        .create_email_verification(user_id, &"alice@example.com".parse().unwrap(), expires_at)
        .await
        .unwrap();

    // Corrupt the stored address out-of-band so claiming the token yields a
    // value that no longer parses as an email. The `email` column is plain
    // TEXT on both backends, so the same UPDATE is portable.
    raw_exec(
        backend,
        &env,
        "UPDATE email_verifications SET email = 'not-an-email'",
    )
    .await;

    let err = state
        .email_verifications
        .use_email_verification(&raw_token)
        .await
        .unwrap_err();
    assert!(
        matches!(err, UseEmailVerificationError::Internal(_)),
        "expected Internal for unparseable stored email, got {err:?}"
    );
}
