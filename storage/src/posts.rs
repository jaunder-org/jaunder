//! Content storage for posts, revisions, and tagging.

use async_trait::async_trait;
use chrono::{DateTime, Datelike, NaiveDate, Utc};
use sqlx::{Database, Pool};
use thiserror::Error;

use common::feed::FeedPath;
use common::ids::{AudienceId, PostId, RevisionId, TagId, UserId};
use common::post_body::PostBody;
use common::post_summary::PostSummary;
use common::post_title::PostTitle;
use common::slug::Slug;
use common::tag::{Tag, TagLabel};
use common::username::Username;
use common::visibility::{AudienceTarget, TargetKind, ViewerIdentity};
use host::error::{InternalError, InternalResult};

use crate::backend::Backend;
use crate::helpers::{post_record_from_row, PostRow};

pub use common::render::{InvalidPostFormat, PostFormat, RenderedHtml};

/// The `year`/`month`/`day` component of a public permalink lookup key. Bundling
/// the date triple keeps [`PostStorage::get_post_by_permalink`] under the
/// argument limit while naming the trio at every call site.
#[derive(Debug, Clone, Copy)]
pub struct PermalinkDate {
    /// Four-digit calendar year.
    pub year: i32,
    /// Month of year, 1–12.
    pub month: u32,
    /// Day of month, 1–31.
    pub day: u32,
}

/// A post record returned by [`PostStorage`] queries.
///
/// `tags` is populated by the same query that loads the rest of the row via
/// a JSON-aggregating subquery, so post and tag state are always read from
/// the same statement-level snapshot. `author_username` is sourced from the
/// `users` table in the same query (via JOIN or correlated subquery), so
/// callers never need a second roundtrip to look up the post's author.
#[derive(Clone, Debug)]
pub struct PostRecord {
    /// Unique internal identifier.
    pub post_id: PostId,
    /// ID of the user who owns the post.
    pub user_id: UserId,
    /// Username of the author
    pub author_username: Username,
    /// Optional title.
    pub title: Option<PostTitle>,
    /// Unique slug (per user, per day).
    pub slug: Slug,
    /// Raw source body (Markdown or Org).
    pub body: PostBody,
    /// Format of the `body`.
    pub format: PostFormat,
    /// HTML produced by `render()` from the `body`. A provenance marker, **not**
    /// a safety guarantee — `render()` does not sanitize (see #445).
    pub rendered_html: RenderedHtml,
    /// When the post was first created.
    pub created_at: DateTime<Utc>,
    /// When the post was last updated.
    pub updated_at: DateTime<Utc>,
    /// When the post was published (None if it is a draft).
    pub published_at: Option<DateTime<Utc>>,
    /// When the post was soft-deleted (None if active).
    pub deleted_at: Option<DateTime<Utc>>,
    /// Optional summary/excerpt of the post.
    pub summary: Option<PostSummary>,
    pub tags: Vec<PostTag>,
}

impl PostRecord {
    /// Returns the canonical permalink for this post.
    /// Uses the publication timestamp if published; otherwise falls back to the creation timestamp.
    #[must_use]
    pub fn permalink(&self) -> String {
        use chrono::Datelike;
        let timestamp = self.published_at.unwrap_or(self.created_at);
        format!(
            "/~{}/{:04}/{:02}/{:02}/{}",
            self.author_username,
            timestamp.year(),
            timestamp.month(),
            timestamp.day(),
            self.slug.as_ref()
        )
    }

    /// Generates a fallback summary from the post's body, title, or slug. The
    /// fallback chain always yields a non-empty label (first non-empty body line →
    /// title → slug), which [`PostSummary::truncated`] length-caps into the newtype.
    pub fn fallback_summary_label(&self) -> PostSummary {
        let label = self
            .body
            .lines()
            .map(str::trim)
            .find(|line| !line.is_empty())
            .map(|line| line.chars().take(100).collect::<String>())
            .filter(|line| !line.is_empty())
            // Guard the title branch too: `PostTitle` is infallible and may be
            // empty-after-trim, so fall through to the always-non-empty slug rather
            // than feed `truncated` an empty label (its one invariant gap).
            .or_else(|| {
                self.title
                    .clone()
                    .map(String::from)
                    .filter(|t| !t.trim().is_empty())
            })
            .unwrap_or_else(|| self.slug.to_string());
        PostSummary::truncated(&label)
    }
}

/// A post revision record returned by [`PostStorage`] queries.
///
/// Revisions are created automatically whenever a post is updated.
#[derive(Clone, Debug)]
pub struct PostRevisionRecord {
    /// Unique identifier for this revision.
    pub revision_id: RevisionId,
    /// ID of the associated post.
    pub post_id: PostId,
    /// ID of the user who made the edit.
    pub user_id: UserId,
    /// Title at the time of this revision.
    pub title: Option<PostTitle>,
    /// Slug at the time of this revision.
    pub slug: Slug,
    /// Raw source body at the time of this revision.
    pub body: PostBody,
    /// Format at the time of this revision.
    pub format: PostFormat,
    /// HTML produced by `render()` at the time of this revision. A provenance
    /// marker, **not** a safety guarantee — `render()` does not sanitize (see #445).
    pub rendered_html: RenderedHtml,
    /// When this revision was created.
    pub edited_at: DateTime<Utc>,
}

/// Errors that can occur when creating a post.
#[derive(Debug, Error)]
pub enum CreatePostError {
    /// A post with the same slug already exists for this user on this day.
    #[error("slug already taken for this user on this date")]
    SlugConflict,
    /// The `(user_id, idempotency_key)` pair has already been used to create a
    /// post; the create is a duplicate of an earlier one.
    #[error("idempotency key already used for this user")]
    IdempotencyConflict,
    /// An unexpected database error occurred.
    #[error(transparent)]
    Internal(#[from] sqlx::Error),
}

/// Errors that can occur when updating a post.
#[derive(Debug, Error)]
pub enum UpdatePostError {
    /// The requested post does not exist.
    #[error("post not found")]
    NotFound,
    /// The user is not authorized to edit this post.
    #[error("not authorized")]
    Unauthorized,
    /// An unexpected database error occurred.
    #[error(transparent)]
    Internal(#[from] sqlx::Error),
}

impl From<UpdatePostError> for host::error::InternalError {
    /// Reproduces the former inline `web::posts::mod` mapper
    /// `(kind, class, public_message)`: not-found/unauthorized mask as a 404;
    /// an internal failure is a masked storage error.
    fn from(error: UpdatePostError) -> Self {
        use host::error::InternalError;
        match error {
            UpdatePostError::NotFound | UpdatePostError::Unauthorized => {
                InternalError::not_found("Post")
            }
            UpdatePostError::Internal(e) => InternalError::storage(e),
        }
    }
}

/// Cursor for keyset pagination of post listings.
#[derive(Debug)]
pub struct PostCursor {
    /// Creation timestamp of the last item in the previous page.
    pub created_at: DateTime<Utc>,
    /// ID of the last item in the previous page (used for stable ordering).
    pub post_id: PostId,
}

/// Cursor for keyset pagination of the editor-facing per-user collection
/// (ordered by `updated_at DESC, post_id DESC`).
#[derive(Clone, Copy, Debug)]
pub struct CollectionCursor {
    /// Update timestamp of the last item in the previous page.
    pub updated_at: DateTime<Utc>,
    /// ID of the last item in the previous page (used for stable ordering).
    pub post_id: PostId,
}

/// Input for creating a new post.
#[derive(Clone)]
pub struct CreatePostInput {
    pub user_id: UserId,
    pub title: Option<PostTitle>,
    pub slug: Slug,
    pub body: PostBody,
    pub format: PostFormat,
    pub rendered_html: RenderedHtml,
    /// If Some, the post is created in a published state.
    pub published_at: Option<DateTime<Utc>>,
    /// Optional summary/excerpt of the post.
    pub summary: Option<PostSummary>,
    /// Audience targeting for the post. Each entry becomes a `post_audiences`
    /// row; `Private` and an empty vec produce no rows (the post is private).
    pub audiences: Vec<AudienceTarget>,
    /// If `Some`, register this idempotency key against the new post in the
    /// same transaction. A `(user_id, key)` collision maps to
    /// [`CreatePostError::IdempotencyConflict`] and rolls the whole create back.
    pub idempotency_key: Option<String>,
}

/// Input for updating an existing post.
#[derive(Clone)]
pub struct UpdatePostInput {
    pub title: Option<PostTitle>,
    /// The new slug. Note: Slugs are typically immutable once published.
    pub slug: Slug,
    pub body: PostBody,
    pub format: PostFormat,
    pub rendered_html: RenderedHtml,
    /// If `true`, clear `published_at` back to NULL (draft / unschedule). Takes
    /// precedence over `explicit_published_at`.
    pub unpublish: bool,
    /// An exact publication instant to store (future = scheduled, past =
    /// backdated). `None` keeps any existing timestamp, or stamps `now` for a
    /// previously-unpublished post. Ignored when `unpublish` is `true`.
    pub explicit_published_at: Option<DateTime<Utc>>,
    /// Optional summary/excerpt of the post.
    pub summary: Option<PostSummary>,
    /// Audience targeting for the post. On update the existing
    /// `post_audiences` rows are replaced to match this vec; `Private` and an
    /// empty vec produce no rows (the post is private).
    pub audiences: Vec<AudienceTarget>,
}

/// A tag record returned by [`PostStorage`] tag queries.
#[derive(Clone, Debug)]
pub struct TagRecord {
    pub tag_id: TagId,
    pub tag_slug: Tag,
}

/// A post-tag association returned by [`PostStorage`] tag queries.
#[derive(Clone, Debug)]
pub struct PostTag {
    pub post_id: PostId,
    pub tag_id: TagId,
    pub tag_slug: Tag,
    /// The original case-sensitive display name of the tag.
    pub tag_display: TagLabel,
}

/// A post that crossed into "live" within a time window, carrying exactly the
/// data the feed worker needs to compute its affected feed URLs (the author's
/// username and the post's tag slugs).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct GoLivePost {
    pub username: Username,
    pub tag_slugs: Vec<Tag>,
}

/// The slug-level difference between a post's existing tags and a desired set
/// of display tokens, as computed by [`post_tag_diff`].
///
/// Borrows from both inputs; callers perform the actual `tag_post`/`untag_post`
/// writes with their own error mapping.
pub struct PostTagDiff<'a> {
    /// Labels to add (their slug is not already present on the post).
    pub to_add: Vec<&'a TagLabel>,
    /// Existing tags to remove (their slug is not in the desired set).
    pub to_remove: Vec<&'a Tag>,
}

/// Diffs a post's `existing` tags against a `desired` set of [`TagLabel`]s.
///
/// Tagging is keyed on slug, so a desired label is "to add" only when no
/// existing tag shares its slug, and an existing tag is "to remove" only when
/// no desired label maps to its slug. Each `desired` label is already valid (its
/// `FromStr` ran at the boundary), so nothing is skipped here. Re-applying an
/// existing tag with different display casing is a no-op (the existing row's
/// casing is preserved by storage).
///
/// This is the pure core shared by the `web` and `server`/`AtomPub` front-ends;
/// each applies the result with its own error type.
#[must_use]
pub fn post_tag_diff<'a>(existing: &'a [PostTag], desired: &'a [TagLabel]) -> PostTagDiff<'a> {
    use std::collections::HashSet;

    let existing_slugs: HashSet<Tag> = existing.iter().map(|t| t.tag_slug.clone()).collect();
    let desired_slugs: HashSet<Tag> = desired.iter().map(TagLabel::slug).collect();

    let to_add = desired
        .iter()
        .filter(|label| !existing_slugs.contains(&label.slug()))
        .collect();
    let to_remove = existing
        .iter()
        .filter(|tag| !desired_slugs.contains(&tag.tag_slug))
        .map(|tag| &tag.tag_slug)
        .collect();

    PostTagDiff { to_add, to_remove }
}

