//! Test-only tooling that reaches jaunder's storage layer from OUTSIDE the
//! server process — a live-server Playwright e2e drives the `test-support`
//! binary over a process boundary to seed fixtures. It is the cross-process
//! sibling of the in-process `storage::test_support` module and is never linked
//! into the `jaunder` production binary (see ADR-0046, `test-support-seed-binary`).
//!
//! The seed core builds inputs from the shared `storage::seed_post_input` recipe
//! and writes them in one batched transaction (`PostStorage::create_posts`),
//! rather than the `storage::test_support::seed_posts` module helper: the e2e
//! suite shares one database across all tests, so seeds need per-user-unique,
//! content-shaped slugs/bodies that the module helper's fixed `seed-{i}` /
//! `# Post {i}` scheme cannot give.

use std::{fmt::Write as _, fs::OpenOptions, io::Write as _, path::Path, sync::Arc};

use async_trait::async_trait;
use common::display_name::DisplayName;
use common::ids::{FeedEventId, PostId, UserId};
use common::media::{
    ByteSize, ContentHash, ContentType, Filename, MediaReference, MediaSource, detect_content_type,
    path as media_path,
};
use common::post_body::PostBody;
use common::post_title::PostTitle;
use common::root_relative_url::RootRelativeUrl;
use common::site::SiteTitle;
use common::slug::Slug;
use common::theme::{PublicThemeSelection, ThemeImageRole};
use common::time::UtcInstant;
use common::username::Username;
use common::visibility::AudienceTarget;
use host::config_key::SiteConfigKey;
use host::feed::{FeedEventPhase, FeedPath};
use jiff::{Timestamp, ToSpan};
use storage::{
    FeedEventStorage, ForeignEvidenceSink, InstanceId, LocalMediaSink, MediaManager, MediaRecord,
    MediaReferenceEvidence, MediaReferenceOwnershipResolver, MediaStorage, OperatorStatus,
    PersistedMediaReference, PostBookkeepingExpectation, PostFormat, PostStorage, PreparedPassword,
    ProvenLocalMediaRefs, RenderedPostContent, SessionStorage, SiteConfigStorage,
    ThemeAssetManager, ThemeOwner, ThemeRoleBinding, ThemeStorage, UserStorage, WriteScope,
    render_post_input, seed_post_input,
};

pub mod panic_gate;

/// The rendered-body source for seeded post `i` under `prefix`. Its Markdown H1
/// renders the text `"{prefix} {i}"`, which the heavy e2e timeline tests assert
/// on (first/last post title after pagination).
#[must_use]
pub fn seed_body(prefix: &str, i: usize) -> String {
    format!("# {prefix} {i}\n\nBody for {prefix} {i}")
}

/// A slug-valid, per-prefix-unique string for seeded post `i`: `prefix`
/// lowercased with every non-alphanumeric run collapsed to `-`, then the index
/// suffix. Because each heavy test registers a fresh user and the slug
/// uniqueness constraint is per-user, distinct prefixes keep every seed
/// invocation collision-free even against the shared e2e database.
#[must_use]
pub fn seed_slug(prefix: &str, i: usize) -> String {
    let mut base = String::with_capacity(prefix.len());
    let mut separator_pending = false;
    for character in prefix.chars() {
        if character.is_ascii_alphanumeric() {
            if separator_pending && !base.is_empty() {
                base.push('-');
            }
            base.push(character.to_ascii_lowercase());
            separator_pending = false;
        } else {
            separator_pending = true;
        }
    }
    format!("{base}-{i}")
}

/// Extracts the callback value only when its enclosing write received a commit
/// acknowledgement. Fixture writers cannot safely continue after an
/// acknowledgement loss because they need a definitive identifier or token.
fn confirmed_fixture_outcome<T>(
    outcome: common::MutationOutcome<T>,
    operation: impl std::fmt::Display,
) -> anyhow::Result<T> {
    match outcome {
        common::MutationOutcome::Confirmed(value) => Ok(value),
        common::MutationOutcome::CommitIndeterminate(_) => Err(anyhow::anyhow!(
            "{operation} commit acknowledgement was indeterminate"
        )),
    }
}

/// Seed `count` posts for `username` through the shared `seed_post_input`
/// recipe, written in one batched transaction — the same `create_post` write
/// path the server runs, so audience rows, rendered HTML, and both SQL dialects
/// come for free. `published` sets
/// `published_at = now` and a Public audience so the posts surface on the
/// timeline; otherwise they are drafts. Returns the created ids oldest-to-newest.
///
/// Slugs derive from `prefix` + index and the slug-uniqueness constraint is
/// per-user, so callers that share one database (the e2e suite) must pass a
/// distinct `prefix` for each user they seed — re-seeding the same user with the
/// same prefix would collide on the second invocation.
///
/// # Errors
///
/// Returns `Err` if `username` is invalid or unknown, a generated slug or body
/// fails to parse, or a post fails to persist.
pub async fn seed_posts_for_user(
    users: Arc<dyn UserStorage>,
    posts: Arc<dyn PostStorage>,
    write_scope: WriteScope,
    username: &str,
    count: usize,
    published: bool,
    prefix: &str,
) -> anyhow::Result<Vec<PostId>> {
    let uname = username
        .parse::<Username>()
        .map_err(|_| anyhow::anyhow!("invalid username: {username}"))?;
    let user = users
        .get_user_by_username(&uname)
        .await?
        .ok_or_else(|| anyhow::anyhow!("no such user: {username}"))?;

    let mut inputs = Vec::with_capacity(count);
    for i in 0..count {
        let slug = seed_slug(prefix, i).parse().map_err(|_| {
            anyhow::anyhow!("generated slug invalid for prefix {prefix:?} index {i}")
        })?;
        // Unlike the slug, whose validity depends on `prefix`, `seed_body` always emits a
        // literal `Body for …` line, so the non-blank invariant holds by construction.
        let Ok(body) = seed_body(prefix, i).parse() else {
            unreachable!("seed_body always yields a non-blank body");
        };
        inputs.push(seed_post_input(user.user_id, slug, body, published));
    }
    let outcome = write_scope
        .run(move |transaction| {
            Box::pin(async move { posts.create_posts(transaction, &inputs).await })
        })
        .await
        .map_err(|error| anyhow::anyhow!("batch seed of {count} posts failed: {error}"))?;

    confirmed_fixture_outcome(outcome, format_args!("batch seed of {count} posts"))
}
async fn author_fixture_theme(
    themes: Arc<dyn ThemeStorage>,
    write_scope: WriteScope,
    owner: ThemeOwner,
    compiled: &host::theme_package::CompiledThemeRevision,
) -> anyhow::Result<(common::ids::ThemeId, bool)> {
    let existing = themes
        .list_themes(owner)
        .await?
        .into_iter()
        .find(|entry| entry.name == "Test");
    let already_published = existing
        .as_ref()
        .and_then(|entry| entry.current_revision.as_ref())
        .is_some_and(|revision| {
            let mut compiled_revision = String::with_capacity(64);
            for byte in compiled.revision_digest() {
                let _ = write!(compiled_revision, "{byte:02x}");
            }
            revision.as_ref() == compiled_revision
        });
    let theme_id = match existing {
        Some(entry) => entry.id,
        None => {
            storage::seed_theme_fixture::try_create_theme(
                Arc::clone(&themes),
                write_scope.clone(),
                owner,
                compiled,
            )
            .await?
        }
    };
    Ok((theme_id, already_published))
}

