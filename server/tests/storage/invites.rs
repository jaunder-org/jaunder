use std::{sync::Arc, time::Duration};

use crate::storage::fixtures::{password, username};
use common::MutationOutcome;
use common::test_support::parse_display_name;
use common::time::UtcInstant;
use host::invite::InviteCode;
use jiff::ToSpan;
use rstest::*;
use rstest_reuse::*;
use storage::test_support::{Backend, CloseablePool, SeedUser, backends, confirmed_for};
use storage::{
    InviteStorage, OperatorStatus, UserStorage, WriteScope, WriteScopeError,
    account_mutations::{self, RegisterWithInviteError, RegisterWithInviteInput},
};
#[apply(backends)]
#[tokio::test]
async fn create_invite_and_list_invites_includes_it(#[case] backend: Backend) {
    let env = backend.setup().await;

    let expires_at = UtcInstant::from(
        UtcInstant::now()
            .value()
            .checked_add(24.hours())
            .expect("fixture is within Timestamp range"),
    );
    let code = create_invite(env.invites(), env.write_scope(), expires_at).await;

    let list = env.invites().list_invites().await.unwrap();
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

    let invite = env.invites().list_invites().await.unwrap().pop().unwrap();
    assert_eq!(invite.created_at, created_at);
    assert_eq!(invite.expires_at, expires_at);
    assert!(invite.used_at.is_none());

    set_invite_used_at(env.base.pool(), &code, used_at).await;

    let invite = env.invites().list_invites().await.unwrap().pop().unwrap();
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

    let expires_at: UtcInstant = "2099-01-02T03:04:05.123457Z".parse().unwrap();
    let code = create_invite(env.invites(), env.write_scope(), expires_at).await;
    let user_id = create_user_with_invite(
        env.users(),
        env.invites(),
        env.write_scope(),
        InviteRegistration {
            username: username("alice"),
            password: password("password123"),
            display_name: Some(parse_display_name("Alice")),
            is_operator: OperatorStatus::STANDARD,
            code: code.clone(),
        },
    )
    .await;

    let record = env.users().get_user(user_id).await.unwrap().unwrap();
    assert_eq!(record.username, "alice");
    assert_eq!(record.display_name.as_deref(), Some("Alice"));

    let list = env.invites().list_invites().await.unwrap();
    assert_eq!(list.len(), 1);
    assert!(list[0].used_at.is_some());
    assert_eq!(list[0].used_by, Some(user_id));
}

