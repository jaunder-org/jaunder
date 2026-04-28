use std::{fmt, str::FromStr, sync::Arc};

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use thiserror::Error;

use email_address::EmailAddress;

use crate::mailer::MailSender;
use crate::password::Password;
use crate::slug::Slug;
use crate::tag::Tag;
use crate::username::Username;

// ---------------------------------------------------------------------------
// SiteConfig
// ---------------------------------------------------------------------------

/// Async operations on the `site_config` key-value table.
#[async_trait]
pub trait SiteConfigStorage: Send + Sync {
    /// Returns the value for `key`, or `None` if the key is not set.
    async fn get(&self, key: &str) -> sqlx::Result<Option<String>>;

    /// Inserts or replaces the value for `key`.
    async fn set(&self, key: &str, value: &str) -> sqlx::Result<()>;
}

// ---------------------------------------------------------------------------
// Users
// ---------------------------------------------------------------------------

/// A user account record returned by [`UserStorage`] queries.
///
/// Does not expose `password_hash`; that field is only accessed inside the
/// storage implementation.
#[derive(Clone, Debug)]
pub struct UserRecord {
    pub user_id: i64,
    pub username: Username,
    pub display_name: Option<String>,
    pub bio: Option<String>,
    pub created_at: DateTime<Utc>,
    pub last_authenticated_at: Option<DateTime<Utc>>,
    pub email: Option<EmailAddress>,
    pub email_verified: bool,
}

/// Errors that can occur when creating a user.
#[derive(Debug, Error)]
pub enum CreateUserError {
    #[error("username is already taken")]
    UsernameTaken,
    #[error(transparent)]
    Internal(#[from] sqlx::Error),
}

/// Errors that can occur when authenticating a user by password.
#[derive(Debug, Error)]
pub enum UserAuthError {
    #[error("invalid credentials")]
    InvalidCredentials,
    #[error("internal error: {0}")]
    Internal(String),
}

/// Fields to update on a user's profile.
///
/// Each field is `Option<&str>`: `None` clears the field, `Some(v)` sets it.
pub struct ProfileUpdate<'a> {
    pub display_name: Option<&'a str>,
    pub bio: Option<&'a str>,
}

/// Async operations on the `users` table.
#[async_trait]
pub trait UserStorage: Send + Sync {
    async fn create_user(
        &self,
        username: &Username,
        password: &Password,
        display_name: Option<&str>,
    ) -> Result<i64, CreateUserError>;

    async fn authenticate(
        &self,
        username: &Username,
        password: &Password,
    ) -> Result<UserRecord, UserAuthError>;

    async fn get_user(&self, user_id: i64) -> sqlx::Result<Option<UserRecord>>;

    async fn get_user_by_username(&self, username: &Username) -> sqlx::Result<Option<UserRecord>>;

    async fn update_profile(&self, user_id: i64, update: &ProfileUpdate<'_>) -> sqlx::Result<()>;

    async fn set_email(
        &self,
        user_id: i64,
        email: Option<&EmailAddress>,
        verified: bool,
    ) -> sqlx::Result<()>;

    /// Replaces the stored password hash for `user_id` with a hash of `new_password`.
    /// Hashing is performed inside `spawn_blocking`, consistent with `create_user`.
    async fn set_password(&self, user_id: i64, new_password: &Password) -> sqlx::Result<()>;
}

// ---------------------------------------------------------------------------
// Sessions
// ---------------------------------------------------------------------------

/// A session record returned by [`SessionStorage`] queries.
#[derive(Clone, Debug)]
pub struct SessionRecord {
    pub token_hash: String,
    pub user_id: i64,
    pub username: Username,
    pub label: Option<String>,
    pub created_at: DateTime<Utc>,
    pub last_used_at: DateTime<Utc>,
}

/// Errors that can occur when authenticating a session token.
#[derive(Debug, Error)]
pub enum SessionAuthError {
    #[error("invalid token")]
    InvalidToken,
    #[error("session not found")]
    SessionNotFound,
    #[error(transparent)]
    Internal(#[from] sqlx::Error),
}

/// Async operations on the `sessions` table.
#[async_trait]
pub trait SessionStorage: Send + Sync {
    async fn create_session(&self, user_id: i64, label: Option<&str>) -> sqlx::Result<String>;

    async fn authenticate(&self, raw_token: &str) -> Result<SessionRecord, SessionAuthError>;

    async fn revoke_session(&self, token_hash: &str) -> sqlx::Result<()>;

    async fn list_sessions(&self, user_id: i64) -> sqlx::Result<Vec<SessionRecord>>;
}

// ---------------------------------------------------------------------------
// Invites
// ---------------------------------------------------------------------------

/// An invite code record returned by [`InviteStorage`] queries.
#[derive(Clone, Debug)]
pub struct InviteRecord {
    pub code: String,
    pub created_at: DateTime<Utc>,
    pub expires_at: DateTime<Utc>,
    pub used_at: Option<DateTime<Utc>>,
    pub used_by: Option<i64>,
}

/// Errors that can occur when consuming an invite code.
#[derive(Debug, Error)]
pub enum UseInviteError {
    #[error("invite code not found")]
    NotFound,
    #[error("invite code has expired")]
    Expired,
    #[error("invite code has already been used")]
    AlreadyUsed,
}

/// Async operations on the `invites` table.
#[async_trait]
pub trait InviteStorage: Send + Sync {
    async fn create_invite(&self, expires_at: DateTime<Utc>) -> sqlx::Result<String>;

