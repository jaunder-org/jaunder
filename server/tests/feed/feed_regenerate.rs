use common::{
    ids::{PostId, UserId},
    tagged_url::HubUrl,
    time::UtcInstant,
    visibility::AudienceTarget,
};
use jaunder::{feed::regenerate::render, publisher::PublisherService};

use chrono::{TimeZone, Utc};
use rstest::*;
use rstest_reuse::*;

use std::sync::Arc;

use storage::{
    CacheCommitOutcome, FeedCacheRow,
    test_support::{Backend, SeedRawPost, SeedUser, TestEnv, backends, confirmed_for, fp},
};

async fn render_feed(
    state: &Arc<storage::AppState>,
    feed_path: host::feed::FeedPath,
) -> storage::FeedCacheRow {
    let snapshot = state
        .publisher
        .snapshot()
        .await
        .expect("publisher snapshot");
    render(&snapshot, state.posts.as_ref(), feed_path)
        .await
        .expect("render feed")
}

fn fixed_instant(day: u32) -> UtcInstant {
    UtcInstant::from(
        Utc.with_ymd_and_hms(2024, 1, day, 0, 0, 0)
            .single()
            .expect("fixed test instant"),
    )
}

async fn render_and_commit(
    state: &Arc<storage::AppState>,
    publisher: &PublisherService,
    feed_path: host::feed::FeedPath,
    generated_at: UtcInstant,
) -> FeedCacheRow {
    let snapshot = publisher.snapshot().await.expect("publisher snapshot");
    let candidate = render(&snapshot, state.posts.as_ref(), feed_path)
        .await
        .expect("render feed");
    let candidate = FeedCacheRow::new(
        candidate.feed_path().clone(),
        candidate.representation().clone(),
        candidate.etag.clone(),
        candidate.representation_modified_at,
        generated_at,
        candidate.semantic_fingerprint().clone(),
    )
    .expect("rendered row has matching feed format");
    let guard = publisher
        .finalization_guard()
        .await
        .expect("publisher finalization guard");

    match guard
        .commit_cache(snapshot.generation, candidate)
        .await
        .expect("commit cache")
    {
        CacheCommitOutcome::Committed(row) => row,
        CacheCommitOutcome::StaleGeneration => panic!("unchanged generation should commit"),
    }
}

async fn delete_post(
    state: &Arc<storage::AppState>,
    post_id: PostId,
    user_id: UserId,
    deleted_at: UtcInstant,
) {
    let posts = Arc::clone(&state.posts);
    let outcome = state
        .write_scope
        .run(move |transaction| {
            Box::pin(async move {
                posts
                    .soft_delete_post(transaction, post_id, user_id, deleted_at)
                    .await
            })
        })
        .await
        .expect("soft-delete post");
    confirmed_for(outcome, "soft-delete post");
}

#[apply(backends)]
#[tokio::test]
async fn render_user_feed_returns_expected_rss_representation(#[case] backend: Backend) {
    let TestEnv { state, base: _base } = backend.setup().await;

    let user = SeedUser::new().seed(&state).await;

    SeedRawPost::new(user.user_id).seed(&state).await;
    SeedRawPost::new(user.user_id).seed(&state).await;

    let row = render_feed(&state, fp(&format!("/~{}/feed.rss", user.username))).await;

    assert_eq!(
        row.representation().content_type(),
        "application/rss+xml; charset=utf-8",
        "RSS content type"
    );
}

#[apply(backends)]
#[tokio::test]
async fn render_empty_user_feed_returns_representation(#[case] backend: Backend) {
    let TestEnv { state, base: _base } = backend.setup().await;

    // Create a user but no posts
    let user = SeedUser::new().seed(&state).await;

    let row = render_feed(&state, fp(&format!("/~{}/feed.rss", user.username))).await;

    assert_eq!(
        row.representation().content_type(),
        "application/rss+xml; charset=utf-8",
        "empty feed has correct content type"
    );
    assert!(
        !row.representation().body().is_empty(),
        "empty feed still has valid body"
    );
}

