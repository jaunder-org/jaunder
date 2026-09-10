use std::sync::Arc;

use common::MutationOutcome;
use common::test_support::parse_audience_name;
use rstest::*;
use rstest_reuse::*;
use storage::sql::QueryStorageExt;
use storage::test_support::{
    Backend, SeedUser, backends, confirmed_for as confirmed, seed_local_subscription, seed_users,
};
use storage::{AudienceError, WriteScopeError};

#[apply(backends)]
#[tokio::test]
async fn audience_create_list_rename_delete(#[case] backend: Backend) {
    let env = backend.setup().await;
    let author = SeedUser::new()
        .seed(env.users(), env.write_scope())
        .await
        .user_id;

    let friends = create_audience_confirmed(
        env.audiences(),
        env.write_scope(),
        author,
        parse_audience_name("Friends"),
    )
    .await;
    let family = create_audience_confirmed(
        env.audiences(),
        env.write_scope(),
        author,
        parse_audience_name("Family"),
    )
    .await;

    let listed = env.audiences().list_audiences(author).await.unwrap();
    assert_eq!(listed.len(), 2);
    assert_eq!(listed[0].audience_id, friends);
    assert_eq!(listed[0].name, "Friends");
    assert_eq!(listed[1].audience_id, family);
    assert_eq!(listed[1].name, "Family");

    rename_audience_confirmed(
        env.audiences(),
        env.write_scope(),
        author,
        friends,
        parse_audience_name("Close Friends"),
    )
    .await;
    let listed = env.audiences().list_audiences(author).await.unwrap();
    assert_eq!(listed[0].name, "Close Friends");

    let stranger = SeedUser::new()
        .seed(env.users(), env.write_scope())
        .await
        .user_id;
    assert!(matches!(
        rename_audience(
            env.audiences(),
            env.write_scope(),
            stranger,
            friends,
            parse_audience_name("Hijacked"),
        )
        .await,
        Err(WriteScopeError::Operation(AudienceError::NotFound))
    ));

    delete_audience_confirmed(env.audiences(), env.write_scope(), author, friends).await;
    let listed = env.audiences().list_audiences(author).await.unwrap();
    assert_eq!(listed.len(), 1);
    assert_eq!(listed[0].audience_id, family);
}

#[apply(backends)]
#[tokio::test]
async fn audience_duplicate_name_rejected(#[case] backend: Backend) {
    let env = backend.setup().await;
    let [alice, bob] = seed_users(env.users(), env.write_scope()).await;

    create_audience_confirmed(
        env.audiences(),
        env.write_scope(),
        alice,
        parse_audience_name("Friends"),
    )
    .await;
    assert!(matches!(
        create_audience(
            env.audiences(),
            env.write_scope(),
            alice,
            parse_audience_name("Friends"),
        )
        .await,
        Err(WriteScopeError::Operation(AudienceError::DuplicateName))
    ));
    create_audience_confirmed(
        env.audiences(),
        env.write_scope(),
        bob,
        parse_audience_name("Friends"),
    )
    .await;

    let work = create_audience_confirmed(
        env.audiences(),
        env.write_scope(),
        alice,
        parse_audience_name("Work"),
    )
    .await;
    assert!(matches!(
        rename_audience(
            env.audiences(),
            env.write_scope(),
            alice,
            work,
            parse_audience_name("Friends"),
        )
        .await,
        Err(WriteScopeError::Operation(AudienceError::DuplicateName))
    ));
}

