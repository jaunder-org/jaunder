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

use std::{fmt::Write as _, path::Path, sync::Arc};

use async_trait::async_trait;
use common::display_name::DisplayName;
use common::ids::{FeedEventId, PostId, UserId};
use common::media::{ContentType, MediaReference};
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
    FeedEventStorage, ForeignEvidenceSink, InstanceId, LocalMediaSink, MediaManager,
    MediaReferenceEvidence, MediaReferenceOwnershipResolver, OperatorStatus,
    PersistedMediaReference, PostBookkeepingExpectation, PostFormat, PostStorage,
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
    /// Native Markdown or Org source.
    pub body: String,
    /// Source format used by the real renderer.
    pub format: PostFormat,
    /// Exact publication timestamp, or `None` for a draft.
    pub published_at: Option<UtcInstant>,
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

/// Produces the complete typed fixture manifest from canonical upload URLs.
///
/// The offset sequence is deliberately allocated globally: all 60 published
/// Posts have a distinct timestamp while every author still receives the same
/// 12 Markdown / 3 Org distribution.
#[must_use]
pub fn sandbox_profile_manifest(
    anchor: UtcInstant,
    asset_urls: &[RootRelativeUrl; 4],
) -> Vec<SandboxPost> {
    let mut posts = Vec::with_capacity(68);
    let mut first_offset = 1_i64;
    for (fixture, asset_url) in SANDBOX_USER_FIXTURES.iter().zip(asset_urls) {
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
        });
        posts.push(SandboxPost {
            author: fixture.username,
            title: format!("{} sandbox Org draft", fixture.username),
            slug: "sandbox-org-draft".to_owned(),
            body: SHORT_ORG.to_owned(),
            format: PostFormat::Org,
            published_at: None,
        });
    }
    posts
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
        body,
        format: fixture.format,
        published_at: fixture.published_at,
        summary: None,
        audiences: vec![AudienceTarget::Public],
        tags: Vec::new(),
        idempotency_key: None,
        expectations: PostBookkeepingExpectation::default(),
    })
}

/// Seeds the fixed standard sandbox profile through its exact storage services.
///
/// # Errors
///
/// Returns an error when the configuration or User write fails or its commit
/// acknowledgement is indeterminate.
pub async fn seed_standard_sandbox_profile(
    site_config: Arc<dyn SiteConfigStorage>,
    users: Arc<dyn UserStorage>,
    write_scope: WriteScope,
) -> anyhow::Result<()> {
    seed_sandbox_users(site_config, users, write_scope, SandboxProfile::Standard).await?;
    Ok(())
}

/// Seeds the fixed demo sandbox profile through its exact storage services.
///
/// Site configuration and Users commit before Media placement; uploads use their
/// own manager-owned writes; Posts then commit as one final batch. The staged
/// sandbox workspace owns external atomicity across these phases.
///
/// # Errors
///
/// Returns an error when any phase fails or its commit acknowledgement is
/// indeterminate.
pub async fn seed_demo_sandbox_profile(
    site_config: Arc<dyn SiteConfigStorage>,
    users: Arc<dyn UserStorage>,
    posts: Arc<dyn PostStorage>,
    write_scope: WriteScope,
    media_manager: &MediaManager,
    anchor: UtcInstant,
) -> anyhow::Result<()> {
    seed_demo_sandbox_profile_inner(
        site_config,
        users,
        posts,
        write_scope,
        media_manager,
        anchor,
        #[cfg(test)]
        None,
    )
    .await
}

#[cfg(test)]
type SandboxPostPhaseHook = Box<dyn FnOnce() -> anyhow::Result<()> + Send>;

async fn seed_demo_sandbox_profile_inner(
    site_config: Arc<dyn SiteConfigStorage>,
    users: Arc<dyn UserStorage>,
    posts: Arc<dyn PostStorage>,
    write_scope: WriteScope,
    media_manager: &MediaManager,
    anchor: UtcInstant,
    #[cfg(test)] phase_hook: Option<SandboxPostPhaseHook>,
) -> anyhow::Result<()> {
    let user_ids = seed_sandbox_users(
        site_config,
        users,
        write_scope.clone(),
        SandboxProfile::Demo,
    )
    .await?;
    let asset_urls = upload_sandbox_media(media_manager, &user_ids).await?;

    #[cfg(test)]
    if let Some(phase_hook) = phase_hook {
        phase_hook()?;
    }

    seed_sandbox_posts(posts, write_scope, user_ids, anchor, &asset_urls).await
}

