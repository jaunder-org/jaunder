use common::test_support::parse_audience_name;
use rstest::*;
use rstest_reuse::*;
use storage::AudienceError;
use storage::test_support::{Backend, SeedUser, TestEnv, backends, seed_users};

use super::fixtures::{local_channel_id, open_pool};

// create → list → rename → delete round-trip. Every write is author-scoped and
// the listing is ordered by `audience_id`; rename and delete mutate exactly the
// targeted row.
#[apply(backends)]
#[tokio::test]
async fn audience_create_list_rename_delete(#[case] backend: Backend) {
    let env = backend.setup().await;
    let state = &env.state;
    let author = SeedUser::new().seed(state).await.user_id;

    let friends = state
        .audiences
        .create_audience(author, &parse_audience_name("Friends"))
        .await
        .unwrap();
    let family = state
        .audiences
        .create_audience(author, &parse_audience_name("Family"))
        .await
        .unwrap();

    // Listing is author-scoped and ordered by audience_id (insertion order).
    let listed = state.audiences.list_audiences(author).await.unwrap();
    assert_eq!(listed.len(), 2);
    assert_eq!(listed[0].audience_id, friends);
    assert_eq!(listed[0].name, "Friends");
    assert_eq!(listed[1].audience_id, family);
    assert_eq!(listed[1].name, "Family");

    // Rename mutates exactly the targeted audience.
    state
        .audiences
        .rename_audience(author, friends, &parse_audience_name("Close Friends"))
        .await
        .unwrap();
    let listed = state.audiences.list_audiences(author).await.unwrap();
    assert_eq!(listed[0].name, "Close Friends");

    // Renaming an audience the author does not own is NotFound.
    let stranger = SeedUser::new().seed(state).await.user_id;
    assert!(matches!(
        state
            .audiences
            .rename_audience(stranger, friends, &parse_audience_name("Hijacked"))
            .await,
        Err(AudienceError::NotFound)
    ));

    // Delete removes exactly the targeted audience.
    state
        .audiences
        .delete_audience(author, friends)
        .await
        .unwrap();
    let listed = state.audiences.list_audiences(author).await.unwrap();
    assert_eq!(listed.len(), 1);
    assert_eq!(listed[0].audience_id, family);
}

// A duplicate `(author_user_id, name)` is mapped to DuplicateName on both create
// and rename; a different author may reuse the same name.
#[apply(backends)]
#[tokio::test]
async fn audience_duplicate_name_rejected(#[case] backend: Backend) {
    let env = backend.setup().await;
    let state = &env.state;
    let [alice, bob] = seed_users(state).await;

    state
        .audiences
        .create_audience(alice, &parse_audience_name("Friends"))
        .await
        .unwrap();
    // Same author, same name → DuplicateName.
    assert!(matches!(
        state
            .audiences
            .create_audience(alice, &parse_audience_name("Friends"))
            .await,
        Err(AudienceError::DuplicateName)
    ));
    // Different author may reuse the name.
    state
        .audiences
        .create_audience(bob, &parse_audience_name("Friends"))
        .await
        .unwrap();

    // Rename onto an existing name (same author) → DuplicateName.
    let work = state
        .audiences
        .create_audience(alice, &parse_audience_name("Work"))
        .await
        .unwrap();
    assert!(matches!(
        state
            .audiences
            .rename_audience(alice, work, &parse_audience_name("Friends"))
            .await,
        Err(AudienceError::DuplicateName)
    ));
}

// add_member / list_members / remove_member happy path against a same-owner
// subscription seeded via the wired SubscriptionStore.
#[apply(backends)]
#[tokio::test]
async fn audience_membership_round_trip(#[case] backend: Backend) {
    let env = backend.setup().await;
    let state = &env.state;
    let [author, bob] = seed_users(state).await;
    let local = local_channel_id(backend, &env).await;
    let sub = state
        .subscriptions
        .subscribe(author, local, &bob.to_string())
        .await
        .unwrap();
    let audience = state
        .audiences
        .create_audience(author, &parse_audience_name("Friends"))
        .await
        .unwrap();

    assert!(
        state
            .audiences
            .list_members(author, audience)
            .await
            .unwrap()
            .is_empty()
    );

    state
        .audiences
        .add_member(author, audience, sub)
        .await
        .unwrap();
    // add_member is idempotent.
    state
        .audiences
        .add_member(author, audience, sub)
        .await
        .unwrap();
    assert_eq!(
        state
            .audiences
            .list_members(author, audience)
            .await
            .unwrap(),
        vec![sub]
    );

    state
        .audiences
        .remove_member(author, audience, sub)
        .await
        .unwrap();
    assert!(
        state
            .audiences
            .list_members(author, audience)
            .await
            .unwrap()
            .is_empty()
    );
}

