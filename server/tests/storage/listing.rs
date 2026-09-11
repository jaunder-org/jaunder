use common::{
    ids::{AudienceId, PostId, UserId},
    tag::{Tag, TagLabel},
    test_support::{
        parse_audience_name, parse_display_name, parse_etag, parse_post_body, parse_row_limit,
        permalink_date,
    },
    time::UtcInstant,
    username::Username,
    visibility::{AudienceTarget, ViewerIdentity},
};
use jiff::{Span, Timestamp, ToSpan, tz::Offset};
use std::sync::Arc;
use storage::test_support::{
    Backend, SeedFeedCache, SeedRawPost, SeedUser, backends, confirmed_for as confirmed, fp,
    seed_local_subscription,
};
use storage::{
    AudienceStorage, DraftPostCursor, FeedEventStorage, GoLivePost, ListByTagError,
    PostBookkeepingExpectation, PostCursor, PostFormat, PostRecord, PostStorage, ProfileUpdate,
    RenderedPostContent, WriteScope, create_rendered_post,
};

use rstest::*;
use rstest_reuse::*;

use super::fixtures::{anon_by_tag, anon_published};
fn fixed_instant(value: &str) -> UtcInstant {
    UtcInstant::from(value.parse::<Timestamp>().expect("fixed test instant"))
}

fn add(instant: UtcInstant, span: Span) -> UtcInstant {
    UtcInstant::from(
        instant
            .value()
            .checked_add(span)
            .expect("fixture is within Timestamp range"),
    )
}

fn subtract(instant: UtcInstant, span: Span) -> UtcInstant {
    UtcInstant::from(
        instant
            .value()
            .checked_sub(span)
            .expect("fixture is within Timestamp range"),
    )
}

async fn soft_delete_post_confirmed(
    posts: Arc<dyn PostStorage>,
    write_scope: WriteScope,
    post_id: PostId,
    user_id: UserId,
) {
    let outcome = write_scope
        .run(move |transaction| {
            Box::pin(async move {
                posts
                    .soft_delete_post(
                        transaction,
                        post_id,
                        user_id,
                        common::time::UtcInstant::now(),
                    )
                    .await
            })
        })
        .await
        .expect("soft_delete_post failed");
    confirmed(outcome, "post deletion");
}

async fn create_named_audience(
    audiences: Arc<dyn AudienceStorage>,
    write_scope: WriteScope,
    author: UserId,
    name: &str,
) -> AudienceId {
    let name = parse_audience_name(name);
    let outcome = write_scope
        .run(move |transaction| {
            Box::pin(async move { audiences.create_audience(transaction, author, &name).await })
        })
        .await
        .expect("create named audience");
    confirmed(outcome, "named audience fixture")
}

async fn anon_user_by_tag(
    posts: Arc<dyn PostStorage>,
    user_id: UserId,
    tag: &Tag,
    limit: &str,
) -> Vec<PostRecord> {
    posts
        .list_user_posts_by_tag(
            user_id,
            tag,
            storage::PublishedPageRequest::first(
                common::seed::TimelineOrder::Newest,
                parse_row_limit(limit),
            ),
            &ViewerIdentity::Anonymous,
            common::time::UtcInstant::now(),
        )
        .await
        .expect("list_user_posts_by_tag failed")
}

async fn anon_published_by_user(
    posts: Arc<dyn PostStorage>,
    username: &Username,
    limit: &str,
) -> Vec<PostRecord> {
    posts
        .list_published_by_user(
            username,
            storage::PublishedPageRequest::first(
                common::seed::TimelineOrder::Newest,
                parse_row_limit(limit),
            ),
            &ViewerIdentity::Anonymous,
            common::time::UtcInstant::now(),
        )
        .await
        .expect("list_published_by_user failed")
}

async fn drafts_of(posts: Arc<dyn PostStorage>, user_id: UserId, limit: &str) -> Vec<PostRecord> {
    posts
        .list_drafts_by_user(
            user_id,
            None,
            parse_row_limit(limit),
            common::time::UtcInstant::now(),
        )
        .await
        .expect("list_drafts_by_user failed")
}

/// Creates a public post for `user_id` with an explicit `published_at`, returning
/// the new post id. A future `published_at` seeds a *scheduled* post (publicly
/// invisible until its time); a past one a live post. Lets the boundary tests
/// below pin the publication instant relative to the injected `now`.
async fn seed_post_published_at(
    posts: Arc<dyn PostStorage>,
    feed_events: Arc<dyn FeedEventStorage>,
    write_scope: WriteScope,
    user_id: UserId,
    slug: &str,
    published_at: common::time::UtcInstant,
) -> PostId {
    confirmed(
        create_rendered_post(
            &write_scope,
            &storage::test_support::fixture_media_content_locks(),
            posts,
            feed_events,
            RenderedPostContent {
                user_id,
                title: None,
                slug: slug.parse().expect("valid slug"),
                body: parse_post_body(&format!("# {slug}\n\nbody")),
                format: PostFormat::Markdown,
                published_at: Some(published_at),
                summary: None,
                audiences: vec![AudienceTarget::Public],
                tags: vec![],
                idempotency_key: None,
                expectations: PostBookkeepingExpectation::default(),
            },
            published_at,
        )
        .await
        .expect("seed post should be created"),
        "seed post creation",
    )
    .post_id
}

// Scheduled-publishing boundary tests (issue #70): each public read must hide a
// future-dated post (`published_at > now`) and reveal it once `now` reaches its
// `published_at`. One common test per surface, both backends, fixed injected
// `now` (no sleeps) asserting both sides of the `<= now` boundary.

#[apply(backends)]
#[tokio::test]
async fn permalink_hides_scheduled_until_due(#[case] backend: Backend) {
    let env = backend.setup().await;
    let now = fixed_instant("2026-06-26T12:00:00Z");
    let user = SeedUser::new().seed(env.users(), env.write_scope()).await;
    seed_post_published_at(
        Arc::clone(&env.posts()),
        Arc::clone(&env.feed_events()),
        env.write_scope(),
        user.user_id,
        "live-one",
        subtract(now, 1.hour()),
    )
    .await;
    seed_post_published_at(
        Arc::clone(&env.posts()),
        Arc::clone(&env.feed_events()),
        env.write_scope(),
        user.user_id,
        "sched-one",
        add(now, 1.hour()),
    )
    .await;

    // At `now`: the live post is visible, the scheduled one is not.
    let got_live = env
        .posts()
        .get_post_by_permalink(
            &user.username,
            permalink_date(2026, 6, 26),
            &"live-one".parse().unwrap(),
            &ViewerIdentity::Anonymous,
            now,
        )
        .await
        .unwrap();
    assert!(got_live.is_some(), "live post must be visible at now");
    let got_sched = env
        .posts()
        .get_post_by_permalink(
            &user.username,
            permalink_date(2026, 6, 26),
            &"sched-one".parse().unwrap(),
            &ViewerIdentity::Anonymous,
            now,
        )
        .await
        .unwrap();
    assert!(
        got_sched.is_none(),
        "scheduled post must be hidden before its time"
    );

    // Exactly at go-live, the scheduled post appears (locks the `<= now`
    // boundary shared with the unpublished lookup's strict `> now` predicate).
    let due = add(now, 1.hour());
    let got_after = env
        .posts()
        .get_post_by_permalink(
            &user.username,
            permalink_date(2026, 6, 26),
            &"sched-one".parse().unwrap(),
            &ViewerIdentity::Anonymous,
            due,
        )
        .await
        .unwrap();
    assert!(
        got_after.is_some(),
        "scheduled post must appear once now >= published_at"
    );
}

