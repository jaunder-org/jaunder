use std::{sync::Arc, time::Duration};

use crate::storage::fixtures::{password, username};
use chrono::Utc;
use common::MutationOutcome;
use common::test_support::parse_display_name;
use common::time::UtcInstant;
use host::invite::InviteCode;
use rstest::*;
use rstest_reuse::*;
use storage::test_support::{Backend, CloseablePool, SeedUser, backends, confirmed_for};
use storage::{
    AppState, OperatorStatus, WriteScopeError,
    account_mutations::{self, RegisterWithInviteError, RegisterWithInviteInput},
};
#[apply(backends)]
#[tokio::test]
async fn create_invite_and_list_invites_includes_it(#[case] backend: Backend) {
    let env = backend.setup().await;
    let state = &env.state;

    let expires_at = UtcInstant::from(Utc::now() + chrono::Duration::hours(24));
    let code = create_invite(state, expires_at).await;

    let list = state.invites.list_invites().await.unwrap();
    assert_eq!(list.len(), 1);
    assert_eq!(list[0].code.as_ref(), code.as_ref());
    assert!(list[0].used_at.is_none());
}

#[apply(backends)]
#[tokio::test]
async fn invite_list_preserves_timestamp_roles_and_used_state(#[case] backend: Backend) {
    let env = backend.setup().await;
    let created_at: UtcInstant = "2099-01-02T03:04:05.123456Z".parse().unwrap();
    let expires_at: UtcInstant = "2099-01-03T03:04:05.654321Z".parse().unwrap();
    let used_at: UtcInstant = "2099-01-02T04:05:06.234567Z".parse().unwrap();
    let code: InviteCode = "role-ordering-code".parse().unwrap();

    storage::with_closeable_pool!(env.base.pool(), pool, {
        sqlx::query("INSERT INTO invites (code, created_at, expires_at) VALUES ($1, $2, $3)")
            .bind(&code)
            .bind(created_at)
            .bind(expires_at)
            .execute(pool)
            .await
            .unwrap();
    });

    let invite = env
        .state
        .invites
        .list_invites()
        .await
        .unwrap()
        .pop()
        .unwrap();
    assert_eq!(invite.created_at, created_at);
    assert_eq!(invite.expires_at, expires_at);
    assert!(invite.used_at.is_none());

    set_invite_used_at(env.base.pool(), &code, used_at).await;

    let invite = env
        .state
        .invites
        .list_invites()
        .await
        .unwrap()
        .pop()
        .unwrap();
    assert_eq!(invite.created_at, created_at);
    assert_eq!(invite.expires_at, expires_at);
    assert_eq!(invite.used_at, Some(used_at));
}

async fn set_invite_used_at(pool: &CloseablePool, code: &InviteCode, used_at: UtcInstant) {
    storage::with_closeable_pool!(pool, pool, {
        sqlx::query("UPDATE invites SET used_at = $1 WHERE code = $2")
            .bind(used_at)
            .bind(code)
            .execute(pool)
            .await
            .unwrap();
    });
}

// --- create_user_with_invite integration tests ---

#[apply(backends)]
#[tokio::test]
async fn create_user_with_invite_creates_user_and_marks_invite_used(#[case] backend: Backend) {
    let env = backend.setup().await;
    let state = &env.state;

    let expires_at: UtcInstant = "2099-01-02T03:04:05.123457Z".parse().unwrap();
    let code = create_invite(state, expires_at).await;
    let user_id = create_user_with_invite(
        state,
        username("alice"),
        password("password123"),
        Some(parse_display_name("Alice")),
        storage::OperatorStatus::STANDARD,
        code.clone(),
    )
    .await;

    let record = state.users.get_user(user_id).await.unwrap().unwrap();
    assert_eq!(record.username, "alice");
    assert_eq!(record.display_name.as_deref(), Some("Alice"));

    let list = state.invites.list_invites().await.unwrap();
    assert_eq!(list.len(), 1);
    assert!(list[0].used_at.is_some());
    assert_eq!(list[0].used_by, Some(user_id));
}

#[apply(backends)]
#[tokio::test]
async fn create_user_with_invite_second_call_returns_already_used(#[case] backend: Backend) {
    let env = backend.setup().await;
    let state = &env.state;

    let expires_at: UtcInstant = "2099-01-02T03:04:05.123457Z".parse().unwrap();
    let code = create_invite(state, expires_at).await;

    create_user_with_invite(
        state,
        username("alice"),
        password("password123"),
        None,
        storage::OperatorStatus::STANDARD,
        code.clone(),
    )
    .await;

    let err = create_user_with_invite_result(
        state,
        username("bob"),
        password("password123"),
        None,
        storage::OperatorStatus::STANDARD,
        code,
    )
    .await
    .unwrap_err();
    let WriteScopeError::Operation(err) = err else {
        unreachable!("expected invite registration operation error, got {err:?}");
    };

    assert!(matches!(err, RegisterWithInviteError::InviteAlreadyUsed));

    assert!(
        state
            .users
            .get_user_by_username(&username("bob"))
            .await
            .unwrap()
            .is_none()
    );
}

