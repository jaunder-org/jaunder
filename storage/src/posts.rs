//! Content storage for posts, revisions, and tagging.

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use sqlx::{Database, Pool};
use thiserror::Error;

use common::slug::Slug;
use common::tag::Tag;
use common::username::Username;
use common::visibility::{AudienceTarget, ViewerIdentity};

use crate::backend::Backend;
use crate::helpers::{post_record_from_row, PostRow};

pub use common::render::{InvalidPostFormat, PostFormat};

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
    pub post_id: i64,
    /// ID of the user who owns the post.
    pub user_id: i64,
    /// Username of the author
    pub author_username: Username,
    /// Optional title.
    pub title: Option<String>,
    /// Unique slug (per user, per day).
    pub slug: Slug,
    /// Raw source body (Markdown or Org).
    pub body: String,
    /// Format of the `body`.
    pub format: PostFormat,
    /// Sanitized HTML rendering of the `body`.
    pub rendered_html: String,
    /// When the post was first created.
    pub created_at: DateTime<Utc>,
    /// When the post was last updated.
    pub updated_at: DateTime<Utc>,
    /// When the post was published (None if it is a draft).
    pub published_at: Option<DateTime<Utc>>,
    /// When the post was soft-deleted (None if active).
    pub deleted_at: Option<DateTime<Utc>>,
    /// Optional summary/excerpt of the post.
    pub summary: Option<String>,
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
            self.author_username.as_str(),
            timestamp.year(),
            timestamp.month(),
            timestamp.day(),
            self.slug.as_str()
        )
    }

    /// Generates a fallback plain-text summary from the post's body, title, or slug.
    pub fn fallback_summary_label(&self) -> String {
        self.body
            .lines()
            .map(str::trim)
            .find(|line| !line.is_empty())
            .map(|line| line.chars().take(100).collect::<String>())
            .filter(|line| !line.is_empty())
            .or_else(|| self.title.clone())
            .unwrap_or_else(|| self.slug.to_string())
    }
}

/// A post revision record returned by [`PostStorage`] queries.
///
/// Revisions are created automatically whenever a post is updated.
#[derive(Clone, Debug)]
pub struct PostRevisionRecord {
    /// Unique identifier for this revision.
    pub revision_id: i64,
    /// ID of the associated post.
    pub post_id: i64,
    /// ID of the user who made the edit.
    pub user_id: i64,
    /// Title at the time of this revision.
    pub title: Option<String>,
    /// Slug at the time of this revision.
    pub slug: Slug,
    /// Raw source body at the time of this revision.
    pub body: String,
    /// Format at the time of this revision.
    pub format: PostFormat,
    /// Sanitized HTML rendering at the time of this revision.
    pub rendered_html: String,
    /// When this revision was created.
    pub edited_at: DateTime<Utc>,
}

/// Errors that can occur when creating a post.
#[derive(Debug, Error)]
pub enum CreatePostError {
    /// A post with the same slug already exists for this user on this day.
    #[error("slug already taken for this user on this date")]
    SlugConflict,
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

/// Cursor for keyset pagination of post listings.
#[derive(Debug)]
pub struct PostCursor {
    /// Creation timestamp of the last item in the previous page.
    pub created_at: DateTime<Utc>,
    /// ID of the last item in the previous page (used for stable ordering).
    pub post_id: i64,
}

/// Cursor for keyset pagination of the editor-facing per-user collection
/// (ordered by `updated_at DESC, post_id DESC`).
#[derive(Clone, Copy, Debug)]
pub struct CollectionCursor {
    /// Update timestamp of the last item in the previous page.
    pub updated_at: DateTime<Utc>,
    /// ID of the last item in the previous page (used for stable ordering).
    pub post_id: i64,
}

/// Input for creating a new post.
#[derive(Clone)]
pub struct CreatePostInput {
    pub user_id: i64,
    pub title: Option<String>,
    pub slug: Slug,
    pub body: String,
    pub format: PostFormat,
    pub rendered_html: String,
    /// If Some, the post is created in a published state.
    pub published_at: Option<DateTime<Utc>>,
    /// Optional summary/excerpt of the post.
    pub summary: Option<String>,
    /// Audience targeting for the post. Each entry becomes a `post_audiences`
    /// row; `Private` and an empty vec produce no rows (the post is private).
    pub audiences: Vec<AudienceTarget>,
}

/// Input for updating an existing post.
#[derive(Clone)]
pub struct UpdatePostInput {
    pub title: Option<String>,
    /// The new slug. Note: Slugs are typically immutable once published.
    pub slug: Slug,
    pub body: String,
    pub format: PostFormat,
    pub rendered_html: String,
    /// If `true`, publish the post (sets `published_at` to now if not already set).
    /// If `false`, un-publish the post (clears `published_at`).
    pub publish: bool,
    /// Optional summary/excerpt of the post.
    pub summary: Option<String>,
    /// Audience targeting for the post. On update the existing
    /// `post_audiences` rows are replaced to match this vec; `Private` and an
    /// empty vec produce no rows (the post is private).
    pub audiences: Vec<AudienceTarget>,
}

/// A tag record returned by [`PostStorage`] tag queries.
#[derive(Clone, Debug)]
pub struct TagRecord {
    pub tag_id: i64,
    pub tag_slug: Tag,
}

/// A post-tag association returned by [`PostStorage`] tag queries.
#[derive(Clone, Debug)]
pub struct PostTag {
    pub post_id: i64,
    pub tag_id: i64,
    pub tag_slug: Tag,
    /// The original case-sensitive display name of the tag.
    pub tag_display: String,
}

/// The slug-level difference between a post's existing tags and a desired set
/// of display tokens, as computed by [`post_tag_diff`].
///
/// Borrows from both inputs; callers perform the actual `tag_post`/`untag_post`
/// writes with their own error mapping.
pub struct PostTagDiff<'a> {
    /// Display tokens to add (their slug is not already present on the post).
    pub to_add: Vec<&'a str>,
    /// Existing tags to remove (their slug is not in the desired set).
    pub to_remove: Vec<&'a Tag>,
}

