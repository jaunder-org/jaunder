//! Content storage for posts, revisions, and tagging.

use std::{fmt, str::FromStr};

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use thiserror::Error;

use common::slug::Slug;
use common::tag::Tag;
use common::username::Username;

/// The format/markup language used to author a post body.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum PostFormat {
    /// CommonMark/GitHub-flavored Markdown.
    Markdown,
    /// Emacs Org-mode format.
    Org,
}

/// Error returned when a string cannot be parsed as a [`PostFormat`].
#[derive(Debug, Error)]
#[error("post format must be \"markdown\" or \"org\"")]
pub struct InvalidPostFormat;

impl fmt::Display for PostFormat {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            PostFormat::Markdown => f.write_str("markdown"),
            PostFormat::Org => f.write_str("org"),
        }
    }
}

impl FromStr for PostFormat {
    type Err = InvalidPostFormat;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "markdown" => Ok(PostFormat::Markdown),
            "org" => Ok(PostFormat::Org),
            _ => Err(InvalidPostFormat),
        }
    }
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
pub struct PostCursor {
    /// Creation timestamp of the last item in the previous page.
    pub created_at: DateTime<Utc>,
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

    /// Fetches a post by its ID.
    async fn get_post_by_id(&self, post_id: i64) -> sqlx::Result<Option<PostRecord>>;

    /// Fetches a post by its public permalink components.
    async fn get_post_by_permalink(
        &self,
        username: &Username,
        year: i32,
        month: u32,
        day: u32,
        slug: &Slug,
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

    /// Lists published posts for a specific user, ordered by creation date.
    async fn list_published_by_user(
        &self,
        username: &Username,
        cursor: Option<&PostCursor>,
        limit: u32,
    ) -> sqlx::Result<Vec<PostRecord>>;

    /// Lists all published posts across the entire site.
    async fn list_published(
        &self,
        cursor: Option<&PostCursor>,
        limit: u32,
    ) -> sqlx::Result<Vec<PostRecord>>;

    /// Lists draft posts for a specific user.
    async fn list_drafts_by_user(
        &self,
        user_id: i64,
        cursor: Option<&PostCursor>,
        limit: u32,
    ) -> sqlx::Result<Vec<PostRecord>>;

    /// Associates a post with a tag. If the tag doesn't exist, it is created.
    async fn tag_post(&self, post_id: i64, tag_display: &str) -> Result<(), TaggingError>;

    /// Removes a tag association from a post.
    async fn untag_post(&self, post_id: i64, tag_slug: &Tag) -> Result<(), TaggingError>;

    /// Returns all tags associated with a specific post.
    async fn get_tags_for_post(&self, post_id: i64) -> sqlx::Result<Vec<PostTag>>;

    /// Lists published posts that carry a specific tag.
    async fn list_posts_by_tag(
        &self,
        tag_slug: &Tag,
        cursor: Option<&PostCursor>,
        limit: u32,
    ) -> Result<Vec<PostRecord>, ListByTagError>;

    /// Lists published posts for a specific user that carry a specific tag.
    async fn list_user_posts_by_tag(
        &self,
        user_id: i64,
        tag_slug: &Tag,
        cursor: Option<&PostCursor>,
        limit: u32,
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
    ) -> sqlx::Result<Vec<PostRecord>>;
}

#[cfg(test)]
mod tests {
    use super::*;

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
        let debug_str = format!("{:?}", err);
        assert!(debug_str.contains("PostNotFound"));

        let err2 = TaggingError::TagNotFound;
        let debug_str2 = format!("{:?}", err2);
        assert!(debug_str2.contains("TagNotFound"));

        let err3 = TaggingError::AlreadyTagged;
        let debug_str3 = format!("{:?}", err3);
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
        let debug_str = format!("{:?}", err);
        assert!(debug_str.contains("TagNotFound"));
    }

    #[test]
    fn post_format_markdown_variant() {
        let fmt = PostFormat::Markdown;
        assert_eq!(fmt, PostFormat::Markdown);
    }

    #[test]
    fn post_format_org_variant() {
        let fmt = PostFormat::Org;
        assert_eq!(fmt, PostFormat::Org);
    }

    #[test]
    fn post_format_display_round_trips() {
        assert_eq!(PostFormat::Markdown.to_string(), "markdown");
        assert_eq!(PostFormat::Org.to_string(), "org");
        assert_eq!(
            "markdown".parse::<PostFormat>().unwrap(),
            PostFormat::Markdown
        );
        assert_eq!("org".parse::<PostFormat>().unwrap(), PostFormat::Org);
    }

    #[test]
    fn post_format_rejects_invalid_value() {
        let err = "html".parse::<PostFormat>().unwrap_err();
        assert_eq!(
            err.to_string(),
            "post format must be \"markdown\" or \"org\""
        );
    }

    #[test]
    fn post_format_debug() {
        let fmt = PostFormat::Markdown;
        let debug_str = format!("{:?}", fmt);
        assert_eq!(debug_str, "Markdown");

        let fmt2 = PostFormat::Org;
        let debug_str2 = format!("{:?}", fmt2);
        assert_eq!(debug_str2, "Org");
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
            tags: vec![],
        };

        // Case 1: Body is populated. It should use the first non-empty line.
        assert_eq!(
            post.fallback_summary_label(),
            "The first non-empty line of the body is here."
        );

        // Case 2: Body is empty but title is populated.
        post.body = "".to_string();
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
            tags: vec![],
        };

        assert_eq!(post.permalink(), "/~author/2026/04/12/hello-world");
    }
}