#[apply(backends)]
#[tokio::test]
async fn concurrent_registrations_claim_exactly_one_invite(#[case] backend: Backend) {
    let env = backend.setup().await;
    let state = Arc::clone(&env.state);
    let code = create_invite(
        &state,
        "2099-01-02T03:04:05.123457Z".parse::<UtcInstant>().unwrap(),
    )
    .await;
    let start_barrier = Arc::new(tokio::sync::Barrier::new(2));

    let first = tokio::spawn(register_after_start_barrier(
        Arc::clone(&state),
        Arc::clone(&start_barrier),
        code.clone(),
        username("alice"),
        password("alice-password"),
    ));
    let second = tokio::spawn(register_after_start_barrier(
        Arc::clone(&state),
        start_barrier,
        code,
        username("bob"),
        password("bob-password"),
    ));
    let (first, second) = tokio::time::timeout(Duration::from_secs(20), async {
        tokio::join!(first, second)
    })
    .await
    .expect("concurrent registrations must finish");

    assert_exactly_one_invite_registration(
        &state,
        first.expect("first concurrent registration task must not panic"),
        second.expect("second concurrent registration task must not panic"),
    )
    .await;
}

#[apply(backends)]
#[tokio::test]
async fn create_user_with_invite_expired_returns_invite_expired(#[case] backend: Backend) {
    let env = backend.setup().await;
    let state = &env.state;

    let expires_at: UtcInstant = "2000-01-02T03:04:05.123455Z".parse().unwrap();
    let code = create_invite(state, expires_at).await;

    let err = create_user_with_invite_result(
        state,
        username("alice"),
        password("password123"),
        None,
        storage::OperatorStatus::STANDARD,
        code,
    )
    .await
    .unwrap_err();
    let WriteScopeError::Operation(err) = err else {
        unreachable!("expected invite registration operation error, got {err:?}");
    };

    assert!(matches!(err, RegisterWithInviteError::InviteExpired));

    assert!(
        state
            .users
            .get_user_by_username(&username("alice"))
            .await
            .unwrap()
            .is_none()
    );
}

#[apply(backends)]
#[tokio::test]
async fn create_user_with_invite_unknown_code_returns_not_found(#[case] backend: Backend) {
    let env = backend.setup().await;
    let state = &env.state;

    let err = create_user_with_invite_result(
        state,
        username("alice"),
        password("password123"),
        None,
        storage::OperatorStatus::STANDARD,
        "no-such-code".parse().unwrap(),
    )
    .await
    .unwrap_err();
    let WriteScopeError::Operation(err) = err else {
        unreachable!("expected invite registration operation error, got {err:?}");
    };

    assert!(matches!(err, RegisterWithInviteError::InviteNotFound));

    assert!(
        state
            .users
            .get_user_by_username(&username("alice"))
            .await
            .unwrap()
            .is_none()
    );
}

#[apply(backends)]
#[tokio::test]
async fn create_user_with_invite_duplicate_username_returns_username_taken(
    #[case] backend: Backend,
) {
    let env = backend.setup().await;
    let state = &env.state;

    // alice exists before the invite is used
    let user = SeedUser::new().seed(state).await;

    let expires_at = UtcInstant::from(Utc::now() + chrono::Duration::hours(24));
    let code = create_invite(state, expires_at).await;

    let err = create_user_with_invite_result(
        state,
        user.username.clone(),
        password("other_password"),
        None,
        storage::OperatorStatus::STANDARD,
        code,
    )
    .await
    .unwrap_err();
    let WriteScopeError::Operation(err) = err else {
        unreachable!("expected invite registration operation error, got {err:?}");
    };

    assert!(matches!(err, RegisterWithInviteError::UsernameTaken));

    // A failed registration must not consume the invite.
    let list = state.invites.list_invites().await.unwrap();
    assert_eq!(list.len(), 1);
    assert!(list[0].used_at.is_none());
}
#[apply(backends)]
#[tokio::test]
async fn create_user_with_invite_hash_failure_preserves_password_error_and_invite(
    #[case] backend: Backend,
) {
    let env = backend.setup().await;
    let state = &env.state;
    let code = create_invite(
        state,
        "2099-01-02T03:04:05.123457Z".parse::<UtcInstant>().unwrap(),
    )
    .await;

    let error = create_user_with_invite_result(
        state,
        username("alice"),
        password("force-hash-error-for-test-coverage"),
        None,
        OperatorStatus::STANDARD,
        code,
    )
    .await
    .expect_err("a forced password hash failure must reject registration");
    let WriteScopeError::Operation(RegisterWithInviteError::Internal(sqlx::Error::Io(source))) =
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

    let invite = state.invites.list_invites().await.unwrap().pop().unwrap();
    assert!(invite.used_at.is_none());
    assert!(invite.used_by.is_none());
    assert!(
        state
            .users
            .get_user_by_username(&username("alice"))
            .await
            .unwrap()
            .is_none()
    );
}

