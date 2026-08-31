use std::{sync::Arc, time::Duration};

use common::time::UtcInstant;
use rstest::*;
use rstest_reuse::*;
use storage::test_support::{Backend, SeedUser, postgres_only};
use storage::{AppState, UseInviteError, WriteScopeError};

use super::super::invites::create_invite;

#[apply(postgres_only)]
// reason: verifies PostgreSQL concurrent conditional-claim behavior under independent transactions.
#[tokio::test]
async fn concurrent_invite_claims_allow_exactly_one_user(#[case] backend: Backend) {
    let env = backend.setup().await;
    let state = Arc::clone(&env.state);
    let code = create_invite(
        &state,
        "2099-01-02T03:04:05.123457Z".parse::<UtcInstant>().unwrap(),
    )
    .await;
    let first_user = SeedUser::new().seed(&state).await.user_id;
    let second_user = SeedUser::new().seed(&state).await.user_id;
    let barrier = Arc::new(tokio::sync::Barrier::new(2));

    let first = tokio::spawn(claim_invite_after_barrier(
        Arc::clone(&state),
        Arc::clone(&barrier),
        code.clone(),
        first_user,
    ));
    let second = tokio::spawn(claim_invite_after_barrier(
        state,
        barrier,
        code,
        second_user,
    ));
    let (first, second) = tokio::time::timeout(Duration::from_secs(20), async {
        tokio::join!(first, second)
    })
    .await
    .expect("concurrent claims must finish");
    let first = first.expect("first concurrent claim task must not panic");
    let second = second.expect("second concurrent claim task must not panic");

    let successes = [first.as_ref(), second.as_ref()]
        .into_iter()
        .flatten()
        .count();
    assert_eq!(successes, 1, "exactly one conditional claim must succeed");
    for result in [first, second] {
        if let Err(WriteScopeError::Operation(error)) = result {
            assert!(
                matches!(error, UseInviteError::AlreadyUsed),
                "losing conditional claim must be AlreadyUsed, got {error:?}"
            );
        }
    }
}

async fn claim_invite_after_barrier(
    state: Arc<AppState>,
    barrier: Arc<tokio::sync::Barrier>,
    code: host::invite::InviteCode,
    user_id: common::ids::UserId,
) -> Result<common::MutationOutcome<()>, WriteScopeError<UseInviteError>> {
    let invites = Arc::clone(&state.invites);
    barrier.wait().await;
    state
        .write_scope
        .run(|transaction| {
            Box::pin(async move { invites.claim_invite(transaction, &code, user_id).await })
        })
        .await
}
