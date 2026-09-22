use common::{
    MutationOutcome,
    content_license::ContentLicense,
    test_support::{parse_audience_name, parse_display_name},
    time::UtcInstant,
    visibility::AudienceTarget,
};
use jiff::{Timestamp, ToSpan};
use rstest::*;
use rstest_reuse::*;
use storage::test_support::{Backend, SeedRawPost, SeedUser, backends, confirmed_for};
use storage::{FeedEventError, MockFeedEventStorage, ProfileUpdate};

async fn clear_feed_events(env: &storage::test_support::TestEnv) {
    let events = env.feed_events();
    let claimed = confirmed_for(
        env.write_scope()
            .run(move |transaction| {
                Box::pin(async move {
                    events
                        .claim_pending_batch(transaction, 100, std::time::Duration::from_mins(1))
                        .await
                })
            })
            .await
            .expect("claim fixture events"),
        "claim fixture events",
    );
    if claimed.is_empty() {
        return;
    }
    let ids = claimed
        .into_iter()
        .map(|event| event.id)
        .collect::<Vec<_>>();
    let events = env.feed_events();
    env.write_scope()
        .run(move |transaction| {
            Box::pin(async move {
                events
                    .mark_pinged(transaction, &ids, UtcInstant::now())
                    .await
            })
        })
        .await
        .expect("complete fixture events");
}

async fn claimed_feed_paths(env: &storage::test_support::TestEnv) -> Vec<host::feed::FeedPath> {
    let events = env.feed_events();
    confirmed_for(
        env.write_scope()
            .run(move |transaction| {
                Box::pin(async move {
                    events
                        .claim_pending_batch(transaction, 100, std::time::Duration::from_mins(1))
                        .await
                })
            })
            .await
            .expect("claim mutation events"),
        "claim mutation events",
    )
    .into_iter()
    .map(|event| event.feed_path)
    .collect()
}

async fn create_named_audience(
    env: &storage::test_support::TestEnv,
    user_id: common::ids::UserId,
) -> common::ids::AudienceId {
    let audiences = env.audiences();
    let name = parse_audience_name("Friends");
    confirmed_for(
        env.write_scope()
            .run(move |transaction| {
                Box::pin(
                    async move { audiences.create_audience(transaction, user_id, &name).await },
                )
            })
            .await
            .expect("create named audience"),
        "create named audience",
    )
}

#[apply(backends)]
#[tokio::test]
async fn account_mutation_fanout_is_complete_distinct_and_public_only(#[case] backend: Backend) {
    let env = backend.setup().await;
    let user = SeedUser::new().seed(env.users(), env.write_scope()).await;
    let now = UtcInstant::from(
        "2026-09-21T12:00:00Z"
            .parse::<Timestamp>()
            .expect("fixed instant"),
    );
    let due = UtcInstant::from(
        now.value()
            .checked_sub(1.minute())
            .expect("fixture instant"),
    );
    let future = UtcInstant::from(
        now.value()
            .checked_add(1.minute())
            .expect("fixture instant"),
    );

    SeedRawPost::new(user.user_id)
        .published_at(due)
        .tags(["rust", "testing"])
        .seed(env.posts(), env.write_scope())
        .await;
    SeedRawPost::new(user.user_id)
        .published_at(due)
        .tags(["rust"])
        .seed(env.posts(), env.write_scope())
        .await;
    SeedRawPost::new(user.user_id)
        .draft()
        .tags(["draft"])
        .seed(env.posts(), env.write_scope())
        .await;
    SeedRawPost::new(user.user_id)
        .published_at(future)
        .tags(["scheduled"])
        .seed(env.posts(), env.write_scope())
        .await;
    let deleted = SeedRawPost::new(user.user_id)
        .published_at(due)
        .tags(["deleted"])
        .seed(env.posts(), env.write_scope())
        .await;
    let posts = env.posts();
    env.write_scope()
        .run(move |transaction| {
            Box::pin(async move {
                posts
                    .soft_delete_post(transaction, deleted.post_id, user.user_id, now)
                    .await
            })
        })
        .await
        .expect("delete fixture post");
    SeedRawPost::new(user.user_id)
        .published_at(due)
        .audiences(vec![AudienceTarget::Private])
        .tags(["private"])
        .seed(env.posts(), env.write_scope())
        .await;
    SeedRawPost::new(user.user_id)
        .published_at(due)
        .audiences(vec![AudienceTarget::Subscribers])
        .tags(["subscribers"])
        .seed(env.posts(), env.write_scope())
        .await;
    let audience = create_named_audience(&env, user.user_id).await;
    SeedRawPost::new(user.user_id)
        .published_at(due)
        .audiences(vec![AudienceTarget::Named(audience)])
        .tags(["named"])
        .seed(env.posts(), env.write_scope())
        .await;
    clear_feed_events(&env).await;

    let display_name = parse_display_name("Changed Name");
    let users = env.users();
    let posts = env.posts();
    let events = env.feed_events();
    let outcome = env
        .write_scope()
        .run(move |transaction| {
            Box::pin(async move {
                storage::update_profile_with_feed_events(
                    transaction,
                    users.as_ref(),
                    posts.as_ref(),
                    events.as_ref(),
                    user.user_id,
                    &ProfileUpdate {
                        display_name: Some(&display_name),
                        bio: None,
                    },
                    now,
                )
                .await
            })
        })
        .await
        .expect("display-name mutation");
    assert!(matches!(outcome, MutationOutcome::Confirmed(())));

    let mut actual = claimed_feed_paths(&env).await;
    let tags = [
        "rust".parse().expect("tag"),
        "testing".parse().expect("tag"),
    ];
    let mut expected = host::feed::affected_feed_urls(&user.username, tags.iter());
    actual.sort();
    expected.sort();
    assert_eq!(actual, expected);
    assert_eq!(
        actual.len(),
        18,
        "Site/User/SiteTag/UserTag × RSS/Atom/JSON"
    );
}

