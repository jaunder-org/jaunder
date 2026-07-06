use chrono::Utc;
use common::password::Password;
use common::slug::Slug;
use common::username::Username;
use common::visibility::AudienceTarget;
use jaunder::feed::regenerate::regenerate_feed;
use storage::{CreatePostInput, PostFormat};

use rstest::*;
use rstest_reuse::*;

use crate::helpers::{backends, Backend, TestEnv};

#[apply(backends)]
#[tokio::test]
async fn regenerate_writes_cache_row_for_user_feed(#[case] backend: Backend) {
    let TestEnv { state, base: _base } = backend.setup().await;

    let username: Username = "alice".parse().expect("valid username");
    let password: Password = "password123".parse().expect("valid password");
    let user_id = state
        .users
        .create_user(&username, &password, None, false)
        .await
        .expect("create user");

    let now = Utc::now();
    let _post1_id = state
        .posts
        .create_post(&CreatePostInput {
            user_id,
            title: Some("Post 1".to_string()),
            slug: "post-1".parse::<Slug>().expect("valid slug"),
            body: "Post 1 body".to_string(),
            format: PostFormat::Markdown,
            rendered_html: "<p>Post 1 body</p>".to_string(),
            published_at: Some(now),
            summary: None,
            audiences: vec![AudienceTarget::Public],
        })
        .await
        .expect("create post 1");

    let _post2_id = state
        .posts
        .create_post(&CreatePostInput {
            user_id,
            title: Some("Post 2".to_string()),
            slug: "post-2".parse::<Slug>().expect("valid slug"),
            body: "Post 2 body".to_string(),
            format: PostFormat::Markdown,
            rendered_html: "<p>Post 2 body</p>".to_string(),
            published_at: Some(now),
            summary: None,
            audiences: vec![AudienceTarget::Public],
        })
        .await
        .expect("create post 2");

    let row = regenerate_feed(
        state.site_config.as_ref(),
        state.posts.as_ref(),
        state.feed_cache.as_ref(),
        "/~alice/feed.rss",
    )
    .await
    .expect("regenerate feed");

    assert_eq!(
        row.content_type, "application/rss+xml; charset=utf-8",
        "RSS content type"
    );

    let from_cache = state
        .feed_cache
        .get("/~alice/feed.rss")
        .await
        .expect("get from cache")
        .expect("cache entry exists");

    assert_eq!(
        from_cache.body, row.body,
        "cached body matches returned row"
    );
    assert_eq!(
        from_cache.etag, row.etag,
        "cached etag matches returned row"
    );
}

#[apply(backends)]
#[tokio::test]
async fn regenerate_writes_empty_feed_for_user_with_no_posts(#[case] backend: Backend) {
    let TestEnv { state, base: _base } = backend.setup().await;

    // Create a user but no posts
    let username: Username = "bob".parse().expect("valid username");
    let password: Password = "password123".parse().expect("valid password");
    state
        .users
        .create_user(&username, &password, None, false)
        .await
        .expect("create user");

    let row = regenerate_feed(
        state.site_config.as_ref(),
        state.posts.as_ref(),
        state.feed_cache.as_ref(),
        "/~bob/feed.rss",
    )
    .await
    .expect("regenerate feed");

    assert_eq!(
        row.content_type, "application/rss+xml; charset=utf-8",
        "empty feed has correct content type"
    );
    assert!(!row.body.is_empty(), "empty feed still has valid body");
    let cached = state
        .feed_cache
        .get("/~bob/feed.rss")
        .await
        .expect("get from cache")
        .expect("cache entry exists");
    assert_eq!(cached.body, row.body, "cached body matches returned body");
}

#[apply(backends)]
#[tokio::test]
async fn regenerate_writes_cache_rows_for_tag_surfaces(#[case] backend: Backend) {
    let TestEnv { state, base: _base } = backend.setup().await;

    // Create a user (posts are not required: the tag-window queries and the
    // SiteTag/UserTag canonical_url arms execute regardless of matches).
    let username: Username = "alice".parse().expect("valid username");
    let password: Password = "password123".parse().expect("valid password");
    state
        .users
        .create_user(&username, &password, None, false)
        .await
        .expect("create user");

    // Site-tag surface exercises the SiteTag canonical_url arm and the
    // window_site_tag storage query.
    let site_tag = regenerate_feed(
        state.site_config.as_ref(),
        state.posts.as_ref(),
        state.feed_cache.as_ref(),
        "/tags/rust/feed.rss",
    )
    .await
    .expect("regenerate site-tag feed");
    assert_eq!(
        site_tag.content_type, "application/rss+xml; charset=utf-8",
        "site-tag RSS content type"
    );
    assert!(
        state
            .feed_cache
            .get("/tags/rust/feed.rss")
            .await
            .expect("get site-tag from cache")
            .is_some(),
        "site-tag feed should be cached"
    );

    // User-tag surface exercises the UserTag canonical_url arm and the
    // window_user_tag storage query.
    let user_tag = regenerate_feed(
        state.site_config.as_ref(),
        state.posts.as_ref(),
        state.feed_cache.as_ref(),
        "/~alice/tags/rust/feed.rss",
    )
    .await
    .expect("regenerate user-tag feed");
    assert_eq!(
        user_tag.content_type, "application/rss+xml; charset=utf-8",
        "user-tag RSS content type"
    );
    assert!(
        state
            .feed_cache
            .get("/~alice/tags/rust/feed.rss")
            .await
            .expect("get user-tag from cache")
            .is_some(),
        "user-tag feed should be cached"
    );
}