async fn seed_sandbox_users(
    site_config: Arc<dyn SiteConfigStorage>,
    users: Arc<dyn UserStorage>,
    write_scope: WriteScope,
    profile: SandboxProfile,
) -> anyhow::Result<Vec<(&'static str, UserId)>> {
    let sandbox_users = match profile {
        SandboxProfile::Standard => &SANDBOX_USER_FIXTURES[..2],
        SandboxProfile::Demo => &SANDBOX_USER_FIXTURES,
    };
    let password = SANDBOX_PASSWORD
        .parse::<host::password::Password>()
        .map_err(|error| anyhow::anyhow!("invalid fixed sandbox password: {error}"))?;
    let mut passwords = Vec::with_capacity(sandbox_users.len());
    for _ in sandbox_users {
        passwords.push(
            storage::prepare_password(password.clone())
                .await
                .map_err(|error| anyhow::anyhow!("sandbox password preparation failed: {error}"))?,
        );
    }
    let title = SANDBOX_TITLE
        .parse::<SiteTitle>()
        .map_err(|error| anyhow::anyhow!("invalid fixed sandbox title: {error}"))?
        .to_string();
    let outcome = write_scope
        .run(move |transaction| {
            Box::pin(async move {
                site_config
                    .set(transaction, SiteConfigKey::SiteTitle, &title)
                    .await?;
                let mut user_ids = Vec::with_capacity(sandbox_users.len());
                for (fixture, password) in sandbox_users.iter().copied().zip(passwords) {
                    let Ok(username) = fixture.username.parse::<Username>() else {
                        unreachable!("fixed sandbox usernames are valid");
                    };
                    let role = if fixture.operator {
                        OperatorStatus::OPERATOR
                    } else {
                        OperatorStatus::STANDARD
                    };
                    let user_id = users
                        .create_user(transaction, &username, &password, None, role)
                        .await?;
                    user_ids.push((fixture.username, user_id));
                }
                Ok::<_, anyhow::Error>(user_ids)
            })
        })
        .await
        .map_err(|error| anyhow::anyhow!("sandbox Users write failed: {error}"))?;
    confirmed_fixture_outcome(outcome, "sandbox Users write")
}

async fn upload_sandbox_media(
    media_manager: &MediaManager,
    user_ids: &[(&str, UserId)],
) -> anyhow::Result<[RootRelativeUrl; 4]> {
    let Ok(content_type) = "image/svg+xml".parse::<ContentType>() else {
        unreachable!("fixed SVG content type is valid");
    };
    let mut asset_urls = Vec::with_capacity(SANDBOX_USER_FIXTURES.len());
    for fixture in SANDBOX_USER_FIXTURES {
        let Some(user_id) = user_ids
            .iter()
            .find_map(|(username, user_id)| (*username == fixture.username).then_some(*user_id))
        else {
            unreachable!("demo fixture User was created");
        };
        let filename = MediaManager::validate_filename(Some(fixture.media.filename))
            .map_err(|error| anyhow::anyhow!("invalid fixed sandbox Media filename: {error}"))?;
        let outcome = media_manager
            .upload_bytes(
                user_id,
                &filename,
                content_type.clone(),
                fixture.media.bytes,
            )
            .await
            .map_err(|error| anyhow::anyhow!("sandbox Media upload failed: {error}"))?;
        asset_urls.push(confirmed_fixture_outcome(outcome, "sandbox Media upload")?.url);
    }
    let Ok(asset_urls) = <[RootRelativeUrl; 4]>::try_from(asset_urls) else {
        unreachable!("one upload result per fixed sandbox User");
    };
    Ok(asset_urls)
}

