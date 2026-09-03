use std::sync::Arc;

use common::MutationOutcome;
use common::test_support::parse_audience_name;
use rstest::*;
use rstest_reuse::*;
use storage::test_support::{
    Backend, SeedUser, TestEnv, backends, confirmed_for as confirmed, seed_local_subscription,
    seed_users,
};
use storage::{AppState, AudienceError, WriteScopeError};

use super::fixtures::open_pool;

#[apply(backends)]
#[tokio::test]
async fn audience_create_list_rename_delete(#[case] backend: Backend) {
    let env = backend.setup().await;
    let state = &env.state;
    let author = SeedUser::new().seed(state).await.user_id;

    let friends = create_audience_confirmed(state, author, parse_audience_name("Friends")).await;
    let family = create_audience_confirmed(state, author, parse_audience_name("Family")).await;

    let listed = state.audiences.list_audiences(author).await.unwrap();
    assert_eq!(listed.len(), 2);
    assert_eq!(listed[0].audience_id, friends);
    assert_eq!(listed[0].name, "Friends");
    assert_eq!(listed[1].audience_id, family);
    assert_eq!(listed[1].name, "Family");

    rename_audience_confirmed(state, author, friends, parse_audience_name("Close Friends")).await;
    let listed = state.audiences.list_audiences(author).await.unwrap();
    assert_eq!(listed[0].name, "Close Friends");

    let stranger = SeedUser::new().seed(state).await.user_id;
    assert!(matches!(
        rename_audience(state, stranger, friends, parse_audience_name("Hijacked")).await,
        Err(WriteScopeError::Operation(AudienceError::NotFound))
    ));

    delete_audience_confirmed(state, author, friends).await;
    let listed = state.audiences.list_audiences(author).await.unwrap();
    assert_eq!(listed.len(), 1);
    assert_eq!(listed[0].audience_id, family);
}

#[apply(backends)]
#[tokio::test]
async fn audience_duplicate_name_rejected(#[case] backend: Backend) {
    let env = backend.setup().await;
    let state = &env.state;
    let [alice, bob] = seed_users(state).await;

    create_audience_confirmed(state, alice, parse_audience_name("Friends")).await;
    assert!(matches!(
        create_audience(state, alice, parse_audience_name("Friends")).await,
        Err(WriteScopeError::Operation(AudienceError::DuplicateName))
    ));
    create_audience_confirmed(state, bob, parse_audience_name("Friends")).await;

    let work = create_audience_confirmed(state, alice, parse_audience_name("Work")).await;
    assert!(matches!(
        rename_audience(state, alice, work, parse_audience_name("Friends")).await,
        Err(WriteScopeError::Operation(AudienceError::DuplicateName))
    ));
}

#[apply(backends)]
#[tokio::test]
async fn audience_membership_round_trip(#[case] backend: Backend) {
    let env = backend.setup().await;
    let state = &env.state;
    let [author, bob] = seed_users(state).await;
    let sub = seed_local_subscription(state, author, bob).await;
    let audience = create_audience_confirmed(state, author, parse_audience_name("Friends")).await;

    assert!(
        state
            .audiences
            .list_members(author, audience)
            .await
            .unwrap()
            .is_empty()
    );

    add_member_confirmed(state, author, audience, sub).await;
    add_member_confirmed(state, author, audience, sub).await;
    assert_eq!(
        state
            .audiences
            .list_members(author, audience)
            .await
            .unwrap(),
        vec![sub]
    );

    remove_member_confirmed(state, author, audience, sub).await;
    assert!(
        state
            .audiences
            .list_members(author, audience)
            .await
            .unwrap()
            .is_empty()
    );
}

