use chrono::Utc;
use common::test_support::parse_display_name;
use common::time::UtcInstant;
use host::invite::InviteCode;
use rstest::*;
use rstest_reuse::*;
use storage::RegisterWithInviteError;
use storage::test_support::{Backend, CloseablePool, SeedUser, backends};

use crate::storage::fixtures::{password, username};
#[apply(backends)]
#[tokio::test]
async fn create_invite_and_list_invites_includes_it(#[case] backend: Backend) {
    let env = backend.setup().await;
    let state = &env.state;

    let expires_at = UtcInstant::from(Utc::now() + chrono::Duration::hours(24));
    let code = state.invites.create_invite(expires_at).await.unwrap();

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
    let code = state.invites.create_invite(expires_at).await.unwrap();

    let user_id = state
        .atomic
        .create_user_with_invite(
            &username("alice"),
            &password("password123"),
            Some(&parse_display_name("Alice")),
            false,
            &code,
        )
        .await
        .unwrap();

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
    let code = state.invites.create_invite(expires_at).await.unwrap();

    state
        .atomic
        .create_user_with_invite(
            &username("alice"),
            &password("password123"),
            None,
            false,
            &code,
        )
        .await
        .unwrap();

    let err = state
        .atomic
        .create_user_with_invite(
            &username("bob"),
            &password("password123"),
            None,
            false,
            &code,
        )
        .await
        .unwrap_err();

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
async fn create_user_with_invite_expired_returns_invite_expired(#[case] backend: Backend) {
    let env = backend.setup().await;
    let state = &env.state;

    let expires_at: UtcInstant = "2000-01-02T03:04:05.123455Z".parse().unwrap();
    let code = state.invites.create_invite(expires_at).await.unwrap();

    let err = state
        .atomic
        .create_user_with_invite(
            &username("alice"),
            &password("password123"),
            None,
            false,
            &code,
        )
        .await
        .unwrap_err();

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

    let err = state
        .atomic
        .create_user_with_invite(
            &username("alice"),
            &password("password123"),
            None,
            false,
            &"no-such-code".parse::<InviteCode>().unwrap(),
        )
        .await
        .unwrap_err();

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
    let code = state.invites.create_invite(expires_at).await.unwrap();

    let err = state
        .atomic
        .create_user_with_invite(
            &user.username,
            &password("other_password"),
            None,
            false,
            &code,
        )
        .await
        .unwrap_err();

    assert!(matches!(err, RegisterWithInviteError::UsernameTaken));

    // A failed registration must not consume the invite.
    let list = state.invites.list_invites().await.unwrap();
    assert_eq!(list.len(), 1);
    assert!(list[0].used_at.is_none());
}
#[apply(backends)]
#[tokio::test]
async fn invite_list_operations(#[case] backend: Backend) {
    let env = backend.setup().await;
    let state = &env.state;
    let now = UtcInstant::now();
    let future = UtcInstant::from(now.value() + chrono::Duration::hours(1));
    let past = UtcInstant::from(now.value() - chrono::Duration::hours(1));

    let _invite1 = state
        .invites
        .create_invite(future)
        .await
        .expect("create_invite 1 failed");

    let _invite2 = state
        .invites
        .create_invite(past)
        .await
        .expect("create_invite 2 failed");

    let invites = state
        .invites
        .list_invites()
        .await
        .expect("list_invites failed");

    assert!(invites.len() >= 2);

    let unused_count = invites.iter().filter(|i| i.used_at.is_none()).count();
    assert!(unused_count >= 2);
}
