use std::sync::Arc;

use common::ids::UserId;
use common::time::UtcInstant;
use common::visibility::{
    SubscriberIdentity, SubscriptionPolicy, SubscriptionStatus, ViewerIdentity,
    local_subscriber_identity,
};
use rstest::*;
use rstest_reuse::*;
use storage::sql::QueryStorageExt;
use storage::test_support::{Backend, SeedUser, backends, confirmed_for as confirmed, seed_users};
use storage::{
    CorruptSubscriberRef, PostgresSubscriptionStorage, SqliteSubscriptionStorage,
    SubscriptionStorage, WriteScope,
};

use super::fixtures::{
    channel_id_by_name, local_channel_id, open_pool, raw_exec, update_subscription_created_at,
};

#[apply(backends)]
#[tokio::test]
async fn local_channel_id_returns_seeded_local(#[case] backend: Backend) {
    let env = backend.setup().await;
    let expected = local_channel_id(backend, &env).await;
    let actual = env.state.subscriptions.local_channel_id().await.unwrap();
    assert_eq!(actual, expected);
}

#[apply(backends)]
#[tokio::test]
async fn local_channel_id_names_the_row_when_the_seed_is_missing(#[case] backend: Backend) {
    let env = backend.setup().await;
    raw_exec(backend, &env, "DELETE FROM channels WHERE name = 'local'").await;
    let error = env
        .state
        .subscriptions
        .local_channel_id()
        .await
        .unwrap_err();
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
async fn subscribe_round_trips_fixed_created_at_and_preserves_order(#[case] backend: Backend) {
    let env = backend.setup().await;
    let state = &env.state;
    let author = SeedUser::new().seed(state).await.user_id;
    let bob = SeedUser::new().seed(state).await.user_id;
    let carol = SeedUser::new().seed(state).await.user_id;
    let local = local_channel_id(backend, &env).await;
    let bob_subscriber = local_subscriber_identity(local, bob);
    let carol_subscriber = local_subscriber_identity(local, carol);
    let bob_id = subscribe_confirmed(
        &state.write_scope,
        Arc::clone(&state.subscriptions),
        author,
        bob_subscriber.clone(),
    )
    .await;
    let repeated_bob_id = subscribe_confirmed(
        &state.write_scope,
        Arc::clone(&state.subscriptions),
        author,
        bob_subscriber.clone(),
    )
    .await;
    let carol_id = subscribe_confirmed(
        &state.write_scope,
        Arc::clone(&state.subscriptions),
        author,
        carol_subscriber,
    )
    .await;
    assert_eq!(bob_id, repeated_bob_id, "subscribe is idempotent");

    let bob_created_at: UtcInstant = "2026-01-02T03:04:05.123457Z".parse().unwrap();
    let carol_created_at: UtcInstant = "2026-01-02T03:04:05.123456Z".parse().unwrap();
    update_subscription_created_at(backend, &env, bob_id, bob_created_at).await;
    update_subscription_created_at(backend, &env, carol_id, carol_created_at).await;

    let subs = state.subscriptions.list_subscribers(author).await.unwrap();
    assert_eq!(subs.len(), 2);
    assert_eq!(
        subs.iter()
            .map(|subscription| subscription.subscription_id)
            .collect::<Vec<_>>(),
        vec![bob_id, carol_id]
    );
    assert_eq!(
        subs.iter()
            .map(|subscription| subscription.created_at)
            .collect::<Vec<_>>(),
        vec![bob_created_at, carol_created_at]
    );
    assert_eq!(subs[0].subscriber.channel_id, local);
    assert_eq!(subs[0].subscriber.subscriber_ref.as_ref(), bob.to_string());
    assert_eq!(subs[0].status, SubscriptionStatus::Active);
    assert_eq!(subs[1].subscriber.channel_id, local);
    assert_eq!(
        subs[1].subscriber.subscriber_ref.as_ref(),
        carol.to_string()
    );
    assert_eq!(subs[1].status, SubscriptionStatus::Active);
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

    unsubscribe_confirmed(
        &state.write_scope,
        Arc::clone(&state.subscriptions),
        author,
        bob_subscriber,
    )
    .await;
    assert!(
        !state
            .subscriptions
            .is_subscriber(author, &ViewerIdentity::local(bob))
            .await
            .unwrap()
    );
    let remaining = state.subscriptions.list_subscribers(author).await.unwrap();
    assert_eq!(remaining.len(), 1);
    assert_eq!(remaining[0].subscription_id, carol_id);
    assert_eq!(remaining[0].created_at, carol_created_at);
}

#[apply(backends)]
#[tokio::test]
async fn list_subscriber_summaries_resolves_labels_on_both_dialects(#[case] backend: Backend) {
    let env = backend.setup().await;
    let state = &env.state;
    let author = SeedUser::new().seed(state).await.user_id;
    let local_user = SeedUser::new().seed(state).await;
    let local = local_channel_id(backend, &env).await;
    raw_exec(
        backend,
        &env,
        "INSERT INTO channels (name) VALUES ('activitypub')",
    )
    .await;
    let remote = channel_id_by_name(backend, &env, "activitypub").await;

    let resolved = subscribe_confirmed(
        &state.write_scope,
        Arc::clone(&state.subscriptions),
        author,
        local_subscriber_identity(local, local_user.user_id),
    )
    .await;
    let numeric_remote_ref = local_user.user_id.to_string();
    let remote_numeric = subscribe_confirmed(
        &state.write_scope,
        Arc::clone(&state.subscriptions),
        author,
        SubscriberIdentity::new(remote, numeric_remote_ref.parse().unwrap()),
    )
    .await;
    let missing_ref = "999999999";
    let missing_local = subscribe_confirmed(
        &state.write_scope,
        Arc::clone(&state.subscriptions),
        author,
        SubscriberIdentity::new(local, missing_ref.parse().unwrap()),
    )
    .await;

    let rows = state
        .subscriptions
        .list_subscriber_summaries(author)
        .await
        .unwrap();
    assert_eq!(
        rows.into_iter()
            .map(|row| (row.subscription_id, row.label))
            .collect::<Vec<_>>(),
        vec![
            (resolved, local_user.username.to_string()),
            (remote_numeric, numeric_remote_ref),
            (missing_local, missing_ref.to_string()),
        ]
    );
}

#[apply(backends)]
#[tokio::test]
async fn subscriber_bulk_reads_skip_unicode_blank_stored_refs(#[case] backend: Backend) {
    let env = backend.setup().await;
    let state = &env.state;
    let author = SeedUser::new().seed(state).await.user_id;
    let valid_subscriber = SeedUser::new().seed(state).await;
    let local = local_channel_id(backend, &env).await;
    let valid_identity = local_subscriber_identity(local, valid_subscriber.user_id);
    let valid_subscription_id = subscribe_confirmed(
        &state.write_scope,
        Arc::clone(&state.subscriptions),
        author,
        valid_identity.clone(),
    )
    .await;

    let malformed_subscription = storage::with_closeable_pool!(env.base.pool(), pool, {
        sqlx::query(
            "INSERT INTO subscriptions \
             (author_user_id, channel_id, subscriber_ref, status_id) \
             VALUES ($1, $2, $3, \
             (SELECT status_id FROM subscription_statuses WHERE name = 'active'))",
        )
        .bind_storage(author)
        .bind_storage(local)
        .bind_storage(CorruptSubscriberRef("\u{2003}".to_owned()))
        .execute(pool)
        .await
        .map(|_| ())
    });
    malformed_subscription.expect("malformed subscriber fixture setup should succeed");

    let listing = state
        .subscriptions
        .list_subscribers(author)
        .await
        .expect("list subscribers");
    assert_eq!(listing.len(), 1, "only the valid subscriber is returned");
    assert_eq!(listing[0].subscription_id, valid_subscription_id);
    assert_eq!(listing[0].subscriber, valid_identity);
    assert_eq!(listing[0].status, SubscriptionStatus::Active);

    let summaries = state
        .subscriptions
        .list_subscriber_summaries(author)
        .await
        .expect("list subscriber summaries");
    assert_eq!(
        summaries
            .into_iter()
            .map(|summary| (summary.subscription_id, summary.label))
            .collect::<Vec<_>>(),
        vec![(valid_subscription_id, valid_subscriber.username.to_string())],
        "the summary omits only the invalid subscriber"
    );
}

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
    subscribe_confirmed(
        &state.write_scope,
        Arc::clone(&state.subscriptions),
        author,
        SubscriberIdentity::new(remote, actor.parse().unwrap()),
    )
    .await;

    assert!(
        state
            .subscriptions
            .is_subscriber(
                author,
                &ViewerIdentity::Remote {
                    channel_id: remote,
                    subscriber_ref: actor.parse().unwrap()
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
                    subscriber_ref: actor.parse().unwrap()
                }
            )
            .await
            .unwrap(),
        "the same ref on another channel is a different subscriber"
    );
}

