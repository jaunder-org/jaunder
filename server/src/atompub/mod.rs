//! Server-side `AtomPub` surface: the boundary mapping Jaunder posts/media to
//! `AtomPub` wire types, plus the HTTP handlers.

use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use axum::Router;
use storage::AppState;
use web::auth::AuthUser;

pub mod mapping;
pub mod media;
pub mod posts;
pub mod rsd;
pub mod service;

/// Builds the `AtomPub` routes (mergeable into the main application router).
///
/// The handlers read shared state via `Extension`, so the routes are generic
/// over the application's router state type.
pub fn router<S>() -> Router<S>
where
    S: Clone + Send + Sync + 'static,
{
    Router::new()
        .route("/atompub/service", get(service::service_document))
        .route(
            "/atompub/{username}/posts",
            get(posts::collection_get).post(posts::collection_post),
        )
        .route(
            "/atompub/{username}/posts/{post_id}",
            get(posts::member_get)
                .put(posts::member_put)
                .delete(posts::member_delete),
        )
        .route("/atompub/{username}/media", post(media::collection_post))
        .route(
            "/atompub/{username}/media/{sha}/{filename}",
            get(media::member_get).delete(media::member_delete),
        )
        .route("/~{username}/rsd.xml", get(rsd::rsd_document))
}

/// Authorizes that `auth_user` may act on resources scoped to `username`.
///
/// `AtomPub` collection handlers are addressed by `{username}`; a user may only
/// act on their own resources, so a mismatch yields `403 Forbidden`. The member
/// handlers fold the same check into `owned_post`.
pub(crate) fn require_user_match(auth_user: &AuthUser, username: &str) -> Result<(), HandlerError> {
    if auth_user.username.as_str() == username {
        Ok(())
    } else {
        Err(HandlerError::Forbidden)
    }
}

/// Returns the site's public base URL (scheme + host, no trailing slash), or an
/// empty string when unconfigured (callers then emit root-relative URLs).
pub(crate) async fn base_url(state: &AppState) -> String {
    state
        .site_config
        .get_identity()
        .await
        .ok()
        .and_then(|identity| identity.base_url)
        .unwrap_or_default()
}

/// The error type for the raw `AtomPub` HTTP handlers.
///
/// Handlers and their helpers (`require_user_match`, `owned_post`,
/// `apply_categories`) return this domain error; the single [`IntoResponse`]
/// impl below is the **only** place an HTTP status is chosen, keeping
/// `StatusCode` out of the helper layer (the boundary principle). Genuine
/// internal failures are logged at `error` level as they are converted (see the
/// `From` impls), so a `500` is never a blank, un-diagnosable response. The
/// logged error is infrastructure detail (a storage/IO failure), not user
/// content, so it carries no PII.
#[derive(Debug)]
pub enum HandlerError {
    /// Malformed request input (bad entry XML, bad cursor, empty filename). `400`.
    BadRequest,
    /// The caller may not act on another user's resources. `403`.
    Forbidden,
    /// The addressed resource is missing, deleted, or hidden from this user. `404`.
    NotFound,
    /// A conditional request (`If-Match`) did not match the current `ETag`. `412`.
    PreconditionFailed,
    /// A status already decided by a subsystem that maps its own errors (e.g. the
    /// media upload pipeline via `MediaManager::map_error`), passed through unchanged.
    Status(StatusCode),
    /// A genuine internal failure (storage/IO). Logged on construction. `500`.
    Internal,
}

impl IntoResponse for HandlerError {
    fn into_response(self) -> Response {
        let status = match self {
            HandlerError::BadRequest => StatusCode::BAD_REQUEST,
            HandlerError::Forbidden => StatusCode::FORBIDDEN,
            HandlerError::NotFound => StatusCode::NOT_FOUND,
            HandlerError::PreconditionFailed => StatusCode::PRECONDITION_FAILED,
            HandlerError::Status(code) => code,
            HandlerError::Internal => StatusCode::INTERNAL_SERVER_ERROR,
        };
        status.into_response()
    }
}

/// Records a genuine internal failure at `error` level before it is mapped to a
/// `500`. The error is a storage/IO failure, not user content, so it has no PII.
fn log_internal<E: std::error::Error>(err: &E) {
    tracing::error!(error = %err, "AtomPub handler internal error");
}

impl From<sqlx::Error> for HandlerError {
    fn from(err: sqlx::Error) -> Self {
        log_internal(&err);
        HandlerError::Internal
    }
}

impl From<StatusCode> for HandlerError {
    fn from(code: StatusCode) -> Self {
        HandlerError::Status(code)
    }
}

impl From<common::atompub::AtomPubError> for HandlerError {
    /// A malformed `AtomPub` document supplied by the client is a `400`.
    fn from(_: common::atompub::AtomPubError) -> Self {
        HandlerError::BadRequest
    }
}

impl From<storage::TaggingError> for HandlerError {
    /// In the create/update flow the post and tags are freshly resolved, so any
    /// `TaggingError` is an internal inconsistency or DB failure.
    fn from(err: storage::TaggingError) -> Self {
        log_internal(&err);
        HandlerError::Internal
    }
}

impl From<storage::PerformCreationError> for HandlerError {
    fn from(err: storage::PerformCreationError) -> Self {
        use storage::PerformCreationError as E;
        match &err {
            E::EmptyPost | E::NoSlugFromPost | E::InvalidSlug(_) => HandlerError::BadRequest,
            // Exhausted/CreatedNotFound/Storage are all internal failures.
            _ => {
                log_internal(&err);
                HandlerError::Internal
            }
        }
    }
}