#[apply(backends)]
#[tokio::test]
async fn account_mutation_noops_and_no_public_posts_enqueue_nothing(#[case] backend: Backend) {
    let env = backend.setup().await;
    let user = SeedUser::new()
        .display_name("Same Name")
        .seed(env.users(), env.write_scope())
        .await;
    let now = UtcInstant::now();
    let display_name = parse_display_name("Same Name");
    let users = env.users();
    let posts = env.posts();
    let events = env.feed_events();
    env.write_scope()
        .run(move |transaction| {
            Box::pin(async move {
                storage::update_profile_with_feed_events(
                    transaction,
                    users.as_ref(),
                    posts.as_ref(),
                    events.as_ref(),
                    user.user_id,
                    &ProfileUpdate {
                        display_name: Some(&display_name),
                        bio: None,
                    },
                    now,
                )
                .await
            })
        })
        .await
        .expect("unchanged display name");
    assert!(claimed_feed_paths(&env).await.is_empty());

    let users = env.users();
    let config = env.user_config();
    let posts = env.posts();
    let events = env.feed_events();
    env.write_scope()
        .run(move |transaction| {
            Box::pin(async move {
                storage::update_content_license_with_feed_events(
                    transaction,
                    users.as_ref(),
                    config.as_ref(),
                    posts.as_ref(),
                    events.as_ref(),
                    storage::ContentLicenseUpdate {
                        user_id: user.user_id,
                        license: ContentLicense::AllRightsReserved,
                    },
                    now,
                )
                .await
            })
        })
        .await
        .expect("unchanged content license");
    assert!(claimed_feed_paths(&env).await.is_empty());

    let users = env.users();
    let config = env.user_config();
    let posts = env.posts();
    let events = env.feed_events();
    env.write_scope()
        .run(move |transaction| {
            Box::pin(async move {
                storage::update_content_license_with_feed_events(
                    transaction,
                    users.as_ref(),
                    config.as_ref(),
                    posts.as_ref(),
                    events.as_ref(),
                    storage::ContentLicenseUpdate {
                        user_id: user.user_id,
                        license: ContentLicense::CcBy4_0,
                    },
                    now,
                )
                .await
            })
        })
        .await
        .expect("changed content license without public posts");
    assert!(claimed_feed_paths(&env).await.is_empty());

    SeedRawPost::new(user.user_id)
        .published_at(now)
        .tags(["rights"])
        .seed(env.posts(), env.write_scope())
        .await;
    clear_feed_events(&env).await;

    let display_name = parse_display_name("Same Name");
    let users = env.users();
    let posts = env.posts();
    let events = env.feed_events();
    env.write_scope()
        .run(move |transaction| {
            Box::pin(async move {
                storage::update_profile_with_feed_events(
                    transaction,
                    users.as_ref(),
                    posts.as_ref(),
                    events.as_ref(),
                    user.user_id,
                    &ProfileUpdate {
                        display_name: Some(&display_name),
                        bio: None,
                    },
                    now,
                )
                .await
            })
        })
        .await
        .expect("unchanged display name with a public post");
    assert!(claimed_feed_paths(&env).await.is_empty());

    let users = env.users();
    let config = env.user_config();
    let posts = env.posts();
    let events = env.feed_events();
    env.write_scope()
        .run(move |transaction| {
            Box::pin(async move {
                storage::update_content_license_with_feed_events(
                    transaction,
                    users.as_ref(),
                    config.as_ref(),
                    posts.as_ref(),
                    events.as_ref(),
                    storage::ContentLicenseUpdate {
                        user_id: user.user_id,
                        license: ContentLicense::CcBy4_0,
                    },
                    now,
                )
                .await
            })
        })
        .await
        .expect("unchanged content license with a public post");
    assert!(claimed_feed_paths(&env).await.is_empty());

    let users = env.users();
    let config = env.user_config();
    let posts = env.posts();
    let events = env.feed_events();
    env.write_scope()
        .run(move |transaction| {
            Box::pin(async move {
                storage::update_content_license_with_feed_events(
                    transaction,
                    users.as_ref(),
                    config.as_ref(),
                    posts.as_ref(),
                    events.as_ref(),
                    storage::ContentLicenseUpdate {
                        user_id: user.user_id,
                        license: ContentLicense::CcBySa4_0,
                    },
                    now,
                )
                .await
            })
        })
        .await
        .expect("changed content license with public posts");
    let mut actual = claimed_feed_paths(&env).await;
    let tags = ["rights".parse().expect("tag")];
    let mut expected = host::feed::affected_feed_urls(&user.username, tags.iter());
    actual.sort();
    expected.sort();
    assert_eq!(actual, expected);
}