/// Errors that can occur when tagging a post.
#[derive(Debug, Error)]
pub enum TaggingError {
    /// The target post does not exist.
    #[error("post not found")]
    PostNotFound,
    /// The specified tag does not exist.
    #[error("tag not found")]
    TagNotFound,
    /// The post is already associated with this tag.
    #[error("post is already tagged with this tag")]
    AlreadyTagged,
    /// An unexpected database error occurred.
    #[error(transparent)]
    Internal(#[from] sqlx::Error),
}

impl From<TaggingError> for host::error::InternalError {
    /// Preserves the current wire class of the `tag_post`/`untag_post` lift:
    /// the former `web` sites used `InternalError::server_message(e.to_string())`
    /// (kind `Internal`, public `"server operation failed"`). Routing through
    /// `server` keeps that projection while carrying the typed `TaggingError`
    /// as the operator-side source instead of stringifying it (A19).
    fn from(error: TaggingError) -> Self {
        host::error::InternalError::server(error)
    }
}

/// Errors that can occur when listing posts by tag.
#[derive(Debug, Error)]
pub enum ListByTagError {
    /// The specified tag does not exist.
    #[error("tag not found")]
    TagNotFound,
    /// An unexpected database error occurred.
    #[error(transparent)]
    Internal(#[from] sqlx::Error),
}

// ---------------------------------------------------------------------------
// Cursor + effectful post orchestration
//
// Cursor (de)serialization plus the effectful read/tag helpers shared by
// `web`'s `#[server]` bodies and the `server` crate's public projector. They
// take `&dyn PostStorage`/`PostRecord`/`PostCursor` — storage abstractions the
// `host` floor cannot name — so they home here in `storage`, returning
// `host::error::InternalError` where fallible.
// ---------------------------------------------------------------------------

/// Projects a [`PostRecord`] onto the keyset [`PostCursor`] that paginates the
/// listing after it.
#[must_use]
pub fn to_post_cursor(post: &PostRecord) -> PostCursor {
    PostCursor {
        created_at: post.created_at,
        post_id: post.post_id,
    }
}

/// Parses the wire cursor pair into a [`PostCursor`]. Both components must be
/// supplied together (an opaque paging token) or both absent (the first page).
///
/// # Errors
///
/// Returns a validation error if only one component is present.
pub fn parse_post_cursor(
    cursor_created_at: Option<DateTime<Utc>>,
    cursor_post_id: Option<PostId>,
) -> InternalResult<Option<PostCursor>> {
    match (cursor_created_at, cursor_post_id) {
        (None, None) => Ok(None),
        (Some(created_at), Some(post_id)) => Ok(Some(PostCursor {
            created_at,
            post_id,
        })),
        _ => Err(InternalError::validation(
            "cursor_created_at and cursor_post_id must be provided together",
        )),
    }
}

/// Diff the existing tag set against `desired` (a Vec of validated display
/// tokens) and apply the difference: `tag_post` for new entries, `untag_post`
/// for removed entries. Re-applying an existing tag with new display casing
/// is a no-op at the slug level (the storage layer keys on slug); the
/// display casing of the existing row is preserved.
///
/// # Errors
///
/// Returns a storage error if the existing tags cannot be read, or a server
/// error (via `From<TaggingError>`) if a `tag_post`/`untag_post` write fails.
pub async fn apply_post_tag_diff(
    posts: &dyn PostStorage,
    post_id: PostId,
    desired: &[TagLabel],
) -> InternalResult<()> {
    let existing = posts.get_tags_for_post(post_id).await?;
    let diff = post_tag_diff(&existing, desired);

    for label in diff.to_add {
        posts.tag_post(post_id, label).await?;
    }
    for slug in diff.to_remove {
        posts.untag_post(post_id, slug).await?;
    }
    Ok(())
}

/// The shared public-permalink lookup used by both the `get_post` server fn and
/// the non-reactive public projector.
///
/// Validates the date, then does the visibility-filtered store lookup for
/// `viewer`. The caller maps the record to a `PostResponse` with its own
/// `is_author` (the projector always anonymous → `false`; the server fn derives
/// it from the session), so there is one query and no drift between the two
/// public surfaces.
///
/// # Errors
///
/// Returns a validation error for an impossible calendar date, or a storage
/// error if the permalink lookup fails.
pub async fn fetch_post_record(
    posts: &dyn PostStorage,
    viewer: &ViewerIdentity,
    username: &Username,
    year: i32,
    month: u32,
    day: u32,
    slug: &Slug,
) -> InternalResult<Option<PostRecord>> {
    NaiveDate::from_ymd_opt(year, month, day)
        .ok_or_else(|| InternalError::validation("Invalid permalink"))?;
    posts
        .get_post_by_permalink(
            username,
            PermalinkDate { year, month, day },
            slug,
            viewer,
            Utc::now(),
        )
        .await
        .map_err(InternalError::storage)
}

/// Finds an authenticated author's own draft at a given permalink by paging
/// their draft list.
///
/// # Errors
///
/// Returns a storage error if a draft-listing page fails to load.
pub async fn find_draft_by_permalink_for_user(
    posts: &dyn PostStorage,
    user_id: UserId,
    year: i32,
    month: u32,
    day: u32,
    slug: &Slug,
) -> InternalResult<Option<PostRecord>> {
    let mut cursor = None;

    // Search through up to 10,000 drafts (200 pages of 50). This 200-iteration
    // limit is a safety bound to prevent infinite loops or excessive DB load
    // while still being large enough for almost any user's draft list.
    for _ in 0..200 {
        let drafts = posts
            .list_drafts_by_user(user_id, cursor.as_ref(), 50, chrono::Utc::now())
            .await?;
        if drafts.is_empty() {
            return Ok(None);
        }

        let next_cursor = drafts.last().map(to_post_cursor);

        if let Some(found) = drafts.into_iter().find(|post| {
            post.slug == *slug
                && post.created_at.year() == year
                && post.created_at.month() == month
                && post.created_at.day() == day
        }) {
            return Ok(Some(found));
        }

        let Some(next_cursor) = next_cursor else {
            unreachable!("drafts is non-empty after the is_empty guard, so last() is Some")
        };
        cursor = Some(next_cursor);
    }

    Ok(None)
}

/// Applies the `TagNotFound → empty` business rule to a by-tag listing result:
/// a missing tag yields an empty page (not an error), while a real storage
/// failure propagates.
///
/// # Errors
///
/// Returns a storage error if the underlying listing failed for any reason
/// other than a missing tag.
pub fn list_by_tag_rows(
    result: Result<Vec<PostRecord>, ListByTagError>,
) -> InternalResult<Vec<PostRecord>> {
    match result {
        Ok(rows) => Ok(rows),
        Err(ListByTagError::TagNotFound) => Ok(Vec::new()),
        Err(ListByTagError::Internal(e)) => Err(InternalError::storage(e)),
    }
}

/// Async operations on the `posts` and `post_revisions` tables.
///
/// This trait manages the lifecycle of posts, including versioned edits,
/// draft/publish status, soft-deletion, and tagging.
#[cfg_attr(feature = "test-utils", mockall::automock)]
#[async_trait]
pub trait PostStorage: Send + Sync {
    /// Creates a new post.
    async fn create_post(&self, input: &CreatePostInput) -> Result<PostId, CreatePostError>;

    /// Creates `inputs.len()` posts in a single transaction, returning their new
    /// ids in input order. All-or-nothing: any failure (e.g. a slug conflict on
    /// one row) rolls the whole batch back and nothing persists. An empty slice
    /// is a no-op returning an empty vec without opening a transaction.
    async fn create_posts(
        &self,
        inputs: &[CreatePostInput],
    ) -> Result<Vec<PostId>, CreatePostError>;

    /// Returns the `post_id` a `(user_id, key)` idempotency pair maps to, or
    /// `None` if the key was never used by that user. Used to look up the
    /// original post on an [`CreatePostError::IdempotencyConflict`] retry.
    async fn post_id_for_idempotency_key(
        &self,
        user_id: UserId,
        key: &str,
    ) -> Result<Option<PostId>, sqlx::Error>;

    /// Fetches a post by its ID, applying the viewer-resolution filter: the post
    /// is returned only if `viewer` is the author or a targeted audience admits
    /// them. See ADR-0020.
    async fn get_post_by_id(
        &self,
        post_id: PostId,
        viewer: &ViewerIdentity,
    ) -> sqlx::Result<Option<PostRecord>>;

    /// Fetches a post by its public permalink components, applying the
    /// viewer-resolution filter. See ADR-0020.
    ///
    /// `now` gates scheduled posts: a post with `published_at > now` is
    /// future-dated and stays invisible on this public surface until its time.
    async fn get_post_by_permalink(
        &self,
        username: &Username,
        date: PermalinkDate,
        slug: &Slug,
        viewer: &ViewerIdentity,
        now: DateTime<Utc>,
    ) -> sqlx::Result<Option<PostRecord>>;

    /// Updates a post and creates a new revision.
    ///
    /// # Errors
    ///
    /// Returns [`UpdatePostError::NotFound`] if the post doesn't exist, or
    /// [`UpdatePostError::Unauthorized`] if the editor isn't the owner.
    async fn update_post(
        &self,
        post_id: PostId,
        editor_user_id: UserId,
        input: &UpdatePostInput,
    ) -> Result<PostRecord, UpdatePostError>;

    /// Marks a post as deleted without removing it from the database.
    async fn soft_delete_post(&self, post_id: PostId) -> sqlx::Result<()>;

    /// Reverts a published post to draft status.
    async fn unpublish_post(&self, post_id: PostId) -> sqlx::Result<()>;

    /// Lists published posts for a specific user, ordered by creation date,
    /// applying the viewer-resolution filter. See ADR-0020.
    ///
    /// `now` gates scheduled posts (`published_at > now`) off this public
    /// surface until their time.
    ///
    /// The explicit `'a` on the `cursor` reference exists so
    /// `mockall::automock` can mock this trait: automock cannot synthesize a
    /// lifetime for a reference nested inside a generic (here
    /// `Option<&PostCursor>`), so we name it. Behaviour is identical to
    /// lifetime elision — the annotation is purely to satisfy the macro
    /// (ref #245).
    async fn list_published_by_user<'a>(
        &self,
        username: &Username,
        cursor: Option<&'a PostCursor>,
        limit: u32,
        viewer: &ViewerIdentity,
        now: DateTime<Utc>,
    ) -> sqlx::Result<Vec<PostRecord>>;

    /// Lists all published posts across the entire site, applying the
    /// viewer-resolution filter. See ADR-0020.
    ///
    /// `now` gates scheduled posts (`published_at > now`) off this public
    /// surface until their time.
    // Explicit `'a` for `mockall::automock` — see `list_published_by_user`.
    async fn list_published<'a>(
        &self,
        cursor: Option<&'a PostCursor>,
        limit: u32,
        viewer: &ViewerIdentity,
        now: DateTime<Utc>,
    ) -> sqlx::Result<Vec<PostRecord>>;

    /// Lists draft posts for a specific user.
    ///
    /// This is the author's "not-yet-live" surface: it returns true drafts
    /// (`published_at IS NULL`) **and** scheduled posts (`published_at > now`),
    /// so a future-dated post — invisible on every public surface until its
    /// time — stays visible to its own author. `now` gates which posts count
    /// as not-yet-live.
    // Explicit `'a` for `mockall::automock` — see `list_published_by_user`.
    async fn list_drafts_by_user<'a>(
        &self,
        user_id: UserId,
        cursor: Option<&'a PostCursor>,
        limit: u32,
        now: DateTime<Utc>,
    ) -> sqlx::Result<Vec<PostRecord>>;

    /// Lists all of a user's non-soft-deleted posts (drafts + published)
    /// ordered by `updated_at DESC, post_id DESC` for the `AtomPub` Collection
    /// surface. Tags are hydrated.
    // Explicit `'a` for `mockall::automock` — see `list_published_by_user`.
    async fn list_collection_by_user<'a>(
        &self,
        user_id: UserId,
        cursor: Option<&'a CollectionCursor>,
        limit: u32,
    ) -> sqlx::Result<Vec<PostRecord>>;

    /// Associates a post with a tag. If the tag doesn't exist, it is created.
    async fn tag_post(&self, post_id: PostId, tag: &TagLabel) -> Result<(), TaggingError>;

    /// Removes a tag association from a post.
    async fn untag_post(&self, post_id: PostId, tag_slug: &Tag) -> Result<(), TaggingError>;

    /// Returns all tags associated with a specific post.
    async fn get_tags_for_post(&self, post_id: PostId) -> sqlx::Result<Vec<PostTag>>;

    /// Lists published posts that carry a specific tag, applying the
    /// viewer-resolution filter. See ADR-0020.
    ///
    /// `now` gates scheduled posts (`published_at > now`) off this public
    /// surface until their time.
    // Explicit `'a` for `mockall::automock` — see `list_published_by_user`.
    async fn list_posts_by_tag<'a>(
        &self,
        tag_slug: &Tag,
        cursor: Option<&'a PostCursor>,
        limit: u32,
        viewer: &ViewerIdentity,
        now: DateTime<Utc>,
    ) -> Result<Vec<PostRecord>, ListByTagError>;

    /// Lists published posts for a specific user that carry a specific tag,
    /// applying the viewer-resolution filter. See ADR-0020.
    ///
    /// `now` gates scheduled posts (`published_at > now`) off this public
    /// surface until their time.
    // Explicit `'a` for `mockall::automock` — see `list_published_by_user`.
    async fn list_user_posts_by_tag<'a>(
        &self,
        user_id: UserId,
        tag_slug: &Tag,
        cursor: Option<&'a PostCursor>,
        limit: u32,
        viewer: &ViewerIdentity,
        now: DateTime<Utc>,
    ) -> Result<Vec<PostRecord>, ListByTagError>;

    /// Returns tag records whose slug begins with `prefix` (case-insensitive
    /// on the slug). An empty / `None` prefix returns all tags, alphabetically,
    /// up to `limit`.
    // Explicit `'a` for `mockall::automock` — see `list_published_by_user`.
    async fn list_tags<'a>(
        &self,
        prefix: Option<&'a str>,
        limit: u32,
    ) -> sqlx::Result<Vec<TagRecord>>;

    /// Lists published posts matching `surface`, applying the
    /// [`HybridWindow`](common::feed::HybridWindow) selection rule (union of
    /// "the most recent `min_items` items" and "all items published within the
    /// last `min_days`"). Results are ordered by `published_at DESC`.
    ///
    /// `now` is passed in so callers can supply a deterministic clock in
    /// tests. Posts with `published_at > now` (future-dated) are excluded.
    async fn list_published_in_window(
        &self,
        surface: &common::feed::FeedSurface,
        window: &common::feed::HybridWindow,
        now: DateTime<Utc>,
        viewer: &ViewerIdentity,
    ) -> sqlx::Result<Vec<PostRecord>>;

    /// Lists posts that crossed into "live" within the window `(after, upto]`
    /// (exclusive lower, inclusive upper): `published_at > after AND
    /// published_at <= upto AND deleted_at IS NULL`. Each [`GoLivePost`] carries
    /// its author username and tag slugs so the feed worker can fan out to the
    /// affected feed surfaces. Drives the steady-state go-live pass.
    async fn list_posts_gone_live_between(
        &self,
        after: DateTime<Utc>,
        upto: DateTime<Utc>,
    ) -> sqlx::Result<Vec<GoLivePost>>;

    /// Returns the URLs of cached feeds whose surface has a live post
    /// (`published_at <= now`, not deleted) strictly newer than the feed's own
    /// `generated_at` — i.e. cached feeds that missed a go-live while the worker
    /// was down. Drives the feed-relative startup catch-up.
    async fn feed_urls_needing_catchup(&self, now: DateTime<Utc>) -> sqlx::Result<Vec<FeedPath>>;

    /// Reads a post's audience targeting as a [`Vec<AudienceTarget>`], for
    /// pre-selecting the editor's audience picker.
    ///
    /// Owner-only: this performs no viewer resolution and is intended to be
    /// called for a post the caller already owns. Maps each `post_audiences`
    /// row back to its [`AudienceTarget`] (`public` → [`AudienceTarget::Public`],
    /// `subscribers` → [`AudienceTarget::Subscribers`], `named` →
    /// [`AudienceTarget::Named`]); a post with no rows yields an empty vec
    /// (equivalent to [`AudienceTarget::Private`]). See ADR-0020.
    async fn get_post_audiences(&self, post_id: PostId) -> sqlx::Result<Vec<AudienceTarget>>;
}

/// Backend-specific divergence for [`PostStore`].
///
/// Two consts capture SQL-fragment divergence shared by many methods:
/// [`TAGS_SUBQUERY`][PostDialect::TAGS_SUBQUERY] (SQLite `json_group_array`
/// vs Postgres `json_agg`/`::text`) and
/// [`PERMALINK_DATE_CLAUSE`][PostDialect::PERMALINK_DATE_CLAUSE] (SQLite
/// `date(...)` vs Postgres `date(... AT TIME ZONE 'UTC') = $3::date`).
///
/// The two transaction-bearing mutations [`update_post`][PostDialect::update_post]
/// (Postgres locks the row with `FOR UPDATE`) and
/// [`tag_post`][PostDialect::tag_post] (SQLite `INSERT OR IGNORE` vs Postgres
/// `INSERT … ON CONFLICT DO NOTHING`) are monomorphised per backend, as is
/// [`untag_post`][PostDialect::untag_post], whose `.rows_affected()` call has no
/// generic form in sqlx 0.8. Everything else is shared on [`PostStore`].
/// See ADR-0019.
#[async_trait]
pub trait PostDialect: Backend {
    /// Correlated JSON tag-aggregation subquery (on `p.post_id`) spelled in
    /// this backend's JSON dialect, yielding a `text` column.
    const TAGS_SUBQUERY: &'static str;

    /// Predicate matching a post's `published_at` date against the bound
    /// `YYYY-MM-DD` string (`$3`), in this backend's date dialect.
    const PERMALINK_DATE_CLAUSE: &'static str;

    /// Deletes every `post_audiences` row for a post. Bind order: `post_id`.
    const DELETE_POST_AUDIENCES: &'static str;
    /// Inserts one `post_audiences` row, resolving the target-kind name to its
    /// `kind_id` via a subquery. Bind order: `post_id, audience_id, kind_name`.
    const INSERT_POST_AUDIENCE: &'static str;

