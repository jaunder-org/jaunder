//! Typed errors returned by post storage operations.

use thiserror::Error;

use common::ids::PostId;

/// Errors that can occur when creating a post.
#[derive(Debug, Error)]
pub enum CreatePostError {
    /// A post with the same slug already exists for this user on this day.
    #[error("slug already taken for this user on this date")]
    SlugConflict,
    /// A non-authoritative bookkeeping property disagreed with the final row.
    #[error("post bookkeeping does not match the stored post")]
    BookkeepingMismatch,
    /// The `(user_id, key)` pair already maps to the returned Post. The mapping
    /// was selected under the same transaction that rejected this duplicate.
    #[error("idempotency key already used for this user")]
    IdempotencyConflict(PostId),
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
    /// A non-authoritative target/final-state property disagreed with the locked row.
    #[error("post bookkeeping does not match the stored post")]
    BookkeepingMismatch,
    /// The non-authoritative current-content validator is stale.
    #[error("post content has changed")]
    StaleContent,
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
            UpdatePostError::BookkeepingMismatch | UpdatePostError::StaleContent => {
                InternalError::validation_source(error.to_string(), error)
            }
            UpdatePostError::Internal(e) => InternalError::storage(e),
        }
    }
}

/// Errors that can occur when tagging a post.
#[derive(Debug, Error)]
pub enum TaggingError {
    /// The target post is absent from the active owner surface.
    #[error("post not found")]
    PostNotFound,
    /// The post belongs to another owner.
    #[error("not authorized to tag this post")]
    Unauthorized,
    /// An unexpected database error occurred.
    #[error(transparent)]
    Internal(#[from] sqlx::Error),
}

impl From<TaggingError> for host::error::InternalError {
    /// Preserves the current wire class of the `set_post_tags` lift:
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

#[cfg(test)]
mod tests {
    use super::{ListByTagError, TaggingError};

    #[test]
    fn tagging_error_display_post_not_found() {
        let err = TaggingError::PostNotFound;
        assert_eq!(err.to_string(), "post not found");
    }

    #[test]
    fn tagging_error_debug() {
        let err = TaggingError::PostNotFound;
        let debug_str = format!("{err:?}");
        assert!(debug_str.contains("PostNotFound"));

        let err2 = TaggingError::Internal(sqlx::Error::RowNotFound);
        let debug_str2 = format!("{err2:?}");
        assert!(debug_str2.contains("Internal"));
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
}
