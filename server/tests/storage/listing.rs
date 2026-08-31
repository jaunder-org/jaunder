use chrono::{Datelike, Utc};
use common::{
    ids::{PostId, UserId},
    tag::{Tag, TagLabel},
    test_support::{parse_etag, parse_post_body, parse_row_limit, permalink_date},
    time::UtcInstant,
    username::Username,
    visibility::{AudienceTarget, ViewerIdentity},
};
use std::sync::Arc;
use storage::test_support::{
    Backend, SeedRawPost, SeedUser, backends, confirmed_for as confirmed, fp,
};
use storage::{
    AppState, FeedCacheRow, GoLivePost, ListByTagError, PostBookkeepingExpectation, PostCursor,
    PostFormat, PostRecord, RenderedPostContent, create_rendered_post,
};

use rstest::*;
use rstest_reuse::*;

use super::fixtures::{anon_by_tag, anon_published};

async fn soft_delete_post_confirmed(state: &AppState, post_id: PostId, user_id: UserId) {
    let posts = Arc::clone(&state.posts);
    let outcome = state
        .write_scope
        .run(move |transaction| {
            Box::pin(async move { posts.soft_delete_post(transaction, post_id, user_id).await })
        })
        .await
        .expect("soft_delete_post failed");
    confirmed(outcome, "post deletion");
}

async fn upsert_cache_confirmed(state: &AppState, row: FeedCacheRow) {
    let cache = Arc::clone(&state.feed_cache);
    let outcome = state
        .write_scope
        .run(move |transaction| Box::pin(async move { cache.upsert(transaction, row).await }))
        .await
        .expect("seed cached feed");
    confirmed(outcome, "feed-cache fixture");
}

async fn anon_user_by_tag(
    state: &AppState,
    user_id: UserId,
    tag: &Tag,
    limit: &str,
) -> Vec<PostRecord> {
    state
        .posts
        .list_user_posts_by_tag(
            user_id,
            tag,
            None,
            parse_row_limit(limit),
            &ViewerIdentity::Anonymous,
            common::time::UtcInstant::now(),
        )
        .await
        .expect("list_user_posts_by_tag failed")
}

async fn anon_published_by_user(
    state: &AppState,
    username: &Username,
    limit: &str,
) -> Vec<PostRecord> {
    state
        .posts
        .list_published_by_user(
            username,
            None,
            parse_row_limit(limit),
            &ViewerIdentity::Anonymous,
            common::time::UtcInstant::now(),
        )
        .await
        .expect("list_published_by_user failed")
}