/// Diffs a post's `existing` tags against a `desired` set of display tokens.
///
/// Tagging is keyed on slug, so a desired token is "to add" only when no
/// existing tag shares its slug, and an existing tag is "to remove" only when
/// no desired token maps to its slug. Tokens that fail to parse as [`Tag`] are
/// ignored. Re-applying an existing tag with different display casing is a
/// no-op (the existing row's casing is preserved by storage).
///
/// This is the pure core shared by the `web` and `server`/`AtomPub` front-ends;
/// each applies the result with its own error type.
#[must_use]
pub fn post_tag_diff<'a>(existing: &'a [PostTag], desired: &'a [String]) -> PostTagDiff<'a> {
    use std::collections::HashSet;
    use std::str::FromStr;

    let existing_slugs: HashSet<String> = existing.iter().map(|t| t.tag_slug.to_string()).collect();
    let desired_slugs: HashSet<String> = desired
        .iter()
        .filter_map(|d| Tag::from_str(d).ok())
        .map(|t| t.to_string())
        .collect();

    let to_add = desired
        .iter()
        .filter(|display| {
            Tag::from_str(display).is_ok_and(|slug| !existing_slugs.contains(&slug.to_string()))
        })
        .map(String::as_str)
        .collect();
    let to_remove = existing
        .iter()
        .filter(|tag| !desired_slugs.contains(&tag.tag_slug.to_string()))
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

/// Async operations on the `posts` and `post_revisions` tables.
///
/// This trait manages the lifecycle of posts, including versioned edits,
/// draft/publish status, soft-deletion, and tagging.
#[async_trait]
pub trait PostStorage: Send + Sync {
    /// Creates a new post.
    async fn create_post(&self, input: &CreatePostInput) -> Result<i64, CreatePostError>;

    /// Fetches a post by its ID, applying the viewer-resolution filter: the post
    /// is returned only if `viewer` is the author or a targeted audience admits
    /// them. See ADR-0020.
    async fn get_post_by_id(
        &self,
        post_id: i64,
        viewer: &ViewerIdentity,
    ) -> sqlx::Result<Option<PostRecord>>;

    /// Fetches a post by its public permalink components, applying the
    /// viewer-resolution filter. See ADR-0020.
    async fn get_post_by_permalink(
        &self,
        username: &Username,
        year: i32,
        month: u32,
        day: u32,
        slug: &Slug,
        viewer: &ViewerIdentity,
    ) -> sqlx::Result<Option<PostRecord>>;

    /// Updates a post and creates a new revision.
    ///
    /// # Errors
    ///
    /// Returns [`UpdatePostError::NotFound`] if the post doesn't exist, or
    /// [`UpdatePostError::Unauthorized`] if the editor isn't the owner.
    async fn update_post(
        &self,
        post_id: i64,
        editor_user_id: i64,
        input: &UpdatePostInput,
    ) -> Result<PostRecord, UpdatePostError>;

    /// Marks a post as deleted without removing it from the database.
    async fn soft_delete_post(&self, post_id: i64) -> sqlx::Result<()>;

    /// Reverts a published post to draft status.
    async fn unpublish_post(&self, post_id: i64) -> sqlx::Result<()>;

    /// Lists published posts for a specific user, ordered by creation date,
    /// applying the viewer-resolution filter. See ADR-0020.
    async fn list_published_by_user(
        &self,
        username: &Username,
        cursor: Option<&PostCursor>,
        limit: u32,
        viewer: &ViewerIdentity,
    ) -> sqlx::Result<Vec<PostRecord>>;

    /// Lists all published posts across the entire site, applying the
    /// viewer-resolution filter. See ADR-0020.
    async fn list_published(
        &self,
        cursor: Option<&PostCursor>,
        limit: u32,
        viewer: &ViewerIdentity,
    ) -> sqlx::Result<Vec<PostRecord>>;

    /// Lists draft posts for a specific user.
    async fn list_drafts_by_user(
        &self,
        user_id: i64,
        cursor: Option<&PostCursor>,
        limit: u32,
    ) -> sqlx::Result<Vec<PostRecord>>;

    /// Lists all of a user's non-soft-deleted posts (drafts + published)
    /// ordered by `updated_at DESC, post_id DESC` for the `AtomPub` Collection
    /// surface. Tags are hydrated.
    async fn list_collection_by_user(
        &self,
        user_id: i64,
        cursor: Option<&CollectionCursor>,
        limit: u32,
    ) -> sqlx::Result<Vec<PostRecord>>;

    /// Associates a post with a tag. If the tag doesn't exist, it is created.
    async fn tag_post(&self, post_id: i64, tag_display: &str) -> Result<(), TaggingError>;

    /// Removes a tag association from a post.
    async fn untag_post(&self, post_id: i64, tag_slug: &Tag) -> Result<(), TaggingError>;

    /// Returns all tags associated with a specific post.
    async fn get_tags_for_post(&self, post_id: i64) -> sqlx::Result<Vec<PostTag>>;

    /// Lists published posts that carry a specific tag, applying the
    /// viewer-resolution filter. See ADR-0020.
    async fn list_posts_by_tag(
        &self,
        tag_slug: &Tag,
        cursor: Option<&PostCursor>,
        limit: u32,
        viewer: &ViewerIdentity,
    ) -> Result<Vec<PostRecord>, ListByTagError>;

    /// Lists published posts for a specific user that carry a specific tag,
    /// applying the viewer-resolution filter. See ADR-0020.
    async fn list_user_posts_by_tag(
        &self,
        user_id: i64,
        tag_slug: &Tag,
        cursor: Option<&PostCursor>,
        limit: u32,
        viewer: &ViewerIdentity,
    ) -> Result<Vec<PostRecord>, ListByTagError>;

    /// Returns tag records whose slug begins with `prefix` (case-insensitive
    /// on the slug). An empty / `None` prefix returns all tags, alphabetically,
    /// up to `limit`.
    async fn list_tags(&self, prefix: Option<&str>, limit: u32) -> sqlx::Result<Vec<TagRecord>>;

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
        post_id: i64,
        editor_user_id: i64,
        input: &UpdatePostInput,
    ) -> Result<PostRecord, UpdatePostError>;

    /// Associate `post_id` with the tag parsed from `tag_display`, creating the
    /// tag if it does not yet exist.
    async fn tag_post(
        pool: &Pool<Self>,
        post_id: i64,
        tag_display: &str,
    ) -> Result<(), TaggingError>;

    /// Remove a tag association; returns [`TaggingError::TagNotFound`] when no
    /// row was deleted.
    async fn untag_post(
        pool: &Pool<Self>,
        post_id: i64,
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
    (i64, i64, String, String): for<'r> sqlx::FromRow<'r, DB::Row>,
    (i64, String): for<'r> sqlx::FromRow<'r, DB::Row>,
    for<'q> i64: sqlx::Encode<'q, DB> + sqlx::Type<DB>,
    for<'q> &'q str: sqlx::Encode<'q, DB> + sqlx::Type<DB>,
    for<'q> Option<String>: sqlx::Encode<'q, DB> + sqlx::Type<DB>,
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
    async fn create_post(&self, input: &CreatePostInput) -> Result<i64, CreatePostError> {
        let now = Utc::now();
        let format = input.format.to_string();

        let mut tx = self.pool.begin().await?;

        let result = sqlx::query_scalar::<_, i64>(
            "INSERT INTO posts (user_id, title, slug, body, format, rendered_html, created_at, updated_at, published_at, summary)
             VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10)
             RETURNING post_id",
        )
        .bind(input.user_id)
        .bind(input.title.clone())
        .bind(input.slug.as_str())
        .bind(input.body.as_str())
        .bind(format.as_str())
        .bind(input.rendered_html.as_str())
        .bind(now)
        .bind(now)
        .bind(input.published_at)
        .bind(input.summary.clone())
        .fetch_one(&mut *tx)
        .await;

        let post_id = match result {
            Ok(id) => id,
            Err(sqlx::Error::Database(e)) if e.is_unique_violation() => {
                tx.rollback().await.ok();
                return Err(CreatePostError::SlugConflict);
            }
            Err(e) => {
                tx.rollback().await.ok();
                return Err(CreatePostError::Internal(e));
            }
        };

        replace_post_audiences::<DB>(&mut tx, post_id, &input.audiences).await?;
        tx.commit().await?;
        // (`&mut tx` derefs to `&mut DB::Connection` for the helper.)
        Ok(post_id)
    }

    #[tracing::instrument(
        name = "storage.posts.get_by_id",
        skip(self),
        fields(db.system = DB::DB_SYSTEM)
    )]
    async fn get_post_by_id(
        &self,
        post_id: i64,
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
        let query = sqlx::query_as::<_, PostRow>(&sql).bind(post_id);
        let row = binds.bind_onto(query).fetch_optional(&self.pool).await?;
        Ok(row.map(post_record_from_row).transpose()?)
    }

    #[tracing::instrument(
        name = "storage.posts.get_by_permalink",
        skip(self),
        fields(db.system = DB::DB_SYSTEM)
    )]
    async fn get_post_by_permalink(
        &self,
        username: &Username,
        year: i32,
        month: u32,
        day: u32,
        slug: &Slug,
        viewer: &ViewerIdentity,
    ) -> sqlx::Result<Option<PostRecord>> {
        let date_str = format!("{year:04}-{month:02}-{day:02}");
        let (resolution, binds, _) = resolution_where(viewer, 4);
        let sql = format!(
            "SELECT p.post_id, p.user_id, u.username, p.title, p.slug, p.body, p.format, p.rendered_html,
                    p.created_at, p.updated_at, p.published_at, p.deleted_at, p.summary,
                    {tags} AS tags
             FROM posts p
             JOIN users u ON p.user_id = u.user_id
             WHERE u.username = $1
               AND p.slug = $2
               AND p.published_at IS NOT NULL
               AND p.deleted_at IS NULL
               AND {date_clause}
               AND {resolution}",
            tags = DB::TAGS_SUBQUERY,
            date_clause = DB::PERMALINK_DATE_CLAUSE,
        );
        let query = sqlx::query_as::<_, PostRow>(&sql)
            .bind(username.as_str())
            .bind(slug.as_str())
            .bind(date_str.as_str());
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
        post_id: i64,
        editor_user_id: i64,
        input: &UpdatePostInput,
    ) -> Result<PostRecord, UpdatePostError> {
        DB::update_post(&self.pool, post_id, editor_user_id, input).await
    }

    #[tracing::instrument(
        name = "storage.posts.soft_delete",
        skip(self),
        fields(db.system = DB::DB_SYSTEM)
    )]
    async fn soft_delete_post(&self, post_id: i64) -> sqlx::Result<()> {
        let now = Utc::now();
        sqlx::query("UPDATE posts SET deleted_at = $1 WHERE post_id = $2")
            .bind(now)
            .bind(post_id)
            .execute(&self.pool)
            .await?;
        Ok(())
    }

    #[tracing::instrument(
        name = "storage.posts.unpublish",
        skip(self),
        fields(db.system = DB::DB_SYSTEM)
    )]
    async fn unpublish_post(&self, post_id: i64) -> sqlx::Result<()> {
        sqlx::query("UPDATE posts SET published_at = NULL WHERE post_id = $1")
            .bind(post_id)
            .execute(&self.pool)
            .await?;
        Ok(())
    }

    #[tracing::instrument(
        name = "storage.posts.list_published_by_user",
        skip(self),
        fields(db.system = DB::DB_SYSTEM)
    )]
    async fn list_published_by_user(
        &self,
        username: &Username,
        cursor: Option<&PostCursor>,
        limit: u32,
        viewer: &ViewerIdentity,
    ) -> sqlx::Result<Vec<PostRecord>> {
        let tags = DB::TAGS_SUBQUERY;
        let rows = if let Some(cursor) = cursor {
            // Binds: $1 username, $2/$3 cursor, $4 post_id, $5..$9 resolution,
            // $10 limit.
            let (resolution, binds, limit_idx) = resolution_where(viewer, 5);
            let sql = format!(
                "SELECT p.post_id, p.user_id, u.username, p.title, p.slug, p.body, p.format, p.rendered_html,
                        p.created_at, p.updated_at, p.published_at, p.deleted_at, p.summary,
                        {tags} AS tags
                 FROM posts p
                 JOIN users u ON p.user_id = u.user_id
                 WHERE u.username = $1
                   AND p.published_at IS NOT NULL
                   AND p.deleted_at IS NULL
                   AND (p.created_at < $2 OR (p.created_at = $3 AND p.post_id < $4))
                   AND {resolution}
                 ORDER BY p.created_at DESC, p.post_id DESC
                 LIMIT ${limit_idx}"
            );
            let query = sqlx::query_as::<_, PostRow>(&sql)
                .bind(username.as_str())
                .bind(cursor.created_at)
                .bind(cursor.created_at)
                .bind(cursor.post_id);
            binds
                .bind_onto(query)
                .bind(i64::from(limit))
                .fetch_all(&self.pool)
                .await?
        } else {
            // Binds: $1 username, $2..$6 resolution, $7 limit.
            let (resolution, binds, limit_idx) = resolution_where(viewer, 2);
            let sql = format!(
                "SELECT p.post_id, p.user_id, u.username, p.title, p.slug, p.body, p.format, p.rendered_html,
                        p.created_at, p.updated_at, p.published_at, p.deleted_at, p.summary,
                        {tags} AS tags
                 FROM posts p
                 JOIN users u ON p.user_id = u.user_id
                 WHERE u.username = $1
                   AND p.published_at IS NOT NULL
                   AND p.deleted_at IS NULL
                   AND {resolution}
                 ORDER BY p.created_at DESC, p.post_id DESC
                 LIMIT ${limit_idx}"
            );
            let query = sqlx::query_as::<_, PostRow>(&sql).bind(username.as_str());
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
    async fn list_published(
        &self,
        cursor: Option<&PostCursor>,
        limit: u32,
        viewer: &ViewerIdentity,
    ) -> sqlx::Result<Vec<PostRecord>> {
        let tags = DB::TAGS_SUBQUERY;
        let rows = if let Some(cursor) = cursor {
            // Binds: $1/$2 cursor, $3 post_id, $4..$8 resolution, $9 limit.
            let (resolution, binds, limit_idx) = resolution_where(viewer, 4);
            let sql = format!(
                "SELECT p.post_id, p.user_id, u.username, p.title, p.slug, p.body, p.format, p.rendered_html,
                        p.created_at, p.updated_at, p.published_at, p.deleted_at, p.summary,
                        {tags} AS tags
                 FROM posts p
                 JOIN users u ON p.user_id = u.user_id
                 WHERE p.published_at IS NOT NULL
                   AND p.deleted_at IS NULL
                   AND (p.created_at < $1 OR (p.created_at = $2 AND p.post_id < $3))
                   AND {resolution}
                 ORDER BY p.created_at DESC, p.post_id DESC
                 LIMIT ${limit_idx}"
            );
            let query = sqlx::query_as::<_, PostRow>(&sql)
                .bind(cursor.created_at)
                .bind(cursor.created_at)
                .bind(cursor.post_id);
            binds
                .bind_onto(query)
                .bind(i64::from(limit))
                .fetch_all(&self.pool)
                .await?
        } else {
            // Binds: $1..$5 resolution, $6 limit.
            let (resolution, binds, limit_idx) = resolution_where(viewer, 1);
            let sql = format!(
                "SELECT p.post_id, p.user_id, u.username, p.title, p.slug, p.body, p.format, p.rendered_html,
                        p.created_at, p.updated_at, p.published_at, p.deleted_at, p.summary,
                        {tags} AS tags
                 FROM posts p
                 JOIN users u ON p.user_id = u.user_id
                 WHERE p.published_at IS NOT NULL
                   AND p.deleted_at IS NULL
                   AND {resolution}
                 ORDER BY p.created_at DESC, p.post_id DESC
                 LIMIT ${limit_idx}"
            );
            let query = sqlx::query_as::<_, PostRow>(&sql);
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
    async fn list_drafts_by_user(
        &self,
        user_id: i64,
        cursor: Option<&PostCursor>,
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
                   AND p.published_at IS NULL
                   AND p.deleted_at IS NULL
                   AND (p.created_at < $2 OR (p.created_at = $3 AND p.post_id < $4))
                 ORDER BY p.created_at DESC, p.post_id DESC
                 LIMIT $5"
            );
            sqlx::query_as::<_, PostRow>(&sql)
                .bind(user_id)
                .bind(cursor.created_at)
                .bind(cursor.created_at)
                .bind(cursor.post_id)
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
                   AND p.published_at IS NULL
                   AND p.deleted_at IS NULL
                 ORDER BY p.created_at DESC, p.post_id DESC
                 LIMIT $2"
            );
            sqlx::query_as::<_, PostRow>(&sql)
                .bind(user_id)
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
    async fn list_collection_by_user(
        &self,
        user_id: i64,
        cursor: Option<&CollectionCursor>,
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
                .bind(user_id)
                .bind(cursor.updated_at)
                .bind(cursor.post_id)
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
                .bind(user_id)
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
    async fn tag_post(&self, post_id: i64, tag_display: &str) -> Result<(), TaggingError> {
        DB::tag_post(&self.pool, post_id, tag_display).await
    }

    #[tracing::instrument(
        name = "storage.posts.untag",
        skip(self),
        fields(db.system = DB::DB_SYSTEM)
    )]
    async fn untag_post(&self, post_id: i64, tag_slug: &Tag) -> Result<(), TaggingError> {
        DB::untag_post(&self.pool, post_id, tag_slug).await
    }

    #[tracing::instrument(
        name = "storage.posts.get_tags_for_post",
        skip(self),
        fields(db.system = DB::DB_SYSTEM)
    )]
    async fn get_tags_for_post(&self, post_id: i64) -> sqlx::Result<Vec<PostTag>> {
        let rows = sqlx::query_as::<_, (i64, i64, String, String)>(
            "SELECT pt.post_id, pt.tag_id, t.tag_slug, pt.tag_display
             FROM post_tags pt
             JOIN tags t ON pt.tag_id = t.tag_id
             WHERE pt.post_id = $1
             ORDER BY t.tag_slug",
        )
        .bind(post_id)
        .fetch_all(&self.pool)
        .await?;

        rows.into_iter()
            .map(|(post_id, tag_id, tag_slug_str, tag_display)| {
                let tag_slug: Tag = tag_slug_str
                    .parse()
                    .map_err(|_| sqlx::Error::Decode("invalid tag format".into()))?;
                Ok(PostTag {
                    post_id,
                    tag_id,
                    tag_slug,
                    tag_display,
                })
            })
            .collect()
    }

    #[tracing::instrument(
        name = "storage.posts.list_posts_by_tag",
        skip(self),
        fields(db.system = DB::DB_SYSTEM)
    )]
    async fn list_posts_by_tag(
        &self,
        tag_slug: &Tag,
        cursor: Option<&PostCursor>,
        limit: u32,
        viewer: &ViewerIdentity,
    ) -> Result<Vec<PostRecord>, ListByTagError> {
        let tag_exists: bool =
            sqlx::query_scalar("SELECT COUNT(*) > 0 FROM tags WHERE tag_slug = $1")
                .bind(tag_slug.as_str())
                .fetch_one(&self.pool)
                .await?;

        if !tag_exists {
            return Err(ListByTagError::TagNotFound);
        }

        let tags = DB::TAGS_SUBQUERY;
        let rows = if let Some(cursor) = cursor {
            // Binds: $1 tag, $2/$3 cursor, $4 post_id, $5..$9 resolution, $10 limit.
            let (resolution, binds, limit_idx) = resolution_where(viewer, 5);
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
                   AND p.deleted_at IS NULL
                   AND (p.created_at < $2 OR (p.created_at = $3 AND p.post_id < $4))
                   AND {resolution}
                 ORDER BY p.created_at DESC, p.post_id DESC
                 LIMIT ${limit_idx}"
            );
            let query = sqlx::query_as::<_, PostRow>(&sql)
                .bind(tag_slug.as_str())
                .bind(cursor.created_at)
                .bind(cursor.created_at)
                .bind(cursor.post_id);
            binds
                .bind_onto(query)
                .bind(i64::from(limit))
                .fetch_all(&self.pool)
                .await?
        } else {
            // Binds: $1 tag, $2..$6 resolution, $7 limit.
            let (resolution, binds, limit_idx) = resolution_where(viewer, 2);
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
                   AND p.deleted_at IS NULL
                   AND {resolution}
                 ORDER BY p.created_at DESC, p.post_id DESC
                 LIMIT ${limit_idx}"
            );
            let query = sqlx::query_as::<_, PostRow>(&sql).bind(tag_slug.as_str());
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
    async fn list_user_posts_by_tag(
        &self,
        user_id: i64,
        tag_slug: &Tag,
        cursor: Option<&PostCursor>,
        limit: u32,
        viewer: &ViewerIdentity,
    ) -> Result<Vec<PostRecord>, ListByTagError> {
        let tag_exists: bool =
            sqlx::query_scalar("SELECT COUNT(*) > 0 FROM tags WHERE tag_slug = $1")
                .bind(tag_slug.as_str())
                .fetch_one(&self.pool)
                .await?;

        if !tag_exists {
            return Err(ListByTagError::TagNotFound);
        }

        let tags = DB::TAGS_SUBQUERY;
        let rows = if let Some(cursor) = cursor {
            // Binds: $1 user_id, $2 tag, $3/$4 cursor, $5 post_id,
            // $6..$10 resolution, $11 limit.
            let (resolution, binds, limit_idx) = resolution_where(viewer, 6);
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
                   AND p.deleted_at IS NULL
                   AND (p.created_at < $3 OR (p.created_at = $4 AND p.post_id < $5))
                   AND {resolution}
                 ORDER BY p.created_at DESC, p.post_id DESC
                 LIMIT ${limit_idx}"
            );
            let query = sqlx::query_as::<_, PostRow>(&sql)
                .bind(user_id)
                .bind(tag_slug.as_str())
                .bind(cursor.created_at)
                .bind(cursor.created_at)
                .bind(cursor.post_id);
            binds
                .bind_onto(query)
                .bind(i64::from(limit))
                .fetch_all(&self.pool)
                .await?
        } else {
            // Binds: $1 user_id, $2 tag, $3..$7 resolution, $8 limit.
            let (resolution, binds, limit_idx) = resolution_where(viewer, 3);
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
                   AND p.deleted_at IS NULL
                   AND {resolution}
                 ORDER BY p.created_at DESC, p.post_id DESC
                 LIMIT ${limit_idx}"
            );
            let query = sqlx::query_as::<_, PostRow>(&sql)
                .bind(user_id)
                .bind(tag_slug.as_str());
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
    async fn list_tags(&self, prefix: Option<&str>, limit: u32) -> sqlx::Result<Vec<TagRecord>> {
        let normalized = prefix
            .map(str::trim)
            .filter(|p| !p.is_empty())
            .map(str::to_ascii_lowercase);
        let pattern = normalized.as_deref().map(|p| format!("{p}%"));
        let limit_i64 = i64::from(limit);

        let rows = match pattern {
            Some(ref like) => {
                sqlx::query_as::<_, (i64, String)>(
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
                sqlx::query_as::<_, (i64, String)>(
                    "SELECT tag_id, tag_slug FROM tags
                     ORDER BY tag_slug
                     LIMIT $1",
                )
                .bind(limit_i64)
                .fetch_all(&self.pool)
                .await?
            }
        };

        rows.into_iter()
            .map(|(tag_id, tag_slug_str)| {
                let tag_slug: Tag = tag_slug_str
                    .parse()
                    .map_err(|_| sqlx::Error::Decode("invalid tag format".into()))?;
                Ok(TagRecord { tag_id, tag_slug })
            })
            .collect()
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
        let min_items = i64::from(window.min_items);
        let rows = list_published_in_window_rows::<DB>(
            &self.pool, surface, now, cutoff, min_items, viewer,
        )
        .await?;
        rows.into_iter().map(post_record_from_row).collect()
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
            (author_id, *channel_id, subscriber_ref.clone())
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
        AudienceTarget::Named(id) => Some((TargetKind::Named.as_str(), Some(*id))),
        AudienceTarget::Private => None,
    }
}