/// Publish the shared compiled fixture as the author's selected custom theme.
///
/// This is deliberately an out-of-process e2e fixture: it uses the same
/// immutable asset publisher and transactional selection write as production,
/// against the exact storage root used by the live server.
///
/// # Errors
///
/// Returns `Err` if the fixture cannot be published, selected, or its commit
/// acknowledgement is indeterminate.
pub async fn seed_published_author_theme(
    users: Arc<dyn UserStorage>,
    themes: Arc<dyn ThemeStorage>,
    write_scope: WriteScope,
    storage_path: &Path,
    author_username: &str,
) -> anyhow::Result<()> {
    let author_username = author_username
        .parse::<Username>()
        .map_err(|_| anyhow::anyhow!("invalid username: {author_username}"))?;
    let author = users
        .get_user_by_username(&author_username)
        .await?
        .ok_or_else(|| anyhow::anyhow!("no such user: {author_username}"))?;
    let owner = ThemeOwner::Author(author.user_id);
    let compiled = storage::seed_theme_fixture::try_compiled_theme_fixture()?;
    let (theme_id, already_published) =
        author_fixture_theme(Arc::clone(&themes), write_scope.clone(), owner, &compiled).await?;
    let content_bytes = compiled
        .css()
        .bytes()
        .len()
        .checked_add(compiled.assets().map(|(_, _, bytes, _)| bytes.len()).sum())
        .and_then(|bytes| i64::try_from(bytes).ok())
        .ok_or_else(|| anyhow::anyhow!("fixture theme content exceeds quota size"))?;
    // E2E scenarios share one instance and may already have unrelated published
    // revisions. Keep the fixture's owner cap tight while avoiding a false
    // dependency on globally empty site quota state.
    let mut limits = storage::seed_theme_fixture::theme_quota_limits(content_bytes);
    limits.site_retained_revisions = i64::MAX;
    limits.site_physical_bytes = i64::MAX;
    let manager = ThemeAssetManager::new(
        Arc::clone(&themes),
        write_scope.clone(),
        Arc::new(storage_path.to_path_buf()),
    );
    if !already_published {
        let publication = manager
            .publish(
                owner,
                theme_id,
                &compiled,
                limits,
                Timestamp::now().as_second(),
            )
            .await?;
        confirmed_fixture_outcome(publication, "publish fixture author theme")?;
    }

    let bindings = [ThemeImageRole::Logo, ThemeImageRole::Header]
        .map(|role| ThemeRoleBinding::PackagedDefault { theme_id, role });
    let selection = write_scope
        .run(move |transaction| {
            Box::pin(async move {
                for binding in &bindings {
                    themes
                        .replace_role_binding(transaction, owner, binding)
                        .await?;
                }
                themes
                    .set_selection(
                        transaction,
                        owner,
                        Some(PublicThemeSelection::Custom(theme_id)),
                    )
                    .await
            })
        })
        .await
        .map_err(|error| anyhow::anyhow!("select fixture author theme failed: {error}"))?;
    confirmed_fixture_outcome(selection, "select fixture author theme")?;
    Ok(())
}
/// Restore the author selection changed by [`seed_published_author_theme`].
///
/// # Errors
///
/// Returns `Err` when the author is unknown or the selection cannot be reset.
pub async fn reset_author_theme_fixture(
    users: Arc<dyn UserStorage>,
    themes: Arc<dyn ThemeStorage>,
    write_scope: WriteScope,
    author_username: &str,
) -> anyhow::Result<()> {
    let author_username = author_username
        .parse::<Username>()
        .map_err(|_| anyhow::anyhow!("invalid username: {author_username}"))?;
    let author = users
        .get_user_by_username(&author_username)
        .await?
        .ok_or_else(|| anyhow::anyhow!("no such user: {author_username}"))?;
    let reset = write_scope
        .run(move |transaction| {
            Box::pin(async move {
                themes
                    .set_selection(transaction, ThemeOwner::Author(author.user_id), None)
                    .await
            })
        })
        .await
        .map_err(|error| anyhow::anyhow!("reset fixture theme selections failed: {error}"))?;
    confirmed_fixture_outcome(reset, "reset fixture theme selections")?;
    Ok(())
}

/// Seed terminal feed events through the same storage lifecycle used by the
/// worker. This is e2e-only fixture setup; it deliberately avoids queue SQL.
///
/// # Errors
///
/// Returns an error if a generated feed path or any storage transition fails.
pub async fn seed_dead_letters(
    feed_events: Arc<dyn FeedEventStorage>,
    write_scope: WriteScope,
    phase: FeedEventPhase,
    count: usize,
) -> anyhow::Result<Vec<FeedEventId>> {
    let mut ids = Vec::with_capacity(count);
    for index in 0..count {
        let feed_path = format!("/~websub-fixture-{index}/feed.rss")
            .parse::<FeedPath>()
            .map_err(|_| anyhow::anyhow!("generated WebSub fixture feed path was invalid"))?;
        let feed_events = Arc::clone(&feed_events);
        let diagnostic = format!("fixture {phase:?} failure {index}");
        let transaction_outcome = write_scope
            .run(move |transaction| {
                Box::pin(async move {
                    let id = feed_events.enqueue(transaction, &feed_path).await?;
                    match phase {
                        FeedEventPhase::Regeneration => {
                            feed_events
                                .dead_letter_regeneration(
                                    transaction,
                                    &[id],
                                    &diagnostic,
                                    common::time::UtcInstant::now(),
                                )
                                .await?;
                        }
                        FeedEventPhase::Publication => {
                            feed_events
                                .dead_letter_publication(
                                    transaction,
                                    &[id],
                                    &diagnostic,
                                    common::time::UtcInstant::now(),
                                )
                                .await?;
                        }
                    }
                    Ok::<FeedEventId, storage::FeedEventError>(id)
                })
            })
            .await
            .map_err(|error| {
                anyhow::anyhow!("atomic WebSub dead-letter fixture failed: {error}")
            })?;
        let id =
            confirmed_fixture_outcome(transaction_outcome, "atomic WebSub dead-letter fixture")?;
        ids.push(id);
    }
    Ok(ids)
}

/// Create a fixture user through the real `UserStorage::create_user` path — the
/// same call `jaunder user-create` makes (`server::commands::cmd_user_create`),
/// minus that command's `CliBypass` registration metric: this is out-of-process
/// test seeding and must not emit observability noise the e2e suite might assert
/// on. Assumes a freshly-initialised DB (no upsert). Returns the new user id.
///
/// # Errors
///
/// Returns `Err` if the username or password is invalid, or the user cannot be
/// created (e.g. a duplicate username).
pub async fn create_user(
    users: Arc<dyn UserStorage>,
    write_scope: WriteScope,
    username: &str,
    password: &str,
    display_name: Option<&DisplayName>,
    operator: bool,
) -> anyhow::Result<UserId> {
    let uname = username
        .parse::<Username>()
        .map_err(|_| anyhow::anyhow!("invalid username: {username}"))?;
    let pw = password
        .parse::<host::password::Password>()
        .map_err(|e| anyhow::anyhow!("invalid password: {e}"))?;
    let prepared = storage::prepare_password(pw)
        .await
        .map_err(|error| anyhow::anyhow!("fixture password preparation failed: {error}"))?;
    let display_name = display_name.cloned();
    let operator = if operator {
        OperatorStatus::OPERATOR
    } else {
        OperatorStatus::STANDARD
    };
    let outcome = write_scope
        .run(move |transaction| {
            Box::pin(async move {
                users
                    .create_user(
                        transaction,
                        &uname,
                        &prepared,
                        display_name.as_ref(),
                        operator,
                    )
                    .await
            })
        })
        .await
        .map_err(|error| anyhow::anyhow!("fixture user creation failed: {error}"))?;
    confirmed_fixture_outcome(outcome, "fixture user creation")
}

/// The two profile states accepted by the sandbox seed boundary.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SandboxProfile {
    /// Two loginable accounts and no Posts.
    Standard,
    /// The standard accounts plus authored Markdown and Org fixture Posts.
    Demo,
}

/// The canonical semantic visibility of one seeded Post.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SandboxVisibility {
    /// Visible to every reader.
    Public,
    /// Visible only to subscribers.
    Subscribers,
    /// Visible only to its author.
    Private,
}

impl SandboxVisibility {
    fn write_targets(self) -> Vec<AudienceTarget> {
        match self {
            Self::Public => vec![AudienceTarget::Public],
            Self::Subscribers => vec![AudienceTarget::Subscribers],
            Self::Private => vec![AudienceTarget::Private],
        }
    }

    #[cfg(test)]
    fn from_read_targets(targets: &[AudienceTarget]) -> Self {
        match targets {
            [AudienceTarget::Public] => Self::Public,
            [AudienceTarget::Subscribers] => Self::Subscribers,
            [] => Self::Private,
            _ => panic!("sandbox fixture has unsupported audience targets"),
        }
    }
}

/// One expected sandbox Post. The manifest is both the fixture definition and
/// the independent expected value used by profile tests.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SandboxPost {
    /// Canonical local author username.
    pub author: &'static str,
    /// Authored title.
    pub title: String,
    /// Per-author Post slug.
    pub slug: String,
    /// Native Markdown, Org, or HTML source.
    pub body: String,
    /// Source format used by the real renderer.
    pub format: PostFormat,
    /// Exact publication timestamp, or `None` for a draft.
    pub published_at: Option<UtcInstant>,
    /// Canonical semantic visibility, independent of physical audience-row encoding.
    pub visibility: SandboxVisibility,
}
/// Versioned, Rust-owned description of an entire sandbox seed. This is the
/// cross-process contract consumed by E2E callers; never make TypeScript repeat
/// one of its fixture constants.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SandboxSeedManifest {
    /// Wire schema version.
    pub version: u8,
    /// Seeded Posts and their read-only expectations.
    pub posts: Vec<SandboxPost>,
    /// Seeded Media, when the selected profile includes it.
    pub media: Option<SandboxMedia>,
}

/// One canonical Media object created with the sandbox profile.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SandboxMedia {
    /// Local owner username.
    pub author: &'static str,
    /// Original filename.
    pub filename: &'static str,
    /// SHA-256 digest of the exact bytes.
    pub sha256: &'static str,
    /// Public content-addressed URL path.
    pub content_url: String,
    /// Exact byte count.
    pub size_bytes: usize,
}

impl SandboxSeedManifest {
    /// Serializes the stable E2E wire contract. The storage types intentionally
    #[must_use]
    pub fn to_json(&self) -> serde_json::Value {
        serde_json::json!({
            "version": self.version,
            "posts": self.posts.iter().map(|post| serde_json::json!({
                "author": post.author,
                "title": post.title,
                "slug": post.slug,
                "body": post.body,
                "format": post.format.to_string(),
                "publishedAt": post.published_at.map(|at| at.to_string()),
                "visibility": match post.visibility {
                    SandboxVisibility::Public => "public",
                    SandboxVisibility::Subscribers => "subscribers",
                    SandboxVisibility::Private => "private",
                },
            })).collect::<Vec<_>>(),
            "media": self.media.as_ref().map(|media| serde_json::json!({
                "author": media.author,
                "filename": media.filename,
                "sha256": media.sha256,
                "contentUrl": media.content_url,
                "sizeBytes": media.size_bytes,
            })),
        })
    }
}

/// A deterministic, self-authored SVG uploaded for one sandbox User.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SandboxMediaAsset {
    /// Canonical upload filename.
    pub filename: &'static str,
    /// Exact SVG upload bytes.
    pub bytes: &'static [u8],
}

