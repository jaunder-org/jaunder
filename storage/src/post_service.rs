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
use common::render::{derive_post_metadata, render};
use common::slug::{slugify_title, InvalidSlug, Slug};
use common::visibility::AudienceTarget;

// ---------------------------------------------------------------------------
// Orchestration helpers
// ---------------------------------------------------------------------------

/// Renders `body` according to `format` and creates the post via storage.
///
/// # Errors
///
/// Returns `Err(CreatePostError)` if the storage layer returns an error.
#[allow(clippy::too_many_arguments)]
pub async fn create_rendered_post(
    storage: &dyn PostStorage,
    user_id: i64,
    title: Option<String>,
    slug: Slug,
    body: String,
    format: PostFormat,
    published_at: Option<DateTime<Utc>>,
    summary: Option<String>,
    audiences: Vec<AudienceTarget>,
) -> Result<i64, CreatePostError> {
    let rendered_html = render(&body, &format);
    let input = CreatePostInput {
        user_id,
        title,
        slug,
        body,
        format,
        rendered_html,
        published_at,
        summary,
        audiences,
    };
    storage.create_post(&input).await
}

/// Renders `body` according to `format` and updates the post via storage.
///
/// # Errors
///
/// Returns `Err(UpdatePostError)` if the storage layer returns an error.
#[allow(clippy::too_many_arguments)]
pub async fn update_rendered_post(
    storage: &dyn PostStorage,
    post_id: i64,
    editor_user_id: i64,
    title: Option<String>,
    slug: Slug,
    body: String,
    format: PostFormat,
    publish: PublishUpdate,
    summary: Option<String>,
    audiences: Vec<AudienceTarget>,
) -> Result<PostRecord, UpdatePostError> {
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
    #[error("post must contain at least one ASCII letter or digit for its slug")]
    NoSlugFromPost,
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
/// (`title`/`slug_override`, both `Option<&str>`) named at every call site.
pub struct PostUpdate<'a> {
    /// Post being edited.
    pub post_id: i64,
    /// User performing the edit (ownership is checked in storage).
    pub editor_user_id: i64,
    /// Raw post body in `format`.
    pub body: String,
    /// Explicit title, or `None` to derive one from the body.
    pub title: Option<&'a str>,
    /// Markup format of `body`.
    pub format: PostFormat,
    /// Explicit slug, or `None` to derive one from the title/body.
    pub slug_override: Option<&'a str>,
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
    let body = if matches!(format, PostFormat::Org) {
        common::render::canonicalize_org_body(&body)
    } else {
        body
    };

    let slug = match slug_override.and_then(common::text::non_empty) {
        Some(raw) => raw
            .to_ascii_lowercase()
            .parse::<Slug>()
            .map_err(|_| PerformUpdateError::InvalidSlug)?,
        None => slugify_title(&metadata.slug_seed)
            .ok_or(PerformUpdateError::NoSlugFromPost)?
            .parse::<Slug>()
            .map_err(|_| PerformUpdateError::NoSlugFromPost)?,
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
    #[error("post must contain at least one ASCII letter or digit for its slug")]
    NoSlugFromPost,
    #[error(transparent)]
    InvalidSlug(#[from] InvalidSlug),
    #[error("unable to allocate a unique slug after {0} attempts")]
    Exhausted(usize),
    #[error("created post not found")]
    CreatedNotFound,
    #[error("storage error: {0}")]
    Storage(#[source] sqlx::Error),
}

/// Generates a unique slug attempt using a suffix for attempts > 0.
#[must_use]
pub fn candidate_slug(slug_seed: &str, attempt: usize) -> String {
    if attempt == 0 {
        slug_seed.to_owned()
    } else {
        format!("{slug_seed}-{}", attempt + 1)
    }
}

/// Raw, front-end-supplied inputs to [`perform_post_creation`].
///
/// Grouping these into a struct keeps the easy-to-transpose pair
/// (`title`/`slug_override`, both `Option<&str>`) named at every call site.
pub struct PostCreation<'a> {
    /// Author of the new post.
    pub user_id: i64,
    /// Raw post body in `format`.
    pub body: String,
    /// Explicit title, or `None` to derive one from the body.
    pub title: Option<&'a str>,
    /// Markup format of `body`.
    pub format: PostFormat,
    /// Explicit slug, or `None` to derive one from the title/body.
    pub slug_override: Option<&'a str>,
    /// Publication timestamp, or `None` to create as a draft.
    pub published_at: Option<DateTime<Utc>>,
    /// Maximum slug-collision retries before giving up.
    pub max_attempts: usize,
    /// Optional summary/excerpt.
    pub summary: Option<String>,
    /// Audience targeting for the new post. An empty vec (or `[Private]`) makes
    /// the post author-only.
    pub audiences: Vec<AudienceTarget>,
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
    } = input;
    let metadata =
        derive_post_metadata(title, &body, &format).ok_or(PerformCreationError::EmptyPost)?;

    // Derive the title from the *original* body above, then canonicalize the stored
    // Org body (ADR-0024): strip the title-source line, keep everything else. Web and
    // AtomPub thus converge on one stored body. Non-Org bodies are stored verbatim.
    let body = if matches!(format, PostFormat::Org) {
        common::render::canonicalize_org_body(&body)
    } else {
        body
    };

    let slug_seed = match slug_override.and_then(common::text::non_empty) {
        Some(raw) => raw
            .to_ascii_lowercase()
            .parse::<Slug>()
            .map_err(PerformCreationError::InvalidSlug)?
            .to_string(),
        None => slugify_title(&metadata.slug_seed).ok_or(PerformCreationError::NoSlugFromPost)?,
    };

    for attempt in 0..max_attempts {
        let slug_string = candidate_slug(&slug_seed, attempt);
        let slug = slug_string
            .parse::<Slug>()
            .map_err(PerformCreationError::InvalidSlug)?;

        match create_rendered_post(
            storage,
            user_id,
            metadata.title.clone(),
            slug,
            body.clone(),
            format.clone(),
            published_at,
            summary.clone(),
            audiences.clone(),
        )
        .await
        {
            Ok(post_id) => {
                // Re-read as the author so the fetch succeeds regardless of the
                // post's targeting (a private/subscribers/named post is invisible
                // to an Anonymous viewer). The author branch of the resolution
                // filter keys on `user_id` alone, so the channel id is irrelevant
                // here; `0` is a harmless placeholder.
                let viewer = common::visibility::ViewerIdentity::local(user_id, 0);
                let record = storage
                    .get_post_by_id(post_id, &viewer)
                    .await
                    .map_err(PerformCreationError::Storage)?
                    .ok_or(PerformCreationError::CreatedNotFound)?;
                return Ok(record);
            }
            Err(CreatePostError::SlugConflict) => {}
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

    // -- perform_post_creation tests --

    async fn setup_test_db() -> (sqlx::SqlitePool, crate::SqlitePostStorage) {
        let pool = sqlx::SqlitePool::connect("sqlite::memory:").await.unwrap();
        sqlx::migrate!("./migrations/sqlite")
            .run(&pool)
            .await
            .unwrap();

        sqlx::query("INSERT INTO users (user_id, username, password_hash, created_at) VALUES (1, 'testuser', 'some_hash', '2026-05-20T12:00:00Z')")
            .execute(&pool)
            .await
            .unwrap();

        let storage = crate::SqlitePostStorage::new(pool.clone());
        (pool, storage)
    }

    #[tokio::test]
    async fn test_perform_post_creation_success() {
        let (_pool, storage) = setup_test_db().await;
        let record = perform_post_creation(
            &storage,
            PostCreation {
                user_id: 1,
                body: "Hello, world!".to_owned(),
                title: None,
                format: PostFormat::Markdown,
                slug_override: None,
                published_at: None,
                max_attempts: 100,
                summary: None,
                audiences: vec![AudienceTarget::Public],
            },
        )
        .await
        .unwrap();

        assert_eq!(record.user_id, 1);
        assert_eq!(record.slug.as_str(), "hello-world");
        assert_eq!(record.body, "Hello, world!");
        assert_eq!(record.format, PostFormat::Markdown);
        assert!(record.rendered_html.contains("<p>Hello, world!</p>"));
    }

    #[tokio::test]
    async fn test_perform_post_creation_uses_explicit_title() {
        let (_pool, storage) = setup_test_db().await;
        // The body has no heading, so any title must come from the explicit arg,
        // which also seeds the slug.
        let record = perform_post_creation(
            &storage,
            PostCreation {
                user_id: 1,
                body: "Body without a heading.".to_owned(),
                title: Some("Explicit Title"),
                format: PostFormat::Markdown,
                slug_override: None,
                published_at: None,
                max_attempts: 100,
                summary: None,
                audiences: vec![AudienceTarget::Public],
            },
        )
        .await
        .unwrap();

        assert_eq!(record.title.as_deref(), Some("Explicit Title"));
        assert_eq!(record.slug.as_str(), "explicit-title");
    }

    #[tokio::test]
    async fn test_perform_post_creation_slug_override() {
        let (_pool, storage) = setup_test_db().await;
        let record = perform_post_creation(
            &storage,
            PostCreation {
                user_id: 1,
                body: "Hello, world!".to_owned(),
                title: None,
                format: PostFormat::Markdown,
                slug_override: Some("my-custom-slug"),
                published_at: None,
                max_attempts: 100,
                summary: None,
                audiences: vec![AudienceTarget::Public],
            },
        )
        .await
        .unwrap();

        assert_eq!(record.slug.as_str(), "my-custom-slug");
    }

    #[tokio::test]
    async fn test_perform_post_creation_invalid_slug_override() {
        let (_pool, storage) = setup_test_db().await;
        let err = perform_post_creation(
            &storage,
            PostCreation {
                user_id: 1,
                body: "Hello, world!".to_owned(),
                title: None,
                format: PostFormat::Markdown,
                slug_override: Some("Invalid Slug!"),
                published_at: None,
                max_attempts: 100,
                summary: None,
                audiences: vec![AudienceTarget::Public],
            },
        )
        .await
        .unwrap_err();

        assert!(matches!(err, PerformCreationError::InvalidSlug(_)));
    }

    #[tokio::test]
    async fn test_perform_post_creation_empty_body() {
        let (_pool, storage) = setup_test_db().await;
        let err = perform_post_creation(
            &storage,
            PostCreation {
                user_id: 1,
                body: "   ".to_owned(),
                title: None,
                format: PostFormat::Markdown,
                slug_override: None,
                published_at: None,
                max_attempts: 100,
                summary: None,
                audiences: vec![AudienceTarget::Public],
            },
        )
        .await
        .unwrap_err();

        assert!(matches!(err, PerformCreationError::EmptyPost));
    }

    #[tokio::test]
    async fn test_perform_post_creation_no_slug_from_body() {
        let (_pool, storage) = setup_test_db().await;
        let err = perform_post_creation(
            &storage,
            PostCreation {
                user_id: 1,
                body: "!!!".to_owned(),
                title: None,
                format: PostFormat::Markdown,
                slug_override: None,
                published_at: None,
                max_attempts: 100,
                summary: None,
                audiences: vec![AudienceTarget::Public],
            },
        )
        .await
        .unwrap_err();

        assert!(matches!(err, PerformCreationError::NoSlugFromPost));
    }

    #[tokio::test]
    async fn test_perform_post_creation_slug_conflict_retries() {
        let (_pool, storage) = setup_test_db().await;

        let r1 = perform_post_creation(
            &storage,
            PostCreation {
                user_id: 1,
                body: "Hello, world!".to_owned(),
                title: None,
                format: PostFormat::Markdown,
                slug_override: None,
                published_at: None,
                max_attempts: 100,
                summary: None,
                audiences: vec![AudienceTarget::Public],
            },
        )
        .await
        .unwrap();

        let r2 = perform_post_creation(
            &storage,
            PostCreation {
                user_id: 1,
                body: "Hello, world!".to_owned(),
                title: None,
                format: PostFormat::Markdown,
                slug_override: None,
                published_at: None,
                max_attempts: 100,
                summary: None,
                audiences: vec![AudienceTarget::Public],
            },
        )
        .await
        .unwrap();

        let r3 = perform_post_creation(
            &storage,
            PostCreation {
                user_id: 1,
                body: "Hello, world!".to_owned(),
                title: None,
                format: PostFormat::Markdown,
                slug_override: None,
                published_at: None,
                max_attempts: 100,
                summary: None,
                audiences: vec![AudienceTarget::Public],
            },
        )
        .await
        .unwrap();

        assert_eq!(r1.slug.as_str(), "hello-world");
        assert_eq!(r2.slug.as_str(), "hello-world-2");
        assert_eq!(r3.slug.as_str(), "hello-world-3");
    }

    #[tokio::test]
    async fn test_perform_post_creation_slug_exhaustion() {
        let (_pool, storage) = setup_test_db().await;

        let r1 = perform_post_creation(
            &storage,
            PostCreation {
                user_id: 1,
                body: "Hello, world!".to_owned(),
                title: None,
                format: PostFormat::Markdown,
                slug_override: None,
                published_at: None,
                max_attempts: 2,
                summary: None,
                audiences: vec![AudienceTarget::Public],
            },
        )
        .await
        .unwrap();

        let r2 = perform_post_creation(
            &storage,
            PostCreation {
                user_id: 1,
                body: "Hello, world!".to_owned(),
                title: None,
                format: PostFormat::Markdown,
                slug_override: None,
                published_at: None,
                max_attempts: 2,
                summary: None,
                audiences: vec![AudienceTarget::Public],
            },
        )
        .await
        .unwrap();

        assert_eq!(r1.slug.as_str(), "hello-world");
        assert_eq!(r2.slug.as_str(), "hello-world-2");

        let err = perform_post_creation(
            &storage,
            PostCreation {
                user_id: 1,
                body: "Hello, world!".to_owned(),
                title: None,
                format: PostFormat::Markdown,
                slug_override: None,
                published_at: None,
                max_attempts: 2,
                summary: None,
                audiences: vec![AudienceTarget::Public],
            },
        )
        .await
        .unwrap_err();

        assert!(matches!(err, PerformCreationError::Exhausted(2)));
    }

    #[tokio::test]
    async fn test_perform_post_creation_canonicalizes_org_body() {
        let (_pool, storage) = setup_test_db().await;
        // Title is derived from the original body's #+TITLE:, then the stored body is
        // canonicalized: the #+TITLE: line is stripped while #+FOO: and content stay.
        let record = perform_post_creation(
            &storage,
            PostCreation {
                user_id: 1,
                body: "#+TITLE: Hi\n#+FOO: x\n\nHello".to_owned(),
                title: None,
                format: PostFormat::Org,
                slug_override: None,
                published_at: None,
                max_attempts: 100,
                summary: None,
                audiences: vec![AudienceTarget::Public],
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

    #[tokio::test]
    async fn test_perform_post_update_canonicalizes_org_body() {
        let (_pool, storage) = setup_test_db().await;
        // Canonicalization runs on the update path too: a re-saved Org body has its
        // #+TITLE: stripped while an unrecognized #+FOO: and the content survive.
        let created = perform_post_creation(
            &storage,
            PostCreation {
                user_id: 1,
                body: "#+TITLE: First\n\noriginal".to_owned(),
                title: None,
                format: PostFormat::Org,
                slug_override: None,
                published_at: None,
                max_attempts: 100,
                summary: None,
                audiences: vec![AudienceTarget::Public],
            },
        )
        .await
        .unwrap();

        let record = perform_post_update(
            &storage,
            PostUpdate {
                post_id: created.post_id,
                editor_user_id: 1,
                body: "#+TITLE: Second\n#+FOO: keep\n\nupdated".to_owned(),
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

    #[tokio::test]
    async fn test_perform_post_creation_markdown_body_is_not_canonicalized() {
        let (_pool, storage) = setup_test_db().await;
        // Canonicalization is Org-only: a Markdown body with a leading `# H1` is
        // stored verbatim (the `# H1` is not stripped).
        let record = perform_post_creation(
            &storage,
            PostCreation {
                user_id: 1,
                body: "# H1\n\nBody text".to_owned(),
                title: None,
                format: PostFormat::Markdown,
                slug_override: None,
                published_at: None,
                max_attempts: 100,
                summary: None,
                audiences: vec![AudienceTarget::Public],
            },
        )
        .await
        .unwrap();

        assert_eq!(record.body, "# H1\n\nBody text");
    }

    #[tokio::test]
    async fn test_perform_post_creation_org_title_rendered_once() {
        let (_pool, storage) = setup_test_db().await;
        // Double-title regression: the title text from the #+TITLE: line must not
        // survive into the stored body (hence rendered_html), so the page chrome's
        // title is the only place it appears. record.title still carries it.
        let record = perform_post_creation(
            &storage,
            PostCreation {
                user_id: 1,
                body: "#+TITLE: Distinct Headline\n\nParagraph body".to_owned(),
                title: None,
                format: PostFormat::Org,
                slug_override: None,
                published_at: None,
                max_attempts: 100,
                summary: None,
                audiences: vec![AudienceTarget::Public],
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

    #[test]
    fn test_perform_creation_error_display_and_debug() {
        let err = PerformCreationError::EmptyPost;
        assert_eq!(err.to_string(), "post body is required");
        let debug = format!("{err:?}");
        assert!(debug.contains("EmptyPost"));

        let err = PerformCreationError::NoSlugFromPost;
        assert_eq!(
            err.to_string(),
            "post must contain at least one ASCII letter or digit for its slug"
        );

        let err = PerformCreationError::InvalidSlug(InvalidSlug);
        assert_eq!(
            err.to_string(),
            "slug must be non-empty and match [a-z0-9][a-z0-9-]*"
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
    fn perform_update_error_no_slug_from_title_display() {
        let err = PerformUpdateError::NoSlugFromPost;
        assert!(err.to_string().contains("ASCII"));
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
}