#[apply(backends)]
#[tokio::test]
async fn account_mutation_enqueue_failure_rolls_back_profile_and_content_license(
    #[case] backend: Backend,
) {
    let env = backend.setup().await;
    let user = SeedUser::new().seed(env.users(), env.write_scope()).await;
    SeedRawPost::new(user.user_id)
        .seed(env.posts(), env.write_scope())
        .await;
    clear_feed_events(&env).await;
    let now = UtcInstant::now();

    let mut failing_events = MockFeedEventStorage::new();
    failing_events
        .expect_enqueue_many()
        .returning(|_, _| Err(FeedEventError::Db(sqlx::Error::RowNotFound)));
    let display_name = parse_display_name("Not Persisted");
    let users = env.users();
    let posts = env.posts();
    let result = env
        .write_scope()
        .run(move |transaction| {
            Box::pin(async move {
                storage::update_profile_with_feed_events(
                    transaction,
                    users.as_ref(),
                    posts.as_ref(),
                    &failing_events,
                    user.user_id,
                    &ProfileUpdate {
                        display_name: Some(&display_name),
                        bio: None,
                    },
                    now,
                )
                .await
            })
        })
        .await;
    assert!(matches!(
        result,
        Err(storage::WriteScopeError::Operation(_))
    ));
    assert_eq!(
        env.users()
            .get_user(user.user_id)
            .await
            .expect("read user")
            .expect("user")
            .display_name,
        None
    );

    let mut failing_events = MockFeedEventStorage::new();
    failing_events
        .expect_enqueue_many()
        .returning(|_, _| Err(FeedEventError::Db(sqlx::Error::RowNotFound)));
    let users = env.users();
    let config = env.user_config();
    let posts = env.posts();
    let result = env
        .write_scope()
        .run(move |transaction| {
            Box::pin(async move {
                storage::update_content_license_with_feed_events(
                    transaction,
                    users.as_ref(),
                    config.as_ref(),
                    posts.as_ref(),
                    &failing_events,
                    storage::ContentLicenseUpdate {
                        user_id: user.user_id,
                        license: ContentLicense::CcBy4_0,
                    },
                    now,
                )
                .await
            })
        })
        .await;
    assert!(matches!(
        result,
        Err(storage::WriteScopeError::Operation(_))
    ));
    assert_eq!(
        env.user_config()
            .get_content_license(user.user_id)
            .await
            .expect("read license"),
        ContentLicense::AllRightsReserved
    );
}
