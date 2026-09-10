use std::{sync::Arc, time::Duration};

use async_trait::async_trait;
use common::{
    MutationOutcome, ids::UserId, test_support::parse_session_label, time::UtcInstant,
    token::RawToken,
};
use host::password::Password;
use rstest::*;
use rstest_reuse::*;
use storage::test_support::{Backend, SeedUser, postgres_only};
use storage::{
    PasswordResetStorage, SessionStorage, UsePasswordResetError, UserAuthError, UserStorage,
    WriteScope, WriteScopeError, WriteTransaction,
    account_mutations::{self, ConfirmPasswordResetError},
};

use super::super::fixtures::password;

struct BarrierPasswordResetStorage {
    inner: Arc<dyn PasswordResetStorage>,
    claim_barrier: Arc<tokio::sync::Barrier>,
}

#[async_trait]
impl PasswordResetStorage for BarrierPasswordResetStorage {
    async fn create_password_reset(
        &self,
        transaction: &mut WriteTransaction,
        user_id: UserId,
        expires_at: UtcInstant,
    ) -> sqlx::Result<RawToken> {
        self.inner
            .create_password_reset(transaction, user_id, expires_at)
            .await
    }

    async fn use_password_reset(
        &self,
        transaction: &mut WriteTransaction,
        raw_token: &RawToken,
    ) -> Result<UserId, UsePasswordResetError> {
        self.claim_barrier.wait().await;
        self.inner.use_password_reset(transaction, raw_token).await
    }

    async fn prune_password_resets(&self, now: UtcInstant) -> sqlx::Result<u64> {
        self.inner.prune_password_resets(now).await
    }
}

#[apply(postgres_only)]
// reason: SQLite serializes write transactions, so only Postgres can exercise competing reset claims.
#[tokio::test]
async fn concurrent_password_reset_confirmations_claim_exactly_once(#[case] backend: Backend) {
    let env = backend.setup().await;
    let users = env.users();
    let sessions = env.sessions();
    let reset_storage = env.password_resets();
    let write_scope = env.write_scope();
    let user = SeedUser::new()
        .seed(Arc::clone(&users), write_scope.clone())
        .await;
    let raw_token = create_password_reset(
        Arc::clone(&reset_storage),
        write_scope.clone(),
        user.user_id,
    )
    .await;
    create_session(Arc::clone(&sessions), write_scope.clone(), user.user_id).await;

    let password_resets: Arc<dyn PasswordResetStorage> = Arc::new(BarrierPasswordResetStorage {
        inner: Arc::clone(&reset_storage),
        claim_barrier: Arc::new(tokio::sync::Barrier::new(2)),
    });
    let first = tokio::spawn(confirm_after_claim_barrier(
        Arc::clone(&password_resets),
        Arc::clone(&users),
        Arc::clone(&sessions),
        write_scope.clone(),
        raw_token.clone(),
        password("first-new-password"),
    ));
    let second = tokio::spawn(confirm_after_claim_barrier(
        Arc::clone(&password_resets),
        Arc::clone(&users),
        Arc::clone(&sessions),
        write_scope.clone(),
        raw_token.clone(),
        password("second-new-password"),
    ));
    let (first, second) = tokio::time::timeout(Duration::from_secs(20), async {
        tokio::join!(first, second)
    })
    .await
    .expect("concurrent reset confirmations must finish");
    let first = first.expect("first concurrent reset task must not panic");
    let second = second.expect("second concurrent reset task must not panic");

    let (winner_password, loser_password) = match (first, second) {
        (Ok(outcome), Err(WriteScopeError::Operation(ConfirmPasswordResetError::AlreadyUsed))) => {
            storage::test_support::confirmed_for(outcome, "winning concurrent reset");
            ("first-new-password", "second-new-password")
        }
        (Err(WriteScopeError::Operation(ConfirmPasswordResetError::AlreadyUsed)), Ok(outcome)) => {
            storage::test_support::confirmed_for(outcome, "winning concurrent reset");
            ("second-new-password", "first-new-password")
        }
        (first, second) => {
            panic!("expected one confirmed reset and one AlreadyUsed, got {first:?} and {second:?}")
        }
    };

    let token_error =
        use_password_reset_result(Arc::clone(&reset_storage), write_scope.clone(), raw_token)
            .await
            .expect_err("the winning confirmation must consume the reset token");
    assert!(matches!(
        token_error,
        WriteScopeError::Operation(UsePasswordResetError::AlreadyUsed)
    ));
    let authenticated = storage::test_support::confirmed_for(
        authenticate_result(
            Arc::clone(&users),
            write_scope.clone(),
            user.username.clone(),
            password(winner_password),
        )
        .await
        .unwrap(),
        "winning password authentication",
    );
    assert_eq!(authenticated.user_id, user.user_id);
    assert!(matches!(
        authenticate_result(
            Arc::clone(&users),
            write_scope.clone(),
            user.username,
            password(loser_password),
        )
        .await,
        Err(WriteScopeError::Operation(
            UserAuthError::InvalidCredentials
        ))
    ));
    assert!(
        sessions
            .list_sessions(user.user_id)
            .await
            .unwrap()
            .is_empty(),
        "the winning reset must atomically revoke pre-existing sessions"
    );
}