    /// Update a post and record a revision, returning the updated record.
    async fn update_post(
        pool: &Pool<Self>,
        post_id: PostId,
        editor_user_id: UserId,
        input: &UpdatePostInput,
    ) -> Result<PostRecord, UpdatePostError>;

    /// Associate `post_id` with `tag` (its slug is the canonical key, its label
    /// the stored casing), creating the tag if it does not yet exist.
    async fn tag_post(
        pool: &Pool<Self>,
        post_id: PostId,
        tag: &TagLabel,
    ) -> Result<(), TaggingError>;

    /// Remove a tag association; returns [`TaggingError::TagNotFound`] when no
    /// row was deleted.
    async fn untag_post(
        pool: &Pool<Self>,
        post_id: PostId,
        tag_slug: &Tag,
    ) -> Result<(), TaggingError>;
}

/// Generic [`PostStorage`] backed by any [`PostDialect`] database.
///
/// Every read and the non-transactional shared mutations live here, splicing
/// [`PostDialect::TAGS_SUBQUERY`] / [`PostDialect::PERMALINK_DATE_CLAUSE`] into
/// otherwise-identical SQL; the transaction-bearing and `rows_affected`
/// mutations delegate to [`PostDialect`]. See ADR-0019.
pub struct PostStore<DB: Database> {
    pool: Pool<DB>,
}

impl<DB: Database> PostStore<DB> {
    #[must_use]
    pub fn new(pool: Pool<DB>) -> Self {
        Self { pool }
    }
}