    async fn use_invite(&self, code: &str, user_id: i64) -> Result<(), UseInviteError>;

    async fn list_invites(&self) -> sqlx::Result<Vec<InviteRecord>>;
}

// ---------------------------------------------------------------------------
// Atomic cross-table operations
// ---------------------------------------------------------------------------

/// Errors that can occur during atomic invite-and-user creation.
#[derive(Debug, Error)]
pub enum RegisterWithInviteError {
    #[error("invite code not found")]
    InviteNotFound,
    #[error("invite code has expired")]
    InviteExpired,
    #[error("invite code has already been used")]
    InviteAlreadyUsed,
    #[error("username is already taken")]
    UsernameTaken,
    #[error(transparent)]
    Internal(#[from] sqlx::Error),
}

/// Errors returned by an atomic password-reset confirmation.
#[derive(Debug, Error)]
pub enum ConfirmPasswordResetError {
    #[error("token not found")]
    NotFound,
    #[error("token has expired")]
    Expired,
    #[error("token has already been used")]
    AlreadyUsed,
    #[error(transparent)]
    Internal(#[from] sqlx::Error),
}

/// Cross-table operations that span multiple storage traits and must be
/// executed atomically.
///
/// The concrete implementation holds the database pool; `common` never
/// depends on a specific database driver.
#[async_trait]
pub trait AtomicOps: Send + Sync {
    /// Atomically creates a user and marks an invite code as used within a
    /// single transaction.
    async fn create_user_with_invite(
        &self,
        username: &Username,
        password: &Password,
        display_name: Option<&str>,
        invite_code: &str,
    ) -> Result<i64, RegisterWithInviteError>;

    /// Atomically consumes a password-reset token, updates the password, and
    /// revokes all sessions for the user.
    async fn confirm_password_reset(
        &self,
        raw_token: &str,
        new_password: &Password,
    ) -> Result<(), ConfirmPasswordResetError>;
}

// ---------------------------------------------------------------------------
// EmailVerification (stub — full trait added in M3.7)
// ---------------------------------------------------------------------------

/// Errors returned by [`EmailVerificationStorage::use_email_verification`].
#[derive(Debug, Error)]
pub enum UseEmailVerificationError {
    #[error("token not found")]
    NotFound,
    #[error("token has expired")]
    Expired,
    #[error("token has already been used")]
    AlreadyUsed,
    #[error(transparent)]
    Internal(#[from] sqlx::Error),
}

/// Storage for email verification tokens.
#[async_trait]
pub trait EmailVerificationStorage: Send + Sync {
    /// Stores a new verification token for `user_id` / `email` expiring at
    /// `expires_at`.  Any existing pending token for the same user is
    /// superseded (marked expired) so only one pending token exists at a time.
    /// Returns the raw (un-hashed) token to be delivered to the user by email.
    async fn create_email_verification(
        &self,
        user_id: i64,
        email: &str,
        expires_at: DateTime<Utc>,
    ) -> sqlx::Result<String>;

    /// Validates `raw_token`, marks it used, and returns `(user_id, email)`.
    async fn use_email_verification(
        &self,
        raw_token: &str,
    ) -> Result<(i64, String), UseEmailVerificationError>;
}

// ---------------------------------------------------------------------------
// PasswordReset
// ---------------------------------------------------------------------------

/// Errors returned by [`PasswordResetStorage::use_password_reset`].
#[derive(Debug, Error)]
pub enum UsePasswordResetError {
    #[error("token not found")]
    NotFound,
    #[error("token has expired")]
    Expired,
    #[error("token has already been used")]
    AlreadyUsed,
    #[error(transparent)]
    Internal(#[from] sqlx::Error),
}

/// Storage for password-reset tokens.
#[async_trait]
pub trait PasswordResetStorage: Send + Sync {
    /// Stores a new reset token for `user_id` expiring at `expires_at`.
    /// Returns the raw (un-hashed) token to be delivered to the user by email.
    async fn create_password_reset(
        &self,
        user_id: i64,
        expires_at: DateTime<Utc>,
    ) -> sqlx::Result<String>;

    /// Validates `raw_token`, marks it used, and returns `user_id`.
    async fn use_password_reset(&self, raw_token: &str) -> Result<i64, UsePasswordResetError>;
}

// ---------------------------------------------------------------------------
// Posts
// ---------------------------------------------------------------------------

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

// ---------------------------------------------------------------------------
// AppState
// ---------------------------------------------------------------------------

/// Application-wide state bundling all storage handles.
pub struct AppState {
    pub site_config: Arc<dyn SiteConfigStorage>,
    pub users: Arc<dyn UserStorage>,
    pub sessions: Arc<dyn SessionStorage>,
    pub invites: Arc<dyn InviteStorage>,
    /// Cross-table atomic operations.  The concrete implementation (in the
    /// `server` crate) holds the database pool so `common` and `web` stay
    /// free of SQLite implementation details.
    pub atomic: Arc<dyn AtomicOps>,
    /// Email verification token storage (stub until Step 7).
    pub email_verifications: Arc<dyn EmailVerificationStorage>,
    /// Password reset token storage (stub until Step 8).
    pub password_resets: Arc<dyn PasswordResetStorage>,
    /// Post storage.
    pub posts: Arc<dyn PostStorage>,
    /// Outbound email sender.  `NoopMailSender` when SMTP is not configured.
    pub mailer: Arc<dyn MailSender>,
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