// The same-owner invariant is enforced by the composite FKs: pairing an audience
// with a subscription owned by a *different* author must be rejected by the DB
// and surface as `AudienceError::Storage` (no app-level check). Complements the
// raw-SQL `composite_fks_reject_cross_author_membership` test at the trait layer.
#[apply(backends)]
#[tokio::test]
async fn audience_add_member_cross_author_rejected(#[case] backend: Backend) {
    let env = backend.setup().await;
    let state = &env.state;
    let [alice, bob] = seed_users(state).await;
    let local = local_channel_id(backend, &env).await;
    // Subscription owned by BOB.
    let bob_sub = state
        .subscriptions
        .subscribe(bob, local, &alice.to_string())
        .await
        .unwrap();
    // Audience owned by ALICE.
    let alice_audience = state
        .audiences
        .create_audience(alice, &parse_audience_name("Friends"))
        .await
        .unwrap();

    // Alice pairs her audience with Bob's subscription: the
    // (subscription_id, author_user_id) FK fails → Storage error.
    assert!(matches!(
        state
            .audiences
            .add_member(alice, alice_audience, bob_sub)
            .await,
        Err(AudienceError::Storage(_))
    ));
    assert!(
        state
            .audiences
            .list_members(alice, alice_audience)
            .await
            .unwrap()
            .is_empty()
    );
}

// `list_members` / `remove_member` are author-scoped: a different author can
// neither see nor mutate another author's audience membership (the WHERE clause
// filters by `author_user_id`, so a cross-author `audience_id` matches nothing).
#[apply(backends)]
#[tokio::test]
async fn audience_members_are_author_scoped(#[case] backend: Backend) {
    let env = backend.setup().await;
    let state = &env.state;
    let [alice, bob] = seed_users(state).await;
    let local = local_channel_id(backend, &env).await;
    // A subscription and audience both owned by ALICE, with the sub as a member.
    let alice_sub = state
        .subscriptions
        .subscribe(alice, local, &bob.to_string())
        .await
        .unwrap();
    let alice_audience = state
        .audiences
        .create_audience(alice, &parse_audience_name("Friends"))
        .await
        .unwrap();
    state
        .audiences
        .add_member(alice, alice_audience, alice_sub)
        .await
        .unwrap();

    // Bob cannot list Alice's members...
    assert!(
        state
            .audiences
            .list_members(bob, alice_audience)
            .await
            .unwrap()
            .is_empty()
    );
    // ...and a Bob-scoped remove leaves Alice's membership untouched (no-op).
    state
        .audiences
        .remove_member(bob, alice_audience, alice_sub)
        .await
        .unwrap();
    assert_eq!(
        state
            .audiences
            .list_members(alice, alice_audience)
            .await
            .unwrap(),
        vec![alice_sub]
    );
}

// `delete_audience` must remove the audience's membership rows in the same
// transaction, not just the `audiences` row. The schema declares no
// `ON DELETE CASCADE` and SQLite enforces foreign keys off by default, so a
// dropped `DELETE FROM audience_members` would silently orphan membership rows.
// A raw `COUNT(*)` proves they are gone (`list_members` on a deleted audience is
// trivially empty regardless, so it cannot catch the orphan).
#[apply(backends)]
#[tokio::test]
async fn audience_delete_cascades_memberships(#[case] backend: Backend) {
    let env = backend.setup().await;
    let state = &env.state;
    let [alice, bob] = seed_users(state).await;
    let local = local_channel_id(backend, &env).await;
    let sub = state
        .subscriptions
        .subscribe(alice, local, &bob.to_string())
        .await
        .unwrap();
    let audience = state
        .audiences
        .create_audience(alice, &parse_audience_name("Friends"))
        .await
        .unwrap();
    state
        .audiences
        .add_member(alice, audience, sub)
        .await
        .unwrap();

    // Precondition: the membership row exists.
    let members_sql =
        format!("SELECT COUNT(*) FROM audience_members WHERE audience_id = {audience}");
    assert_eq!(raw_scalar_i64(backend, &env, &members_sql).await, 1);

    state
        .audiences
        .delete_audience(alice, audience)
        .await
        .unwrap();

    // The membership row is gone, not orphaned.
    assert_eq!(
        raw_scalar_i64(backend, &env, &members_sql).await,
        0,
        "delete_audience must cascade-remove its membership rows"
    );
}

// Reads a single `i64` (e.g. a `COUNT(*)`) on the FK-enabled pool for `backend`,
// so a test can observe rows the trait API cannot reach (e.g. membership rows of a
// deleted audience). Mirrors `raw_exec`'s per-backend pool selection.
async fn raw_scalar_i64(backend: Backend, env: &TestEnv, sql: &str) -> i64 {
    match backend {
        Backend::Sqlite => sqlx::query_scalar::<_, i64>(sql)
            .fetch_one(&open_pool(&env.base).await)
            .await
            .unwrap(),
        Backend::Postgres => {
            let pool = env.base.pool().postgres();
            sqlx::query_scalar::<_, i64>(sql)
                .fetch_one(pool)
                .await
                .unwrap()
        }
    }
}
