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

use common::display_name::DisplayName;
use common::ids::{FeedEventId, PostId, UserId};
use common::post_body::PostBody;
use common::post_title::PostTitle;
use common::site::SiteTitle;
use common::slug::Slug;
use common::theme::{PublicThemeSelection, Theme, ThemeImageBindingMode, ThemeImageRole};
use common::time::UtcInstant;
use common::username::Username;
use common::visibility::AudienceTarget;
use host::config_key::SiteConfigKey;
use host::feed::{FeedEventPhase, FeedPath};
use jiff::{Timestamp, ToSpan};
use storage::{
    AppState, OperatorStatus, PostBookkeepingExpectation, PostFormat, PostStorage,
    RenderedPostContent, SiteConfigStorage, ThemeAssetManager, ThemeOwner, ThemeRoleBinding,
    UserStorage, WriteScope, render_post_input, seed_post_input,
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
    let base: String = prefix
        .to_lowercase()
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() { c } else { '-' })
        .collect();
    let base = base.trim_matches('-');
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
    state: &Arc<AppState>,
    username: &str,
    count: usize,
    published: bool,
    prefix: &str,
) -> anyhow::Result<Vec<PostId>> {
    let uname = username
        .parse::<Username>()
        .map_err(|_| anyhow::anyhow!("invalid username: {username}"))?;
    let user = state
        .users
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
    let posts = Arc::clone(&state.posts);
    let outcome = state
        .write_scope
        .run(move |transaction| {
            Box::pin(async move { posts.create_posts(transaction, &inputs).await })
        })
        .await
        .map_err(|error| anyhow::anyhow!("batch seed of {count} posts failed: {error}"))?;

    confirmed_fixture_outcome(outcome, format_args!("batch seed of {count} posts"))
}
async fn author_fixture_theme(
    state: &Arc<AppState>,
    owner: ThemeOwner,
    compiled: &host::theme_package::CompiledThemeRevision,
) -> anyhow::Result<(common::ids::ThemeId, bool)> {
    let existing = state
        .themes
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
                Arc::clone(&state.themes),
                state.write_scope.clone(),
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
    state: &Arc<AppState>,
    storage_path: &Path,
    author_username: &str,
) -> anyhow::Result<()> {
    let author_username = author_username
        .parse::<Username>()
        .map_err(|_| anyhow::anyhow!("invalid username: {author_username}"))?;
    let author = state
        .users
        .get_user_by_username(&author_username)
        .await?
        .ok_or_else(|| anyhow::anyhow!("no such user: {author_username}"))?;
    let owner = ThemeOwner::Author(author.user_id);
    let compiled = storage::seed_theme_fixture::try_compiled_theme_fixture()?;
    let (theme_id, already_published) = author_fixture_theme(state, owner, &compiled).await?;
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
        Arc::clone(&state.themes),
        state.write_scope.clone(),
        Arc::new(storage_path.to_path_buf()),
    );
    if !already_published {
        let publication = manager
            .publish(
                owner,
                theme_id,
                &compiled,
                limits,
                chrono::Utc::now().timestamp(),
            )
            .await?;
        confirmed_fixture_outcome(publication, "publish fixture author theme")?;
    }

    let bindings = [ThemeImageRole::Logo, ThemeImageRole::Header].map(|role| ThemeRoleBinding {
        theme_id,
        role,
        mode: ThemeImageBindingMode::PackagedDefault,
        package_path: None,
        media_user_id: None,
        media_source: None,
        media_digest: None,
        media_filename: None,
        pool_revision: None,
        shuffle_seed: None,
    });
    let themes = Arc::clone(&state.themes);
    let selection = state
        .write_scope
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
    state: &Arc<AppState>,
    author_username: &str,
) -> anyhow::Result<()> {
    let author_username = author_username
        .parse::<Username>()
        .map_err(|_| anyhow::anyhow!("invalid username: {author_username}"))?;
    let author = state
        .users
        .get_user_by_username(&author_username)
        .await?
        .ok_or_else(|| anyhow::anyhow!("no such user: {author_username}"))?;
    let themes = Arc::clone(&state.themes);
    let reset = state
        .write_scope
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
    state: &Arc<AppState>,
    phase: FeedEventPhase,
    count: usize,
) -> anyhow::Result<Vec<FeedEventId>> {
    let mut ids = Vec::with_capacity(count);
    for index in 0..count {
        let feed_path = format!("/~websub-fixture-{index}/feed.rss")
            .parse::<FeedPath>()
            .map_err(|_| anyhow::anyhow!("generated WebSub fixture feed path was invalid"))?;
        let feed_events = Arc::clone(&state.feed_events);
        let diagnostic = format!("fixture {phase:?} failure {index}");
        let id = confirmed_fixture_outcome(
            state
                .write_scope
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
                })?,
            "atomic WebSub dead-letter fixture",
        )?; // cov:ignore — llvm-cov attributes the already-covered outer `?` to its closing span
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
    state: &Arc<AppState>,
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
    let users = Arc::clone(&state.users);
    let outcome = state
        .write_scope
        .run(move |transaction| {
            let users = Arc::clone(&users);
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
    pub body: &'static str,
    /// Source format used by the real renderer.
    pub format: PostFormat,
    /// Exact publication timestamp, or `None` for a draft.
    pub published_at: Option<UtcInstant>,
}
const SANDBOX_TITLE: &str = "Jaunder Sandbox";
const SANDBOX_PASSWORD: &str = "jaunder-dev";
const SANDBOX_USERS: [(&str, bool); 4] = [
    ("user", false),
    ("operator", true),
    ("alice", false),
    ("bob", false),
];
const SHORT_MARKDOWN: &str = "A short sandbox note with one clear idea.";
const MEDIUM_MARKDOWN: &str = "A medium sandbox note has enough detail to make a timeline card feel lived in.\n\nIt remains concise enough to scan.";
const SHORT_ORG: &str = "* A short Org sandbox note\n\nA clear idea in Org.";
const MEDIUM_ORG: &str = "* A medium Org sandbox note\n\nThis fixture has enough detail to exercise the rendered detail surface.\n\n- a stable item\n- another stable item";
const LONG_MARKDOWN: &str = "\
The first paragraph establishes a long-form sandbox post.

The second paragraph adds a concrete observation for a detail view.

The third paragraph keeps the reading rhythm deliberately calm.

The fourth paragraph supplies enough prose for a substantial excerpt.

The fifth paragraph gives the fixture a stable middle section.

The sixth paragraph describes a small decision and its consequence.

The seventh paragraph makes scrolling necessary in an ordinary browser.

The eighth paragraph retains plain Markdown without incidental syntax.

The ninth paragraph lets archive and timeline views meet real length.

The tenth paragraph remains readable without introducing dynamic data.

The eleventh paragraph closes the main thought with a useful detail.

The twelfth paragraph is the stable long-body terminus.";

/// Captures the profile creation instant at minute precision.
#[must_use]
pub fn sandbox_profile_anchor() -> UtcInstant {
    let now = UtcInstant::now().value();
    let minute = now.as_second().div_euclid(60) * 60;
    UtcInstant::from(Timestamp::from_second(minute).map_or(now, std::convert::identity))
}

/// Produces the complete typed fixture manifest for a profile creation anchor.
///
/// The offset sequence is deliberately allocated globally: all 60 published
/// Posts have a distinct timestamp while every author still receives the same
/// 12 Markdown / 3 Org distribution.
#[must_use]
pub fn sandbox_profile_manifest(anchor: UtcInstant) -> Vec<SandboxPost> {
    let mut posts = Vec::with_capacity(68);
    let mut first_offset = 1_i64;
    for &(author, _) in &SANDBOX_USERS {
        for sequence in 1..=15_i64 {
            let (body, format) = match sequence {
                1 => (LONG_MARKDOWN, PostFormat::Markdown),
                2..=12 if sequence % 2 == 0 => (SHORT_MARKDOWN, PostFormat::Markdown),
                2..=12 => (MEDIUM_MARKDOWN, PostFormat::Markdown),
                13 | 15 => (SHORT_ORG, PostFormat::Org),
                14 => (MEDIUM_ORG, PostFormat::Org),
                _ => unreachable!("published sequence is bounded to 1..=15"),
            };
            posts.push(SandboxPost {
                author,
                title: format!("{author} sandbox post {sequence:02}"),
                slug: format!("sandbox-post-{sequence:02}"),
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
            author,
            title: format!("{author} sandbox Markdown draft"),
            slug: "sandbox-markdown-draft".to_owned(),
            body: SHORT_MARKDOWN,
            format: PostFormat::Markdown,
            published_at: None,
        });
        posts.push(SandboxPost {
            author,
            title: format!("{author} sandbox Org draft"),
            slug: "sandbox-org-draft".to_owned(),
            body: SHORT_ORG,
            format: PostFormat::Org,
            published_at: None,
        });
    }
    posts
}

/// Seeds the exact non-idempotent sandbox profile through the normal typed
/// storage write services. All profile rows share one write scope, so a failed
/// creation cannot leave a workspace with a partial fixture.
///
/// # Errors
///
/// Returns an error when password preparation, typed input construction, or the
/// single profile write fails.
pub async fn seed_sandbox_profile(
    site_config: Arc<dyn SiteConfigStorage>,
    users: Arc<dyn UserStorage>,
    posts: Arc<dyn PostStorage>,
    write_scope: WriteScope,
    profile: SandboxProfile,
    anchor: UtcInstant,
) -> anyhow::Result<()> {
    let sandbox_users = match profile {
        SandboxProfile::Standard => &SANDBOX_USERS[..2],
        SandboxProfile::Demo => &SANDBOX_USERS,
    };
    let manifest = match profile {
        SandboxProfile::Standard => Vec::new(),
        SandboxProfile::Demo => sandbox_profile_manifest(anchor),
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
                for ((username_text, operator), password) in
                    sandbox_users.iter().copied().zip(passwords)
                {
                    let username = username_text.parse::<Username>().map_err(|error| {
                        anyhow::anyhow!("invalid fixed sandbox username `{username_text}`: {error}")
                    })?;
                    let role = if operator {
                        OperatorStatus::OPERATOR
                    } else {
                        OperatorStatus::STANDARD
                    };
                    let user_id = users
                        .create_user(transaction, &username, &password, None, role)
                        .await?;
                    user_ids.push((username_text, user_id));
                }
                let mut inputs = Vec::with_capacity(manifest.len());
                for fixture in manifest {
                    let user_id = user_ids
                        .iter()
                        .find_map(|(username, user_id)| {
                            (*username == fixture.author).then_some(*user_id)
                        })
                        .ok_or_else(|| {
                            anyhow::anyhow!(
                                "sandbox manifest author `{}` has no seeded user",
                                fixture.author
                            )
                        })?;
                    let title = fixture
                        .title
                        .parse::<PostTitle>()
                        .map_err(|error| anyhow::anyhow!("invalid sandbox Post title: {error}"))?;
                    let slug = fixture
                        .slug
                        .parse::<Slug>()
                        .map_err(|error| anyhow::anyhow!("invalid sandbox Post slug: {error}"))?;
                    let body = fixture
                        .body
                        .parse::<PostBody>()
                        .map_err(|error| anyhow::anyhow!("invalid sandbox Post body: {error}"))?;
                    inputs.push(render_post_input(RenderedPostContent {
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
                    }));
                }
                let ids = posts.create_posts(transaction, &inputs).await?;
                Ok::<_, anyhow::Error>(ids)
            })
        })
        .await
        .map_err(|error| anyhow::anyhow!("sandbox profile write failed: {error}"))?;
    confirmed_fixture_outcome(outcome, "sandbox profile write")?;
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
    state: &Arc<AppState>,
    user_id: UserId,
    username: &Username,
    is_operator: bool,
    label: Option<&str>,
) -> anyhow::Result<SeedRecord> {
    let label = label
        .unwrap_or(DEFAULT_SEED_LABEL)
        .parse::<common::session_label::SessionLabel>()
        .map_err(|e| anyhow::anyhow!("invalid session label: {e}"))?;
    let sessions = Arc::clone(&state.sessions);
    let outcome = state
        .write_scope
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
    state: &Arc<AppState>,
    username: &str,
    password: &str,
    label: Option<&str>,
) -> anyhow::Result<SeedRecord> {
    let user_id = create_user(state, username, password, None, false).await?;
    let uname = username
        .parse::<Username>()
        .map_err(|_| anyhow::anyhow!("invalid username: {username}"))?;
    session_record(state, user_id, &uname, false, label).await
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
    state: &Arc<AppState>,
    username: &str,
    label: Option<&str>,
) -> anyhow::Result<SeedRecord> {
    let uname = username
        .parse::<Username>()
        .map_err(|_| anyhow::anyhow!("invalid username: {username}"))?;
    let user = state
        .users
        .get_user_by_username(&uname)
        .await?
        .ok_or_else(|| anyhow::anyhow!("no such user: {username}"))?;
    session_record(
        state,
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
        assert_eq!(
            actual
                .iter()
                .filter(|(_, _, _, body, _, _)| body == LONG_MARKDOWN)
                .count(),
            4
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

    async fn assert_loginable(state: &Arc<AppState>, username: &str) {
        let username = username.parse::<Username>().expect("fixed username");
        let password = SANDBOX_PASSWORD
            .parse::<host::password::Password>()
            .expect("fixed password");
        state
            .users
            .prepare_authentication(&username, &password)
            .await
            .expect("fixed fixture credentials authenticate");
    }

    async fn assert_sandbox_users(state: &Arc<AppState>, expected: &[(&str, bool)]) {
        for &(username, operator) in expected {
            let username = username.parse::<Username>().expect("fixed username");
            let user = state
                .users
                .get_user_by_username(&username)
                .await
                .expect("user lookup")
                .expect("fixture user exists");
            assert_eq!(
                user.is_operator,
                if operator {
                    OperatorStatus::OPERATOR
                } else {
                    OperatorStatus::STANDARD
                }
            );
            assert_loginable(state, username.as_ref()).await;
        }
    }

    #[tokio::test]
    async fn standard_profile_has_only_its_explicit_configuration_and_loginable_users() {
        let test_support::TestEnv { state, base: _base } =
            test_support::Backend::Sqlite.setup().pristine().await;
        let anchor = "2026-09-06T12:34:00Z"
            .parse::<UtcInstant>()
            .expect("fixed anchor");

        seed_sandbox_profile(
            Arc::clone(&state.site_config),
            Arc::clone(&state.users),
            Arc::clone(&state.posts),
            state.write_scope.clone(),
            SandboxProfile::Standard,
            anchor,
        )
        .await
        .expect("standard profile seeds");

        assert_eq!(
            state.site_config.list().await.expect("site config list"),
            vec![("site.title".to_owned(), SANDBOX_TITLE.to_owned())]
        );
        assert!(
            state
                .site_config
                .get_raw(SiteConfigKey::SiteBaseUrl)
                .await
                .expect("base URL lookup")
                .is_none()
        );
        assert!(
            state
                .site_config
                .get_raw(SiteConfigKey::SiteRegistrationPolicy)
                .await
                .expect("registration policy lookup")
                .is_none()
        );
        assert_sandbox_users(&state, &SANDBOX_USERS[..2]).await;
        for &(username, _) in &SANDBOX_USERS[2..] {
            assert!(
                state
                    .users
                    .get_user_by_username(&username.parse().expect("fixed username"))
                    .await
                    .expect("user lookup")
                    .is_none()
            );
        }
        for &(username, _) in &SANDBOX_USERS[..2] {
            let user = state
                .users
                .get_user_by_username(&username.parse().expect("fixed username"))
                .await
                .expect("user lookup")
                .expect("fixture user exists");
            assert!(
                state
                    .posts
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

    #[tokio::test]
    async fn demo_profile_matches_the_typed_manifest_and_rounded_anchor() {
        let test_support::TestEnv { state, base: _base } =
            test_support::Backend::Sqlite.setup().pristine().await;
        let anchor = "2026-09-06T12:34:00Z"
            .parse::<UtcInstant>()
            .expect("fixed minute anchor");
        let expected = sandbox_profile_manifest(anchor);

        seed_sandbox_profile(
            Arc::clone(&state.site_config),
            Arc::clone(&state.users),
            Arc::clone(&state.posts),
            state.write_scope.clone(),
            SandboxProfile::Demo,
            anchor,
        )
        .await
        .expect("demo profile seeds");

        assert_eq!(
            state.site_config.list().await.expect("site config list"),
            vec![("site.title".to_owned(), SANDBOX_TITLE.to_owned())]
        );
        assert_sandbox_users(&state, &SANDBOX_USERS).await;

        let mut actual = Vec::new();
        for &(username, _) in &SANDBOX_USERS {
            let user = state
                .users
                .get_user_by_username(&username.parse().expect("fixed username"))
                .await
                .expect("user lookup")
                .expect("fixture user exists");
            actual.extend(
                state
                    .posts
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
        let mut expected = expected
            .into_iter()
            .map(|fixture| {
                (
                    fixture.author.to_owned(),
                    fixture.title,
                    fixture.slug,
                    fixture.body.to_owned(),
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
    fn seed_slug_is_slug_safe() {
        assert_eq!(seed_slug("Timeline Post", 0), "timeline-post-0");
        assert_eq!(seed_slug("Home Feed Mine", 12), "home-feed-mine-12");
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
    use storage::test_support;

    #[tokio::test]
    async fn seeds_public_published_posts_visible_to_a_non_author() {
        let test_support::TestEnv { state, base: _base } =
            test_support::Backend::Sqlite.setup().await;
        let user = test_support::SeedUser::new().seed(&state).await;

        let ids = seed_posts_for_user(&state, &user.username, 3, true, "Timeline Post")
            .await
            .expect("seed ok");
        assert_eq!(ids.len(), 3);

        // The point of the tool: seeded posts are Public + published, so an
        // Anonymous (non-author) viewer sees all three. A bare `posts` insert
        // with no `post_audiences` row would be private and this would return 0
        // — this asserts the tool seeds a *timeline-visible* post, not just a row.
        let page = state
            .posts
            .list_published_by_user(
                &user.username,
                None,
                common::test_support::parse_row_limit("10"),
                &common::visibility::ViewerIdentity::Anonymous,
                common::time::UtcInstant::now(),
            )
            .await
            .expect("list ok");
        assert_eq!(page.len(), 3);
    }

    #[tokio::test]
    async fn publishes_and_resets_the_author_theme_fixture() {
        let test_support::TestEnv { state, base: _base } =
            test_support::Backend::Sqlite.setup().await;
        let user = test_support::SeedUser::new().seed(&state).await;
        let storage = tempfile::TempDir::new().expect("temporary storage");
        let compiled =
            storage::seed_theme_fixture::try_compiled_theme_fixture().expect("valid theme fixture");
        let site_theme = storage::seed_theme_fixture::try_create_theme(
            Arc::clone(&state.themes),
            state.write_scope.clone(),
            ThemeOwner::Site,
            &compiled,
        )
        .await
        .expect("site fixture theme");
        let manager = ThemeAssetManager::new(
            Arc::clone(&state.themes),
            state.write_scope.clone(),
            Arc::new(storage.path().to_path_buf()),
        );
        let site_publication = manager
            .publish(
                ThemeOwner::Site,
                site_theme,
                &compiled,
                storage::seed_theme_fixture::theme_quota_limits(i64::MAX),
                chrono::Utc::now().timestamp(),
            )
            .await
            .expect("site fixture publishes");
        confirmed_fixture_outcome(site_publication, "publish site fixture")
            .expect("site fixture commit confirmed");

        seed_published_author_theme(&state, storage.path(), user.username.as_ref())
            .await
            .expect("theme fixture publishes");
        reset_author_theme_fixture(&state, user.username.as_ref())
            .await
            .expect("theme fixture resets");
    }

    #[tokio::test]
    async fn rejects_a_prefix_that_cannot_form_a_valid_slug() {
        let test_support::TestEnv { state, base: _base } =
            test_support::Backend::Sqlite.setup().await;
        let user = test_support::SeedUser::new().seed(&state).await;

        // A prefix with no alphanumerics collapses to an empty base, so the slug
        // would begin with '-' and fail `Slug` parsing — surfaced as an error
        // (not a panic) before any post is persisted.
        let err = seed_posts_for_user(&state, &user.username, 1, false, "***")
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
        let test_support::TestEnv { state, base: _base } =
            test_support::Backend::Sqlite.setup().await;

        for phase in [FeedEventPhase::Regeneration, FeedEventPhase::Publication] {
            let ids = seed_dead_letters(&state, phase, 2).await.expect("seed ok");
            assert_eq!(ids.len(), 2);

            let page = state
                .feed_events
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
        let test_support::TestEnv { state, base } = test_support::Backend::Sqlite.setup().await;
        base.close_pool().await;

        let error = seed_dead_letters(&state, FeedEventPhase::Regeneration, 1)
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
        let test_support::TestEnv { state, base: _base } =
            test_support::Backend::Sqlite.setup().await;

        let id = create_user(&state, "testoperator", "testpassword123", None, true)
            .await
            .expect("create ok");

        let u = state
            .users
            .get_user_by_username(&"testoperator".parse().unwrap())
            .await
            .expect("lookup ok")
            .expect("user exists");
        assert_eq!(u.user_id, id);
        assert_eq!(u.is_operator, OperatorStatus::OPERATOR);

        // A freshly-init'd DB has a per-user uniqueness constraint, so a second
        // create with the same username surfaces as an error (no upsert).
        create_user(&state, "testoperator", "testpassword123", None, false)
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
        state: &Arc<AppState>,
        token: &RawToken,
    ) -> anyhow::Result<storage::SessionRecord> {
        let token = token.clone();
        let sessions = Arc::clone(&state.sessions);
        let outcome = state
            .write_scope
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
        let test_support::TestEnv { state, base: _base } =
            test_support::Backend::Sqlite.setup().await;

        let record = seed_user(&state, "alice", "password123", None)
            .await
            .expect("seed ok");

        // The cookie's token authenticates and resolves to the seeded user.
        let token = cookie_token(&record);
        let session = authenticate_session(&state, &token)
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
        let sessions = state
            .sessions
            .list_sessions(session.user_id)
            .await
            .expect("list ok");
        assert_eq!(sessions.len(), 1);
        assert_eq!(sessions[0].label, "E2E seed");
    }

    #[tokio::test]
    async fn seed_user_honours_an_explicit_label() {
        let test_support::TestEnv { state, base: _base } =
            test_support::Backend::Sqlite.setup().await;

        let record = seed_user(&state, "alice", "password123", Some("CI bot"))
            .await
            .expect("seed ok");
        let sessions = state
            .sessions
            .list_sessions(UserId::from(record.user_id))
            .await
            .expect("list ok");
        assert_eq!(sessions[0].label, "CI bot");
    }

    #[tokio::test]
    async fn create_session_for_user_reflects_the_operator_flag() {
        let test_support::TestEnv { state, base: _base } =
            test_support::Backend::Sqlite.setup().await;
        create_user(&state, "testoperator", "testpassword123", None, true)
            .await
            .expect("create ok");

        let record = create_session_for_user(&state, "testoperator", None)
            .await
            .expect("session ok");
        let marker = decode_marker(&record.marker).expect("marker decodes");
        assert!(marker.is_operator, "operator user's marker must say so");
        let token = cookie_token(&record);
        authenticate_session(&state, &token)
            .await
            .expect("token authenticates");
    }

    #[tokio::test]
    async fn create_session_for_user_unknown_username_errors() {
        let test_support::TestEnv { state, base: _base } =
            test_support::Backend::Sqlite.setup().await;
        create_session_for_user(&state, "ghost", None)
            .await
            .expect_err("unknown user should error");
    }

    #[tokio::test]
    async fn seed_user_duplicate_username_errors() {
        let test_support::TestEnv { state, base: _base } =
            test_support::Backend::Sqlite.setup().await;
        seed_user(&state, "alice", "password123", None)
            .await
            .expect("first seed ok");
        seed_user(&state, "alice", "password123", None)
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