#[apply(backends)]
#[tokio::test]
async fn list_published_by_user_hides_scheduled_until_due(#[case] backend: Backend) {
    let env = backend.setup().await;
    let now = fixed_instant("2026-06-26T12:00:00Z");
    let user = SeedUser::new().seed(env.users(), env.write_scope()).await;
    let live = seed_post_published_at(
        Arc::clone(&env.posts()),
        Arc::clone(&env.feed_events()),
        env.write_scope(),
        user.user_id,
        "live-one",
        subtract(now, 1.hour()),
    )
    .await;
    let sched = seed_post_published_at(
        Arc::clone(&env.posts()),
        Arc::clone(&env.feed_events()),
        env.write_scope(),
        user.user_id,
        "sched-one",
        add(now, 1.hour()),
    )
    .await;

    let at_now = env
        .posts()
        .list_published_by_user(
            &user.username,
            storage::PublishedPageRequest::first(
                common::seed::TimelineOrder::Newest,
                parse_row_limit("50"),
            ),
            &ViewerIdentity::Anonymous,
            now,
        )
        .await
        .unwrap();
    let ids_now: Vec<PostId> = at_now.iter().map(|p| p.post_id).collect();
    assert!(ids_now.contains(&live), "live post must be listed at now");
    assert!(
        !ids_now.contains(&sched),
        "scheduled post must be hidden before its time"
    );

    let after = add(add(now, 1.hour()), 1.second());
    let at_after = env
        .posts()
        .list_published_by_user(
            &user.username,
            storage::PublishedPageRequest::first(
                common::seed::TimelineOrder::Newest,
                parse_row_limit("50"),
            ),
            &ViewerIdentity::Anonymous,
            after,
        )
        .await
        .unwrap();
    assert!(
        at_after.iter().any(|p| p.post_id == sched),
        "scheduled post must be listed once now >= published_at"
    );
}

#[apply(backends)]
#[tokio::test]
async fn list_published_hides_scheduled_until_due(#[case] backend: Backend) {
    let env = backend.setup().await;
    let now = fixed_instant("2026-06-26T12:00:00Z");
    let user_id = SeedUser::new()
        .seed(env.users(), env.write_scope())
        .await
        .user_id;
    let live = seed_post_published_at(
        Arc::clone(&env.posts()),
        Arc::clone(&env.feed_events()),
        env.write_scope(),
        user_id,
        "live-one",
        subtract(now, 1.hour()),
    )
    .await;
    let sched = seed_post_published_at(
        Arc::clone(&env.posts()),
        Arc::clone(&env.feed_events()),
        env.write_scope(),
        user_id,
        "sched-one",
        add(now, 1.hour()),
    )
    .await;

    let at_now = env
        .posts()
        .list_published(
            storage::PublishedPageRequest::first(
                common::seed::TimelineOrder::Newest,
                parse_row_limit("50"),
            ),
            &ViewerIdentity::Anonymous,
            now,
        )
        .await
        .unwrap();
    let ids_now: Vec<PostId> = at_now.iter().map(|p| p.post_id).collect();
    assert!(ids_now.contains(&live), "live post must be listed at now");
    assert!(
        !ids_now.contains(&sched),
        "scheduled post must be hidden before its time"
    );

    let after = add(add(now, 1.hour()), 1.second());
    let at_after = env
        .posts()
        .list_published(
            storage::PublishedPageRequest::first(
                common::seed::TimelineOrder::Newest,
                parse_row_limit("50"),
            ),
            &ViewerIdentity::Anonymous,
            after,
        )
        .await
        .unwrap();
    assert!(
        at_after.iter().any(|p| p.post_id == sched),
        "scheduled post must be listed once now >= published_at"
    );
}

#[apply(backends)]
#[tokio::test]
async fn site_post_timeline_newest_orders_by_publication_time(#[case] backend: Backend) {
    let env = backend.setup().await;
    let now = fixed_instant("2026-06-26T12:00:00Z");
    let user_id = SeedUser::new()
        .seed(env.users(), env.write_scope())
        .await
        .user_id;
    let later_publication = SeedRawPost::new(user_id)
        .published_at(subtract(now, 1.hour()))
        .seed(env.posts(), env.write_scope())
        .await
        .post_id;
    // This later-created Post deliberately carries the earlier displayed time.
    let earlier_publication = SeedRawPost::new(user_id)
        .published_at(subtract(now, 2.hours()))
        .seed(env.posts(), env.write_scope())
        .await
        .post_id;

    let posts = env
        .posts()
        .list_published(
            storage::PublishedPageRequest::first(
                common::seed::TimelineOrder::Newest,
                parse_row_limit("50"),
            ),
            &ViewerIdentity::Anonymous,
            now,
        )
        .await
        .expect("list published site timeline");

    assert_eq!(
        posts.iter().map(|post| post.post_id).collect::<Vec<_>>(),
        vec![later_publication, earlier_publication],
        "Newest site Post timeline follows the displayed publication time"
    );
}

#[apply(backends)]
#[tokio::test]
async fn list_posts_by_tag_hides_scheduled_until_due(#[case] backend: Backend) {
    let env = backend.setup().await;
    let now = fixed_instant("2026-06-26T12:00:00Z");
    let user_id = SeedUser::new()
        .seed(env.users(), env.write_scope())
        .await
        .user_id;
    let live = seed_post_published_at(
        Arc::clone(&env.posts()),
        Arc::clone(&env.feed_events()),
        env.write_scope(),
        user_id,
        "live-one",
        subtract(now, 1.hour()),
    )
    .await;
    let sched = seed_post_published_at(
        Arc::clone(&env.posts()),
        Arc::clone(&env.feed_events()),
        env.write_scope(),
        user_id,
        "sched-one",
        add(now, 1.hour()),
    )
    .await;
    storage::test_support::set_post_tags_confirmed(
        &env.write_scope(),
        std::sync::Arc::clone(&env.posts()),
        live,
        user_id,
        &["scheduling".parse::<TagLabel>().unwrap()],
    )
    .await
    .unwrap();
    storage::test_support::set_post_tags_confirmed(
        &env.write_scope(),
        std::sync::Arc::clone(&env.posts()),
        sched,
        user_id,
        &["scheduling".parse::<TagLabel>().unwrap()],
    )
    .await
    .unwrap();
    let tag_slug: Tag = "scheduling".parse().unwrap();

    let at_now = env
        .posts()
        .list_posts_by_tag(
            &tag_slug,
            storage::PublishedPageRequest::first(
                common::seed::TimelineOrder::Newest,
                parse_row_limit("50"),
            ),
            &ViewerIdentity::Anonymous,
            now,
        )
        .await
        .unwrap();
    let ids_now: Vec<PostId> = at_now.iter().map(|p| p.post_id).collect();
    assert!(ids_now.contains(&live), "live post must be listed at now");
    assert!(
        !ids_now.contains(&sched),
        "scheduled post must be hidden before its time"
    );

    let after = add(add(now, 1.hour()), 1.second());
    let at_after = env
        .posts()
        .list_posts_by_tag(
            &tag_slug,
            storage::PublishedPageRequest::first(
                common::seed::TimelineOrder::Newest,
                parse_row_limit("50"),
            ),
            &ViewerIdentity::Anonymous,
            after,
        )
        .await
        .unwrap();
    assert!(
        at_after.iter().any(|p| p.post_id == sched),
        "scheduled post must be listed once now >= published_at"
    );
}

#[apply(backends)]
#[tokio::test]
async fn list_user_posts_by_tag_hides_scheduled_until_due(#[case] backend: Backend) {
    let env = backend.setup().await;
    let now = fixed_instant("2026-06-26T12:00:00Z");
    let user_id = SeedUser::new()
        .seed(env.users(), env.write_scope())
        .await
        .user_id;
    let live = seed_post_published_at(
        Arc::clone(&env.posts()),
        Arc::clone(&env.feed_events()),
        env.write_scope(),
        user_id,
        "live-one",
        subtract(now, 1.hour()),
    )
    .await;
    let sched = seed_post_published_at(
        Arc::clone(&env.posts()),
        Arc::clone(&env.feed_events()),
        env.write_scope(),
        user_id,
        "sched-one",
        add(now, 1.hour()),
    )
    .await;
    storage::test_support::set_post_tags_confirmed(
        &env.write_scope(),
        std::sync::Arc::clone(&env.posts()),
        live,
        user_id,
        &["scheduling".parse::<TagLabel>().unwrap()],
    )
    .await
    .unwrap();
    storage::test_support::set_post_tags_confirmed(
        &env.write_scope(),
        std::sync::Arc::clone(&env.posts()),
        sched,
        user_id,
        &["scheduling".parse::<TagLabel>().unwrap()],
    )
    .await
    .unwrap();
    let tag_slug: Tag = "scheduling".parse().unwrap();

    let at_now = env
        .posts()
        .list_user_posts_by_tag(
            user_id,
            &tag_slug,
            storage::PublishedPageRequest::first(
                common::seed::TimelineOrder::Newest,
                parse_row_limit("50"),
            ),
            &ViewerIdentity::Anonymous,
            now,
        )
        .await
        .unwrap();
    let ids_now: Vec<PostId> = at_now.iter().map(|p| p.post_id).collect();
    assert!(ids_now.contains(&live), "live post must be listed at now");
    assert!(
        !ids_now.contains(&sched),
        "scheduled post must be hidden before its time"
    );

    let after = add(add(now, 1.hour()), 1.second());
    let at_after = env
        .posts()
        .list_user_posts_by_tag(
            user_id,
            &tag_slug,
            storage::PublishedPageRequest::first(
                common::seed::TimelineOrder::Newest,
                parse_row_limit("50"),
            ),
            &ViewerIdentity::Anonymous,
            after,
        )
        .await
        .unwrap();
    assert!(
        at_after.iter().any(|p| p.post_id == sched),
        "scheduled post must be listed once now >= published_at"
    );
}

