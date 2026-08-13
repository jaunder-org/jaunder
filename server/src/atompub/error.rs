use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};

/// The error type for the raw `AtomPub` HTTP handlers.
///
/// Handlers and their helpers (`require_user_match`, `owned_post`) return this
/// domain error; the single [`IntoResponse`] impl below is the **only** place an
/// HTTP status is chosen, keeping `StatusCode` out of the helper layer (the
/// boundary principle). Genuine internal failures are logged at `error` level as
/// they are converted (see the `From` impls), so a `500` is never a blank,
/// un-diagnosable response. The logged error is infrastructure detail (a
/// storage/IO failure), not user content, so it carries no PII.
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
    /// media upload pipeline via `media::map_error`), passed through unchanged.
    Status(StatusCode),
    /// A composed `AtomPub` URL was requested but `site.base_url` is unset, so no
    /// spec-valid absolute `atom:id` can be emitted (#560). Logged on response. `500`.
    BaseUrlRequired,
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
            HandlerError::BaseUrlRequired => {
                tracing::error!("AtomPub requires site.base_url to be configured");
                StatusCode::INTERNAL_SERVER_ERROR
            }
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

impl From<common::atompub::AtomError> for HandlerError {
    /// A document the client sent that `atom_syndication` will not parse is a `400`.
    /// This is the whole read-side mapping: handlers call `body.parse::<Entry>()?`
    /// and land here.
    fn from(_: common::atompub::AtomError) -> Self {
        HandlerError::BadRequest
    }
}

impl From<common::atompub::AtomPubError> for HandlerError {
    /// Failing to *write* a document we composed is ours, not the request's, so it
    /// logs and becomes a `500` rather than blaming the client.
    fn from(err: common::atompub::AtomPubError) -> Self {
        log_internal(&err);
        HandlerError::Internal
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

impl From<common::tag::TagValidationError> for HandlerError {
    /// An over-cap or otherwise invalid category set is the client's error, not
    /// an internal one — unlike `TaggingError`, which is always an internal
    /// inconsistency. Bounding this is what keeps the batched tag write capped by
    /// construction (#771, ADR-0092). `BadRequest` is a unit variant, so the
    /// error text is dropped: the status is the whole client-facing answer.
    fn from(_: common::tag::TagValidationError) -> Self {
        HandlerError::BadRequest
    }
}

impl From<common::post_body::InvalidPostBody> for HandlerError {
    /// An entry whose content is nothing but blank lines describes no post, so it is
    /// the client's error — the same `400` the service layer's `EmptyPost` earns
    /// below, just detected a layer earlier now that the body is typed (#811).
    fn from(_: common::post_body::InvalidPostBody) -> Self {
        HandlerError::BadRequest
    }
}

impl From<storage::PerformCreationError> for HandlerError {
    fn from(err: storage::PerformCreationError) -> Self {
        use storage::PerformCreationError as E;
        match &err {
            E::EmptyPost | E::InvalidSlug(_) => HandlerError::BadRequest,
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
            E::EmptyPost => HandlerError::BadRequest,
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
    /// `anyhow::Error`; `media::map_error` decides the client-facing status
    /// (e.g. `413` for an oversized payload). Log the underlying error — it is
    /// infrastructure detail, not user content — then pass the mapped status through.
    fn from(err: anyhow::Error) -> Self {
        tracing::error!(error = %err, "AtomPub media upload failed");
        HandlerError::Status(crate::media::map_error(&err))
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
    fn an_unparseable_document_is_a_bad_request() {
        let err = common::atompub::AtomError::InvalidStartTag;
        assert_eq!(status(err.into()), StatusCode::BAD_REQUEST);
    }

    #[test]
    fn a_serialization_failure_is_internal_not_a_bad_request() {
        // Writing a document is the server's job, so a failure there must not be
        // reported as the client having sent something wrong.
        let err = common::atompub::AtomPubError::new("x");
        assert_eq!(status(err.into()), StatusCode::INTERNAL_SERVER_ERROR);
    }

    #[test]
    fn anyhow_error_maps_through_media_map_error() {
        // Media-upload failures arrive as anyhow::Error and flow through
        // media::map_error; a generic error yields a non-success status.
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
            status(common::atompub::AtomError::InvalidStartTag.into()),
            StatusCode::BAD_REQUEST
        );
        assert_eq!(
            status(TaggingError::PostNotFound.into()),
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