/// A curated native-source Post that refers only to its owner's Media asset.
#[derive(Clone, Copy, Debug)]
pub struct SandboxCuratedPostTemplate {
    title: &'static str,
    slug: &'static str,
    format: PostFormat,
    source: fn(&RootRelativeUrl) -> String,
}

impl SandboxCuratedPostTemplate {
    fn materialize_post(self, author: &'static str, asset_url: &RootRelativeUrl) -> SandboxPost {
        SandboxPost {
            author,
            title: self.title.to_owned(),
            slug: self.slug.to_owned(),
            body: (self.source)(asset_url),
            format: self.format,
            published_at: None,
            visibility: SandboxVisibility::Public,
        }
    }
}

/// The complete fixture owned by one sandbox User.
#[derive(Clone, Copy, Debug)]
pub struct SandboxUserFixture {
    /// Canonical local Username.
    pub username: &'static str,
    /// Whether this User is the sandbox operator.
    pub operator: bool,
    /// The User's one self-authored local SVG upload.
    pub media: SandboxMediaAsset,
    /// The User's curated Markdown Post.
    pub markdown: SandboxCuratedPostTemplate,
    /// The User's curated Org Post.
    pub org: SandboxCuratedPostTemplate,
}

impl SandboxUserFixture {
    /// Materializes this User's two curated Posts from only their sibling asset URL.
    #[must_use]
    pub fn materialize_curated_posts(self, asset_url: &RootRelativeUrl) -> [SandboxPost; 2] {
        [
            self.markdown.materialize_post(self.username, asset_url),
            self.org.materialize_post(self.username, asset_url),
        ]
    }
}

const SANDBOX_TITLE: &str = "Jaunder Sandbox";
const SANDBOX_PASSWORD: &str = "jaunder-dev";
const USER_SVG: &[u8] = br##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 96 64" role="img" aria-label="A blue horizon"><rect width="96" height="64" fill="#dbeafe"/><path d="M0 43h96v21H0z" fill="#2563eb"/><circle cx="70" cy="20" r="11" fill="#facc15"/></svg>"##;
const OPERATOR_SVG: &[u8] = br##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 96 64" role="img" aria-label="A warm workshop"><rect width="96" height="64" fill="#ffedd5"/><path d="M16 48 48 12l32 36z" fill="#ea580c"/><path d="M38 48V34h20v14" fill="#7c2d12"/></svg>"##;
const ALICE_SVG: &[u8] = br##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 96 64" role="img" aria-label="A green field"><rect width="96" height="64" fill="#dcfce7"/><path d="M0 38c18-16 34 10 52-5 15-13 28 3 44-8v39H0z" fill="#16a34a"/><path d="m24 36 9-17 9 17z" fill="#166534"/></svg>"##;
const BOB_SVG: &[u8] = br##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 96 64" role="img" aria-label="A violet night"><rect width="96" height="64" fill="#ede9fe"/><path d="M0 44h96v20H0z" fill="#7c3aed"/><path d="m18 35 12-18 12 18 12-12 12 12 12-18 12 18z" fill="#4c1d95"/></svg>"##;

fn user_markdown(asset_url: &RootRelativeUrl) -> String {
    format!(
        "# A horizon worth keeping\n\nA **small observation** can guide a whole day.\n\n[Read the field notes](/notes/horizon).\n\n- Watch the light\n- Keep the useful detail\n\n```text\nhorizon = \"clear\"\n```\n\n| Moment | Choice |\n| --- | --- |\n| Morning | Walk |\n| Evening | Write |\n\n![Blue horizon]({asset_url})"
    )
}

fn user_org(asset_url: &RootRelativeUrl) -> String {
    format!(
        "* A calm field note\n\nA /steady practice/ makes room for better work.\n\n[[/notes/practice][Read the practice note]]\n\n- Name the question\n- Share the answer\n\n#+begin_src text\nanswer = \"kind\"\n#+end_src\n\n| Moment | Choice |\n|---------+--------|\n| Morning | Listen |\n| Evening | Rest   |\n\n#+caption: Blue horizon\n[[{asset_url}]]"
    )
}

fn operator_markdown(asset_url: &RootRelativeUrl) -> String {
    format!(
        "# Workshop checks\n\nA **clear checklist** makes maintenance less surprising.\n\n[Review the runbook](/notes/workshop).\n\n1. Open the bench\n2. Record the result\n\n```text\nstatus = \"ready\"\n```\n\n| Tool | State |\n| --- | --- |\n| Saw | Ready |\n| Lamp | Warm |\n\n![Warm workshop]({asset_url})"
    )
}

fn operator_org(asset_url: &RootRelativeUrl) -> String {
    format!(
        "* Workshop rhythm\n\nA *shared routine* keeps the room useful.\n\n[[/notes/rhythm][Read the workshop rhythm]]\n\n1. Check the bench\n2. Leave a note\n\n#+begin_src text\nroom = \"open\"\n#+end_src\n\n| Tool | State |\n|------+-------|\n| Saw  | Ready |\n| Lamp | Warm  |\n\n#+caption: Warm workshop\n[[{asset_url}]]"
    )
}

fn alice_markdown(asset_url: &RootRelativeUrl) -> String {
    format!(
        "# Field paths\n\nA **patient route** notices what hurried travel misses.\n\n[See the path map](/notes/field-paths).\n\n- Follow the shade\n- Mark the turn\n\n```text\npace = \"slow\"\n```\n\n| Place | Sound |\n| --- | --- |\n| Gate | Birds |\n| Hill | Wind |\n\n![Green field]({asset_url})"
    )
}

fn alice_org(asset_url: &RootRelativeUrl) -> String {
    format!(
        "* Field margins\n\nA /careful walk/ gives a place time to speak.\n\n[[/notes/margins][Read the field margin]]\n\n- Follow the shade\n- Mark the turn\n\n#+begin_src text\npace = \"slow\"\n#+end_src\n\n| Place | Sound |\n|-------+-------|\n| Gate  | Birds |\n| Hill  | Wind  |\n\n#+caption: Green field\n[[{asset_url}]]"
    )
}

fn bob_markdown(asset_url: &RootRelativeUrl) -> String {
    format!(
        "# Night signals\n\nA **quiet sky** makes a distant signal easier to see.\n\n[Open the signal log](/notes/night-signals).\n\n1. Dim the lamp\n2. Wait for the blink\n\n```text\nsignal = \"seen\"\n```\n\n| Hour | Signal |\n| --- | --- |\n| Nine | Faint |\n| Ten | Clear |\n\n![Violet night]({asset_url})"
    )
}

fn bob_org(asset_url: &RootRelativeUrl) -> String {
    format!(
        "* Night watch\n\nA *quiet room* turns waiting into attention.\n\n[[/notes/night-watch][Read the night watch]]\n\n1. Dim the lamp\n2. Wait for the blink\n\n#+begin_src text\nsignal = \"seen\"\n#+end_src\n\n| Hour | Signal |\n|------+--------|\n| Nine | Faint  |\n| Ten  | Clear  |\n\n#+caption: Violet night\n[[{asset_url}]]"
    )
}

const SANDBOX_USER_FIXTURES: [SandboxUserFixture; 4] = [
    SandboxUserFixture {
        username: "user",
        operator: false,
        media: SandboxMediaAsset {
            filename: "blue-horizon.svg",
            bytes: USER_SVG,
        },
        markdown: SandboxCuratedPostTemplate {
            title: "A horizon worth keeping",
            slug: "horizon-worth-keeping",
            format: PostFormat::Markdown,
            source: user_markdown,
        },
        org: SandboxCuratedPostTemplate {
            title: "A calm field note",
            slug: "calm-field-note",
            format: PostFormat::Org,
            source: user_org,
        },
    },
    SandboxUserFixture {
        username: "operator",
        operator: true,
        media: SandboxMediaAsset {
            filename: "warm-workshop.svg",
            bytes: OPERATOR_SVG,
        },
        markdown: SandboxCuratedPostTemplate {
            title: "Workshop checks",
            slug: "workshop-checks",
            format: PostFormat::Markdown,
            source: operator_markdown,
        },
        org: SandboxCuratedPostTemplate {
            title: "Workshop rhythm",
            slug: "workshop-rhythm",
            format: PostFormat::Org,
            source: operator_org,
        },
    },
    SandboxUserFixture {
        username: "alice",
        operator: false,
        media: SandboxMediaAsset {
            filename: "green-field.svg",
            bytes: ALICE_SVG,
        },
        markdown: SandboxCuratedPostTemplate {
            title: "Field paths",
            slug: "field-paths",
            format: PostFormat::Markdown,
            source: alice_markdown,
        },
        org: SandboxCuratedPostTemplate {
            title: "Field margins",
            slug: "field-margins",
            format: PostFormat::Org,
            source: alice_org,
        },
    },
    SandboxUserFixture {
        username: "bob",
        operator: false,
        media: SandboxMediaAsset {
            filename: "violet-night.svg",
            bytes: BOB_SVG,
        },
        markdown: SandboxCuratedPostTemplate {
            title: "Night signals",
            slug: "night-signals",
            format: PostFormat::Markdown,
            source: bob_markdown,
        },
        org: SandboxCuratedPostTemplate {
            title: "Night watch",
            slug: "night-watch",
            format: PostFormat::Org,
            source: bob_org,
        },
    },
];
const SANDBOX_SEEDED_MEDIA_BYTES: &[u8] = b"baseline seeded media\n";
const SANDBOX_SEEDED_MEDIA_HASH: &str =
    "958e3ce706b2891ab01e58e0f180af8ef646dc9d6368f3a2ba6d6cbff3321c5e";
