use common::ids::UserId;
use common::test_support::{parse_bio, parse_display_name, parse_email};
use rstest::*;
use rstest_reuse::*;
use storage::test_support::{Backend, SeedUser, backends};
use storage::{CreateUserError, ProfileUpdate, UserAuthError};

use crate::storage::fixtures::{password, username};
#[apply(backends)]
#[tokio::test]
async fn create_user_succeeds_and_get_by_username_returns_record(#[case] backend: Backend) {
    let env = backend.setup().await;
    let state = &env.state;

    let user_id = state
        .users
        .create_user(
            &username("alice"),
            &password("password123"),
            Some(&parse_display_name("Alice")),
            false,
        )
        .await
        .unwrap();

    let record = state
        .users
        .get_user_by_username(&username("alice"))
        .await
        .unwrap()
        .unwrap();
    assert_eq!(record.user_id, user_id);
    assert_eq!(record.username, "alice");
    assert_eq!(record.display_name.as_deref(), Some("Alice"));
}

#[apply(backends)]
#[tokio::test]
async fn duplicate_username_returns_username_taken(#[case] backend: Backend) {
    let env = backend.setup().await;
    let state = &env.state;

    state
        .users
        .create_user(&username("alice"), &password("password123"), None, false)
        .await
        .unwrap();

    let err = state
        .users
        .create_user(&username("alice"), &password("other_password"), None, false)
        .await
        .unwrap_err();
    assert!(matches!(err, CreateUserError::UsernameTaken));
}

#[apply(backends)]
#[tokio::test]
async fn authenticate_correct_password_returns_record_and_sets_last_authenticated_at(
    #[case] backend: Backend,
) {
    let env = backend.setup().await;
    let state = &env.state;

    let user = SeedUser::new()
        .password("secret_password")
        .seed(state)
        .await;

    let record = state
        .users
        .authenticate(&user.username, &password("secret_password"))
        .await
        .unwrap();
    assert_eq!(record.username, user.username);
    assert!(record.last_authenticated_at.is_some());

    let fetched = state.users.get_user(record.user_id).await.unwrap().unwrap();
    assert!(fetched.last_authenticated_at.is_some());
}

#[apply(backends)]
#[tokio::test]
async fn authenticate_wrong_password_returns_invalid_credentials(#[case] backend: Backend) {
    let env = backend.setup().await;
    let state = &env.state;

    let user = SeedUser::new()
        .password("correct_password")
        .seed(state)
        .await;

    let err = state
        .users
        .authenticate(&user.username, &password("wrong_password"))
        .await
        .unwrap_err();
    assert!(matches!(err, UserAuthError::InvalidCredentials));
}

#[apply(backends)]
#[tokio::test]
async fn authenticate_unknown_username_returns_invalid_credentials(#[case] backend: Backend) {
    let env = backend.setup().await;
    let state = &env.state;

    let err = state
        .users
        .authenticate(&username("nobody"), &password("some_password"))
        .await
        .unwrap_err();
    assert!(matches!(err, UserAuthError::InvalidCredentials));
}

#[apply(backends)]
#[tokio::test]
async fn update_profile_persists_changes(#[case] backend: Backend) {
    let env = backend.setup().await;
    let state = &env.state;

    let user_id = SeedUser::new()
        .display_name("Dave")
        .seed(state)
        .await
        .user_id;

    state
        .users
        .update_profile(
            user_id,
            &ProfileUpdate {
                display_name: Some(&parse_display_name("David")),
                bio: Some(&parse_bio("A bio")),
            },
        )
        .await
        .unwrap();

    let record = state.users.get_user(user_id).await.unwrap().unwrap();
    assert_eq!(record.display_name.as_deref(), Some("David"));
    assert_eq!(record.bio.as_deref(), Some("A bio"));
}

#[apply(backends)]
#[tokio::test]
async fn get_user_unknown_id_returns_none(#[case] backend: Backend) {
    let env = backend.setup().await;
    let state = &env.state;

    let record = state.users.get_user(UserId::from(999)).await.unwrap();
    assert!(record.is_none());
}
#[apply(backends)]
#[tokio::test]
async fn set_email_persists_and_get_user_reflects_it(#[case] backend: Backend) {
    let env = backend.setup().await;
    let state = &env.state;

    let user_id = SeedUser::new().seed(state).await.user_id;

    let addr = parse_email("alice@example.com");
    state
        .users
        .set_email(user_id, Some(&addr), true)
        .await
        .unwrap();

    let record = state.users.get_user(user_id).await.unwrap().unwrap();
    assert_eq!(record.email, Some(addr));
    assert!(record.email_verified);
}

#[apply(backends)]
#[tokio::test]
async fn set_email_clears_previously_set_email(#[case] backend: Backend) {
    let env = backend.setup().await;
    let state = &env.state;

    let user_id = SeedUser::new().seed(state).await.user_id;

    let addr = parse_email("bob@example.com");
    state
        .users
        .set_email(user_id, Some(&addr), true)
        .await
        .unwrap();

    state.users.set_email(user_id, None, false).await.unwrap();

    let record = state.users.get_user(user_id).await.unwrap().unwrap();
    assert!(record.email.is_none());
    assert!(!record.email_verified);
}
#[apply(backends)]
#[tokio::test]
async fn set_password_authenticate_with_old_returns_invalid_and_new_succeeds(
    #[case] backend: Backend,
) {
    let env = backend.setup().await;
    let state = &env.state;
    let users = &state.users;

    let user = SeedUser::new().password("old_password1").seed(state).await;

    users
        .set_password(user.user_id, &password("new_password2"))
        .await
        .unwrap();

    // Old password no longer works.
    let err = users
        .authenticate(&user.username, &password("old_password1"))
        .await
        .unwrap_err();
    assert!(
        matches!(err, UserAuthError::InvalidCredentials),
        "expected InvalidCredentials, got {err:?}"
    );

    // New password works.
    let record = users
        .authenticate(&user.username, &password("new_password2"))
        .await
        .unwrap();
    assert_eq!(record.user_id, user.user_id);
}
