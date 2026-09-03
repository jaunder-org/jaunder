//! Fixture-only local subscription setup shared by storage and server integration tests.
//!
//! Subscription contract tests stay explicit so they can vary identity and lifecycle behavior.

use std::sync::Arc;

use common::ids::{SubscriptionId, UserId};
use common::visibility;

use crate::AppState;

/// Seed an active local subscription for fixture-only test setup.
///
/// # Panics
///
/// If the local channel cannot be read or the subscription cannot be created.
pub async fn seed_local_subscription(
    state: &AppState,
    author: UserId,
    subscriber: UserId,
) -> SubscriptionId {
    let local = state
        .subscriptions
        .local_channel_id()
        .await
        .expect("local subscription fixture channel should exist");
    let subscriber = visibility::local_subscriber_identity(local, subscriber);
    let subscriptions = Arc::clone(&state.subscriptions);
    let outcome = state
        .write_scope
        .run(move |transaction| {
            Box::pin(async move {
                subscriptions
                    .subscribe(transaction, author, &subscriber)
                    .await
            })
        })
        .await
        .expect("local subscription fixture should be created");
    super::confirmed_for(outcome, "local subscription fixture")
}