const SANDBOX_SEEDED_MEDIA_FILENAME: &str = "baseline-seeded.txt";
const SHORT_MARKDOWN: &str = "A short sandbox note with one clear idea.";
const MEDIUM_MARKDOWN: &str = "A medium sandbox note has enough detail to make a timeline card feel lived in.\n\nIt remains concise enough to scan.";
const SHORT_ORG: &str = "* A short Org sandbox note\n\nA clear idea in Org.";
const MEDIUM_ORG: &str = "* A medium Org sandbox note\n\nThis fixture has enough detail to exercise the rendered detail surface.\n\n- a stable item\n- another stable item";

/// Captures the profile creation instant at minute precision.
#[must_use]
pub fn sandbox_profile_anchor() -> UtcInstant {
    let now = UtcInstant::now().value();
    let minute = now.as_second().div_euclid(60) * 60;
    UtcInstant::from(Timestamp::from_second(minute).map_or(now, std::convert::identity))
}

fn sandbox_asset_urls() -> [RootRelativeUrl; 4] {
    [
        "/media/upload/81/aa/81aa7378e6c8ef6a707e281d5a92c646a1ba3c01de428131ecf677763a8cddd9/blue-horizon.svg",
        "/media/upload/92/c5/92c501114a2d5f3b82f50a4ac01ab2c1eeb1223cbc46e7840347adb3e6aa8847/warm-workshop.svg",
        "/media/upload/bb/2c/bb2cd32aaa8d6b4bd87af8980a5bb220753bdfafdd953bc4c9b4a861d1e4c233/green-field.svg",
        "/media/upload/2e/8d/2e8d0525784fb6a8b04b82131e9d82cddca52445e21c1faa0baccbc7ad44290c/violet-night.svg",
    ]
    .map(|url| url.parse().expect("fixed sandbox Media URL"))
}

/// Produces the complete versioned typed fixture manifest for a profile creation
/// anchor. The historical corpus retains globally distinct timestamps; the
/// canonical extension adds explicit HTML, scheduling, non-public, and
/// Media-reference records without backend-specific identities.
#[must_use]
pub fn sandbox_profile_manifest(anchor: UtcInstant) -> SandboxSeedManifest {
    let asset_urls = sandbox_asset_urls();
    let mut posts = Vec::with_capacity(73);
    let mut first_offset = 1_i64;
    for (fixture, asset_url) in SANDBOX_USER_FIXTURES.iter().zip(&asset_urls) {
        let curated = fixture.materialize_curated_posts(asset_url);
        for sequence in 1..=15_i64 {
            let (title, slug, body, format) = match sequence {
                1 => (
                    curated[0].title.clone(),
                    curated[0].slug.clone(),
                    curated[0].body.clone(),
                    curated[0].format,
                ),
                2..=12 if sequence % 2 == 0 => (
                    format!("{} sandbox post {sequence:02}", fixture.username),
                    format!("sandbox-post-{sequence:02}"),
                    SHORT_MARKDOWN.to_owned(),
                    PostFormat::Markdown,
                ),
                2..=12 => (
                    format!("{} sandbox post {sequence:02}", fixture.username),
                    format!("sandbox-post-{sequence:02}"),
                    MEDIUM_MARKDOWN.to_owned(),
                    PostFormat::Markdown,
                ),
                13 => (
                    curated[1].title.clone(),
                    curated[1].slug.clone(),
                    curated[1].body.clone(),
                    curated[1].format,
                ),
                14 => (
                    format!("{} sandbox post {sequence:02}", fixture.username),
                    format!("sandbox-post-{sequence:02}"),
                    MEDIUM_ORG.to_owned(),
                    PostFormat::Org,
                ),
                15 => (
                    format!("{} sandbox post {sequence:02}", fixture.username),
                    format!("sandbox-post-{sequence:02}"),
                    SHORT_ORG.to_owned(),
                    PostFormat::Org,
                ),
                _ => unreachable!("published sequence is bounded to 1..=15"),
            };
            posts.push(SandboxPost {
                author: fixture.username,
                title,
                slug,
                body,
                format,
                published_at: Some(UtcInstant::from(
                    anchor
                        .value()
                        .saturating_sub(((first_offset + sequence - 1) * 24).hours())
                        .map_or(Timestamp::MIN, std::convert::identity),
                )),
                visibility: SandboxVisibility::Public,
            });
        }
        first_offset += 15;
        posts.push(SandboxPost {
            author: fixture.username,
            title: format!("{} sandbox Markdown draft", fixture.username),
            slug: "sandbox-markdown-draft".to_owned(),
            body: SHORT_MARKDOWN.to_owned(),
            format: PostFormat::Markdown,
            published_at: None,
            visibility: SandboxVisibility::Public,
        });
        posts.push(SandboxPost {
            author: fixture.username,
            title: format!("{} sandbox Org draft", fixture.username),
            slug: "sandbox-org-draft".to_owned(),
            body: SHORT_ORG.to_owned(),
            format: PostFormat::Org,
            published_at: None,
            visibility: SandboxVisibility::Public,
        });
    }
    posts.extend(sandbox_extension(anchor));
    SandboxSeedManifest {
        version: 1,
        posts,
        media: Some(SandboxMedia {
            author: "alice",
            filename: SANDBOX_SEEDED_MEDIA_FILENAME,
            sha256: SANDBOX_SEEDED_MEDIA_HASH,
            content_url: format!(
                "/media/upload/95/8e/{SANDBOX_SEEDED_MEDIA_HASH}/{SANDBOX_SEEDED_MEDIA_FILENAME}"
            ),
            size_bytes: SANDBOX_SEEDED_MEDIA_BYTES.len(),
        }),
    }
}

fn sandbox_extension(anchor: UtcInstant) -> [SandboxPost; 5] {
    [
        SandboxPost {
            author: "alice",
            title: "Alice canonical HTML".to_owned(),
            slug: "baseline-seeded-html".to_owned(),
            body: "<p>Canonical seeded <strong>HTML</strong> body.</p>".to_owned(),
            format: PostFormat::Html,
            published_at: Some(anchor),
            visibility: SandboxVisibility::Public,
        },
        SandboxPost {
            author: "alice",
            title: "Alice scheduled Markdown".to_owned(),
            slug: "baseline-seeded-scheduled".to_owned(),
            body: "# Canonical scheduled Markdown\n\nThis post remains scheduled.".to_owned(),
            format: PostFormat::Markdown,
            published_at: Some(UtcInstant::from(
                anchor
                    .value()
                    .saturating_add(168.hours())
                    .map_or(Timestamp::MAX, std::convert::identity),
            )),
            visibility: SandboxVisibility::Public,
        },
        SandboxPost {
            author: "alice",
            title: "Alice subscribers Org".to_owned(),
            slug: "baseline-seeded-subscribers".to_owned(),
            body: "* Canonical subscribers Org\n\nThis post is not public.".to_owned(),
            format: PostFormat::Org,
            published_at: Some(anchor),
            visibility: SandboxVisibility::Subscribers,
        },
        SandboxPost {
            author: "alice",
            title: "Alice private HTML".to_owned(),
            slug: "baseline-seeded-private".to_owned(),
            body: "<p>Canonical private HTML body.</p>".to_owned(),
            format: PostFormat::Html,
            published_at: Some(anchor),
            visibility: SandboxVisibility::Private,
        },
        SandboxPost {
            author: "alice",
            title: "Alice media reference".to_owned(),
            slug: "baseline-seeded-media-reference".to_owned(),
            body: "![Canonical seeded Media](/media/upload/95/8e/958e3ce706b2891ab01e58e0f180af8ef646dc9d6368f3a2ba6d6cbff3321c5e/baseline-seeded.txt)".to_owned(),
            format: PostFormat::Markdown,
            published_at: Some(anchor),
            visibility: SandboxVisibility::Public,
        },
    ]
}

/// Sandbox-local proof policy: root-relative stored-Media references are
/// intrinsically local; every network-dependent form remains unproved.
///
/// The seed process never performs network I/O and does not need deletion
/// evidence. Keeping unknown forms absent makes this resolver fail closed.
pub struct SandboxMediaOwnershipResolver;

// cov:ignore-start: sandbox seeding never resolves deletion ownership
#[async_trait]
impl MediaReferenceOwnershipResolver for SandboxMediaOwnershipResolver {
    async fn resolve(
        &self,
        _references: &[PersistedMediaReference],
        _instance_id: &InstanceId,
        _base_url: Option<&common::tagged_url::BaseUrl>,
        foreign: ForeignEvidenceSink,
    ) -> MediaReferenceEvidence {
        foreign.finish()
    }

    async fn resolve_local(
        &self,
        _references: &[MediaReference],
        _instance_id: &InstanceId,
        _base_url: Option<&common::tagged_url::BaseUrl>,
        local: LocalMediaSink,
    ) -> ProvenLocalMediaRefs {
        local.finish()
    }
}
// cov:ignore-stop