#[async_trait]
impl<DB> PostStorage for PostStore<DB>
where
    DB: PostDialect,
    PostRow: for<'r> sqlx::FromRow<'r, DB::Row>,
    (i64,): for<'r> sqlx::FromRow<'r, DB::Row>,
    (bool,): for<'r> sqlx::FromRow<'r, DB::Row>,
    (i64, i64, Tag, TagLabel): for<'r> sqlx::FromRow<'r, DB::Row>,
    (i64, Tag): for<'r> sqlx::FromRow<'r, DB::Row>,
    (String, Option<i64>): for<'r> sqlx::FromRow<'r, DB::Row>,
    (DateTime<Utc>,): for<'r> sqlx::FromRow<'r, DB::Row>,
    (String, DateTime<Utc>): for<'r> sqlx::FromRow<'r, DB::Row>,
    for<'q> i64: sqlx::Encode<'q, DB> + sqlx::Type<DB>,
    for<'q> &'q str: sqlx::Encode<'q, DB> + sqlx::Type<DB>,
    for<'q> Option<&'q str>: sqlx::Encode<'q, DB> + sqlx::Type<DB>,
    for<'q> Option<String>: sqlx::Encode<'q, DB> + sqlx::Type<DB>,
    // `Slug`/`Tag`/`Username` bind and decode as themselves via the sqlx bridge
    // (#438), which delegates to `String`; this pair makes that bridge available
    // on the generic backend (the reads decode the `slug`/`tag_slug`/`username`
    // columns straight into their newtypes). The `Option<&PostTitle>` bound is the
    // nullable `title` bind, forwarded from `write_post_in_tx` (create paths).
    String: sqlx::Type<DB>,
    for<'q> String: sqlx::Encode<'q, DB>,
    for<'q> Option<&'q PostTitle>: sqlx::Encode<'q, DB> + sqlx::Type<DB>,
    // `summary` binds as `Option<&PostSummary>` via the ADR-0071 sqlx bridge
    // (delegates to `String`) on the create paths, mirroring the
    // `Option<&PostTitle>` bound above.
    for<'q> Option<&'q PostSummary>: sqlx::Encode<'q, DB> + sqlx::Type<DB>,
    for<'q> Option<i64>: sqlx::Encode<'q, DB> + sqlx::Type<DB>,
    for<'q> DateTime<Utc>: sqlx::Encode<'q, DB> + sqlx::Type<DB>,
    for<'q> Option<DateTime<Utc>>: sqlx::Encode<'q, DB> + sqlx::Type<DB>,
    for<'c> &'c Pool<DB>: sqlx::Executor<'c, Database = DB>,
    for<'c> &'c mut DB::Connection: sqlx::Executor<'c, Database = DB>,
    for<'q> DB::Arguments<'q>: sqlx::IntoArguments<'q, DB>,
{
    #[tracing::instrument(
        name = "storage.posts.create",
        skip(self, input),
        fields(db.system = DB::DB_SYSTEM)
    )]
    async fn create_post(&self, input: &CreatePostInput) -> Result<PostId, CreatePostError> {
        let mut tx = self.pool.begin().await?;
        // On any error the `?` drops `tx`, which sqlx rolls back — equivalent to
        // the previous explicit `tx.rollback()` before returning. (`&mut tx`
        // coerces to `&mut DB::Connection` for the helper.)
        let post_id = write_post_in_tx::<DB>(&mut tx, input).await?;
        tx.commit().await?;
        Ok(post_id)
    }

    #[tracing::instrument(
        name = "storage.posts.create_batch",
        skip(self, inputs),
        fields(db.system = DB::DB_SYSTEM, count = inputs.len())
    )]
    async fn create_posts(
        &self,
        inputs: &[CreatePostInput],
    ) -> Result<Vec<PostId>, CreatePostError> {
        if inputs.is_empty() {
            return Ok(Vec::new());
        }
        let mut tx = self.pool.begin().await?;
        let mut ids = Vec::with_capacity(inputs.len());
        for input in inputs {
            // `?` drops `tx` on error → whole-batch rollback (atomic seed).
            ids.push(write_post_in_tx::<DB>(&mut tx, input).await?);
        }
        tx.commit().await?;
        Ok(ids)
    }

    #[tracing::instrument(
        name = "storage.posts.post_id_for_idempotency_key",
        skip(self),
        fields(db.system = DB::DB_SYSTEM)
    )]
    async fn post_id_for_idempotency_key(
        &self,
        user_id: UserId,
        key: &str,
    ) -> Result<Option<PostId>, sqlx::Error> {
        let post_id = sqlx::query_scalar::<_, i64>(
            "SELECT post_id FROM idempotency_keys WHERE user_id = $1 AND key = $2",
        )
        .bind(i64::from(user_id))
        .bind(key)
        .fetch_optional(&self.pool)
        .await?;
        Ok(post_id.map(PostId::from))
    }

    #[tracing::instrument(
        name = "storage.posts.get_by_id",
        skip(self),
        fields(db.system = DB::DB_SYSTEM)
    )]
    async fn get_post_by_id(
        &self,
        post_id: PostId,
        viewer: &ViewerIdentity,
    ) -> sqlx::Result<Option<PostRecord>> {
        let (resolution, binds, _) = resolution_where(viewer, 2);
        let sql = format!(
            "SELECT p.post_id, p.user_id, u.username, p.title, p.slug, p.body, p.format, p.rendered_html,
                    p.created_at, p.updated_at, p.published_at, p.deleted_at, p.summary,
                    {tags} AS tags
             FROM posts p
             JOIN users u ON p.user_id = u.user_id
             WHERE p.post_id = $1
               AND {resolution}",
            tags = DB::TAGS_SUBQUERY,
        );
        let query = sqlx::query_as::<_, PostRow>(&sql).bind(i64::from(post_id));
        let row = binds.bind_onto(query).fetch_optional(&self.pool).await?;
        Ok(row.map(post_record_from_row).transpose()?)
    }

    #[tracing::instrument(
        name = "storage.posts.get_audiences",
        skip(self),
        fields(db.system = DB::DB_SYSTEM)
    )]
    async fn get_post_audiences(&self, post_id: PostId) -> sqlx::Result<Vec<AudienceTarget>> {
        // Owner-only: no viewer resolution. `ORDER BY` makes the result
        // deterministic so callers can compare vecs directly.
        let rows: Vec<(String, Option<i64>)> = sqlx::query_as(
            "SELECT tk.name, pa.audience_id \
             FROM post_audiences pa \
             JOIN target_kinds tk ON tk.kind_id = pa.target_kind_id \
             WHERE pa.post_id = $1 \
             ORDER BY tk.name, pa.audience_id",
        )
        .bind(i64::from(post_id))
        .fetch_all(&self.pool)
        .await?;
        Ok(rows
            .into_iter()
            .filter_map(|(kind, audience_id)| audience_target_from_row(&kind, audience_id))
            .collect())
    }

    #[tracing::instrument(
        name = "storage.posts.get_by_permalink",
        skip(self),
        fields(db.system = DB::DB_SYSTEM)
    )]
    async fn get_post_by_permalink(
        &self,
        username: &Username,
        date: PermalinkDate,
        slug: &Slug,
        viewer: &ViewerIdentity,
        now: DateTime<Utc>,
    ) -> sqlx::Result<Option<PostRecord>> {
        let PermalinkDate { year, month, day } = date;
        let date_str = format!("{year:04}-{month:02}-{day:02}");
        let (resolution, binds, _) = resolution_where(viewer, 5);
        // `published_at <= $4` hides scheduled (future-dated) posts until due.
        let sql = format!(
            "SELECT p.post_id, p.user_id, u.username, p.title, p.slug, p.body, p.format, p.rendered_html,
                    p.created_at, p.updated_at, p.published_at, p.deleted_at, p.summary,
                    {tags} AS tags
             FROM posts p
             JOIN users u ON p.user_id = u.user_id
             WHERE u.username = $1
               AND p.slug = $2
               AND p.published_at IS NOT NULL
               AND p.published_at <= $4
               AND p.deleted_at IS NULL
               AND {date_clause}
               AND {resolution}",
            tags = DB::TAGS_SUBQUERY,
            date_clause = DB::PERMALINK_DATE_CLAUSE,
        );
        let query = sqlx::query_as::<_, PostRow>(&sql)
            .bind(username)
            .bind(slug)
            .bind(date_str.as_str())
            .bind(now);
        let row = binds.bind_onto(query).fetch_optional(&self.pool).await?;
        Ok(row.map(post_record_from_row).transpose()?)
    }

    #[tracing::instrument(
        name = "storage.posts.update",
        skip(self, input),
        fields(db.system = DB::DB_SYSTEM)
    )]
    async fn update_post(
        &self,
        post_id: PostId,
        editor_user_id: UserId,
        input: &UpdatePostInput,
    ) -> Result<PostRecord, UpdatePostError> {
        DB::update_post(&self.pool, post_id, editor_user_id, input).await
    }

    #[tracing::instrument(
        name = "storage.posts.soft_delete",
        skip(self),
        fields(db.system = DB::DB_SYSTEM)
    )]
    async fn soft_delete_post(&self, post_id: PostId) -> sqlx::Result<()> {
        let now = Utc::now();
        sqlx::query("UPDATE posts SET deleted_at = $1 WHERE post_id = $2")
            .bind(now)
            .bind(i64::from(post_id))
            .execute(&self.pool)
            .await?;
        Ok(())
    }

    #[tracing::instrument(
        name = "storage.posts.unpublish",
        skip(self),
        fields(db.system = DB::DB_SYSTEM)
    )]
    async fn unpublish_post(&self, post_id: PostId) -> sqlx::Result<()> {
        sqlx::query("UPDATE posts SET published_at = NULL WHERE post_id = $1")
            .bind(i64::from(post_id))
            .execute(&self.pool)
            .await?;
        Ok(())
    }

    #[tracing::instrument(
        name = "storage.posts.list_published_by_user",
        skip(self),
        fields(db.system = DB::DB_SYSTEM)
    )]
    async fn list_published_by_user<'a>(
        &self,
        username: &Username,
        cursor: Option<&'a PostCursor>,
        limit: u32,
        viewer: &ViewerIdentity,
        now: DateTime<Utc>,
    ) -> sqlx::Result<Vec<PostRecord>> {
        let tags = DB::TAGS_SUBQUERY;
        let rows = if let Some(cursor) = cursor {
            // Binds: $1 username, $2/$3 cursor, $4 post_id, $5 now,
            // $6..$10 resolution, $11 limit.
            let (resolution, binds, limit_idx) = resolution_where(viewer, 6);
            // `published_at <= $5` hides scheduled (future-dated) posts.
            let sql = format!(
                "SELECT p.post_id, p.user_id, u.username, p.title, p.slug, p.body, p.format, p.rendered_html,
                        p.created_at, p.updated_at, p.published_at, p.deleted_at, p.summary,
                        {tags} AS tags
                 FROM posts p
                 JOIN users u ON p.user_id = u.user_id
                 WHERE u.username = $1
                   AND p.published_at IS NOT NULL
                   AND p.published_at <= $5
                   AND p.deleted_at IS NULL
                   AND (p.created_at < $2 OR (p.created_at = $3 AND p.post_id < $4))
                   AND {resolution}
                 ORDER BY p.created_at DESC, p.post_id DESC
                 LIMIT ${limit_idx}"
            );
            let query = sqlx::query_as::<_, PostRow>(&sql)
                .bind(username)
                .bind(cursor.created_at)
                .bind(cursor.created_at)
                .bind(i64::from(cursor.post_id))
                .bind(now);
            binds
                .bind_onto(query)
                .bind(i64::from(limit))
                .fetch_all(&self.pool)
                .await?
        } else {
            // Binds: $1 username, $2 now, $3..$7 resolution, $8 limit.
            let (resolution, binds, limit_idx) = resolution_where(viewer, 3);
            // `published_at <= $2` hides scheduled (future-dated) posts.
            let sql = format!(
                "SELECT p.post_id, p.user_id, u.username, p.title, p.slug, p.body, p.format, p.rendered_html,
                        p.created_at, p.updated_at, p.published_at, p.deleted_at, p.summary,
                        {tags} AS tags
                 FROM posts p
                 JOIN users u ON p.user_id = u.user_id
                 WHERE u.username = $1
                   AND p.published_at IS NOT NULL
                   AND p.published_at <= $2
                   AND p.deleted_at IS NULL
                   AND {resolution}
                 ORDER BY p.created_at DESC, p.post_id DESC
                 LIMIT ${limit_idx}"
            );
            let query = sqlx::query_as::<_, PostRow>(&sql).bind(username).bind(now);
            binds
                .bind_onto(query)
                .bind(i64::from(limit))
                .fetch_all(&self.pool)
                .await?
        };
        rows.into_iter().map(post_record_from_row).collect()
    }

    #[tracing::instrument(
        name = "storage.posts.list_published",
        skip(self),
        fields(db.system = DB::DB_SYSTEM)
    )]
    async fn list_published<'a>(
        &self,
        cursor: Option<&'a PostCursor>,
        limit: u32,
        viewer: &ViewerIdentity,
        now: DateTime<Utc>,
    ) -> sqlx::Result<Vec<PostRecord>> {
        let tags = DB::TAGS_SUBQUERY;
        let rows = if let Some(cursor) = cursor {
            // Binds: $1/$2 cursor, $3 post_id, $4 now, $5..$9 resolution,
            // $10 limit.
            let (resolution, binds, limit_idx) = resolution_where(viewer, 5);
            // `published_at <= $4` hides scheduled (future-dated) posts.
            let sql = format!(
                "SELECT p.post_id, p.user_id, u.username, p.title, p.slug, p.body, p.format, p.rendered_html,
                        p.created_at, p.updated_at, p.published_at, p.deleted_at, p.summary,
                        {tags} AS tags
                 FROM posts p
                 JOIN users u ON p.user_id = u.user_id
                 WHERE p.published_at IS NOT NULL
                   AND p.published_at <= $4
                   AND p.deleted_at IS NULL
                   AND (p.created_at < $1 OR (p.created_at = $2 AND p.post_id < $3))
                   AND {resolution}
                 ORDER BY p.created_at DESC, p.post_id DESC
                 LIMIT ${limit_idx}"
            );
            let query = sqlx::query_as::<_, PostRow>(&sql)
                .bind(cursor.created_at)
                .bind(cursor.created_at)
                .bind(i64::from(cursor.post_id))
                .bind(now);
            binds
                .bind_onto(query)
                .bind(i64::from(limit))
                .fetch_all(&self.pool)
                .await?
        } else {
            // Binds: $1 now, $2..$6 resolution, $7 limit.
            let (resolution, binds, limit_idx) = resolution_where(viewer, 2);
            // `published_at <= $1` hides scheduled (future-dated) posts.
            let sql = format!(
                "SELECT p.post_id, p.user_id, u.username, p.title, p.slug, p.body, p.format, p.rendered_html,
                        p.created_at, p.updated_at, p.published_at, p.deleted_at, p.summary,
                        {tags} AS tags
                 FROM posts p
                 JOIN users u ON p.user_id = u.user_id
                 WHERE p.published_at IS NOT NULL
                   AND p.published_at <= $1
                   AND p.deleted_at IS NULL
                   AND {resolution}
                 ORDER BY p.created_at DESC, p.post_id DESC
                 LIMIT ${limit_idx}"
            );
            let query = sqlx::query_as::<_, PostRow>(&sql).bind(now);
            binds
                .bind_onto(query)
                .bind(i64::from(limit))
                .fetch_all(&self.pool)
                .await?
        };
        rows.into_iter().map(post_record_from_row).collect()
    }

    #[tracing::instrument(
        name = "storage.posts.list_drafts_by_user",
        skip(self),
        fields(db.system = DB::DB_SYSTEM)
    )]
    async fn list_drafts_by_user<'a>(
        &self,
        user_id: UserId,
        cursor: Option<&'a PostCursor>,
        limit: u32,
        now: DateTime<Utc>,
    ) -> sqlx::Result<Vec<PostRecord>> {
        let tags = DB::TAGS_SUBQUERY;
        let rows = if let Some(cursor) = cursor {
            // `published_at IS NULL OR published_at > $5` surfaces both true
            // drafts and scheduled (future-dated) posts to the author.
            let sql = format!(
                "SELECT p.post_id, p.user_id, u.username, p.title, p.slug, p.body, p.format, p.rendered_html,
                        p.created_at, p.updated_at, p.published_at, p.deleted_at, p.summary,
                        {tags} AS tags
                 FROM posts p
                 JOIN users u ON p.user_id = u.user_id
                 WHERE p.user_id = $1
                   AND (p.published_at IS NULL OR p.published_at > $5)
                   AND p.deleted_at IS NULL
                   AND (p.created_at < $2 OR (p.created_at = $3 AND p.post_id < $4))
                 ORDER BY p.created_at DESC, p.post_id DESC
                 LIMIT $6"
            );
            sqlx::query_as::<_, PostRow>(&sql)
                .bind(i64::from(user_id))
                .bind(cursor.created_at)
                .bind(cursor.created_at)
                .bind(i64::from(cursor.post_id))
                .bind(now)
                .bind(i64::from(limit))
                .fetch_all(&self.pool)
                .await?
        } else {
            // `published_at IS NULL OR published_at > $2` surfaces both true
            // drafts and scheduled (future-dated) posts to the author.
            let sql = format!(
                "SELECT p.post_id, p.user_id, u.username, p.title, p.slug, p.body, p.format, p.rendered_html,
                        p.created_at, p.updated_at, p.published_at, p.deleted_at, p.summary,
                        {tags} AS tags
                 FROM posts p
                 JOIN users u ON p.user_id = u.user_id
                 WHERE p.user_id = $1
                   AND (p.published_at IS NULL OR p.published_at > $2)
                   AND p.deleted_at IS NULL
                 ORDER BY p.created_at DESC, p.post_id DESC
                 LIMIT $3"
            );
            sqlx::query_as::<_, PostRow>(&sql)
                .bind(i64::from(user_id))
                .bind(now)
                .bind(i64::from(limit))
                .fetch_all(&self.pool)
                .await?
        };
        rows.into_iter().map(post_record_from_row).collect()
    }

    #[tracing::instrument(
        name = "storage.posts.list_collection_by_user",
        skip(self),
        fields(db.system = DB::DB_SYSTEM)
    )]
    async fn list_collection_by_user<'a>(
        &self,
        user_id: UserId,
        cursor: Option<&'a CollectionCursor>,
        limit: u32,
    ) -> sqlx::Result<Vec<PostRecord>> {
        let tags = DB::TAGS_SUBQUERY;
        let rows = if let Some(cursor) = cursor {
            let sql = format!(
                "SELECT p.post_id, p.user_id, u.username, p.title, p.slug, p.body, p.format, p.rendered_html,
                        p.created_at, p.updated_at, p.published_at, p.deleted_at, p.summary,
                        {tags} AS tags
                 FROM posts p
                 JOIN users u ON p.user_id = u.user_id
                 WHERE p.user_id = $1
                   AND p.deleted_at IS NULL
                   AND (p.updated_at, p.post_id) < ($2, $3)
                 ORDER BY p.updated_at DESC, p.post_id DESC
                 LIMIT $4"
            );
            sqlx::query_as::<_, PostRow>(&sql)
                .bind(i64::from(user_id))
                .bind(cursor.updated_at)
                .bind(i64::from(cursor.post_id))
                .bind(i64::from(limit))
                .fetch_all(&self.pool)
                .await?
        } else {
            let sql = format!(
                "SELECT p.post_id, p.user_id, u.username, p.title, p.slug, p.body, p.format, p.rendered_html,
                        p.created_at, p.updated_at, p.published_at, p.deleted_at, p.summary,
                        {tags} AS tags
                 FROM posts p
                 JOIN users u ON p.user_id = u.user_id
                 WHERE p.user_id = $1
                   AND p.deleted_at IS NULL
                 ORDER BY p.updated_at DESC, p.post_id DESC
                 LIMIT $2"
            );
            sqlx::query_as::<_, PostRow>(&sql)
                .bind(i64::from(user_id))
                .bind(i64::from(limit))
                .fetch_all(&self.pool)
                .await?
        };
        rows.into_iter().map(post_record_from_row).collect()
    }

    #[tracing::instrument(
        name = "storage.posts.tag",
        skip(self),
        fields(db.system = DB::DB_SYSTEM)
    )]
    async fn tag_post(&self, post_id: PostId, tag: &TagLabel) -> Result<(), TaggingError> {
        DB::tag_post(&self.pool, post_id, tag).await
    }

    #[tracing::instrument(
        name = "storage.posts.untag",
        skip(self),
        fields(db.system = DB::DB_SYSTEM)
    )]
    async fn untag_post(&self, post_id: PostId, tag_slug: &Tag) -> Result<(), TaggingError> {
        DB::untag_post(&self.pool, post_id, tag_slug).await
    }

    #[tracing::instrument(
        name = "storage.posts.get_tags_for_post",
        skip(self),
        fields(db.system = DB::DB_SYSTEM)
    )]
    async fn get_tags_for_post(&self, post_id: PostId) -> sqlx::Result<Vec<PostTag>> {
        let rows = sqlx::query_as::<_, (i64, i64, Tag, TagLabel)>(
            "SELECT pt.post_id, pt.tag_id, t.tag_slug, pt.tag_display
             FROM post_tags pt
             JOIN tags t ON pt.tag_id = t.tag_id
             WHERE pt.post_id = $1
             ORDER BY t.tag_slug",
        )
        .bind(i64::from(post_id))
        .fetch_all(&self.pool)
        .await?;

        // `tag_slug`/`tag_display` decode straight into `Tag`/`TagLabel` via the
        // sqlx bridge (#438), so a malformed stored value is rejected as a
        // column-decode error above; this is a straight field-move.
        Ok(rows
            .into_iter()
            .map(|(post_id, tag_id, tag_slug, tag_display)| PostTag {
                post_id: PostId::from(post_id),
                tag_id: TagId::from(tag_id),
                tag_slug,
                tag_display,
            })
            .collect())
    }

    #[tracing::instrument(
        name = "storage.posts.list_posts_by_tag",
        skip(self),
        fields(db.system = DB::DB_SYSTEM)
    )]
    async fn list_posts_by_tag<'a>(
        &self,
        tag_slug: &Tag,
        cursor: Option<&'a PostCursor>,
        limit: u32,
        viewer: &ViewerIdentity,
        now: DateTime<Utc>,
    ) -> Result<Vec<PostRecord>, ListByTagError> {
        let tag_exists: bool =
            sqlx::query_scalar("SELECT COUNT(*) > 0 FROM tags WHERE tag_slug = $1")
                .bind(tag_slug)
                .fetch_one(&self.pool)
                .await?;

        if !tag_exists {
            return Err(ListByTagError::TagNotFound);
        }

        let tags = DB::TAGS_SUBQUERY;
        let rows = if let Some(cursor) = cursor {
            // Binds: $1 tag, $2/$3 cursor, $4 post_id, $5 now,
            // $6..$10 resolution, $11 limit.
            let (resolution, binds, limit_idx) = resolution_where(viewer, 6);
            // `published_at <= $5` hides scheduled (future-dated) posts.
            let sql = format!(
                "SELECT p.post_id, p.user_id, u.username, p.title, p.slug, p.body, p.format, p.rendered_html,
                        p.created_at, p.updated_at, p.published_at, p.deleted_at, p.summary,
                        {tags} AS tags
                 FROM posts p
                 JOIN users u ON p.user_id = u.user_id
                 JOIN post_tags pt ON p.post_id = pt.post_id
                 JOIN tags t ON pt.tag_id = t.tag_id
                 WHERE t.tag_slug = $1
                   AND p.published_at IS NOT NULL
                   AND p.published_at <= $5
                   AND p.deleted_at IS NULL
                   AND (p.created_at < $2 OR (p.created_at = $3 AND p.post_id < $4))
                   AND {resolution}
                 ORDER BY p.created_at DESC, p.post_id DESC
                 LIMIT ${limit_idx}"
            );
            let query = sqlx::query_as::<_, PostRow>(&sql)
                .bind(tag_slug)
                .bind(cursor.created_at)
                .bind(cursor.created_at)
                .bind(i64::from(cursor.post_id))
                .bind(now);
            binds
                .bind_onto(query)
                .bind(i64::from(limit))
                .fetch_all(&self.pool)
                .await?
        } else {
            // Binds: $1 tag, $2 now, $3..$7 resolution, $8 limit.
            let (resolution, binds, limit_idx) = resolution_where(viewer, 3);
            // `published_at <= $2` hides scheduled (future-dated) posts.
            let sql = format!(
                "SELECT p.post_id, p.user_id, u.username, p.title, p.slug, p.body, p.format, p.rendered_html,
                        p.created_at, p.updated_at, p.published_at, p.deleted_at, p.summary,
                        {tags} AS tags
                 FROM posts p
                 JOIN users u ON p.user_id = u.user_id
                 JOIN post_tags pt ON p.post_id = pt.post_id
                 JOIN tags t ON pt.tag_id = t.tag_id
                 WHERE t.tag_slug = $1
                   AND p.published_at IS NOT NULL
                   AND p.published_at <= $2
                   AND p.deleted_at IS NULL
                   AND {resolution}
                 ORDER BY p.created_at DESC, p.post_id DESC
                 LIMIT ${limit_idx}"
            );
            let query = sqlx::query_as::<_, PostRow>(&sql).bind(tag_slug).bind(now);
            binds
                .bind_onto(query)
                .bind(i64::from(limit))
                .fetch_all(&self.pool)
                .await?
        };

        rows.into_iter()
            .map(post_record_from_row)
            .collect::<sqlx::Result<_>>()
            .map_err(ListByTagError::Internal)
    }

    #[tracing::instrument(
        name = "storage.posts.list_user_posts_by_tag",
        skip(self),
        fields(db.system = DB::DB_SYSTEM)
    )]
    async fn list_user_posts_by_tag<'a>(
        &self,
        user_id: UserId,
        tag_slug: &Tag,
        cursor: Option<&'a PostCursor>,
        limit: u32,
        viewer: &ViewerIdentity,
        now: DateTime<Utc>,
    ) -> Result<Vec<PostRecord>, ListByTagError> {
        let tag_exists: bool =
            sqlx::query_scalar("SELECT COUNT(*) > 0 FROM tags WHERE tag_slug = $1")
                .bind(tag_slug)
                .fetch_one(&self.pool)
                .await?;

        if !tag_exists {
            return Err(ListByTagError::TagNotFound);
        }

        let tags = DB::TAGS_SUBQUERY;
        let rows = if let Some(cursor) = cursor {
            // Binds: $1 user_id, $2 tag, $3/$4 cursor, $5 post_id, $6 now,
            // $7..$11 resolution, $12 limit.
            let (resolution, binds, limit_idx) = resolution_where(viewer, 7);
            // `published_at <= $6` hides scheduled (future-dated) posts.
            let sql = format!(
                "SELECT p.post_id, p.user_id, u.username, p.title, p.slug, p.body, p.format, p.rendered_html,
                        p.created_at, p.updated_at, p.published_at, p.deleted_at, p.summary,
                        {tags} AS tags
                 FROM posts p
                 JOIN users u ON p.user_id = u.user_id
                 JOIN post_tags pt ON p.post_id = pt.post_id
                 JOIN tags t ON pt.tag_id = t.tag_id
                 WHERE p.user_id = $1
                   AND t.tag_slug = $2
                   AND p.published_at IS NOT NULL
                   AND p.published_at <= $6
                   AND p.deleted_at IS NULL
                   AND (p.created_at < $3 OR (p.created_at = $4 AND p.post_id < $5))
                   AND {resolution}
                 ORDER BY p.created_at DESC, p.post_id DESC
                 LIMIT ${limit_idx}"
            );
            let query = sqlx::query_as::<_, PostRow>(&sql)
                .bind(i64::from(user_id))
                .bind(tag_slug)
                .bind(cursor.created_at)
                .bind(cursor.created_at)
                .bind(i64::from(cursor.post_id))
                .bind(now);
            binds
                .bind_onto(query)
                .bind(i64::from(limit))
                .fetch_all(&self.pool)
                .await?
        } else {
            // Binds: $1 user_id, $2 tag, $3 now, $4..$8 resolution, $9 limit.
            let (resolution, binds, limit_idx) = resolution_where(viewer, 4);
            // `published_at <= $3` hides scheduled (future-dated) posts.
            let sql = format!(
                "SELECT p.post_id, p.user_id, u.username, p.title, p.slug, p.body, p.format, p.rendered_html,
                        p.created_at, p.updated_at, p.published_at, p.deleted_at, p.summary,
                        {tags} AS tags
                 FROM posts p
                 JOIN users u ON p.user_id = u.user_id
                 JOIN post_tags pt ON p.post_id = pt.post_id
                 JOIN tags t ON pt.tag_id = t.tag_id
                 WHERE p.user_id = $1
                   AND t.tag_slug = $2
                   AND p.published_at IS NOT NULL
                   AND p.published_at <= $3
                   AND p.deleted_at IS NULL
                   AND {resolution}
                 ORDER BY p.created_at DESC, p.post_id DESC
                 LIMIT ${limit_idx}"
            );
            let query = sqlx::query_as::<_, PostRow>(&sql)
                .bind(i64::from(user_id))
                .bind(tag_slug)
                .bind(now);
            binds
                .bind_onto(query)
                .bind(i64::from(limit))
                .fetch_all(&self.pool)
                .await?
        };

        rows.into_iter()
            .map(post_record_from_row)
            .collect::<sqlx::Result<_>>()
            .map_err(ListByTagError::Internal)
    }

    #[tracing::instrument(
        name = "storage.posts.list_tags",
        skip(self),
        fields(db.system = DB::DB_SYSTEM)
    )]
    async fn list_tags<'a>(
        &self,
        prefix: Option<&'a str>,
        limit: u32,
    ) -> sqlx::Result<Vec<TagRecord>> {
        let normalized = prefix
            .map(str::trim)
            .filter(|p| !p.is_empty())
            .map(str::to_ascii_lowercase);
        let pattern = normalized.as_deref().map(|p| format!("{p}%"));
        let limit_i64 = i64::from(limit);

        let rows = match pattern {
            Some(ref like) => {
                sqlx::query_as::<_, (i64, Tag)>(
                    "SELECT tag_id, tag_slug FROM tags
                     WHERE tag_slug LIKE $1
                     ORDER BY tag_slug
                     LIMIT $2",
                )
                .bind(like.as_str())
                .bind(limit_i64)
                .fetch_all(&self.pool)
                .await?
            }
            None => {
                sqlx::query_as::<_, (i64, Tag)>(
                    "SELECT tag_id, tag_slug FROM tags
                     ORDER BY tag_slug
                     LIMIT $1",
                )
                .bind(limit_i64)
                .fetch_all(&self.pool)
                .await?
            }
        };

        // `tag_slug` decodes straight into `Tag` via the sqlx bridge (#438), so a
        // malformed stored value is rejected as a column-decode error above.
        Ok(rows
            .into_iter()
            .map(|(tag_id, tag_slug)| TagRecord {
                tag_id: TagId::from(tag_id),
                tag_slug,
            })
            .collect())
    }

    #[tracing::instrument(
        name = "storage.posts.list_published_in_window",
        skip(self),
        fields(db.system = DB::DB_SYSTEM)
    )]
    async fn list_published_in_window(
        &self,
        surface: &common::feed::FeedSurface,
        window: &common::feed::HybridWindow,
        now: DateTime<Utc>,
        viewer: &ViewerIdentity,
    ) -> sqlx::Result<Vec<PostRecord>> {
        // ROW_NUMBER() identifies the top `min_items` posts; OR-combining with
        // `published_at >= cutoff` produces the hybrid-window union in a single
        // query. Only the JSON tag aggregation differs per backend, so the SQL
        // is shared via `DB::TAGS_SUBQUERY`.
        let cutoff = window.cutoff_date(now);
        let min_items = i64::from(window.min_items.value());
        let rows = list_published_in_window_rows::<DB>(
            &self.pool, surface, now, cutoff, min_items, viewer,
        )
        .await?;
        rows.into_iter().map(post_record_from_row).collect()
    }

    #[tracing::instrument(
        name = "storage.posts.list_posts_gone_live_between",
        skip(self),
        fields(db.system = DB::DB_SYSTEM)
    )]
    async fn list_posts_gone_live_between(
        &self,
        after: DateTime<Utc>,
        upto: DateTime<Utc>,
    ) -> sqlx::Result<Vec<GoLivePost>> {
        // `published_at > $1 AND published_at <= $2` selects exactly the posts
        // that crossed into "live" within the half-open window `(after, upto]`.
        // The standard post projection (incl. the JSON tag subquery) is reused
        // so the row decodes through `post_record_from_row`; we then keep only
        // the username + tag slugs the feed fan-out needs. No viewer filter:
        // go-live regeneration is independent of any reader's audience.
        let tags = DB::TAGS_SUBQUERY;
        let sql = format!(
            "SELECT p.post_id, p.user_id, u.username, p.title, p.slug, p.body, p.format, p.rendered_html,
                    p.created_at, p.updated_at, p.published_at, p.deleted_at, p.summary,
                    {tags} AS tags
             FROM posts p
             JOIN users u ON p.user_id = u.user_id
             WHERE p.published_at > $1
               AND p.published_at <= $2
               AND p.deleted_at IS NULL
             ORDER BY p.published_at ASC, p.post_id ASC"
        );
        let rows = sqlx::query_as::<_, PostRow>(&sql)
            .bind(after)
            .bind(upto)
            .fetch_all(&self.pool)
            .await?;
        rows.into_iter()
            .map(|row| {
                let rec = post_record_from_row(row)?;
                Ok(GoLivePost {
                    username: rec.author_username,
                    tag_slugs: rec.tags.into_iter().map(|t| t.tag_slug).collect(),
                })
            })
            .collect()
    }

    #[tracing::instrument(
        name = "storage.posts.feed_urls_needing_catchup",
        skip(self),
        fields(db.system = DB::DB_SYSTEM)
    )]
    async fn feed_urls_needing_catchup(&self, now: DateTime<Utc>) -> sqlx::Result<Vec<FeedPath>> {
        // Cached feeds live in the same database, so they are enumerated here
        // and, for each, the newest live post on that surface is compared
        // against the feed's own `generated_at`. Feed count is small, so a
        // per-feed check is simpler than a set-based join and keeps the
        // `feed_url` → surface parsing in Rust (`common::feed::parse`).
        let cached: Vec<(String, DateTime<Utc>)> =
            sqlx::query_as("SELECT feed_url, generated_at FROM feed_cache")
                .fetch_all(&self.pool)
                .await?;
        let mut needing = Vec::new();
        for (feed_url, generated_at) in cached {
            let Some((surface, format)) = common::feed::parse(&feed_url) else {
                continue;
            };
            if let Some(max) = max_published_at_for_surface::<DB>(&self.pool, &surface, now).await?
            {
                // Strictly newer => a go-live happened after this feed was last
                // generated, so it must be regenerated. Rebuild the key as a
                // `FeedPath` from the already-parsed surface (infallible; also
                // re-canonicalizes, harmless since the column is canonical).
                if max > generated_at {
                    needing.push(FeedPath::canonical(&surface, format));
                }
            }
        }
        Ok(needing)
    }
}