#[apply(backends)]
#[tokio::test]
async fn pending_subscription_is_not_admitted(#[case] backend: Backend) {
    struct StubPending;
    impl SubscriptionPolicy for StubPending {
        fn initial_status(
            &self,
            _author: UserId,
            _subscriber: &SubscriberIdentity,
        ) -> SubscriptionStatus {
            SubscriptionStatus::Pending
        }
    }

    let env = backend.setup().await;
    let store: Arc<dyn SubscriptionStorage> = match backend {
        Backend::Sqlite => {
            let pool = open_pool(&env.base).await;
            sqlx::query("INSERT INTO subscription_statuses (name) VALUES ('pending')")
                .execute(&pool)
                .await
                .unwrap();
            Arc::new(SqliteSubscriptionStorage::new(pool, Arc::new(StubPending)))
        }
        Backend::Postgres => {
            let pool = env.base.pool().postgres().clone();
            sqlx::query("INSERT INTO subscription_statuses (name) VALUES ('pending')")
                .execute(&pool)
                .await
                .unwrap();
            Arc::new(PostgresSubscriptionStorage::new(
                pool,
                Arc::new(StubPending),
            ))
        }
    };
    let [author, bob] = seed_users(&env.state).await;
    let local = local_channel_id(backend, &env).await;
    subscribe_confirmed(
        &env.state.write_scope,
        Arc::clone(&store),
        author,
        local_subscriber_identity(local, bob),
    )
    .await;
    assert!(
        !store
            .is_subscriber(author, &ViewerIdentity::local(bob))
            .await
            .unwrap()
    );
    assert!(store.list_subscribers(author).await.unwrap().is_empty());
    assert!(
        store
            .list_subscriber_summaries(author)
            .await
            .unwrap()
            .is_empty()
    );
}

async fn subscribe_confirmed(
    write_scope: &WriteScope,
    subscriptions: Arc<dyn SubscriptionStorage>,
    author: UserId,
    subscriber: SubscriberIdentity,
) -> common::ids::SubscriptionId {
    let outcome = write_scope
        .run(move |transaction| {
            Box::pin(async move {
                subscriptions
                    .subscribe(transaction, author, &subscriber)
                    .await
            })
        })
        .await
        .expect("subscription fixture setup should succeed");
    confirmed(outcome, "subscription fixture setup")
}

async fn unsubscribe_confirmed(
    write_scope: &WriteScope,
    subscriptions: Arc<dyn SubscriptionStorage>,
    author: UserId,
    subscriber: SubscriberIdentity,
) {
    let outcome = write_scope
        .run(move |transaction| {
            Box::pin(async move {
                subscriptions
                    .unsubscribe(transaction, author, &subscriber)
                    .await
            })
        })
        .await
        .expect("subscription removal should succeed");
    confirmed(outcome, "subscription removal");
}