fn sandbox_post_content(
    fixture: &SandboxPost,
    user_id: UserId,
) -> anyhow::Result<RenderedPostContent> {
    let title = fixture
        .title
        .as_str()
        .parse::<PostTitle>()
        .map_err(|error| anyhow::anyhow!("invalid sandbox Post title: {error}"))?;
    let slug = fixture
        .slug
        .as_str()
        .parse::<Slug>()
        .map_err(|error| anyhow::anyhow!("invalid sandbox Post slug: {error}"))?;
    let body = fixture
        .body
        .as_str()
        .parse::<PostBody>()
        .map_err(|error| anyhow::anyhow!("invalid sandbox Post body: {error}"))?;
    Ok(RenderedPostContent {
        user_id,
        title: Some(title),
        slug,
        audiences: fixture.visibility.write_targets(),
        body,
        format: fixture.format,
        published_at: fixture.published_at,
        summary: None,
        tags: Vec::new(),
        idempotency_key: None,
        expectations: PostBookkeepingExpectation::default(),
    })
}

/// Seeds the fixed standard sandbox profile through its exact storage services.
///
/// Publishes fixed fixture bytes through the production content-address layout.
/// The temporary file is durable before an atomic hard-link claim, so a failed
/// seed never exposes a partial canonical object. Existing content must exactly
/// match the immutable fixture rather than silently being trusted.
fn publish_seeded_media(target: &Path) -> anyhow::Result<bool> {
    let parent = target
        .parent()
        .ok_or_else(|| anyhow::anyhow!("canonical Media path has no parent"))?;
    std::fs::create_dir_all(parent)
        .map_err(|error| anyhow::anyhow!("creating seeded Media directory failed: {error}"))?;
    match std::fs::read(target) {
        Ok(bytes) => {
            anyhow::ensure!(
                bytes == SANDBOX_SEEDED_MEDIA_BYTES,
                "existing seeded Media bytes do not match the canonical fixture"
            );
            return Ok(false);
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(error) => {
            return Err(anyhow::anyhow!(
                "reading seeded Media content failed: {error}"
            ));
        }
    }
    let temporary = parent.join(format!(".seeded-media-{}.tmp", std::process::id()));
    let write_result = (|| {
        let mut file = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&temporary)?;
        file.write_all(SANDBOX_SEEDED_MEDIA_BYTES)?;
        file.sync_all()?;
        drop(file);
        std::fs::hard_link(&temporary, target)?;
        std::fs::remove_file(&temporary)?;
        std::fs::File::open(parent)?.sync_all()?;
        Ok::<_, std::io::Error>(true)
    })();
    match write_result {
        Ok(created) => Ok(created),
        Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {
            let _ = std::fs::remove_file(&temporary);
            let bytes = std::fs::read(target).map_err(|read_error| {
                anyhow::anyhow!("reading concurrent seeded Media content failed: {read_error}")
            })?;
            anyhow::ensure!(
                bytes == SANDBOX_SEEDED_MEDIA_BYTES,
                "concurrent seeded Media bytes do not match the canonical fixture"
            );
            Ok(false)
        }
        Err(error) => {
            let _ = std::fs::remove_file(&temporary);
            Err(anyhow::anyhow!(
                "publishing seeded Media content failed: {error}"
            ))
        }
    }
}

/// Storage dependencies for one atomic sandbox profile seed.
pub struct SandboxSeedStorage {
    pub site_config: Arc<dyn SiteConfigStorage>,
    pub users: Arc<dyn UserStorage>,
    pub posts: Arc<dyn PostStorage>,
    pub media: Arc<dyn MediaStorage>,
    pub write_scope: WriteScope,
}

struct PreparedSandboxUser {
    username_text: &'static str,
    username: Username,
    operator: bool,
    password: PreparedPassword,
}

struct PreparedSeededMedia {
    filename: Filename,
    sha256: ContentHash,
    size_bytes: ByteSize,
    path: std::path::PathBuf,
}

fn profile_manifest(profile: SandboxProfile, anchor: UtcInstant) -> SandboxSeedManifest {
    match profile {
        SandboxProfile::Standard => SandboxSeedManifest {
            version: 1,
            posts: Vec::new(),
            media: None,
        },
        SandboxProfile::Demo => sandbox_profile_manifest(anchor),
    }
}

fn profile_users(profile: SandboxProfile) -> &'static [SandboxUserFixture] {
    match profile {
        SandboxProfile::Standard => &SANDBOX_USER_FIXTURES[..2],
        SandboxProfile::Demo => &SANDBOX_USER_FIXTURES,
    }
}

async fn prepare_sandbox_users(
    profile: SandboxProfile,
) -> anyhow::Result<Vec<PreparedSandboxUser>> {
    let password = SANDBOX_PASSWORD
        .parse::<host::password::Password>()
        .map_err(|error| anyhow::anyhow!("invalid fixed sandbox password: {error}"))?;
    let mut prepared = Vec::with_capacity(profile_users(profile).len());
    for fixture in profile_users(profile) {
        let username = fixture
            .username
            .parse::<Username>()
            .map_err(|error| anyhow::anyhow!("invalid fixed sandbox username: {error}"))?;
        let password = storage::prepare_password(password.clone())
            .await
            .map_err(|error| anyhow::anyhow!("sandbox password preparation failed: {error}"))?;
        prepared.push(PreparedSandboxUser {
            username_text: fixture.username,
            username,
            operator: fixture.operator,
            password,
        });
    }
    Ok(prepared)
}

fn prepare_seeded_media(
    profile: SandboxProfile,
    storage_path: &Path,
) -> anyhow::Result<Option<PreparedSeededMedia>> {
    if profile == SandboxProfile::Standard {
        return Ok(None);
    }
    let filename = Filename::sanitized(SANDBOX_SEEDED_MEDIA_FILENAME)
        .map_err(|error| anyhow::anyhow!("invalid fixed Media filename: {error}"))?;
    let sha256 = SANDBOX_SEEDED_MEDIA_HASH
        .parse::<ContentHash>()
        .map_err(|error| anyhow::anyhow!("invalid fixed Media hash: {error}"))?;
    let size_bytes = SANDBOX_SEEDED_MEDIA_BYTES
        .len()
        .to_string()
        .parse::<ByteSize>()
        .map_err(|error| anyhow::anyhow!("invalid fixed Media size: {error}"))?;
    let path =
        storage_path
            .join("media")
            .join(media_path(&MediaSource::Upload, &sha256, &filename));
    Ok(Some(PreparedSeededMedia {
        filename,
        sha256,
        size_bytes,
        path,
    }))
}

fn cleanup_newly_published_media(created: bool, path: Option<&Path>) {
    if let (true, Some(path)) = (created, path) {
        let _ = std::fs::remove_file(path);
    }
}

enum SandboxProfileWriteError {
    Failed(anyhow::Error),
    CommitIndeterminate,
}

async fn write_sandbox_profile(
    storage: SandboxSeedStorage,
    title: String,
    prepared_users: Vec<PreparedSandboxUser>,
    seeded_media: Option<PreparedSeededMedia>,
    manifest: SandboxSeedManifest,
    anchor: UtcInstant,
) -> Result<(), SandboxProfileWriteError> {
    let SandboxSeedStorage {
        site_config,
        users,
        posts,
        media,
        write_scope,
    } = storage;
    let outcome = write_scope
        .run(move |transaction| {
            Box::pin(async move {
                site_config
                    .set(transaction, SiteConfigKey::SiteTitle, &title)
                    .await?;
                let mut user_ids = Vec::with_capacity(prepared_users.len());
                for prepared in prepared_users {
                    let role = if prepared.operator {
                        OperatorStatus::OPERATOR
                    } else {
                        OperatorStatus::STANDARD
                    };
                    let user_id = users
                        .create_user(
                            transaction,
                            &prepared.username,
                            &prepared.password,
                            None,
                            role,
                        )
                        .await?;
                    user_ids.push((prepared.username_text, user_id));
                }
                if let Some(seeded) = seeded_media {
                    let user_id = user_ids
                        .iter()
                        .find_map(|(username, id)| (*username == "alice").then_some(*id))
                        .ok_or_else(|| anyhow::anyhow!("seeded Media owner is missing"))?;
                    let record = MediaRecord {
                        user_id,
                        sha256: seeded.sha256,
                        content_type: detect_content_type(&seeded.filename),
                        filename: seeded.filename,
                        source: MediaSource::Upload,
                        size_bytes: seeded.size_bytes,
                        source_url: None,
                        created_at: anchor,
                    };
                    media.create_media(transaction, &record).await?;
                }
                let mut inputs = Vec::with_capacity(manifest.posts.len());
                for fixture in manifest.posts {
                    let user_id = user_ids
                        .iter()
                        .find_map(|(username, id)| (*username == fixture.author).then_some(*id))
                        .ok_or_else(|| {
                            anyhow::anyhow!(
                                "sandbox manifest author {} is not seeded",
                                fixture.author
                            )
                        })?;
                    inputs.push(render_post_input(sandbox_post_content(&fixture, user_id)?));
                }
                let post_outcome = posts.create_posts(transaction, &inputs).await?;
                Ok::<_, anyhow::Error>(post_outcome)
            })
        })
        .await
        .map_err(|error| {
            SandboxProfileWriteError::Failed(anyhow::anyhow!(
                "sandbox profile write failed: {error}"
            ))
        })?;
    match outcome {
        common::MutationOutcome::Confirmed(_) => Ok(()),
        common::MutationOutcome::CommitIndeterminate(_) => {
            Err(SandboxProfileWriteError::CommitIndeterminate)
        }
    }
}