#[apply(backends)]
#[tokio::test]
async fn soft_delete_excludes_post_from_lists(#[case] backend: Backend) {
    let env = backend.setup().await;
    let user_id = SeedUser::new()
        .seed(env.users(), env.write_scope())
        .await
        .user_id;

    let post_id = SeedRawPost::new(user_id)
        .seed(env.posts(), env.write_scope())
        .await
        .post_id;

    let published = anon_published(env.posts(), "10").await;
    assert!(published.iter().any(|p| p.post_id == post_id));

    soft_delete_post_confirmed(
        Arc::clone(&env.posts()),
        env.write_scope(),
        post_id,
        user_id,
    )
    .await;

    let published = anon_published(env.posts(), "10").await;
    assert!(!published.iter().any(|p| p.post_id == post_id));

    let record = env
        .posts()
        .get_post_by_id(post_id, &ViewerIdentity::Anonymous)
        .await
        .unwrap()
        .unwrap();
    assert!(record.deleted_at.is_some());
}

#[apply(backends)]
#[tokio::test]
async fn list_published_in_window_applies_hybrid_rule_across_surfaces(#[case] backend: Backend) {
    use common::feed::FeedSurface;
    use host::{
        feed::HybridWindow,
        test_support::{parse_feed_min_days, parse_feed_min_items},
    };

    let env = backend.setup().await;

    let alice = SeedUser::new().seed(env.users(), env.write_scope()).await;
    let bob = SeedUser::new().seed(env.users(), env.write_scope()).await;
    let alice_id = alice.user_id;
    let bob_id = bob.user_id;

    let now = UtcInstant::now();
    let make_post = |user_id: UserId, days_ago: i64| {
        SeedRawPost::new(user_id).published_at(subtract(now, (days_ago * 24).hours()))
    };

    // Alice: 4 posts published 1, 2, 100, 200 days ago.
    let alice_recent_1 = make_post(alice_id, 1)
        .seed(env.posts(), env.write_scope())
        .await;
    make_post(alice_id, 2)
        .seed(env.posts(), env.write_scope())
        .await;
    make_post(alice_id, 100)
        .seed(env.posts(), env.write_scope())
        .await;
    make_post(alice_id, 200)
        .seed(env.posts(), env.write_scope())
        .await;

    // Bob: 1 post published 5 days ago.
    make_post(bob_id, 5)
        .seed(env.posts(), env.write_scope())
        .await;

    // Future-dated draft-equivalent (excluded).
    make_post(alice_id, -1)
        .seed(env.posts(), env.write_scope())
        .await;

    // Site feed, window {3 items, 30 days} → union of "top 3" and "in last 30
    // days". Alice 1d+2d and Bob 5d are in-window (3 posts). Alice 100d/200d
    // and the future post are excluded by their respective filters; the union
    // still picks at least 3 by ROW_NUMBER, so we get exactly those 3.
    let window = HybridWindow {
        min_items: parse_feed_min_items("3"),
        min_days: parse_feed_min_days("30"),
    };
    let site = env
        .posts()
        .list_published_in_window(&FeedSurface::Site, &window, now, &ViewerIdentity::Anonymous)
        .await
        .unwrap();
    assert_eq!(site.len(), 3, "site feed in {{3 items, 30 days}}");
    assert!(
        site.iter()
            .all(|p| p.published_at.unwrap().value() >= subtract(now, 720.hours()).value())
    );

    // Site feed with min_items=5: top 5 includes all four real posts plus
    // Bob's, regardless of age — total 5 (alice-old-2 included by count).
    let big = HybridWindow {
        min_items: parse_feed_min_items("5"),
        min_days: parse_feed_min_days("30"),
    };
    let site_big = env
        .posts()
        .list_published_in_window(&FeedSurface::Site, &big, now, &ViewerIdentity::Anonymous)
        .await
        .unwrap();
    assert_eq!(site_big.len(), 5, "min_items=5 pulls in older posts");

    // User feed for Alice, {2 items, 30 days}: union of "Alice's top 2"
    // (alice-recent-1, alice-recent-2) and "Alice's posts in last 30 days"
    // (same two) → 2. The 100/200-day-old posts and future are excluded.
    let alice_window = HybridWindow {
        min_items: parse_feed_min_items("2"),
        min_days: parse_feed_min_days("30"),
    };
    let alice_feed = env
        .posts()
        .list_published_in_window(
            &FeedSurface::User {
                username: alice.username.clone(),
            },
            &alice_window,
            now,
            &ViewerIdentity::Anonymous,
        )
        .await
        .unwrap();
    assert_eq!(alice_feed.len(), 2);
    assert!(alice_feed.iter().all(|p| p.user_id == alice_id));

    // User feed: bob has only 1 post, returned even with min_items=10.
    let bob_feed = env
        .posts()
        .list_published_in_window(
            &FeedSurface::User {
                username: bob.username.clone(),
            },
            &HybridWindow {
                min_items: parse_feed_min_items("10"),
                min_days: parse_feed_min_days("1"),
            },
            now,
            &ViewerIdentity::Anonymous,
        )
        .await
        .unwrap();
    assert_eq!(bob_feed.len(), 1);
    assert_eq!(bob_feed[0].user_id, bob_id);

    // Add a tag to alice-recent-1 and verify site-tag / user-tag feeds.
    storage::test_support::set_post_tags_confirmed(
        &env.write_scope(),
        std::sync::Arc::clone(&env.posts()),
        alice_recent_1.post_id,
        alice_id,
        &["rust".parse::<TagLabel>().unwrap()],
    )
    .await
    .unwrap();

    let tag_site = env
        .posts()
        .list_published_in_window(
            &FeedSurface::SiteTag {
                tag: "rust".parse().unwrap(),
            },
            &HybridWindow {
                min_items: parse_feed_min_items("20"),
                min_days: parse_feed_min_days("30"),
            },
            now,
            &ViewerIdentity::Anonymous,
        )
        .await
        .unwrap();
    assert_eq!(tag_site.len(), 1);
    assert_eq!(tag_site[0].slug, alice_recent_1.slug);

    let tag_user = env
        .posts()
        .list_published_in_window(
            &FeedSurface::UserTag {
                username: alice.username.clone(),
                tag: "rust".parse().unwrap(),
            },
            &HybridWindow {
                min_items: parse_feed_min_items("20"),
                min_days: parse_feed_min_days("30"),
            },
            now,
            &ViewerIdentity::Anonymous,
        )
        .await
        .unwrap();
    assert_eq!(tag_user.len(), 1);

    // User-tag for bob+rust: bob has no rust post → empty.
    let bob_tag = env
        .posts()
        .list_published_in_window(
            &FeedSurface::UserTag {
                username: bob.username.clone(),
                tag: "rust".parse().unwrap(),
            },
            &HybridWindow {
                min_items: parse_feed_min_items("20"),
                min_days: parse_feed_min_days("30"),
            },
            now,
            &ViewerIdentity::Anonymous,
        )
        .await
        .unwrap();
    assert!(bob_tag.is_empty());
}