#[apply(backends)]
#[tokio::test]
async fn render_tag_surfaces_returns_representations(#[case] backend: Backend) {
    let TestEnv { state, base: _base } = backend.setup().await;

    // Create a user (posts are not required: the tag-window queries and the
    // SiteTag/UserTag canonical_url arms execute regardless of matches).
    let user = SeedUser::new().seed(&state).await;

    // Site-tag surface exercises the SiteTag canonical_url arm and the
    // window_site_tag storage query.
    let site_tag = render_feed(&state, fp("/tags/rust/feed.rss")).await;
    assert_eq!(
        site_tag.representation().content_type(),
        "application/rss+xml; charset=utf-8",
        "site-tag RSS content type"
    );

    // User-tag surface exercises the UserTag canonical_url arm and the
    // window_user_tag storage query.
    let user_tag = render_feed(
        &state,
        fp(&format!("/~{}/tags/rust/feed.rss", user.username)),
    )
    .await;
    assert_eq!(
        user_tag.representation().content_type(),
        "application/rss+xml; charset=utf-8",
        "user-tag RSS content type"
    );
}

#[apply(backends)]
#[tokio::test]
async fn render_each_format(#[case] backend: Backend) {
    let TestEnv { state, base: _base } = backend.setup().await;

    // Create a user with one post
    let user = SeedUser::new().seed(&state).await;

    SeedRawPost::new(user.user_id).seed(&state).await;

    // Test each format
    let formats = [
        (
            format!("/~{}/feed.rss", user.username),
            "application/rss+xml; charset=utf-8",
        ),
        (
            format!("/~{}/feed.atom", user.username),
            "application/atom+xml; charset=utf-8",
        ),
        (
            format!("/~{}/feed.json", user.username),
            "application/feed+json",
        ),
    ];

    for (feed_url, expected_content_type) in &formats {
        let row = render_feed(&state, fp(feed_url)).await;
        assert_eq!(
            row.representation().content_type(),
            *expected_content_type,
            "content_type for {feed_url}"
        );
        assert!(
            !row.representation().body().is_empty(),
            "body not empty for {feed_url}"
        );
    }
}

/// Published feeds are public-only (M8): [`render`] resolves posts as an
/// anonymous viewer, so a mix of Public / Subscribers / Private posts emits ONLY
/// the Public one. This locks the `ViewerIdentity::Anonymous` intent in
/// [`render`] — if a non-anonymous viewer ever leaked in, the
/// Subscribers/Private titles would appear and this test would fail.
#[apply(backends)]
#[tokio::test]
async fn feed_contains_only_public_posts(#[case] backend: Backend) {
    let TestEnv { state, base: _base } = backend.setup().await;

    let user = SeedUser::new().seed(&state).await;

    let public = SeedRawPost::new(user.user_id)
        .audiences(vec![AudienceTarget::Public])
        .seed(&state)
        .await;
    let subscribers = SeedRawPost::new(user.user_id)
        .audiences(vec![AudienceTarget::Subscribers])
        .seed(&state)
        .await;
    // Private = no audience rows.
    let private = SeedRawPost::new(user.user_id)
        .audiences(vec![])
        .seed(&state)
        .await;

    let row = render_feed(&state, fp(&format!("/~{}/feed.rss", user.username))).await;

    let body = row.representation().body();
    assert!(
        body.contains(public.title.as_ref()),
        "Public post must appear in the feed: {body}",
    );
    assert!(
        !body.contains(subscribers.title.as_ref()),
        "Subscribers post must NOT appear in the public feed: {body}",
    );
    assert!(
        !body.contains(private.title.as_ref()),
        "Private post must NOT appear in the public feed: {body}",
    );
}

/// #772: the feed reads tags off the records `list_published_in_window` already
/// returned instead of issuing one per-post tag query. This pins the
/// observable contract that must survive that switch — tags still reach the body,
/// slug-ordered even when written in the opposite order.
///
/// JSON is deliberate. RSS renders no tags at all (`common/src/feed/rss.rs` never
/// reads `item.tags`), and Atom's `<category term=…>` would force a substring-index
/// comparison; JSON Feed's `tags` is an array that takes an exact vector assertion.
#[apply(backends)]
#[tokio::test]
async fn regenerated_json_feed_carries_slug_ordered_tags(#[case] backend: Backend) {
    let TestEnv { state, base: _base } = backend.setup().await;

    let user = SeedUser::new().seed(&state).await;
    // Applied in reverse-slug order: an unordered read would surface "web" first.
    SeedRawPost::new(user.user_id)
        .tags(["web", "Rust"])
        .seed(&state)
        .await;

    let row = render_feed(&state, fp(&format!("/~{}/feed.json", user.username))).await;

    let body = row.representation().body();
    let v: serde_json::Value = serde_json::from_str(body).expect("feed body is JSON");
    assert_eq!(
        v["items"].as_array().map(Vec::len),
        Some(1),
        "one published post in the feed: {body}",
    );
    // Ordered by slug (rust < web); the *display* casing the author supplied is
    // what the feed emits.
    assert_eq!(
        v["items"][0]["tags"],
        serde_json::json!(["Rust", "web"]),
        "tags slug-ordered in the JSON feed body: {body}",
    );
}