/// The viewer-resolution binds folded into a read query's `WHERE`, in the exact
/// left-to-right order their placeholders appear in [`resolution_where`]'s
/// fragment. `channel`/`subref` repeat (subscribers branch, then named branch)
/// because each occurrence gets its own placeholder — see [`resolution_where`].
struct ResolutionBinds {
    /// `p.user_id = $author_id` — the viewer's local user id for the author
    /// branch, or the sentinel `-1` (no post has `user_id` -1) for `Anonymous`.
    author_id: i64,
    /// `s.channel_id` for the subscribers/named `EXISTS` branches; sentinel `-1`
    /// for `Anonymous` (no subscription has `channel_id` -1).
    channel: i64,
    /// `s.subscriber_ref` for the subscribers/named branches; sentinel `""` for
    /// `Anonymous`.
    subref: String,
}

/// The viewer-resolution predicate and its binds, for folding into a read
/// query's `WHERE`. A post is returned to `viewer` only if the viewer is the
/// author OR some targeted audience admits them. See ADR-0020, Task 13.
///
/// The fragment is emitted in full for every viewer; `Anonymous` is handled by
/// binding sentinels (`author_id = -1`, `channel = -1`, `subref = ""`) that make
/// every non-`public` branch dead, so it reduces to "public posts only" without
/// a second query shape.
///
/// `start` is the next free `$n` index. The fragment uses FIVE distinct
/// placeholders (`$start`..`$start+4`) — the `channel`/`subref` pair appears once
/// in the subscribers branch and again in the named branch, and each occurrence
/// gets its own number so the binds are positional on both backends (`SQLite`
/// accepts `$n` and binds by position; see ADR-0019). The returned
/// [`ResolutionBinds`] therefore carries `channel`/`subref` once each but the
/// caller binds them **twice**, in fragment order:
/// `author_id, channel, subref, channel, subref`. Returns `(sql, binds, next)`
/// where `next` is the first free index after the fragment.
fn resolution_where(viewer: &ViewerIdentity, start: usize) -> (String, ResolutionBinds, usize) {
    let (author_id, channel, subref) = match viewer {
        ViewerIdentity::Anonymous => (-1_i64, -1_i64, String::new()),
        ViewerIdentity::Channel {
            channel_id,
            subscriber_ref,
        } => {
            // The author branch fires only for a local viewer whose
            // `subscriber_ref` parses to a real user id (the post's `user_id`).
            // A non-numeric ref (no local user) falls through to -1, so it never
            // matches `p.user_id`.
            let author_id = subscriber_ref.parse::<i64>().unwrap_or(-1);
            (author_id, i64::from(*channel_id), subscriber_ref.clone())
        }
    };
    let author = start;
    let sub_channel = start + 1;
    let sub_refnum = start + 2;
    let named_channel = start + 3;
    let named_refnum = start + 4;
    let sql = format!(
        "( p.user_id = ${author}
  OR EXISTS (
    SELECT 1 FROM post_audiences pa
    JOIN target_kinds tk ON tk.kind_id = pa.target_kind_id
    WHERE pa.post_id = p.post_id AND (
         tk.name = 'public'
      OR (tk.name = 'subscribers' AND EXISTS (
            SELECT 1 FROM subscriptions s JOIN subscription_statuses st ON st.status_id = s.status_id
            WHERE s.author_user_id = p.user_id AND s.channel_id = ${sub_channel}
              AND s.subscriber_ref = ${sub_refnum} AND st.name = 'active'))
      OR (tk.name = 'named' AND EXISTS (
            SELECT 1 FROM audience_members am
            JOIN subscriptions s ON s.subscription_id = am.subscription_id
            JOIN subscription_statuses st ON st.status_id = s.status_id
            WHERE am.audience_id = pa.audience_id AND s.channel_id = ${named_channel}
              AND s.subscriber_ref = ${named_refnum} AND st.name = 'active'))
  ))
)"
    );
    (
        sql,
        ResolutionBinds {
            author_id,
            channel,
            subref,
        },
        start + 5,
    )
}

impl ResolutionBinds {
    /// Binds the five resolution placeholders onto `query` in the exact
    /// fragment order: `author_id, channel, subref, channel, subref`. The caller
    /// must have already bound everything to the left of the fragment, and must
    /// bind the query's trailing binds (e.g. `LIMIT`) afterward.
    fn bind_onto<'q, DB>(
        &'q self,
        query: sqlx::query::QueryAs<'q, DB, PostRow, DB::Arguments<'q>>,
    ) -> sqlx::query::QueryAs<'q, DB, PostRow, DB::Arguments<'q>>
    where
        DB: Database,
        i64: sqlx::Encode<'q, DB> + sqlx::Type<DB>,
        &'q str: sqlx::Encode<'q, DB> + sqlx::Type<DB>,
    {
        query
            .bind(self.author_id)
            .bind(self.channel)
            .bind(self.subref.as_str())
            .bind(self.channel)
            .bind(self.subref.as_str())
    }
}

/// Maps an [`AudienceTarget`] to its `post_audiences` row shape:
/// `(target_kind name, audience_id)`. `Private` produces no row.
fn audience_target_row(target: &AudienceTarget) -> Option<(&'static str, Option<i64>)> {
    use common::visibility::TargetKind;
    match target {
        AudienceTarget::Public => Some((TargetKind::Public.as_str(), None)),
        AudienceTarget::Subscribers => Some((TargetKind::Subscribers.as_str(), None)),
        AudienceTarget::Named(id) => Some((TargetKind::Named.as_str(), Some(i64::from(*id)))),
        AudienceTarget::Private => None,
    }
}

/// Maps a `post_audiences` row `(target_kind name, audience_id)` back to its
/// [`AudienceTarget`] — the inverse of [`audience_target_row`], used by
/// [`PostStorage::get_post_audiences`].
///
/// `public` → [`AudienceTarget::Public`], `subscribers` →
/// [`AudienceTarget::Subscribers`], `named` (with an id) →
/// [`AudienceTarget::Named`]. A `named` row missing its id, or any kind name
/// the lookup table never holds, yields `None` (the row is dropped).
fn audience_target_from_row(kind: &str, audience_id: Option<i64>) -> Option<AudienceTarget> {
    match TargetKind::try_from(kind) {
        Ok(TargetKind::Public) => Some(AudienceTarget::Public),
        Ok(TargetKind::Subscribers) => Some(AudienceTarget::Subscribers),
        Ok(TargetKind::Named) => audience_id.map(|id| AudienceTarget::Named(AudienceId::from(id))),
        Err(()) => None,
    }
}

/// Maps an error from the idempotency-key `INSERT`. A `(user_id, key)` unique
/// violation is a [`CreatePostError::IdempotencyConflict`] (a duplicate create),
/// distinct from the post `INSERT`'s `SlugConflict` — attribution is by which
/// statement's mapper runs. Any other error passes through as `Internal`.
fn map_idempotency_insert_error(e: sqlx::Error) -> CreatePostError {
    match e {
        sqlx::Error::Database(db) if db.is_unique_violation() => {
            CreatePostError::IdempotencyConflict
        }
        e => CreatePostError::Internal(e),
    }
}