#[apply(backends)]
#[tokio::test]
async fn list_published_in_window_with_unrepresentable_cutoff_keeps_eligible_history(
    #[case] backend: Backend,
) {
    use common::feed::FeedSurface;
    use host::{
        feed::HybridWindow,
        test_support::{parse_feed_min_days, parse_feed_min_items},
    };

    let env = backend.setup().await;
    let author = SeedUser::new().seed(env.users(), env.write_scope()).await;
    let now = UtcInstant::now();
    let publish = |days_ago: i64| {
        SeedRawPost::new(author.user_id).published_at(subtract(now, (days_ago * 24).hours()))
    };

    let yesterday = publish(1).seed(env.posts(), env.write_scope()).await;
    let last_month = publish(31).seed(env.posts(), env.write_scope()).await;
    let last_year = publish(365).seed(env.posts(), env.write_scope()).await;
    publish(-1).seed(env.posts(), env.write_scope()).await;

    let posts = env
        .posts()
        .list_published_in_window(
            &FeedSurface::Site,
            &HybridWindow {
                min_items: parse_feed_min_items("1"),
                min_days: parse_feed_min_days(&u32::MAX.to_string()),
            },
            now,
            &ViewerIdentity::Anonymous,
        )
        .await
        .unwrap();

    // An unrepresentably old cutoff is all eligible history, never an overflow.
    let actual: Vec<_> = posts.iter().map(|post| post.post_id).collect();
    let expected = vec![yesterday.post_id, last_month.post_id, last_year.post_id];
    assert_eq!(actual, expected);
}

#[apply(backends)]
#[tokio::test]
async fn list_published_in_window_resolves_viewers_before_ranking(#[case] backend: Backend) {
    use common::feed::FeedSurface;
    use host::{
        feed::HybridWindow,
        test_support::{parse_feed_min_days, parse_feed_min_items},
    };

    let env = backend.setup().await;
    let alice = SeedUser::new().seed(env.users(), env.write_scope()).await;
    let bob = SeedUser::new().seed(env.users(), env.write_scope()).await;
    let now = UtcInstant::now();
    let public = SeedRawPost::new(alice.user_id)
        .published_at(subtract(now, 2_160.hours()))
        .audiences(vec![AudienceTarget::Public])
        .seed(env.posts(), env.write_scope())
        .await
        .post_id;
    let subscribers = SeedRawPost::new(alice.user_id)
        .published_at(subtract(now, 2_184.hours()))
        .audiences(vec![AudienceTarget::Subscribers])
        .seed(env.posts(), env.write_scope())
        .await
        .post_id;
    let private = SeedRawPost::new(alice.user_id)
        .published_at(subtract(now, 24.hours()))
        .audiences(vec![])
        .seed(env.posts(), env.write_scope())
        .await
        .post_id;
    for post_id in [public, subscribers, private] {
        storage::test_support::set_post_tags_confirmed(
            &env.write_scope(),
            Arc::clone(&env.posts()),
            post_id,
            alice.user_id,
            &["rust".parse::<TagLabel>().unwrap()],
        )
        .await
        .expect("tag hybrid-window fixture");
    }

    seed_local_subscription(
        env.subscriptions(),
        env.write_scope(),
        alice.user_id,
        bob.user_id,
    )
    .await;

    let surfaces = [
        FeedSurface::Site,
        FeedSurface::User {
            username: alice.username.clone(),
        },
        FeedSurface::SiteTag {
            tag: "rust".parse().unwrap(),
        },
        FeedSurface::UserTag {
            username: alice.username.clone(),
            tag: "rust".parse().unwrap(),
        },
    ];
    let anonymous_window = HybridWindow {
        min_items: parse_feed_min_items("1"),
        min_days: parse_feed_min_days("30"),
    };
    let authenticated_window = HybridWindow {
        min_items: parse_feed_min_items("2"),
        min_days: parse_feed_min_days("30"),
    };
    let authenticated = ViewerIdentity::local(bob.user_id);

    for surface in surfaces {
        let anonymous = env
            .posts()
            .list_published_in_window(&surface, &anonymous_window, now, &ViewerIdentity::Anonymous)
            .await
            .expect("list anonymous hybrid window");
        assert_eq!(
            anonymous
                .iter()
                .map(|post| post.post_id)
                .collect::<Vec<_>>(),
            vec![public],
            "{surface:?}: the older Public post still satisfies the count floor"
        );

        let visible_to_subscriber = env
            .posts()
            .list_published_in_window(&surface, &authenticated_window, now, &authenticated)
            .await
            .expect("list authenticated hybrid window");
        assert_eq!(
            visible_to_subscriber
                .iter()
                .map(|post| post.post_id)
                .collect::<Vec<_>>(),
            vec![public, subscribers],
            "{surface:?}: the newer Private post cannot consume either visible count slot"
        );
    }
}

#[apply(backends)]
#[tokio::test]
async fn list_published_by_user_returns_only_user_posts(#[case] backend: Backend) {
    let env = backend.setup().await;
    let alice = SeedUser::new().seed(env.users(), env.write_scope()).await;
    let bob = SeedUser::new().seed(env.users(), env.write_scope()).await;
    let alice_id = alice.user_id;
    let bob_id = bob.user_id;

    SeedRawPost::new(alice_id)
        .seed(env.posts(), env.write_scope())
        .await;
    SeedRawPost::new(alice_id)
        .seed(env.posts(), env.write_scope())
        .await;
    SeedRawPost::new(bob_id)
        .seed(env.posts(), env.write_scope())
        .await;

    let alice_posts = anon_published_by_user(Arc::clone(&env.posts()), &alice.username, "10").await;
    assert_eq!(alice_posts.len(), 2);
    assert!(alice_posts.iter().all(|p| p.user_id == alice_id));

    let bob_posts = anon_published_by_user(Arc::clone(&env.posts()), &bob.username, "10").await;
    assert_eq!(bob_posts.len(), 1);
    assert_eq!(bob_posts[0].user_id, bob_id);
}

#[apply(backends)]
#[tokio::test]
async fn list_published_by_user_uses_current_display_name(#[case] backend: Backend) {
    let env = backend.setup().await;
    let user = SeedUser::new()
        .display_name("Old Name")
        .seed(env.users(), env.write_scope())
        .await;
    let post = SeedRawPost::new(user.user_id)
        .seed(env.posts(), env.write_scope())
        .await;
    let users = Arc::clone(&env.users());
    let user_id = user.user_id;
    let new_display_name = parse_display_name("New Name");
    let outcome = env
        .write_scope()
        .run(move |transaction| {
            Box::pin(async move {
                users
                    .update_profile(
                        transaction,
                        user_id,
                        &ProfileUpdate {
                            display_name: Some(&new_display_name),
                            bio: None,
                        },
                    )
                    .await
            })
        })
        .await
        .expect("profile update should succeed");
    confirmed(outcome, "profile update");

    let posts = anon_published_by_user(Arc::clone(&env.posts()), &user.username, "10").await;
    let listed = posts
        .iter()
        .find(|listed| listed.post_id == post.post_id)
        .expect("existing post should remain listed");
    assert_eq!(listed.author_display_name.as_deref(), Some("New Name"));
}

#[apply(backends)]
#[tokio::test]
async fn list_published_returns_published_non_deleted_posts(#[case] backend: Backend) {
    let env = backend.setup().await;
    let user_id = SeedUser::new()
        .seed(env.users(), env.write_scope())
        .await
        .user_id;

    // Create a draft (should not appear)
    SeedRawPost::new(user_id)
        .draft()
        .seed(env.posts(), env.write_scope())
        .await;

    SeedRawPost::new(user_id)
        .seed(env.posts(), env.write_scope())
        .await;
    SeedRawPost::new(user_id)
        .seed(env.posts(), env.write_scope())
        .await;

    let published = anon_published(env.posts(), "10").await;
    assert_eq!(published.len(), 2);
    assert!(published.iter().all(|p| p.published_at.is_some()));
}

#[apply(backends)]
#[tokio::test]
async fn list_drafts_by_user_returns_only_drafts(#[case] backend: Backend) {
    let env = backend.setup().await;
    let user_id = SeedUser::new()
        .seed(env.users(), env.write_scope())
        .await
        .user_id;

    SeedRawPost::new(user_id)
        .draft()
        .seed(env.posts(), env.write_scope())
        .await;
    SeedRawPost::new(user_id)
        .draft()
        .seed(env.posts(), env.write_scope())
        .await;

    // Create a published post (should not appear in drafts)
    SeedRawPost::new(user_id)
        .seed(env.posts(), env.write_scope())
        .await;

    let drafts = drafts_of(Arc::clone(&env.posts()), user_id, "10").await;
    assert_eq!(drafts.len(), 2);
    assert!(drafts.iter().all(|p| p.published_at.is_none()));
    assert!(drafts.iter().all(|p| p.user_id == user_id));
}