/// Seeds the exact non-idempotent sandbox profile through the normal typed
/// storage write services. All profile rows share one write scope, so a failed
/// creation cannot leave a workspace with a partial fixture.
///
/// # Errors
///
/// the single profile transaction fails. Newly published Media is removed only
/// after a confirmed rollback; an indeterminate commit retains it because the
/// database may already reference the canonical content.
pub async fn seed_sandbox_profile(
    storage: SandboxSeedStorage,
    storage_path: &Path,
    profile: SandboxProfile,
    anchor: UtcInstant,
) -> anyhow::Result<SandboxSeedManifest> {
    let manifest = profile_manifest(profile, anchor);
    let prepared_users = prepare_sandbox_users(profile).await?;
    let seeded_media = prepare_seeded_media(profile, storage_path)?;
    let seeded_media_path = seeded_media.as_ref().map(|media| media.path.clone());
    let created_media = seeded_media
        .as_ref()
        .map_or(Ok(false), |media| publish_seeded_media(&media.path))?;
    let title = SANDBOX_TITLE
        .parse::<SiteTitle>()
        .map_err(|error| anyhow::anyhow!("invalid fixed sandbox title: {error}"))?
        .to_string();
    match write_sandbox_profile(
        storage,
        title,
        prepared_users,
        seeded_media,
        manifest.clone(),
        anchor,
    )
    .await
    {
        Ok(()) => Ok(manifest),
        Err(SandboxProfileWriteError::Failed(error)) => {
            cleanup_newly_published_media(created_media, seeded_media_path.as_deref());
            Err(error)
        }
        Err(SandboxProfileWriteError::CommitIndeterminate) => Err(anyhow::anyhow!(
            "sandbox profile write commit acknowledgement was indeterminate; retained seeded Media for revalidation"
        )),
    }
}

/// Reset the mail-capture file: delete `path` if it exists. A missing file is
/// success (`rm -f` semantics); any other error propagates. The one fixture
/// step that is not storage-linked.
///
/// # Errors
///
/// Returns `Err` if `path` exists but cannot be removed.
pub fn reset_mail(path: &std::path::Path) -> anyhow::Result<()> {
    match std::fs::remove_file(path) {
        Ok(()) => Ok(()),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(e) => Err(anyhow::anyhow!("reset-mail: {}: {e}", path.display())),
    }
}

/// The JSON seed record printed by the `seed-user` / `create-session`
/// subcommands: everything a browser context needs to boot authenticated
/// pre-paint — the session cookie and the advisory marker, both built by the
/// server's own primitives (`host::auth::session_cookie_header`,
/// `common::session_user`) so TypeScript never restates them (#791).
#[derive(Debug, Clone, serde::Serialize)]
pub struct SeedRecord {
    pub username: String,
    pub user_id: i64,
    pub is_operator: bool,
    pub token: String,
    pub set_cookie: String,
    pub marker_key: String,
    pub marker: String,
}

/// The default session label for seeded sessions — distinct, so they are
/// obvious on `/sessions` and in debugging.
const DEFAULT_SEED_LABEL: &str = "E2E seed";

/// Build the record for one fresh session: mint the token through the real
/// `SessionStorage` path and derive both client-visible artifacts from the
/// server's own primitives.
async fn session_record(
    sessions: Arc<dyn SessionStorage>,
    write_scope: WriteScope,
    user_id: UserId,
    username: &Username,
    is_operator: bool,
    label: Option<&str>,
) -> anyhow::Result<SeedRecord> {
    let label = label
        .unwrap_or(DEFAULT_SEED_LABEL)
        .parse::<common::session_label::SessionLabel>()
        .map_err(|e| anyhow::anyhow!("invalid session label: {e}"))?;
    let outcome = write_scope
        .run(move |transaction| {
            let sessions = Arc::clone(&sessions);
            Box::pin(async move { sessions.create_session(transaction, user_id, &label).await })
        })
        .await
        .map_err(|error| anyhow::anyhow!("fixture session creation failed: {error}"))?;
    let token = confirmed_fixture_outcome(outcome, "fixture session creation")?;
    Ok(SeedRecord {
        username: username.to_string(),
        user_id: i64::from(user_id),
        is_operator,
        set_cookie: host::auth::session_cookie_header(&token, false),
        token: token.to_string(),
        marker_key: common::local_storage_key::LocalStorageKey::AuthMarker
            .as_ref()
            .to_owned(),
        marker: common::session_user::encode_marker(&common::session_user::SessionUser {
            username: username.clone(),
            is_operator,
        }),
    })
}

/// Create a fixture user (real `UserStorage::create_user` path — genuinely
/// argon2-hashed, so the account stays loginable through the UI) and a session
/// in one DB open. `label` defaults to `"E2E seed"`.
///
/// # Errors
///
/// Returns `Err` if the username or password is invalid, the label is invalid,
/// or the user cannot be created (e.g. a duplicate username).
pub async fn seed_user(
    users: Arc<dyn UserStorage>,
    sessions: Arc<dyn SessionStorage>,
    write_scope: WriteScope,
    username: &str,
    password: &str,
    label: Option<&str>,
) -> anyhow::Result<SeedRecord> {
    let user_id = create_user(
        Arc::clone(&users),
        write_scope.clone(),
        username,
        password,
        None,
        false,
    )
    .await?;
    let uname = username
        .parse::<Username>()
        .map_err(|_| anyhow::anyhow!("invalid username: {username}"))?;
    session_record(sessions, write_scope, user_id, &uname, false, label).await
}

/// Create a session for an EXISTING user (e.g. the harness-seeded
/// `testoperator`); `is_operator` is read back from the user record so the
/// marker matches what a real login would write. `label` defaults to
/// `"E2E seed"`.
///
/// # Errors
///
pub async fn create_session_for_user(
    users: Arc<dyn UserStorage>,
    sessions: Arc<dyn SessionStorage>,
    write_scope: WriteScope,
    username: &str,
    label: Option<&str>,
) -> anyhow::Result<SeedRecord> {
    let uname = username
        .parse::<Username>()
        .map_err(|_| anyhow::anyhow!("invalid username: {username}"))?;
    let user = users
        .get_user_by_username(&uname)
        .await?
        .ok_or_else(|| anyhow::anyhow!("no such user: {username}"))?;
    session_record(
        sessions,
        write_scope,
        user.user_id,
        &uname,
        user.is_operator.is_operator(),
        label,
    )
    .await
}

#[cfg(test)]
mod sandbox_profile_tests {
    use super::*;
    use common::media::MediaRef;
    use rstest_reuse::apply;
    use storage::test_support::{Backend, backends};

    async fn assert_sandbox_users(users: Arc<dyn UserStorage>, expected: &[SandboxUserFixture]) {
        for fixture in expected {
            let username = fixture
                .username
                .parse::<Username>()
                .expect("fixed username");
            let user = users
                .get_user_by_username(&username)
                .await
                .expect("user lookup")
                .expect("fixture user exists");
            assert_eq!(
                user.is_operator,
                if fixture.operator {
                    OperatorStatus::OPERATOR
                } else {
                    OperatorStatus::STANDARD
                }
            );
        }
    }

    #[apply(backends)]
    #[tokio::test]
    async fn standard_profile_has_only_its_explicit_configuration_and_loginable_users(
        #[case] backend: Backend,
    ) {
        let env = backend.setup().pristine().await;
        let site_config = env.site_config();
        let users = env.users();
        let posts = env.posts();
        seed_sandbox_profile(
            SandboxSeedStorage {
                site_config: Arc::clone(&site_config),
                users: Arc::clone(&users),
                posts: Arc::clone(&posts),
                media: env.media(),
                write_scope: env.write_scope(),
            },
            env.base.path(),
            SandboxProfile::Standard,
            "2026-09-06T12:34:00Z".parse().expect("fixed anchor"),
        )
        .await
        .expect("standard profile seeds");
        assert_eq!(
            site_config.list().await.expect("site config list"),
            vec![("site.title".to_owned(), SANDBOX_TITLE.to_owned())]
        );
        assert_sandbox_users(users, &SANDBOX_USER_FIXTURES[..2]).await;
    }

    #[apply(backends)]
    #[tokio::test]
    async fn demo_profile_matches_the_typed_manifest_and_seeded_media(#[case] backend: Backend) {
        let env = backend.setup().pristine().await;
        let users = env.users();
        let posts = env.posts();
        let media = env.media();
        let anchor = "2026-09-06T12:34:00Z".parse().expect("fixed minute anchor");
        let expected = sandbox_profile_manifest(anchor);
        let actual = seed_sandbox_profile(
            SandboxSeedStorage {
                site_config: env.site_config(),
                users: Arc::clone(&users),
                posts: Arc::clone(&posts),
                media: Arc::clone(&media),
                write_scope: env.write_scope(),
            },
            env.base.path(),
            SandboxProfile::Demo,
            anchor,
        )
        .await
        .expect("demo profile seeds");
        assert_eq!(actual, expected);
        assert_sandbox_users(users, &SANDBOX_USER_FIXTURES).await;

        let seeded_media = MediaRef {
            source: MediaSource::Upload,
            sha256: SANDBOX_SEEDED_MEDIA_HASH.parse().expect("fixed Media hash"),
            filename: Filename::sanitized(SANDBOX_SEEDED_MEDIA_FILENAME)
                .expect("fixed Media filename"),
        };
        let alice = users
            .get_user_by_username(&"alice".parse().expect("fixed username"))
            .await
            .expect("alice lookup")
            .expect("alice exists");
        assert!(
            media
                .get_media(
                    alice.user_id,
                    &seeded_media.sha256,
                    &seeded_media.filename,
                    &seeded_media.source,
                )
                .await
                .expect("seeded Media lookup")
                .is_some()
        );
        assert_eq!(
            posts
                .list_media_references(&seeded_media)
                .await
                .expect("seeded Media references")
                .references()
                .len(),
            1
        );
    }