#[apply(backends)]
#[tokio::test]
async fn create_user_with_invite_second_call_returns_already_used(#[case] backend: Backend) {
    let env = backend.setup().await;

    let expires_at: UtcInstant = "2099-01-02T03:04:05.123457Z".parse().unwrap();
    let code = create_invite(env.invites(), env.write_scope(), expires_at).await;

    create_user_with_invite(
        env.users(),
        env.invites(),
        env.write_scope(),
        InviteRegistration::standard(username("alice"), password("password123"), code.clone()),
    )
    .await;

    let err = create_user_with_invite_result(
        env.users(),
        env.invites(),
        env.write_scope(),
        InviteRegistration::standard(username("bob"), password("password123"), code),
    )
    .await
    .unwrap_err();
    let WriteScopeError::Operation(err) = err else {
        unreachable!("expected invite registration operation error, got {err:?}");
    };

    assert!(matches!(err, RegisterWithInviteError::InviteAlreadyUsed));

    assert!(
        env.users()
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
    let invites = Arc::clone(&env.invites());
    let users = Arc::clone(&env.users());
    let write_scope = env.write_scope();
    let code = create_invite(
        Arc::clone(&invites),
        write_scope.clone(),
        "2099-01-02T03:04:05.123457Z".parse::<UtcInstant>().unwrap(),
    )
    .await;
    let start_barrier = Arc::new(tokio::sync::Barrier::new(2));

    let first = tokio::spawn(register_after_start_barrier(
        Arc::clone(&users),
        Arc::clone(&invites),
        write_scope.clone(),
        Arc::clone(&start_barrier),
        code.clone(),
        username("alice"),
        password("alice-password"),
    ));
    let second = tokio::spawn(register_after_start_barrier(
        users,
        invites,
        write_scope,
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
        Arc::clone(&env.invites()),
        Arc::clone(&env.users()),
        first.expect("first concurrent registration task must not panic"),
        second.expect("second concurrent registration task must not panic"),
    )
    .await;
}

#[apply(backends)]
#[tokio::test]
async fn create_user_with_invite_expired_returns_invite_expired(#[case] backend: Backend) {
    let env = backend.setup().await;

    let expires_at: UtcInstant = "2000-01-02T03:04:05.123455Z".parse().unwrap();
    let code = create_invite(env.invites(), env.write_scope(), expires_at).await;

    let err = create_user_with_invite_result(
        env.users(),
        env.invites(),
        env.write_scope(),
        InviteRegistration::standard(username("alice"), password("password123"), code),
    )
    .await
    .unwrap_err();
    let WriteScopeError::Operation(err) = err else {
        unreachable!("expected invite registration operation error, got {err:?}");
    };

    assert!(matches!(err, RegisterWithInviteError::InviteExpired));

    assert!(
        env.users()
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

    let err = create_user_with_invite_result(
        env.users(),
        env.invites(),
        env.write_scope(),
        InviteRegistration::standard(
            username("alice"),
            password("password123"),
            "no-such-code".parse().unwrap(),
        ),
    )
    .await
    .unwrap_err();
    let WriteScopeError::Operation(err) = err else {
        unreachable!("expected invite registration operation error, got {err:?}");
    };

    assert!(matches!(err, RegisterWithInviteError::InviteNotFound));

    assert!(
        env.users()
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

    // alice exists before the invite is used
    let user = SeedUser::new().seed(env.users(), env.write_scope()).await;

    let expires_at = UtcInstant::from(
        UtcInstant::now()
            .value()
            .checked_add(24.hours())
            .expect("fixture is within Timestamp range"),
    );
    let code = create_invite(env.invites(), env.write_scope(), expires_at).await;

    let err = create_user_with_invite_result(
        env.users(),
        env.invites(),
        env.write_scope(),
        InviteRegistration::standard(user.username.clone(), password("other_password"), code),
    )
    .await
    .unwrap_err();
    let WriteScopeError::Operation(err) = err else {
        unreachable!("expected invite registration operation error, got {err:?}");
    };

    assert!(matches!(err, RegisterWithInviteError::UsernameTaken));

    // A failed registration must not consume the invite.
    let list = env.invites().list_invites().await.unwrap();
    assert_eq!(list.len(), 1);
    assert!(list[0].used_at.is_none());
}
#[apply(backends)]
#[tokio::test]
async fn create_user_with_invite_hash_failure_preserves_password_error_and_invite(
    #[case] backend: Backend,
) {
    let env = backend.setup().await;
    let code = create_invite(
        env.invites(),
        env.write_scope(),
        "2099-01-02T03:04:05.123457Z".parse::<UtcInstant>().unwrap(),
    )
    .await;

    let error = create_user_with_invite_result(
        env.users(),
        env.invites(),
        env.write_scope(),
        InviteRegistration::standard(
            username("alice"),
            password("force-hash-error-for-test-coverage"),
            code,
        ),
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

    let invite = env.invites().list_invites().await.unwrap().pop().unwrap();
    assert!(invite.used_at.is_none());
    assert!(invite.used_by.is_none());
    assert!(
        env.users()
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
    let now = UtcInstant::now();
    let future = UtcInstant::from(
        now.value()
            .checked_add(1.hour())
            .expect("fixture is within Timestamp range"),
    );
    let past = UtcInstant::from(
        now.value()
            .checked_sub(1.hour())
            .expect("fixture is within Timestamp range"),
    );

    let _invite1 = create_invite(env.invites(), env.write_scope(), future).await;

    let _invite2 = create_invite(env.invites(), env.write_scope(), past).await;

    let invites = env
        .invites()
        .list_invites()
        .await
        .expect("list_invites failed");

    assert!(invites.len() >= 2);

    let unused_count = invites.iter().filter(|i| i.used_at.is_none()).count();
    assert!(unused_count >= 2);
}

pub(super) async fn create_invite(
    invites: Arc<dyn InviteStorage>,
    write_scope: WriteScope,
    expires_at: UtcInstant,
) -> InviteCode {
    let outcome = write_scope
        .run(|transaction| {
            Box::pin(async move { invites.create_invite(transaction, expires_at).await })
        })
        .await
        .expect("invite fixture setup should succeed");
    confirmed_for(outcome, "invite fixture setup")
}

struct InviteRegistration {
    username: common::username::Username,
    password: host::password::Password,
    display_name: Option<common::display_name::DisplayName>,
    is_operator: OperatorStatus,
    code: InviteCode,
}

impl InviteRegistration {
    fn standard(
        username: common::username::Username,
        password: host::password::Password,
        code: InviteCode,
    ) -> Self {
        Self {
            username,
            password,
            display_name: None,
            is_operator: OperatorStatus::STANDARD,
            code,
        }
    }
}

async fn create_user_with_invite(
    users: Arc<dyn UserStorage>,
    invites: Arc<dyn InviteStorage>,
    write_scope: WriteScope,
    registration: InviteRegistration,
) -> common::ids::UserId {
    let outcome = create_user_with_invite_result(users, invites, write_scope, registration)
        .await
        .expect("invite registration should succeed");
    confirmed_for(outcome, "invite registration")
}

async fn create_user_with_invite_result(
    users: Arc<dyn UserStorage>,
    invites: Arc<dyn InviteStorage>,
    write_scope: WriteScope,
    registration: InviteRegistration,
) -> Result<MutationOutcome<common::ids::UserId>, WriteScopeError<RegisterWithInviteError>> {
    write_scope
        .run(|transaction| {
            Box::pin(async move {
                account_mutations::register_with_invite(
                    transaction,
                    users.as_ref(),
                    invites.as_ref(),
                    RegisterWithInviteInput {
                        username: &registration.username,
                        password: &registration.password,
                        display_name: registration.display_name.as_ref(),
                        is_operator: registration.is_operator,
                        invite_code: &registration.code,
                    },
                )
                .await
            })
        })
        .await
}

async fn register_after_start_barrier(
    users: Arc<dyn UserStorage>,
    invites: Arc<dyn InviteStorage>,
    write_scope: WriteScope,
    start_barrier: Arc<tokio::sync::Barrier>,
    code: InviteCode,
    username: common::username::Username,
    password: host::password::Password,
) -> Result<MutationOutcome<common::ids::UserId>, WriteScopeError<RegisterWithInviteError>> {
    start_barrier.wait().await;
    create_user_with_invite_result(
        users,
        invites,
        write_scope,
        InviteRegistration::standard(username, password, code),
    )
    .await
}

pub(super) async fn assert_exactly_one_invite_registration(
    invites: Arc<dyn InviteStorage>,
    users: Arc<dyn UserStorage>,
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
        ) => confirmed_for(outcome, "winning concurrent registration"),
        (first, second) => panic!(
            "expected one confirmed registration and one InviteAlreadyUsed, got {first:?} and {second:?}"
        ),
    };

    let invite = invites.list_invites().await.unwrap().pop().unwrap();
    assert_eq!(invite.used_by, Some(winner));
    let alice = users
        .get_user_by_username(&username("alice"))
        .await
        .unwrap();
    let bob = users.get_user_by_username(&username("bob")).await.unwrap();
    match (alice, bob) {
        (Some(alice), None) => assert_eq!(alice.user_id, winner),
        (None, Some(bob)) => assert_eq!(bob.user_id, winner),
        (alice, bob) => panic!(
            "only the winning registration user must persist, found alice={alice:?}, bob={bob:?}"
        ),
    }
}