// The author's drafts surface is the "not-yet-live" surface: it must include
// true drafts AND scheduled (future-dated) posts, but exclude posts that are
// already live (`published_at <= now`). One common test, both backends, fixed
// injected `now` (issue #70).
#[apply(backends)]
#[tokio::test]
async fn drafts_list_includes_scheduled_excludes_live(#[case] backend: Backend) {
    let env = backend.setup().await;
    let now = fixed_instant("2026-06-26T12:00:00Z");
    let user_id = SeedUser::new()
        .seed(env.users(), env.write_scope())
        .await
        .user_id;

    // True draft (published_at NULL).
    SeedRawPost::new(user_id)
        .draft()
        .slug("a-draft")
        .seed(env.posts(), env.write_scope())
        .await;
    // Scheduled post (published_at in the future).
    seed_post_published_at(
        Arc::clone(&env.posts()),
        Arc::clone(&env.feed_events()),
        env.write_scope(),
        user_id,
        "a-sched",
        add(now, 2.hour()),
    )
    .await;
    // Live post (published_at in the past).
    seed_post_published_at(
        Arc::clone(&env.posts()),
        Arc::clone(&env.feed_events()),
        env.write_scope(),
        user_id,
        "a-live",
        subtract(now, 2.hour()),
    )
    .await;

    let rows = env
        .posts()
        .list_drafts_by_user(user_id, None, parse_row_limit("50"), now)
        .await
        .unwrap();
    let slugs: Vec<String> = rows.iter().map(|p| p.slug.to_string()).collect();
    assert!(
        slugs.contains(&"a-draft".to_string()),
        "drafts must include true drafts: {slugs:?}"
    );
    assert!(
        slugs.contains(&"a-sched".to_string()),
        "drafts must include scheduled posts: {slugs:?}"
    );
    assert!(
        !slugs.contains(&"a-live".to_string()),
        "drafts must exclude live posts: {slugs:?}"
    );
}

// Go-live window/catch-up reads (issue #70, Task 7): the feed worker uses these
// to nudge cached feeds when a future-dated post crosses into "live" with no
// accompanying write. One common test per read, both backends, fixed injected
// clock (no sleeps).

#[apply(backends)]
#[tokio::test]
async fn list_posts_gone_live_between_returns_only_window_with_tags(#[case] backend: Backend) {
    let env = backend.setup().await;
    let after = fixed_instant("2026-06-26T12:00:00Z");
    let upto = add(after, 1.hour());
    let alice = SeedUser::new().seed(env.users(), env.write_scope()).await;
    let bob = SeedUser::new().seed(env.users(), env.write_scope()).await;

    // Inside the window (after, upto], tagged: must be returned with its tag.
    let inside = seed_post_published_at(
        Arc::clone(&env.posts()),
        Arc::clone(&env.feed_events()),
        env.write_scope(),
        alice.user_id,
        "in-window",
        add(after, 30.minute()),
    )
    .await;
    storage::test_support::set_post_tags_confirmed(
        &env.write_scope(),
        std::sync::Arc::clone(&env.posts()),
        inside,
        alice.user_id,
        &["scheduling".parse::<TagLabel>().unwrap()],
    )
    .await
    .unwrap();
    // Exactly at the inclusive upper bound: must be returned (untagged).
    seed_post_published_at(
        Arc::clone(&env.posts()),
        Arc::clone(&env.feed_events()),
        env.write_scope(),
        bob.user_id,
        "at-upto",
        upto,
    )
    .await;
    // Exactly at the exclusive lower bound: must be excluded.
    seed_post_published_at(
        Arc::clone(&env.posts()),
        Arc::clone(&env.feed_events()),
        env.write_scope(),
        alice.user_id,
        "at-after",
        after,
    )
    .await;
    // Past the window: must be excluded.
    seed_post_published_at(
        Arc::clone(&env.posts()),
        Arc::clone(&env.feed_events()),
        env.write_scope(),
        alice.user_id,
        "out-window",
        add(upto, 1.hour()),
    )
    .await;

    // These posts are live in the time window, but only Public Posts may enqueue
    // the public Syndication Feed surfaces.
    SeedRawPost::new(alice.user_id)
        .slug("private-in-window")
        .published_at(add(after, 35.minute()))
        .audiences(vec![])
        .seed(env.posts(), env.write_scope())
        .await;
    SeedRawPost::new(alice.user_id)
        .slug("subscribers-in-window")
        .published_at(add(after, 40.minute()))
        .audiences(vec![AudienceTarget::Subscribers])
        .seed(env.posts(), env.write_scope())
        .await;
    let named = create_named_audience(
        Arc::clone(&env.audiences()),
        env.write_scope(),
        alice.user_id,
        "go-live-private",
    )
    .await;
    SeedRawPost::new(alice.user_id)
        .slug("named-in-window")
        .published_at(add(after, 45.minute()))
        .audiences(vec![AudienceTarget::Named(named)])
        .seed(env.posts(), env.write_scope())
        .await;
    let deleted = SeedRawPost::new(alice.user_id)
        .slug("deleted-in-window")
        .published_at(add(after, 50.minute()))
        .seed(env.posts(), env.write_scope())
        .await
        .post_id;
    soft_delete_post_confirmed(
        Arc::clone(&env.posts()),
        env.write_scope(),
        deleted,
        alice.user_id,
    )
    .await;

    let live: Vec<GoLivePost> = env
        .posts()
        .list_posts_gone_live_between(after, upto)
        .await
        .unwrap();
    assert_eq!(
        live.len(),
        2,
        "only the (after, upto] posts are returned: {live:?}"
    );

    let alice_live = live
        .iter()
        .find(|p| p.username == alice.username)
        .expect("alice's in-window post is present");
    let slugs: Vec<String> = alice_live
        .tag_slugs
        .iter()
        .map(ToString::to_string)
        .collect();
    assert_eq!(slugs, vec!["scheduling".to_string()], "tags are hydrated");

    let bob_live = live
        .iter()
        .find(|p| p.username == bob.username)
        .expect("bob's at-upto post is present (inclusive upper)");
    assert!(
        bob_live.tag_slugs.is_empty(),
        "untagged post yields empty tag_slugs"
    );
}

