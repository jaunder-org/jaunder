use std::{sync::Arc, time::Duration};

use async_trait::async_trait;
use common::{ids::UserId, time::UtcInstant, username::Username};
use host::{invite::InviteCode, password::Password};
use rstest::*;
use rstest_reuse::*;
use storage::test_support::{Backend, postgres_only};
use storage::{
    AppState, InviteRecord, InviteStorage, UseInviteError, WriteScopeError, WriteTransaction,
    account_mutations::{self, RegisterWithInviteError, RegisterWithInviteInput},
};

use super::super::{
    fixtures::{password, username},
    invites::create_invite,
};

struct BarrierInviteStorage {
    inner: Arc<dyn InviteStorage>,
    claim_barrier: Arc<tokio::sync::Barrier>,
}

#[async_trait]
impl InviteStorage for BarrierInviteStorage {
    async fn create_invite(
        &self,
        transaction: &mut WriteTransaction,
        expires_at: UtcInstant,
    ) -> sqlx::Result<InviteCode> {
        self.inner.create_invite(transaction, expires_at).await
    }

    async fn precheck_invite(&self, code: &InviteCode) -> Result<(), UseInviteError> {
        self.inner.precheck_invite(code).await
    }

    async fn claim_invite(
        &self,
        transaction: &mut WriteTransaction,
        code: &InviteCode,
        user_id: UserId,
    ) -> Result<(), UseInviteError> {
        self.claim_barrier.wait().await;
        self.inner.claim_invite(transaction, code, user_id).await
    }

    async fn list_invites(&self) -> sqlx::Result<Vec<InviteRecord>> {
        self.inner.list_invites().await
    }
}

#[apply(postgres_only)]
// reason: forces both registrations through user insertion before competing to claim one invite.
#[tokio::test]
async fn concurrent_registrations_claim_exactly_one_invite(#[case] backend: Backend) {
    let env = backend.setup().await;
    let state = Arc::clone(&env.state);
    let code = create_invite(
        &state,
        "2099-01-02T03:04:05.123457Z".parse::<UtcInstant>().unwrap(),
    )
    .await;
    let invites: Arc<dyn InviteStorage> = Arc::new(BarrierInviteStorage {
        inner: Arc::clone(&state.invites),
        claim_barrier: Arc::new(tokio::sync::Barrier::new(2)),
    });

    let first = tokio::spawn(register_after_claim_barrier(
        Arc::clone(&state),
        Arc::clone(&invites),
        code.clone(),
        username("alice"),
        password("alice-password"),
    ));
    let second = tokio::spawn(register_after_claim_barrier(
        Arc::clone(&state),
        invites,
        code,
        username("bob"),
        password("bob-password"),
    ));
    let (first, second) = tokio::time::timeout(Duration::from_secs(20), async {
        tokio::join!(first, second)
    })
    .await
    .expect("concurrent registrations must finish");
    let first = first.expect("first concurrent registration task must not panic");
    let second = second.expect("second concurrent registration task must not panic");

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

async fn register_after_claim_barrier(
    state: Arc<AppState>,
    invites: Arc<dyn InviteStorage>,
    code: InviteCode,
    username: Username,
    password: Password,
) -> Result<common::MutationOutcome<UserId>, WriteScopeError<RegisterWithInviteError>> {
    let users = Arc::clone(&state.users);
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
                        display_name: None,
                        is_operator: false,
                        invite_code: &code,
                    },
                )
                .await
            })
        })
        .await
}
