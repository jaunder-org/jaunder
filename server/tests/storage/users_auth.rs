use std::sync::Arc;

use common::MutationOutcome;
use common::ids::UserId;
use common::test_support::{parse_bio, parse_display_name, parse_email};
use rstest::*;
use rstest_reuse::*;
use storage::test_support::{Backend, SeedUser, backends};
use storage::{AppState, CreateUserError, ProfileUpdate, UserAuthError, WriteScopeError};

use crate::storage::fixtures::{password, username};
#[apply(backends)]
#[tokio::test]
async fn create_user_succeeds_and_get_by_username_returns_record(#[case] backend: Backend) {
    let env = backend.setup().await;
    let state = &env.state;

    let user_id = create_user(
        state,
        username("alice"),
        password("password123"),
        Some(parse_display_name("Alice")),
        false,
    )
    .await;

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

    create_user(
        state,
        username("alice"),
        password("password123"),
        None,
        false,
    )
    .await;

    let err = create_user_result(
        state,
        username("alice"),
        password("other_password"),
        None,
        false,
    )
    .await
    .unwrap_err();
    let WriteScopeError::Operation(err) = err else {
        panic!("expected user creation operation error, got {err:?}");
    };
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

    let record = authenticate(state, user.username.clone(), password("secret_password")).await;
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

    let err = authenticate_result(state, user.username.clone(), password("wrong_password"))
        .await
        .unwrap_err();
    let WriteScopeError::Operation(err) = err else {
        panic!("expected authentication operation error, got {err:?}");
    };
    assert!(matches!(err, UserAuthError::InvalidCredentials));
}

#[apply(backends)]
#[tokio::test]
async fn authenticate_unknown_username_returns_invalid_credentials(#[case] backend: Backend) {
    let env = backend.setup().await;
    let state = &env.state;

    let err = authenticate_result(state, username("nobody"), password("some_password"))
        .await
        .unwrap_err();
    let WriteScopeError::Operation(err) = err else {
        panic!("expected authentication operation error, got {err:?}");
    };
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

    update_profile(
        state,
        user_id,
        Some(parse_display_name("David")),
        Some(parse_bio("A bio")),
    )
    .await;

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
    set_email(state, user_id, Some(addr.clone()), true).await;

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
    set_email(state, user_id, Some(addr), true).await;

    set_email(state, user_id, None, false).await;

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
    let user = SeedUser::new().password("old_password1").seed(state).await;

    set_password(state, user.user_id, password("new_password2")).await;

    // Old password no longer works.
    let err = authenticate_result(state, user.username.clone(), password("old_password1"))
        .await
        .unwrap_err();
    let WriteScopeError::Operation(err) = err else {
        panic!("expected authentication operation error, got {err:?}");
    };
    assert!(
        matches!(err, UserAuthError::InvalidCredentials),
        "expected InvalidCredentials, got {err:?}"
    );

    // New password works.
    let record = authenticate(state, user.username.clone(), password("new_password2")).await;
    assert_eq!(record.user_id, user.user_id);
}

async fn create_user(
    state: &AppState,
    username: common::username::Username,
    password: host::password::Password,
    display_name: Option<common::display_name::DisplayName>,
    is_operator: bool,
) -> UserId {
    let outcome = create_user_result(state, username, password, display_name, is_operator)
        .await
        .expect("user fixture setup should succeed");
    match outcome {
        MutationOutcome::Confirmed(user_id) => user_id,
        MutationOutcome::CommitIndeterminate(_) => {
            panic!("user fixture setup requires a confirmed commit")
        }
    }
}

async fn create_user_result(
    state: &AppState,
    username: common::username::Username,
    password: host::password::Password,
    display_name: Option<common::display_name::DisplayName>,
    is_operator: bool,
) -> Result<MutationOutcome<UserId>, WriteScopeError<CreateUserError>> {
    let users = Arc::clone(&state.users);
    let password = storage::prepare_password(password).await.map_err(|error| {
        WriteScopeError::Operation(CreateUserError::Internal(sqlx::Error::Io(error)))
    })?;
    state
        .write_scope
        .run(|transaction| {
            Box::pin(async move {
                users
                    .create_user(
                        transaction,
                        &username,
                        &password,
                        display_name.as_ref(),
                        is_operator,
                    )
                    .await
            })
        })
        .await
}

async fn authenticate(
    state: &AppState,
    username: common::username::Username,
    password: host::password::Password,
) -> storage::UserRecord {
    let outcome = authenticate_result(state, username, password)
        .await
        .expect("authentication should succeed");
    match outcome {
        MutationOutcome::Confirmed(user) => user,
        MutationOutcome::CommitIndeterminate(_) => {
            panic!("authentication requires a confirmed commit")
        }
    }
}

async fn authenticate_result(
    state: &AppState,
    username: common::username::Username,
    password: host::password::Password,
) -> Result<MutationOutcome<storage::UserRecord>, WriteScopeError<UserAuthError>> {
    let users = Arc::clone(&state.users);
    let authentication = users
        .prepare_authentication(&username, &password)
        .await
        .map_err(WriteScopeError::Operation)?;
    state
        .write_scope
        .run(|transaction| {
            Box::pin(async move { users.authenticate(transaction, authentication).await })
        })
        .await
}

async fn update_profile(
    state: &AppState,
    user_id: UserId,
    display_name: Option<common::display_name::DisplayName>,
    bio: Option<common::bio::Bio>,
) {
    let users = Arc::clone(&state.users);
    let outcome = state
        .write_scope
        .run(|transaction| {
            Box::pin(async move {
                users
                    .update_profile(
                        transaction,
                        user_id,
                        &ProfileUpdate {
                            display_name: display_name.as_ref(),
                            bio: bio.as_ref(),
                        },
                    )
                    .await
            })
        })
        .await
        .expect("profile update should succeed");
    if matches!(outcome, MutationOutcome::CommitIndeterminate(())) {
        panic!("profile update requires a confirmed commit");
    }
}

async fn set_email(
    state: &AppState,
    user_id: UserId,
    email: Option<common::email::Email>,
    verified: bool,
) {
    let users = Arc::clone(&state.users);
    let outcome = state
        .write_scope
        .run(|transaction| {
            Box::pin(async move {
                users
                    .set_email(transaction, user_id, email.as_ref(), verified)
                    .await
            })
        })
        .await
        .expect("set email should succeed");
    if matches!(outcome, MutationOutcome::CommitIndeterminate(())) {
        panic!("set email requires a confirmed commit");
    }
}

async fn set_password(state: &AppState, user_id: UserId, password: host::password::Password) {
    let users = Arc::clone(&state.users);
    let password = storage::prepare_password(password)
        .await
        .expect("password preparation should succeed");
    let outcome = state
        .write_scope
        .run(|transaction| {
            Box::pin(async move { users.set_password(transaction, user_id, &password).await })
        })
        .await
        .expect("set password should succeed");
    if matches!(outcome, MutationOutcome::CommitIndeterminate(())) {
        panic!("set password requires a confirmed commit");
    }
}
