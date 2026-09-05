//! Post create/update orchestration over the [`PostStorage`] trait.
//!
//! Validates input, derives titles/slugs (via `common::render`), renders the
//! body, and performs the storage write with slug-collision retry. Shared by
//! the `web` and `server` `AtomPub` front-ends.

use std::collections::HashSet;
use std::sync::Arc;

use thiserror::Error;

use crate::{
    CreatePostError, CreatePostInput, FeedEventStorage, MediaContentLocks,
    PostBookkeepingExpectation, PostFormat, PostMutation, PostRecord, PostStorage, PublishUpdate,
    UpdatePostError, UpdatePostInput, WriteScope, WriteScopeError, WriteTransaction,
};
use common::idempotency_key::IdempotencyKey;
use common::ids::{PostId, UserId};
use common::mutation::MutationOutcome;
use common::post_body::PostBody;
use common::post_summary::PostSummary;
use common::post_title::PostTitle;
use common::slug::{InvalidSlug, Slug};
use common::time::UtcInstant;
use common::visibility::AudienceTarget;
use host::feed;
use host::metrics::{self, IdempotencyEvent};

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
    pub published_at: Option<UtcInstant>,
    /// Optional summary/excerpt.
    pub summary: Option<PostSummary>,
    /// Audience targeting for the new post.
    pub audiences: Vec<AudienceTarget>,
    /// Tags for the new post, persisted in the same creation transaction.
    pub tags: Vec<common::tag::TagLabel>,
    /// Owned idempotency key to register with the post, or `None`.
    pub idempotency_key: Option<IdempotencyKey>,
    /// Non-authoritative Org bookkeeping expected to match the final stored row.
    pub expectations: PostBookkeepingExpectation,
}

/// Converts a post-creation write-scope failure into its public service error.
fn map_create_post_scope_error(error: WriteScopeError<CreatePostError>) -> CreatePostError {
    match error {
        WriteScopeError::Operation(error) => error,
        WriteScopeError::Begin(error) => CreatePostError::Internal(error),
    }
}

/// Converts a post-update write-scope failure into its public service error.
fn map_post_update_scope_error(error: WriteScopeError<UpdatePostError>) -> PerformUpdateError {
    match error {
        WriteScopeError::Operation(error) => error.into(),
        WriteScopeError::Begin(error) => PerformUpdateError::Storage(error),
    }
}

fn is_currently_public(record: &PostRecord, has_public_audience: bool, now: UtcInstant) -> bool {
    has_public_audience
        && record.deleted_at.is_none()
        && record
            .published_at
            .is_some_and(|published_at| published_at <= now)
}

fn affected_post_feed_paths(
    previous: Option<(&PostRecord, bool)>,
    current: (&PostRecord, bool),
    now: UtcInstant,
) -> Vec<host::feed::FeedPath> {
    let previous_is_public =
        previous.is_some_and(|(record, has_public)| is_currently_public(record, has_public, now));
    let current_is_public = is_currently_public(current.0, current.1, now);
    if !previous_is_public && !current_is_public {
        return Vec::new();
    }
    let record = current.0;
    let mut tags = HashSet::new();
    let tag_slugs = previous
        .into_iter()
        .flat_map(|(record, _)| record.tags.iter())
        .chain(current.0.tags.iter())
        .map(|tag| &tag.tag_slug)
        .filter(|tag| tags.insert(*tag));
    feed::affected_feed_urls(&record.author_username, tag_slugs)
}

async fn enqueue_lifecycle_feed_paths(
    transaction: &mut WriteTransaction,
    feed_events: &Arc<dyn FeedEventStorage>,
    mutation: &PostMutation,
    now: UtcInstant,
) -> Result<(), UpdatePostError> {
    if !mutation.changed {
        return Ok(());
    }
    let feed_paths = affected_post_feed_paths(
        Some((&mutation.previous, mutation.previous_has_public_audience)),
        (&mutation.record, mutation.previous_has_public_audience),
        now,
    );
    if !feed_paths.is_empty() {
        feed_events
            .enqueue_many(transaction, &feed_paths)
            .await
            .map_err(|error| match error {
                crate::FeedEventError::Db(error) => UpdatePostError::Internal(error),
            })?;
    }
    Ok(())
}

/// Renders `body` according to `format` and creates the post through one caller-owned
/// write scope.
///
/// # Errors
///
/// Returns the Post creation failure or a feed-event enqueue failure mapped to
/// [`CreatePostError::Internal`]. A scope acquisition failure is also mapped to
/// [`CreatePostError::Internal`].
pub async fn create_rendered_post(
    write_scope: &WriteScope,
    content_locks: &MediaContentLocks,
    storage: Arc<dyn PostStorage>,
    feed_events: Arc<dyn FeedEventStorage>,
    content: RenderedPostContent,
    now: UtcInstant,
) -> Result<MutationOutcome<PostRecord>, CreatePostError> {
    let input = render_post_input(content);
    let _media_locks = content_locks
        .acquire(
            input
                .rendered
                .media()
                .iter()
                .map(common::media::MediaReference::media),
        )
        .await
        .map_err(|error| CreatePostError::Internal(sqlx::Error::Io(error)))?;
    let outcome = write_scope
        .run(move |transaction| {
            let storage = Arc::clone(&storage);
            let feed_events = Arc::clone(&feed_events);
            Box::pin(async move {
                let created = storage.create_post(transaction, &input, now).await?;
                let feed_paths = affected_post_feed_paths(
                    None,
                    (
                        &created.record,
                        input
                            .audiences
                            .iter()
                            .any(|audience| matches!(audience, AudienceTarget::Public)),
                    ),
                    now,
                );
                if !feed_paths.is_empty() {
                    feed_events
                        .enqueue_many(transaction, &feed_paths)
                        .await
                        .map_err(|error| match error {
                            crate::FeedEventError::Db(error) => CreatePostError::Internal(error),
                        })?;
                }
                Ok(created)
            })
        })
        .await
        .map_err(map_create_post_scope_error)?;
    match outcome {
        MutationOutcome::Confirmed(created) => {
            if created.idempotency_key_expired {
                metrics::idempotency(IdempotencyEvent::Expired);
            }
            Ok(MutationOutcome::Confirmed(created.record))
        }
        MutationOutcome::CommitIndeterminate(created) => {
            Ok(MutationOutcome::CommitIndeterminate(created.record))
        }
    }
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
        tags,
        idempotency_key,
        expectations,
    } = content;
    let rendered = host::render::with_media(&body, &format);
    CreatePostInput {
        user_id,
        title,
        slug,
        body,
        format,
        rendered,
        published_at,
        summary,
        audiences,
        tags,
        expectations,
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
        published_at: published.then(UtcInstant::now),
        summary: None,
        audiences: vec![AudienceTarget::Public],
        tags: Vec::new(),
        idempotency_key: None,
        expectations: PostBookkeepingExpectation::default(),
    })
}

// ---------------------------------------------------------------------------
// High-level post-update orchestration
// ---------------------------------------------------------------------------

/// Errors that can occur during a high-level post update.
#[derive(Debug, Error)]
pub enum PerformUpdateError {
    /// Reachable only through canonicalization (#811): a blank body cannot be
    /// built at all any more, so the sole way to arrive here is a body that
    /// *becomes* blank — an Org post whose title source is its entire content.
    #[error("post body is only its title, leaving nothing to store")]
    EmptyPost,
    #[error("post not found")]
    NotFound,
    #[error("not authorized")]
    Unauthorized,
    #[error("post bookkeeping does not match the stored post")]
    BookkeepingMismatch,
    #[error("post content has changed")]
    StaleContent,
    #[error("storage error: {0}")]
    Storage(#[source] sqlx::Error),
}

impl From<UpdatePostError> for PerformUpdateError {
    fn from(e: UpdatePostError) -> Self {
        match e {
            UpdatePostError::NotFound => Self::NotFound,
            UpdatePostError::Unauthorized => Self::Unauthorized,
            UpdatePostError::BookkeepingMismatch => Self::BookkeepingMismatch,
            UpdatePostError::StaleContent => Self::StaleContent,
            UpdatePostError::Internal(e) => Self::Storage(e),
        }
    }
}

impl From<PerformUpdateError> for host::error::InternalError {
    /// Reproduces the former `web::posts::server::perform_update_error`
    /// `(kind, class, public_message)`: the empty-post arm is a client validation
    /// error, not-found/unauthorized mask as a 404, storage is a masked storage
    /// failure. The validation arm carries the typed `PerformUpdateError` as the
    /// operator-side source instead of flattening it (A19).
    fn from(error: PerformUpdateError) -> Self {
        use host::error::InternalError;
        match error {
            PerformUpdateError::EmptyPost
            | PerformUpdateError::BookkeepingMismatch
            | PerformUpdateError::StaleContent => {
                InternalError::validation_source(error.to_string(), error)
            }
            PerformUpdateError::NotFound | PerformUpdateError::Unauthorized => {
                InternalError::not_found("Post")
            }
            PerformUpdateError::Storage(e) => InternalError::storage(e),
        }
    }
}

/// Raw, front-end-supplied inputs to [`perform_post_update`].
///
pub struct PostUpdate<'a> {
    /// Post being edited.
    pub post_id: PostId,
    /// User performing the edit (ownership is checked in storage).
    pub editor_user_id: UserId,
    /// Raw post body in `format`.
    pub body: PostBody,
    /// Explicit title, or `None` to derive one from the body.
    pub title: Option<&'a PostTitle>,
    /// Markup format of `body`.
    pub format: PostFormat,
    /// Explicit slug (already validated at the wire/CLI boundary), or `None` to
    /// derive one from the title/body.
    pub slug_override: Option<&'a Slug>,
    /// What this update does to the post's publication state.
    pub publish: PublishUpdate,
    /// Optional summary/excerpt.
    pub summary: Option<PostSummary>,
    /// Audience targeting for the post (replaces its existing rows). An empty
    pub audiences: Vec<AudienceTarget>,
    /// Tags replacing the current set, or `None` to preserve the locked state.
    pub tags: Option<Vec<common::tag::TagLabel>>,
    /// The request clock reused if this update publishes a draft without a date.
    pub request_clock: UtcInstant,
    /// Non-authoritative Org bookkeeping expected to match the locked row.
    pub expectations: PostBookkeepingExpectation,
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
    write_scope: &WriteScope,
    content_locks: &MediaContentLocks,
    storage: Arc<dyn PostStorage>,
    feed_events: Arc<dyn FeedEventStorage>,
    input: PostUpdate<'_>,
) -> Result<MutationOutcome<PostRecord>, PerformUpdateError> {
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
        tags,
        request_clock,
        expectations,
    } = input;
    let (title, derived_slug) = common::render::derive_post_naming(title, &body, &format);

    // Derive the naming from the *original* body above, then canonicalize what gets
    // stored. Web and AtomPub thus converge on one stored body. A title-only Org post
    // canonicalizes to nothing, which names no body (#811): there is nothing left to
    // store, so it earns the same rejection as an empty post.
    let body = common::render::canonicalize_body(&body, &format)
        .map_err(|_| PerformUpdateError::EmptyPost)?;

    let slug = match slug_override {
        // Pre-validated at the boundary (wire/CLI); updates keep the slug as-is,
        // no collision dedup.
        Some(slug) => slug.clone(),
        None => derived_slug,
    };

    let rendered = host::render::with_media(&body, &format);
    let input = UpdatePostInput {
        title,
        slug,
        body,
        format,
        rendered,
        publish,
        summary,
        audiences,
        tags,
        request_clock,
        expectations,
    };
    let _media_locks = content_locks
        .acquire(
            input
                .rendered
                .media()
                .iter()
                .map(common::media::MediaReference::media),
        )
        .await
        .map_err(|error| PerformUpdateError::Storage(sqlx::Error::Io(error)))?;
    write_scope
        .run(move |transaction| {
            let storage = Arc::clone(&storage);
            let feed_events = Arc::clone(&feed_events);
            Box::pin(async move {
                let mutation = storage
                    .update_post(transaction, post_id, editor_user_id, &input)
                    .await?;
                if mutation.changed {
                    let current_has_public_audience = input
                        .audiences
                        .iter()
                        .any(|audience| matches!(audience, AudienceTarget::Public));
                    let feed_paths = affected_post_feed_paths(
                        Some((&mutation.previous, mutation.previous_has_public_audience)),
                        (&mutation.record, current_has_public_audience),
                        input.request_clock,
                    );
                    if !feed_paths.is_empty() {
                        feed_events
                            .enqueue_many(transaction, &feed_paths)
                            .await
                            .map_err(|error| match error {
                                crate::FeedEventError::Db(error) => {
                                    UpdatePostError::Internal(error)
                                }
                            })?;
                    }
                }
                Ok::<PostRecord, UpdatePostError>(mutation.record)
            })
        })
        .await
        .map_err(map_post_update_scope_error)
}

