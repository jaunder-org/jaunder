use chrono::{DateTime, Utc};
use common::ids::UserId;
use common::password::Password;
use common::tag::TagLabel;
use common::test_support::{parse_audience_name, parse_display_name};
use common::username::Username;
use common::visibility::{AudienceTarget, ViewerIdentity};
use jaunder::cli::StorageArgs;
use storage::test_support::fp;
use storage::{
    open_existing_database, AppState, CreatePostInput, MediaRecord, MediaSource, PostFormat,
    RenderedHtml,
};

/// SHA-256 the media-table fixture row is keyed by; any stable value works, since
/// the media *files* are mirrored separately from the media *table*.
const FIXTURE_MEDIA_SHA256: &str =
    "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";

/// Identifiers returned by [`populate_backup_fixture`] so
/// [`assert_backup_fixture_restored`] can check non-author visibility fidelity
/// (a `Named`-audience post must survive restore visible to its subscriber, not
/// silently Private — the bug in issue #4).
pub struct BackupFixtureIds {
    /// The operator who authors the fixture posts and owns the audience.
    pub author: UserId,
    /// A second, non-operator account subscribed to the author and a member of
    /// the `Named` audience — the viewer who must still see the private post.
    pub viewer: UserId,
    /// A `Public` post (visible to everyone, including anonymous).
    pub public_post: i64,
    /// A post targeted at a `Named` audience the viewer belongs to.
    pub named_post: i64,
}

/// Fixed microsecond-precision publish time: deterministic and safe from
/// Postgres's µs quantization, so a restored value can be asserted exactly (DEC-D).
pub fn fixture_published_at() -> DateTime<Utc> {
    "2026-04-29T12:34:56.789012Z"
        .parse()
        .expect("valid fixture timestamp")
}

pub async fn populate_backup_fixture(args: &StorageArgs) -> BackupFixtureIds {
    let state = open_existing_database(&args.db)
        .await
        .expect("open database");
    let username: Username = "backupuser".parse().expect("valid username");
    let password: Password = "password123".parse().expect("valid password");
    let author = state
        .users
        .create_user(
            &username,
            &password,
            Some(&parse_display_name("Backup User")),
            true,
        )
        .await
        .expect("create user");
    let public_post = state
        .posts
        .create_post(&CreatePostInput {
            user_id: author,
            title: Some("Restored Post".into()),
            slug: "restored-post".parse().expect("valid slug"),
            body: "body text".into(),
            format: PostFormat::Markdown,
            rendered_html: RenderedHtml::from_trusted("<p>body text</p>"),
            published_at: Some(fixture_published_at()),
            summary: None,
            audiences: vec![AudienceTarget::Public],
            idempotency_key: None,
        })
        .await
        .expect("create post");
    state
        .posts
        .tag_post(public_post, &"Backup-Test".parse::<TagLabel>().unwrap())
        .await
        .expect("tag post");

    let (viewer, named_post) = seed_named_audience_post(&state, author, &password).await;
    seed_side_tables(&state, author).await;

    std::fs::write(args.storage_path.join("media").join("avatar.txt"), "media")
        .expect("write media");
    BackupFixtureIds {
        author,
        viewer,
        public_post,
        named_post,
    }
}

