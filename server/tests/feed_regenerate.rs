mod helpers;

use chrono::Utc;
use common::password::Password;
use common::slug::Slug;
use common::username::Username;
use jaunder::feed::regenerate::regenerate_feed;
use storage::{CreatePostInput, PostFormat};
use tempfile::TempDir;

#[tokio::test]
async fn regenerate_writes_cache_row_for_user_feed() {
    let base = TempDir::new().expect("temp dir");
    let state = helpers::test_state(&base).await;

    // Create a user
    let username: Username = "alice".parse().expect("valid username");
    let password: Password = "password123".parse().expect("valid password");
    let user_id = state
        .users
        .create_user(&username, &password, None, false)
        .await
        .expect("create user");

    // Create 2 published posts
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
        })
        .await
        .expect("create post 2");

    // Regenerate feed
    let row = regenerate_feed(&state, "/~alice/feed.rss")
        .await
        .expect("regenerate feed");

    // Assert content type
    assert_eq!(
        row.content_type, "application/rss+xml; charset=utf-8",
        "RSS content type"
    );

    // Assert cache was written
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

#[tokio::test]
async fn regenerate_writes_empty_feed_for_user_with_no_posts() {
    let base = TempDir::new().expect("temp dir");
    let state = helpers::test_state(&base).await;

    // Create a user but no posts
    let username: Username = "bob".parse().expect("valid username");
    let password: Password = "password123".parse().expect("valid password");
    state
        .users
        .create_user(&username, &password, None, false)
        .await
        .expect("create user");

    // Regenerate feed
    let row = regenerate_feed(&state, "/~bob/feed.rss")
        .await
        .expect("regenerate feed");

    // Should have valid content type and non-empty body
    assert_eq!(
        row.content_type, "application/rss+xml; charset=utf-8",
        "empty feed has correct content type"
    );
    assert!(!row.body.is_empty(), "empty feed still has valid body");
    // Verify it was cached
    let cached = state
        .feed_cache
        .get("/~bob/feed.rss")
        .await
        .expect("get from cache")
        .expect("cache entry exists");
    assert_eq!(cached.body, row.body, "cached body matches returned body");
}

#[tokio::test]
async fn regenerate_writes_each_format() {
    let base = TempDir::new().expect("temp dir");
    let state = helpers::test_state(&base).await;

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
        let row = regenerate_feed(&state, feed_url)
            .await
            .expect(&format!("regenerate {feed_url}"));
        assert_eq!(
            row.content_type, *expected_content_type,
            "content_type for {feed_url}"
        );
        assert!(!row.body.is_empty(), "body not empty for {feed_url}");
    }
}
