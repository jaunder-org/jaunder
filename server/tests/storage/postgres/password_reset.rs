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
    AppState, PasswordResetStorage, UsePasswordResetError, UserAuthError, WriteScopeError,
    WriteTransaction,
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
}

#[apply(postgres_only)]
// reason: SQLite serializes write transactions, so only Postgres can exercise competing reset claims.
#[tokio::test]
async fn concurrent_password_reset_confirmations_claim_exactly_once(#[case] backend: Backend) {
    let env = backend.setup().await;
    let state = Arc::clone(&env.state);
    let user = SeedUser::new().seed(&state).await;
    let raw_token = create_password_reset(&state, user.user_id).await;
    create_session(&state, user.user_id).await;

    let password_resets: Arc<dyn PasswordResetStorage> = Arc::new(BarrierPasswordResetStorage {
        inner: Arc::clone(&state.password_resets),
        claim_barrier: Arc::new(tokio::sync::Barrier::new(2)),
    });
    let first = tokio::spawn(confirm_after_claim_barrier(
        Arc::clone(&state),
        Arc::clone(&password_resets),
        raw_token.clone(),
        password("first-new-password"),
    ));
    let second = tokio::spawn(confirm_after_claim_barrier(
        Arc::clone(&state),
        password_resets,
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

    let token_error = use_password_reset_result(&state, raw_token)
        .await
        .expect_err("the winning confirmation must consume the reset token");
    assert!(matches!(
        token_error,
        WriteScopeError::Operation(UsePasswordResetError::AlreadyUsed)
    ));
    let authenticated = storage::test_support::confirmed_for(
        authenticate_result(&state, user.username.clone(), password(winner_password))
            .await
            .unwrap(),
        "winning password authentication",
    );
    assert_eq!(authenticated.user_id, user.user_id);
    assert!(matches!(
        authenticate_result(&state, user.username, password(loser_password)).await,
        Err(WriteScopeError::Operation(
            UserAuthError::InvalidCredentials
        ))
    ));
    assert!(
        state
            .sessions
            .list_sessions(user.user_id)
            .await
            .unwrap()
            .is_empty(),
        "the winning reset must atomically revoke pre-existing sessions"
    );
}

async fn create_password_reset(state: &AppState, user_id: UserId) -> RawToken {
    let password_resets = Arc::clone(&state.password_resets);
    let outcome = state
        .write_scope
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

async fn create_session(state: &AppState, user_id: UserId) {
    let sessions = Arc::clone(&state.sessions);
    let label = parse_session_label("Existing device");
    let outcome = state
        .write_scope
        .run(|transaction| {
            Box::pin(async move { sessions.create_session(transaction, user_id, &label).await })
        })
        .await
        .expect("session fixture setup should succeed");
    storage::test_support::confirmed_for(outcome, "session fixture setup");
}

async fn confirm_after_claim_barrier(
    state: Arc<AppState>,
    password_resets: Arc<dyn PasswordResetStorage>,
    raw_token: RawToken,
    new_password: Password,
) -> Result<MutationOutcome<()>, WriteScopeError<ConfirmPasswordResetError>> {
    let users = Arc::clone(&state.users);
    let sessions = Arc::clone(&state.sessions);
    state
        .write_scope
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
    state: &AppState,
    raw_token: RawToken,
) -> Result<MutationOutcome<UserId>, WriteScopeError<UsePasswordResetError>> {
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

async fn authenticate_result(
    state: &AppState,
    username: common::username::Username,
    password: Password,
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