#[apply(backends)]
#[tokio::test]
async fn feed_urls_needing_catchup_returns_stale_feeds(#[case] backend: Backend) {
    use common::feed::{FeedFormat, FeedSurface};
    use host::feed::FeedPath;

    let env = backend.setup().await;
    let now = fixed_instant("2026-06-26T12:00:00Z");
    let t0 = subtract(now, 2.hour());
    let alice = SeedUser::new().seed(env.users(), env.write_scope()).await;

    // A live post, newer than t0, on the site/user feeds and — once tagged —
    // on the site-tag and user-tag feeds too.
    let post = seed_post_published_at(
        Arc::clone(&env.posts()),
        Arc::clone(&env.feed_events()),
        env.write_scope(),
        alice.user_id,
        "live-one",
        subtract(now, 1.hour()),
    )
    .await;
    storage::test_support::set_post_tags_confirmed(
        &env.write_scope(),
        std::sync::Arc::clone(&env.posts()),
        post,
        alice.user_id,
        &["rust".parse::<TagLabel>().unwrap()],
    )
    .await
    .unwrap();

    // A stale feed with only non-Public or Deleted Posts must remain quiet: a
    // restart catch-up pass materializes public projections, not every live row.
    let bob = SeedUser::new().seed(env.users(), env.write_scope()).await;
    SeedRawPost::new(bob.user_id)
        .slug("private-live")
        .published_at(subtract(now, 30.minute()))
        .audiences(vec![])
        .seed(env.posts(), env.write_scope())
        .await;
    SeedRawPost::new(bob.user_id)
        .slug("subscribers-live")
        .published_at(subtract(now, 25.minute()))
        .audiences(vec![AudienceTarget::Subscribers])
        .seed(env.posts(), env.write_scope())
        .await;
    let named = create_named_audience(
        Arc::clone(&env.audiences()),
        env.write_scope(),
        bob.user_id,
        "catch-up-private",
    )
    .await;
    SeedRawPost::new(bob.user_id)
        .slug("named-live")
        .published_at(subtract(now, 20.minute()))
        .audiences(vec![AudienceTarget::Named(named)])
        .seed(env.posts(), env.write_scope())
        .await;
    let deleted = SeedRawPost::new(bob.user_id)
        .slug("deleted-live")
        .published_at(subtract(now, 15.minute()))
        .seed(env.posts(), env.write_scope())
        .await
        .post_id;
    soft_delete_post_confirmed(
        Arc::clone(&env.posts()),
        env.write_scope(),
        deleted,
        bob.user_id,
    )
    .await;

    // The exact feed-url keys for each surface, built the same way the worker
    // does, so the per-surface arms of `max_published_at_for_surface` are all
    // exercised (Site, User, SiteTag, UserTag).
    let tag = "rust".parse().unwrap();
    let site_tag_url = FeedPath::canonical(&FeedSurface::SiteTag { tag }, FeedFormat::Atom);
    let user_tag_url = FeedPath::canonical(
        &FeedSurface::UserTag {
            username: alice.username.clone(),
            tag: "rust".parse().unwrap(),
        },
        FeedFormat::Atom,
    );

    // Stale (generated before go-live) => must be returned.
    SeedFeedCache::new(fp("/feed.atom"))
        .body("cached".to_owned())
        .etag(parse_etag("\"etag\""))
        .representation_modified_at(t0)
        .generated_at(t0)
        .seed(env.feed_cache(), env.write_scope())
        .await;
    SeedFeedCache::new(site_tag_url.clone())
        .body("cached".to_owned())
        .etag(parse_etag("\"etag\""))
        .representation_modified_at(t0)
        .generated_at(t0)
        .seed(env.feed_cache(), env.write_scope())
        .await;
    SeedFeedCache::new(user_tag_url.clone())
        .body("cached".to_owned())
        .etag(parse_etag("\"etag\""))
        .representation_modified_at(t0)
        .generated_at(t0)
        .seed(env.feed_cache(), env.write_scope())
        .await;
    let bob_feed_url = format!("/~{}/feed.atom", bob.username);
    SeedFeedCache::new(fp(&bob_feed_url))
        .body("cached".to_owned())
        .etag(parse_etag("\"etag\""))
        .representation_modified_at(t0)
        .generated_at(t0)
        .seed(env.feed_cache(), env.write_scope())
        .await;
    // Fresh (generated after the newest live post) => must NOT be returned.
    SeedFeedCache::new(fp("/~alice/feed.atom"))
        .body("cached".to_owned())
        .etag(parse_etag("\"etag\""))
        .representation_modified_at(now)
        .generated_at(now)
        .seed(env.feed_cache(), env.write_scope())
        .await;

    let stale = env.posts().feed_urls_needing_catchup(now).await.unwrap();
    assert!(
        stale.iter().any(|u| u.as_ref() == "/feed.atom"),
        "a stale site feed is returned: {stale:?}"
    );
    assert!(
        stale.contains(&site_tag_url),
        "a stale site-tag feed is returned: {stale:?}"
    );
    assert!(
        stale.contains(&user_tag_url),
        "a stale user-tag feed is returned: {stale:?}"
    );
    assert!(
        !stale.iter().any(|u| u.as_ref() == "/~alice/feed.atom"),
        "a feed newer than its surface's newest post is not stale: {stale:?}"
    );
    assert!(
        !stale.iter().any(|url| url.as_ref() == bob_feed_url),
        "non-Public and Deleted Posts do not make a cached feed stale: {stale:?}"
    );
}

#[apply(backends)]
#[tokio::test]
async fn tag_list_pagination(#[case] backend: Backend) {
    let env = backend.setup().await;
    let user = SeedUser::new()
        .display_name("Pagination")
        .seed(env.users(), env.write_scope())
        .await
        .user_id;

    let mut post_ids = Vec::new();
    for _ in 0..5 {
        let post_id = SeedRawPost::new(user)
            .seed(env.posts(), env.write_scope())
            .await
            .post_id;
        post_ids.push(post_id);

        storage::test_support::set_post_tags_confirmed(
            &env.write_scope(),
            std::sync::Arc::clone(&env.posts()),
            post_id,
            user,
            &["pagination-test".parse::<TagLabel>().unwrap()],
        )
        .await
        .expect("set_post_tags failed");
    }

    let tag_slug: Tag = "pagination-test".parse().unwrap();
    let posts = anon_by_tag(env.posts(), &tag_slug, "2").await;

    assert_eq!(posts.len(), 2);
    // Should be newest-first.
    assert!(posts[0].created_at >= posts[1].created_at);
}

#[apply(backends)]
#[tokio::test]
async fn list_user_posts_by_tag_excludes_other_users(#[case] backend: Backend) {
    let env = backend.setup().await;
    let user1 = SeedUser::new()
        .display_name("User1")
        .seed(env.users(), env.write_scope())
        .await
        .user_id;

    let user2 = SeedUser::new()
        .display_name("User2")
        .seed(env.users(), env.write_scope())
        .await
        .user_id;

    let post1 = SeedRawPost::new(user1)
        .seed(env.posts(), env.write_scope())
        .await
        .post_id;

    let post2 = SeedRawPost::new(user2)
        .seed(env.posts(), env.write_scope())
        .await
        .post_id;

    storage::test_support::set_post_tags_confirmed(
        &env.write_scope(),
        std::sync::Arc::clone(&env.posts()),
        post1,
        user1,
        &["shared-tag".parse::<TagLabel>().unwrap()],
    )
    .await
    .expect("tag post1 failed");
    storage::test_support::set_post_tags_confirmed(
        &env.write_scope(),
        std::sync::Arc::clone(&env.posts()),
        post2,
        user2,
        &["shared-tag".parse::<TagLabel>().unwrap()],
    )
    .await
    .expect("tag post2 failed");

    let tag_slug: Tag = "shared-tag".parse().unwrap();
    let user1_posts = anon_user_by_tag(Arc::clone(&env.posts()), user1, &tag_slug, "50").await;

    assert_eq!(user1_posts.len(), 1);
    assert_eq!(user1_posts[0].post_id, post1);

    let user2_posts = anon_user_by_tag(Arc::clone(&env.posts()), user2, &tag_slug, "50").await;

    assert_eq!(user2_posts.len(), 1);
    assert_eq!(user2_posts[0].post_id, post2);
}

#[apply(backends)]
#[tokio::test]
async fn list_posts_by_nonexistent_tag(#[case] backend: Backend) {
    let env = backend.setup().await;
    let tag_slug: Tag = "nosuch-tag".parse().unwrap();
    let result = env
        .posts()
        .list_posts_by_tag(
            &tag_slug,
            storage::PublishedPageRequest::first(
                common::seed::TimelineOrder::Newest,
                parse_row_limit("50"),
            ),
            &ViewerIdentity::Anonymous,
            common::time::UtcInstant::now(),
        )
        .await;

    assert!(matches!(result, Err(ListByTagError::TagNotFound)));
}

#[apply(backends)]
#[tokio::test]
async fn list_user_posts_by_nonexistent_tag(#[case] backend: Backend) {
    let env = backend.setup().await;
    let user = SeedUser::new()
        .display_name("UserTagNope")
        .seed(env.users(), env.write_scope())
        .await
        .user_id;

    let tag_slug: Tag = "nonexistent-tag-99".parse().unwrap();
    let result = env
        .posts()
        .list_user_posts_by_tag(
            user,
            &tag_slug,
            storage::PublishedPageRequest::first(
                common::seed::TimelineOrder::Newest,
                parse_row_limit("50"),
            ),
            &ViewerIdentity::Anonymous,
            common::time::UtcInstant::now(),
        )
        .await;

    assert!(matches!(result, Err(ListByTagError::TagNotFound)));
}

// `set_post_tags`' add/reconcile/clear contract is a generic-contract test,
// homed in `storage/src/posts.rs` as `set_post_tags_adds_removes_and_clears`
// (ADR-0053 §1, #771).