#[apply(backends)]
#[tokio::test]
async fn audience_add_member_cross_author_rejected(#[case] backend: Backend) {
    let env = backend.setup().await;
    let state = &env.state;
    let [alice, bob] = seed_users(state).await;
    let bob_sub = seed_local_subscription(state, bob, alice).await;
    let alice_audience =
        create_audience_confirmed(state, alice, parse_audience_name("Friends")).await;

    assert!(matches!(
        add_member(state, alice, alice_audience, bob_sub).await,
        Err(WriteScopeError::Operation(AudienceError::Storage(_)))
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

#[apply(backends)]
#[tokio::test]
async fn audience_members_are_author_scoped(#[case] backend: Backend) {
    let env = backend.setup().await;
    let state = &env.state;
    let [alice, bob] = seed_users(state).await;
    let alice_sub = seed_local_subscription(state, alice, bob).await;
    let alice_audience =
        create_audience_confirmed(state, alice, parse_audience_name("Friends")).await;
    add_member_confirmed(state, alice, alice_audience, alice_sub).await;

    assert!(
        state
            .audiences
            .list_members(bob, alice_audience)
            .await
            .unwrap()
            .is_empty()
    );
    remove_member_confirmed(state, bob, alice_audience, alice_sub).await;
    assert_eq!(
        state
            .audiences
            .list_members(alice, alice_audience)
            .await
            .unwrap(),
        vec![alice_sub]
    );
}

#[apply(backends)]
#[tokio::test]
async fn audience_delete_cascades_memberships(#[case] backend: Backend) {
    let env = backend.setup().await;
    let state = &env.state;
    let [alice, bob] = seed_users(state).await;
    let sub = seed_local_subscription(state, alice, bob).await;
    let audience = create_audience_confirmed(state, alice, parse_audience_name("Friends")).await;
    add_member_confirmed(state, alice, audience, sub).await;

    let members_sql =
        format!("SELECT COUNT(*) FROM audience_members WHERE audience_id = {audience}");
    assert_eq!(raw_scalar_i64(backend, &env, &members_sql).await, 1);

    delete_audience_confirmed(state, alice, audience).await;
    assert_eq!(
        raw_scalar_i64(backend, &env, &members_sql).await,
        0,
        "delete_audience must cascade-remove its membership rows"
    );
}

async fn create_audience(
    state: &AppState,
    author: common::ids::UserId,
    name: common::audience::AudienceName,
) -> Result<MutationOutcome<common::ids::AudienceId>, WriteScopeError<AudienceError>> {
    let audiences = Arc::clone(&state.audiences);
    state
        .write_scope
        .run(move |transaction| {
            Box::pin(async move { audiences.create_audience(transaction, author, &name).await })
        })
        .await
}

async fn create_audience_confirmed(
    state: &AppState,
    author: common::ids::UserId,
    name: common::audience::AudienceName,
) -> common::ids::AudienceId {
    confirmed(
        create_audience(state, author, name)
            .await
            .expect("audience fixture setup should succeed"),
        "audience fixture setup",
    )
}

async fn rename_audience(
    state: &AppState,
    author: common::ids::UserId,
    audience: common::ids::AudienceId,
    name: common::audience::AudienceName,
) -> Result<MutationOutcome<()>, WriteScopeError<AudienceError>> {
    let audiences = Arc::clone(&state.audiences);
    state
        .write_scope
        .run(move |transaction| {
            Box::pin(async move {
                audiences
                    .rename_audience(transaction, author, audience, &name)
                    .await
            })
        })
        .await
}

async fn rename_audience_confirmed(
    state: &AppState,
    author: common::ids::UserId,
    audience: common::ids::AudienceId,
    name: common::audience::AudienceName,
) {
    confirmed(
        rename_audience(state, author, audience, name)
            .await
            .expect("audience rename should succeed"),
        "audience rename",
    );
}

async fn delete_audience_confirmed(
    state: &AppState,
    author: common::ids::UserId,
    audience: common::ids::AudienceId,
) {
    let audiences = Arc::clone(&state.audiences);
    let outcome = state
        .write_scope
        .run(move |transaction| {
            Box::pin(async move {
                audiences
                    .delete_audience(transaction, author, audience)
                    .await
            })
        })
        .await
        .expect("audience deletion should succeed");
    confirmed(outcome, "audience deletion");
}

async fn add_member(
    state: &AppState,
    author: common::ids::UserId,
    audience: common::ids::AudienceId,
    subscription: common::ids::SubscriptionId,
) -> Result<MutationOutcome<()>, WriteScopeError<AudienceError>> {
    let audiences = Arc::clone(&state.audiences);
    state
        .write_scope
        .run(move |transaction| {
            Box::pin(async move {
                audiences
                    .add_member(transaction, author, audience, subscription)
                    .await
            })
        })
        .await
}

async fn add_member_confirmed(
    state: &AppState,
    author: common::ids::UserId,
    audience: common::ids::AudienceId,
    subscription: common::ids::SubscriptionId,
) {
    confirmed(
        add_member(state, author, audience, subscription)
            .await
            .expect("audience membership mutation should succeed"),
        "audience membership mutation",
    );
}

async fn remove_member_confirmed(
    state: &AppState,
    author: common::ids::UserId,
    audience: common::ids::AudienceId,
    subscription: common::ids::SubscriptionId,
) {
    let audiences = Arc::clone(&state.audiences);
    let outcome = state
        .write_scope
        .run(move |transaction| {
            Box::pin(async move {
                audiences
                    .remove_member(transaction, author, audience, subscription)
                    .await
            })
        })
        .await
        .expect("audience membership removal should succeed");
    confirmed(outcome, "audience membership removal");
}

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