impl From<storage::PerformUpdateError> for HandlerError {
    fn from(err: storage::PerformUpdateError) -> Self {
        use storage::PerformUpdateError as E;
        match &err {
            E::EmptyPost | E::NoSlugFromPost | E::InvalidSlug => HandlerError::BadRequest,
            E::NotFound | E::Unauthorized => HandlerError::NotFound,
            E::Storage(_) => {
                log_internal(&err);
                HandlerError::Internal
            }
        }
    }
}

impl From<storage::DeleteMediaError> for HandlerError {
    fn from(err: storage::DeleteMediaError) -> Self {
        use storage::DeleteMediaError as E;
        match &err {
            E::NotFound => HandlerError::NotFound,
            E::Internal(_) => {
                log_internal(&err);
                HandlerError::Internal
            }
        }
    }
}

impl From<anyhow::Error> for HandlerError {
    /// The media upload pipeline (`MediaManager::upload_bytes`) reports failures as
    /// `anyhow::Error`; `MediaManager::map_error` decides the client-facing status
    /// (e.g. `413` for an oversized payload). Log the underlying error — it is
    /// infrastructure detail, not user content — then pass the mapped status through.
    fn from(err: anyhow::Error) -> Self {
        tracing::error!(error = %err, "AtomPub media upload failed");
        HandlerError::Status(crate::media_manager::MediaManager::map_error(&err))
    }
}

#[cfg(test)]
mod tests {
    use super::HandlerError;
    use axum::http::StatusCode;
    use axum::response::IntoResponse;
    use storage::{DeleteMediaError, PerformCreationError, PerformUpdateError, TaggingError};

    /// The status an error maps to through the single `IntoResponse` boundary.
    fn status(err: HandlerError) -> StatusCode {
        err.into_response().status()
    }

    #[test]
    fn anyhow_error_maps_through_media_map_error() {
        // Media-upload failures arrive as anyhow::Error and flow through
        // MediaManager::map_error; a generic error yields a non-success status.
        let code = status(anyhow::anyhow!("upload boom").into());
        assert!(code.is_client_error() || code.is_server_error());
    }

    #[test]
    fn plain_variants_map_to_their_status() {
        assert_eq!(status(HandlerError::BadRequest), StatusCode::BAD_REQUEST);
        assert_eq!(status(HandlerError::Forbidden), StatusCode::FORBIDDEN);
        assert_eq!(status(HandlerError::NotFound), StatusCode::NOT_FOUND);
        assert_eq!(
            status(HandlerError::PreconditionFailed),
            StatusCode::PRECONDITION_FAILED
        );
        assert_eq!(
            status(HandlerError::Internal),
            StatusCode::INTERNAL_SERVER_ERROR
        );
        assert_eq!(
            status(HandlerError::from(StatusCode::IM_A_TEAPOT)),
            StatusCode::IM_A_TEAPOT
        );
    }

    #[test]
    fn storage_and_document_errors_map_to_status() {
        assert_eq!(
            status(sqlx::Error::PoolClosed.into()),
            StatusCode::INTERNAL_SERVER_ERROR
        );
        assert_eq!(
            status(common::atompub::AtomPubError::Malformed("x".into()).into()),
            StatusCode::BAD_REQUEST
        );
        assert_eq!(
            status(TaggingError::AlreadyTagged.into()),
            StatusCode::INTERNAL_SERVER_ERROR
        );
    }

    #[test]
    fn creation_error_maps_validation_to_400_else_500() {
        assert_eq!(
            status(PerformCreationError::EmptyPost.into()),
            StatusCode::BAD_REQUEST
        );
        assert_eq!(
            status(PerformCreationError::NoSlugFromPost.into()),
            StatusCode::BAD_REQUEST
        );
        assert_eq!(
            status(PerformCreationError::InvalidSlug(common::slug::InvalidSlug).into()),
            StatusCode::BAD_REQUEST
        );
        assert_eq!(
            status(PerformCreationError::CreatedNotFound.into()),
            StatusCode::INTERNAL_SERVER_ERROR
        );
        assert_eq!(
            status(PerformCreationError::Storage(sqlx::Error::PoolClosed).into()),
            StatusCode::INTERNAL_SERVER_ERROR
        );
    }

    #[test]
    fn update_error_maps_each_class() {
        assert_eq!(
            status(PerformUpdateError::EmptyPost.into()),
            StatusCode::BAD_REQUEST
        );
        assert_eq!(
            status(PerformUpdateError::NoSlugFromPost.into()),
            StatusCode::BAD_REQUEST
        );
        assert_eq!(
            status(PerformUpdateError::InvalidSlug.into()),
            StatusCode::BAD_REQUEST
        );
        assert_eq!(
            status(PerformUpdateError::NotFound.into()),
            StatusCode::NOT_FOUND
        );
        assert_eq!(
            status(PerformUpdateError::Unauthorized.into()),
            StatusCode::NOT_FOUND
        );
        assert_eq!(
            status(PerformUpdateError::Storage(sqlx::Error::PoolClosed).into()),
            StatusCode::INTERNAL_SERVER_ERROR
        );
    }

    #[test]
    fn delete_media_error_maps_not_found_and_internal() {
        assert_eq!(
            status(DeleteMediaError::NotFound.into()),
            StatusCode::NOT_FOUND
        );
        assert_eq!(
            status(DeleteMediaError::Internal(sqlx::Error::PoolClosed).into()),
            StatusCode::INTERNAL_SERVER_ERROR
        );
    }
}