    #[apply(backends)]
    #[tokio::test]
    async fn failed_profile_write_removes_newly_published_seeded_media(#[case] backend: Backend) {
        let env = backend.setup().pristine().await;
        let users = env.users();
        let filename =
            Filename::sanitized(SANDBOX_SEEDED_MEDIA_FILENAME).expect("fixed Media filename");
        let sha256 = SANDBOX_SEEDED_MEDIA_HASH
            .parse::<ContentHash>()
            .expect("fixed Media hash");
        let target = env.base.path().join("media").join(media_path(
            &MediaSource::Upload,
            &sha256,
            &filename,
        ));
        create_user(
            Arc::clone(&users),
            env.write_scope(),
            "alice",
            SANDBOX_PASSWORD,
            None,
            false,
        )
        .await
        .expect("duplicate fixture user setup");

        seed_sandbox_profile(
            SandboxSeedStorage {
                site_config: env.site_config(),
                users,
                posts: env.posts(),
                media: env.media(),
                write_scope: env.write_scope(),
            },
            env.base.path(),
            SandboxProfile::Demo,
            "2026-09-06T12:34:00Z".parse().expect("fixed anchor"),
        )
        .await
        .expect_err("duplicate user makes profile write fail");
        assert!(!target.exists());
    }
}

#[cfg(test)]
mod content_tests {
    use super::*;

    #[test]
    fn seed_body_renders_prefix_and_index() {
        assert_eq!(
            seed_body("Timeline Post", 50),
            "# Timeline Post 50\n\nBody for Timeline Post 50"
        );
    }

    #[test]
    fn seed_slug_is_slug_safe_and_collapses_separator_runs() {
        assert_eq!(seed_slug("Timeline Post", 0), "timeline-post-0");
        assert_eq!(seed_slug("Home Feed Mine", 12), "home-feed-mine-12");
        assert_eq!(
            seed_slug("  Timeline__Post — Mine!  ", 7),
            "timeline-post-mine-7"
        );
    }

    #[test]
    fn sandbox_post_content_rejects_invalid_fixed_fixture_fields() {
        let anchor = "2026-09-06T12:34:00Z"
            .parse::<UtcInstant>()
            .expect("fixed anchor");
        let mut fixture = sandbox_profile_manifest(anchor)
            .posts
            .pop()
            .expect("manifest contains posts");
        fixture.title.clear();
        let title_error = sandbox_post_content(&fixture, UserId::from(1))
            .err()
            .expect("blank fixture title is invalid");
        assert!(title_error.to_string().contains("sandbox Post title"));

        fixture.title = "valid title".to_owned();
        fixture.slug.clear();
        let slug_error = sandbox_post_content(&fixture, UserId::from(1))
            .err()
            .expect("blank fixture slug is invalid");
        assert!(slug_error.to_string().contains("sandbox Post slug"));
    }

    #[test]
    fn seeded_media_publication_persists_exact_bytes_and_rejects_mismatched_existing_content() {
        let root = tempfile::tempdir().expect("temporary storage root");
        let target = root
            .path()
            .join("media/upload/95/8e/fixture/baseline-seeded.txt");
        assert!(publish_seeded_media(&target).expect("fixture publication succeeds"));
        assert_eq!(
            std::fs::read(&target).expect("fixture content exists"),
            SANDBOX_SEEDED_MEDIA_BYTES
        );
        std::fs::write(&target, b"wrong bytes").expect("tamper fixture");
        assert!(publish_seeded_media(&target).is_err());
    }

    #[test]
    fn fixture_outcome_returns_a_confirmed_value() {
        assert_eq!(
            confirmed_fixture_outcome(common::MutationOutcome::Confirmed(42), "fixture operation")
                .expect("confirmed outcome"),
            42
        );
    }

    #[test]
    fn fixture_outcome_rejects_an_indeterminate_commit_with_the_operation_label() {
        let error = confirmed_fixture_outcome(
            common::MutationOutcome::CommitIndeterminate(()),
            "fixture operation",
        )
        .expect_err("indeterminate outcome");

        assert_eq!(
            error.to_string(),
            "fixture operation commit acknowledgement was indeterminate"
        );
    }
}

#[cfg(test)]
mod seed_tests {
    //! `SQLite`-only by design: `seed_posts_for_user` has no per-backend
    //! branching — it dispatches through `storage::create_rendered_post`, which
    //! the storage layer implements per backend — so these tests smoke the seed
    //! *logic* on `SQLite` for speed. The tool's dual-backend behaviour is proven
    //! end-to-end by the e2e matrix, which drives `test-support` against both
    //! `SQLite` and `Postgres` ({sqlite,postgres}×{chromium,firefox}).
    use super::*;
    use storage::{PublishedPageRequest, test_support};

    #[tokio::test]
    async fn seeds_public_published_posts_visible_to_a_non_author() {
        let env = test_support::Backend::Sqlite.setup().await;
        let users = env.users();
        let posts = env.posts();
        let write_scope = env.write_scope();
        let user = test_support::SeedUser::new()
            .seed(Arc::clone(&users), write_scope.clone())
            .await;

        let ids = seed_posts_for_user(
            Arc::clone(&users),
            Arc::clone(&posts),
            write_scope.clone(),
            &user.username,
            3,
            true,
            "Timeline Post",
        )
        .await
        .expect("seed ok");
        assert_eq!(ids.len(), 3);

        // The point of the tool: seeded posts are Public + published, so an
        // Anonymous (non-author) viewer sees all three. A bare `posts` insert
        // with no `post_audiences` row would be private and this would return 0
        // — this asserts the tool seeds a *timeline-visible* post, not just a row.
        let page = posts
            .list_published_by_user(
                &user.username,
                PublishedPageRequest::first(
                    common::seed::TimelineOrder::Newest,
                    common::test_support::parse_row_limit("10"),
                ),
                &common::visibility::ViewerIdentity::Anonymous,
                common::time::UtcInstant::now(),
            )
            .await
            .expect("list ok");
        assert_eq!(page.len(), 3);
    }

    #[tokio::test]
    async fn publishes_and_resets_the_author_theme_fixture() {
        let env = test_support::Backend::Sqlite.setup().await;
        let users = env.users();
        let themes = env.themes();
        let write_scope = env.write_scope();
        let user = test_support::SeedUser::new()
            .seed(Arc::clone(&users), write_scope.clone())
            .await;
        let storage = tempfile::TempDir::new().expect("temporary storage");
        let compiled =
            storage::seed_theme_fixture::try_compiled_theme_fixture().expect("valid theme fixture");
        let site_theme = storage::seed_theme_fixture::try_create_theme(
            Arc::clone(&themes),
            write_scope.clone(),
            ThemeOwner::Site,
            &compiled,
        )
        .await
        .expect("site fixture theme");
        let manager = ThemeAssetManager::new(
            Arc::clone(&themes),
            write_scope.clone(),
            Arc::new(storage.path().to_path_buf()),
        );
        let site_publication = manager
            .publish(
                ThemeOwner::Site,
                site_theme,
                &compiled,
                storage::seed_theme_fixture::theme_quota_limits(i64::MAX),
                Timestamp::now().as_second(),
            )
            .await
            .expect("site fixture publishes");
        confirmed_fixture_outcome(site_publication, "publish site fixture")
            .expect("site fixture commit confirmed");

        // Seed twice: the second call must recognize the immutable revision it
        // created, reuse that theme row, and retain the selected fixture.
        seed_published_author_theme(
            Arc::clone(&users),
            Arc::clone(&themes),
            write_scope.clone(),
            storage.path(),
            user.username.as_ref(),
        )
        .await
        .expect("theme fixture publishes");
        seed_published_author_theme(
            Arc::clone(&users),
            Arc::clone(&themes),
            write_scope.clone(),
            storage.path(),
            user.username.as_ref(),
        )
        .await
        .expect("existing published fixture is reused");
        reset_author_theme_fixture(
            Arc::clone(&users),
            Arc::clone(&themes),
            write_scope.clone(),
            user.username.as_ref(),
        )
        .await
        .expect("theme fixture resets");
    }

    #[tokio::test]
    async fn rejects_a_prefix_that_cannot_form_a_valid_slug() {
        let env = test_support::Backend::Sqlite.setup().await;
        let users = env.users();
        let posts = env.posts();
        let write_scope = env.write_scope();
        let user = test_support::SeedUser::new()
            .seed(Arc::clone(&users), write_scope.clone())
            .await;

        // A prefix with no alphanumerics collapses to an empty base, so the slug
        // would begin with '-' and fail `Slug` parsing — surfaced as an error
        // (not a panic) before any post is persisted.
        let err = seed_posts_for_user(
            Arc::clone(&users),
            Arc::clone(&posts),
            write_scope.clone(),
            &user.username,
            1,
            false,
            "***",
        )
        .await
        .expect_err("invalid generated slug should error");
        assert!(err.to_string().contains("generated slug invalid"));
    }
}

#[cfg(test)]
mod dead_letter_tests {
    //! `SQLite`-only by design (same rationale as `seed_tests`): this fixture
    //! has no backend-specific policy and the e2e matrix exercises both
    //! supported storage dialects.
    use super::*;
    use common::pagination::PageSize;
    use storage::test_support;

