//! Post create/update orchestration over the [`PostStorage`] trait.
//!
//! Validates input, derives titles/slugs (via `common::render`), renders the
//! body, and performs the storage write with slug-collision retry. Shared by
//! the `web` and `server` `AtomPub` front-ends.

use chrono::{DateTime, Utc};
use thiserror::Error;

use crate::{
    CreatePostError, CreatePostInput, PostFormat, PostRecord, PostStorage, UpdatePostError,
    UpdatePostInput,
};
use common::ids::{ChannelId, PostId, UserId};
use common::post_body::PostBody;
use common::post_title::PostTitle;
use common::render::{derive_post_metadata, render};
use common::slug::{slugify_title, InvalidSlug, Slug};
use common::visibility::AudienceTarget;

// ---------------------------------------------------------------------------
// Orchestration helpers
// ---------------------------------------------------------------------------

/// The raw, unrendered fields of a post to create. Bundles the create inputs so
/// [`create_rendered_post`] and [`render_post_input`] stay under the argument
/// limit and share one named shape at every call site.
pub struct RenderedPostContent {
    /// Author of the new post.
    pub user_id: UserId,
    /// Explicit title, or `None`.
    pub title: Option<PostTitle>,
    /// Slug for the new post.
    pub slug: Slug,
    /// Raw post body in `format`.
    pub body: PostBody,
    /// Markup format of `body`.
    pub format: PostFormat,
    /// Publication timestamp, or `None` for a draft.
    pub published_at: Option<DateTime<Utc>>,
    /// Optional summary/excerpt.
    pub summary: Option<String>,
    /// Audience targeting for the new post.
    pub audiences: Vec<AudienceTarget>,
    /// Owned idempotency key to register with the post, or `None`.
    pub idempotency_key: Option<String>,
}

/// Renders `body` according to `format` and creates the post via storage.
///
/// # Errors
///
/// Returns `Err(CreatePostError)` if the storage layer returns an error.
pub async fn create_rendered_post(
    storage: &dyn PostStorage,
    content: RenderedPostContent,
) -> Result<PostId, CreatePostError> {
    let input = render_post_input(content);
    storage.create_post(&input).await
}

/// Renders `body` per `format` and assembles the [`CreatePostInput`] without
/// writing it. Shared by [`create_rendered_post`] (write one) and the batch
/// seeders (collect many), so the render-and-assemble recipe lives in one place.
#[must_use]
pub fn render_post_input(content: RenderedPostContent) -> CreatePostInput {
    let RenderedPostContent {
        user_id,
        title,
        slug,
        body,
        format,
        published_at,
        summary,
        audiences,
        idempotency_key,
    } = content;
    let rendered_html = render(&body, &format);
    CreatePostInput {
        user_id,
        title,
        slug,
        body,
        format,
        rendered_html,
        published_at,
        summary,
        audiences,
        idempotency_key,
    }
}

/// The single definition of "a timeline-visible seeded post", as data: a public,
/// Markdown-rendered post, published now iff `published` — the Public audience
/// plus rendered HTML that make it timeline-visible. Returns the
/// [`CreatePostInput`] instead of writing it, so both seeders
/// (`storage::test_support::seed_posts` in-process and the `test-support`
/// binary's `seed_posts_for_user` out-of-process) build a `Vec` and write them
/// in one batched transaction via [`PostStorage::create_posts`]. Gated so a
/// normal `storage` build never compiles it, yet the `test-support` binary
/// reaches it via the lightweight `seed-posts` feature (no
/// `tempfile`/`rstest_reuse`).
#[cfg(any(test, feature = "seed-posts"))]
#[must_use]
pub fn seed_post_input(
    user_id: UserId,
    slug: Slug,
    body: PostBody,
    published: bool,
) -> CreatePostInput {
    render_post_input(RenderedPostContent {
        user_id,
        title: None,
        slug,
        body,
        format: PostFormat::Markdown,
        published_at: published.then(Utc::now),
        summary: None,
        audiences: vec![AudienceTarget::Public],
        idempotency_key: None,
    })
}

/// The raw, unrendered fields of a post edit. Bundles the update inputs so
/// [`update_rendered_post`] stays under the argument limit and names its shape
/// at every call site.
pub struct RenderedPostUpdate {
    /// Post being edited.
    pub post_id: PostId,
    /// User performing the edit (ownership is checked in storage).
    pub editor_user_id: UserId,
    /// Explicit title, or `None`.
    pub title: Option<PostTitle>,
    /// New slug for the post.
    pub slug: Slug,
    /// Raw post body in `format`.
    pub body: PostBody,
    /// Markup format of `body`.
    pub format: PostFormat,
    /// What this update does to the post's publication state.
    pub publish: PublishUpdate,
    /// Optional summary/excerpt.
    pub summary: Option<String>,
    /// Audience targeting for the post (replaces its existing rows).
    pub audiences: Vec<AudienceTarget>,
}

/// Renders `body` according to `format` and updates the post via storage.
///
/// # Errors
///
/// Returns `Err(UpdatePostError)` if the storage layer returns an error.
pub async fn update_rendered_post(
    storage: &dyn PostStorage,
    update: RenderedPostUpdate,
) -> Result<PostRecord, UpdatePostError> {
    let RenderedPostUpdate {
        post_id,
        editor_user_id,
        title,
        slug,
        body,
        format,
        publish,
        summary,
        audiences,
    } = update;
    let rendered_html = render(&body, &format);
    let (unpublish, explicit_published_at) = publish.into_inputs();
    let input = UpdatePostInput {
        title,
        slug,
        body,
        format,
        rendered_html,
        unpublish,
        explicit_published_at,
        summary,
        audiences,
    };
    storage.update_post(post_id, editor_user_id, &input).await
}

