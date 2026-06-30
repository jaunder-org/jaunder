#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::too_many_lines,
    clippy::similar_names,
    clippy::items_after_statements,
    clippy::unused_async
)]

use chrono::Utc;
use common::password::Password;
use common::username::Username;
use common::visibility::AudienceTarget;
use jaunder::cli::StorageArgs;
use jaunder::commands::{cmd_backup, cmd_init, cmd_restore};
use storage::{open_existing_database, BackupMode, CreatePostInput, PostFormat};
use tempfile::TempDir;

use rstest::*;
#[allow(clippy::single_component_path_imports)]
use rstest_reuse;
use rstest_reuse::*;

use crate::helpers::{postgres_only, postgres_testing_enabled, unique_postgres_url, Backend};

fn sqlite_storage_args(base: &TempDir, name: &str) -> StorageArgs {
    StorageArgs {
        storage_path: base.path().join(format!("{name}-storage")),
        db: format!(
            "sqlite:{}",
            base.path().join(format!("{name}.db")).display()
        )
        .parse()
        .expect("sqlite db"),
    }
}

async fn postgres_storage_args(base: &TempDir, name: &str) -> StorageArgs {
    StorageArgs {
        storage_path: base.path().join(format!("{name}-storage")),
        db: unique_postgres_url().await,
    }
}

async fn populate_backup_fixture(args: &StorageArgs) -> i64 {
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
            published_at: Some(Utc::now()),
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

async fn assert_backup_fixture_restored(args: &StorageArgs, post_id: i64) {
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
    // `post_audiences` rows (see TABLES_IN_EXPORT_ORDER), so an Anonymous viewer
    // would be filtered out by the resolution predicate; the owner is always
    // admitted via the author branch, which is the correct viewer here.
    let local = state
        .subscriptions
        .local_channel_id()
        .await
        .expect("local channel id");
    let post = state
        .posts
        .get_post_by_id(
            post_id,
            &common::visibility::ViewerIdentity::local(user.user_id, local),
        )
        .await
        .expect("get post")
        .expect("restored post");
    assert_eq!(post.title.as_deref(), Some("Restored Post"));
    assert_eq!(post.slug.as_str(), "restored-post");

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

#[apply(postgres_only)]
// reason: cross-backend backup interop exercises BOTH engines in one test
// (SQLite source restored into Postgres target), so it needs a live Postgres.
#[tokio::test]
async fn sqlite_backup_restores_into_postgres(#[case] backend: Backend) {
    let _ = backend;
    if !postgres_testing_enabled() {
        return;
    }

    let base = TempDir::new().expect("temp dir");
    let source_args = sqlite_storage_args(&base, "sqlite-source");
    cmd_init(&source_args, false)
        .await
        .expect("init sqlite source");
    let post_id = populate_backup_fixture(&source_args).await;

    let backup_path = base.path().join("sqlite-backup");
    cmd_backup(
        &source_args,
        BackupMode::Directory,
        Some(backup_path.clone()),
    )
    .await
    .expect("sqlite backup");

    let target_args = postgres_storage_args(&base, "postgres-target").await;
    cmd_init(&target_args, false)
        .await
        .expect("init postgres target");
    cmd_restore(&target_args, &backup_path)
        .await
        .expect("restore into postgres");

    assert_backup_fixture_restored(&target_args, post_id).await;
}

#[apply(postgres_only)]
// reason: cross-backend backup interop exercises BOTH engines in one test
// (Postgres source restored into SQLite target), so it needs a live Postgres.
#[tokio::test]
async fn postgres_backup_restores_into_sqlite(#[case] backend: Backend) {
    let _ = backend;
    if !postgres_testing_enabled() {
        return;
    }

    let base = TempDir::new().expect("temp dir");
    let source_args = postgres_storage_args(&base, "postgres-source").await;
    cmd_init(&source_args, false)
        .await
        .expect("init postgres source");
    let post_id = populate_backup_fixture(&source_args).await;

    let backup_path = base.path().join("postgres-backup");
    cmd_backup(
        &source_args,
        BackupMode::Directory,
        Some(backup_path.clone()),
    )
    .await
    .expect("postgres backup");

    let target_args = sqlite_storage_args(&base, "sqlite-target");
    cmd_init(&target_args, false)
        .await
        .expect("init sqlite target");
    cmd_restore(&target_args, &backup_path)
        .await
        .expect("restore into sqlite");

    assert_backup_fixture_restored(&target_args, post_id).await;
}