/// Writes one post row and its audience rows onto a caller-supplied transaction
/// connection, so it joins whatever transaction is open.
///
/// This is the single place that knows the post `INSERT` and the
/// unique-violation → [`CreatePostError::SlugConflict`] mapping: both
/// `create_post` (write one) and `create_posts` (write many in one transaction)
/// are pure transaction orchestration over it, so the row-write logic lives once
/// rather than being duplicated per arity.
pub(crate) async fn write_post_in_tx<DB>(
    conn: &mut DB::Connection,
    input: &CreatePostInput,
) -> Result<PostId, CreatePostError>
where
    DB: PostDialect,
    for<'q> i64: sqlx::Encode<'q, DB> + sqlx::Type<DB>,
    for<'q> Option<i64>: sqlx::Encode<'q, DB> + sqlx::Type<DB>,
    for<'q> &'q str: sqlx::Encode<'q, DB> + sqlx::Type<DB>,
    for<'q> Option<&'q str>: sqlx::Encode<'q, DB> + sqlx::Type<DB>,
    for<'q> Option<String>: sqlx::Encode<'q, DB> + sqlx::Type<DB>,
    for<'q> DateTime<Utc>: sqlx::Encode<'q, DB> + sqlx::Type<DB>,
    for<'q> Option<DateTime<Utc>>: sqlx::Encode<'q, DB> + sqlx::Type<DB>,
    // `Slug`/`PostBody` bind as themselves and `PostTitle` as `Option<&PostTitle>`
    // via the sqlx bridge (#438), which delegates to `String`; these bounds make
    // that bridge available on the generic backend (the `Option<&…>` pair covers
    // the nullable `title` bind, mirroring the `Option<&str>` the old `as_deref`
    // bind required).
    String: sqlx::Type<DB>,
    for<'q> String: sqlx::Encode<'q, DB>,
    for<'q> Option<&'q PostTitle>: sqlx::Encode<'q, DB> + sqlx::Type<DB>,
    // `summary` binds as `Option<&PostSummary>` via the ADR-0071 sqlx bridge
    // (delegates to `String`) on the create paths, mirroring the
    // `Option<&PostTitle>` bound above.
    for<'q> Option<&'q PostSummary>: sqlx::Encode<'q, DB> + sqlx::Type<DB>,
    (i64,): for<'r> sqlx::FromRow<'r, DB::Row>,
    for<'c> &'c mut DB::Connection: sqlx::Executor<'c, Database = DB>,
    for<'q> DB::Arguments<'q>: sqlx::IntoArguments<'q, DB>,
{
    let now = Utc::now();
    let format = input.format.to_string();

    let post_id = sqlx::query_scalar::<_, i64>(
        "INSERT INTO posts (user_id, title, slug, body, format, rendered_html, created_at, updated_at, published_at, summary)
         VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10)
         RETURNING post_id",
    )
    .bind(i64::from(input.user_id))
    // `Option::as_ref` → `Option<&PostTitle>` (a typed newtype bind, not an
    // `AsRef<str>` strip); the sqlx bridge encodes `Option<&PostTitle>`.
    .bind(input.title.as_ref())
    .bind(&input.slug)
    .bind(&input.body)
    .bind(format.as_str())
    .bind(&input.rendered_html)
    .bind(now)
    .bind(now)
    .bind(input.published_at)
    // `Option::as_ref` → `Option<&PostSummary>` (a typed newtype bind via the
    // ADR-0071 sqlx bridge, not an `AsRef<str>` strip); the `sqlx-newtype-bind`
    // gate forbids stripping to `&str` here.
    .bind(input.summary.as_ref())
    .fetch_one(&mut *conn)
    .await
    .map_err(|e| match e {
        sqlx::Error::Database(db) if db.is_unique_violation() => CreatePostError::SlugConflict,
        e => CreatePostError::Internal(e),
    })?;
    let post_id = PostId::from(post_id);

    replace_post_audiences::<DB>(conn, post_id, &input.audiences).await?;

    // Register the idempotency key in the same transaction as the post. This
    // INSERT has its own unique-violation mapping — a `(user_id, key)` clash is
    // an `IdempotencyConflict` (a duplicate create), distinct from the post
    // INSERT's `SlugConflict` above. Attribution is by which statement's
    // `map_err` fires, not by inspecting the constraint name.
    if let Some(key) = input.idempotency_key.as_deref() {
        sqlx::query("INSERT INTO idempotency_keys (user_id, key, post_id) VALUES ($1, $2, $3)")
            .bind(i64::from(input.user_id))
            .bind(key)
            .bind(i64::from(post_id))
            .execute(&mut *conn)
            .await
            .map_err(map_idempotency_insert_error)?;
    }

    Ok(post_id)
}

/// Replaces a post's `post_audiences` rows to exactly match `audiences`.
///
/// Deletes every existing row for `post_id`, then inserts one row per targeting
/// entry (`Public`/`Subscribers` carry a NULL `audience_id`; `Named(id)` carries
/// the id; `Private` and an empty vec leave the post with no rows). Runs on the
/// caller's executor so it shares the create/update transaction. See ADR-0020.
pub(crate) async fn replace_post_audiences<DB>(
    conn: &mut DB::Connection,
    post_id: PostId,
    audiences: &[AudienceTarget],
) -> sqlx::Result<()>
where
    DB: PostDialect,
    for<'q> i64: sqlx::Encode<'q, DB> + sqlx::Type<DB>,
    for<'q> Option<i64>: sqlx::Encode<'q, DB> + sqlx::Type<DB>,
    for<'q> &'q str: sqlx::Encode<'q, DB> + sqlx::Type<DB>,
    for<'c> &'c mut DB::Connection: sqlx::Executor<'c, Database = DB>,
    for<'q> DB::Arguments<'q>: sqlx::IntoArguments<'q, DB>,
{
    sqlx::query(DB::DELETE_POST_AUDIENCES)
        .bind(i64::from(post_id))
        .execute(&mut *conn)
        .await?;
    for target in audiences {
        if let Some((kind_name, audience_id)) = audience_target_row(target) {
            sqlx::query(DB::INSERT_POST_AUDIENCE)
                .bind(i64::from(post_id))
                .bind(audience_id)
                .bind(kind_name)
                .execute(&mut *conn)
                .await?;
        }
    }
    Ok(())
}

/// Runs the hybrid-window query for `surface`, returning raw [`PostRow`]s.
///
/// Shared across backends: the four `FeedSurface` variants differ only in the
/// ranked-CTE source/predicate and bind list, and the JSON tag aggregation is
/// supplied by [`PostDialect::TAGS_SUBQUERY`].
async fn list_published_in_window_rows<DB>(
    pool: &Pool<DB>,
    surface: &common::feed::FeedSurface,
    now: DateTime<Utc>,
    cutoff: DateTime<Utc>,
    min_items: i64,
    viewer: &ViewerIdentity,
) -> sqlx::Result<Vec<PostRow>>
where
    DB: PostDialect,
    PostRow: for<'r> sqlx::FromRow<'r, DB::Row>,
    for<'q> i64: sqlx::Encode<'q, DB> + sqlx::Type<DB>,
    for<'q> &'q str: sqlx::Encode<'q, DB> + sqlx::Type<DB>,
    for<'q> DateTime<Utc>: sqlx::Encode<'q, DB> + sqlx::Type<DB>,
    // `Username`/`Tag` bind as themselves via the sqlx bridge (#438), which
    // delegates to `String`; this pair makes that bridge available on the generic
    // backend for the surface `username`/`tag` binds.
    String: sqlx::Type<DB>,
    for<'q> String: sqlx::Encode<'q, DB>,
    for<'c> &'c Pool<DB>: sqlx::Executor<'c, Database = DB>,
    for<'q> DB::Arguments<'q>: sqlx::IntoArguments<'q, DB>,
{
    use common::feed::FeedSurface;
    let tags = DB::TAGS_SUBQUERY;
    match surface {
        FeedSurface::Site => {
            // Binds: $1 now, $2 min_items, $3 cutoff, $4..$8 resolution.
            let (resolution, binds, _) = resolution_where(viewer, 4);
            let sql = window_sql(surface, tags, &resolution);
            let query = sqlx::query_as::<_, PostRow>(&sql)
                .bind(now)
                .bind(min_items)
                .bind(cutoff);
            binds.bind_onto(query).fetch_all(pool).await
        }
        FeedSurface::User { username } => {
            // Binds: $1 now, $2 username, $3 min_items, $4 cutoff,
            // $5..$9 resolution.
            let (resolution, binds, _) = resolution_where(viewer, 5);
            let sql = window_sql(surface, tags, &resolution);
            let query = sqlx::query_as::<_, PostRow>(&sql)
                .bind(now)
                .bind(username)
                .bind(min_items)
                .bind(cutoff);
            binds.bind_onto(query).fetch_all(pool).await
        }
        FeedSurface::SiteTag { tag } => {
            // Binds: $1 now, $2 tag, $3 min_items, $4 cutoff, $5..$9 resolution.
            let (resolution, binds, _) = resolution_where(viewer, 5);
            let sql = window_sql(surface, tags, &resolution);
            let query = sqlx::query_as::<_, PostRow>(&sql)
                .bind(now)
                .bind(tag)
                .bind(min_items)
                .bind(cutoff);
            binds.bind_onto(query).fetch_all(pool).await
        }
        FeedSurface::UserTag { username, tag } => {
            // Binds: $1 now, $2 username, $3 tag, $4 min_items, $5 cutoff,
            // $6..$10 resolution.
            let (resolution, binds, _) = resolution_where(viewer, 6);
            let sql = window_sql(surface, tags, &resolution);
            let query = sqlx::query_as::<_, PostRow>(&sql)
                .bind(now)
                .bind(username)
                .bind(tag)
                .bind(min_items)
                .bind(cutoff);
            binds.bind_onto(query).fetch_all(pool).await
        }
    }
}

/// Assembles the hybrid-window SQL for `surface`.
///
/// Pure string construction with no DB generics: the four near-identical
/// templates — differing only in the ranked-CTE source/predicate and bind
/// placeholders — live here, while [`list_published_in_window_rows`] keeps the
/// generic `where`-clause, per-surface bind list, and execution. `tags` supplies
/// the JSON tag aggregation ([`PostDialect::TAGS_SUBQUERY`]) and `resolution` the
/// audience-resolution predicate.
fn window_sql(surface: &common::feed::FeedSurface, tags: &str, resolution: &str) -> String {
    use common::feed::FeedSurface;
    match surface {
        FeedSurface::Site => format!(
            "WITH ranked AS (
     SELECT p.post_id, p.published_at,
            ROW_NUMBER() OVER (ORDER BY p.published_at DESC, p.post_id DESC) AS rn
     FROM posts p
     WHERE p.published_at IS NOT NULL
       AND p.deleted_at IS NULL
       AND p.published_at <= $1
 )
 SELECT p.post_id, p.user_id, u.username, p.title, p.slug, p.body, p.format, p.rendered_html,
        p.created_at, p.updated_at, p.published_at, p.deleted_at, p.summary,
        {tags} AS tags
 FROM ranked r
 JOIN posts p ON p.post_id = r.post_id
 JOIN users u ON p.user_id = u.user_id
 WHERE (r.rn <= $2 OR r.published_at >= $3)
   AND {resolution}
 ORDER BY p.published_at DESC, p.post_id DESC"
        ),
        FeedSurface::User { .. } => format!(
            "WITH ranked AS (
     SELECT p.post_id, p.published_at,
            ROW_NUMBER() OVER (ORDER BY p.published_at DESC, p.post_id DESC) AS rn
     FROM posts p
     JOIN users u ON p.user_id = u.user_id
     WHERE p.published_at IS NOT NULL
       AND p.deleted_at IS NULL
       AND p.published_at <= $1
       AND u.username = $2
 )
 SELECT p.post_id, p.user_id, u.username, p.title, p.slug, p.body, p.format, p.rendered_html,
        p.created_at, p.updated_at, p.published_at, p.deleted_at, p.summary,
        {tags} AS tags
 FROM ranked r
 JOIN posts p ON p.post_id = r.post_id
 JOIN users u ON p.user_id = u.user_id
 WHERE (r.rn <= $3 OR r.published_at >= $4)
   AND {resolution}
 ORDER BY p.published_at DESC, p.post_id DESC"
        ),
        FeedSurface::SiteTag { .. } => format!(
            "WITH ranked AS (
     SELECT p.post_id, p.published_at,
            ROW_NUMBER() OVER (ORDER BY p.published_at DESC, p.post_id DESC) AS rn
     FROM posts p
     JOIN post_tags pt ON p.post_id = pt.post_id
     JOIN tags t ON pt.tag_id = t.tag_id
     WHERE p.published_at IS NOT NULL
       AND p.deleted_at IS NULL
       AND p.published_at <= $1
       AND t.tag_slug = $2
 )
 SELECT p.post_id, p.user_id, u.username, p.title, p.slug, p.body, p.format, p.rendered_html,
        p.created_at, p.updated_at, p.published_at, p.deleted_at, p.summary,
        {tags} AS tags
 FROM ranked r
 JOIN posts p ON p.post_id = r.post_id
 JOIN users u ON p.user_id = u.user_id
 WHERE (r.rn <= $3 OR r.published_at >= $4)
   AND {resolution}
 ORDER BY p.published_at DESC, p.post_id DESC"
        ),
        FeedSurface::UserTag { .. } => format!(
            "WITH ranked AS (
     SELECT p.post_id, p.published_at,
            ROW_NUMBER() OVER (ORDER BY p.published_at DESC, p.post_id DESC) AS rn
     FROM posts p
     JOIN users u ON p.user_id = u.user_id
     JOIN post_tags pt ON p.post_id = pt.post_id
     JOIN tags t ON pt.tag_id = t.tag_id
     WHERE p.published_at IS NOT NULL
       AND p.deleted_at IS NULL
       AND p.published_at <= $1
       AND u.username = $2
       AND t.tag_slug = $3
 )
 SELECT p.post_id, p.user_id, u.username, p.title, p.slug, p.body, p.format, p.rendered_html,
        p.created_at, p.updated_at, p.published_at, p.deleted_at, p.summary,
        {tags} AS tags
 FROM ranked r
 JOIN posts p ON p.post_id = r.post_id
 JOIN users u ON p.user_id = u.user_id
 WHERE (r.rn <= $4 OR r.published_at >= $5)
   AND {resolution}
 ORDER BY p.published_at DESC, p.post_id DESC"
        ),
    }
}