async fn create_password_reset(
    password_resets: Arc<dyn PasswordResetStorage>,
    write_scope: WriteScope,
    user_id: UserId,
) -> RawToken {
    let outcome = write_scope
        .run(|transaction| {
            Box::pin(async move {
                password_resets
                    .create_password_reset(
                        transaction,
                        user_id,
                        "2099-01-02T03:04:05.123456Z".parse().unwrap(),
                    )
                    .await
            })
        })
        .await
        .expect("password-reset fixture setup should succeed");
    storage::test_support::confirmed_for(outcome, "password-reset fixture setup")
}

async fn create_session(
    sessions: Arc<dyn SessionStorage>,
    write_scope: WriteScope,
    user_id: UserId,
) {
    let label = parse_session_label("Existing device");
    let outcome = write_scope
        .run(|transaction| {
            Box::pin(async move { sessions.create_session(transaction, user_id, &label).await })
        })
        .await
        .expect("session fixture setup should succeed");
    storage::test_support::confirmed_for(outcome, "session fixture setup");
}

async fn confirm_after_claim_barrier(
    password_resets: Arc<dyn PasswordResetStorage>,
    users: Arc<dyn UserStorage>,
    sessions: Arc<dyn SessionStorage>,
    write_scope: WriteScope,
    raw_token: RawToken,
    new_password: Password,
) -> Result<MutationOutcome<UserId>, WriteScopeError<ConfirmPasswordResetError>> {
    write_scope
        .run(|transaction| {
            Box::pin(async move {
                account_mutations::confirm_password_reset(
                    transaction,
                    password_resets.as_ref(),
                    users.as_ref(),
                    sessions.as_ref(),
                    &raw_token,
                    &new_password,
                )
                .await
            })
        })
        .await
}

async fn use_password_reset_result(
    password_resets: Arc<dyn PasswordResetStorage>,
    write_scope: WriteScope,
    raw_token: RawToken,
) -> Result<MutationOutcome<UserId>, WriteScopeError<UsePasswordResetError>> {
    write_scope
        .run(|transaction| {
            Box::pin(async move {
                password_resets
                    .use_password_reset(transaction, &raw_token)
                    .await
            })
        })
        .await
}

async fn authenticate_result(
    users: Arc<dyn UserStorage>,
    write_scope: WriteScope,
    username: common::username::Username,
    password: Password,
) -> Result<MutationOutcome<storage::UserRecord>, WriteScopeError<UserAuthError>> {
    let authentication = users
        .prepare_authentication(&username, &password)
        .await
        .map_err(WriteScopeError::Operation)?;
    write_scope
        .run(|transaction| {
            Box::pin(async move { users.authenticate(transaction, authentication).await })
        })
        .await
}