    #[tokio::test]
    async fn seeds_terminal_events_in_each_requested_phase() {
        let env = test_support::Backend::Sqlite.setup().await;
        let feed_events = env.feed_events();
        let write_scope = env.write_scope();

        for phase in [FeedEventPhase::Regeneration, FeedEventPhase::Publication] {
            let ids = seed_dead_letters(Arc::clone(&feed_events), write_scope.clone(), phase, 2)
                .await
                .expect("seed ok");

            let page = feed_events
                .dead_letters(phase, None, PageSize::default())
                .await
                .expect("dead-letter page");
            assert_eq!(page.events.len(), 2);
            assert!(page.next_cursor.is_none());
            for event in page.events {
                assert!(ids.contains(&event.id));
                assert_eq!(event.phase, phase);
                assert_eq!(event.attempts, 1);
                assert!(
                    event
                        .diagnostic
                        .as_deref()
                        .is_some_and(|diagnostic| diagnostic.contains("fixture"))
                );
            }
        }
    }

    #[tokio::test]
    async fn reports_atomic_seed_transaction_failures() {
        let env = test_support::Backend::Sqlite.setup().await;
        let feed_events = env.feed_events();
        let write_scope = env.write_scope();
        env.base.close_pool().await;

        let error = seed_dead_letters(
            Arc::clone(&feed_events),
            write_scope.clone(),
            FeedEventPhase::Regeneration,
            1,
        )
        .await
        .expect_err("closed storage must reject the atomic fixture");

        assert!(
            error
                .to_string()
                .contains("atomic WebSub dead-letter fixture failed")
        );
    }
}

#[cfg(test)]
mod create_user_tests {
    //! `SQLite`-only by design (same rationale as `seed_tests`): `create_user`
    //! has no per-backend branching — it dispatches through
    //! `UserStorage::create_user`, implemented per backend — so the e2e matrix
    //! proves the dual-backend path; here we smoke the logic on `SQLite`.
    use super::*;
    use storage::test_support;

    #[tokio::test]
    async fn creates_a_lookupable_operator_and_rejects_duplicates() {
        let env = test_support::Backend::Sqlite.setup().await;
        let users = env.users();
        let write_scope = env.write_scope();

        let id = create_user(
            Arc::clone(&users),
            write_scope.clone(),
            "testoperator",
            "testpassword123",
            None,
            true,
        )
        .await
        .expect("create ok");

        let u = users
            .get_user_by_username(&"testoperator".parse().unwrap())
            .await
            .expect("lookup ok")
            .expect("user exists");
        assert_eq!(u.user_id, id);
        assert_eq!(u.is_operator, OperatorStatus::OPERATOR);

        // A freshly-init'd DB has a per-user uniqueness constraint, so a second
        // create with the same username surfaces as an error (no upsert).
        create_user(
            Arc::clone(&users),
            write_scope.clone(),
            "testoperator",
            "testpassword123",
            None,
            false,
        )
        .await
        .expect_err("duplicate username should error");
    }
}

#[cfg(test)]
mod seed_session_tests {
    //! `SQLite`-only by design (same rationale as `seed_tests`): the seed
    //! functions have no per-backend branching — they dispatch through
    //! `UserStorage` / `SessionStorage`, implemented per backend — so the e2e
    //! matrix proves the dual-backend path; here we smoke the logic on `SQLite`.
    use super::*;
    use common::local_storage_key::LocalStorageKey;
    use common::session_user::decode_marker;
    use common::token::RawToken;
    use storage::test_support;

    /// The token a browser would send back, recovered from the record's
    /// `Set-Cookie` value exactly as a client would: the first `name=value`
    /// pair, up to the first `;`.
    fn cookie_token(record: &SeedRecord) -> RawToken {
        record
            .set_cookie
            .split(';')
            .next()
            .expect("cookie pair")
            .strip_prefix("session=")
            .expect("session cookie")
            .parse()
            .expect("token parses")
    }

    async fn authenticate_session(
        sessions: Arc<dyn SessionStorage>,
        write_scope: WriteScope,
        token: &RawToken,
    ) -> anyhow::Result<storage::SessionRecord> {
        let token = token.clone();
        let outcome = write_scope
            .run(move |transaction| {
                let sessions = Arc::clone(&sessions);
                Box::pin(async move { sessions.authenticate(transaction, &token).await })
            })
            .await
            .map_err(|error| anyhow::anyhow!("fixture session authentication failed: {error}"))?;
        confirmed_fixture_outcome(outcome, "fixture session authentication")
    }

    #[tokio::test]
    async fn seed_user_returns_a_session_that_authenticates() {
        let env = test_support::Backend::Sqlite.setup().await;
        let users = env.users();
        let sessions = env.sessions();
        let write_scope = env.write_scope();

        let record = seed_user(
            Arc::clone(&users),
            Arc::clone(&sessions),
            write_scope.clone(),
            "alice",
            "password123",
            None,
        )
        .await
        .expect("seed ok");

        // The cookie's token authenticates and resolves to the seeded user.
        let token = cookie_token(&record);
        let session = authenticate_session(Arc::clone(&sessions), write_scope.clone(), &token)
            .await
            .expect("token authenticates");
        assert_eq!(session.user_id, UserId::from(record.user_id));

        // The marker round-trips to the seeded identity, keyed by the shared
        // registry — never a restated literal.
        assert_eq!(record.marker_key, LocalStorageKey::AuthMarker.as_ref());
        let marker = decode_marker(&record.marker).expect("marker decodes");
        assert_eq!(marker.username, "alice");
        assert!(!marker.is_operator);

        // The default label makes seeded sessions obvious on /sessions.
        let listed_sessions = sessions
            .list_sessions(session.user_id)
            .await
            .expect("list ok");
        assert_eq!(listed_sessions.len(), 1);
        assert_eq!(listed_sessions[0].label, "E2E seed");
    }

    #[tokio::test]
    async fn seed_user_honours_an_explicit_label() {
        let env = test_support::Backend::Sqlite.setup().await;
        let users = env.users();
        let sessions = env.sessions();
        let write_scope = env.write_scope();

        let record = seed_user(
            Arc::clone(&users),
            Arc::clone(&sessions),
            write_scope.clone(),
            "alice",
            "password123",
            Some("CI bot"),
        )
        .await
        .expect("seed ok");
        let listed_sessions = sessions
            .list_sessions(UserId::from(record.user_id))
            .await
            .expect("list ok");
        assert_eq!(listed_sessions[0].label, "CI bot");
    }

    #[tokio::test]
    async fn create_session_for_user_reflects_the_operator_flag() {
        let env = test_support::Backend::Sqlite.setup().await;
        let users = env.users();
        let sessions = env.sessions();
        let write_scope = env.write_scope();
        create_user(
            Arc::clone(&users),
            write_scope.clone(),
            "testoperator",
            "testpassword123",
            None,
            true,
        )
        .await
        .expect("create ok");

        let record = create_session_for_user(
            Arc::clone(&users),
            Arc::clone(&sessions),
            write_scope.clone(),
            "testoperator",
            None,
        )
        .await
        .expect("session ok");
        let marker = decode_marker(&record.marker).expect("marker decodes");
        assert!(marker.is_operator, "operator user's marker must say so");
        let token = cookie_token(&record);
        authenticate_session(Arc::clone(&sessions), write_scope.clone(), &token)
            .await
            .expect("token authenticates");
    }

    #[tokio::test]
    async fn create_session_for_user_unknown_username_errors() {
        let env = test_support::Backend::Sqlite.setup().await;
        let users = env.users();
        let sessions = env.sessions();
        let write_scope = env.write_scope();
        create_session_for_user(
            Arc::clone(&users),
            Arc::clone(&sessions),
            write_scope.clone(),
            "ghost",
            None,
        )
        .await
        .expect_err("unknown user should error");
    }

    #[tokio::test]
    async fn seed_user_duplicate_username_errors() {
        let env = test_support::Backend::Sqlite.setup().await;
        let users = env.users();
        let sessions = env.sessions();
        let write_scope = env.write_scope();
        seed_user(
            Arc::clone(&users),
            Arc::clone(&sessions),
            write_scope.clone(),
            "alice",
            "password123",
            None,
        )
        .await
        .expect("first seed ok");
        seed_user(
            Arc::clone(&users),
            Arc::clone(&sessions),
            write_scope.clone(),
            "alice",
            "password123",
            None,
        )
        .await
        .expect_err("duplicate username should error");
    }
}

#[cfg(test)]
mod reset_mail_tests {
    use super::*;

    #[test]
    fn removes_an_existing_file_and_is_idempotent() {
        let dir = tempfile::TempDir::new().unwrap();
        let path = dir.path().join("mail.jsonl");
        std::fs::write(&path, "{}\n").unwrap();
        assert!(path.exists());

        reset_mail(&path).expect("remove ok");
        assert!(!path.exists(), "file should be gone");

        // rm -f semantics: a second reset on the now-missing file is still Ok.
        reset_mail(&path).expect("missing file is not an error");
    }

    #[test]
    fn propagates_errors_other_than_not_found() {
        // `remove_file` on a directory fails with a non-`NotFound` error, so the
        // catch-all arm surfaces it (rather than swallowing it like a missing file).
        let dir = tempfile::TempDir::new().unwrap();
        let subdir = dir.path().join("a-directory");
        std::fs::create_dir(&subdir).unwrap();

        let err = reset_mail(&subdir).expect_err("removing a directory should error");
        assert!(err.to_string().contains("reset-mail"));
    }
}