/// Seeds a non-author subscriber and a `Named`-audience post they belong to,
/// returning `(viewer_id, named_post_id)`. These visibility rows
/// (`subscriptions`, `audiences`, `audience_members`, `post_audiences`) must
/// survive restore so the subscriber still resolves the private post (issue #4).
async fn seed_named_audience_post(
    state: &AppState,
    author: UserId,
    password: &Password,
) -> (UserId, i64) {
    let viewer_name: Username = "viewer".parse().expect("valid username");
    let viewer = state
        .users
        .create_user(
            &viewer_name,
            password,
            Some(&parse_display_name("Viewer")),
            false,
        )
        .await
        .expect("create viewer");
    let local = state
        .subscriptions
        .local_channel_id()
        .await
        .expect("local channel");
    let subscription = state
        .subscriptions
        .subscribe(author, local, &i64::from(viewer).to_string())
        .await
        .expect("subscribe viewer");
    let audience = state
        .audiences
        .create_audience(author, &parse_audience_name("friends"))
        .await
        .expect("create audience");
    state
        .audiences
        .add_member(author, audience, subscription)
        .await
        .expect("add audience member");
    let named_post = state
        .posts
        .create_post(&CreatePostInput {
            user_id: author,
            title: Some("Friends Only".into()),
            slug: "friends-only".parse().expect("valid slug"),
            body: "secret body".into(),
            format: PostFormat::Markdown,
            rendered_html: RenderedHtml::from_trusted("<p>secret body</p>"),
            published_at: Some(fixture_published_at()),
            summary: None,
            audiences: vec![AudienceTarget::Named(audience)],
            idempotency_key: None,
        })
        .await
        .expect("create named post");
    (viewer, named_post)
}

/// Seeds the previously-unbacked side tables: a `user_config` row, a media-table
/// row, and a `feed_events` row.
async fn seed_side_tables(state: &AppState, author: UserId) {
    state
        .user_config
        .set(author, "editor.theme", "dark")
        .await
        .expect("set user config");
    state
        .media
        .create_media(&MediaRecord {
            user_id: author,
            sha256: FIXTURE_MEDIA_SHA256.to_owned(),
            filename: "photo.jpg".to_owned(),
            source: MediaSource::Upload,
            content_type: "image/jpeg".to_owned(),
            size_bytes: 4,
            source_url: None,
            created_at: fixture_published_at(),
        })
        .await
        .expect("create media row");
    state
        .feed_events
        .enqueue(&fp("/feed.rss"))
        .await
        .expect("enqueue feed event");
}

pub async fn assert_backup_fixture_restored(args: &StorageArgs, ids: &BackupFixtureIds) {
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

    let local = state
        .subscriptions
        .local_channel_id()
        .await
        .expect("local channel id");

    // The public post resolves for its author.
    let post = state
        .posts
        .get_post_by_id(ids.public_post, &ViewerIdentity::local(ids.author, local))
        .await
        .expect("get post")
        .expect("restored post");
    assert_eq!(post.title.as_deref(), Some("Restored Post"));
    assert_eq!(post.slug, "restored-post");
    // Value interop (DEC-D): the timestamp survives with its value.
    assert_eq!(post.published_at, Some(fixture_published_at()));

    let tags = state
        .posts
        .get_tags_for_post(ids.public_post)
        .await
        .expect("get tags");
    assert_eq!(tags.len(), 1);
    assert_eq!(tags[0].tag_slug, "backup-test");
    assert_eq!(tags[0].tag_display, "Backup-Test");

    // #4 closed: the Named-audience post survives restore visible to its
    // non-author subscriber — its post_audiences / subscriptions / audience_members
    // rows are carried — and correctly invisible to an anonymous viewer.
    assert!(
        state
            .posts
            .get_post_by_id(ids.named_post, &ViewerIdentity::local(ids.viewer, local))
            .await
            .expect("get named post")
            .is_some(),
        "restored Named-audience post must be visible to its subscriber"
    );
    assert!(
        state
            .posts
            .get_post_by_id(ids.named_post, &ViewerIdentity::Anonymous)
            .await
            .expect("get named post as anonymous")
            .is_none(),
        "a Named-audience post must not be visible to anonymous"
    );

    // The previously-unbacked tables survived the round trip.
    assert_eq!(
        state
            .user_config
            .get(ids.author, "editor.theme")
            .await
            .expect("get user config")
            .as_deref(),
        Some("dark")
    );
    assert!(
        state
            .media
            .get_media(
                ids.author,
                FIXTURE_MEDIA_SHA256,
                "photo.jpg",
                &MediaSource::Upload
            )
            .await
            .expect("get media")
            .is_some(),
        "restored media table row must be present"
    );

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