/// Publishes an owned Post and atomically queues only its earned public feed
/// invalidation paths.
///
/// # Errors
///
/// Returns the underlying ownership, liveness, storage, or feed-event enqueue
/// failure as [`PerformUpdateError`].
pub async fn publish_post(
    write_scope: &WriteScope,
    storage: Arc<dyn PostStorage>,
    feed_events: Arc<dyn FeedEventStorage>,
    post_id: PostId,
    user_id: UserId,
    now: UtcInstant,
) -> Result<MutationOutcome<PostRecord>, PerformUpdateError> {
    write_scope
        .run(move |transaction| {
            let storage = Arc::clone(&storage);
            let feed_events = Arc::clone(&feed_events);
            Box::pin(async move {
                let mutation = storage
                    .publish_post(transaction, post_id, user_id, now)
                    .await?;
                enqueue_lifecycle_feed_paths(transaction, &feed_events, &mutation, now).await?;
                Ok::<PostRecord, UpdatePostError>(mutation.record)
            })
        })
        .await
        .map_err(map_post_update_scope_error)
}

/// Reverts an owned Post to draft status and atomically queues only its earned
/// public feed invalidation paths.
///
/// # Errors
///
/// Returns the underlying ownership, liveness, storage, or feed-event enqueue
/// failure as [`PerformUpdateError`].
pub async fn unpublish_post(
    write_scope: &WriteScope,
    storage: Arc<dyn PostStorage>,
    feed_events: Arc<dyn FeedEventStorage>,
    post_id: PostId,
    user_id: UserId,
    now: UtcInstant,
) -> Result<MutationOutcome<PostRecord>, PerformUpdateError> {
    write_scope
        .run(move |transaction| {
            let storage = Arc::clone(&storage);
            let feed_events = Arc::clone(&feed_events);
            Box::pin(async move {
                let mutation = storage
                    .unpublish_post(transaction, post_id, user_id, now)
                    .await?;
                enqueue_lifecycle_feed_paths(transaction, &feed_events, &mutation, now).await?;
                Ok::<PostRecord, UpdatePostError>(mutation.record)
            })
        })
        .await
        .map_err(map_post_update_scope_error)
}

/// Soft-deletes an owned Post and atomically queues only its earned public feed
/// invalidation paths.
///
/// # Errors
///
/// Returns the underlying ownership, liveness, storage, or feed-event enqueue
/// failure as [`PerformUpdateError`].
pub async fn soft_delete_post(
    write_scope: &WriteScope,
    storage: Arc<dyn PostStorage>,
    feed_events: Arc<dyn FeedEventStorage>,
    post_id: PostId,
    user_id: UserId,
    now: UtcInstant,
) -> Result<MutationOutcome<()>, PerformUpdateError> {
    write_scope
        .run(move |transaction| {
            let storage = Arc::clone(&storage);
            let feed_events = Arc::clone(&feed_events);
            Box::pin(async move {
                let mutation = storage
                    .soft_delete_post(transaction, post_id, user_id, now)
                    .await?;
                enqueue_lifecycle_feed_paths(transaction, &feed_events, &mutation, now).await?;
                Ok::<(), UpdatePostError>(())
            })
        })
        .await
        .map_err(map_post_update_scope_error)
}

// ---------------------------------------------------------------------------
// High-level post-creation orchestration
// ---------------------------------------------------------------------------

