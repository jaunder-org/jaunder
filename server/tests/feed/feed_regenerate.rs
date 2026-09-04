use common::visibility::AudienceTarget;
use jaunder::feed::regenerate::render;

use rstest::*;
use rstest_reuse::*;

use std::sync::Arc;

use storage::test_support::{Backend, SeedRawPost, SeedUser, TestEnv, backends, fp};

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