// ---------------------------------------------------------------------------
// High-level post-update orchestration
// ---------------------------------------------------------------------------

/// Errors that can occur during a high-level post update.
#[derive(Debug, Error)]
pub enum PerformUpdateError {
    #[error("post body or title is required")]
    EmptyPost,
    #[error("invalid slug")]
    InvalidSlug,
    #[error("post not found")]
    NotFound,
    #[error("not authorized")]
    Unauthorized,
    #[error("storage error: {0}")]
    Storage(#[source] sqlx::Error),
}

impl From<UpdatePostError> for PerformUpdateError {
    fn from(e: UpdatePostError) -> Self {
        match e {
            UpdatePostError::NotFound => Self::NotFound,
            UpdatePostError::Unauthorized => Self::Unauthorized,
            UpdatePostError::Internal(e) => Self::Storage(e),
        }
    }
}

impl From<PerformUpdateError> for host::error::InternalError {
    /// Reproduces the former `web::posts::server::perform_update_error`
    /// `(kind, class, public_message)`: empty/invalid-slug are client validation
    /// errors, not-found/unauthorized mask as a 404, storage is a masked storage
    /// failure. The validation arms carry the typed `PerformUpdateError` as the
    /// operator-side source instead of flattening it (A19).
    fn from(error: PerformUpdateError) -> Self {
        use host::error::InternalError;
        match error {
            PerformUpdateError::EmptyPost | PerformUpdateError::InvalidSlug => {
                InternalError::validation_source(error.to_string(), error)
            }
            PerformUpdateError::NotFound | PerformUpdateError::Unauthorized => {
                InternalError::not_found("Post")
            }
            PerformUpdateError::Storage(e) => InternalError::storage(e),
        }
    }
}

/// What an update does to a post's publication state.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PublishUpdate {
    /// Clear `published_at` back to NULL (draft / unschedule).
    Unpublish,
    /// Publish. `at = Some(t)` sets `published_at = t` (future = scheduled,
    /// past = backdated-live). `at = None` keeps an existing timestamp or
    /// stamps `now` for a previously-unpublished post.
    Publish { at: Option<DateTime<Utc>> },
}

impl PublishUpdate {
    /// Splits the publication verb into the `(unpublish, explicit_published_at)`
    /// pair the dialect `update_post` SQL binds. `unpublish` clears the
    /// timestamp; `explicit_published_at` is an exact instant to store; with
    /// both inert the SQL keeps any existing timestamp (or stamps `now`).
    #[must_use]
    fn into_inputs(self) -> (bool, Option<DateTime<Utc>>) {
        match self {
            Self::Unpublish => (true, None),
            Self::Publish { at } => (false, at),
        }
    }
}

/// Raw, front-end-supplied inputs to [`perform_post_update`].
///
/// Grouping these into a struct keeps the easy-to-transpose pair
/// (`title: Option<&str>` / `slug_override: Option<&Slug>`) named at every call site.
pub struct PostUpdate<'a> {
    /// Post being edited.
    pub post_id: PostId,
    /// User performing the edit (ownership is checked in storage).
    pub editor_user_id: UserId,
    /// Raw post body in `format`.
    pub body: PostBody,
    /// Explicit title, or `None` to derive one from the body.
    pub title: Option<&'a str>,
    /// Markup format of `body`.
    pub format: PostFormat,
    /// Explicit slug (already validated at the wire/CLI boundary), or `None` to
    /// derive one from the title/body.
    pub slug_override: Option<&'a Slug>,
    /// What this update does to the post's publication state.
    pub publish: PublishUpdate,
    /// Optional summary/excerpt.
    pub summary: Option<String>,
    /// Audience targeting for the post (replaces its existing rows). An empty
    /// vec (or `[Private]`) makes the post author-only.
    pub audiences: Vec<AudienceTarget>,
}

/// Validates inputs, computes the slug, renders the body, and atomically
/// updates the post via storage.
///
/// The storage layer freezes the slug if the post is already published.
/// Ownership and deletion checks are also performed atomically in storage.
///
/// # Errors
///
/// Returns `Err(PerformUpdateError)` if rendering fails or the storage layer returns an error.
pub async fn perform_post_update(
    storage: &dyn PostStorage,
    input: PostUpdate<'_>,
) -> Result<PostRecord, PerformUpdateError> {
    let PostUpdate {
        post_id,
        editor_user_id,
        body,
        title,
        format,
        slug_override,
        publish,
        summary,
        audiences,
    } = input;
    let metadata =
        derive_post_metadata(title, &body, &format).ok_or(PerformUpdateError::EmptyPost)?;

    // Derive the title from the *original* body above, then canonicalize the stored
    // Org body (ADR-0024): strip the title-source line, keep everything else. Web and
    // AtomPub thus converge on one stored body. Non-Org bodies are stored verbatim.
    let body: PostBody = if matches!(format, PostFormat::Org) {
        common::render::canonicalize_org_body(&body).into()
    } else {
        body
    };

    let slug = match slug_override {
        // Pre-validated at the boundary (wire/CLI); updates keep the slug as-is,
        // no collision dedup.
        Some(slug) => slug.clone(),
        None => slugify_title(&metadata.slug_seed)
            .parse::<Slug>()
            .map_err(|_| PerformUpdateError::InvalidSlug)?,
    };

    let rendered_html = render(&body, &format);
    let (unpublish, explicit_published_at) = publish.into_inputs();
    let input = UpdatePostInput {
        title: metadata.title,
        slug,
        body,
        format,
        rendered_html,
        unpublish,
        explicit_published_at,
        summary,
        audiences,
    };
    storage
        .update_post(post_id, editor_user_id, &input)
        .await
        .map_err(PerformUpdateError::from)
}