#[apply(backends)]
#[tokio::test]
async fn invite_list_operations(#[case] backend: Backend) {
    let env = backend.setup().await;
    let state = &env.state;
    let now = UtcInstant::now();
    let future = UtcInstant::from(now.value() + chrono::Duration::hours(1));
    let past = UtcInstant::from(now.value() - chrono::Duration::hours(1));

    let _invite1 = create_invite(state, future).await;

    let _invite2 = create_invite(state, past).await;

    let invites = state
        .invites
        .list_invites()
        .await
        .expect("list_invites failed");

    assert!(invites.len() >= 2);

    let unused_count = invites.iter().filter(|i| i.used_at.is_none()).count();
    assert!(unused_count >= 2);
}

pub(super) async fn create_invite(state: &AppState, expires_at: UtcInstant) -> InviteCode {
    let invites = Arc::clone(&state.invites);
    let outcome = state
        .write_scope
        .run(|transaction| {
            Box::pin(async move { invites.create_invite(transaction, expires_at).await })
        })
        .await
        .expect("invite fixture setup should succeed");
    confirmed_for(outcome, "invite fixture setup")
}

async fn create_user_with_invite(
    state: &AppState,
    username: common::username::Username,
    password: host::password::Password,
    display_name: Option<common::display_name::DisplayName>,
    is_operator: OperatorStatus,
    code: InviteCode,
) -> common::ids::UserId {
    let outcome =
        create_user_with_invite_result(state, username, password, display_name, is_operator, code)
            .await
            .expect("invite registration should succeed");
    confirmed_for(outcome, "invite registration")
}

async fn create_user_with_invite_result(
    state: &AppState,
    username: common::username::Username,
    password: host::password::Password,
    display_name: Option<common::display_name::DisplayName>,
    is_operator: OperatorStatus,
    code: InviteCode,
) -> Result<MutationOutcome<common::ids::UserId>, WriteScopeError<RegisterWithInviteError>> {
    let users = Arc::clone(&state.users);
    let invites = Arc::clone(&state.invites);
    state
        .write_scope
        .run(|transaction| {
            Box::pin(async move {
                account_mutations::register_with_invite(
                    transaction,
                    users.as_ref(),
                    invites.as_ref(),
                    RegisterWithInviteInput {
                        username: &username,
                        password: &password,
                        display_name: display_name.as_ref(),
                        is_operator,
                        invite_code: &code,
                    },
                )
                .await
            })
        })
        .await
}

async fn register_after_start_barrier(
    state: Arc<AppState>,
    start_barrier: Arc<tokio::sync::Barrier>,
    code: InviteCode,
    username: common::username::Username,
    password: host::password::Password,
) -> Result<MutationOutcome<common::ids::UserId>, WriteScopeError<RegisterWithInviteError>> {
    start_barrier.wait().await;
    create_user_with_invite_result(
        &state,
        username,
        password,
        None,
        OperatorStatus::STANDARD,
        code,
    )
    .await
}

pub(super) async fn assert_exactly_one_invite_registration(
    state: &AppState,
    first: Result<MutationOutcome<common::ids::UserId>, WriteScopeError<RegisterWithInviteError>>,
    second: Result<MutationOutcome<common::ids::UserId>, WriteScopeError<RegisterWithInviteError>>,
) {
    let winner = match (first, second) {
        (
            Ok(outcome),
            Err(WriteScopeError::Operation(RegisterWithInviteError::InviteAlreadyUsed)),
        )
        | (
            Err(WriteScopeError::Operation(RegisterWithInviteError::InviteAlreadyUsed)),
            Ok(outcome),
        ) => storage::test_support::confirmed_for(outcome, "winning concurrent registration"),
        (first, second) => panic!(
            "expected one confirmed registration and one InviteAlreadyUsed, got {first:?} and {second:?}"
        ),
    };

    let invite = state.invites.list_invites().await.unwrap().pop().unwrap();
    assert_eq!(invite.used_by, Some(winner));
    let alice = state
        .users
        .get_user_by_username(&username("alice"))
        .await
        .unwrap();
    let bob = state
        .users
        .get_user_by_username(&username("bob"))
        .await
        .unwrap();
    match (alice, bob) {
        (Some(alice), None) => assert_eq!(alice.user_id, winner),
        (None, Some(bob)) => assert_eq!(bob.user_id, winner),
        (alice, bob) => panic!(
            "only the winning registration user must persist, found alice={alice:?}, bob={bob:?}"
        ),
    }
}