#[apply(backends)]
#[tokio::test]
async fn list_posts_by_tag(#[case] backend: Backend) {
    let env = backend.setup().await;
    let user1 = SeedUser::new()
        .display_name("Eve")
        .seed(env.users(), env.write_scope())
        .await
        .user_id;

    let user2 = SeedUser::new()
        .display_name("Frank")
        .seed(env.users(), env.write_scope())
        .await
        .user_id;

    let post1 = SeedRawPost::new(user1)
        .seed(env.posts(), env.write_scope())
        .await
        .post_id;

    let post2 = SeedRawPost::new(user2)
        .seed(env.posts(), env.write_scope())
        .await
        .post_id;

    storage::test_support::set_post_tags_confirmed(
        &env.write_scope(),
        std::sync::Arc::clone(&env.posts()),
        post1,
        user1,
        &["javascript".parse::<TagLabel>().unwrap()],
    )
    .await
    .expect("set_post_tags failed");
    storage::test_support::set_post_tags_confirmed(
        &env.write_scope(),
        std::sync::Arc::clone(&env.posts()),
        post2,
        user2,
        &["javascript".parse::<TagLabel>().unwrap()],
    )
    .await
    .expect("set_post_tags failed");

    let tag_slug: Tag = "javascript".parse().unwrap();
    let posts = anon_by_tag(env.posts(), &tag_slug, "50").await;

    assert_eq!(posts.len(), 2);
    assert!(posts.iter().any(|p| p.post_id == post1));
    assert!(posts.iter().any(|p| p.post_id == post2));
}

#[apply(backends)]
#[tokio::test]
async fn list_user_posts_by_tag(#[case] backend: Backend) {
    let env = backend.setup().await;
    let user1 = SeedUser::new()
        .display_name("Grace")
        .seed(env.users(), env.write_scope())
        .await
        .user_id;

    let user2 = SeedUser::new()
        .display_name("Henry")
        .seed(env.users(), env.write_scope())
        .await
        .user_id;

    let post1 = SeedRawPost::new(user1)
        .seed(env.posts(), env.write_scope())
        .await
        .post_id;

    let post2 = SeedRawPost::new(user1)
        .seed(env.posts(), env.write_scope())
        .await
        .post_id;

    let post3 = SeedRawPost::new(user2)
        .seed(env.posts(), env.write_scope())
        .await
        .post_id;

    storage::test_support::set_post_tags_confirmed(
        &env.write_scope(),
        std::sync::Arc::clone(&env.posts()),
        post1,
        user1,
        &["clojure".parse::<TagLabel>().unwrap()],
    )
    .await
    .expect("set_post_tags failed");
    storage::test_support::set_post_tags_confirmed(
        &env.write_scope(),
        std::sync::Arc::clone(&env.posts()),
        post2,
        user1,
        &["clojure".parse::<TagLabel>().unwrap()],
    )
    .await
    .expect("set_post_tags failed");
    storage::test_support::set_post_tags_confirmed(
        &env.write_scope(),
        std::sync::Arc::clone(&env.posts()),
        post3,
        user2,
        &["clojure".parse::<TagLabel>().unwrap()],
    )
    .await
    .expect("set_post_tags failed");

    let tag_slug: Tag = "clojure".parse().unwrap();
    let posts = anon_user_by_tag(Arc::clone(&env.posts()), user1, &tag_slug, "50").await;

    assert_eq!(posts.len(), 2);
    assert!(posts.iter().all(|p| p.user_id == user1));
}

#[apply(backends)]
#[tokio::test]
async fn tag_not_found_error(#[case] backend: Backend) {
    let env = backend.setup().await;
    let tag_slug: Tag = "nonexistent".parse().unwrap();
    let result = env
        .posts()
        .list_posts_by_tag(
            &tag_slug,
            storage::PublishedPageRequest::first(
                common::seed::TimelineOrder::Newest,
                parse_row_limit("50"),
            ),
            &ViewerIdentity::Anonymous,
            common::time::UtcInstant::now(),
        )
        .await;

    match result {
        Err(ListByTagError::TagNotFound) => {}
        other => panic!("Expected TagNotFound, got {other:?}"),
    }
}

#[apply(backends)]
#[tokio::test]
async fn soft_deleted_posts_excluded_from_tag_list(#[case] backend: Backend) {
    let env = backend.setup().await;
    let user = SeedUser::new()
        .display_name("Iris")
        .seed(env.users(), env.write_scope())
        .await
        .user_id;

    let post1 = SeedRawPost::new(user)
        .seed(env.posts(), env.write_scope())
        .await
        .post_id;

    let post2 = SeedRawPost::new(user)
        .seed(env.posts(), env.write_scope())
        .await
        .post_id;

    storage::test_support::set_post_tags_confirmed(
        &env.write_scope(),
        std::sync::Arc::clone(&env.posts()),
        post1,
        user,
        &["haskell".parse::<TagLabel>().unwrap()],
    )
    .await
    .expect("set_post_tags failed");
    storage::test_support::set_post_tags_confirmed(
        &env.write_scope(),
        std::sync::Arc::clone(&env.posts()),
        post2,
        user,
        &["haskell".parse::<TagLabel>().unwrap()],
    )
    .await
    .expect("set_post_tags failed");

    soft_delete_post_confirmed(Arc::clone(&env.posts()), env.write_scope(), post1, user).await;

    let tag_slug: Tag = "haskell".parse().unwrap();
    let posts = anon_by_tag(env.posts(), &tag_slug, "50").await;

    assert_eq!(posts.len(), 1);
    assert_eq!(posts[0].post_id, post2);
}

// The `PostNotFound` contract is a generic-contract test, homed in
// `storage/src/posts.rs` as
// `set_post_tags_rejects_missing_post_but_allows_soft_deleted` (ADR-0053 §1, #771).

#[apply(backends)]
#[tokio::test]
async fn draft_posts_excluded_from_tag_list(#[case] backend: Backend) {
    let env = backend.setup().await;
    let user = SeedUser::new()
        .display_name("Jack")
        .seed(env.users(), env.write_scope())
        .await
        .user_id;

    let post1 = SeedRawPost::new(user)
        .draft()
        .seed(env.posts(), env.write_scope())
        .await
        .post_id;

    let post2 = SeedRawPost::new(user)
        .seed(env.posts(), env.write_scope())
        .await
        .post_id;

    storage::test_support::set_post_tags_confirmed(
        &env.write_scope(),
        std::sync::Arc::clone(&env.posts()),
        post1,
        user,
        &["kotlin".parse::<TagLabel>().unwrap()],
    )
    .await
    .expect("set_post_tags failed");
    storage::test_support::set_post_tags_confirmed(
        &env.write_scope(),
        std::sync::Arc::clone(&env.posts()),
        post2,
        user,
        &["kotlin".parse::<TagLabel>().unwrap()],
    )
    .await
    .expect("set_post_tags failed");

    let tag_slug: Tag = "kotlin".parse().unwrap();
    let posts = anon_by_tag(env.posts(), &tag_slug, "50").await;

    assert_eq!(posts.len(), 1);
    assert_eq!(posts[0].post_id, post2);
}

