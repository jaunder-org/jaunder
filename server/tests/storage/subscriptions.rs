use std::sync::Arc;

use common::ids::{ChannelId, UserId};
use common::visibility::{SubscriptionPolicy, SubscriptionStatus, ViewerIdentity};
use rstest::*;
use rstest_reuse::*;
use storage::test_support::{Backend, backends, seed_users};
use storage::{PostgresSubscriptionStorage, SqliteSubscriptionStorage, SubscriptionStorage};

use super::fixtures::{channel_id_by_name, local_channel_id, open_pool, raw_exec};

// The production `SubscriptionStorage::local_channel_id()` accessor must return
// the same id as the seeded `'local'` channel row (read here via the raw test
// helper of the same name).
#[apply(backends)]
#[tokio::test]
async fn local_channel_id_returns_seeded_local(#[case] backend: Backend) {
    let env = backend.setup().await;
    let state = &env.state;
    let expected = local_channel_id(backend, &env).await;
    let actual = state.subscriptions.local_channel_id().await.unwrap();
    assert_eq!(actual, expected);
}

// The other half of the accessor's contract: with the seed gone, the absence is
// reported as a *named* missing row, not as an anonymous driver error the
// boundary would page on with "storage operation failed" (#343). Deleting the
// row is possible on both backends because `subscriptions` is the only table
// referencing `channels` and a fresh test database has no subscription rows.
#[apply(backends)]
#[tokio::test]
async fn local_channel_id_names_the_row_when_the_seed_is_missing(#[case] backend: Backend) {
    let env = backend.setup().await;
    let state = &env.state;
    raw_exec(backend, &env, "DELETE FROM channels WHERE name = 'local'").await;
    let error = state.subscriptions.local_channel_id().await.unwrap_err();
    assert_eq!(error.kind(), host::error::ErrorKind::Internal);
    assert_eq!(error.class(), host::error::ErrorClass::Bug);
    let operator = error.operator_message();
    assert!(
        operator.contains("the seeded 'local' channel row"),
        "operator message must name the missing row, got: {operator}"
    );
}

#[apply(backends)]
#[tokio::test]
async fn subscribe_is_idempotent_and_active(#[case] backend: Backend) {
    let env = backend.setup().await;
    let state = &env.state;
    let [author, bob] = seed_users(state).await;
    let local = local_channel_id(backend, &env).await;
    let id1 = state
        .subscriptions
        .subscribe(author, local, &bob.to_string())
        .await
        .unwrap();
    let id2 = state
        .subscriptions
        .subscribe(author, local, &bob.to_string())
        .await
        .unwrap();
    assert_eq!(id1, id2, "subscribe is idempotent");
    assert!(
        state
            .subscriptions
            .is_subscriber(author, &ViewerIdentity::local(bob))
            .await
            .unwrap()
    );
    assert!(
        !state
            .subscriptions
            .is_subscriber(author, &ViewerIdentity::Anonymous)
            .await
            .unwrap()
    );
    // Active subscriber appears in the listing.
    let subs = state.subscriptions.list_subscribers(author).await.unwrap();
    assert_eq!(subs.len(), 1);
    assert_eq!(subs[0].subscription_id, id1);
    assert_eq!(subs[0].channel_id, local);
    assert_eq!(subs[0].subscriber_ref, bob.to_string());
    assert_eq!(subs[0].status, SubscriptionStatus::Active);
    // Unsubscribe round-trips: no longer a subscriber, listing empties.
    state
        .subscriptions
        .unsubscribe(author, local, &bob.to_string())
        .await
        .unwrap();
    assert!(
        !state
            .subscriptions
            .is_subscriber(author, &ViewerIdentity::local(bob))
            .await
            .unwrap()
    );
    assert!(
        state
            .subscriptions
            .list_subscribers(author)
            .await
            .unwrap()
            .is_empty()
    );
}

// `is_subscriber` resolves a `Remote` viewer against its own channel: admission
// is the (channel, ref) pair on the subscription row, so the same opaque ref on
// a different channel is a different subscriber. This is the non-local half of
// the variant split (#6) — the `Local` arm is covered by the test above.
#[apply(backends)]
#[tokio::test]
async fn is_subscriber_resolves_a_remote_viewer_by_its_own_channel(#[case] backend: Backend) {
    let env = backend.setup().await;
    let state = &env.state;
    let [author] = seed_users(state).await;
    let local = local_channel_id(backend, &env).await;
    raw_exec(
        backend,
        &env,
        "INSERT INTO channels (name) VALUES ('activitypub')",
    )
    .await;
    let remote = channel_id_by_name(backend, &env, "activitypub").await;

    let actor = "https://remote.example/users/alice";
    state
        .subscriptions
        .subscribe(author, remote, actor)
        .await
        .unwrap();

    assert!(
        state
            .subscriptions
            .is_subscriber(
                author,
                &ViewerIdentity::Remote {
                    channel_id: remote,
                    subscriber_ref: actor.to_owned(),
                }
            )
            .await
            .unwrap(),
        "a remote viewer matching its own subscription row is admitted"
    );
    assert!(
        !state
            .subscriptions
            .is_subscriber(
                author,
                &ViewerIdentity::Remote {
                    channel_id: local,
                    subscriber_ref: actor.to_owned(),
                }
            )
            .await
            .unwrap(),
        "the same ref on another channel is a different subscriber"
    );
}

// Fail-closed admission: `is_subscriber` admits only `active` rows, so a
// subscription a stricter policy left `pending` must NOT be admitted. The
// default `state.subscriptions` uses `OpenSubscriptionPolicy` (always active),
// so we construct the store directly with a stub policy returning `Pending`.
#[apply(backends)]
#[tokio::test]
async fn pending_subscription_is_not_admitted(#[case] backend: Backend) {
    struct StubPending;
    impl SubscriptionPolicy for StubPending {
        fn initial_status(&self, _a: UserId, _c: ChannelId, _r: &str) -> SubscriptionStatus {
            SubscriptionStatus::Pending
        }
    }

    let env = backend.setup().await;
    // Only `active` is seeded this milestone (M13 adds `pending`). Seed the
    // `pending` lookup row locally so `subscribe` can persist a pending row and
    // we can prove `is_subscriber` still excludes it (the fail-closed property).
    // Build the store over the *same* per-test database as `env.state`, with the
    // stub `Pending` policy, per backend.
    let store: Box<dyn SubscriptionStorage> = match backend {
        Backend::Sqlite => {
            let pool = open_pool(&env.base).await; // same DB file as env.state
            sqlx::query("INSERT INTO subscription_statuses (name) VALUES ('pending')")
                .execute(&pool)
                .await
                .unwrap();
            Box::new(SqliteSubscriptionStorage::new(pool, Arc::new(StubPending)))
        }
        Backend::Postgres => {
            let pool = env.base.pool().postgres().clone(); // same DB as env.state
            sqlx::query("INSERT INTO subscription_statuses (name) VALUES ('pending')")
                .execute(&pool)
                .await
                .unwrap();
            Box::new(PostgresSubscriptionStorage::new(
                pool,
                Arc::new(StubPending),
            ))
        }
    };
    let [author, bob] = seed_users(&env.state).await;
    let local = local_channel_id(backend, &env).await;
    store
        .subscribe(author, local, &bob.to_string())
        .await
        .unwrap();
    // Resolution admits only `active` → a pending subscriber is excluded.
    assert!(
        !store
            .is_subscriber(author, &ViewerIdentity::local(bob))
            .await
            .unwrap()
    );
    // ...and it is not listed (list_subscribers is active-only).
    assert!(store.list_subscribers(author).await.unwrap().is_empty());
}