#[apply(backends)]
#[tokio::test]
async fn regenerate_writes_each_format(#[case] backend: Backend) {
    let TestEnv { state, base: _base } = backend.setup().await;

    // Create a user with one post
    let username: Username = "charlie".parse().expect("valid username");
    let password: Password = "password123".parse().expect("valid password");
    let user_id = state
        .users
        .create_user(&username, &password, None, false)
        .await
        .expect("create user");

    let now = Utc::now();
    state
        .posts
        .create_post(&CreatePostInput {
            user_id,
            title: Some("Test Post".to_string()),
            slug: "test-post".parse::<Slug>().expect("valid slug"),
            body: "Test body".to_string(),
            format: PostFormat::Markdown,
            rendered_html: "<p>Test body</p>".to_string(),
            published_at: Some(now),
            summary: None,
            audiences: vec![AudienceTarget::Public],
        })
        .await
        .expect("create post");

    // Test each format
    let formats = [
        ("/~charlie/feed.rss", "application/rss+xml; charset=utf-8"),
        ("/~charlie/feed.atom", "application/atom+xml; charset=utf-8"),
        ("/~charlie/feed.json", "application/feed+json"),
    ];

    for (feed_url, expected_content_type) in &formats {
        let row = regenerate_feed(
            state.site_config.as_ref(),
            state.posts.as_ref(),
            state.feed_cache.as_ref(),
            feed_url,
        )
        .await
        .unwrap_or_else(|_| panic!("regenerate {feed_url}"));
        assert_eq!(
            row.content_type, *expected_content_type,
            "content_type for {feed_url}"
        );
        assert!(!row.body.is_empty(), "body not empty for {feed_url}");
    }
}

/// Published feeds are public-only (M8): `regenerate_feed` resolves posts as an
/// anonymous viewer, so a mix of Public / Subscribers / Private posts emits ONLY
/// the Public one. This locks the `ViewerIdentity::Anonymous` intent in
/// `regenerate_feed` — if a non-anonymous viewer ever leaked in, the
/// Subscribers/Private titles would appear and this test would fail.
#[apply(backends)]
#[tokio::test]
async fn feed_contains_only_public_posts(#[case] backend: Backend) {
    let TestEnv { state, base: _base } = backend.setup().await;

    let username: Username = "alice".parse().expect("valid username");
    let password: Password = "password123".parse().expect("valid password");
    let user_id = state
        .users
        .create_user(&username, &password, None, false)
        .await
        .expect("create user");

    let now = Utc::now();
    let mk = |title: &str, slug: &str, audiences: Vec<AudienceTarget>| CreatePostInput {
        user_id,
        title: Some(title.to_string()),
        slug: slug.parse::<Slug>().expect("valid slug"),
        body: format!("{title} body"),
        format: PostFormat::Markdown,
        rendered_html: format!("<p>{title} body</p>"),
        published_at: Some(now),
        summary: None,
        audiences,
    };

    state
        .posts
        .create_post(&mk(
            "Public Post",
            "public-post",
            vec![AudienceTarget::Public],
        ))
        .await
        .expect("create public post");
    state
        .posts
        .create_post(&mk(
            "Subscribers Post",
            "subscribers-post",
            vec![AudienceTarget::Subscribers],
        ))
        .await
        .expect("create subscribers post");
    // Private = no audience rows.
    state
        .posts
        .create_post(&mk("Private Post", "private-post", vec![]))
        .await
        .expect("create private post");

    let row = regenerate_feed(
        state.site_config.as_ref(),
        state.posts.as_ref(),
        state.feed_cache.as_ref(),
        "/~alice/feed.rss",
    )
    .await
    .expect("regenerate feed");

    assert!(
        row.body.contains("Public Post"),
        "Public post must appear in the feed: {}",
        row.body
    );
    assert!(
        !row.body.contains("Subscribers Post"),
        "Subscribers post must NOT appear in the public feed: {}",
        row.body
    );
    assert!(
        !row.body.contains("Private Post"),
        "Private post must NOT appear in the public feed: {}",
        row.body
    );
}