async fn drafts_of(state: &AppState, user_id: UserId, limit: &str) -> Vec<PostRecord> {
    state
        .posts
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
    state: &Arc<AppState>,
    user_id: UserId,
    slug: &str,
    published_at: common::time::UtcInstant,
) -> PostId {
    confirmed(
        create_rendered_post(
            &state.write_scope,
            Arc::clone(&state.posts),
            Arc::clone(&state.feed_events),
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
    use chrono::{Duration, TimeZone};
    let env = backend.setup().await;
    let state = &env.state;
    let now = Utc.with_ymd_and_hms(2026, 6, 26, 12, 0, 0).unwrap();
    let user = SeedUser::new().seed(state).await;
    seed_post_published_at(
        state,
        user.user_id,
        "live-one",
        common::time::UtcInstant::from(now - Duration::hours(1)),
    )
    .await;
    seed_post_published_at(
        state,
        user.user_id,
        "sched-one",
        common::time::UtcInstant::from(now + Duration::hours(1)),
    )
    .await;

    // At `now`: the live post is visible, the scheduled one is not.
    let got_live = state
        .posts
        .get_post_by_permalink(
            &user.username,
            permalink_date(2026, 6, 26),
            &"live-one".parse().unwrap(),
            &ViewerIdentity::Anonymous,
            common::time::UtcInstant::from(now),
        )
        .await
        .unwrap();
    assert!(got_live.is_some(), "live post must be visible at now");
    let got_sched = state
        .posts
        .get_post_by_permalink(
            &user.username,
            permalink_date(2026, 6, 26),
            &"sched-one".parse().unwrap(),
            &ViewerIdentity::Anonymous,
            common::time::UtcInstant::from(now),
        )
        .await
        .unwrap();
    assert!(
        got_sched.is_none(),
        "scheduled post must be hidden before its time"
    );

    // Exactly at go-live, the scheduled post appears (locks the `<= now`
    // boundary shared with the unpublished lookup's strict `> now` predicate).
    let due = now + Duration::hours(1);
    let got_after = state
        .posts
        .get_post_by_permalink(
            &user.username,
            permalink_date(2026, 6, 26),
            &"sched-one".parse().unwrap(),
            &ViewerIdentity::Anonymous,
            common::time::UtcInstant::from(due),
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
    use chrono::{Duration, TimeZone};
    let env = backend.setup().await;
    let state = &env.state;
    let now = Utc.with_ymd_and_hms(2026, 6, 26, 12, 0, 0).unwrap();
    let user = SeedUser::new().seed(state).await;
    let live = seed_post_published_at(
        state,
        user.user_id,
        "live-one",
        common::time::UtcInstant::from(now - Duration::hours(1)),
    )
    .await;
    let sched = seed_post_published_at(
        state,
        user.user_id,
        "sched-one",
        common::time::UtcInstant::from(now + Duration::hours(1)),
    )
    .await;

    let at_now = state
        .posts
        .list_published_by_user(
            &user.username,
            None,
            parse_row_limit("50"),
            &ViewerIdentity::Anonymous,
            common::time::UtcInstant::from(now),
        )
        .await
        .unwrap();
    let ids_now: Vec<PostId> = at_now.iter().map(|p| p.post_id).collect();
    assert!(ids_now.contains(&live), "live post must be listed at now");
    assert!(
        !ids_now.contains(&sched),
        "scheduled post must be hidden before its time"
    );

    let after = now + Duration::hours(1) + Duration::seconds(1);
    let at_after = state
        .posts
        .list_published_by_user(
            &user.username,
            None,
            parse_row_limit("50"),
            &ViewerIdentity::Anonymous,
            common::time::UtcInstant::from(after),
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
    use chrono::{Duration, TimeZone};
    let env = backend.setup().await;
    let state = &env.state;
    let now = Utc.with_ymd_and_hms(2026, 6, 26, 12, 0, 0).unwrap();
    let user_id = SeedUser::new().seed(state).await.user_id;
    let live = seed_post_published_at(
        state,
        user_id,
        "live-one",
        common::time::UtcInstant::from(now - Duration::hours(1)),
    )
    .await;
    let sched = seed_post_published_at(
        state,
        user_id,
        "sched-one",
        common::time::UtcInstant::from(now + Duration::hours(1)),
    )
    .await;

    let at_now = state
        .posts
        .list_published(
            None,
            parse_row_limit("50"),
            &ViewerIdentity::Anonymous,
            common::time::UtcInstant::from(now),
        )
        .await
        .unwrap();
    let ids_now: Vec<PostId> = at_now.iter().map(|p| p.post_id).collect();
    assert!(ids_now.contains(&live), "live post must be listed at now");
    assert!(
        !ids_now.contains(&sched),
        "scheduled post must be hidden before its time"
    );

    let after = now + Duration::hours(1) + Duration::seconds(1);
    let at_after = state
        .posts
        .list_published(
            None,
            parse_row_limit("50"),
            &ViewerIdentity::Anonymous,
            common::time::UtcInstant::from(after),
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
async fn list_posts_by_tag_hides_scheduled_until_due(#[case] backend: Backend) {
    use chrono::{Duration, TimeZone};
    let env = backend.setup().await;
    let state = &env.state;
    let now = Utc.with_ymd_and_hms(2026, 6, 26, 12, 0, 0).unwrap();
    let user_id = SeedUser::new().seed(state).await.user_id;
    let live = seed_post_published_at(
        state,
        user_id,
        "live-one",
        common::time::UtcInstant::from(now - Duration::hours(1)),
    )
    .await;
    let sched = seed_post_published_at(
        state,
        user_id,
        "sched-one",
        common::time::UtcInstant::from(now + Duration::hours(1)),
    )
    .await;
    storage::test_support::set_post_tags_confirmed(
        &state.write_scope,
        std::sync::Arc::clone(&state.posts),
        live,
        user_id,
        &["scheduling".parse::<TagLabel>().unwrap()],
    )
    .await
    .unwrap();
    storage::test_support::set_post_tags_confirmed(
        &state.write_scope,
        std::sync::Arc::clone(&state.posts),
        sched,
        user_id,
        &["scheduling".parse::<TagLabel>().unwrap()],
    )
    .await
    .unwrap();
    let tag_slug: Tag = "scheduling".parse().unwrap();

    let at_now = state
        .posts
        .list_posts_by_tag(
            &tag_slug,
            None,
            parse_row_limit("50"),
            &ViewerIdentity::Anonymous,
            common::time::UtcInstant::from(now),
        )
        .await
        .unwrap();
    let ids_now: Vec<PostId> = at_now.iter().map(|p| p.post_id).collect();
    assert!(ids_now.contains(&live), "live post must be listed at now");
    assert!(
        !ids_now.contains(&sched),
        "scheduled post must be hidden before its time"
    );

    let after = now + Duration::hours(1) + Duration::seconds(1);
    let at_after = state
        .posts
        .list_posts_by_tag(
            &tag_slug,
            None,
            parse_row_limit("50"),
            &ViewerIdentity::Anonymous,
            common::time::UtcInstant::from(after),
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
    use chrono::{Duration, TimeZone};
    let env = backend.setup().await;
    let state = &env.state;
    let now = Utc.with_ymd_and_hms(2026, 6, 26, 12, 0, 0).unwrap();
    let user_id = SeedUser::new().seed(state).await.user_id;
    let live = seed_post_published_at(
        state,
        user_id,
        "live-one",
        common::time::UtcInstant::from(now - Duration::hours(1)),
    )
    .await;
    let sched = seed_post_published_at(
        state,
        user_id,
        "sched-one",
        common::time::UtcInstant::from(now + Duration::hours(1)),
    )
    .await;
    storage::test_support::set_post_tags_confirmed(
        &state.write_scope,
        std::sync::Arc::clone(&state.posts),
        live,
        user_id,
        &["scheduling".parse::<TagLabel>().unwrap()],
    )
    .await
    .unwrap();
    storage::test_support::set_post_tags_confirmed(
        &state.write_scope,
        std::sync::Arc::clone(&state.posts),
        sched,
        user_id,
        &["scheduling".parse::<TagLabel>().unwrap()],
    )
    .await
    .unwrap();
    let tag_slug: Tag = "scheduling".parse().unwrap();

    let at_now = state
        .posts
        .list_user_posts_by_tag(
            user_id,
            &tag_slug,
            None,
            parse_row_limit("50"),
            &ViewerIdentity::Anonymous,
            common::time::UtcInstant::from(now),
        )
        .await
        .unwrap();
    let ids_now: Vec<PostId> = at_now.iter().map(|p| p.post_id).collect();
    assert!(ids_now.contains(&live), "live post must be listed at now");
    assert!(
        !ids_now.contains(&sched),
        "scheduled post must be hidden before its time"
    );

    let after = now + Duration::hours(1) + Duration::seconds(1);
    let at_after = state
        .posts
        .list_user_posts_by_tag(
            user_id,
            &tag_slug,
            None,
            parse_row_limit("50"),
            &ViewerIdentity::Anonymous,
            common::time::UtcInstant::from(after),
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
    let state = &env.state;
    let user_id = SeedUser::new().seed(state).await.user_id;

    let post_id = SeedRawPost::new(user_id).seed(state).await.post_id;

    let published = anon_published(state, "10").await;
    assert!(published.iter().any(|p| p.post_id == post_id));

    soft_delete_post_confirmed(state, post_id, user_id).await;

    let published = anon_published(state, "10").await;
    assert!(!published.iter().any(|p| p.post_id == post_id));

    let record = state
        .posts
        .get_post_by_id(post_id, &ViewerIdentity::Anonymous)
        .await
        .unwrap()
        .unwrap();
    assert!(record.deleted_at.is_some());
}

#[apply(backends)]
#[tokio::test]
async fn list_published_in_window_applies_hybrid_rule_across_surfaces(#[case] backend: Backend) {
    use chrono::Duration;
    use common::feed::FeedSurface;
    use host::{
        feed::HybridWindow,
        test_support::{parse_feed_min_days, parse_feed_min_items},
    };

    let env = backend.setup().await;
    let state = &env.state;

    let alice = SeedUser::new().seed(state).await;
    let bob = SeedUser::new().seed(state).await;
    let alice_id = alice.user_id;
    let bob_id = bob.user_id;

    let now = Utc::now();
    let make_post = |user_id: UserId, days_ago: i64| {
        SeedRawPost::new(user_id).published_at(common::time::UtcInstant::from(
            now - Duration::days(days_ago),
        ))
    };

    // Alice: 4 posts published 1, 2, 100, 200 days ago.
    let alice_recent_1 = make_post(alice_id, 1).seed(state).await;
    make_post(alice_id, 2).seed(state).await;
    make_post(alice_id, 100).seed(state).await;
    make_post(alice_id, 200).seed(state).await;

    // Bob: 1 post published 5 days ago.
    make_post(bob_id, 5).seed(state).await;

    // Future-dated draft-equivalent (excluded).
    make_post(alice_id, -1).seed(state).await;

    // Site feed, window {3 items, 30 days} → union of "top 3" and "in last 30
    // days". Alice 1d+2d and Bob 5d are in-window (3 posts). Alice 100d/200d
    // and the future post are excluded by their respective filters; the union
    // still picks at least 3 by ROW_NUMBER, so we get exactly those 3.
    let window = HybridWindow {
        min_items: parse_feed_min_items("3"),
        min_days: parse_feed_min_days("30"),
    };
    let site = state
        .posts
        .list_published_in_window(
            &FeedSurface::Site,
            &window,
            common::time::UtcInstant::from(now),
            &ViewerIdentity::Anonymous,
        )
        .await
        .unwrap();
    assert_eq!(site.len(), 3, "site feed in {{3 items, 30 days}}");
    assert!(
        site.iter()
            .all(|p| p.published_at.unwrap().value() >= now - Duration::days(30))
    );

    // Site feed with min_items=5: top 5 includes all four real posts plus
    // Bob's, regardless of age — total 5 (alice-old-2 included by count).
    let big = HybridWindow {
        min_items: parse_feed_min_items("5"),
        min_days: parse_feed_min_days("30"),
    };
    let site_big = state
        .posts
        .list_published_in_window(
            &FeedSurface::Site,
            &big,
            common::time::UtcInstant::from(now),
            &ViewerIdentity::Anonymous,
        )
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
    let alice_feed = state
        .posts
        .list_published_in_window(
            &FeedSurface::User {
                username: alice.username.clone(),
            },
            &alice_window,
            common::time::UtcInstant::from(now),
            &ViewerIdentity::Anonymous,
        )
        .await
        .unwrap();
    assert_eq!(alice_feed.len(), 2);
    assert!(alice_feed.iter().all(|p| p.user_id == alice_id));

    // User feed: bob has only 1 post, returned even with min_items=10.
    let bob_feed = state
        .posts
        .list_published_in_window(
            &FeedSurface::User {
                username: bob.username.clone(),
            },
            &HybridWindow {
                min_items: parse_feed_min_items("10"),
                min_days: parse_feed_min_days("1"),
            },
            common::time::UtcInstant::from(now),
            &ViewerIdentity::Anonymous,
        )
        .await
        .unwrap();
    assert_eq!(bob_feed.len(), 1);
    assert_eq!(bob_feed[0].user_id, bob_id);

    // Add a tag to alice-recent-1 and verify site-tag / user-tag feeds.
    storage::test_support::set_post_tags_confirmed(
        &state.write_scope,
        std::sync::Arc::clone(&state.posts),
        alice_recent_1.post_id,
        alice_id,
        &["rust".parse::<TagLabel>().unwrap()],
    )
    .await
    .unwrap();

    let tag_site = state
        .posts
        .list_published_in_window(
            &FeedSurface::SiteTag {
                tag: "rust".parse().unwrap(),
            },
            &HybridWindow {
                min_items: parse_feed_min_items("20"),
                min_days: parse_feed_min_days("30"),
            },
            common::time::UtcInstant::from(now),
            &ViewerIdentity::Anonymous,
        )
        .await
        .unwrap();
    assert_eq!(tag_site.len(), 1);
    assert_eq!(tag_site[0].slug, alice_recent_1.slug);

    let tag_user = state
        .posts
        .list_published_in_window(
            &FeedSurface::UserTag {
                username: alice.username.clone(),
                tag: "rust".parse().unwrap(),
            },
            &HybridWindow {
                min_items: parse_feed_min_items("20"),
                min_days: parse_feed_min_days("30"),
            },
            common::time::UtcInstant::from(now),
            &ViewerIdentity::Anonymous,
        )
        .await
        .unwrap();
    assert_eq!(tag_user.len(), 1);

    // User-tag for bob+rust: bob has no rust post → empty.
    let bob_tag = state
        .posts
        .list_published_in_window(
            &FeedSurface::UserTag {
                username: bob.username.clone(),
                tag: "rust".parse().unwrap(),
            },
            &HybridWindow {
                min_items: parse_feed_min_items("20"),
                min_days: parse_feed_min_days("30"),
            },
            common::time::UtcInstant::from(now),
            &ViewerIdentity::Anonymous,
        )
        .await
        .unwrap();
    assert!(bob_tag.is_empty());
}

#[apply(backends)]
#[tokio::test]
async fn list_published_by_user_returns_only_user_posts(#[case] backend: Backend) {
    let env = backend.setup().await;
    let state = &env.state;
    let alice = SeedUser::new().seed(state).await;
    let bob = SeedUser::new().seed(state).await;
    let alice_id = alice.user_id;
    let bob_id = bob.user_id;

    SeedRawPost::new(alice_id).seed(state).await;
    SeedRawPost::new(alice_id).seed(state).await;
    SeedRawPost::new(bob_id).seed(state).await;

    let alice_posts = anon_published_by_user(state, &alice.username, "10").await;
    assert_eq!(alice_posts.len(), 2);
    assert!(alice_posts.iter().all(|p| p.user_id == alice_id));

    let bob_posts = anon_published_by_user(state, &bob.username, "10").await;
    assert_eq!(bob_posts.len(), 1);
    assert_eq!(bob_posts[0].user_id, bob_id);
}

#[apply(backends)]
#[tokio::test]
async fn list_published_returns_published_non_deleted_posts(#[case] backend: Backend) {
    let env = backend.setup().await;
    let state = &env.state;
    let user_id = SeedUser::new().seed(state).await.user_id;

    // Create a draft (should not appear)
    SeedRawPost::new(user_id).draft().seed(state).await;

    SeedRawPost::new(user_id).seed(state).await;
    SeedRawPost::new(user_id).seed(state).await;

    let published = anon_published(state, "10").await;
    assert_eq!(published.len(), 2);
    assert!(published.iter().all(|p| p.published_at.is_some()));
}

#[apply(backends)]
#[tokio::test]
async fn list_drafts_by_user_returns_only_drafts(#[case] backend: Backend) {
    let env = backend.setup().await;
    let state = &env.state;
    let user_id = SeedUser::new().seed(state).await.user_id;

    SeedRawPost::new(user_id).draft().seed(state).await;
    SeedRawPost::new(user_id).draft().seed(state).await;

    // Create a published post (should not appear in drafts)
    SeedRawPost::new(user_id).seed(state).await;

    let drafts = drafts_of(state, user_id, "10").await;
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
    use chrono::{Duration, TimeZone};
    let env = backend.setup().await;
    let state = &env.state;
    let now = Utc.with_ymd_and_hms(2026, 6, 26, 12, 0, 0).unwrap();
    let user_id = SeedUser::new().seed(state).await.user_id;

    // True draft (published_at NULL).
    SeedRawPost::new(user_id)
        .draft()
        .slug("a-draft")
        .seed(state)
        .await;
    // Scheduled post (published_at in the future).
    seed_post_published_at(
        state,
        user_id,
        "a-sched",
        common::time::UtcInstant::from(now + Duration::hours(2)),
    )
    .await;
    // Live post (published_at in the past).
    seed_post_published_at(
        state,
        user_id,
        "a-live",
        common::time::UtcInstant::from(now - Duration::hours(2)),
    )
    .await;

    let rows = state
        .posts
        .list_drafts_by_user(
            user_id,
            None,
            parse_row_limit("50"),
            common::time::UtcInstant::from(now),
        )
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
    use chrono::{Duration, TimeZone};
    let env = backend.setup().await;
    let state = &env.state;
    let after = Utc.with_ymd_and_hms(2026, 6, 26, 12, 0, 0).unwrap();
    let upto = after + Duration::hours(1);
    let alice = SeedUser::new().seed(state).await;
    let bob = SeedUser::new().seed(state).await;

    // Inside the window (after, upto], tagged: must be returned with its tag.
    let inside = seed_post_published_at(
        state,
        alice.user_id,
        "in-window",
        common::time::UtcInstant::from(after + Duration::minutes(30)),
    )
    .await;
    storage::test_support::set_post_tags_confirmed(
        &state.write_scope,
        std::sync::Arc::clone(&state.posts),
        inside,
        alice.user_id,
        &["scheduling".parse::<TagLabel>().unwrap()],
    )
    .await
    .unwrap();
    // Exactly at the inclusive upper bound: must be returned (untagged).
    seed_post_published_at(
        state,
        bob.user_id,
        "at-upto",
        common::time::UtcInstant::from(upto),
    )
    .await;
    // Exactly at the exclusive lower bound: must be excluded.
    seed_post_published_at(
        state,
        alice.user_id,
        "at-after",
        common::time::UtcInstant::from(after),
    )
    .await;
    // Past the window: must be excluded.
    seed_post_published_at(
        state,
        alice.user_id,
        "out-window",
        common::time::UtcInstant::from(upto + Duration::hours(1)),
    )
    .await;

    let live: Vec<GoLivePost> = state
        .posts
        .list_posts_gone_live_between(
            common::time::UtcInstant::from(after),
            common::time::UtcInstant::from(upto),
        )
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
    use chrono::{Duration, TimeZone};
    use common::feed::{FeedFormat, FeedSurface};
    use host::feed::{FeedPath, SyndicationFeedRepresentation};
    let env = backend.setup().await;
    let state = &env.state;
    let now = Utc.with_ymd_and_hms(2026, 6, 26, 12, 0, 0).unwrap();
    let t0 = now - Duration::hours(2);
    let alice = SeedUser::new().seed(state).await;

    // A live post, newer than t0, on the site/user feeds and — once tagged —
    // on the site-tag and user-tag feeds too.
    let post = seed_post_published_at(
        state,
        alice.user_id,
        "live-one",
        common::time::UtcInstant::from(now - Duration::hours(1)),
    )
    .await;
    storage::test_support::set_post_tags_confirmed(
        &state.write_scope,
        std::sync::Arc::clone(&state.posts),
        post,
        alice.user_id,
        &["rust".parse::<TagLabel>().unwrap()],
    )
    .await
    .unwrap();

    let mk_row = |feed_url: &str, generated_at: UtcInstant| {
        FeedCacheRow::new(
            fp(feed_url),
            SyndicationFeedRepresentation::try_from_stored(
                FeedFormat::Atom,
                FeedFormat::Atom.content_type(),
                "cached".to_string(),
            )
            .expect("matching stored representation metadata"),
            parse_etag("\"etag\""),
            generated_at,
            generated_at,
        )
        .expect("matching cache row formats")
    };
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
    upsert_cache_confirmed(state, mk_row("/feed.atom", UtcInstant::from(t0))).await;
    upsert_cache_confirmed(state, mk_row(&site_tag_url, UtcInstant::from(t0))).await;
    upsert_cache_confirmed(state, mk_row(&user_tag_url, UtcInstant::from(t0))).await;
    // Fresh (generated after the newest live post) => must NOT be returned.
    upsert_cache_confirmed(state, mk_row("/~alice/feed.atom", UtcInstant::from(now))).await;

    let stale = state
        .posts
        .feed_urls_needing_catchup(common::time::UtcInstant::from(now))
        .await
        .unwrap();
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
}

#[apply(backends)]
#[tokio::test]
async fn tag_list_pagination(#[case] backend: Backend) {
    let env = backend.setup().await;
    let state = &env.state;
    let user = SeedUser::new()
        .display_name("Pagination")
        .seed(state)
        .await
        .user_id;

    let mut post_ids = Vec::new();
    for _ in 0..5 {
        let post_id = SeedRawPost::new(user).seed(state).await.post_id;
        post_ids.push(post_id);

        storage::test_support::set_post_tags_confirmed(
            &state.write_scope,
            std::sync::Arc::clone(&state.posts),
            post_id,
            user,
            &["pagination-test".parse::<TagLabel>().unwrap()],
        )
        .await
        .expect("set_post_tags failed");
    }

    let tag_slug: Tag = "pagination-test".parse().unwrap();
    let posts = anon_by_tag(state, &tag_slug, "2").await;

    assert_eq!(posts.len(), 2);
    // Should be reverse chronological
    assert!(posts[0].created_at >= posts[1].created_at);
}

#[apply(backends)]
#[tokio::test]
async fn list_user_posts_by_tag_excludes_other_users(#[case] backend: Backend) {
    let env = backend.setup().await;
    let state = &env.state;
    let user1 = SeedUser::new()
        .display_name("User1")
        .seed(state)
        .await
        .user_id;

    let user2 = SeedUser::new()
        .display_name("User2")
        .seed(state)
        .await
        .user_id;

    let post1 = SeedRawPost::new(user1).seed(state).await.post_id;

    let post2 = SeedRawPost::new(user2).seed(state).await.post_id;

    storage::test_support::set_post_tags_confirmed(
        &state.write_scope,
        std::sync::Arc::clone(&state.posts),
        post1,
        user1,
        &["shared-tag".parse::<TagLabel>().unwrap()],
    )
    .await
    .expect("tag post1 failed");
    storage::test_support::set_post_tags_confirmed(
        &state.write_scope,
        std::sync::Arc::clone(&state.posts),
        post2,
        user2,
        &["shared-tag".parse::<TagLabel>().unwrap()],
    )
    .await
    .expect("tag post2 failed");

    let tag_slug: Tag = "shared-tag".parse().unwrap();
    let user1_posts = anon_user_by_tag(state, user1, &tag_slug, "50").await;

    assert_eq!(user1_posts.len(), 1);
    assert_eq!(user1_posts[0].post_id, post1);

    let user2_posts = anon_user_by_tag(state, user2, &tag_slug, "50").await;

    assert_eq!(user2_posts.len(), 1);
    assert_eq!(user2_posts[0].post_id, post2);
}

#[apply(backends)]
#[tokio::test]
async fn list_posts_by_nonexistent_tag(#[case] backend: Backend) {
    let env = backend.setup().await;
    let state = &env.state;
    let tag_slug: Tag = "nosuch-tag".parse().unwrap();
    let result = state
        .posts
        .list_posts_by_tag(
            &tag_slug,
            None,
            parse_row_limit("50"),
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
    let state = &env.state;
    let user = SeedUser::new()
        .display_name("UserTagNope")
        .seed(state)
        .await
        .user_id;

    let tag_slug: Tag = "nonexistent-tag-99".parse().unwrap();
    let result = state
        .posts
        .list_user_posts_by_tag(
            user,
            &tag_slug,
            None,
            parse_row_limit("50"),
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
    let state = &env.state;
    let user1 = SeedUser::new()
        .display_name("Eve")
        .seed(state)
        .await
        .user_id;

    let user2 = SeedUser::new()
        .display_name("Frank")
        .seed(state)
        .await
        .user_id;

    let post1 = SeedRawPost::new(user1).seed(state).await.post_id;

    let post2 = SeedRawPost::new(user2).seed(state).await.post_id;

    storage::test_support::set_post_tags_confirmed(
        &state.write_scope,
        std::sync::Arc::clone(&state.posts),
        post1,
        user1,
        &["javascript".parse::<TagLabel>().unwrap()],
    )
    .await
    .expect("set_post_tags failed");
    storage::test_support::set_post_tags_confirmed(
        &state.write_scope,
        std::sync::Arc::clone(&state.posts),
        post2,
        user2,
        &["javascript".parse::<TagLabel>().unwrap()],
    )
    .await
    .expect("set_post_tags failed");

    let tag_slug: Tag = "javascript".parse().unwrap();
    let posts = anon_by_tag(state, &tag_slug, "50").await;

    assert_eq!(posts.len(), 2);
    assert!(posts.iter().any(|p| p.post_id == post1));
    assert!(posts.iter().any(|p| p.post_id == post2));
}

#[apply(backends)]
#[tokio::test]
async fn list_user_posts_by_tag(#[case] backend: Backend) {
    let env = backend.setup().await;
    let state = &env.state;
    let user1 = SeedUser::new()
        .display_name("Grace")
        .seed(state)
        .await
        .user_id;

    let user2 = SeedUser::new()
        .display_name("Henry")
        .seed(state)
        .await
        .user_id;

    let post1 = SeedRawPost::new(user1).seed(state).await.post_id;

    let post2 = SeedRawPost::new(user1).seed(state).await.post_id;

    let post3 = SeedRawPost::new(user2).seed(state).await.post_id;

    storage::test_support::set_post_tags_confirmed(
        &state.write_scope,
        std::sync::Arc::clone(&state.posts),
        post1,
        user1,
        &["clojure".parse::<TagLabel>().unwrap()],
    )
    .await
    .expect("set_post_tags failed");
    storage::test_support::set_post_tags_confirmed(
        &state.write_scope,
        std::sync::Arc::clone(&state.posts),
        post2,
        user1,
        &["clojure".parse::<TagLabel>().unwrap()],
    )
    .await
    .expect("set_post_tags failed");
    storage::test_support::set_post_tags_confirmed(
        &state.write_scope,
        std::sync::Arc::clone(&state.posts),
        post3,
        user2,
        &["clojure".parse::<TagLabel>().unwrap()],
    )
    .await
    .expect("set_post_tags failed");

    let tag_slug: Tag = "clojure".parse().unwrap();
    let posts = anon_user_by_tag(state, user1, &tag_slug, "50").await;

    assert_eq!(posts.len(), 2);
    assert!(posts.iter().all(|p| p.user_id == user1));
}

#[apply(backends)]
#[tokio::test]
async fn tag_not_found_error(#[case] backend: Backend) {
    let env = backend.setup().await;
    let state = &env.state;
    let tag_slug: Tag = "nonexistent".parse().unwrap();
    let result = state
        .posts
        .list_posts_by_tag(
            &tag_slug,
            None,
            parse_row_limit("50"),
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
    let state = &env.state;
    let user = SeedUser::new()
        .display_name("Iris")
        .seed(state)
        .await
        .user_id;

    let post1 = SeedRawPost::new(user).seed(state).await.post_id;

    let post2 = SeedRawPost::new(user).seed(state).await.post_id;

    storage::test_support::set_post_tags_confirmed(
        &state.write_scope,
        std::sync::Arc::clone(&state.posts),
        post1,
        user,
        &["haskell".parse::<TagLabel>().unwrap()],
    )
    .await
    .expect("set_post_tags failed");
    storage::test_support::set_post_tags_confirmed(
        &state.write_scope,
        std::sync::Arc::clone(&state.posts),
        post2,
        user,
        &["haskell".parse::<TagLabel>().unwrap()],
    )
    .await
    .expect("set_post_tags failed");

    soft_delete_post_confirmed(state, post1, user).await;

    let tag_slug: Tag = "haskell".parse().unwrap();
    let posts = anon_by_tag(state, &tag_slug, "50").await;

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
    let state = &env.state;
    let user = SeedUser::new()
        .display_name("Jack")
        .seed(state)
        .await
        .user_id;

    let post1 = SeedRawPost::new(user).draft().seed(state).await.post_id;

    let post2 = SeedRawPost::new(user).seed(state).await.post_id;

    storage::test_support::set_post_tags_confirmed(
        &state.write_scope,
        std::sync::Arc::clone(&state.posts),
        post1,
        user,
        &["kotlin".parse::<TagLabel>().unwrap()],
    )
    .await
    .expect("set_post_tags failed");
    storage::test_support::set_post_tags_confirmed(
        &state.write_scope,
        std::sync::Arc::clone(&state.posts),
        post2,
        user,
        &["kotlin".parse::<TagLabel>().unwrap()],
    )
    .await
    .expect("set_post_tags failed");

    let tag_slug: Tag = "kotlin".parse().unwrap();
    let posts = anon_by_tag(state, &tag_slug, "50").await;

    assert_eq!(posts.len(), 1);
    assert_eq!(posts[0].post_id, post2);
}

// ====== Additional coverage tests for error paths ======

#[apply(backends)]
#[tokio::test]
async fn list_published_cursor_boundary(#[case] backend: Backend) {
    let env = backend.setup().await;
    let state = &env.state;
    let user = SeedUser::new().seed(state).await.user_id;

    for _ in 0..5 {
        SeedRawPost::new(user).seed(state).await;
    }

    let all = anon_published(state, "10").await;
    assert_eq!(all.len(), 5);

    let first = anon_published(state, "2").await;
    assert_eq!(first.len(), 2);

    if !first.is_empty() {
        let cursor = PostCursor {
            created_at: first[first.len() - 1].created_at,
            post_id: first[first.len() - 1].post_id,
        };
        let next = state
            .posts
            .list_published(
                Some(&cursor),
                parse_row_limit("2"),
                &ViewerIdentity::Anonymous,
                common::time::UtcInstant::now(),
            )
            .await
            .expect("list_published with cursor failed");
        assert_eq!(next.len(), 2);
    }
}

#[apply(backends)]
#[tokio::test]
async fn list_drafts_cursor_boundary(#[case] backend: Backend) {
    let env = backend.setup().await;
    let state = &env.state;
    let user = SeedUser::new().seed(state).await.user_id;

    let _now = Utc::now();

    for _ in 0..3 {
        SeedRawPost::new(user).draft().seed(state).await;
    }

    let all = drafts_of(state, user, "10").await;
    assert_eq!(all.len(), 3);

    let first = drafts_of(state, user, "1").await;
    assert_eq!(first.len(), 1);

    if !first.is_empty() {
        let cursor = PostCursor {
            created_at: first[0].created_at,
            post_id: first[0].post_id,
        };
        let next = state
            .posts
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
    let state = &env.state;
    let user = SeedUser::new().seed(state).await.user_id;

    for _ in 0..3 {
        let post_id = SeedRawPost::new(user).seed(state).await.post_id;

        storage::test_support::set_post_tags_confirmed(
            &state.write_scope,
            std::sync::Arc::clone(&state.posts),
            post_id,
            user,
            &["cursor-tag".parse::<TagLabel>().unwrap()],
        )
        .await
        .expect("set_post_tags failed");
    }

    let tag: Tag = "cursor-tag".parse().unwrap();

    let all = anon_user_by_tag(state, user, &tag, "10").await;
    assert_eq!(all.len(), 3);

    let first = anon_user_by_tag(state, user, &tag, "1").await;
    assert_eq!(first.len(), 1);

    if !first.is_empty() {
        let cursor = PostCursor {
            created_at: first[0].created_at,
            post_id: first[0].post_id,
        };
        let next = state
            .posts
            .list_user_posts_by_tag(
                user,
                &tag,
                Some(&cursor),
                parse_row_limit("2"),
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
    let state = &env.state;
    let user = SeedUser::new().seed(state).await.user_id;

    for _ in 0..3 {
        let post_id = SeedRawPost::new(user).seed(state).await.post_id;

        storage::test_support::set_post_tags_confirmed(
            &state.write_scope,
            std::sync::Arc::clone(&state.posts),
            post_id,
            user,
            &["global-tag".parse::<TagLabel>().unwrap()],
        )
        .await
        .expect("set_post_tags failed");
    }

    let tag: Tag = "global-tag".parse().unwrap();

    let all = anon_by_tag(state, &tag, "10").await;
    assert_eq!(all.len(), 3);

    let first = anon_by_tag(state, &tag, "1").await;
    assert_eq!(first.len(), 1);

    if !first.is_empty() {
        let cursor = PostCursor {
            created_at: first[0].created_at,
            post_id: first[0].post_id,
        };
        let next = state
            .posts
            .list_posts_by_tag(
                &tag,
                Some(&cursor),
                parse_row_limit("2"),
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
    let state = &env.state;
    let user = SeedUser::new().seed(state).await;

    let posts = anon_published_by_user(state, &user.username, "10").await;
    assert!(posts.is_empty());

    let cursor = PostCursor {
        created_at: common::time::UtcInstant::now(),
        post_id: PostId::from(999),
    };
    let posts = state
        .posts
        .list_published_by_user(
            &user.username,
            Some(&cursor),
            parse_row_limit("10"),
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
    let state = &env.state;
    let user = SeedUser::new().seed(state).await;

    let created_at = Utc::now();

    let seeded = SeedRawPost::new(user.user_id)
        .published_at(common::time::UtcInstant::from(created_at))
        .seed(state)
        .await;

    let post = state
        .posts
        .get_post_by_permalink(
            &user.username,
            permalink_date(created_at.year(), created_at.month(), created_at.day()),
            &seeded.slug,
            &ViewerIdentity::Anonymous,
            common::time::UtcInstant::now(),
        )
        .await
        .expect("get_post_by_permalink failed");
    assert!(post.is_some());

    soft_delete_post_confirmed(state, seeded.post_id, user.user_id).await;

    let post = state
        .posts
        .get_post_by_permalink(
            &user.username,
            permalink_date(created_at.year(), created_at.month(), created_at.day()),
            &seeded.slug,
            &ViewerIdentity::Anonymous,
            common::time::UtcInstant::now(),
        )
        .await
        .expect("get_post_by_permalink after delete failed");
    assert!(post.is_none());
}

// ====== Comprehensive error path coverage ======

#[apply(backends)]
#[tokio::test]
async fn list_published_with_cursor_same_timestamp(#[case] backend: Backend) {
    let env = backend.setup().await;
    let state = &env.state;
    let user = SeedUser::new().seed(state).await.user_id;

    // Create posts at the same time
    let mut post_ids = vec![];
    for _ in 0..4 {
        let post_id = SeedRawPost::new(user).seed(state).await.post_id;
        post_ids.push(post_id);
    }

    let first = anon_published(state, "2").await;
    assert_eq!(first.len(), 2);

    // Use cursor to get next batch with same created_at but different post_id
    if !first.is_empty() {
        let cursor = PostCursor {
            created_at: first[first.len() - 1].created_at,
            post_id: first[first.len() - 1].post_id,
        };
        let next = state
            .posts
            .list_published(
                Some(&cursor),
                parse_row_limit("2"),
                &ViewerIdentity::Anonymous,
                common::time::UtcInstant::now(),
            )
            .await
            .expect("list_published with cursor failed");
        assert_eq!(next.len(), 2);
    }
}