#[apply(backends)]
#[tokio::test]
async fn regeneration_preserves_identity_only_for_byte_identical_cached_representations(
    #[case] backend: Backend,
) {
    let TestEnv { state, base } = backend.setup().await;
    let publisher = PublisherService::new(
        base.path().to_path_buf(),
        Arc::clone(&state.publisher),
        state.write_scope.clone(),
    );
    let user = SeedUser::new().seed(&state).await;
    let feed_path = fp(&format!("/~{}/feed.rss", user.username));

    let empty = render_and_commit(&state, &publisher, feed_path.clone(), fixed_instant(1)).await;
    let empty_no_op =
        render_and_commit(&state, &publisher, feed_path.clone(), fixed_instant(2)).await;
    assert_eq!(
        empty_no_op.representation().body(),
        empty.representation().body(),
        "empty-feed regeneration preserves the stored body",
    );
    assert_eq!(empty_no_op.etag, empty.etag, "empty-feed ETag is stable");
    assert_eq!(
        empty_no_op.representation_modified_at, empty.representation_modified_at,
        "empty-feed representation time is stable",
    );
    assert_eq!(
        empty_no_op.generated_at,
        fixed_instant(2),
        "no-op regeneration advances generated_at through the fenced commit",
    );

    let hub: HubUrl = "https://hub.example.test/".parse().expect("valid hub URL");
    confirmed_for(
        publisher
            .mutate_hub_with_feedback(Some(&hub))
            .await
            .expect("set WebSub hub"),
        "set WebSub hub",
    );
    let metadata_changed =
        render_and_commit(&state, &publisher, feed_path.clone(), fixed_instant(3)).await;
    assert_ne!(
        metadata_changed.representation().body(),
        empty.representation().body(),
        "metadata-only hub change replaces the serialized representation",
    );
    assert_ne!(
        metadata_changed.etag, empty.etag,
        "metadata-only hub change replaces representation identity",
    );

    let first = SeedRawPost::new(user.user_id).seed(&state).await;
    let from_empty =
        render_and_commit(&state, &publisher, feed_path.clone(), fixed_instant(4)).await;
    assert_ne!(
        from_empty.representation().body(),
        metadata_changed.representation().body(),
        "adding the first item transitions from an empty feed",
    );
    assert_ne!(
        from_empty.etag, metadata_changed.etag,
        "transition from empty changes representation identity",
    );

    let second = SeedRawPost::new(user.user_id).seed(&state).await;
    let two_items =
        render_and_commit(&state, &publisher, feed_path.clone(), fixed_instant(5)).await;
    delete_post(&state, second.post_id, user.user_id, fixed_instant(6)).await;
    let one_item = render_and_commit(&state, &publisher, feed_path.clone(), fixed_instant(7)).await;
    assert_ne!(
        one_item.representation().body(),
        two_items.representation().body(),
        "removing one item replaces the cached representation",
    );
    assert_ne!(
        one_item.etag, two_items.etag,
        "removing one item changes representation identity",
    );
    assert!(
        one_item
            .representation()
            .body()
            .contains(first.title.as_ref()),
        "the remaining item survives a removal",
    );
    assert!(
        !one_item
            .representation()
            .body()
            .contains(second.title.as_ref()),
        "the removed item leaves the representation",
    );

    delete_post(&state, first.post_id, user.user_id, fixed_instant(8)).await;
    let to_empty = render_and_commit(&state, &publisher, feed_path, fixed_instant(9)).await;
    assert_ne!(
        to_empty.representation().body(),
        one_item.representation().body(),
        "removing the final item transitions to an empty feed",
    );
    assert_ne!(
        to_empty.etag, one_item.etag,
        "transition to empty changes representation identity",
    );
    assert!(
        !to_empty
            .representation()
            .body()
            .contains(first.title.as_ref()),
        "empty feed has no residual item",
    );
}
