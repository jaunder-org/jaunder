//! Typed errors returned by post storage operations.

use thiserror::Error;

use common::ids::PostId;

const ACTIVE_POST_SLUG_INDEX: &str = "posts_user_slug";
const SQLITE_ACTIVE_POST_SLUG_COLUMNS: &str = "posts.user_id, posts.slug";

/// Whether a database failure names the active per-User slug invariant.
pub(crate) fn is_active_post_slug_conflict(error: &dyn sqlx::error::DatabaseError) -> bool {
    error.is_unique_violation()
        && (error.constraint() == Some(ACTIVE_POST_SLUG_INDEX)
            || error.message().contains(SQLITE_ACTIVE_POST_SLUG_COLUMNS))
}

/// Errors that can occur when creating a post.
#[derive(Debug, Error)]
pub enum CreatePostError {
    /// An active Post with the same slug already exists for this User.
    #[error("slug already taken for this user")]
    SlugConflict,
    /// A non-authoritative bookkeeping property disagreed with the final row.
    #[error("post bookkeeping does not match the stored post")]
    BookkeepingMismatch,
    /// The `(user_id, key)` pair already maps to the returned Post. The mapping
    /// was selected under the same transaction that rejected this duplicate.
    #[error("idempotency key already used for this user")]
    IdempotencyConflict(PostId),
    /// Rendering failed before the Post was written.
    #[error(transparent)]
    Render(#[from] host::render::HighlightError),
    /// Reserving a Post ID failed before the content write began.
    #[error(transparent)]
    Reservation(#[from] crate::post_service::ReservePostIdError),
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
    /// Another active Post already owns the requested slug for this User.
    #[error("slug already taken for this user")]
    SlugConflict,
    /// A non-authoritative target/final-state property disagreed with the locked row.
    #[error("post bookkeeping does not match the stored post")]
    BookkeepingMismatch,
    /// The non-authoritative current-content validator is stale.
    #[error("post content has changed")]
    StaleContent,
    /// Rendering failed before the Post was written.
    #[error(transparent)]
    Render(#[from] host::render::HighlightError),
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
            UpdatePostError::SlugConflict
            | UpdatePostError::BookkeepingMismatch
            | UpdatePostError::StaleContent => {
                InternalError::validation_source(error.to_string(), error)
            }
            UpdatePostError::Render(e) => InternalError::server(e),
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
    use super::{ListByTagError, TaggingError, UpdatePostError};

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

    // Not-found/unauthorized mask as a 404; internal is a masked storage failure.
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

        let render: InternalError =
            UpdatePostError::Render(host::render::HighlightError::Initialization {
                language: "injected-invalid-query",
                detail: "unknown node".to_owned(),
            })
            .into();
        assert_eq!(render.kind(), ErrorKind::Internal);
        assert_eq!(render.public_message(), "server operation failed");
        assert!(render.operator_message().contains("injected-invalid-query"));

        for error in [
            UpdatePostError::SlugConflict,
            UpdatePostError::BookkeepingMismatch,
            UpdatePostError::StaleContent,
        ] {
            let expected_operator_message = error.to_string();
            let internal: InternalError = error.into();
            assert_eq!(internal.kind(), ErrorKind::Validation);
            assert_eq!(internal.public_message(), expected_operator_message);
            assert_eq!(internal.operator_message(), expected_operator_message);
        }
    }

    // The `set_post_tags` lift masks as a server error
    // (`"server operation failed"`, kind `Internal`) while the typed
    // `TaggingError` is preserved on the operator side rather than stringified.
    #[test]
    fn from_tagging_error_maps_to_server() {
        use host::error::{ErrorKind, InternalError};

        let error: InternalError = TaggingError::PostNotFound.into();
        assert_eq!(error.kind(), ErrorKind::Internal);
        assert_eq!(error.public_message(), "server operation failed");
        // The typed source is preserved (not flattened to the wire message).
        assert!(error.operator_message().contains("post not found"));
    }
}