// The timeline's page boundary is publication time plus Post ID in both
// directions.  This deliberately includes a same-time pair so a cursor cannot
// skip or duplicate a tie across the first continuation.
#[apply(backends)]
#[tokio::test]
async fn published_timeline_orders_and_paginates_in_both_directions(#[case] backend: Backend) {
    let env = backend.setup().await;
    let now = fixed_instant("2026-06-26T12:00:00Z");
    let user = SeedUser::new()
        .seed(env.users(), env.write_scope())
        .await
        .user_id;
    let early = seed_post_published_at(
        Arc::clone(&env.posts()),
        Arc::clone(&env.feed_events()),
        env.write_scope(),
        user,
        "early",
        subtract(now, 4.hours()),
    )
    .await;
    let tied_first = seed_post_published_at(
        Arc::clone(&env.posts()),
        Arc::clone(&env.feed_events()),
        env.write_scope(),
        user,
        "tied-first",
        subtract(now, 2.hours()),
    )
    .await;
    let tied_second = seed_post_published_at(
        Arc::clone(&env.posts()),
        Arc::clone(&env.feed_events()),
        env.write_scope(),
        user,
        "tied-second",
        subtract(now, 2.hours()),
    )
    .await;
    let late = seed_post_published_at(
        Arc::clone(&env.posts()),
        Arc::clone(&env.feed_events()),
        env.write_scope(),
        user,
        "late",
        subtract(now, 1.hour()),
    )
    .await;

    for (order, expected) in [
        (
            common::seed::TimelineOrder::Newest,
            vec![late, tied_second, tied_first, early],
        ),
        (
            common::seed::TimelineOrder::Oldest,
            vec![early, tied_first, tied_second, late],
        ),
    ] {
        let first = env
            .posts()
            .list_published(
                storage::PublishedPageRequest::first(order, parse_row_limit("2")),
                &ViewerIdentity::Anonymous,
                now,
            )
            .await
            .expect("first timeline page");
        let last = first.last().expect("first page has two posts");
        let cursor = PostCursor {
            published_at: last.published_at.expect("published timeline row has time"),
            post_id: last.post_id,
            order,
        };
        let next = env
            .posts()
            .list_published(
                storage::PublishedPageRequest::after(&cursor, parse_row_limit("2")),
                &ViewerIdentity::Anonymous,
                now,
            )
            .await
            .expect("continuation timeline page");
        let actual = first
            .into_iter()
            .chain(next)
            .map(|post| post.post_id)
            .collect::<Vec<_>>();

        assert_eq!(actual, expected, "{order:?} uses publication-time keysets");
    }
}

#[apply(backends)]
#[tokio::test]
async fn list_drafts_cursor_boundary(#[case] backend: Backend) {
    let env = backend.setup().await;
    let user = SeedUser::new()
        .seed(env.users(), env.write_scope())
        .await
        .user_id;

    let _now = UtcInstant::now();

    for _ in 0..3 {
        SeedRawPost::new(user)
            .draft()
            .seed(env.posts(), env.write_scope())
            .await;
    }

    let all = drafts_of(Arc::clone(&env.posts()), user, "10").await;
    assert_eq!(all.len(), 3);

    let first = drafts_of(Arc::clone(&env.posts()), user, "1").await;
    assert_eq!(first.len(), 1);

    if !first.is_empty() {
        let cursor = DraftPostCursor {
            created_at: first[0].created_at,
            post_id: first[0].post_id,
        };
        let next = env
            .posts()
            .list_drafts_by_user(
                user,
                Some(&cursor),
                parse_row_limit("2"),
                common::time::UtcInstant::now(),
            )
            .await
            .expect("list_drafts_by_user with cursor failed");
        assert!(next.len() <= 2);
    }
}

#[apply(backends)]
#[tokio::test]
async fn list_user_posts_by_tag_cursor(#[case] backend: Backend) {
    let env = backend.setup().await;
    let user = SeedUser::new()
        .seed(env.users(), env.write_scope())
        .await
        .user_id;

    for _ in 0..3 {
        let post_id = SeedRawPost::new(user)
            .seed(env.posts(), env.write_scope())
            .await
            .post_id;

        storage::test_support::set_post_tags_confirmed(
            &env.write_scope(),
            std::sync::Arc::clone(&env.posts()),
            post_id,
            user,
            &["cursor-tag".parse::<TagLabel>().unwrap()],
        )
        .await
        .expect("set_post_tags failed");
    }

    let tag: Tag = "cursor-tag".parse().unwrap();

    let all = anon_user_by_tag(Arc::clone(&env.posts()), user, &tag, "10").await;
    assert_eq!(all.len(), 3);

    let first = anon_user_by_tag(Arc::clone(&env.posts()), user, &tag, "1").await;
    assert_eq!(first.len(), 1);

    if !first.is_empty() {
        let cursor = PostCursor {
            published_at: first[0]
                .published_at
                .expect("published timeline row has time"),
            post_id: first[0].post_id,
            order: common::seed::TimelineOrder::Newest,
        };
        let next = env
            .posts()
            .list_user_posts_by_tag(
                user,
                &tag,
                storage::PublishedPageRequest::after(&cursor, parse_row_limit("2")),
                &ViewerIdentity::Anonymous,
                common::time::UtcInstant::now(),
            )
            .await
            .expect("list_user_posts_by_tag with cursor failed");
        assert!(next.len() <= 2);
    }
}

#[apply(backends)]
#[tokio::test]
async fn list_posts_by_tag_cursor(#[case] backend: Backend) {
    let env = backend.setup().await;
    let user = SeedUser::new()
        .seed(env.users(), env.write_scope())
        .await
        .user_id;

    for _ in 0..3 {
        let post_id = SeedRawPost::new(user)
            .seed(env.posts(), env.write_scope())
            .await
            .post_id;

        storage::test_support::set_post_tags_confirmed(
            &env.write_scope(),
            std::sync::Arc::clone(&env.posts()),
            post_id,
            user,
            &["global-tag".parse::<TagLabel>().unwrap()],
        )
        .await
        .expect("set_post_tags failed");
    }

    let tag: Tag = "global-tag".parse().unwrap();

    let all = anon_by_tag(env.posts(), &tag, "10").await;
    assert_eq!(all.len(), 3);

    let first = anon_by_tag(env.posts(), &tag, "1").await;
    assert_eq!(first.len(), 1);

    if !first.is_empty() {
        let cursor = PostCursor {
            published_at: first[0]
                .published_at
                .expect("published timeline row has time"),
            post_id: first[0].post_id,
            order: common::seed::TimelineOrder::Newest,
        };
        let next = env
            .posts()
            .list_posts_by_tag(
                &tag,
                storage::PublishedPageRequest::after(&cursor, parse_row_limit("2")),
                &ViewerIdentity::Anonymous,
                common::time::UtcInstant::now(),
            )
            .await
            .expect("list_posts_by_tag with cursor failed");
        assert!(next.len() <= 2);
    }
}

// ====== Additional error path and rollback scenario tests ======

#[apply(backends)]
#[tokio::test]
async fn list_published_by_user_no_posts(#[case] backend: Backend) {
    let env = backend.setup().await;
    let user = SeedUser::new().seed(env.users(), env.write_scope()).await;

    let posts = anon_published_by_user(Arc::clone(&env.posts()), &user.username, "10").await;
    assert!(posts.is_empty());

    let cursor = PostCursor {
        published_at: common::time::UtcInstant::now(),
        post_id: PostId::from(999),
        order: common::seed::TimelineOrder::Newest,
    };
    let posts = env
        .posts()
        .list_published_by_user(
            &user.username,
            storage::PublishedPageRequest::after(&cursor, parse_row_limit("10")),
            &ViewerIdentity::Anonymous,
            common::time::UtcInstant::now(),
        )
        .await
        .expect("list_published_by_user with cursor failed");
    assert!(posts.is_empty());
}

#[apply(backends)]
#[tokio::test]
async fn get_by_permalink_soft_deleted(#[case] backend: Backend) {
    let env = backend.setup().await;
    let user = SeedUser::new().seed(env.users(), env.write_scope()).await;

    let created_at = UtcInstant::now();
    let created_date = Offset::UTC.to_datetime(created_at.value()).date();

    let seeded = SeedRawPost::new(user.user_id)
        .published_at(created_at)
        .seed(env.posts(), env.write_scope())
        .await;

    let post = env
        .posts()
        .get_post_by_permalink(
            &user.username,
            permalink_date(
                i32::from(created_date.year()),
                u32::try_from(created_date.month()).expect("Jiff civil month fits u32"),
                u32::try_from(created_date.day()).expect("Jiff civil day fits u32"),
            ),
            &seeded.slug,
            &ViewerIdentity::Anonymous,
            common::time::UtcInstant::now(),
        )
        .await
        .expect("get_post_by_permalink failed");
    assert!(post.is_some());

    soft_delete_post_confirmed(
        Arc::clone(&env.posts()),
        env.write_scope(),
        seeded.post_id,
        user.user_id,
    )
    .await;

    let post = env
        .posts()
        .get_post_by_permalink(
            &user.username,
            permalink_date(
                i32::from(created_date.year()),
                u32::try_from(created_date.month()).expect("Jiff civil month fits u32"),
                u32::try_from(created_date.day()).expect("Jiff civil day fits u32"),
            ),
            &seeded.slug,
            &ViewerIdentity::Anonymous,
            common::time::UtcInstant::now(),
        )
        .await
        .expect("get_post_by_permalink after delete failed");
    assert!(post.is_none());
}
