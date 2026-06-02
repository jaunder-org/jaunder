use chrono::{DateTime, Utc};
use thiserror::Error;

use crate::{
    CreatePostError, CreatePostInput, PostFormat, PostRecord, PostStorage, UpdatePostError,
    UpdatePostInput,
};
use common::render::{derive_post_metadata, render, RenderError};
use common::slug::{slugify_title, Slug};

// ---------------------------------------------------------------------------
// Orchestration error types
// ---------------------------------------------------------------------------

#[derive(Debug, Error)]
pub enum CreateRenderedPostError {
    #[error(transparent)]
    Render(#[from] RenderError),
    #[error(transparent)]
    Storage(#[from] CreatePostError),
}

#[derive(Debug, Error)]
pub enum UpdateRenderedPostError {
    #[error(transparent)]
    Render(#[from] RenderError),
    #[error(transparent)]
    Storage(#[from] UpdatePostError),
}

// ---------------------------------------------------------------------------
// Orchestration helpers
// ---------------------------------------------------------------------------

/// Renders `body` according to `format` and creates the post via storage.
///
/// # Errors
///
/// Returns `Err(CreateRenderedPostError)` if rendering fails or the storage layer returns an error.
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
) -> Result<i64, CreateRenderedPostError> {
    let rendered_html = render(&body, &format)?;
    let input = CreatePostInput {
        user_id,
        title,
        slug,
        body,
        format,
        rendered_html,
        published_at,
        summary,
    };
    Ok(storage.create_post(&input).await?)
}

/// Renders `body` according to `format` and updates the post via storage.
///
/// # Errors
///
/// Returns `Err(UpdateRenderedPostError)` if rendering fails or the storage layer returns an error.
#[allow(clippy::too_many_arguments)]
pub async fn update_rendered_post(
    storage: &dyn PostStorage,
    post_id: i64,
    editor_user_id: i64,
    title: Option<String>,
    slug: Slug,
    body: String,
    format: PostFormat,
    publish: bool,
    summary: Option<String>,
) -> Result<PostRecord, UpdateRenderedPostError> {
    let rendered_html = render(&body, &format)?;
    let input = UpdatePostInput {
        title,
        slug,
        body,
        format,
        rendered_html,
        publish,
        summary,
    };
    Ok(storage.update_post(post_id, editor_user_id, &input).await?)
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
    #[error(transparent)]
    Render(#[from] RenderError),
    #[error("post not found")]
    NotFound,
    #[error("not authorized")]
    Unauthorized,
    #[error(transparent)]
    Storage(sqlx::Error),
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

/// Validates inputs, computes the slug, renders the body, and atomically
/// updates the post via storage.
///
/// The storage layer freezes the slug if the post is already published.
/// Ownership and deletion checks are also performed atomically in storage.
///
/// # Errors
///
/// Returns `Err(PerformUpdateError)` if rendering fails or the storage layer returns an error.
#[allow(clippy::too_many_arguments)]
pub async fn perform_post_update(
    storage: &dyn PostStorage,
    post_id: i64,
    editor_user_id: i64,
    body: String,
    title: Option<&str>,
    format: PostFormat,
    slug_override: Option<&str>,
    publish: bool,
    summary: Option<String>,
) -> Result<PostRecord, PerformUpdateError> {
    let metadata =
        derive_post_metadata(title, &body, &format).ok_or(PerformUpdateError::EmptyPost)?;

    let slug = match slug_override.map(str::trim).filter(|s| !s.is_empty()) {
        Some(raw) => raw
            .to_ascii_lowercase()
            .parse::<Slug>()
            .map_err(|_| PerformUpdateError::InvalidSlug)?,
        None => slugify_title(&metadata.slug_seed)
            .ok_or(PerformUpdateError::NoSlugFromPost)?
            .parse::<Slug>()
            .map_err(|_| PerformUpdateError::NoSlugFromPost)?,
    };

    let rendered_html = render(&body, &format)?;
    let input = UpdatePostInput {
        title: metadata.title,
        slug,
        body,
        format,
        rendered_html,
        publish,
        summary,
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
    #[error("{0}")]
    InvalidSlug(String),
    #[error("unable to allocate a unique slug after {0} attempts")]
    Exhausted(usize),
    #[error(transparent)]
    Render(#[from] RenderError),
    #[error("created post not found")]
    CreatedNotFound,
    #[error(transparent)]
    Storage(sqlx::Error),
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

/// Validates inputs, computes the slug, renders the body, and atomically
/// creates the post in storage, retrying on slug collision.
///
/// # Errors
///
/// Returns `Err(PerformCreationError)` if rendering fails, slug validation
/// fails, attempts to find a unique slug are exhausted, or storage fails.
#[allow(clippy::too_many_arguments)]
pub async fn perform_post_creation(
    storage: &dyn PostStorage,
    user_id: i64,
    body: String,
    title: Option<&str>,
    format: PostFormat,
    slug_override: Option<&str>,
    published_at: Option<DateTime<Utc>>,
    max_attempts: usize,
    summary: Option<String>,
) -> Result<PostRecord, PerformCreationError> {
    let metadata =
        derive_post_metadata(title, &body, &format).ok_or(PerformCreationError::EmptyPost)?;

    let slug_seed = match slug_override.map(str::trim).filter(|s| !s.is_empty()) {
        Some(raw) => raw
            .to_ascii_lowercase()
            .parse::<Slug>()
            .map_err(|e| PerformCreationError::InvalidSlug(e.to_string()))?
            .to_string(),
        None => slugify_title(&metadata.slug_seed).ok_or(PerformCreationError::NoSlugFromPost)?,
    };

    for attempt in 0..max_attempts {
        let slug_string = candidate_slug(&slug_seed, attempt);
        let slug = slug_string
            .parse::<Slug>()
            .map_err(|e| PerformCreationError::InvalidSlug(e.to_string()))?;

        match create_rendered_post(
            storage,
            user_id,
            metadata.title.clone(),
            slug,
            body.clone(),
            format.clone(),
            published_at,
            summary.clone(),
        )
        .await
        {
            Ok(post_id) => {
                let record = storage
                    .get_post_by_id(post_id)
                    .await
                    .map_err(PerformCreationError::Storage)?
                    .ok_or(PerformCreationError::CreatedNotFound)?;
                return Ok(record);
            }
            Err(CreateRenderedPostError::Storage(CreatePostError::SlugConflict)) => {}
            Err(CreateRenderedPostError::Storage(CreatePostError::Internal(e))) => {
                return Err(PerformCreationError::Storage(e));
            }
            Err(CreateRenderedPostError::Render(e)) => {
                return Err(PerformCreationError::Render(e));
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
            1,
            "Hello, world!".to_owned(),
            None,
            PostFormat::Markdown,
            None,
            None,
            100,
            None,
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
            1,
            "Body without a heading.".to_owned(),
            Some("Explicit Title"),
            PostFormat::Markdown,
            None,
            None,
            100,
            None,
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
            1,
            "Hello, world!".to_owned(),
            None,
            PostFormat::Markdown,
            Some("my-custom-slug"),
            None,
            100,
            None,
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
            1,
            "Hello, world!".to_owned(),
            None,
            PostFormat::Markdown,
            Some("Invalid Slug!"),
            None,
            100,
            None,
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
            1,
            "   ".to_owned(),
            None,
            PostFormat::Markdown,
            None,
            None,
            100,
            None,
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
            1,
            "!!!".to_owned(),
            None,
            PostFormat::Markdown,
            None,
            None,
            100,
            None,
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
            1,
            "Hello, world!".to_owned(),
            None,
            PostFormat::Markdown,
            None,
            None,
            100,
            None,
        )
        .await
        .unwrap();

        let r2 = perform_post_creation(
            &storage,
            1,
            "Hello, world!".to_owned(),
            None,
            PostFormat::Markdown,
            None,
            None,
            100,
            None,
        )
        .await
        .unwrap();

        let r3 = perform_post_creation(
            &storage,
            1,
            "Hello, world!".to_owned(),
            None,
            PostFormat::Markdown,
            None,
            None,
            100,
            None,
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
            1,
            "Hello, world!".to_owned(),
            None,
            PostFormat::Markdown,
            None,
            None,
            2,
            None,
        )
        .await
        .unwrap();

        let r2 = perform_post_creation(
            &storage,
            1,
            "Hello, world!".to_owned(),
            None,
            PostFormat::Markdown,
            None,
            None,
            2,
            None,
        )
        .await
        .unwrap();

        assert_eq!(r1.slug.as_str(), "hello-world");
        assert_eq!(r2.slug.as_str(), "hello-world-2");

        let err = perform_post_creation(
            &storage,
            1,
            "Hello, world!".to_owned(),
            None,
            PostFormat::Markdown,
            None,
            None,
            2,
            None,
        )
        .await
        .unwrap_err();

        assert!(matches!(err, PerformCreationError::Exhausted(2)));
    }

    #[test]
    fn test_perform_creation_error_display_and_debug() {
        let err = PerformCreationError::EmptyPost;
        assert_eq!(err.to_string(), "post body is required");
        let debug = format!("{:?}", err);
        assert!(debug.contains("EmptyPost"));

        let err = PerformCreationError::NoSlugFromPost;
        assert_eq!(
            err.to_string(),
            "post must contain at least one ASCII letter or digit for its slug"
        );

        let err = PerformCreationError::InvalidSlug("invalid slug message".to_string());
        assert_eq!(err.to_string(), "invalid slug message");

        let err = PerformCreationError::Exhausted(10);
        assert_eq!(
            err.to_string(),
            "unable to allocate a unique slug after 10 attempts"
        );

        let err = PerformCreationError::CreatedNotFound;
        assert_eq!(err.to_string(), "created post not found");
    }
}