/// Replaces a post's `post_audiences` rows to exactly match `audiences`.
///
/// Deletes every existing row for `post_id`, then inserts one row per targeting
/// entry (`Public`/`Subscribers` carry a NULL `audience_id`; `Named(id)` carries
/// the id; `Private` and an empty vec leave the post with no rows). Runs on the
/// caller's executor so it shares the create/update transaction. See ADR-0020.
pub(crate) async fn replace_post_audiences<DB>(
    conn: &mut DB::Connection,
    post_id: i64,
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
        .bind(post_id)
        .execute(&mut *conn)
        .await?;
    for target in audiences {
        if let Some((kind_name, audience_id)) = audience_target_row(target) {
            sqlx::query(DB::INSERT_POST_AUDIENCE)
                .bind(post_id)
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
// The body is dominated by four near-identical SQL string literals; splitting
// it would only duplicate the generic `where`-clause four times.
#[allow(clippy::too_many_lines)]
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
    for<'c> &'c Pool<DB>: sqlx::Executor<'c, Database = DB>,
    for<'q> DB::Arguments<'q>: sqlx::IntoArguments<'q, DB>,
{
    use common::feed::FeedSurface;
    let tags = DB::TAGS_SUBQUERY;
    match surface {
        FeedSurface::Site => {
            // Binds: $1 now, $2 min_items, $3 cutoff, $4..$8 resolution.
            let (resolution, binds, _) = resolution_where(viewer, 4);
            let sql = format!(
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
            );
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
            let sql = format!(
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
            );
            let query = sqlx::query_as::<_, PostRow>(&sql)
                .bind(now)
                .bind(username.as_str())
                .bind(min_items)
                .bind(cutoff);
            binds.bind_onto(query).fetch_all(pool).await
        }
        FeedSurface::SiteTag { tag } => {
            // Binds: $1 now, $2 tag, $3 min_items, $4 cutoff, $5..$9 resolution.
            let (resolution, binds, _) = resolution_where(viewer, 5);
            let sql = format!(
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
            );
            let query = sqlx::query_as::<_, PostRow>(&sql)
                .bind(now)
                .bind(tag.as_str())
                .bind(min_items)
                .bind(cutoff);
            binds.bind_onto(query).fetch_all(pool).await
        }
        FeedSurface::UserTag { username, tag } => {
            // Binds: $1 now, $2 username, $3 tag, $4 min_items, $5 cutoff,
            // $6..$10 resolution.
            let (resolution, binds, _) = resolution_where(viewer, 6);
            let sql = format!(
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
            );
            let query = sqlx::query_as::<_, PostRow>(&sql)
                .bind(now)
                .bind(username.as_str())
                .bind(tag.as_str())
                .bind(min_items)
                .bind(cutoff);
            binds.bind_onto(query).fetch_all(pool).await
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn post_tag(slug: &str, display: &str) -> PostTag {
        PostTag {
            post_id: 1,
            tag_id: 0,
            tag_slug: slug.parse::<Tag>().expect("valid tag slug"),
            tag_display: display.to_string(),
        }
    }

    #[test]
    fn post_tag_diff_adds_removes_keeps_and_skips_invalid() {
        let existing = vec![post_tag("rust", "Rust"), post_tag("leptos", "Leptos")];
        let desired = vec![
            // Same slug as an existing tag (different casing): kept, not re-added.
            "Rust".to_string(),
            // New slug: added.
            "wasm".to_string(),
            // Fails to parse as a Tag (underscore): ignored entirely.
            "has_underscore".to_string(),
        ];

        let diff = post_tag_diff(&existing, &desired);

        assert_eq!(diff.to_add, vec!["wasm"]);
        let removed: Vec<&str> = diff.to_remove.iter().map(|t| t.as_str()).collect();
        assert_eq!(removed, vec!["leptos"]);
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
            post_id: 1,
            user_id: 1,
            author_username: "author".parse().unwrap(),
            title: Some("My Title".to_string()),
            slug: "my-slug".parse().unwrap(),
            body: "\n\n   The first non-empty line of the body is here. \n\n Another line."
                .to_string(),
            format: PostFormat::Markdown,
            rendered_html: "<p>The first non-empty line of the body is here.</p>".to_string(),
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
        post.body = String::new();
        assert_eq!(post.fallback_summary_label(), "My Title");

        // Case 3: Body and title are empty. It should use the slug.
        post.title = None;
        assert_eq!(post.fallback_summary_label(), "my-slug");
    }

    #[test]
    fn permalink_formats_username_date_and_slug() {
        use chrono::TimeZone;
        let post = PostRecord {
            post_id: 1,
            user_id: 1,
            author_username: "author".parse().unwrap(),
            title: Some("My Title".to_string()),
            slug: "hello-world".parse().unwrap(),
            body: "My body".to_string(),
            format: PostFormat::Markdown,
            rendered_html: "<p>My body</p>".to_string(),
            created_at: Utc.with_ymd_and_hms(2026, 4, 12, 8, 30, 0).unwrap(),
            updated_at: Utc.with_ymd_and_hms(2026, 4, 12, 8, 30, 0).unwrap(),
            published_at: Some(Utc.with_ymd_and_hms(2026, 4, 12, 8, 30, 0).unwrap()),
            deleted_at: None,
            summary: None,
            tags: vec![],
        };

        assert_eq!(post.permalink(), "/~author/2026/04/12/hello-world");
    }

    #[tokio::test]
    async fn create_post_persists_summary() {
        use crate::sqlite::SqlitePostStorage;
        use chrono::Utc;

        let pool = sqlx::SqlitePool::connect("sqlite::memory:").await.unwrap();
        sqlx::migrate!("../storage/migrations/sqlite")
            .run(&pool)
            .await
            .unwrap();

        // Create a test user
        sqlx::query(
            "INSERT INTO users (username, password_hash, created_at, is_operator) VALUES (?, ?, ?, ?)",
        )
        .bind("testuser")
        .bind("hash")
        .bind(Utc::now())
        .bind(false)
        .execute(&pool)
        .await
        .unwrap();

        let posts = SqlitePostStorage::new(pool);
        let input = CreatePostInput {
            user_id: 1,
            title: Some("Test Title".into()),
            slug: "test-slug".parse().unwrap(),
            body: "Test body".into(),
            format: PostFormat::Markdown,
            rendered_html: "<p>Test body</p>".into(),
            published_at: None,
            summary: Some("the summary".into()),
            audiences: vec![AudienceTarget::Public],
        };

        let post_id = posts.create_post(&input).await.unwrap();
        let post = posts
            .get_post_by_id(post_id, &ViewerIdentity::Anonymous)
            .await
            .unwrap()
            .unwrap();

        assert_eq!(post.summary, Some("the summary".into()));
    }
}
