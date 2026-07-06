#![allow(clippy::expect_used, clippy::unwrap_used, clippy::similar_names)]

use chrono::{DateTime, Utc};
use common::password::Password;
use common::username::Username;
use common::visibility::{AudienceTarget, ViewerIdentity};
use jaunder::cli::StorageArgs;
use storage::{open_existing_database, CreatePostInput, PostFormat};

/// Fixed microsecond-precision publish time: deterministic and safe from
/// Postgres's µs quantization, so a restored value can be asserted exactly (DEC-D).
pub fn fixture_published_at() -> DateTime<Utc> {
    "2026-04-29T12:34:56.789012Z"
        .parse()
        .expect("valid fixture timestamp")
}

pub async fn populate_backup_fixture(args: &StorageArgs) -> i64 {
    let state = open_existing_database(&args.db)
        .await
        .expect("open database");
    let username: Username = "backupuser".parse().expect("valid username");
    let password: Password = "password123".parse().expect("valid password");
    let user_id = state
        .users
        .create_user(&username, &password, Some("Backup User"), true)
        .await
        .expect("create user");
    let post_id = state
        .posts
        .create_post(&CreatePostInput {
            user_id,
            title: Some("Restored Post".to_owned()),
            slug: "restored-post".parse().expect("valid slug"),
            body: "body text".to_owned(),
            format: PostFormat::Markdown,
            rendered_html: "<p>body text</p>".to_owned(),
            published_at: Some(fixture_published_at()),
            summary: None,
            audiences: vec![AudienceTarget::Public],
        })
        .await
        .expect("create post");
    state
        .posts
        .tag_post(post_id, "Backup-Test")
        .await
        .expect("tag post");
    std::fs::write(args.storage_path.join("media").join("avatar.txt"), "media")
        .expect("write media");
    post_id
}

pub async fn assert_backup_fixture_restored(args: &StorageArgs, post_id: i64) {
    let state = open_existing_database(&args.db)
        .await
        .expect("open restored database");
    let username: Username = "backupuser".parse().expect("valid username");
    let user = state
        .users
        .get_user_by_username(&username)
        .await
        .expect("get user")
        .expect("restored user");
    assert!(user.is_operator);
    assert_eq!(user.display_name.as_deref(), Some("Backup User"));

    // View as the restored post's author. Backup/restore does not yet carry the
    // `post_audiences` rows (the `TABLES_IN_EXPORT_ORDER` gap tracked in issue #4),
    // so an Anonymous viewer would be filtered out by the resolution predicate; the
    // owner is always admitted via the author branch, which is the correct viewer here.
    let local = state
        .subscriptions
        .local_channel_id()
        .await
        .expect("local channel id");
    let post = state
        .posts
        .get_post_by_id(post_id, &ViewerIdentity::local(user.user_id, local))
        .await
        .expect("get post")
        .expect("restored post");
    assert_eq!(post.title.as_deref(), Some("Restored Post"));
    assert_eq!(post.slug.as_str(), "restored-post");
    // Value interop (DEC-D): the timestamp survives with its value.
    assert_eq!(post.published_at, Some(fixture_published_at()));

    let tags = state
        .posts
        .get_tags_for_post(post_id)
        .await
        .expect("get tags");
    assert_eq!(tags.len(), 1);
    assert_eq!(tags[0].tag_slug.as_str(), "backup-test");
    assert_eq!(tags[0].tag_display, "Backup-Test");

    assert_eq!(
        std::fs::read_to_string(args.storage_path.join("media").join("avatar.txt"))
            .expect("read restored media"),
        "media"
    );
}

/// Assert a restore target is untouched — the fixture's operator user is absent —
/// after a rejected restore rolled back.
pub async fn assert_target_unmodified(args: &StorageArgs) {
    let state = open_existing_database(&args.db).await.expect("open target");
    let username: Username = "backupuser".parse().expect("valid username");
    assert!(
        state
            .users
            .get_user_by_username(&username)
            .await
            .expect("get user")
            .is_none(),
        "target must be unmodified after a rejected restore"
    );
}