/// Errors that can occur during high-level post creation.
#[derive(Debug, Error)]
pub enum PerformCreationError {
    /// Reachable only through canonicalization (#811): a blank body cannot be
    /// built at all any more, so the sole way to arrive here is a body that
    /// *becomes* blank — an Org post whose title source is its entire content.
    #[error("post body is only its title, leaving nothing to store")]
    EmptyPost,
    #[error(transparent)]
    InvalidSlug(#[from] InvalidSlug),
    #[error("unable to allocate a unique slug after {0} attempts")]
    Exhausted(usize),
    #[error("created post not found")]
    CreatedNotFound,
    /// The idempotency key already maps to this Post. The mapping was chosen
    /// atomically with the rejected create.
    #[error("idempotency key already used for this user")]
    IdempotencyConflict(PostId),
    #[error("post bookkeeping does not match the stored post")]
    BookkeepingMismatch,
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
            // Single-sourced from the variant's `#[error]` so the public message
            PerformCreationError::EmptyPost | PerformCreationError::BookkeepingMismatch => {
                InternalError::validation(error.to_string())
            }
            PerformCreationError::InvalidSlug(_) => {
                InternalError::validation_source(error.to_string(), error)
            }
            // Carry the typed error as the operator source (its `Display` renders the real
            // attempt count) rather than a hardcoded literal that lies when the retry bound
            // isn't 100. Wire projection is unchanged (kind `Internal` → "server operation failed").
            //
            // The AtomPub handler intercepts the replay decision and returns the
            // selected post as `200`; this conversion covers non-AtomPub misuse.
            PerformCreationError::Exhausted(_) | PerformCreationError::IdempotencyConflict(_) => {
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
pub struct PostCreation<'a> {
    /// Author of the new post.
    pub user_id: UserId,
    /// Raw post body in `format`.
    pub body: PostBody,
    /// Explicit title, or `None` to derive one from the body.
    pub title: Option<&'a PostTitle>,
    /// Markup format of `body`.
    pub format: PostFormat,
    /// Explicit slug (already validated at the wire/CLI boundary), or `None` to
    /// derive one from the title/body.
    pub slug_override: Option<&'a Slug>,
    /// Publication timestamp, or `None` to create as a draft.
    pub published_at: Option<UtcInstant>,
    /// Maximum slug-collision retries before giving up.
    pub max_attempts: usize,
    /// Optional summary/excerpt.
    pub summary: Option<PostSummary>,
    /// Audience targeting for the new post. An empty vec (or `[Private]`) makes
    /// the post author-only.
    pub audiences: Vec<AudienceTarget>,
    /// Tags for the new post.
    pub tags: Vec<common::tag::TagLabel>,
    /// Client-supplied idempotency key (already trimmed / non-empty), or `None`
    /// to create without deduplication.
    pub idempotency_key: Option<&'a IdempotencyKey>,
    /// Non-authoritative Org bookkeeping expected to match the collision winner.
    pub expectations: PostBookkeepingExpectation,
}

/// Validates inputs, computes the slug, renders the body, and atomically
/// creates the post in storage, retrying on slug collision.
///
/// # Errors
///
/// Returns `Err(PerformCreationError)` if slug validation fails, attempts to
/// find a unique slug are exhausted, or storage fails.
pub async fn perform_post_creation(
    write_scope: &WriteScope,
    content_locks: &MediaContentLocks,
    storage: Arc<dyn PostStorage>,
    feed_events: Arc<dyn FeedEventStorage>,
    input: PostCreation<'_>,
) -> Result<MutationOutcome<PostRecord>, PerformCreationError> {
    perform_post_creation_at(
        write_scope,
        content_locks,
        storage,
        feed_events,
        UtcInstant::now(),
        input,
    )
    .await
}

/// Performs post creation against one explicit request clock.
///
/// `AtomPub` supplies its request clock so an Idempotency Key mapping and its
/// replay cutoff cannot be extended by an implicit storage clock.
///
/// # Errors
///
/// Returns `Err(PerformCreationError)` if slug validation fails, attempts to
/// find a unique slug are exhausted, or storage fails.
pub async fn perform_post_creation_at(
    write_scope: &WriteScope,
    content_locks: &MediaContentLocks,
    storage: Arc<dyn PostStorage>,
    feed_events: Arc<dyn FeedEventStorage>,
    now: UtcInstant,
    input: PostCreation<'_>,
) -> Result<MutationOutcome<PostRecord>, PerformCreationError> {
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
        tags,
        idempotency_key,
        expectations,
    } = input;
    let (title, derived_slug) = common::render::derive_post_naming(title, &body, &format);

    // Derive the naming from the *original* body above, then canonicalize what gets
    // stored. Web and AtomPub thus converge on one stored body. A title-only Org post
    // canonicalizes to nothing, which names no body (#811): there is nothing left to
    // store, so it earns the same rejection as an empty post.
    let body = common::render::canonicalize_body(&body, &format)
        .map_err(|_| PerformCreationError::EmptyPost)?;

    let slug_seed: Slug = match slug_override {
        // Pre-validated at the boundary (wire/CLI); a valid override is still fed
        // through the collision-suffix generator below for uniqueness.
        Some(slug) => slug.clone(),
        None => derived_slug,
    };

    for attempt in 0..max_attempts {
        let slug =
            candidate_slug(&slug_seed, attempt).map_err(PerformCreationError::InvalidSlug)?;
        let is_expected_slug = expectations
            .slug
            .as_ref()
            .is_some_and(|expected| expected == &slug);

        match create_rendered_post(
            write_scope,
            content_locks,
            Arc::clone(&storage),
            Arc::clone(&feed_events),
            RenderedPostContent {
                user_id,
                title: title.clone(),
                slug,
                body: body.clone(),
                format,
                published_at,
                summary: summary.clone(),
                audiences: audiences.clone(),
                idempotency_key: idempotency_key.cloned(),
                tags: tags.clone(),
                expectations: expectations.clone(),
            },
            now,
        )
        .await
        {
            Ok(outcome) => return Ok(outcome),
            Err(CreatePostError::SlugConflict) if is_expected_slug => {
                return Err(PerformCreationError::BookkeepingMismatch);
            }
            Err(CreatePostError::SlugConflict) => {}
            // A duplicate key is not a slug collision. Storage chose the live
            // mapping inside this transaction, so return that immutable decision
            // rather than performing a racy lookup after rollback.
            Err(CreatePostError::IdempotencyConflict(post_id)) => {
                return Err(PerformCreationError::IdempotencyConflict(post_id));
            }
            Err(CreatePostError::BookkeepingMismatch) => {
                return Err(PerformCreationError::BookkeepingMismatch);
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
    use crate::sql::QueryStorageExt;
    #[cfg(feature = "test-utils")]
    use crate::test_support::mock_write_scope;
    use crate::test_support::{
        Backend, SeedPost, SeedUser, backends, confirmed, fetch_post_media, media_ref_for,
        media_url_for, seed_media,
    };
    #[cfg(feature = "test-utils")]
    use crate::{MockFeedEventStorage, MockPostStorage};
    use chrono::{Duration, TimeZone, Utc};
    use common::idempotency_key::IdempotencyKey;
    use common::media::{MediaReferenceForm, MediaReferenceKind};
    use common::test_support::{parse_post_body, parse_post_title, parse_row_limit, parse_slug};
    #[cfg(feature = "test-utils")]
    use common::test_support::{parse_tag, parse_tag_label};
    #[cfg(feature = "test-utils")]
    use sqlx::Error as SqlxError;

    use rstest::*;
    use rstest_reuse::*;
    #[test]
    fn create_post_scope_error_maps_operation_and_begin() {
        assert!(matches!(
            map_create_post_scope_error(WriteScopeError::Operation(CreatePostError::SlugConflict)),
            CreatePostError::SlugConflict
        ));

        let error = map_create_post_scope_error(WriteScopeError::Begin(sqlx::Error::PoolClosed));
        let CreatePostError::Internal(source) = error else {
            unreachable!("scope_error maps scope begin failures to CreatePostError::Internal");
        };
        assert!(matches!(source, sqlx::Error::PoolClosed));
    }

    #[test]
    fn post_update_scope_error_maps_operation_and_begin() {
        assert!(matches!(
            map_post_update_scope_error(WriteScopeError::Operation(UpdatePostError::NotFound)),
            PerformUpdateError::NotFound
        ));

        let error = map_post_update_scope_error(WriteScopeError::Begin(sqlx::Error::PoolClosed));
        let PerformUpdateError::Storage(source) = error else {
            unreachable!("scope_error maps scope begin failures to PerformUpdateError::Storage");
        };
        assert!(matches!(source, sqlx::Error::PoolClosed));
    }

    // -- perform_post_creation tests --

    #[apply(backends)]
    #[tokio::test]
    async fn test_perform_post_creation_success(#[case] backend: Backend) {
        let env = backend.setup().await;
        let seeded_user = SeedUser::new().seed(&env.state).await;
        let user_id = seeded_user.user_id;

        let storage = Arc::clone(&env.state.posts);
        let record = perform_post_creation(
            &env.state.write_scope,
            &env.media_content_locks(),
            Arc::clone(&storage),
            Arc::clone(&env.state.feed_events),
            PostCreation {
                user_id,
                body: parse_post_body("Hello, world!"),
                title: None,
                format: PostFormat::Markdown,
                slug_override: None,
                published_at: None,
                max_attempts: 100,
                summary: None,
                audiences: vec![AudienceTarget::Public],
                tags: Vec::new(),
                idempotency_key: None,
                expectations: PostBookkeepingExpectation::default(),
            },
        )
        .await
        .unwrap();

        let record = confirmed(record);
        assert_eq!(record.user_id, user_id);
        assert_eq!(record.slug, "hello-world");
        // Canonicalized on write (#811): the stored body carries a terminating newline.
        assert_eq!(record.body, "Hello, world!\n");
        assert_eq!(record.format, PostFormat::Markdown);
        assert!(record.rendered_html.contains("<p>Hello, world!</p>"));
    }

    #[apply(backends)]
    #[tokio::test]
    async fn test_perform_post_creation_returns_a_private_post(#[case] backend: Backend) {
        let env = backend.setup().await;
        let seeded_user = SeedUser::new().seed(&env.state).await;
        let user_id = seeded_user.user_id;

        let storage = Arc::clone(&env.state.posts);
        // No audience target at all: the post is visible to its author and to
        // nobody else. Every other create test targets Public, so this is the
        // only one that can observe that the post-create re-read resolves *as
        // the author* rather than incidentally as an anonymous reader.
        let title = parse_post_title("Private Note");
        let record = perform_post_creation(
            &env.state.write_scope,
            &env.media_content_locks(),
            Arc::clone(&storage),
            Arc::clone(&env.state.feed_events),
            PostCreation {
                user_id,
                body: parse_post_body("Private note."),
                title: Some(&title),
                format: PostFormat::Markdown,
                slug_override: None,
                published_at: None,
                max_attempts: 100,
                summary: None,
                audiences: vec![],
                tags: Vec::new(),
                idempotency_key: None,
                expectations: PostBookkeepingExpectation::default(),
            },
        )
        .await
        .unwrap();

        let record = confirmed(record);
        assert_eq!(record.user_id, user_id);
        assert_eq!(record.slug, "private-note");

        // Guards the premise: if targeting ever started defaulting to public,
        // the assertion above would keep passing while proving nothing.
        assert!(
            storage
                .get_post_by_id(
                    record.post_id,
                    &common::visibility::ViewerIdentity::Anonymous
                )
                .await
                .unwrap()
                .is_none()
        );
    }

    #[apply(backends)]
    #[tokio::test]
    async fn test_perform_post_creation_uses_explicit_title(#[case] backend: Backend) {
        let env = backend.setup().await;
        let seeded_user = SeedUser::new().seed(&env.state).await;
        let user_id = seeded_user.user_id;

        let storage = Arc::clone(&env.state.posts);
        // The body has no heading, so any title must come from the explicit arg,
        // which also seeds the slug.
        let title = parse_post_title("Explicit Title");
        let record = perform_post_creation(
            &env.state.write_scope,
            &env.media_content_locks(),
            Arc::clone(&storage),
            Arc::clone(&env.state.feed_events),
            PostCreation {
                user_id,
                body: parse_post_body("Body without a heading."),
                title: Some(&title),
                format: PostFormat::Markdown,
                slug_override: None,
                published_at: None,
                max_attempts: 100,
                summary: None,
                audiences: vec![AudienceTarget::Public],
                tags: Vec::new(),
                idempotency_key: None,
                expectations: PostBookkeepingExpectation::default(),
            },
        )
        .await
        .unwrap();

        let record = confirmed(record);
        assert_eq!(record.title.as_deref(), Some("Explicit Title"));
        assert_eq!(record.slug, "explicit-title");
    }

    #[apply(backends)]
    #[tokio::test]
    async fn test_perform_post_creation_slug_override(#[case] backend: Backend) {
        let env = backend.setup().await;
        let seeded_user = SeedUser::new().seed(&env.state).await;
        let user_id = seeded_user.user_id;

        let storage = Arc::clone(&env.state.posts);
        // The override arrives already validated as a `Slug` (the wire/CLI boundary
        // parses it); an invalid override cannot reach this layer — that rejection
        // lives at the boundary (web `field_error` + the serde bridge).
        let slug: Slug = parse_slug("my-custom-slug");
        let record = perform_post_creation(
            &env.state.write_scope,
            &env.media_content_locks(),
            Arc::clone(&storage),
            Arc::clone(&env.state.feed_events),
            PostCreation {
                user_id,
                body: parse_post_body("Hello, world!"),
                title: None,
                format: PostFormat::Markdown,
                slug_override: Some(&slug),
                published_at: None,
                max_attempts: 100,
                summary: None,
                audiences: vec![AudienceTarget::Public],
                tags: Vec::new(),
                idempotency_key: None,
                expectations: PostBookkeepingExpectation::default(),
            },
        )
        .await
        .unwrap();

        let record = confirmed(record);
        assert_eq!(record.slug, "my-custom-slug");
    }

    // guard:no-backend — injects a MockPostStorage whose create_post returns an
    // Internal error; no live database backend
    #[cfg(feature = "test-utils")]
    #[tokio::test]
    async fn test_perform_post_creation_storage_internal_error() {
        // A storage-layer `Internal` error from `create_post` (as opposed to the
        // retryable `SlugConflict`) short-circuits the slug-retry loop into
        // `PerformCreationError::Storage`.
        let mut storage = MockPostStorage::new();
        storage
            .expect_create_post()
            .returning(|_, _, _| Err(CreatePostError::Internal(SqlxError::RowNotFound)));
        let write_scope = mock_write_scope();
        let feed_events = MockFeedEventStorage::new();
        let storage: Arc<dyn PostStorage> = Arc::new(storage);
        let feed_events: Arc<dyn FeedEventStorage> = Arc::new(feed_events);
        let temp = tempfile::tempdir().unwrap();
        let content_locks = MediaContentLocks::new(Arc::new(temp.path().to_path_buf()));
        let err = perform_post_creation(
            &write_scope,
            &content_locks,
            Arc::clone(&storage),
            feed_events,
            PostCreation {
                user_id: UserId::from(1),
                body: parse_post_body("Hello, world!"),
                title: None,
                format: PostFormat::Markdown,
                slug_override: None,
                published_at: None,
                max_attempts: 100,
                summary: None,
                audiences: vec![AudienceTarget::Public],
                tags: Vec::new(),
                idempotency_key: None,
                expectations: PostBookkeepingExpectation::default(),
            },
        )
        .await
        .unwrap_err();

        assert!(matches!(err, PerformCreationError::Storage(_)));
    }
    #[cfg(feature = "test-utils")]
    #[apply(backends)]
    #[tokio::test]
    async fn feed_enqueue_failure_rolls_back_the_created_post(#[case] backend: Backend) {
        let env = backend.setup().await;
        let seeded_user = SeedUser::new().seed(&env.state).await;
        let mut feed_events = crate::MockFeedEventStorage::new();
        feed_events
            .expect_enqueue_many()
            .returning(|_, _| Err(crate::FeedEventError::Db(sqlx::Error::RowNotFound)));
        let feed_events: Arc<dyn FeedEventStorage> = Arc::new(feed_events);
        let now = UtcInstant::now();

        let error = perform_post_creation_at(
            &env.state.write_scope,
            &env.media_content_locks(),
            Arc::clone(&env.state.posts),
            feed_events,
            now,
            PostCreation {
                user_id: seeded_user.user_id,
                body: parse_post_body("Post must not survive a failed feed enqueue."),
                title: None,
                format: PostFormat::Markdown,
                slug_override: None,
                published_at: Some(now),
                max_attempts: 1,
                summary: None,
                audiences: vec![AudienceTarget::Public],
                tags: Vec::new(),
                idempotency_key: None,
                expectations: PostBookkeepingExpectation::default(),
            },
        )
        .await
        .expect_err("feed enqueue fails after the post insert");
        assert!(matches!(error, PerformCreationError::Storage(_)));
        assert!(
            env.state
                .posts
                .list_collection_by_user(seeded_user.user_id, None, parse_row_limit("10"))
                .await
                .expect("post collection loads")
                .is_empty(),
            "the enclosing write scope must roll back the earlier post insert"
        );
    }

    #[cfg(feature = "test-utils")]
    #[apply(backends)]
    #[tokio::test]
    async fn lifecycle_services_enqueue_only_changed_public_transitions(#[case] backend: Backend) {
        let env = backend.setup().await;
        let user = SeedUser::new().seed(&env.state).await;
        let post = SeedPost::new(user.user_id).seed(&env.state).await;
        let mut feed_events = MockFeedEventStorage::new();
        feed_events
            .expect_enqueue_many()
            .times(3)
            .returning(|_, _| Ok(()));
        let feed_events: Arc<dyn FeedEventStorage> = Arc::new(feed_events);
        let storage: Arc<dyn PostStorage> = Arc::clone(&env.state.posts) as Arc<dyn PostStorage>;
        let now = UtcInstant::now();

        confirmed(
            unpublish_post(
                &env.state.write_scope,
                Arc::clone(&storage),
                Arc::clone(&feed_events),
                post.post_id,
                user.user_id,
                now,
            )
            .await
            .expect("public unpublish succeeds"),
        );
        confirmed(
            unpublish_post(
                &env.state.write_scope,
                Arc::clone(&storage),
                Arc::clone(&feed_events),
                post.post_id,
                user.user_id,
                now,
            )
            .await
            .expect("repeated unpublish is a no-op"),
        );
        confirmed(
            publish_post(
                &env.state.write_scope,
                Arc::clone(&storage),
                Arc::clone(&feed_events),
                post.post_id,
                user.user_id,
                now,
            )
            .await
            .expect("public publish succeeds"),
        );
        confirmed(
            publish_post(
                &env.state.write_scope,
                Arc::clone(&storage),
                Arc::clone(&feed_events),
                post.post_id,
                user.user_id,
                now,
            )
            .await
            .expect("repeated publish is a no-op"),
        );
        confirmed(
            soft_delete_post(
                &env.state.write_scope,
                storage,
                feed_events,
                post.post_id,
                user.user_id,
                now,
            )
            .await
            .expect("public delete succeeds"),
        );
    }

    #[cfg(feature = "test-utils")]
    #[apply(backends)]
    #[tokio::test]
    async fn lifecycle_services_skip_nonpublic_transitions(#[case] backend: Backend) {
        let env = backend.setup().await;
        let user = SeedUser::new().seed(&env.state).await;
        let post = SeedPost::new(user.user_id)
            .audiences(vec![AudienceTarget::Subscribers])
            .seed(&env.state)
            .await;
        let mut feed_events = MockFeedEventStorage::new();
        feed_events.expect_enqueue_many().times(0);
        let feed_events: Arc<dyn FeedEventStorage> = Arc::new(feed_events);
        let storage: Arc<dyn PostStorage> = Arc::clone(&env.state.posts) as Arc<dyn PostStorage>;
        let now = UtcInstant::now();

        confirmed(
            unpublish_post(
                &env.state.write_scope,
                Arc::clone(&storage),
                Arc::clone(&feed_events),
                post.post_id,
                user.user_id,
                now,
            )
            .await
            .expect("non-public unpublish succeeds"),
        );
        confirmed(
            publish_post(
                &env.state.write_scope,
                Arc::clone(&storage),
                Arc::clone(&feed_events),
                post.post_id,
                user.user_id,
                now,
            )
            .await
            .expect("non-public publish succeeds"),
        );
        confirmed(
            soft_delete_post(
                &env.state.write_scope,
                storage,
                feed_events,
                post.post_id,
                user.user_id,
                now,
            )
            .await
            .expect("non-public delete succeeds"),
        );
    }

    #[cfg(feature = "test-utils")]
    #[apply(backends)]
    #[tokio::test]
    async fn lifecycle_enqueue_failure_rolls_back_the_post_transition(#[case] backend: Backend) {
        let env = backend.setup().await;
        let user = SeedUser::new().seed(&env.state).await;
        let post = SeedPost::new(user.user_id).seed(&env.state).await;
        let mut feed_events = MockFeedEventStorage::new();
        feed_events
            .expect_enqueue_many()
            .times(1)
            .returning(|_, _| Err(crate::FeedEventError::Db(sqlx::Error::RowNotFound)));
        let error = unpublish_post(
            &env.state.write_scope,
            Arc::clone(&env.state.posts) as Arc<dyn PostStorage>,
            Arc::new(feed_events),
            post.post_id,
            user.user_id,
            UtcInstant::now(),
        )
        .await
        .expect_err("feed enqueue fails after lifecycle mutation");
        assert!(matches!(error, PerformUpdateError::Storage(_)));
        let retained = env
            .state
            .posts
            .get_post_by_id(
                post.post_id,
                &common::visibility::ViewerIdentity::local(user.user_id),
            )
            .await
            .expect("post loads")
            .expect("post remains after rollback");
        assert!(
            retained.published_at.is_some(),
            "failed enqueue rolls back the unpublish"
        );
    }

    #[cfg(feature = "test-utils")]
    #[apply(backends)]
    #[tokio::test]
    async fn update_enqueues_only_changes_touching_the_current_public_projection(
        #[case] backend: Backend,
    ) {
        let env = backend.setup().await;
        let user = SeedUser::new().seed(&env.state).await;
        let now: UtcInstant = "2042-07-01T12:00:00Z".parse().unwrap();
        let future = UtcInstant::from(now.value() + Duration::hours(1));
        let later_future = UtcInstant::from(now.value() + Duration::hours(2));
        let post = crate::test_support::SeedRawPost::new(user.user_id)
            .published_at(UtcInstant::from(now.value() - Duration::hours(1)))
            .audiences(vec![AudienceTarget::Public])
            .tags(["rust"])
            .seed(&env.state)
            .await;
        let mut feed_events = MockFeedEventStorage::new();
        feed_events
            .expect_enqueue_many()
            .times(3)
            .withf(|_, paths| paths.len() == 12)
            .returning(|_, _| Ok(()));
        let feed_events: Arc<dyn FeedEventStorage> = Arc::new(feed_events);
        let update = |body: &str, publish, audiences| PostUpdate {
            post_id: post.post_id,
            editor_user_id: user.user_id,
            body: parse_post_body(body),
            title: None,
            format: PostFormat::Markdown,
            slug_override: None,
            publish,
            summary: None,
            audiences,
            tags: Some(vec![parse_tag_label("rust")]),
            request_clock: now,
            expectations: PostBookkeepingExpectation::default(),
        };

        for input in [
            update("Private.", PublishUpdate::Publish { at: None }, vec![]),
            update(
                "Public again.",
                PublishUpdate::Publish { at: None },
                vec![AudienceTarget::Public],
            ),
            update(
                "Scheduled.",
                PublishUpdate::Publish { at: Some(future) },
                vec![AudienceTarget::Public],
            ),
            update(
                "Still scheduled.",
                PublishUpdate::Publish {
                    at: Some(later_future),
                },
                vec![AudienceTarget::Public],
            ),
            update(
                "Still scheduled.",
                PublishUpdate::Publish {
                    at: Some(later_future),
                },
                vec![AudienceTarget::Public],
            ),
        ] {
            confirmed(
                perform_post_update(
                    &env.state.write_scope,
                    &env.media_content_locks(),
                    Arc::clone(&env.state.posts),
                    Arc::clone(&feed_events),
                    input,
                )
                .await
                .expect("projection transition update succeeds"),
            );
        }
    }

    #[cfg(feature = "test-utils")]
    #[apply(backends)]
    #[tokio::test]
    async fn update_preserves_tags_without_a_transport_preread(#[case] backend: Backend) {
        let env = backend.setup().await;
        let user = SeedUser::new().seed(&env.state).await;
        let post = crate::test_support::SeedRawPost::new(user.user_id)
            .tags(["rust"])
            .seed(&env.state)
            .await;

        confirmed(
            perform_post_update(
                &env.state.write_scope,
                &env.media_content_locks(),
                Arc::clone(&env.state.posts),
                Arc::clone(&env.state.feed_events),
                PostUpdate {
                    post_id: post.post_id,
                    editor_user_id: user.user_id,
                    body: parse_post_body("Changed body."),
                    title: None,
                    format: PostFormat::Markdown,
                    slug_override: None,
                    publish: PublishUpdate::Publish { at: None },
                    summary: None,
                    audiences: vec![AudienceTarget::Public],
                    tags: None,
                    request_clock: UtcInstant::now(),
                    expectations: PostBookkeepingExpectation::default(),
                },
            )
            .await
            .expect("update without replacement tags succeeds"),
        );

        let record = env
            .state
            .posts
            .get_post_by_id(
                post.post_id,
                &common::visibility::ViewerIdentity::local(user.user_id),
            )
            .await
            .expect("post loads")
            .expect("post remains");
        assert_eq!(
            record
                .tags
                .into_iter()
                .map(|tag| tag.tag_slug)
                .collect::<Vec<_>>(),
            vec![parse_tag("rust")]
        );
    }

    #[cfg(feature = "test-utils")]
    #[apply(backends)]
    #[tokio::test]
    async fn feed_enqueue_failure_rolls_back_update_after_enqueuing_old_and_new_tag_feeds(
        #[case] backend: Backend,
    ) {
        let env = backend.setup().await;
        let seeded_user = SeedUser::new().seed(&env.state).await;
        let expected_tag_slugs = [parse_tag("old"), parse_tag("shared"), parse_tag("new")];
        let expected_feed_paths =
            feed::affected_feed_urls(&seeded_user.username, expected_tag_slugs.iter());
        let mut feed_events = crate::MockFeedEventStorage::new();
        feed_events
            .expect_enqueue_many()
            .withf(move |_, feed_paths| feed_paths == expected_feed_paths)
            .returning(|_, _| Err(crate::FeedEventError::Db(sqlx::Error::RowNotFound)));
        let feed_events: Arc<dyn FeedEventStorage> = Arc::new(feed_events);
        let post = crate::test_support::SeedRawPost::new(seeded_user.user_id)
            .tags(["old", "shared"])
            .seed(&env.state)
            .await;

        let error = perform_post_update(
            &env.state.write_scope,
            &env.media_content_locks(),
            Arc::clone(&env.state.posts),
            feed_events,
            PostUpdate {
                post_id: post.post_id,
                editor_user_id: seeded_user.user_id,
                body: parse_post_body("Changed body."),
                title: None,
                format: PostFormat::Markdown,
                slug_override: None,
                publish: PublishUpdate::Publish { at: None },
                summary: None,
                audiences: vec![AudienceTarget::Public],
                tags: Some(vec![parse_tag_label("shared"), parse_tag_label("new")]),
                request_clock: UtcInstant::now(),
                expectations: PostBookkeepingExpectation::default(),
            },
        )
        .await
        .expect_err("feed enqueue fails after the post update");
        assert!(matches!(error, PerformUpdateError::Storage(_)));

        let post = env
            .state
            .posts
            .get_post_by_id(
                post.post_id,
                &common::visibility::ViewerIdentity::local(seeded_user.user_id),
            )
            .await
            .expect("post loads")
            .expect("post remains");
        assert_eq!(post.body, "seed body");
        assert_eq!(
            post.tags
                .into_iter()
                .map(|tag| tag.tag_slug)
                .collect::<Vec<_>>(),
            vec![parse_tag("old"), parse_tag("shared")],
            "the enclosing write scope must roll back the earlier tag replacement"
        );
    }
    #[apply(backends)]
    #[tokio::test]
    async fn test_perform_post_creation_symbol_only_title_falls_back_to_post(
        #[case] backend: Backend,
    ) {
        let env = backend.setup().await;
        let seeded_user = SeedUser::new().seed(&env.state).await;
        let user_id = seeded_user.user_id;

        let storage = Arc::clone(&env.state.posts);
        let record = perform_post_creation(
            &env.state.write_scope,
            &env.media_content_locks(),
            Arc::clone(&storage),
            Arc::clone(&env.state.feed_events),
            PostCreation {
                user_id,
                body: parse_post_body("!!!"),
                title: None,
                format: PostFormat::Markdown,
                slug_override: None,
                published_at: None,
                max_attempts: 100,
                summary: None,
                audiences: vec![AudienceTarget::Public],
                tags: Vec::new(),
                idempotency_key: None,
                expectations: PostBookkeepingExpectation::default(),
            },
        )
        .await
        .unwrap();

        // Never hard-fails: a title with no usable characters lands on the
        // synthetic `post` fallback rather than NoSlugFromPost.
        let record = confirmed(record);
        assert_eq!(record.slug, "post");
    }

    #[apply(backends)]
    #[tokio::test]
    async fn test_perform_post_creation_unicode_title_preserves_slug(#[case] backend: Backend) {
        let env = backend.setup().await;
        let seeded_user = SeedUser::new().seed(&env.state).await;
        let user_id = seeded_user.user_id;

        let storage = Arc::clone(&env.state.posts);
        let record = perform_post_creation(
            &env.state.write_scope,
            &env.media_content_locks(),
            Arc::clone(&storage),
            Arc::clone(&env.state.feed_events),
            PostCreation {
                user_id,
                body: parse_post_body("# 日本語\n\nbody"),
                title: None,
                format: PostFormat::Markdown,
                slug_override: None,
                published_at: None,
                max_attempts: 100,
                summary: None,
                audiences: vec![AudienceTarget::Public],
                tags: Vec::new(),
                idempotency_key: None,
                expectations: PostBookkeepingExpectation::default(),
            },
        )
        .await
        .unwrap();

        let record = confirmed(record);
        assert_eq!(record.slug, "日本語");
    }

    #[test]
    fn candidate_slug_keeps_suffix_within_cap() {
        use common::slug::{MAX_SLUG_CHARS, Slug};
        // A seed already at the cap: the naive "{seed}-2" would be 82 chars and
        // be rejected by from_str; candidate_slug truncates the base to fit. The
        // seed is a valid Slug, so it is by construction ≤ MAX_SLUG_CHARS.
        let seed: Slug = parse_slug(&"a".repeat(MAX_SLUG_CHARS));
        // `unwrap` is itself the validity check: candidate_slug parses internally,
        // so a candidate exceeding the cap would fail here.
        let c = candidate_slug(&seed, 1).unwrap();
        assert!(c.chars().count() <= MAX_SLUG_CHARS);
        assert!(c.ends_with("-2"));

        // Truncation that would land on a '-' trims it so no "--" boundary forms:
        // an at-cap seed whose 78th char (the base cutoff for a "-2" suffix) is '-'.
        let seed2: Slug = parse_slug(&format!("{}-{}", "a".repeat(77), "b".repeat(2)));
        let c2 = candidate_slug(&seed2, 1).unwrap();
        assert!(c2.chars().count() <= MAX_SLUG_CHARS);
        assert!(!c2.contains("--"));

        // attempt 0 returns the seed unchanged.
        let hello: Slug = parse_slug("hello");
        assert_eq!(candidate_slug(&hello, 0).unwrap().as_ref(), "hello");
    }

    #[apply(backends)]
    #[tokio::test]
    async fn test_perform_post_creation_slug_conflict_retries(#[case] backend: Backend) {
        let env = backend.setup().await;
        let seeded_user = SeedUser::new().seed(&env.state).await;
        let user_id = seeded_user.user_id;

        let storage = Arc::clone(&env.state.posts);

        let r1 = perform_post_creation(
            &env.state.write_scope,
            &env.media_content_locks(),
            Arc::clone(&storage),
            Arc::clone(&env.state.feed_events),
            PostCreation {
                user_id,
                body: parse_post_body("Hello, world!"),
                title: None,
                format: PostFormat::Markdown,
                slug_override: None,
                published_at: None,
                max_attempts: 100,
                summary: None,
                audiences: vec![AudienceTarget::Public],
                tags: Vec::new(),
                idempotency_key: None,
                expectations: PostBookkeepingExpectation::default(),
            },
        )
        .await
        .unwrap();

        let r2 = perform_post_creation(
            &env.state.write_scope,
            &env.media_content_locks(),
            Arc::clone(&storage),
            Arc::clone(&env.state.feed_events),
            PostCreation {
                user_id,
                body: parse_post_body("Hello, world!"),
                title: None,
                format: PostFormat::Markdown,
                slug_override: None,
                published_at: None,
                max_attempts: 100,
                summary: None,
                audiences: vec![AudienceTarget::Public],
                tags: Vec::new(),
                idempotency_key: None,
                expectations: PostBookkeepingExpectation::default(),
            },
        )
        .await
        .unwrap();

        let r3 = perform_post_creation(
            &env.state.write_scope,
            &env.media_content_locks(),
            Arc::clone(&storage),
            Arc::clone(&env.state.feed_events),
            PostCreation {
                user_id,
                body: parse_post_body("Hello, world!"),
                title: None,
                format: PostFormat::Markdown,
                slug_override: None,
                published_at: None,
                max_attempts: 100,
                summary: None,
                audiences: vec![AudienceTarget::Public],
                tags: Vec::new(),
                idempotency_key: None,
                expectations: PostBookkeepingExpectation::default(),
            },
        )
        .await
        .unwrap();

        assert_eq!(confirmed(r1).slug, "hello-world");
        assert_eq!(confirmed(r2).slug, "hello-world-2");
        assert_eq!(confirmed(r3).slug, "hello-world-3");
    }

    #[apply(backends)]
    #[tokio::test]
    async fn bookkeeping_creation_expectations_follow_the_slug_retry_matrix(
        #[case] backend: Backend,
    ) {
        let env = backend.setup().await;
        let storage = Arc::clone(&env.state.posts);
        let expected = parse_slug("expected");
        let expected_second = parse_slug("expected-2");

        let create = |user_id, expectations| PostCreation {
            user_id,
            body: parse_post_body("Expected"),
            title: None,
            format: PostFormat::Markdown,
            slug_override: None,
            published_at: None,
            max_attempts: 10,
            summary: None,
            audiences: vec![AudienceTarget::Public],
            tags: Vec::new(),
            idempotency_key: None,
            expectations,
        };

        let first_free_user = SeedUser::new().seed(&env.state).await.user_id;
        let first_free = perform_post_creation(
            &env.state.write_scope,
            &env.media_content_locks(),
            Arc::clone(&storage),
            Arc::clone(&env.state.feed_events),
            create(
                first_free_user,
                PostBookkeepingExpectation {
                    slug: Some(expected.clone()),
                    ..Default::default()
                },
            ),
        )
        .await
        .unwrap();
        let first_free = confirmed(first_free);
        assert_eq!(first_free.slug, expected);

        let earlier_free_user = SeedUser::new().seed(&env.state).await.user_id;
        assert!(matches!(
            perform_post_creation(
                &env.state.write_scope,
                &env.media_content_locks(),
                Arc::clone(&storage),
                Arc::clone(&env.state.feed_events),
                create(
                    earlier_free_user,
                    PostBookkeepingExpectation {
                        slug: Some(expected_second.clone()),
                        ..Default::default()
                    },
                ),
            )
            .await,
            Err(PerformCreationError::BookkeepingMismatch)
        ));
        assert!(
            storage
                .list_collection_by_user(earlier_free_user, None, parse_row_limit("10"))
                .await
                .unwrap()
                .is_empty()
        );

        let conflict_before_expected_user = SeedUser::new().seed(&env.state).await.user_id;
        perform_post_creation(
            &env.state.write_scope,
            &env.media_content_locks(),
            Arc::clone(&storage),
            Arc::clone(&env.state.feed_events),
            create(
                conflict_before_expected_user,
                PostBookkeepingExpectation::default(),
            ),
        )
        .await
        .unwrap();
        let collision_winner = perform_post_creation(
            &env.state.write_scope,
            &env.media_content_locks(),
            Arc::clone(&storage),
            Arc::clone(&env.state.feed_events),
            create(
                conflict_before_expected_user,
                PostBookkeepingExpectation {
                    slug: Some(expected_second.clone()),
                    ..Default::default()
                },
            ),
        )
        .await
        .unwrap();
        let collision_winner = confirmed(collision_winner);
        assert_eq!(collision_winner.slug, expected_second);

        let occupied_expected_user = SeedUser::new().seed(&env.state).await.user_id;
        perform_post_creation(
            &env.state.write_scope,
            &env.media_content_locks(),
            Arc::clone(&storage),
            Arc::clone(&env.state.feed_events),
            create(
                occupied_expected_user,
                PostBookkeepingExpectation::default(),
            ),
        )
        .await
        .unwrap();
        assert!(matches!(
            perform_post_creation(
                &env.state.write_scope,
                &env.media_content_locks(),
                Arc::clone(&storage),
                Arc::clone(&env.state.feed_events),
                create(
                    occupied_expected_user,
                    PostBookkeepingExpectation {
                        slug: Some(expected),
                        ..Default::default()
                    },
                ),
            )
            .await,
            Err(PerformCreationError::BookkeepingMismatch)
        ));
        assert_eq!(
            storage
                .list_collection_by_user(occupied_expected_user, None, parse_row_limit("10"))
                .await
                .unwrap()
                .len(),
            1
        );
    }

    #[apply(backends)]
    #[tokio::test]
    async fn bookkeeping_creation_format_and_publication_mismatches_roll_back(
        #[case] backend: Backend,
    ) {
        use common::time::UtcInstant;

        let env = backend.setup().await;
        let storage = Arc::clone(&env.state.posts);

        let create = |user_id, expectations| PostCreation {
            user_id,
            body: parse_post_body("Mismatch"),
            title: None,
            format: PostFormat::Markdown,
            slug_override: None,
            published_at: None,
            max_attempts: 10,
            summary: None,
            audiences: vec![AudienceTarget::Public],
            tags: Vec::new(),
            idempotency_key: None,
            expectations,
        };

        let format_user = SeedUser::new().seed(&env.state).await.user_id;
        assert!(matches!(
            perform_post_creation(
                &env.state.write_scope,
                &env.media_content_locks(),
                Arc::clone(&storage),
                Arc::clone(&env.state.feed_events),
                create(
                    format_user,
                    PostBookkeepingExpectation {
                        format: Some(PostFormat::Org),
                        ..Default::default()
                    },
                ),
            )
            .await,
            Err(PerformCreationError::BookkeepingMismatch)
        ));
        assert!(
            storage
                .list_collection_by_user(format_user, None, parse_row_limit("10"))
                .await
                .unwrap()
                .is_empty()
        );

        let publication_user = SeedUser::new().seed(&env.state).await.user_id;
        assert!(matches!(
            perform_post_creation(
                &env.state.write_scope,
                &env.media_content_locks(),
                Arc::clone(&storage),
                Arc::clone(&env.state.feed_events),
                create(
                    publication_user,
                    PostBookkeepingExpectation {
                        published_at: Some(Some(UtcInstant::now())),
                        ..Default::default()
                    },
                ),
            )
            .await,
            Err(PerformCreationError::BookkeepingMismatch)
        ));
        assert!(
            storage
                .list_collection_by_user(publication_user, None, parse_row_limit("10"))
                .await
                .unwrap()
                .is_empty()
        );
    }

    #[apply(backends)]
    #[tokio::test]
    async fn bookkeeping_update_uses_final_draft_or_published_slug(#[case] backend: Backend) {
        let env = backend.setup().await;
        let storage = Arc::clone(&env.state.posts);
        let seeded_user = SeedUser::new().seed(&env.state).await;
        let user_id = seeded_user.user_id;

        let draft = crate::test_support::SeedRawPost::new(user_id)
            .draft()
            .seed(&env.state)
            .await;
        let published = crate::test_support::SeedRawPost::new(user_id)
            .seed(&env.state)
            .await;
        let changed_slug = parse_slug("changed-slug");
        let update = |post_id, expected_slug| PostUpdate {
            post_id,
            editor_user_id: user_id,
            body: parse_post_body("updated body"),
            title: None,
            format: PostFormat::Markdown,
            slug_override: Some(&changed_slug),
            publish: PublishUpdate::Publish { at: None },
            summary: None,
            audiences: vec![AudienceTarget::Public],
            tags: Some(Vec::new()),
            request_clock: UtcInstant::now(),
            expectations: PostBookkeepingExpectation {
                slug: Some(expected_slug),
                ..Default::default()
            },
        };

        let updated_draft = perform_post_update(
            &env.state.write_scope,
            &env.media_content_locks(),
            Arc::clone(&storage),
            Arc::clone(&env.state.feed_events),
            update(draft.post_id, changed_slug.clone()),
        )
        .await
        .unwrap();
        assert_eq!(confirmed(updated_draft).slug, changed_slug);

        let updated_published = perform_post_update(
            &env.state.write_scope,
            &env.media_content_locks(),
            Arc::clone(&storage),
            Arc::clone(&env.state.feed_events),
            update(published.post_id, published.slug.clone()),
        )
        .await
        .unwrap();
        assert_eq!(confirmed(updated_published).slug, published.slug);
    }

    #[apply(backends)]
    #[tokio::test]
    async fn bookkeeping_update_publishes_now_at_the_supplied_request_clock(
        #[case] backend: Backend,
    ) {
        let env = backend.setup().await;
        let storage = Arc::clone(&env.state.posts);
        let seeded_user = SeedUser::new().seed(&env.state).await;
        let user_id = seeded_user.user_id;

        let draft = crate::test_support::SeedRawPost::new(user_id)
            .draft()
            .seed(&env.state)
            .await;
        let clock: UtcInstant = "2042-07-01T12:00:00Z".parse().unwrap();
        let record = perform_post_update(
            &env.state.write_scope,
            &env.media_content_locks(),
            Arc::clone(&storage),
            Arc::clone(&env.state.feed_events),
            PostUpdate {
                post_id: draft.post_id,
                editor_user_id: user_id,
                body: parse_post_body("updated body"),
                title: None,
                format: PostFormat::Markdown,
                slug_override: None,
                publish: PublishUpdate::Publish { at: None },
                summary: None,
                audiences: vec![AudienceTarget::Public],
                tags: Some(Vec::new()),
                request_clock: clock,
                expectations: PostBookkeepingExpectation::default(),
            },
        )
        .await
        .unwrap();
        assert_eq!(confirmed(record).published_at, Some(clock));
    }

    #[apply(backends)]
    #[tokio::test]
    async fn bookkeeping_update_id_and_etag_mismatches_leave_the_post_unchanged(
        #[case] backend: Backend,
    ) {
        let env = backend.setup().await;
        let storage = Arc::clone(&env.state.posts);
        let seeded_user = SeedUser::new().seed(&env.state).await;
        let user_id = seeded_user.user_id;

        let post = crate::test_support::SeedRawPost::new(user_id)
            .draft()
            .seed(&env.state)
            .await;
        let viewer = common::visibility::ViewerIdentity::local(user_id);
        let original = storage
            .get_post_by_id(post.post_id, &viewer)
            .await
            .unwrap()
            .unwrap();
        let revision_count = env
            .base
            .pool()
            .scalar_i64("SELECT COUNT(*) FROM post_revisions")
            .await
            .unwrap();
        let update = |expectations| PostUpdate {
            post_id: post.post_id,
            editor_user_id: user_id,
            body: parse_post_body("changed body"),
            title: None,
            format: PostFormat::Markdown,
            slug_override: None,
            publish: PublishUpdate::Unpublish,
            summary: None,
            audiences: vec![AudienceTarget::Public],
            tags: Some(Vec::new()),
            request_clock: UtcInstant::now(),
            expectations,
        };

        assert!(matches!(
            perform_post_update(
                &env.state.write_scope,
                &env.media_content_locks(),
                Arc::clone(&storage),
                Arc::clone(&env.state.feed_events),
                update(PostBookkeepingExpectation {
                    post_id: Some(PostId::from(999_999)),
                    ..Default::default()
                }),
            )
            .await,
            Err(PerformUpdateError::BookkeepingMismatch)
        ));
        assert!(matches!(
            perform_post_update(
                &env.state.write_scope,
                &env.media_content_locks(),
                Arc::clone(&storage),
                Arc::clone(&env.state.feed_events),
                update(PostBookkeepingExpectation {
                    format: Some(PostFormat::Html),
                    ..Default::default()
                }),
            )
            .await,
            Err(PerformUpdateError::BookkeepingMismatch)
        ));
        assert!(matches!(
            perform_post_update(
                &env.state.write_scope,
                &env.media_content_locks(),
                Arc::clone(&storage),
                Arc::clone(&env.state.feed_events),
                update(PostBookkeepingExpectation {
                    published_at: Some(Some("2026-08-26T12:00:00Z".parse().unwrap())),
                    ..Default::default()
                }),
            )
            .await,
            Err(PerformUpdateError::BookkeepingMismatch)
        ));
        assert!(matches!(
            perform_post_update(
                &env.state.write_scope,
                &env.media_content_locks(),
                Arc::clone(&storage),
                Arc::clone(&env.state.feed_events),
                update(PostBookkeepingExpectation {
                    content_etag: Some(host::etag::sha256_of(b"stale")),
                    ..Default::default()
                }),
            )
            .await,
            Err(PerformUpdateError::StaleContent)
        ));
        let unchanged = storage
            .get_post_by_id(post.post_id, &viewer)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(unchanged.body, original.body);
        assert_eq!(
            env.base
                .pool()
                .scalar_i64("SELECT COUNT(*) FROM post_revisions")
                .await
                .unwrap(),
            revision_count
        );
    }
    #[apply(backends)]
    #[tokio::test]
    async fn test_perform_post_creation_slug_exhaustion(#[case] backend: Backend) {
        let env = backend.setup().await;
        let seeded_user = SeedUser::new().seed(&env.state).await;
        let user_id = seeded_user.user_id;

        let storage = Arc::clone(&env.state.posts);

        let r1 = perform_post_creation(
            &env.state.write_scope,
            &env.media_content_locks(),
            Arc::clone(&storage),
            Arc::clone(&env.state.feed_events),
            PostCreation {
                user_id,
                body: parse_post_body("Hello, world!"),
                title: None,
                format: PostFormat::Markdown,
                slug_override: None,
                published_at: None,
                max_attempts: 2,
                summary: None,
                audiences: vec![AudienceTarget::Public],
                tags: Vec::new(),
                idempotency_key: None,
                expectations: PostBookkeepingExpectation::default(),
            },
        )
        .await
        .unwrap();

        let r2 = perform_post_creation(
            &env.state.write_scope,
            &env.media_content_locks(),
            Arc::clone(&storage),
            Arc::clone(&env.state.feed_events),
            PostCreation {
                user_id,
                body: parse_post_body("Hello, world!"),
                title: None,
                format: PostFormat::Markdown,
                slug_override: None,
                published_at: None,
                max_attempts: 100,
                summary: None,
                audiences: vec![AudienceTarget::Public],
                tags: Vec::new(),
                idempotency_key: None,
                expectations: PostBookkeepingExpectation::default(),
            },
        )
        .await
        .unwrap();

        assert_eq!(confirmed(r1).slug, "hello-world");
        assert_eq!(confirmed(r2).slug, "hello-world-2");

        let err = perform_post_creation(
            &env.state.write_scope,
            &env.media_content_locks(),
            Arc::clone(&storage),
            Arc::clone(&env.state.feed_events),
            PostCreation {
                user_id,
                body: parse_post_body("Hello, world!"),
                title: None,
                format: PostFormat::Markdown,
                slug_override: None,
                published_at: None,
                max_attempts: 2,
                summary: None,
                audiences: vec![AudienceTarget::Public],
                tags: Vec::new(),
                idempotency_key: None,
                expectations: PostBookkeepingExpectation::default(),
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
        let seeded_user = SeedUser::new().seed(&env.state).await;
        let user_id = seeded_user.user_id;

        let storage = Arc::clone(&env.state.posts);
        // Title is derived from the original body's #+TITLE:, then the stored body is
        // canonicalized: the #+TITLE: line is stripped while #+FOO: and content stay.
        let record = perform_post_creation(
            &env.state.write_scope,
            &env.media_content_locks(),
            Arc::clone(&storage),
            Arc::clone(&env.state.feed_events),
            PostCreation {
                user_id,
                body: parse_post_body("#+TITLE: Hi\n#+FOO: x\n\nHello"),
                title: None,
                format: PostFormat::Org,
                slug_override: None,
                published_at: None,
                max_attempts: 100,
                summary: None,
                audiences: vec![AudienceTarget::Public],
                tags: Vec::new(),
                idempotency_key: None,
                expectations: PostBookkeepingExpectation::default(),
            },
        )
        .await
        .unwrap();

        let record = confirmed(record);
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
        let seeded_user = SeedUser::new().seed(&env.state).await;
        let user_id = seeded_user.user_id;

        let storage = Arc::clone(&env.state.posts);
        // Canonicalization runs on the update path too: a re-saved Org body has its
        // #+TITLE: stripped while an unrecognized #+FOO: and the content survive.
        let created = perform_post_creation(
            &env.state.write_scope,
            &env.media_content_locks(),
            Arc::clone(&storage),
            Arc::clone(&env.state.feed_events),
            PostCreation {
                user_id,
                body: parse_post_body("#+TITLE: First\n\noriginal"),
                title: None,
                format: PostFormat::Org,
                slug_override: None,
                published_at: None,
                max_attempts: 100,
                summary: None,
                audiences: vec![AudienceTarget::Public],
                tags: Vec::new(),
                idempotency_key: None,
                expectations: PostBookkeepingExpectation::default(),
            },
        )
        .await
        .unwrap();
        let created = confirmed(created);

        let record = perform_post_update(
            &env.state.write_scope,
            &env.media_content_locks(),
            Arc::clone(&storage),
            Arc::clone(&env.state.feed_events),
            PostUpdate {
                post_id: created.post_id,
                editor_user_id: user_id,
                body: parse_post_body("#+TITLE: Second\n#+FOO: keep\n\nupdated"),
                title: None,
                format: PostFormat::Org,
                slug_override: None,
                publish: PublishUpdate::Publish { at: None },
                summary: None,
                audiences: vec![AudienceTarget::Public],
                tags: Some(Vec::new()),
                request_clock: UtcInstant::now(),
                expectations: PostBookkeepingExpectation::default(),
            },
        )
        .await
        .unwrap();

        let record = confirmed(record);
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
    async fn perform_post_creation_rejects_title_only_org_body(#[case] backend: Backend) {
        let env = backend.setup().await;
        let seeded_user = SeedUser::new().seed(&env.state).await;
        let user_id = seeded_user.user_id;

        let storage = Arc::clone(&env.state.posts);

        // ADR-0024 canonicalization treats a leading `* heading` as the title *source*
        // and strips it, so this body leaves nothing to store (#811 decision 2).
        let creation = |body: &str, format: PostFormat| PostCreation {
            user_id,
            body: parse_post_body(body),
            title: None,
            format,
            slug_override: None,
            published_at: None,
            max_attempts: 100,
            summary: None,
            audiences: vec![AudienceTarget::Public],
            tags: Vec::new(),
            idempotency_key: None,
            expectations: PostBookkeepingExpectation::default(),
        };

        let err = perform_post_creation(
            &env.state.write_scope,
            &env.media_content_locks(),
            Arc::clone(&storage),
            Arc::clone(&env.state.feed_events),
            creation("* My Title\n", PostFormat::Org),
        )
        .await
        .expect_err("a title-only Org post has nothing left to store");
        assert!(matches!(err, PerformCreationError::EmptyPost), "{err:?}");

        // The discriminator: the same bytes are ordinary content in Markdown, so the
        // rejection is Org's title-stripping and not the `PostBody` parse.
        perform_post_creation(
            &env.state.write_scope,
            &env.media_content_locks(),
            Arc::clone(&storage),
            Arc::clone(&env.state.feed_events),
            creation("* My Title\n", PostFormat::Markdown),
        )
        .await
        .expect("the same bytes are content, not a title source, in Markdown");
    }

    #[apply(backends)]
    #[tokio::test]
    async fn perform_post_update_rejects_title_only_org_body(#[case] backend: Backend) {
        let env = backend.setup().await;
        let seeded_user = SeedUser::new().seed(&env.state).await;
        let user_id = seeded_user.user_id;

        let storage = Arc::clone(&env.state.posts);

        // The update path rejects it too — editing a post down to nothing but its title
        // is the same nonsense as creating one that way.
        let created = perform_post_creation(
            &env.state.write_scope,
            &env.media_content_locks(),
            Arc::clone(&storage),
            Arc::clone(&env.state.feed_events),
            PostCreation {
                user_id,
                body: parse_post_body("* My Title\n\nreal content"),
                title: None,
                format: PostFormat::Org,
                slug_override: None,
                published_at: None,
                max_attempts: 100,
                summary: None,
                audiences: vec![AudienceTarget::Public],
                tags: Vec::new(),
                idempotency_key: None,
                expectations: PostBookkeepingExpectation::default(),
            },
        )
        .await
        .unwrap();
        let created = confirmed(created);

        let err = perform_post_update(
            &env.state.write_scope,
            &env.media_content_locks(),
            Arc::clone(&storage),
            Arc::clone(&env.state.feed_events),
            PostUpdate {
                post_id: created.post_id,
                editor_user_id: user_id,
                body: parse_post_body("* My Title\n"),
                title: None,
                format: PostFormat::Org,
                slug_override: None,
                publish: PublishUpdate::Publish { at: None },
                summary: None,
                audiences: vec![AudienceTarget::Public],
                tags: Some(Vec::new()),
                request_clock: UtcInstant::now(),
                expectations: PostBookkeepingExpectation::default(),
            },
        )
        .await
        .expect_err("a title-only Org post has nothing left to store");
        assert!(matches!(err, PerformUpdateError::EmptyPost), "{err:?}");
    }

    #[apply(backends)]
    #[tokio::test]
    async fn test_perform_post_creation_markdown_body_keeps_its_heading(#[case] backend: Backend) {
        let env = backend.setup().await;
        let seeded_user = SeedUser::new().seed(&env.state).await;
        let user_id = seeded_user.user_id;

        let storage = Arc::clone(&env.state.posts);
        // Every format canonicalizes (#811); what distinguishes them is that only Org
        // treats its title source as a *header* and strips it. A Markdown `# H1` is
        // content and survives. Whitespace is canonicalized for both, hence the newline.
        let record = perform_post_creation(
            &env.state.write_scope,
            &env.media_content_locks(),
            Arc::clone(&storage),
            Arc::clone(&env.state.feed_events),
            PostCreation {
                user_id,
                body: parse_post_body("# H1\n\nBody text"),
                title: None,
                format: PostFormat::Markdown,
                slug_override: None,
                published_at: None,
                max_attempts: 100,
                summary: None,
                audiences: vec![AudienceTarget::Public],
                tags: Vec::new(),
                idempotency_key: None,
                expectations: PostBookkeepingExpectation::default(),
            },
        )
        .await
        .unwrap();

        let record = confirmed(record);
        assert_eq!(record.body, "# H1\n\nBody text\n");
    }

    #[apply(backends)]
    #[tokio::test]
    async fn test_perform_post_creation_org_title_rendered_once(#[case] backend: Backend) {
        let env = backend.setup().await;
        let seeded_user = SeedUser::new().seed(&env.state).await;
        let user_id = seeded_user.user_id;

        let storage = Arc::clone(&env.state.posts);
        // Double-title regression: the title text from the #+TITLE: line must not
        // survive into the stored body (hence rendered_html), so the page chrome's
        // title is the only place it appears. record.title still carries it.
        let record = perform_post_creation(
            &env.state.write_scope,
            &env.media_content_locks(),
            Arc::clone(&storage),
            Arc::clone(&env.state.feed_events),
            PostCreation {
                user_id,
                body: parse_post_body("#+TITLE: Distinct Headline\n\nParagraph body"),
                title: None,
                format: PostFormat::Org,
                slug_override: None,
                published_at: None,
                max_attempts: 100,
                summary: None,
                audiences: vec![AudienceTarget::Public],
                tags: Vec::new(),
                idempotency_key: None,
                expectations: PostBookkeepingExpectation::default(),
            },
        )
        .await
        .unwrap();

        let record = confirmed(record);
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
    fn creation_with_key(
        user_id: UserId,
        body: PostBody,
        key: Option<&IdempotencyKey>,
    ) -> PostCreation<'_> {
        PostCreation {
            user_id,
            body,
            title: None,
            format: PostFormat::Markdown,
            slug_override: None,
            published_at: Some(UtcInstant::now()),
            max_attempts: 100,
            summary: None,
            audiences: vec![AudienceTarget::Public],
            tags: Vec::new(),
            idempotency_key: key,
            expectations: PostBookkeepingExpectation::default(),
        }
    }

    fn parse_idempotency_key(key: &str) -> IdempotencyKey {
        key.parse().unwrap()
    }

    #[apply(backends)]
    #[tokio::test]
    async fn perform_post_creation_dedups_on_idempotency_key(#[case] backend: Backend) {
        let env = backend.setup().await;
        let seeded_user = SeedUser::new().seed(&env.state).await;
        let user_id = seeded_user.user_id;

        let storage = Arc::clone(&env.state.posts);
        let key = parse_idempotency_key("k");
        seed_media(&env.state, user_id, "original.jpg").await;
        seed_media(&env.state, user_id, "attempted.jpg").await;

        let first = perform_post_creation(
            &env.state.write_scope,
            &env.media_content_locks(),
            Arc::clone(&storage),
            Arc::clone(&env.state.feed_events),
            creation_with_key(
                user_id,
                parse_post_body(&format!("<img src=\"{}\">", media_url_for("original.jpg"))),
                Some(&key),
            ),
        )
        .await
        .unwrap();
        let first = confirmed(first);

        // Storage resolves the live mapping before any duplicate post, audience,
        // media, tag, or feed-event writes occur.
        let mut replay = creation_with_key(
            user_id,
            parse_post_body(&format!("<img src=\"{}\">", media_url_for("attempted.jpg"))),
            Some(&key),
        );
        replay.audiences = vec![AudienceTarget::Subscribers];
        let err = perform_post_creation(
            &env.state.write_scope,
            &env.media_content_locks(),
            Arc::clone(&storage),
            Arc::clone(&env.state.feed_events),
            replay,
        )
        .await
        .unwrap_err();
        assert!(matches!(
            err,
            PerformCreationError::IdempotencyConflict(post_id) if post_id == first.post_id
        ));

        let posts = storage
            .list_collection_by_user(user_id, None, parse_row_limit("50"))
            .await
            .unwrap();
        assert_eq!(posts.len(), 1);
        assert_eq!(posts[0].post_id, first.post_id);
        assert_eq!(
            fetch_post_media(&env.base, first.post_id).await,
            vec![(
                media_ref_for("original.jpg"),
                MediaReferenceKind::Local,
                media_url_for("original.jpg")
                    .parse::<MediaReferenceForm>()
                    .expect("valid media reference form"),
            )]
        );
        assert_eq!(
            storage.get_post_audiences(first.post_id).await.unwrap(),
            vec![AudienceTarget::Public]
        );
        for table in ["posts", "post_audiences", "post_media", "idempotency_keys"] {
            let row_count = match table {
                "posts" => env
                    .base
                    .pool()
                    .scalar_i64("SELECT COUNT(*) FROM posts")
                    .await
                    .unwrap(),
                "post_audiences" => env
                    .base
                    .pool()
                    .scalar_i64("SELECT COUNT(*) FROM post_audiences")
                    .await
                    .unwrap(),
                "post_media" => env
                    .base
                    .pool()
                    .scalar_i64("SELECT COUNT(*) FROM post_media")
                    .await
                    .unwrap(),
                "idempotency_keys" => env
                    .base
                    .pool()
                    .scalar_i64("SELECT COUNT(*) FROM idempotency_keys")
                    .await
                    .unwrap(),
                _ => unreachable!("fixed table set"),
            };
            assert_eq!(row_count, 1, "the conflicting create left a row in {table}");
        }
    }

    // -- sanitization (#445) --

    /// A malicious body driven through the real creation path: the persisted
    /// `rendered_html` must carry no active markup.
    ///
    /// Markdown rather than `Html` on purpose — `pulldown-cmark` passes embedded raw
    /// HTML through untouched, so this is the format where the hole was least
    /// obvious. Re-reads through `list_collection_by_user` as well as checking the
    /// returned record, so this covers what is actually *stored* (and therefore what
    /// every later viewer gets) rather than only what `render()` handed back.
    #[apply(backends)]
    #[tokio::test]
    async fn perform_post_creation_sanitizes_stored_rendered_html(#[case] backend: Backend) {
        let env = backend.setup().await;
        let seeded_user = SeedUser::new().seed(&env.state).await;
        let user_id = seeded_user.user_id;

        let storage = Arc::clone(&env.state.posts);

        let record = perform_post_creation(
            &env.state.write_scope,
            &env.media_content_locks(),
            Arc::clone(&storage),
            Arc::clone(&env.state.feed_events),
            creation_with_key(
                user_id,
                parse_post_body(
                    "Hello\n\n<script>alert(1)</script>\n\n<img src=\"x\" onerror=\"alert(1)\">",
                ),
                None,
            ),
        )
        .await
        .unwrap();

        let record = confirmed(record);
        let html = &record.rendered_html;
        assert!(!html.contains("<script"), "{html}");
        assert!(!html.contains("onerror"), "{html}");
        assert!(html.contains("Hello"), "benign content was lost: {html}");

        let posts = storage
            .list_collection_by_user(user_id, None, parse_row_limit("50"))
            .await
            .unwrap();
        let stored = &posts[0].rendered_html;
        assert!(!stored.contains("<script"), "stored: {stored}");
        assert!(!stored.contains("onerror"), "stored: {stored}");
    }

    #[apply(backends)]
    #[tokio::test]
    async fn post_id_for_idempotency_key_maps(#[case] backend: Backend) {
        let env = backend.setup().await;
        let seeded_user = SeedUser::new().seed(&env.state).await;
        let user_id = seeded_user.user_id;

        let storage = Arc::clone(&env.state.posts);
        let key = parse_idempotency_key("k");
        let missing_key = parse_idempotency_key("unknown");

        let record = perform_post_creation(
            &env.state.write_scope,
            &env.media_content_locks(),
            Arc::clone(&storage),
            Arc::clone(&env.state.feed_events),
            creation_with_key(user_id, parse_post_body("Body"), Some(&key)),
        )
        .await
        .unwrap();

        let mapped = storage
            .post_id_for_idempotency_key(user_id, &key, UtcInstant::now())
            .await
            .unwrap();
        let record = confirmed(record);
        assert_eq!(mapped, Some(record.post_id));

        let missing = storage
            .post_id_for_idempotency_key(user_id, &missing_key, UtcInstant::now())
            .await
            .unwrap();
        assert_eq!(missing, None);
    }

    #[apply(backends)]
    #[tokio::test]
    async fn create_rendered_post_preserves_indeterminate_commit_without_reporting_success(
        #[case] backend: Backend,
    ) {
        let env = backend.setup().await;
        let user_id = SeedUser::new().seed(&env.state).await.user_id;
        let storage = Arc::clone(&env.state.posts);
        let key = parse_idempotency_key("indeterminate-commit-key");
        let created_at = UtcInstant::from(
            Utc.with_ymd_and_hms(2026, 8, 31, 12, 0, 0)
                .single()
                .expect("fixed instant"),
        );
        let cutoff = UtcInstant::from(created_at.value() + Duration::hours(1));

        confirmed(
            perform_post_creation_at(
                &env.state.write_scope,
                &env.media_content_locks(),
                Arc::clone(&storage),
                Arc::clone(&env.state.feed_events),
                created_at,
                creation_with_key(user_id, parse_post_body("original body"), Some(&key)),
            )
            .await
            .expect("original keyed create"),
        );

        let outcome = create_rendered_post(
            &env.state
                .write_scope
                .with_commit_acknowledgement_loss_after_commit_for_test(),
            &env.media_content_locks(),
            Arc::clone(&storage),
            Arc::clone(&env.state.feed_events),
            RenderedPostContent {
                user_id,
                title: None,
                slug: parse_slug("indeterminate-commit"),
                body: parse_post_body("indeterminate body"),
                format: PostFormat::Markdown,
                published_at: Some(cutoff),
                summary: None,
                audiences: vec![AudienceTarget::Public],
                tags: Vec::new(),
                idempotency_key: Some(key.clone()),
                expectations: PostBookkeepingExpectation::default(),
            },
            cutoff,
        )
        .await
        .expect("a lost commit acknowledgement is a mutation outcome");
        let MutationOutcome::CommitIndeterminate(record) = outcome else {
            panic!("lost commit acknowledgement must not be reported as a confirmed creation");
        };

        assert_eq!(record.user_id, user_id);
        assert_eq!(record.slug, parse_slug("indeterminate-commit"));
        assert_eq!(record.body, parse_post_body("indeterminate body"));
        assert_eq!(record.published_at, Some(cutoff));
        assert_eq!(
            storage
                .post_id_for_idempotency_key(user_id, &key, cutoff)
                .await
                .expect("replacement idempotency mapping"),
            Some(record.post_id),
            "the indeterminate creation's durable side effects must survive"
        );

        let stored = storage
            .get_post_by_id(
                record.post_id,
                &common::visibility::ViewerIdentity::local(user_id),
            )
            .await
            .expect("load indeterminate creation")
            .expect("indeterminate commit still created the post");
        assert_eq!(stored.post_id, record.post_id);
        assert_eq!(stored.user_id, record.user_id);
        assert_eq!(stored.author_username, record.author_username);
        assert_eq!(stored.title, record.title);
        assert_eq!(stored.slug, record.slug);
        assert_eq!(stored.body, record.body);
        assert_eq!(stored.format, record.format);
        assert_eq!(stored.rendered_html, record.rendered_html);
        assert_eq!(stored.created_at, record.created_at);
        assert_eq!(stored.updated_at, record.updated_at);
        assert_eq!(stored.published_at, record.published_at);
        assert_eq!(stored.deleted_at, record.deleted_at);
        assert_eq!(stored.summary, record.summary);
        assert!(stored.tags.is_empty());
        assert!(record.tags.is_empty());
    }

    #[apply(backends)]
    #[tokio::test]
    async fn idempotency_mapping_expires_at_the_inclusive_cutoff_and_prunes(
        #[case] backend: Backend,
    ) {
        let env = backend.setup().await;
        let user_id = SeedUser::new().seed(&env.state).await.user_id;
        let storage = Arc::clone(&env.state.posts);
        let key = parse_idempotency_key("retained-key");
        let created_at = UtcInstant::from(
            Utc.with_ymd_and_hms(2026, 8, 31, 12, 0, 0)
                .single()
                .expect("fixed instant"),
        );
        let cutoff = UtcInstant::from(created_at.value() + Duration::hours(1));

        let first = confirmed(
            perform_post_creation_at(
                &env.state.write_scope,
                &env.media_content_locks(),
                Arc::clone(&storage),
                Arc::clone(&env.state.feed_events),
                created_at,
                creation_with_key(user_id, parse_post_body("first body"), Some(&key)),
            )
            .await
            .expect("first keyed create"),
        );

        assert_eq!(
            storage
                .post_id_for_idempotency_key(
                    user_id,
                    &key,
                    UtcInstant::from(cutoff.value() - Duration::seconds(1)),
                )
                .await
                .expect("pre-cutoff lookup"),
            Some(first.post_id)
        );
        crate::with_closeable_pool!(env.base.pool(), pool, {
            sqlx::query("UPDATE posts SET deleted_at = created_at WHERE post_id = $1")
                .bind_storage(first.post_id)
                .execute(pool)
                .await
                .map(|_| ())
        })
        .expect("soft-delete original Post");
        assert_eq!(
            storage
                .post_id_for_idempotency_key(user_id, &key, cutoff)
                .await
                .expect("cutoff lookup"),
            None
        );

        let replacement = confirmed(
            perform_post_creation_at(
                &env.state.write_scope,
                &env.media_content_locks(),
                Arc::clone(&storage),
                Arc::clone(&env.state.feed_events),
                cutoff,
                creation_with_key(user_id, parse_post_body("replacement body"), Some(&key)),
            )
            .await
            .expect("cutoff reuse creates a replacement"),
        );
        assert_ne!(replacement.post_id, first.post_id);
        assert_eq!(
            crate::with_closeable_pool!(env.base.pool(), pool, {
                sqlx::query_scalar::<_, i64>(
                    "SELECT COUNT(*) FROM posts WHERE post_id = $1 AND deleted_at IS NOT NULL",
                )
                .bind_storage(first.post_id)
                .fetch_one(pool)
                .await
            })
            .expect("inspect original Post tombstone"),
            1,
            "idempotency expiry must not alter a Deleted Post"
        );
        assert_eq!(
            storage
                .prune_expired_idempotency_keys(UtcInstant::from(
                    cutoff.value() + Duration::hours(1)
                ))
                .await
                .expect("prune expired mapping"),
            1
        );
        assert_eq!(
            storage
                .post_id_for_idempotency_key(
                    user_id,
                    &key,
                    UtcInstant::from(cutoff.value() + Duration::hours(1)),
                )
                .await
                .expect("lookup after pruning"),
            None
        );
    }

    #[apply(backends)]
    #[tokio::test]
    async fn concurrent_exact_cutoff_reuse_creates_one_replacement(#[case] backend: Backend) {
        let env = backend.setup().await;
        let user_id = SeedUser::new().seed(&env.state).await.user_id;
        let storage = Arc::clone(&env.state.posts);
        let key = parse_idempotency_key("concurrent-retained-key");
        let created_at: UtcInstant = "2026-08-31T12:00:00Z".parse().expect("fixed instant");
        let cutoff = UtcInstant::from(created_at.value() + Duration::hours(1));

        let original = confirmed(
            perform_post_creation_at(
                &env.state.write_scope,
                &env.media_content_locks(),
                Arc::clone(&storage),
                Arc::clone(&env.state.feed_events),
                created_at,
                creation_with_key(user_id, parse_post_body("original"), Some(&key)),
            )
            .await
            .expect("original keyed create"),
        );

        let first_locks = env.media_content_locks();
        let second_locks = env.media_content_locks();
        let first_attempt = perform_post_creation_at(
            &env.state.write_scope,
            &first_locks,
            Arc::clone(&storage),
            Arc::clone(&env.state.feed_events),
            cutoff,
            creation_with_key(user_id, parse_post_body("replacement one"), Some(&key)),
        );
        let second_attempt = perform_post_creation_at(
            &env.state.write_scope,
            &second_locks,
            Arc::clone(&storage),
            Arc::clone(&env.state.feed_events),
            cutoff,
            creation_with_key(user_id, parse_post_body("replacement two"), Some(&key)),
        );
        let outcomes = tokio::join!(first_attempt, second_attempt);
        let (replacement, replayed_post_id) = match outcomes {
            (Ok(outcome), Err(PerformCreationError::IdempotencyConflict(post_id)))
            | (Err(PerformCreationError::IdempotencyConflict(post_id)), Ok(outcome)) => {
                (confirmed(outcome), post_id)
            }
            other => panic!("expected one replacement and one replay decision, got {other:?}"),
        };
        assert_eq!(
            replayed_post_id, replacement.post_id,
            "the losing request must retain the winner chosen inside its transaction"
        );

        assert_ne!(replacement.post_id, original.post_id);
        assert_eq!(
            storage
                .post_id_for_idempotency_key(
                    user_id,
                    &key,
                    UtcInstant::from(cutoff.value() + Duration::seconds(1)),
                )
                .await
                .expect("replacement mapping"),
            Some(replacement.post_id)
        );
        assert_eq!(
            env.base
                .pool()
                .scalar_i64("SELECT COUNT(*) FROM posts")
                .await
                .expect("count durable Posts"),
            2
        );
    }

    #[apply(backends)]
    #[tokio::test]
    async fn idempotency_key_is_per_user(#[case] backend: Backend) {
        let env = backend.setup().await;
        let user_a = SeedUser::new().seed(&env.state).await.user_id;
        let user_b = SeedUser::new().seed(&env.state).await.user_id;
        let storage = Arc::clone(&env.state.posts);
        let key = parse_idempotency_key("k");

        // The same key string from two users creates two independent posts.
        let post_a = perform_post_creation(
            &env.state.write_scope,
            &env.media_content_locks(),
            Arc::clone(&storage),
            Arc::clone(&env.state.feed_events),
            creation_with_key(user_a, parse_post_body("A body"), Some(&key)),
        )
        .await
        .unwrap();
        let post_b = perform_post_creation(
            &env.state.write_scope,
            &env.media_content_locks(),
            Arc::clone(&storage),
            Arc::clone(&env.state.feed_events),
            creation_with_key(user_b, parse_post_body("B body"), Some(&key)),
        )
        .await
        .unwrap();
        let post_a = confirmed(post_a);
        let post_b = confirmed(post_b);
        assert_ne!(post_a.post_id, post_b.post_id);

        assert_eq!(
            storage
                .post_id_for_idempotency_key(user_a, &key, UtcInstant::now())
                .await
                .unwrap(),
            Some(post_a.post_id)
        );
        assert_eq!(
            storage
                .post_id_for_idempotency_key(user_b, &key, UtcInstant::now())
                .await
                .unwrap(),
            Some(post_b.post_id)
        );
    }

    #[test]
    fn idempotency_conflict_converts_to_internal_error() {
        use host::error::{ErrorKind, InternalError};

        // Covers the non-AtomPub conversion arm; the AtomPub handler normally
        // intercepts the replay decision first.
        let err: InternalError = PerformCreationError::IdempotencyConflict(PostId::from(42)).into();
        assert_eq!(err.kind(), ErrorKind::Internal);
        assert_eq!(err.public_message(), "server operation failed");
    }

    #[test]
    fn test_perform_creation_error_display_and_debug() {
        let err = PerformCreationError::EmptyPost;
        assert_eq!(
            err.to_string(),
            "post body is only its title, leaving nothing to store"
        );
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
    fn perform_update_error_empty_post_display() {
        let err = PerformUpdateError::EmptyPost;
        assert_eq!(
            err.to_string(),
            "post body is only its title, leaving nothing to store"
        );
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

    // Each arm maps to a fixed `(kind, public_message)` pair.
    #[test]
    fn from_perform_update_error_maps_variants() {
        use host::error::{ErrorKind, InternalError};

        let empty: InternalError = PerformUpdateError::EmptyPost.into();
        assert_eq!(empty.kind(), ErrorKind::Validation);
        assert_eq!(
            empty.public_message(),
            "post body is only its title, leaving nothing to store"
        );

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

    // Each arm maps to a fixed `(kind, public_message)` pair; the invalid-slug
    // arm preserves the typed source.
    #[test]
    fn from_perform_creation_error_maps_variants() {
        use host::error::{ErrorKind, InternalError};

        let empty: InternalError = PerformCreationError::EmptyPost.into();
        assert_eq!(empty.kind(), ErrorKind::Validation);
        assert_eq!(
            empty.public_message(),
            "post body is only its title, leaving nothing to store"
        );

        let invalid_slug: InternalError =
            PerformCreationError::InvalidSlug(common::slug::InvalidSlug).into();
        assert_eq!(invalid_slug.kind(), ErrorKind::Validation);
        assert_eq!(
            invalid_slug.public_message(),
            common::slug::InvalidSlug.to_string()
        );
        // The typed slug error is preserved on the operator side, not flattened.
        assert!(
            invalid_slug
                .operator_message()
                .contains(&common::slug::InvalidSlug.to_string())
        );

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
