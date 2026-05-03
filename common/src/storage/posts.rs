use std::{fmt, str::FromStr};

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use thiserror::Error;

use crate::slug::Slug;
use crate::tag::Tag;
use crate::username::Username;

/// The format/markup language used to author a post body.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum PostFormat {
    Markdown,
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
#[derive(Clone, Debug)]
pub struct PostRecord {
    pub post_id: i64,
    pub user_id: i64,
    pub title: Option<String>,
    pub slug: Slug,
    pub body: String,
    pub format: PostFormat,
    pub rendered_html: String,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    pub published_at: Option<DateTime<Utc>>,
    pub deleted_at: Option<DateTime<Utc>>,
}

/// A post revision record returned by [`PostStorage`] queries.
#[derive(Clone, Debug)]
pub struct PostRevisionRecord {
    pub revision_id: i64,
    pub post_id: i64,
    pub user_id: i64,
    pub title: Option<String>,
    pub slug: Slug,
    pub body: String,
    pub format: PostFormat,
    pub rendered_html: String,
    pub edited_at: DateTime<Utc>,
}

/// Errors that can occur when creating a post.
#[derive(Debug, Error)]
pub enum CreatePostError {
    #[error("slug already taken for this user on this date")]
    SlugConflict,
    #[error(transparent)]
    Internal(#[from] sqlx::Error),
}

/// Errors that can occur when updating a post.
#[derive(Debug, Error)]
pub enum UpdatePostError {
    #[error("post not found")]
    NotFound,
    #[error("not authorized")]
    Unauthorized,
    #[error(transparent)]
    Internal(#[from] sqlx::Error),
}

/// Cursor for keyset pagination of post listings.
pub struct PostCursor {
    pub created_at: DateTime<Utc>,
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
    pub published_at: Option<DateTime<Utc>>,
}

/// Input for updating an existing post.
#[derive(Clone)]
pub struct UpdatePostInput {
    pub title: Option<String>,
    /// Ignored if the post is already published.
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
    pub tag_display: String,
}

/// Errors that can occur when tagging a post.
#[derive(Debug, Error)]
pub enum TaggingError {
    #[error("post not found")]
    PostNotFound,
    #[error("tag not found")]
    TagNotFound,
    #[error("post is already tagged with this tag")]
    AlreadyTagged,
    #[error(transparent)]
    Internal(#[from] sqlx::Error),
}

/// Errors that can occur when listing posts by tag.
#[derive(Debug, Error)]
pub enum ListByTagError {
    #[error("tag not found")]
    TagNotFound,
    #[error(transparent)]
    Internal(#[from] sqlx::Error),
}

/// Async operations on the `posts` and `post_revisions` tables.
#[async_trait]
pub trait PostStorage: Send + Sync {
    async fn create_post(&self, input: &CreatePostInput) -> Result<i64, CreatePostError>;

    async fn get_post_by_id(&self, post_id: i64) -> sqlx::Result<Option<PostRecord>>;

    async fn get_post_by_permalink(
        &self,
        username: &Username,
        year: i32,
        month: u32,
        day: u32,
        slug: &Slug,
    ) -> sqlx::Result<Option<PostRecord>>;

    async fn update_post(
        &self,
        post_id: i64,
        editor_user_id: i64,
        input: &UpdatePostInput,
    ) -> Result<PostRecord, UpdatePostError>;

    async fn soft_delete_post(&self, post_id: i64) -> sqlx::Result<()>;

    /// Clears `published_at`, reverting a published post to draft status.
    async fn unpublish_post(&self, post_id: i64) -> sqlx::Result<()>;

    async fn list_published_by_user(
        &self,
        username: &Username,
        cursor: Option<&PostCursor>,
        limit: u32,
    ) -> sqlx::Result<Vec<PostRecord>>;

    async fn list_published(
        &self,
        cursor: Option<&PostCursor>,
        limit: u32,
    ) -> sqlx::Result<Vec<PostRecord>>;

    async fn list_drafts_by_user(
        &self,
        user_id: i64,
        cursor: Option<&PostCursor>,
        limit: u32,
    ) -> sqlx::Result<Vec<PostRecord>>;

    /// Associates a post with a tag. If the tag doesn't exist, creates it.
    async fn tag_post(&self, post_id: i64, tag_display: &str) -> Result<(), TaggingError>;

    /// Removes a tag association from a post.
    async fn untag_post(&self, post_id: i64, tag_slug: &Tag) -> Result<(), TaggingError>;

    /// Returns all tags on a post.
    async fn get_tags_for_post(&self, post_id: i64) -> sqlx::Result<Vec<PostTag>>;

    /// Returns published, non-deleted posts with a tag.
    async fn list_posts_by_tag(
        &self,
        tag_slug: &Tag,
        cursor: Option<&PostCursor>,
        limit: u32,
    ) -> Result<Vec<PostRecord>, ListByTagError>;

    /// Returns published, non-deleted posts by user with a tag.
    async fn list_user_posts_by_tag(
        &self,
        user_id: i64,
        tag_slug: &Tag,
        cursor: Option<&PostCursor>,
        limit: u32,
    ) -> Result<Vec<PostRecord>, ListByTagError>;
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
}