/// The most recent `published_at` of a *live* post (`published_at <= now`, not
/// deleted) on `surface`, or `None` when the surface has no live post. Each
/// surface variant adds exactly the joins/predicates that define its post set,
/// mirroring the window query's surface filters. Used by
/// [`PostStorage::feed_urls_needing_catchup`] to detect a cached feed that is
/// stale relative to a go-live the worker may have missed while down.
async fn max_published_at_for_surface<DB>(
    pool: &Pool<DB>,
    surface: &common::feed::FeedSurface,
    now: DateTime<Utc>,
) -> sqlx::Result<Option<DateTime<Utc>>>
where
    DB: PostDialect,
    (DateTime<Utc>,): for<'r> sqlx::FromRow<'r, DB::Row>,
    for<'q> &'q str: sqlx::Encode<'q, DB> + sqlx::Type<DB>,
    for<'q> DateTime<Utc>: sqlx::Encode<'q, DB> + sqlx::Type<DB>,
    // `Username`/`Tag` bind as themselves via the sqlx bridge (#438), which
    // delegates to `String`; this pair makes that bridge available on the generic
    // backend for the surface `username`/`tag` binds.
    String: sqlx::Type<DB>,
    for<'q> String: sqlx::Encode<'q, DB>,
    for<'c> &'c Pool<DB>: sqlx::Executor<'c, Database = DB>,
    for<'q> DB::Arguments<'q>: sqlx::IntoArguments<'q, DB>,
{
    use common::feed::FeedSurface;
    let row: Option<(DateTime<Utc>,)> = match surface {
        FeedSurface::Site => {
            sqlx::query_as(
                "SELECT p.published_at FROM posts p
                 WHERE p.published_at IS NOT NULL AND p.published_at <= $1
                   AND p.deleted_at IS NULL
                 ORDER BY p.published_at DESC LIMIT 1",
            )
            .bind(now)
            .fetch_optional(pool)
            .await?
        }
        FeedSurface::User { username } => {
            sqlx::query_as(
                "SELECT p.published_at FROM posts p
                 JOIN users u ON p.user_id = u.user_id
                 WHERE p.published_at IS NOT NULL AND p.published_at <= $1
                   AND p.deleted_at IS NULL AND u.username = $2
                 ORDER BY p.published_at DESC LIMIT 1",
            )
            .bind(now)
            .bind(username)
            .fetch_optional(pool)
            .await?
        }
        FeedSurface::SiteTag { tag } => {
            sqlx::query_as(
                "SELECT p.published_at FROM posts p
                 JOIN post_tags pt ON p.post_id = pt.post_id
                 JOIN tags t ON pt.tag_id = t.tag_id
                 WHERE p.published_at IS NOT NULL AND p.published_at <= $1
                   AND p.deleted_at IS NULL AND t.tag_slug = $2
                 ORDER BY p.published_at DESC LIMIT 1",
            )
            .bind(now)
            .bind(tag)
            .fetch_optional(pool)
            .await?
        }
        FeedSurface::UserTag { username, tag } => {
            sqlx::query_as(
                "SELECT p.published_at FROM posts p
                 JOIN users u ON p.user_id = u.user_id
                 JOIN post_tags pt ON p.post_id = pt.post_id
                 JOIN tags t ON pt.tag_id = t.tag_id
                 WHERE p.published_at IS NOT NULL AND p.published_at <= $1
                   AND p.deleted_at IS NULL AND u.username = $2 AND t.tag_slug = $3
                 ORDER BY p.published_at DESC LIMIT 1",
            )
            .bind(now)
            .bind(username)
            .bind(tag)
            .fetch_optional(pool)
            .await?
        }
    };
    Ok(row.map(|(published_at,)| published_at))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::{backends, seed_user, Backend, CloseablePool};
    use common::test_support::{
        parse_post_summary, parse_post_title, parse_slug, parse_tag, parse_tag_label,
        parse_username,
    };
    use rstest::*;
    use rstest_reuse::*;

    #[test]
    fn map_idempotency_insert_error_passes_non_unique_errors_through() {
        // A unique violation becomes IdempotencyConflict (covered by the create
        // dedup integration test); any other error passes through as Internal.
        let mapped = map_idempotency_insert_error(sqlx::Error::PoolClosed);
        assert!(matches!(mapped, CreatePostError::Internal(_)));
    }

    #[test]
    fn audience_target_from_row_maps_every_kind() {
        // Each lookup-table kind maps to its target; `named` carries the id.
        assert_eq!(
            audience_target_from_row("public", None),
            Some(AudienceTarget::Public)
        );
        assert_eq!(
            audience_target_from_row("subscribers", None),
            Some(AudienceTarget::Subscribers)
        );
        assert_eq!(
            audience_target_from_row("named", Some(7)),
            Some(AudienceTarget::Named(AudienceId::from(7)))
        );
        // A `named` row missing its id, or an unknown kind name, is dropped.
        assert_eq!(audience_target_from_row("named", None), None);
        assert_eq!(audience_target_from_row("bogus", Some(1)), None);
    }

    fn post_tag(slug: &str, display: &str) -> PostTag {
        PostTag {
            post_id: PostId::from(1),
            tag_id: TagId::from(0),
            tag_slug: parse_tag(slug),
            tag_display: parse_tag_label(display),
        }
    }

    #[test]
    fn post_tag_diff_adds_removes_keeps() {
        let existing = vec![post_tag("rust", "Rust"), post_tag("leptos", "Leptos")];
        let desired: Vec<TagLabel> = vec![
            // Same slug as an existing tag (different casing): kept, not re-added.
            parse_tag_label("Rust"),
            // New slug: added.
            parse_tag_label("wasm"),
        ];

        let diff = post_tag_diff(&existing, &desired);

        let added: Vec<String> = diff.to_add.iter().map(ToString::to_string).collect();
        assert_eq!(added, vec!["wasm".to_string()]);
        let removed: Vec<String> = diff.to_remove.iter().map(ToString::to_string).collect();
        assert_eq!(removed, vec!["leptos".to_string()]);
    }

    #[test]
    fn tagging_error_display_post_not_found() {
        let err = TaggingError::PostNotFound;
        assert_eq!(err.to_string(), "post not found");
    }

    #[test]
    fn tagging_error_display_tag_not_found() {
        let err = TaggingError::TagNotFound;
        assert_eq!(err.to_string(), "tag not found");
    }

    #[test]
    fn tagging_error_display_already_tagged() {
        let err = TaggingError::AlreadyTagged;
        assert_eq!(err.to_string(), "post is already tagged with this tag");
    }

    #[test]
    fn tagging_error_debug() {
        let err = TaggingError::PostNotFound;
        let debug_str = format!("{err:?}");
        assert!(debug_str.contains("PostNotFound"));

        let err2 = TaggingError::TagNotFound;
        let debug_str2 = format!("{err2:?}");
        assert!(debug_str2.contains("TagNotFound"));

        let err3 = TaggingError::AlreadyTagged;
        let debug_str3 = format!("{err3:?}");
        assert!(debug_str3.contains("AlreadyTagged"));
    }

    #[test]
    fn list_by_tag_error_display_tag_not_found() {
        let err = ListByTagError::TagNotFound;
        assert_eq!(err.to_string(), "tag not found");
    }

    #[test]
    fn list_by_tag_error_debug() {
        let err = ListByTagError::TagNotFound;
        let debug_str = format!("{err:?}");
        assert!(debug_str.contains("TagNotFound"));
    }

    #[test]
    fn fallback_summary_label_prefers_body_then_title_then_slug() {
        let mut post = PostRecord {
            post_id: PostId::from(1),
            user_id: UserId::from(1),
            author_username: parse_username("author"),
            title: Some("My Title".into()),
            slug: parse_slug("my-slug"),
            body: "\n\n   The first non-empty line of the body is here. \n\n Another line.".into(),
            format: PostFormat::Markdown,
            rendered_html: RenderedHtml::from_trusted(
                "<p>The first non-empty line of the body is here.</p>",
            ),
            created_at: Utc::now(),
            updated_at: Utc::now(),
            published_at: None,
            deleted_at: None,
            summary: None,
            tags: vec![],
        };

        // Case 1: Body is populated. It should use the first non-empty line.
        assert_eq!(
            post.fallback_summary_label(),
            "The first non-empty line of the body is here."
        );

        // Case 2: Body is empty but title is populated.
        post.body = "".into();
        assert_eq!(post.fallback_summary_label(), "My Title");

        // Case 2b: An empty-after-trim title (PostTitle is infallible) must not mint an
        // empty PostSummary — it falls through to the always-non-empty slug.
        post.title = Some("   ".into());
        assert_eq!(post.fallback_summary_label(), "my-slug");

        // Case 3: Body and title are empty. It should use the slug.
        post.title = None;
        assert_eq!(post.fallback_summary_label(), "my-slug");
    }

    #[test]
    fn permalink_formats_username_date_and_slug() {
        use chrono::TimeZone;
        let post = PostRecord {
            post_id: PostId::from(1),
            user_id: UserId::from(1),
            author_username: parse_username("author"),
            title: Some("My Title".into()),
            slug: parse_slug("hello-world"),
            body: "My body".into(),
            format: PostFormat::Markdown,
            rendered_html: RenderedHtml::from_trusted("<p>My body</p>"),
            created_at: Utc.with_ymd_and_hms(2026, 4, 12, 8, 30, 0).unwrap(),
            updated_at: Utc.with_ymd_and_hms(2026, 4, 12, 8, 30, 0).unwrap(),
            published_at: Some(Utc.with_ymd_and_hms(2026, 4, 12, 8, 30, 0).unwrap()),
            deleted_at: None,
            summary: None,
            tags: vec![],
        };

        assert_eq!(post.permalink(), "/~author/2026/04/12/hello-world");
    }

    #[apply(backends)]
    #[tokio::test]
    async fn create_post_persists_summary(#[case] backend: Backend) {
        let env = backend.setup().await;
        let user_id = seed_user(&env.state).await;
        let posts = &*env.state.posts;
        let input = CreatePostInput {
            user_id,
            title: Some("Test Title".into()),
            slug: parse_slug("test-slug"),
            body: "Test body".into(),
            format: PostFormat::Markdown,
            rendered_html: RenderedHtml::from_trusted("<p>Test body</p>"),
            published_at: None,
            summary: Some(parse_post_summary("the summary")),
            audiences: vec![AudienceTarget::Public],
            idempotency_key: None,
        };

        let post_id = posts.create_post(&input).await.unwrap();
        let post = posts
            .get_post_by_id(post_id, &ViewerIdentity::Anonymous)
            .await
            .unwrap()
            .unwrap();

        assert_eq!(post.summary, Some(parse_post_summary("the summary")));
    }

    #[apply(backends)]
    #[tokio::test]
    async fn reading_post_with_overlong_summary_in_db_errors(#[case] backend: Backend) {
        // A pre-existing row whose summary exceeds MAX_POST_SUMMARY_CHARS (the
        // column is unbounded TEXT) must surface as an error at the strict read
        // boundary — never a panic — because the validating sqlx `Decode` fails
        // closed through `PostSummary`'s `FromStr`. The over-cap value is
        // unconstructible via the newtype, so it is forced in with raw SQL.
        // Mirrors `users.rs`'s overlong-display-name fail-closed test.
        let env = backend.setup().await;
        let user_id = seed_user(&env.state).await;
        let posts = &*env.state.posts;
        let input = CreatePostInput {
            user_id,
            title: Some("Test Title".into()),
            slug: parse_slug("overlong-summary"),
            body: "Test body".into(),
            format: PostFormat::Markdown,
            rendered_html: RenderedHtml::from_trusted("<p>Test body</p>"),
            published_at: None,
            summary: Some(parse_post_summary("valid summary")),
            audiences: vec![AudienceTarget::Public],
            idempotency_key: None,
        };
        let post_id = posts.create_post(&input).await.unwrap();

        let overlong = "a".repeat(common::post_summary::MAX_POST_SUMMARY_CHARS + 1);
        let sql = format!(
            "UPDATE posts SET summary='{overlong}' WHERE post_id={}",
            i64::from(post_id)
        );
        env.base.pool().execute(sql.as_str()).await.unwrap();

        let result = posts
            .get_post_by_id(post_id, &ViewerIdentity::Anonymous)
            .await;
        assert!(result.is_err());
    }

    #[apply(backends)]
    #[tokio::test]
    async fn create_post_with_closed_pool_returns_error(#[case] backend: Backend) {
        let env = backend.setup().await;
        env.base.close_pool().await;
        let input = CreatePostInput {
            user_id: UserId::from(1),
            title: Some("Test".into()),
            slug: parse_slug("test-post"),
            body: "body".into(),
            format: PostFormat::Markdown,
            rendered_html: RenderedHtml::from_trusted("<p>body</p>"),
            published_at: None,
            summary: None,
            audiences: vec![AudienceTarget::Public],
            idempotency_key: None,
        };
        let result = env.state.posts.create_post(&input).await;
        assert!(result.is_err());
    }

    #[apply(backends)]
    #[tokio::test]
    async fn get_post_by_id_with_closed_pool_returns_error(#[case] backend: Backend) {
        let env = backend.setup().await;
        env.base.close_pool().await;
        let result = env
            .state
            .posts
            .get_post_by_id(PostId::from(1), &ViewerIdentity::Anonymous)
            .await;
        assert!(result.is_err());
    }

    #[apply(backends)]
    #[tokio::test]
    async fn list_published_with_closed_pool_returns_error(#[case] backend: Backend) {
        let env = backend.setup().await;
        env.base.close_pool().await;
        let result = env
            .state
            .posts
            .list_published(None, 10, &ViewerIdentity::Anonymous, Utc::now())
            .await;
        assert!(result.is_err());
    }

    #[apply(backends)]
    #[tokio::test]
    async fn tag_post_insert_error_returns_internal(#[case] backend: Backend) {
        let env = backend.setup().await;
        let uid = seed_user(&env.state).await;
        let post_id = env
            .state
            .posts
            .create_post(&CreatePostInput {
                user_id: uid,
                title: Some("Post".into()),
                slug: parse_slug("post"),
                body: "body".into(),
                format: PostFormat::Markdown,
                rendered_html: RenderedHtml::from_trusted("<p>body</p>"),
                published_at: None,
                summary: None,
                audiences: vec![AudienceTarget::Public],
                idempotency_key: None,
            })
            .await
            .unwrap();

        // Break the post_tags INSERT (but not the existence check or tag insert) so it
        // returns a non-unique Database error: exercises the catch-all Internal arm and
        // the BEGIN IMMEDIATE rollback path on an unexpected failure.
        env.base
            .pool()
            .execute("ALTER TABLE post_tags RENAME COLUMN tag_display TO tag_display_x")
            .await
            .unwrap();

        let result = env
            .state
            .posts
            .tag_post(post_id, &parse_tag_label("rust"))
            .await;
        assert!(matches!(result, Err(TaggingError::Internal(_))));
    }

    #[apply(backends)]
    #[tokio::test]
    async fn list_collection_by_user_orders_by_updated_at_desc_and_excludes_deleted(
        #[case] backend: Backend,
    ) {
        let env = backend.setup().await;
        let uid = seed_user(&env.state).await;
        let now = Utc::now();

        let mk = |slug: &str, published: bool| CreatePostInput {
            user_id: uid,
            title: Some(format!("Post {slug}").into()),
            slug: parse_slug(slug),
            body: "body".into(),
            format: PostFormat::Markdown,
            rendered_html: RenderedHtml::from_trusted("<p>body</p>"),
            published_at: published.then_some(now - chrono::Duration::minutes(30)),
            summary: None,
            audiences: vec![AudienceTarget::Public],
            idempotency_key: None,
        };

        // Post 1: draft. Post 2: published. Post 3: soft-deleted (excluded).
        let post1_id = env
            .state
            .posts
            .create_post(&mk("draft-post", false))
            .await
            .unwrap();
        let post2_id = env
            .state
            .posts
            .create_post(&mk("published-post", true))
            .await
            .unwrap();
        let post3_id = env
            .state
            .posts
            .create_post(&mk("deleted-post", true))
            .await
            .unwrap();

        // Give distinct updated_at (post2 more recent than post1) and soft-delete post3.
        // ISO-8601 literals inlined so both backends accept the raw statement.
        let t_older = (now - chrono::Duration::hours(2)).to_rfc3339();
        let t_newer = (now - chrono::Duration::hours(1)).to_rfc3339();
        let now_str = now.to_rfc3339();
        env.base
            .pool()
            .execute(&format!(
                "UPDATE posts SET updated_at = '{t_older}' WHERE post_id = {post1_id}"
            ))
            .await
            .unwrap();
        env.base
            .pool()
            .execute(&format!(
                "UPDATE posts SET updated_at = '{t_newer}' WHERE post_id = {post2_id}"
            ))
            .await
            .unwrap();
        env.base
            .pool()
            .execute(&format!(
                "UPDATE posts SET deleted_at = '{now_str}' WHERE post_id = {post3_id}"
            ))
            .await
            .unwrap();

        let results = env
            .state
            .posts
            .list_collection_by_user(uid, None, 10)
            .await
            .unwrap();

        // Should have 2 posts (draft and published, not deleted)
        assert_eq!(results.len(), 2);

        // Check they are ordered by updated_at DESC (post2 updated more recently)
        assert_eq!(results[0].post_id, post2_id);
        assert_eq!(results[1].post_id, post1_id);

        // Verify draft is included
        assert!(results
            .iter()
            .any(|p| p.post_id == post1_id && p.published_at.is_none()));

        // Verify published is included
        assert!(results
            .iter()
            .any(|p| p.post_id == post2_id && p.published_at.is_some()));

        // Verify deleted is not included
        assert!(!results.iter().any(|p| p.post_id == post3_id));
    }

    // Behavior-preserving translation of the former inline `web::posts::mod`
    // `UpdatePostError` mapper: not-found/unauthorized mask as a 404, internal
    // is a masked storage failure.
    #[test]
    fn from_update_post_error_maps_variants() {
        use host::error::{ErrorKind, InternalError};

        let not_found: InternalError = UpdatePostError::NotFound.into();
        assert_eq!(not_found.kind(), ErrorKind::NotFound);
        assert_eq!(not_found.public_message(), "Post not found");

        let unauthorized: InternalError = UpdatePostError::Unauthorized.into();
        assert_eq!(unauthorized.kind(), ErrorKind::NotFound);
        assert_eq!(unauthorized.public_message(), "Post not found");

        let internal: InternalError = UpdatePostError::Internal(sqlx::Error::PoolClosed).into();
        assert_eq!(internal.kind(), ErrorKind::Storage);
        assert_eq!(internal.public_message(), "storage operation failed");
    }

    // The `tag_post`/`untag_post` lift masked as a server error
    // (`"server operation failed"`, kind `Internal`); the typed `TaggingError`
    // is now preserved on the operator side rather than stringified.
    #[test]
    fn from_tagging_error_maps_to_server() {
        use host::error::{ErrorKind, InternalError};

        let error: InternalError = TaggingError::PostNotFound.into();
        assert_eq!(error.kind(), ErrorKind::Internal);
        assert_eq!(error.public_message(), "server operation failed");
        // The typed source is preserved (not flattened to the wire message).
        assert!(error.operator_message().contains("post not found"));
    }

    // -- Cursor + effectful helper tests (Cluster C push-down, #334) --

    #[test]
    fn to_post_cursor_round_trips_through_parse() {
        use chrono::TimeZone;
        let post = PostRecord {
            post_id: PostId::from(42),
            user_id: UserId::from(1),
            author_username: parse_username("author"),
            title: None,
            slug: parse_slug("hello-world"),
            body: "".into(),
            format: PostFormat::Markdown,
            rendered_html: RenderedHtml::from_trusted(""),
            created_at: Utc.with_ymd_and_hms(2026, 4, 12, 8, 30, 0).unwrap(),
            updated_at: Utc.with_ymd_and_hms(2026, 4, 12, 8, 30, 0).unwrap(),
            published_at: None,
            deleted_at: None,
            summary: None,
            tags: vec![],
        };

        let cursor = to_post_cursor(&post);
        let parsed = parse_post_cursor(Some(cursor.created_at), Some(cursor.post_id))
            .unwrap()
            .expect("both components present yields a cursor");
        assert_eq!(parsed.created_at, post.created_at);
        assert_eq!(parsed.post_id, post.post_id);
    }

    #[test]
    fn parse_post_cursor_accepts_empty_cursor() {
        assert!(parse_post_cursor(None, None).unwrap().is_none());
    }

    #[test]
    fn parse_post_cursor_rejects_half_a_cursor() {
        use chrono::TimeZone;
        assert!(parse_post_cursor(
            Some(Utc.with_ymd_and_hms(2026, 4, 12, 8, 30, 0).unwrap()),
            None
        )
        .is_err());
    }

    #[test]
    fn list_by_tag_rows_maps_each_arm() {
        assert!(list_by_tag_rows(Ok(vec![])).is_ok());

        let tag_not_found = list_by_tag_rows(Err(ListByTagError::TagNotFound));
        assert!(matches!(tag_not_found, Ok(rows) if rows.is_empty()));

        let internal = list_by_tag_rows(Err(ListByTagError::Internal(sqlx::Error::PoolClosed)));
        assert!(internal.is_err());
    }

    #[apply(backends)]
    #[tokio::test]
    async fn fetch_post_record_returns_seeded_post_and_none_for_missing(#[case] backend: Backend) {
        let env = backend.setup().await;
        let user_id = seed_user(&env.state).await;
        let posts = &*env.state.posts;
        let ids = crate::test_support::seed_posts(&env.state, user_id, 1, true).await;
        let record = posts
            .get_post_by_id(ids[0], &ViewerIdentity::Anonymous)
            .await
            .unwrap()
            .unwrap();
        let (year, month, day) = (
            record.created_at.year(),
            record.created_at.month(),
            record.created_at.day(),
        );

        // A published, public post is visible to an anonymous viewer at its permalink.
        let found = fetch_post_record(
            posts,
            &ViewerIdentity::Anonymous,
            &record.author_username,
            year,
            month,
            day,
            &record.slug,
        )
        .await
        .unwrap();
        assert_eq!(found.map(|p| p.post_id), Some(record.post_id));

        // A permalink with no matching post resolves to None (not an error).
        let missing = fetch_post_record(
            posts,
            &ViewerIdentity::Anonymous,
            &record.author_username,
            year,
            month,
            day,
            &parse_slug("no-such-slug"),
        )
        .await
        .unwrap();
        assert!(missing.is_none());
    }

    #[apply(backends)]
    #[tokio::test]
    async fn apply_post_tag_diff_adds_then_removes_tags(#[case] backend: Backend) {
        let env = backend.setup().await;
        let user_id = seed_user(&env.state).await;
        let posts = &*env.state.posts;
        let post_id = posts
            .create_post(&CreatePostInput {
                user_id,
                title: Some("Post".into()),
                slug: parse_slug("post"),
                body: "body".into(),
                format: PostFormat::Markdown,
                rendered_html: RenderedHtml::from_trusted("<p>body</p>"),
                published_at: None,
                summary: None,
                audiences: vec![AudienceTarget::Public],
                idempotency_key: None,
            })
            .await
            .unwrap();

        // Adding two tags then reading back yields both slugs.
        apply_post_tag_diff(
            posts,
            post_id,
            &[parse_tag_label("rust"), parse_tag_label("web")],
        )
        .await
        .unwrap();
        let mut slugs: Vec<String> = posts
            .get_tags_for_post(post_id)
            .await
            .unwrap()
            .iter()
            .map(|t| t.tag_slug.to_string())
            .collect();
        slugs.sort();
        assert_eq!(slugs, vec!["rust".to_string(), "web".to_string()]);

        // Narrowing the desired set removes the dropped tag.
        apply_post_tag_diff(posts, post_id, &[parse_tag_label("rust")])
            .await
            .unwrap();
        let remaining: Vec<String> = posts
            .get_tags_for_post(post_id)
            .await
            .unwrap()
            .iter()
            .map(|t| t.tag_slug.to_string())
            .collect();
        assert_eq!(remaining, vec!["rust".to_string()]);
    }

    #[apply(backends)]
    #[tokio::test]
    async fn tag_post_round_trips_slug_and_label(#[case] backend: Backend) {
        let env = backend.setup().await;
        let user_id = seed_user(&env.state).await;
        let posts = &*env.state.posts;
        let post_id = posts
            .create_post(&CreatePostInput {
                user_id,
                title: Some("Post".into()),
                slug: parse_slug("post"),
                body: "body".into(),
                format: PostFormat::Markdown,
                rendered_html: RenderedHtml::from_trusted("<p>body</p>"),
                published_at: None,
                summary: None,
                audiences: vec![AudienceTarget::Public],
                idempotency_key: None,
            })
            .await
            .unwrap();

        // Tagging with a case-preserving label stores the canonical slug and the
        // author's casing; both read back intact on either backend.
        posts
            .tag_post(post_id, &parse_tag_label("Rust"))
            .await
            .unwrap();

        let tags = posts.get_tags_for_post(post_id).await.unwrap();
        assert_eq!(tags.len(), 1);
        assert_eq!(tags[0].tag_slug, "rust"); // canonical slug (lowercased)
        assert_eq!(tags[0].tag_display, "Rust"); // author casing preserved
    }

    #[apply(backends)]
    #[tokio::test]
    async fn post_round_trips_slug_title_body_username_and_tag(#[case] backend: Backend) {
        // Keep the whole `TestEnv` bound: dropping `base` unlinks the SQLite file
        // (ADR-0053 TempDir hazard).
        let env = backend.setup().await;
        let user_id = seed_user(&env.state).await; // username "testuser"
        let posts = &*env.state.posts;

        // `create_post` binds a typed `Slug`, `Option<&PostTitle>`, and `&PostBody`;
        // `tag_post` binds a `TagLabel`. The read decodes the `slug`/`title`/`body`/
        // author-`username` columns and the JSON `tag_slug`/`tag_display` straight
        // back into their newtypes — exercising both bridge directions (#438).
        let slug: Slug = parse_slug("round-trip");
        let title = parse_post_title("A Round-Trip Title");
        let body: PostBody = "the round-trip body".into();
        let post_id = posts
            .create_post(&CreatePostInput {
                user_id,
                title: Some(title.clone()),
                slug: slug.clone(),
                body: body.clone(),
                format: PostFormat::Markdown,
                rendered_html: RenderedHtml::from_trusted("<p>the round-trip body</p>"),
                published_at: None,
                summary: None,
                audiences: vec![AudienceTarget::Public],
                idempotency_key: None,
            })
            .await
            .unwrap();
        posts
            .tag_post(post_id, &parse_tag_label("Rust"))
            .await
            .unwrap();

        let record = posts
            .get_post_by_id(post_id, &ViewerIdentity::Anonymous)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(record.slug, slug);
        assert_eq!(record.title, Some(title));
        assert_eq!(record.body, body);
        assert_eq!(record.author_username, "testuser");
        assert_eq!(record.tags.len(), 1);
        assert_eq!(record.tags[0].tag_slug, "rust");
        assert_eq!(record.tags[0].tag_display, "Rust");

        // A post with no title exercises the `None` decode path for
        // `Option<PostTitle>`.
        let untitled_id = posts
            .create_post(&CreatePostInput {
                user_id,
                title: None,
                slug: parse_slug("no-title"),
                body: "body".into(),
                format: PostFormat::Markdown,
                rendered_html: RenderedHtml::from_trusted("<p>body</p>"),
                published_at: None,
                summary: None,
                audiences: vec![AudienceTarget::Public],
                idempotency_key: None,
            })
            .await
            .unwrap();
        let untitled = posts
            .get_post_by_id(untitled_id, &ViewerIdentity::Anonymous)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(untitled.title, None);
    }

    #[apply(backends)]
    #[tokio::test]
    async fn get_post_rejects_a_malformed_slug_column(#[case] backend: Backend) {
        let env = backend.setup().await;
        let user_id = seed_user(&env.state).await;
        let posts = &*env.state.posts;
        let post_id = posts
            .create_post(&CreatePostInput {
                user_id,
                title: None,
                slug: parse_slug("good-slug"),
                body: "body".into(),
                format: PostFormat::Markdown,
                rendered_html: RenderedHtml::from_trusted("<p>body</p>"),
                published_at: None,
                summary: None,
                audiences: vec![AudienceTarget::Public],
                idempotency_key: None,
            })
            .await
            .unwrap();

        // Overwrite the `slug` column with a value `Slug::from_str` rejects (a space
        // is not a valid slug character), binding it as a raw `&str` so the bad
        // value actually lands in the column — the typed bind could not produce it.
        let sql = "UPDATE posts SET slug = $1 WHERE post_id = $2";
        match env.base.pool() {
            CloseablePool::Sqlite(pool) => {
                sqlx::query(sql)
                    .bind("not a slug")
                    .bind(i64::from(post_id))
                    .execute(pool)
                    .await
                    .unwrap();
            }
            CloseablePool::Postgres(pool) => {
                sqlx::query(sql)
                    .bind("not a slug")
                    .bind(i64::from(post_id))
                    .execute(pool)
                    .await
                    .unwrap();
            }
        }

        // The read decodes the `slug` column into `Slug` via the sqlx bridge, which
        // validates through `FromStr`; the malformed value surfaces as a
        // column-decode error rather than being silently admitted (covers the
        // bridge's `Decode` error arm).
        let err = posts
            .get_post_by_id(post_id, &ViewerIdentity::Anonymous)
            .await
            .unwrap_err();
        assert!(
            matches!(err, sqlx::Error::ColumnDecode { .. }),
            "expected a column-decode error, got: {err:?}"
        );
    }

    #[apply(backends)]
    #[tokio::test]
    async fn find_draft_by_permalink_for_user_finds_draft_and_misses(#[case] backend: Backend) {
        let env = backend.setup().await;
        let user_id = seed_user(&env.state).await;
        let posts = &*env.state.posts;
        // Seed unpublished drafts; read one back (via the per-user draft listing,
        // which is author-scoped and so needs no viewer) for its permalink parts.
        crate::test_support::seed_posts(&env.state, user_id, 3, false).await;
        let drafts = posts
            .list_drafts_by_user(user_id, None, 50, Utc::now())
            .await
            .unwrap();
        let record = drafts.first().expect("seeded draft is listed");
        let (year, month, day) = (
            record.created_at.year(),
            record.created_at.month(),
            record.created_at.day(),
        );

        let found =
            find_draft_by_permalink_for_user(posts, user_id, year, month, day, &record.slug)
                .await
                .unwrap();
        assert_eq!(found.map(|p| p.post_id), Some(record.post_id));

        // A slug the user has no draft for pages to an empty page and returns None.
        let missing = find_draft_by_permalink_for_user(
            posts,
            user_id,
            year,
            month,
            day,
            &parse_slug("no-such-draft"),
        )
        .await
        .unwrap();
        assert!(missing.is_none());
    }

    // guard:no-backend — mock store, no live database backend
    #[cfg(feature = "test-utils")]
    #[tokio::test]
    async fn find_draft_by_permalink_returns_none_after_exhausting_pages() {
        use chrono::TimeZone;
        let mut mock = crate::MockPostStorage::new();
        // Every call returns a full 50-row page of drafts whose slug never matches
        // the searched permalink, each row carrying a distinct created_at/post_id so
        // `to_post_cursor` yields an advancing (non-`None`) cursor. Since the page is
        // always non-empty and never matches, all 200 iterations of the safety bound
        // run and the loop falls through to `Ok(None)`.
        mock.expect_list_drafts_by_user()
            .returning(|_user_id, _cursor, _limit, _now| {
                let base = Utc.with_ymd_and_hms(2020, 1, 1, 0, 0, 0).unwrap();
                let username = parse_username("author");
                let slug = parse_slug("other-slug");
                let page = (0..50_i64)
                    .map(|i| PostRecord {
                        post_id: PostId::from(i),
                        user_id: UserId::from(1),
                        author_username: username.clone(),
                        title: None,
                        slug: slug.clone(),
                        body: "".into(),
                        format: PostFormat::Markdown,
                        rendered_html: RenderedHtml::from_trusted(""),
                        created_at: base + chrono::Duration::seconds(i),
                        updated_at: base,
                        published_at: None,
                        deleted_at: None,
                        summary: None,
                        tags: vec![],
                    })
                    .collect();
                Ok(page)
            });

        let searched = parse_slug("target-slug");
        let result =
            find_draft_by_permalink_for_user(&mock, UserId::from(1), 2020, 1, 1, &searched)
                .await
                .unwrap();
        assert!(result.is_none());
    }
}