#[apply(backends)]
#[tokio::test]
async fn audience_membership_round_trip(#[case] backend: Backend) {
    let env = backend.setup().await;
    let [author, bob] = seed_users(env.users(), env.write_scope()).await;
    let sub = seed_local_subscription(env.subscriptions(), env.write_scope(), author, bob).await;
    let audience = create_audience_confirmed(
        env.audiences(),
        env.write_scope(),
        author,
        parse_audience_name("Friends"),
    )
    .await;

    assert!(
        env.audiences()
            .list_members(author, audience)
            .await
            .unwrap()
            .is_empty()
    );

    add_member_confirmed(env.audiences(), env.write_scope(), author, audience, sub).await;
    add_member_confirmed(env.audiences(), env.write_scope(), author, audience, sub).await;
    assert_eq!(
        env.audiences()
            .list_members(author, audience)
            .await
            .unwrap(),
        vec![sub]
    );

    remove_member_confirmed(env.audiences(), env.write_scope(), author, audience, sub).await;
    assert!(
        env.audiences()
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
    let [alice, bob] = seed_users(env.users(), env.write_scope()).await;
    let bob_sub = seed_local_subscription(env.subscriptions(), env.write_scope(), bob, alice).await;
    let alice_audience = create_audience_confirmed(
        env.audiences(),
        env.write_scope(),
        alice,
        parse_audience_name("Friends"),
    )
    .await;

    assert!(matches!(
        add_member(
            env.audiences(),
            env.write_scope(),
            alice,
            alice_audience,
            bob_sub,
        )
        .await,
        Err(WriteScopeError::Operation(AudienceError::Storage(_)))
    ));
    assert!(
        env.audiences()
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
    let [alice, bob] = seed_users(env.users(), env.write_scope()).await;
    let alice_sub =
        seed_local_subscription(env.subscriptions(), env.write_scope(), alice, bob).await;
    let alice_audience = create_audience_confirmed(
        env.audiences(),
        env.write_scope(),
        alice,
        parse_audience_name("Friends"),
    )
    .await;
    add_member_confirmed(
        env.audiences(),
        env.write_scope(),
        alice,
        alice_audience,
        alice_sub,
    )
    .await;

    assert!(
        env.audiences()
            .list_members(bob, alice_audience)
            .await
            .unwrap()
            .is_empty()
    );
    remove_member_confirmed(
        env.audiences(),
        env.write_scope(),
        bob,
        alice_audience,
        alice_sub,
    )
    .await;
    assert_eq!(
        env.audiences()
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
    let [alice, bob] = seed_users(env.users(), env.write_scope()).await;
    let sub = seed_local_subscription(env.subscriptions(), env.write_scope(), alice, bob).await;
    let audience = create_audience_confirmed(
        env.audiences(),
        env.write_scope(),
        alice,
        parse_audience_name("Friends"),
    )
    .await;
    add_member_confirmed(env.audiences(), env.write_scope(), alice, audience, sub).await;

    let member_count = storage::with_closeable_pool!(env.base.pool(), pool, {
        sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM audience_members WHERE audience_id = $1")
            .bind_storage(audience)
            .fetch_one(pool)
            .await
            .unwrap()
    });
    assert_eq!(member_count, 1);

    delete_audience_confirmed(env.audiences(), env.write_scope(), alice, audience).await;
    let remaining_member_count = storage::with_closeable_pool!(env.base.pool(), pool, {
        sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM audience_members WHERE audience_id = $1")
            .bind_storage(audience)
            .fetch_one(pool)
            .await
            .unwrap()
    });
    assert_eq!(
        remaining_member_count, 0,
        "delete_audience must cascade-remove its membership rows"
    );
}

async fn create_audience(
    audiences: Arc<dyn storage::AudienceStorage>,
    write_scope: storage::WriteScope,
    author: common::ids::UserId,
    name: common::audience::AudienceName,
) -> Result<MutationOutcome<common::ids::AudienceId>, WriteScopeError<AudienceError>> {
    write_scope
        .run(move |transaction| {
            Box::pin(async move { audiences.create_audience(transaction, author, &name).await })
        })
        .await
}

async fn create_audience_confirmed(
    audiences: Arc<dyn storage::AudienceStorage>,
    write_scope: storage::WriteScope,
    author: common::ids::UserId,
    name: common::audience::AudienceName,
) -> common::ids::AudienceId {
    confirmed(
        create_audience(audiences, write_scope, author, name)
            .await
            .expect("audience fixture setup should succeed"),
        "audience fixture setup",
    )
}

async fn rename_audience(
    audiences: Arc<dyn storage::AudienceStorage>,
    write_scope: storage::WriteScope,
    author: common::ids::UserId,
    audience: common::ids::AudienceId,
    name: common::audience::AudienceName,
) -> Result<MutationOutcome<()>, WriteScopeError<AudienceError>> {
    write_scope
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
    audiences: Arc<dyn storage::AudienceStorage>,
    write_scope: storage::WriteScope,
    author: common::ids::UserId,
    audience: common::ids::AudienceId,
    name: common::audience::AudienceName,
) {
    confirmed(
        rename_audience(audiences, write_scope, author, audience, name)
            .await
            .expect("audience rename should succeed"),
        "audience rename",
    );
}

async fn delete_audience_confirmed(
    audiences: Arc<dyn storage::AudienceStorage>,
    write_scope: storage::WriteScope,
    author: common::ids::UserId,
    audience: common::ids::AudienceId,
) {
    let outcome = write_scope
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
    audiences: Arc<dyn storage::AudienceStorage>,
    write_scope: storage::WriteScope,
    author: common::ids::UserId,
    audience: common::ids::AudienceId,
    subscription: common::ids::SubscriptionId,
) -> Result<MutationOutcome<()>, WriteScopeError<AudienceError>> {
    write_scope
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
    audiences: Arc<dyn storage::AudienceStorage>,
    write_scope: storage::WriteScope,
    author: common::ids::UserId,
    audience: common::ids::AudienceId,
    subscription: common::ids::SubscriptionId,
) {
    confirmed(
        add_member(audiences, write_scope, author, audience, subscription)
            .await
            .expect("audience membership mutation should succeed"),
        "audience membership mutation",
    );
}

async fn remove_member_confirmed(
    audiences: Arc<dyn storage::AudienceStorage>,
    write_scope: storage::WriteScope,
    author: common::ids::UserId,
    audience: common::ids::AudienceId,
    subscription: common::ids::SubscriptionId,
) {
    let outcome = write_scope
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