// ---------------------------------------------------------------------------
// High-level post-creation orchestration
// ---------------------------------------------------------------------------

/// Errors that can occur during high-level post creation.
#[derive(Debug, Error)]
pub enum PerformCreationError {
    #[error("post body is required")]
    EmptyPost,
    #[error(transparent)]
    InvalidSlug(#[from] InvalidSlug),
    #[error("unable to allocate a unique slug after {0} attempts")]
    Exhausted(usize),
    #[error("created post not found")]
    CreatedNotFound,
    /// The idempotency key was already used to create a post for this user; the
    /// create is a duplicate and no new post was written.
    #[error("idempotency key already used for this user")]
    IdempotencyConflict,
    #[error("storage error: {0}")]
    Storage(#[source] sqlx::Error),
}

impl From<PerformCreationError> for host::error::InternalError {
    /// Reproduces the former `web::posts::server::perform_creation_error`
    /// `(kind, class, public_message)`. The invalid-slug arm carries the typed
    /// error as the operator-side source instead of flattening it (A19).
    fn from(error: PerformCreationError) -> Self {
        use host::error::InternalError;
        match error {
            PerformCreationError::EmptyPost => InternalError::validation("post body is required"),
            PerformCreationError::InvalidSlug(_) => {
                InternalError::validation_source(error.to_string(), error)
            }
            // Carry the typed error as the operator source (its `Display` renders the real
            // attempt count) rather than a hardcoded literal that lies when the retry bound
            // isn't 100. Wire projection is unchanged (kind `Internal` → "server operation failed").
            //
            // `IdempotencyConflict` is unreachable in practice — the AtomPub handler
            // intercepts the conflict and returns the original post as `200` before this
            // conversion — but shares the same internal-failure projection.
            PerformCreationError::Exhausted(_) | PerformCreationError::IdempotencyConflict => {
                InternalError::server(error)
            }
            PerformCreationError::CreatedNotFound => {
                InternalError::server_message("created post not found")
            }
            PerformCreationError::Storage(e) => InternalError::storage(e),
        }
    }
}

/// Generates a unique slug attempt using a suffix for attempts > 0.
///
/// # Errors
///
/// Returns `Err(InvalidSlug)` if the suffixed candidate is not a valid `Slug`. By
/// construction the base is truncated to keep the candidate within
/// `MAX_SLUG_CHARS`, so this is not expected in practice; attempt 0 (the seed) is
/// always valid.
pub fn candidate_slug(slug_seed: &Slug, attempt: usize) -> Result<Slug, InvalidSlug> {
    if attempt == 0 {
        return Ok(slug_seed.clone()); // already a valid Slug (≤ MAX_SLUG_CHARS)
    }
    // Keep the suffixed candidate within the slug length cap: a seed already at
    // the cap plus "-{n}" would otherwise exceed it and be rejected by from_str.
    let suffix = format!("-{}", attempt + 1);
    let max_base = common::slug::MAX_SLUG_CHARS.saturating_sub(suffix.chars().count());
    let base: String = slug_seed.chars().take(max_base).collect();
    // Single validity chokepoint: funnel the suffixed candidate through from_str.
    format!("{}{suffix}", base.trim_end_matches('-')).parse()
}

/// Raw, front-end-supplied inputs to [`perform_post_creation`].
///
/// Grouping these into a struct keeps the easy-to-transpose pair
/// (`title: Option<&str>` / `slug_override: Option<&Slug>`) named at every call site.
pub struct PostCreation<'a> {
    /// Author of the new post.
    pub user_id: UserId,
    /// Raw post body in `format`.
    pub body: PostBody,
    /// Explicit title, or `None` to derive one from the body.
    pub title: Option<&'a str>,
    /// Markup format of `body`.
    pub format: PostFormat,
    /// Explicit slug (already validated at the wire/CLI boundary), or `None` to
    /// derive one from the title/body.
    pub slug_override: Option<&'a Slug>,
    /// Publication timestamp, or `None` to create as a draft.
    pub published_at: Option<DateTime<Utc>>,
    /// Maximum slug-collision retries before giving up.
    pub max_attempts: usize,
    /// Optional summary/excerpt.
    pub summary: Option<String>,
    /// Audience targeting for the new post. An empty vec (or `[Private]`) makes
    /// the post author-only.
    pub audiences: Vec<AudienceTarget>,
    /// Client-supplied idempotency key (already trimmed / non-empty), or `None`
    /// to create without deduplication.
    pub idempotency_key: Option<&'a str>,
}

/// Validates inputs, computes the slug, renders the body, and atomically
/// creates the post in storage, retrying on slug collision.
///
/// # Errors
///
/// Returns `Err(PerformCreationError)` if slug validation fails, attempts to
/// find a unique slug are exhausted, or storage fails.
pub async fn perform_post_creation(
    storage: &dyn PostStorage,
    input: PostCreation<'_>,
) -> Result<PostRecord, PerformCreationError> {
    let PostCreation {
        user_id,
        body,
        title,
        format,
        slug_override,
        published_at,
        max_attempts,
        summary,
        audiences,
        idempotency_key,
    } = input;
    let metadata =
        derive_post_metadata(title, &body, &format).ok_or(PerformCreationError::EmptyPost)?;

    // Derive the title from the *original* body above, then canonicalize the stored
    // Org body (ADR-0024): strip the title-source line, keep everything else. Web and
    // AtomPub thus converge on one stored body. Non-Org bodies are stored verbatim.
    let body: PostBody = if matches!(format, PostFormat::Org) {
        common::render::canonicalize_org_body(&body).into()
    } else {
        body
    };

    let slug_seed: Slug = match slug_override {
        // Pre-validated at the boundary (wire/CLI); a valid override is still fed
        // through the collision-suffix generator below for uniqueness.
        Some(slug) => slug.clone(),
        // slugify_title never fails, but funnel it through from_str (the single
        // chokepoint) rather than bypass-constructing a Slug.
        None => slugify_title(&metadata.slug_seed)
            .parse()
            .map_err(PerformCreationError::InvalidSlug)?,
    };

    for attempt in 0..max_attempts {
        let slug =
            candidate_slug(&slug_seed, attempt).map_err(PerformCreationError::InvalidSlug)?;

        match create_rendered_post(
            storage,
            RenderedPostContent {
                user_id,
                title: metadata.title.clone(),
                slug,
                body: body.clone(),
                format: format.clone(),
                published_at,
                summary: summary.clone(),
                audiences: audiences.clone(),
                idempotency_key: idempotency_key.map(str::to_owned),
            },
        )
        .await
        {
            Ok(post_id) => {
                // Re-read as the author so the fetch succeeds regardless of the
                // post's targeting (a private/subscribers/named post is invisible
                // to an Anonymous viewer). The author branch of the resolution
                // filter keys on `user_id` alone, so the channel id is irrelevant
                // here; `0` is a harmless placeholder.
                let viewer = common::visibility::ViewerIdentity::local(user_id, ChannelId::from(0));
                let record = storage
                    .get_post_by_id(post_id, &viewer)
                    .await
                    .map_err(PerformCreationError::Storage)?
                    .ok_or(PerformCreationError::CreatedNotFound)?;
                return Ok(record);
            }
            Err(CreatePostError::SlugConflict) => {}
            // A duplicate idempotency key is not a slug collision — do not retry;
            // the whole create (post included) rolled back. The caller looks up
            // and returns the original post.
            Err(CreatePostError::IdempotencyConflict) => {
                return Err(PerformCreationError::IdempotencyConflict);
            }
            Err(CreatePostError::Internal(e)) => {
                return Err(PerformCreationError::Storage(e));
            }
        }
    }

    Err(PerformCreationError::Exhausted(max_attempts))
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::{backends, seed_user, Backend};
    use rstest::*;
    use rstest_reuse::*;

    // -- perform_post_creation tests --

    #[apply(backends)]
    #[tokio::test]
    async fn test_perform_post_creation_success(#[case] backend: Backend) {
        let env = backend.setup().await;
        let user_id = seed_user(&env.state).await;
        let storage = &*env.state.posts;
        let record = perform_post_creation(
            storage,
            PostCreation {
                user_id,
                body: "Hello, world!".into(),
                title: None,
                format: PostFormat::Markdown,
                slug_override: None,
                published_at: None,
                max_attempts: 100,
                summary: None,
                audiences: vec![AudienceTarget::Public],
                idempotency_key: None,
            },
        )
        .await
        .unwrap();

        assert_eq!(record.user_id, user_id);
        assert_eq!(record.slug, "hello-world");
        assert_eq!(record.body, "Hello, world!");
        assert_eq!(record.format, PostFormat::Markdown);
        assert!(record.rendered_html.contains("<p>Hello, world!</p>"));
    }

    #[apply(backends)]
    #[tokio::test]
    async fn test_perform_post_creation_uses_explicit_title(#[case] backend: Backend) {
        let env = backend.setup().await;
        let user_id = seed_user(&env.state).await;
        let storage = &*env.state.posts;
        // The body has no heading, so any title must come from the explicit arg,
        // which also seeds the slug.
        let record = perform_post_creation(
            storage,
            PostCreation {
                user_id,
                body: "Body without a heading.".into(),
                title: Some("Explicit Title"),
                format: PostFormat::Markdown,
                slug_override: None,
                published_at: None,
                max_attempts: 100,
                summary: None,
                audiences: vec![AudienceTarget::Public],
                idempotency_key: None,
            },
        )
        .await
        .unwrap();

        assert_eq!(record.title.as_deref(), Some("Explicit Title"));
        assert_eq!(record.slug, "explicit-title");
    }

    #[apply(backends)]
    #[tokio::test]
    async fn test_perform_post_creation_slug_override(#[case] backend: Backend) {
        let env = backend.setup().await;
        let user_id = seed_user(&env.state).await;
        let storage = &*env.state.posts;
        // The override arrives already validated as a `Slug` (the wire/CLI boundary
        // parses it); an invalid override can no longer reach this layer — that
        // rejection now lives at the boundary (web `field_error` + the serde bridge).
        let slug: Slug = "my-custom-slug".parse().unwrap();
        let record = perform_post_creation(
            storage,
            PostCreation {
                user_id,
                body: "Hello, world!".into(),
                title: None,
                format: PostFormat::Markdown,
                slug_override: Some(&slug),
                published_at: None,
                max_attempts: 100,
                summary: None,
                audiences: vec![AudienceTarget::Public],
                idempotency_key: None,
            },
        )
        .await
        .unwrap();

        assert_eq!(record.slug, "my-custom-slug");
    }

    #[apply(backends)]
    #[tokio::test]
    async fn test_perform_post_creation_empty_body(#[case] backend: Backend) {
        let env = backend.setup().await;
        let user_id = seed_user(&env.state).await;
        let storage = &*env.state.posts;
        let err = perform_post_creation(
            storage,
            PostCreation {
                user_id,
                body: "   ".into(),
                title: None,
                format: PostFormat::Markdown,
                slug_override: None,
                published_at: None,
                max_attempts: 100,
                summary: None,
                audiences: vec![AudienceTarget::Public],
                idempotency_key: None,
            },
        )
        .await
        .unwrap_err();

        assert!(matches!(err, PerformCreationError::EmptyPost));
    }

    // guard:no-backend — injects a MockPostStorage whose create_post returns an
    // Internal error; no live database backend
    #[cfg(feature = "test-utils")]
    #[tokio::test]
    async fn test_perform_post_creation_storage_internal_error() {
        // A storage-layer `Internal` error from `create_post` (as opposed to the
        // retryable `SlugConflict`) short-circuits the slug-retry loop into
        // `PerformCreationError::Storage`.
        let mut storage = crate::MockPostStorage::new();
        storage
            .expect_create_post()
            .returning(|_input| Err(CreatePostError::Internal(sqlx::Error::RowNotFound)));

        let err = perform_post_creation(
            &storage,
            PostCreation {
                user_id: UserId::from(1),
                body: "Hello, world!".into(),
                title: None,
                format: PostFormat::Markdown,
                slug_override: None,
                published_at: None,
                max_attempts: 100,
                summary: None,
                audiences: vec![AudienceTarget::Public],
                idempotency_key: None,
            },
        )
        .await
        .unwrap_err();

        assert!(matches!(err, PerformCreationError::Storage(_)));
    }

    #[apply(backends)]
    #[tokio::test]
    async fn test_perform_post_creation_symbol_only_title_falls_back_to_post(
        #[case] backend: Backend,
    ) {
        let env = backend.setup().await;
        let user_id = seed_user(&env.state).await;
        let storage = &*env.state.posts;
        let record = perform_post_creation(
            storage,
            PostCreation {
                user_id,
                body: "!!!".into(),
                title: None,
                format: PostFormat::Markdown,
                slug_override: None,
                published_at: None,
                max_attempts: 100,
                summary: None,
                audiences: vec![AudienceTarget::Public],
                idempotency_key: None,
            },
        )
        .await
        .unwrap();

        // Never hard-fails: a title with no usable characters lands on the
        // synthetic `post` fallback rather than NoSlugFromPost.
        assert_eq!(record.slug, "post");
    }

    #[apply(backends)]
    #[tokio::test]
    async fn test_perform_post_creation_unicode_title_preserves_slug(#[case] backend: Backend) {
        let env = backend.setup().await;
        let user_id = seed_user(&env.state).await;
        let storage = &*env.state.posts;
        let record = perform_post_creation(
            storage,
            PostCreation {
                user_id,
                body: "# 日本語\n\nbody".into(),
                title: None,
                format: PostFormat::Markdown,
                slug_override: None,
                published_at: None,
                max_attempts: 100,
                summary: None,
                audiences: vec![AudienceTarget::Public],
                idempotency_key: None,
            },
        )
        .await
        .unwrap();

        assert_eq!(record.slug, "日本語");
    }

    #[test]
    fn candidate_slug_keeps_suffix_within_cap() {
        use common::slug::{Slug, MAX_SLUG_CHARS};
        // A seed already at the cap: the naive "{seed}-2" would be 82 chars and
        // be rejected by from_str; candidate_slug truncates the base to fit. The
        // seed is a valid Slug, so it is by construction ≤ MAX_SLUG_CHARS.
        let seed: Slug = "a".repeat(MAX_SLUG_CHARS).parse().unwrap();
        // `unwrap` is itself the validity check: candidate_slug parses internally,
        // so a candidate exceeding the cap would fail here.
        let c = candidate_slug(&seed, 1).unwrap();
        assert!(c.chars().count() <= MAX_SLUG_CHARS);
        assert!(c.ends_with("-2"));

        // Truncation that would land on a '-' trims it so no "--" boundary forms:
        // an at-cap seed whose 78th char (the base cutoff for a "-2" suffix) is '-'.
        let seed2: Slug = format!("{}-{}", "a".repeat(77), "b".repeat(2))
            .parse()
            .unwrap();
        let c2 = candidate_slug(&seed2, 1).unwrap();
        assert!(c2.chars().count() <= MAX_SLUG_CHARS);
        assert!(!c2.contains("--"));

        // attempt 0 returns the seed unchanged.
        let hello: Slug = "hello".parse().unwrap();
        assert_eq!(candidate_slug(&hello, 0).unwrap().as_ref(), "hello");
    }

    #[apply(backends)]
    #[tokio::test]
    async fn test_perform_post_creation_slug_conflict_retries(#[case] backend: Backend) {
        let env = backend.setup().await;
        let user_id = seed_user(&env.state).await;
        let storage = &*env.state.posts;

        let r1 = perform_post_creation(
            storage,
            PostCreation {
                user_id,
                body: "Hello, world!".into(),
                title: None,
                format: PostFormat::Markdown,
                slug_override: None,
                published_at: None,
                max_attempts: 100,
                summary: None,
                audiences: vec![AudienceTarget::Public],
                idempotency_key: None,
            },
        )
        .await
        .unwrap();

        let r2 = perform_post_creation(
            storage,
            PostCreation {
                user_id,
                body: "Hello, world!".into(),
                title: None,
                format: PostFormat::Markdown,
                slug_override: None,
                published_at: None,
                max_attempts: 100,
                summary: None,
                audiences: vec![AudienceTarget::Public],
                idempotency_key: None,
            },
        )
        .await
        .unwrap();

        let r3 = perform_post_creation(
            storage,
            PostCreation {
                user_id,
                body: "Hello, world!".into(),
                title: None,
                format: PostFormat::Markdown,
                slug_override: None,
                published_at: None,
                max_attempts: 100,
                summary: None,
                audiences: vec![AudienceTarget::Public],
                idempotency_key: None,
            },
        )
        .await
        .unwrap();

        assert_eq!(r1.slug, "hello-world");
        assert_eq!(r2.slug, "hello-world-2");
        assert_eq!(r3.slug, "hello-world-3");
    }

    #[apply(backends)]
    #[tokio::test]
    async fn test_perform_post_creation_slug_exhaustion(#[case] backend: Backend) {
        let env = backend.setup().await;
        let user_id = seed_user(&env.state).await;
        let storage = &*env.state.posts;

        let r1 = perform_post_creation(
            storage,
            PostCreation {
                user_id,
                body: "Hello, world!".into(),
                title: None,
                format: PostFormat::Markdown,
                slug_override: None,
                published_at: None,
                max_attempts: 2,
                summary: None,
                audiences: vec![AudienceTarget::Public],
                idempotency_key: None,
            },
        )
        .await
        .unwrap();

        let r2 = perform_post_creation(
            storage,
            PostCreation {
                user_id,
                body: "Hello, world!".into(),
                title: None,
                format: PostFormat::Markdown,
                slug_override: None,
                published_at: None,
                max_attempts: 2,
                summary: None,
                audiences: vec![AudienceTarget::Public],
                idempotency_key: None,
            },
        )
        .await
        .unwrap();

        assert_eq!(r1.slug, "hello-world");
        assert_eq!(r2.slug, "hello-world-2");

        let err = perform_post_creation(
            storage,
            PostCreation {
                user_id,
                body: "Hello, world!".into(),
                title: None,
                format: PostFormat::Markdown,
                slug_override: None,
                published_at: None,
                max_attempts: 2,
                summary: None,
                audiences: vec![AudienceTarget::Public],
                idempotency_key: None,
            },
        )
        .await
        .unwrap_err();

        assert!(matches!(err, PerformCreationError::Exhausted(2)));
    }

    #[apply(backends)]
    #[tokio::test]
    async fn test_perform_post_creation_canonicalizes_org_body(#[case] backend: Backend) {
        let env = backend.setup().await;
        let user_id = seed_user(&env.state).await;
        let storage = &*env.state.posts;
        // Title is derived from the original body's #+TITLE:, then the stored body is
        // canonicalized: the #+TITLE: line is stripped while #+FOO: and content stay.
        let record = perform_post_creation(
            storage,
            PostCreation {
                user_id,
                body: "#+TITLE: Hi\n#+FOO: x\n\nHello".into(),
                title: None,
                format: PostFormat::Org,
                slug_override: None,
                published_at: None,
                max_attempts: 100,
                summary: None,
                audiences: vec![AudienceTarget::Public],
                idempotency_key: None,
            },
        )
        .await
        .unwrap();

        assert_eq!(record.title.as_deref(), Some("Hi"));
        assert!(
            !record.body.contains("#+TITLE:"),
            "stored body still has the title header: {:?}",
            record.body
        );
        assert!(record.body.contains("#+FOO: x"), "body: {:?}", record.body);
        assert!(record.body.contains("Hello"), "body: {:?}", record.body);
    }

    #[apply(backends)]
    #[tokio::test]
    async fn test_perform_post_update_canonicalizes_org_body(#[case] backend: Backend) {
        let env = backend.setup().await;
        let user_id = seed_user(&env.state).await;
        let storage = &*env.state.posts;
        // Canonicalization runs on the update path too: a re-saved Org body has its
        // #+TITLE: stripped while an unrecognized #+FOO: and the content survive.
        let created = perform_post_creation(
            storage,
            PostCreation {
                user_id,
                body: "#+TITLE: First\n\noriginal".into(),
                title: None,
                format: PostFormat::Org,
                slug_override: None,
                published_at: None,
                max_attempts: 100,
                summary: None,
                audiences: vec![AudienceTarget::Public],
                idempotency_key: None,
            },
        )
        .await
        .unwrap();

        let record = perform_post_update(
            storage,
            PostUpdate {
                post_id: created.post_id,
                editor_user_id: user_id,
                body: "#+TITLE: Second\n#+FOO: keep\n\nupdated".into(),
                title: None,
                format: PostFormat::Org,
                slug_override: None,
                publish: PublishUpdate::Publish { at: None },
                summary: None,
                audiences: vec![AudienceTarget::Public],
            },
        )
        .await
        .unwrap();

        assert_eq!(record.title.as_deref(), Some("Second"));
        assert!(
            !record.body.contains("#+TITLE:"),
            "stored body still has the title header: {:?}",
            record.body
        );
        assert!(
            record.body.contains("#+FOO: keep"),
            "body: {:?}",
            record.body
        );
        assert!(record.body.contains("updated"), "body: {:?}", record.body);
    }

    #[apply(backends)]
    #[tokio::test]
    async fn test_perform_post_creation_markdown_body_is_not_canonicalized(
        #[case] backend: Backend,
    ) {
        let env = backend.setup().await;
        let user_id = seed_user(&env.state).await;
        let storage = &*env.state.posts;
        // Canonicalization is Org-only: a Markdown body with a leading `# H1` is
        // stored verbatim (the `# H1` is not stripped).
        let record = perform_post_creation(
            storage,
            PostCreation {
                user_id,
                body: "# H1\n\nBody text".into(),
                title: None,
                format: PostFormat::Markdown,
                slug_override: None,
                published_at: None,
                max_attempts: 100,
                summary: None,
                audiences: vec![AudienceTarget::Public],
                idempotency_key: None,
            },
        )
        .await
        .unwrap();

        assert_eq!(record.body, "# H1\n\nBody text");
    }

    #[apply(backends)]
    #[tokio::test]
    async fn test_perform_post_creation_org_title_rendered_once(#[case] backend: Backend) {
        let env = backend.setup().await;
        let user_id = seed_user(&env.state).await;
        let storage = &*env.state.posts;
        // Double-title regression: the title text from the #+TITLE: line must not
        // survive into the stored body (hence rendered_html), so the page chrome's
        // title is the only place it appears. record.title still carries it.
        let record = perform_post_creation(
            storage,
            PostCreation {
                user_id,
                body: "#+TITLE: Distinct Headline\n\nParagraph body".into(),
                title: None,
                format: PostFormat::Org,
                slug_override: None,
                published_at: None,
                max_attempts: 100,
                summary: None,
                audiences: vec![AudienceTarget::Public],
                idempotency_key: None,
            },
        )
        .await
        .unwrap();

        assert_eq!(record.title.as_deref(), Some("Distinct Headline"));
        assert!(
            !record.body.contains("Distinct Headline"),
            "stored body still carries the title text: {:?}",
            record.body
        );
        assert!(
            !record.rendered_html.contains("Distinct Headline"),
            "rendered html double-renders the title: {:?}",
            record.rendered_html
        );
    }

    // -- idempotency-key tests --

    /// Builds a minimal public Markdown [`PostCreation`] carrying `key`, so the
    /// dedup tests vary only the user, body, and key.
    fn creation_with_key<'a>(
        user_id: UserId,
        body: &str,
        key: Option<&'a str>,
    ) -> PostCreation<'a> {
        PostCreation {
            user_id,
            body: body.into(),
            title: None,
            format: PostFormat::Markdown,
            slug_override: None,
            published_at: None,
            max_attempts: 100,
            summary: None,
            audiences: vec![AudienceTarget::Public],
            idempotency_key: key,
        }
    }

    #[apply(backends)]
    #[tokio::test]
    async fn perform_post_creation_dedups_on_idempotency_key(#[case] backend: Backend) {
        let env = backend.setup().await;
        let user_id = seed_user(&env.state).await;
        let storage = &*env.state.posts;

        let first =
            perform_post_creation(storage, creation_with_key(user_id, "First body", Some("k")))
                .await
                .unwrap();

        // A second create with the same (user, key) is a duplicate: the DB unique
        // constraint fires in the create transaction, rolling the whole thing back.
        let err = perform_post_creation(
            storage,
            creation_with_key(user_id, "Second body", Some("k")),
        )
        .await
        .unwrap_err();
        assert!(matches!(err, PerformCreationError::IdempotencyConflict));

        // No second post row committed — the user still has exactly one post.
        let posts = storage
            .list_collection_by_user(user_id, None, 50)
            .await
            .unwrap();
        assert_eq!(posts.len(), 1);
        assert_eq!(posts[0].post_id, first.post_id);
    }

    #[apply(backends)]
    #[tokio::test]
    async fn post_id_for_idempotency_key_maps(#[case] backend: Backend) {
        let env = backend.setup().await;
        let user_id = seed_user(&env.state).await;
        let storage = &*env.state.posts;

        let record = perform_post_creation(storage, creation_with_key(user_id, "Body", Some("k")))
            .await
            .unwrap();

        let mapped = storage
            .post_id_for_idempotency_key(user_id, "k")
            .await
            .unwrap();
        assert_eq!(mapped, Some(record.post_id));

        let missing = storage
            .post_id_for_idempotency_key(user_id, "unknown")
            .await
            .unwrap();
        assert_eq!(missing, None);
    }

    #[apply(backends)]
    #[tokio::test]
    async fn idempotency_key_is_per_user(#[case] backend: Backend) {
        let env = backend.setup().await;
        let user_a = seed_user(&env.state).await;
        let user_b = env
            .state
            .users
            .create_user(
                &"userb".parse().unwrap(),
                &"password123".parse().unwrap(),
                None,
                false,
            )
            .await
            .unwrap();
        let storage = &*env.state.posts;

        // The same key string from two users creates two independent posts.
        let post_a = perform_post_creation(storage, creation_with_key(user_a, "A body", Some("k")))
            .await
            .unwrap();
        let post_b = perform_post_creation(storage, creation_with_key(user_b, "B body", Some("k")))
            .await
            .unwrap();
        assert_ne!(post_a.post_id, post_b.post_id);

        assert_eq!(
            storage
                .post_id_for_idempotency_key(user_a, "k")
                .await
                .unwrap(),
            Some(post_a.post_id)
        );
        assert_eq!(
            storage
                .post_id_for_idempotency_key(user_b, "k")
                .await
                .unwrap(),
            Some(post_b.post_id)
        );
    }

    #[test]
    fn idempotency_conflict_converts_to_internal_error() {
        use host::error::{ErrorKind, InternalError};

        // Covers the otherwise-unreachable `From` arm (the handler intercepts the
        // conflict before this conversion) so the coverage gate stays green.
        let err: InternalError = PerformCreationError::IdempotencyConflict.into();
        assert_eq!(err.kind(), ErrorKind::Internal);
        assert_eq!(err.public_message(), "server operation failed");
    }

    #[test]
    fn test_perform_creation_error_display_and_debug() {
        let err = PerformCreationError::EmptyPost;
        assert_eq!(err.to_string(), "post body is required");
        let debug = format!("{err:?}");
        assert!(debug.contains("EmptyPost"));

        let err = PerformCreationError::InvalidSlug(InvalidSlug);
        assert_eq!(
            err.to_string(),
            "slug must be non-empty, at most 80 characters, and contain only Unicode letters/digits (with their combining marks) and '-'"
        );

        let err = PerformCreationError::Exhausted(10);
        assert_eq!(
            err.to_string(),
            "unable to allocate a unique slug after 10 attempts"
        );

        let err = PerformCreationError::CreatedNotFound;
        assert_eq!(err.to_string(), "created post not found");
    }

    #[test]
    fn perform_creation_error_storage_preserves_sqlx_source() {
        use std::error::Error;
        // §3.1a: Storage carries the sqlx::Error as a typed source (downcastable
        // for classification), not a flattened string.
        let err = PerformCreationError::Storage(sqlx::Error::RowNotFound);
        let source = err.source().expect("Storage should expose a source");
        assert!(source.downcast_ref::<sqlx::Error>().is_some());
    }

    // -- PerformUpdateError tests --

    #[test]
    fn perform_update_error_empty_title_display() {
        let err = PerformUpdateError::EmptyPost;
        assert_eq!(err.to_string(), "post body or title is required");
    }

    #[test]
    fn perform_update_error_invalid_slug_display() {
        let err = PerformUpdateError::InvalidSlug;
        assert_eq!(err.to_string(), "invalid slug");
    }

    #[test]
    fn perform_update_error_not_found_display() {
        let err = PerformUpdateError::NotFound;
        assert_eq!(err.to_string(), "post not found");
    }

    #[test]
    fn perform_update_error_unauthorized_display() {
        let err = PerformUpdateError::Unauthorized;
        assert_eq!(err.to_string(), "not authorized");
    }

    #[test]
    fn perform_update_error_from_update_post_not_found() {
        use crate::UpdatePostError;
        let err: PerformUpdateError = UpdatePostError::NotFound.into();
        assert!(matches!(err, PerformUpdateError::NotFound));
    }

    #[test]
    fn perform_update_error_from_update_post_unauthorized() {
        use crate::UpdatePostError;
        let err: PerformUpdateError = UpdatePostError::Unauthorized.into();
        assert!(matches!(err, PerformUpdateError::Unauthorized));
    }

    #[test]
    fn perform_update_error_debug() {
        let err = PerformUpdateError::EmptyPost;
        let debug = format!("{err:?}");
        assert!(debug.contains("EmptyPost"));
    }

    #[test]
    fn perform_update_error_from_update_post_internal() {
        use crate::UpdatePostError;
        let err: PerformUpdateError = UpdatePostError::Internal(sqlx::Error::RowNotFound).into();
        assert!(matches!(err, PerformUpdateError::Storage(_)));
    }

    // Behavior-preserving translation of the former `web` `perform_update_error`
    // test: each arm maps to the same `(kind, public_message)`.
    #[test]
    fn from_perform_update_error_maps_variants() {
        use host::error::{ErrorKind, InternalError};

        let empty: InternalError = PerformUpdateError::EmptyPost.into();
        assert_eq!(empty.kind(), ErrorKind::Validation);
        assert_eq!(empty.public_message(), "post body or title is required");

        let invalid_slug: InternalError = PerformUpdateError::InvalidSlug.into();
        assert_eq!(invalid_slug.kind(), ErrorKind::Validation);
        assert_eq!(invalid_slug.public_message(), "invalid slug");

        let not_found: InternalError = PerformUpdateError::NotFound.into();
        assert_eq!(not_found.kind(), ErrorKind::NotFound);
        assert_eq!(not_found.public_message(), "Post not found");

        let unauthorized: InternalError = PerformUpdateError::Unauthorized.into();
        assert_eq!(unauthorized.kind(), ErrorKind::NotFound);
        assert_eq!(unauthorized.public_message(), "Post not found");

        let storage: InternalError = PerformUpdateError::Storage(sqlx::Error::PoolClosed).into();
        assert_eq!(storage.kind(), ErrorKind::Storage);
        assert_eq!(storage.public_message(), "storage operation failed");
    }

    // Behavior-preserving translation of the former `web` `perform_creation_error`
    // test: each arm maps to the same `(kind, public_message)`; the invalid-slug
    // arm preserves the typed source.
    #[test]
    fn from_perform_creation_error_maps_variants() {
        use host::error::{ErrorKind, InternalError};

        let empty: InternalError = PerformCreationError::EmptyPost.into();
        assert_eq!(empty.kind(), ErrorKind::Validation);
        assert_eq!(empty.public_message(), "post body is required");

        let invalid_slug: InternalError =
            PerformCreationError::InvalidSlug(common::slug::InvalidSlug).into();
        assert_eq!(invalid_slug.kind(), ErrorKind::Validation);
        assert_eq!(
            invalid_slug.public_message(),
            common::slug::InvalidSlug.to_string()
        );
        // The typed slug error is preserved on the operator side, not flattened.
        assert!(invalid_slug
            .operator_message()
            .contains(&common::slug::InvalidSlug.to_string()));

        let exhausted: InternalError = PerformCreationError::Exhausted(5).into();
        assert_eq!(exhausted.kind(), ErrorKind::Internal);
        assert_eq!(exhausted.public_message(), "server operation failed");

        let created_not_found: InternalError = PerformCreationError::CreatedNotFound.into();
        assert_eq!(created_not_found.kind(), ErrorKind::Internal);
        assert_eq!(
            created_not_found.public_message(),
            "server operation failed"
        );

        let storage: InternalError = PerformCreationError::Storage(sqlx::Error::PoolClosed).into();
        assert_eq!(storage.kind(), ErrorKind::Storage);
        assert_eq!(storage.public_message(), "storage operation failed");
    }
}
