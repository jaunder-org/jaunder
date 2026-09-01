use std::{sync::Arc, time::Duration};

use async_trait::async_trait;
use common::{ids::UserId, time::UtcInstant, username::Username};
use host::{invite::InviteCode, password::Password};
use rstest::*;
use rstest_reuse::*;
use storage::test_support::{Backend, postgres_only};
use storage::{
    AppState, InviteRecord, InviteStorage, OperatorStatus, UseInviteError, WriteScopeError,
    WriteTransaction,
    account_mutations::{self, RegisterWithInviteError, RegisterWithInviteInput},
};

use super::super::{
    fixtures::{password, username},
    invites::{assert_exactly_one_invite_registration, create_invite},
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

    async fn prune_invites(&self, now: UtcInstant) -> sqlx::Result<u64> {
        self.inner.prune_invites(now).await
    }
}

#[apply(postgres_only)]
// reason: SQLite's BEGIN IMMEDIATE serializes writers before the operation callback, so only
// Postgres can force both user inserts to complete before the claim-stage collision tested here.
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
    assert_exactly_one_invite_registration(
        &state,
        first.expect("first concurrent registration task must not panic"),
        second.expect("second concurrent registration task must not panic"),
    )
    .await;
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
                        is_operator: OperatorStatus::STANDARD,
                        invite_code: &code,
                    },
                )
                .await
            })
        })
        .await
}
