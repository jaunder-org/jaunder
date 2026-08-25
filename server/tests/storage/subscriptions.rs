use std::sync::Arc;

use common::ids::UserId;
use common::visibility::{
    SubscriberIdentity, SubscriptionPolicy, SubscriptionStatus, ViewerIdentity,
    local_subscriber_identity,
};
use rstest::*;
use rstest_reuse::*;
use storage::test_support::{Backend, SeedUser, backends, seed_users};
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
    let subscriber = local_subscriber_identity(local, bob);
    let id1 = state
        .subscriptions
        .subscribe(author, &subscriber)
        .await
        .unwrap();
    let id2 = state
        .subscriptions
        .subscribe(author, &subscriber)
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
    assert_eq!(subs[0].subscriber.channel_id, local);
    assert_eq!(subs[0].subscriber.subscriber_ref.as_ref(), bob.to_string());
    assert_eq!(subs[0].status, SubscriptionStatus::Active);
    // Unsubscribe round-trips: no longer a subscriber, listing empties.
    state
        .subscriptions
        .unsubscribe(author, &subscriber)
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

    let resolved = state
        .subscriptions
        .subscribe(
            author,
            &local_subscriber_identity(local, local_user.user_id),
        )
        .await
        .unwrap();
    let numeric_remote_ref = local_user.user_id.to_string();
    let remote_numeric = state
        .subscriptions
        .subscribe(
            author,
            &SubscriberIdentity::new(remote, numeric_remote_ref.parse().unwrap()),
        )
        .await
        .unwrap();
    let missing_ref = "999999999";
    let missing_local = state
        .subscriptions
        .subscribe(
            author,
            &SubscriberIdentity::new(local, missing_ref.parse().unwrap()),
        )
        .await
        .unwrap();

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
    let valid_subscription_id = state
        .subscriptions
        .subscribe(author, &valid_identity)
        .await
        .expect("subscribe valid user");

    // Unicode whitespace is deliberately beyond the portable database
    // constraint, so insert it beneath the Rust write boundary. ADR-0122 says
    // the bad typed column costs exactly this row rather than the whole scan.
    let sql = format!(
        "INSERT INTO subscriptions \
         (author_user_id, channel_id, subscriber_ref, status_id) \
         VALUES ({author}, {local}, '\u{2003}', \
         (SELECT status_id FROM subscription_statuses WHERE name = 'active'))"
    );
    raw_exec(backend, &env, &sql).await;

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
        .subscribe(
            author,
            &SubscriberIdentity::new(remote, actor.parse().unwrap()),
        )
        .await
        .unwrap();

    assert!(
        state
            .subscriptions
            .is_subscriber(
                author,
                &ViewerIdentity::Remote {
                    channel_id: remote,
                    subscriber_ref: actor.parse().unwrap(),
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
                    subscriber_ref: actor.parse().unwrap(),
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
        fn initial_status(&self, _a: UserId, _s: &SubscriberIdentity) -> SubscriptionStatus {
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
        .subscribe(author, &local_subscriber_identity(local, bob))
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
    assert!(
        store
            .list_subscriber_summaries(author)
            .await
            .unwrap()
            .is_empty()
    );
}