async fn seed_sandbox_posts(
    posts: Arc<dyn PostStorage>,
    write_scope: WriteScope,
    user_ids: Vec<(&'static str, UserId)>,
    anchor: UtcInstant,
    asset_urls: &[RootRelativeUrl; 4],
) -> anyhow::Result<()> {
    let manifest = sandbox_profile_manifest(anchor, asset_urls);
    let outcome = write_scope
        .run(move |transaction| {
            Box::pin(async move {
                let mut inputs = Vec::with_capacity(manifest.len());
                for fixture in manifest {
                    let Some(user_id) = user_ids.iter().find_map(|(username, user_id)| {
                        (*username == fixture.author).then_some(*user_id)
                    }) else {
                        unreachable!("sandbox manifest authors are seeded users");
                    };
                    inputs.push(render_post_input(sandbox_post_content(&fixture, user_id)?));
                }
                let ids = posts.create_posts(transaction, &inputs).await?;
                Ok::<_, anyhow::Error>(ids)
            })
        })
        .await
        .map_err(|error| anyhow::anyhow!("sandbox Posts write failed: {error}"))?;
    confirmed_fixture_outcome(outcome, "sandbox Posts write")?;
    Ok(())
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
/// Returns `Err` if the username is invalid or unknown, or the label is
/// invalid.
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
    use storage::test_support;
    type StoredSandboxPost = (
        String,
        String,
        String,
        String,
        PostFormat,
        Option<UtcInstant>,
    );
    fn assert_demo_aggregates(actual: &[StoredSandboxPost], anchor: UtcInstant) {
        assert_eq!(actual.len(), 68);
        assert_eq!(
            actual
                .iter()
                .filter(|(_, _, _, _, _, published_at)| published_at.is_some())
                .count(),
            60
        );
        assert_eq!(
            actual
                .iter()
                .filter(|(_, _, _, _, format, _)| *format == PostFormat::Markdown)
                .count(),
            52
        );
        assert_eq!(
            actual
                .iter()
                .filter(|(_, _, _, _, format, _)| *format == PostFormat::Org)
                .count(),
            16
        );
        let offsets = actual
            .iter()
            .filter_map(|(_, _, _, _, _, published_at)| *published_at)
            .map(|published_at| {
                (anchor.value().as_second() - published_at.value().as_second()) / 86_400
            })
            .collect::<std::collections::BTreeSet<_>>();
        assert_eq!(offsets, (1_i64..=60).collect());
        assert_eq!(anchor.value().as_second().rem_euclid(60), 0);
        assert_eq!(anchor.value().subsec_nanosecond(), 0);
        let generated_anchor = sandbox_profile_anchor();
        assert_eq!(generated_anchor.value().as_second().rem_euclid(60), 0);
        assert_eq!(generated_anchor.value().subsec_nanosecond(), 0);
    }

    async fn assert_loginable(users: Arc<dyn UserStorage>, username: &str) {
        let username = username.parse::<Username>().expect("fixed username");
        let password = SANDBOX_PASSWORD
            .parse::<host::password::Password>()
            .expect("fixed password");
        users
            .prepare_authentication(&username, &password)
            .await
            .expect("fixed fixture credentials authenticate");
    }

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
            assert_loginable(Arc::clone(&users), username.as_ref()).await;
        }
    }

    fn sandbox_media_manager(env: &test_support::TestEnv) -> MediaManager {
        MediaManager::new(
            env.media(),
            env.posts(),
            env.site_config(),
            env.write_scope(),
            Arc::new(storage::MediaContentLocks::new(Arc::new(
                env.base.path().to_path_buf(),
            ))),
            env.base.instance_id().clone(),
            Arc::new(SandboxMediaOwnershipResolver),
        )
    }

    #[tokio::test]
    async fn standard_profile_has_only_its_explicit_configuration_and_loginable_users() {
        let env = test_support::Backend::Sqlite.setup().pristine().await;
        let site_config = env.site_config();
        let users = env.users();
        let posts = env.posts();
        seed_standard_sandbox_profile(
            Arc::clone(&site_config),
            Arc::clone(&users),
            env.write_scope(),
        )
        .await
        .expect("standard profile seeds");

        assert_eq!(
            site_config.list().await.expect("site config list"),
            vec![("site.title".to_owned(), SANDBOX_TITLE.to_owned())]
        );
        assert!(
            site_config
                .get_raw(SiteConfigKey::SiteBaseUrl)
                .await
                .expect("base URL lookup")
                .is_none()
        );
        assert!(
            site_config
                .get_raw(SiteConfigKey::SiteRegistrationPolicy)
                .await
                .expect("registration policy lookup")
                .is_none()
        );
        assert_sandbox_users(Arc::clone(&users), &SANDBOX_USER_FIXTURES[..2]).await;
        for fixture in &SANDBOX_USER_FIXTURES[2..] {
            assert!(
                users
                    .get_user_by_username(&fixture.username.parse().expect("fixed username"))
                    .await
                    .expect("user lookup")
                    .is_none()
            );
        }
        for fixture in &SANDBOX_USER_FIXTURES[..2] {
            let user = users
                .get_user_by_username(&fixture.username.parse().expect("fixed username"))
                .await
                .expect("user lookup")
                .expect("fixture user exists");
            assert!(
                posts
                    .list_collection_by_user(
                        user.user_id,
                        None,
                        common::test_support::parse_row_limit("100"),
                    )
                    .await
                    .expect("post listing")
                    .is_empty()
            );
        }
    }

    async fn demo_asset_urls(
        env: &test_support::TestEnv,
        users: &Arc<dyn UserStorage>,
    ) -> [RootRelativeUrl; 4] {
        const HASHES: [&str; 4] = [
            "81aa7378e6c8ef6a707e281d5a92c646a1ba3c01de428131ecf677763a8cddd9",
            "92c501114a2d5f3b82f50a4ac01ab2c1eeb1223cbc46e7840347adb3e6aa8847",
            "bb2cd32aaa8d6b4bd87af8980a5bb220753bdfafdd953bc4c9b4a861d1e4c233",
            "2e8d0525784fb6a8b04b82131e9d82cddca52445e21c1faa0baccbc7ad44290c",
        ];
        let mut asset_urls = Vec::with_capacity(SANDBOX_USER_FIXTURES.len());
        for (fixture, expected_hash) in SANDBOX_USER_FIXTURES.iter().zip(HASHES) {
            let user = users
                .get_user_by_username(&fixture.username.parse().expect("fixed username"))
                .await
                .expect("user lookup")
                .expect("fixture user exists");
            let records = env
                .media()
                .list_media(
                    user.user_id,
                    None,
                    common::test_support::parse_row_limit("2"),
                    common::pagination::PageOffset::default(),
                )
                .await
                .expect("list Media");
            assert_eq!(records.len(), 1);
            let record = &records[0];
            assert_eq!(record.user_id, user.user_id);
            assert_eq!(record.filename.as_ref(), fixture.media.filename);
            assert_eq!(record.sha256.as_ref(), expected_hash);
            assert_eq!(record.source_url, None);
            assert_eq!(record.content_type.as_ref(), "image/svg+xml");
            assert_eq!(
                record.size_bytes.value(),
                i64::try_from(fixture.media.bytes.len()).expect("fixture size fits i64")
            );
            assert_eq!(record.source, common::media::MediaSource::Upload);
            let path = env.base.path().join("media").join(common::media::path(
                &record.source,
                &record.sha256,
                &record.filename,
            ));
            assert_eq!(
                std::fs::read(path).expect("read stored Media"),
                fixture.media.bytes
            );
            asset_urls.push(common::media::url(
                &record.source,
                &record.sha256,
                &record.filename,
            ));
        }
        asset_urls
            .try_into()
            .expect("one stored Media URL per sandbox User")
    }

    fn expected_curated_html(slug: &str, asset_url: &RootRelativeUrl) -> String {
        match slug {
            "horizon-worth-keeping" => format!(
                "<h1>A horizon worth keeping</h1>\n<p>A <strong>small observation</strong> can guide a whole day.</p>\n<p><a href=\"/notes/horizon\" rel=\"noopener noreferrer\">Read the field notes</a>.</p>\n<ul>\n<li>Watch the light</li>\n<li>Keep the useful detail</li>\n</ul>\n<pre><code class=\"language-text\">horizon = \"clear\"\n</code></pre>\n<table><thead><tr><th>Moment</th><th>Choice</th></tr></thead><tbody>\n<tr><td>Morning</td><td>Walk</td></tr>\n<tr><td>Evening</td><td>Write</td></tr>\n</tbody></table>\n<p><img src=\"{asset_url}\" alt=\"Blue horizon\"></p>\n"
            ),
            "calm-field-note" => format!(
                "<h1>A calm field note</h1><p></p><p>A <i>steady practice</i> makes room for better work.\n</p><p><a href=\"/notes/practice\" rel=\"noopener noreferrer\">Read the practice note</a>\n</p><ul><li><p>Name the question\n</p></li><li><p>Share the answer\n</p></li></ul><pre><code class=\"language-text\">answer = \"kind\"\n</code></pre><table><thead><tr><td>Moment</td><td>Choice</td></tr></thead><tbody><tr><td>Morning</td><td>Listen</td></tr><tr><td>Evening</td><td>Rest</td></tr></tbody></table><p><img src=\"{asset_url}\"></p>"
            ),
            "workshop-checks" => format!(
                "<h1>Workshop checks</h1>\n<p>A <strong>clear checklist</strong> makes maintenance less surprising.</p>\n<p><a href=\"/notes/workshop\" rel=\"noopener noreferrer\">Review the runbook</a>.</p>\n<ol>\n<li>Open the bench</li>\n<li>Record the result</li>\n</ol>\n<pre><code class=\"language-text\">status = \"ready\"\n</code></pre>\n<table><thead><tr><th>Tool</th><th>State</th></tr></thead><tbody>\n<tr><td>Saw</td><td>Ready</td></tr>\n<tr><td>Lamp</td><td>Warm</td></tr>\n</tbody></table>\n<p><img src=\"{asset_url}\" alt=\"Warm workshop\"></p>\n"
            ),
            "workshop-rhythm" => format!(
                "<h1>Workshop rhythm</h1><p></p><p>A <b>shared routine</b> keeps the room useful.\n</p><p><a href=\"/notes/rhythm\" rel=\"noopener noreferrer\">Read the workshop rhythm</a>\n</p><ol><li><p>Check the bench\n</p></li><li><p>Leave a note\n</p></li></ol><pre><code class=\"language-text\">room = \"open\"\n</code></pre><table><thead><tr><td>Tool</td><td>State</td></tr></thead><tbody><tr><td>Saw</td><td>Ready</td></tr><tr><td>Lamp</td><td>Warm</td></tr></tbody></table><p><img src=\"{asset_url}\"></p>"
            ),
            "field-paths" => format!(
                "<h1>Field paths</h1>\n<p>A <strong>patient route</strong> notices what hurried travel misses.</p>\n<p><a href=\"/notes/field-paths\" rel=\"noopener noreferrer\">See the path map</a>.</p>\n<ul>\n<li>Follow the shade</li>\n<li>Mark the turn</li>\n</ul>\n<pre><code class=\"language-text\">pace = \"slow\"\n</code></pre>\n<table><thead><tr><th>Place</th><th>Sound</th></tr></thead><tbody>\n<tr><td>Gate</td><td>Birds</td></tr>\n<tr><td>Hill</td><td>Wind</td></tr>\n</tbody></table>\n<p><img src=\"{asset_url}\" alt=\"Green field\"></p>\n"
            ),
            "field-margins" => format!(
                "<h1>Field margins</h1><p></p><p>A <i>careful walk</i> gives a place time to speak.\n</p><p><a href=\"/notes/margins\" rel=\"noopener noreferrer\">Read the field margin</a>\n</p><ul><li><p>Follow the shade\n</p></li><li><p>Mark the turn\n</p></li></ul><pre><code class=\"language-text\">pace = \"slow\"\n</code></pre><table><thead><tr><td>Place</td><td>Sound</td></tr></thead><tbody><tr><td>Gate</td><td>Birds</td></tr><tr><td>Hill</td><td>Wind</td></tr></tbody></table><p><img src=\"{asset_url}\"></p>"
            ),
            "night-signals" => format!(
                "<h1>Night signals</h1>\n<p>A <strong>quiet sky</strong> makes a distant signal easier to see.</p>\n<p><a href=\"/notes/night-signals\" rel=\"noopener noreferrer\">Open the signal log</a>.</p>\n<ol>\n<li>Dim the lamp</li>\n<li>Wait for the blink</li>\n</ol>\n<pre><code class=\"language-text\">signal = \"seen\"\n</code></pre>\n<table><thead><tr><th>Hour</th><th>Signal</th></tr></thead><tbody>\n<tr><td>Nine</td><td>Faint</td></tr>\n<tr><td>Ten</td><td>Clear</td></tr>\n</tbody></table>\n<p><img src=\"{asset_url}\" alt=\"Violet night\"></p>\n"
            ),
            "night-watch" => format!(
                "<h1>Night watch</h1><p></p><p>A <b>quiet room</b> turns waiting into attention.\n</p><p><a href=\"/notes/night-watch\" rel=\"noopener noreferrer\">Read the night watch</a>\n</p><ol><li><p>Dim the lamp\n</p></li><li><p>Wait for the blink\n</p></li></ol><pre><code class=\"language-text\">signal = \"seen\"\n</code></pre><table><thead><tr><td>Hour</td><td>Signal</td></tr></thead><tbody><tr><td>Nine</td><td>Faint</td></tr><tr><td>Ten</td><td>Clear</td></tr></tbody></table><p><img src=\"{asset_url}\"></p>"
            ),
            _ => unreachable!("curated Post fixtures use only fixed slugs"),
        }
    }

    async fn assert_curated_rendering(
        users: &Arc<dyn UserStorage>,
        asset_urls: &[RootRelativeUrl; 4],
    ) {
        for (fixture, asset_url) in SANDBOX_USER_FIXTURES.iter().zip(asset_urls) {
            let user = users
                .get_user_by_username(&fixture.username.parse().expect("fixed username"))
                .await
                .expect("user lookup")
                .expect("fixture user exists");
            let expected_media =
                common::media::parse_media_url(asset_url.as_ref()).expect("canonical Media URL");
            for curated in fixture.materialize_curated_posts(asset_url) {
                let rendered = render_post_input(
                    sandbox_post_content(&curated, user.user_id).expect("fixture input"),
                )
                .rendered;
                let html = rendered.html().as_ref();
                assert_eq!(html, expected_curated_html(&curated.slug, asset_url));
                assert_eq!(rendered.media().len(), 1);
                assert!(matches!(
                    rendered.media()[0].kind(),
                    common::media::MediaReferenceKind::Local
                ));
                assert_eq!(rendered.media()[0].media(), expected_media.media());
            }
        }
    }

    async fn stored_sandbox_posts(
        users: &Arc<dyn UserStorage>,
        posts: &Arc<dyn PostStorage>,
    ) -> Vec<StoredSandboxPost> {
        let mut actual = Vec::new();
        for fixture in &SANDBOX_USER_FIXTURES {
            let user = users
                .get_user_by_username(&fixture.username.parse().expect("fixed username"))
                .await
                .expect("user lookup")
                .expect("fixture user exists");
            actual.extend(
                posts
                    .list_collection_by_user(
                        user.user_id,
                        None,
                        common::test_support::parse_row_limit("100"),
                    )
                    .await
                    .expect("post listing")
                    .into_iter()
                    .map(|record| {
                        (
                            record.author_username.to_string(),
                            record.title.expect("manifest title").to_string(),
                            record.slug.to_string(),
                            record.body.to_string(),
                            record.format,
                            record.published_at,
                        )
                    }),
            );
        }
        actual
    }

    #[tokio::test]
    async fn demo_profile_matches_the_typed_manifest_and_rounded_anchor() {
        let env = test_support::Backend::Sqlite.setup().pristine().await;
        let site_config = env.site_config();
        let users = env.users();
        let posts = env.posts();
        let anchor = "2026-09-06T12:34:00Z"
            .parse::<UtcInstant>()
            .expect("fixed minute anchor");
        let media_manager = sandbox_media_manager(&env);
        seed_demo_sandbox_profile(
            Arc::clone(&site_config),
            Arc::clone(&users),
            Arc::clone(&posts),
            env.write_scope(),
            &media_manager,
            anchor,
        )
        .await
        .expect("demo profile seeds");

        let asset_urls = demo_asset_urls(&env, &users).await;
        assert_curated_rendering(&users, &asset_urls).await;
        assert_eq!(
            site_config.list().await.expect("site config list"),
            vec![("site.title".to_owned(), SANDBOX_TITLE.to_owned())]
        );
        assert_sandbox_users(Arc::clone(&users), &SANDBOX_USER_FIXTURES).await;

        let mut actual = stored_sandbox_posts(&users, &posts).await;
        let mut expected = sandbox_profile_manifest(anchor, &asset_urls)
            .into_iter()
            .map(|fixture| {
                (
                    fixture.author.to_owned(),
                    fixture.title,
                    fixture.slug,
                    fixture.body,
                    fixture.format,
                    fixture.published_at,
                )
            })
            .collect::<Vec<_>>();
        actual.sort_by(|left, right| left.2.cmp(&right.2).then(left.0.cmp(&right.0)));
        expected.sort_by(|left, right| left.2.cmp(&right.2).then(left.0.cmp(&right.0)));
        assert_eq!(actual, expected);
        assert_demo_aggregates(&actual, anchor);
    }

    #[tokio::test]
    async fn demo_phase_failure_after_real_uploads_retains_users_and_media_but_not_posts() {
        let env = test_support::Backend::Sqlite.setup().pristine().await;
        let users = env.users();
        let posts = env.posts();
        let media_manager = sandbox_media_manager(&env);
        let error = seed_demo_sandbox_profile_inner(
            env.site_config(),
            Arc::clone(&users),
            Arc::clone(&posts),
            env.write_scope(),
            &media_manager,
            "2026-09-06T12:34:00Z".parse().expect("fixed anchor"),
            Some(Box::new(|| anyhow::bail!("injected post phase failure"))),
        )
        .await
        .expect_err("injected phase failure propagates");
        assert!(error.to_string().contains("injected post phase failure"));

        for fixture in SANDBOX_USER_FIXTURES {
            let user = users
                .get_user_by_username(&fixture.username.parse().expect("fixed username"))
                .await
                .expect("user lookup")
                .expect("phase one User committed");
            let records = env
                .media()
                .list_media(
                    user.user_id,
                    None,
                    common::test_support::parse_row_limit("2"),
                    common::pagination::PageOffset::default(),
                )
                .await
                .expect("list real uploaded Media");
            assert_eq!(records.len(), 1, "upload phase committed each asset");
            assert_eq!(
                std::fs::read(env.base.path().join("media").join(common::media::path(
                    &records[0].source,
                    &records[0].sha256,
                    &records[0].filename,
                )),)
                .expect("uploaded bytes remain"),
                fixture.media.bytes
            );
            assert!(
                posts
                    .list_collection_by_user(
                        user.user_id,
                        None,
                        common::test_support::parse_row_limit("100"),
                    )
                    .await
                    .expect("list Posts")
                    .is_empty(),
                "post phase did not start"
            );
        }
    }

    #[test]
    fn demo_fixture_owns_exact_svg_assets_and_curated_native_sources() {
        let expected_assets = [
            (
                "user",
                false,
                "blue-horizon.svg",
                br##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 96 64" role="img" aria-label="A blue horizon"><rect width="96" height="64" fill="#dbeafe"/><path d="M0 43h96v21H0z" fill="#2563eb"/><circle cx="70" cy="20" r="11" fill="#facc15"/></svg>"## as &[u8],
            ),
            (
                "operator",
                true,
                "warm-workshop.svg",
                br##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 96 64" role="img" aria-label="A warm workshop"><rect width="96" height="64" fill="#ffedd5"/><path d="M16 48 48 12l32 36z" fill="#ea580c"/><path d="M38 48V34h20v14" fill="#7c2d12"/></svg>"##,
            ),
            (
                "alice",
                false,
                "green-field.svg",
                br##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 96 64" role="img" aria-label="A green field"><rect width="96" height="64" fill="#dcfce7"/><path d="M0 38c18-16 34 10 52-5 15-13 28 3 44-8v39H0z" fill="#16a34a"/><path d="m24 36 9-17 9 17z" fill="#166534"/></svg>"##,
            ),
            (
                "bob",
                false,
                "violet-night.svg",
                br##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 96 64" role="img" aria-label="A violet night"><rect width="96" height="64" fill="#ede9fe"/><path d="M0 44h96v20H0z" fill="#7c3aed"/><path d="m18 35 12-18 12 18 12-12 12 12 12-18 12 18z" fill="#4c1d95"/></svg>"##,
            ),
        ];
        for (fixture, (username, operator, filename, bytes)) in
            SANDBOX_USER_FIXTURES.iter().zip(expected_assets)
        {
            assert_eq!(fixture.username, username);
            assert_eq!(fixture.operator, operator);
            assert_eq!(fixture.media.filename, filename);
            assert_eq!(fixture.media.bytes, bytes);

            let asset_url = format!("/media/upload/{username}/{filename}")
                .parse::<RootRelativeUrl>()
                .expect("fixed canonical fixture URL");
            let [markdown, org] = fixture.materialize_curated_posts(&asset_url);
            assert_eq!(markdown.author, username);
            assert_eq!(markdown.format, PostFormat::Markdown);
            assert_eq!(org.author, username);
            assert_eq!(org.format, PostFormat::Org);
            assert!(markdown.published_at.is_none());
            assert!(org.published_at.is_none());
            for body in [&markdown.body, &org.body] {
                assert_eq!(body.matches(asset_url.as_ref()).count(), 1);
                assert!(!body.contains('<'));
                assert!(!body.contains("://"));
            }
            assert!(markdown.body.contains("# "));
            assert!(markdown.body.contains("**"));
            assert!(markdown.body.contains("](/notes/"));
            assert!(markdown.body.contains("\n```text\n"));
            assert!(markdown.body.contains("\n| --- |"));
            assert!(markdown.body.contains("\n!["));
            assert!(
                markdown.body.contains("\n- ") || markdown.body.contains("\n1. "),
                "Markdown fixture has a native list"
            );
            assert!(org.body.contains("* "));
            assert!(org.body.contains(" /") || org.body.contains("\nA *"));
            assert!(org.body.contains("[[/notes/"));
            assert!(org.body.contains("\n#+begin_src text\n"));
            assert!(org.body.contains("\n|"));
            assert!(org.body.contains("\n[[/media/"));
            assert!(
                org.body.contains("\n- ") || org.body.contains("\n1. "),
                "Org fixture has a native list"
            );
        }
    }

    type ExpectedCuratedPost = (&'static str, &'static str, PostFormat, String);

    fn expected_curated_sources(
        username: &str,
        asset_url: &RootRelativeUrl,
    ) -> [ExpectedCuratedPost; 2] {
        match username {
            "user" => [
                (
                    "A horizon worth keeping",
                    "horizon-worth-keeping",
                    PostFormat::Markdown,
                    format!(
                        "# A horizon worth keeping\n\nA **small observation** can guide a whole day.\n\n[Read the field notes](/notes/horizon).\n\n- Watch the light\n- Keep the useful detail\n\n```text\nhorizon = \"clear\"\n```\n\n| Moment | Choice |\n| --- | --- |\n| Morning | Walk |\n| Evening | Write |\n\n![Blue horizon]({asset_url})"
                    ),
                ),
                (
                    "A calm field note",
                    "calm-field-note",
                    PostFormat::Org,
                    format!(
                        "* A calm field note\n\nA /steady practice/ makes room for better work.\n\n[[/notes/practice][Read the practice note]]\n\n- Name the question\n- Share the answer\n\n#+begin_src text\nanswer = \"kind\"\n#+end_src\n\n| Moment | Choice |\n|---------+--------|\n| Morning | Listen |\n| Evening | Rest   |\n\n#+caption: Blue horizon\n[[{asset_url}]]"
                    ),
                ),
            ],
            "operator" => [
                (
                    "Workshop checks",
                    "workshop-checks",
                    PostFormat::Markdown,
                    format!(
                        "# Workshop checks\n\nA **clear checklist** makes maintenance less surprising.\n\n[Review the runbook](/notes/workshop).\n\n1. Open the bench\n2. Record the result\n\n```text\nstatus = \"ready\"\n```\n\n| Tool | State |\n| --- | --- |\n| Saw | Ready |\n| Lamp | Warm |\n\n![Warm workshop]({asset_url})"
                    ),
                ),
                (
                    "Workshop rhythm",
                    "workshop-rhythm",
                    PostFormat::Org,
                    format!(
                        "* Workshop rhythm\n\nA *shared routine* keeps the room useful.\n\n[[/notes/rhythm][Read the workshop rhythm]]\n\n1. Check the bench\n2. Leave a note\n\n#+begin_src text\nroom = \"open\"\n#+end_src\n\n| Tool | State |\n|------+-------|\n| Saw  | Ready |\n| Lamp | Warm  |\n\n#+caption: Warm workshop\n[[{asset_url}]]"
                    ),
                ),
            ],
            "alice" => [
                (
                    "Field paths",
                    "field-paths",
                    PostFormat::Markdown,
                    format!(
                        "# Field paths\n\nA **patient route** notices what hurried travel misses.\n\n[See the path map](/notes/field-paths).\n\n- Follow the shade\n- Mark the turn\n\n```text\npace = \"slow\"\n```\n\n| Place | Sound |\n| --- | --- |\n| Gate | Birds |\n| Hill | Wind |\n\n![Green field]({asset_url})"
                    ),
                ),
                (
                    "Field margins",
                    "field-margins",
                    PostFormat::Org,
                    format!(
                        "* Field margins\n\nA /careful walk/ gives a place time to speak.\n\n[[/notes/margins][Read the field margin]]\n\n- Follow the shade\n- Mark the turn\n\n#+begin_src text\npace = \"slow\"\n#+end_src\n\n| Place | Sound |\n|-------+-------|\n| Gate  | Birds |\n| Hill  | Wind  |\n\n#+caption: Green field\n[[{asset_url}]]"
                    ),
                ),
            ],
            "bob" => [
                (
                    "Night signals",
                    "night-signals",
                    PostFormat::Markdown,
                    format!(
                        "# Night signals\n\nA **quiet sky** makes a distant signal easier to see.\n\n[Open the signal log](/notes/night-signals).\n\n1. Dim the lamp\n2. Wait for the blink\n\n```text\nsignal = \"seen\"\n```\n\n| Hour | Signal |\n| --- | --- |\n| Nine | Faint |\n| Ten | Clear |\n\n![Violet night]({asset_url})"
                    ),
                ),
                (
                    "Night watch",
                    "night-watch",
                    PostFormat::Org,
                    format!(
                        "* Night watch\n\nA *quiet room* turns waiting into attention.\n\n[[/notes/night-watch][Read the night watch]]\n\n1. Dim the lamp\n2. Wait for the blink\n\n#+begin_src text\nsignal = \"seen\"\n#+end_src\n\n| Hour | Signal |\n|------+--------|\n| Nine | Faint  |\n| Ten  | Clear  |\n\n#+caption: Violet night\n[[{asset_url}]]"
                    ),
                ),
            ],
            _ => unreachable!("sandbox fixtures use only fixed Usernames"),
        }
    }

    #[test]
    fn demo_manifest_pins_exact_curated_sources_and_titles() {
        const CURATED_TITLES: [&str; 8] = [
            "A horizon worth keeping",
            "A calm field note",
            "Workshop checks",
            "Workshop rhythm",
            "Field paths",
            "Field margins",
            "Night signals",
            "Night watch",
        ];
        let asset_urls = SANDBOX_USER_FIXTURES
            .iter()
            .map(|fixture| {
                format!(
                    "/media/upload/{}/{}",
                    fixture.username, fixture.media.filename
                )
                .parse::<RootRelativeUrl>()
                .expect("worked-example URL")
            })
            .collect::<Vec<_>>();
        let asset_urls: [RootRelativeUrl; 4] = asset_urls
            .try_into()
            .expect("worked-example URLs cover every fixture User");

        for (fixture, asset_url) in SANDBOX_USER_FIXTURES.iter().zip(&asset_urls) {
            let actual = fixture.materialize_curated_posts(asset_url);
            let expected = expected_curated_sources(fixture.username, asset_url);
            for (actual, (title, slug, format, body)) in actual.into_iter().zip(expected) {
                assert_eq!(actual.author, fixture.username);
                assert_eq!(actual.title, title);
                assert_eq!(actual.slug, slug);
                assert_eq!(actual.format, format);
                assert_eq!(actual.body, body);
                assert!(actual.published_at.is_none());
            }
        }

        let manifest = sandbox_profile_manifest(
            "2026-09-06T12:34:00Z".parse().expect("fixed anchor"),
            &asset_urls,
        );
        assert_eq!(manifest.len(), 68);
        assert_eq!(
            manifest
                .iter()
                .filter(|post| CURATED_TITLES.contains(&post.title.as_str()))
                .count(),
            8
        );
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
        let asset_urls = SANDBOX_USER_FIXTURES
            .iter()
            .map(|fixture| {
                format!(
                    "/media/upload/{}/{}",
                    fixture.username, fixture.media.filename
                )
                .parse::<RootRelativeUrl>()
                .expect("worked-example URL")
            })
            .collect::<Vec<_>>();
        let asset_urls: [RootRelativeUrl; 4] = asset_urls
            .try_into()
            .expect("worked-example URLs cover every fixture User");
        let mut fixture = sandbox_profile_manifest(anchor, &asset_urls)
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
                PublishedPageRequest {
                    cursor: None,
                    order: common::seed::TimelineOrder::Newest,
                    limit: common::test_support::parse_row_limit("10"),
                },
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
