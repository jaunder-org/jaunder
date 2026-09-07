use axum::Json;
use axum::http::{HeaderValue, StatusCode, header};
use axum::response::{IntoResponse, Response};
use common::ids::PostId;
use serde::Serialize;
use thiserror::Error;

/// The error type for the raw `AtomPub` HTTP handlers.
///
/// Handlers and their helpers (`require_user_match`, `owned_post`) return this
/// domain error; the single [`IntoResponse`] impl below is the **only** place an
/// HTTP status is chosen, keeping `StatusCode` out of the helper layer (the
/// boundary principle). Genuine internal failures are logged at `error` level as
/// they are converted (see the `From` impls), so a `500` is never a blank,
/// un-diagnosable response. The logged error is infrastructure detail (a
/// storage/IO failure), not user content, so it carries no PII.
#[derive(Debug, Error)]
pub enum HandlerError {
    /// Malformed request input (bad entry XML, bad cursor, empty filename). `400`.
    #[error("bad request")]
    BadRequest,
    /// The caller may not act on another user's resources. `403`.
    #[error("forbidden")]
    Forbidden,
    /// New media creation is disabled site-wide. `403`.
    #[error("media uploads are disabled")]
    UploadsDisabled,
    /// The addressed resource is missing, deleted, or hidden from this user. `404`.
    #[error("not found")]
    NotFound,
    /// A conditional request (`If-Match`) did not match the current `ETag`. `412`.
    #[error("precondition failed")]
    PreconditionFailed,
    /// A status already decided by a subsystem that maps its own errors.
    #[error("HTTP status {0}")]
    Status(StatusCode),
    /// A composed URL was requested while `site.base_url` is unset. `500`.
    #[error("site.base_url is required")]
    BaseUrlRequired,
    /// A genuine internal failure with its typed source retained. `500`.
    #[error("AtomPub internal failure")]
    Internal(#[source] Box<dyn std::error::Error + Send + Sync>),
    /// A source-less invariant failure. `500`.
    #[error("AtomPub invariant failure")]
    Invariant,
}

/// The problem document returned when an `AtomPub` media deletion is unsafe.
#[derive(Debug)]
pub(super) enum MediaDeleteConflict {
    /// The authenticated owner's live Posts reference the media.
    OwnerReferences {
        post_ids: Vec<PostId>,
        theme_reference_count: u64,
    },
    /// Global reference evidence makes deletion unsafe without reportable owner IDs.
    GlobalSafety { theme_reference_count: u64 },
}

impl MediaDeleteConflict {
    /// Builds an owner-reference conflict whose Post IDs are safe to disclose.
    pub(super) fn owner_references(
        post_ids: impl IntoIterator<Item = PostId>,
        theme_reference_count: u64,
    ) -> Self {
        let mut post_ids: Vec<_> = post_ids.into_iter().collect();
        post_ids.sort_unstable_by_key(|post_id| i64::from(*post_id));
        post_ids.dedup();
        Self::OwnerReferences {
            post_ids,
            theme_reference_count,
        }
    }

    /// Builds a global safety conflict without disclosing any Post IDs.
    pub(super) const fn global_safety(theme_reference_count: u64) -> Self {
        Self::GlobalSafety {
            theme_reference_count,
        }
    }
}

impl IntoResponse for MediaDeleteConflict {
    fn into_response(self) -> Response {
        let (detail, post_ids, theme_reference_count) = match self {
            Self::OwnerReferences {
                post_ids,
                theme_reference_count,
            } => (
                "Media is referenced by retained Posts or revisions. Use Jaunder's web media library to review references before deleting.",
                post_ids,
                theme_reference_count,
            ),
            Self::GlobalSafety {
                theme_reference_count,
            } => (
                "Media deletion is blocked because Jaunder cannot prove that removing this record would preserve referenced media.",
                Vec::new(),
                theme_reference_count,
            ),
        };
        let mut response = Json(MediaDeleteProblem {
            problem_type: "https://jaunder.org/problems/media-delete-conflict",
            title: "Media deletion refused",
            status: StatusCode::CONFLICT.as_u16(),
            detail,
            post_ids,
            theme_reference_count,
        })
        .into_response();
        *response.status_mut() = StatusCode::CONFLICT;
        response.headers_mut().insert(
            header::CONTENT_TYPE,
            HeaderValue::from_static("application/problem+json"),
        );
        response
    }
}

/// The fixed schema for `AtomPub` media deletion refusal documents.
#[derive(Serialize)]
struct MediaDeleteProblem {
    #[serde(rename = "type")]
    problem_type: &'static str,
    title: &'static str,
    status: u16,
    detail: &'static str,
    post_ids: Vec<PostId>,
    theme_reference_count: u64,
}

impl IntoResponse for HandlerError {
    fn into_response(self) -> Response {
        match self {
            HandlerError::UploadsDisabled => {
                (StatusCode::FORBIDDEN, "media uploads are disabled").into_response()
            }
            HandlerError::BadRequest => StatusCode::BAD_REQUEST.into_response(),
            HandlerError::Forbidden => StatusCode::FORBIDDEN.into_response(),
            HandlerError::NotFound => StatusCode::NOT_FOUND.into_response(),
            HandlerError::PreconditionFailed => StatusCode::PRECONDITION_FAILED.into_response(),
            HandlerError::Status(code) => code.into_response(),
            HandlerError::BaseUrlRequired => {
                tracing::error!("AtomPub requires site.base_url to be configured");
                StatusCode::INTERNAL_SERVER_ERROR.into_response()
            }
            HandlerError::Internal(_) | HandlerError::Invariant => {
                StatusCode::INTERNAL_SERVER_ERROR.into_response()
            }
        }
    }
}

/// Records a genuine internal failure at `error` level before it is mapped to a
/// `500`. The error is a storage/IO failure, not user content, so it has no PII.
fn log_internal<E: std::error::Error>(err: &E) {
    tracing::error!(error = %err, "AtomPub handler internal error");
}

fn internal<E>(err: E) -> HandlerError
where
    E: std::error::Error + Send + Sync + 'static,
{
    log_internal(&err);
    HandlerError::Internal(Box::new(err))
}

/// Reports and retains an opaque manager failure before returning a masked `500`.
pub(super) fn internal_anyhow(err: anyhow::Error) -> HandlerError {
    tracing::error!(error = %err, "AtomPub handler internal error");
    HandlerError::Internal(err.into())
}

impl From<sqlx::Error> for HandlerError {
    fn from(err: sqlx::Error) -> Self {
        internal(err)
    }
}

impl From<StatusCode> for HandlerError {
    fn from(code: StatusCode) -> Self {
        HandlerError::Status(code)
    }
}

impl From<host::atompub::Error> for HandlerError {
    /// A document the client sent that `atom_syndication` will not parse is a `400`.
    /// This is the whole read-side mapping: handlers call `body.parse::<Entry>()?`
    /// and land here.
    fn from(_: host::atompub::Error) -> Self {
        HandlerError::BadRequest
    }
}

impl From<host::atompub::AtomPubError> for HandlerError {
    /// Failing to *write* a document we composed is ours, not the request's, so it
    /// retains its source and becomes a `500` rather than blaming the client.
    fn from(err: host::atompub::AtomPubError) -> Self {
        internal(err)
    }
}

impl From<storage::TaggingError> for HandlerError {
    /// In the create/update flow the post and tags are freshly resolved, so any
    /// `TaggingError` is an internal inconsistency or DB failure.
    fn from(err: storage::TaggingError) -> Self {
        internal(err)
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

impl From<common::org::OrgMetadataError> for HandlerError {
    /// The parsed Org metadata is request input; neither malformed metadata nor
    /// a metadata-only document may reach persistence.
    fn from(_: common::org::OrgMetadataError) -> Self {
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
        match err {
            storage::PerformCreationError::EmptyPost
            | storage::PerformCreationError::InvalidSlug(_)
            | storage::PerformCreationError::BookkeepingMismatch => HandlerError::BadRequest,
            // Exhausted/CreatedNotFound/Storage are all internal failures.
            error => internal(error),
        }
    }
}

impl From<storage::PerformUpdateError> for HandlerError {
    fn from(err: storage::PerformUpdateError) -> Self {
        match err {
            storage::PerformUpdateError::EmptyPost
            | storage::PerformUpdateError::BookkeepingMismatch => HandlerError::BadRequest,
            storage::PerformUpdateError::StaleContent => HandlerError::PreconditionFailed,
            storage::PerformUpdateError::NotFound | storage::PerformUpdateError::Unauthorized => {
                HandlerError::NotFound
            }
            error @ storage::PerformUpdateError::Storage(_) => internal(error),
        }
    }
}

impl From<storage::DeleteMediaError> for HandlerError {
    fn from(err: storage::DeleteMediaError) -> Self {
        internal(err)
    }
}

impl From<anyhow::Error> for HandlerError {
    /// The media upload pipeline (`MediaManager::upload_bytes`) reports failures as
    /// `anyhow::Error`; `media::map_error` decides the client-facing status
    /// (e.g. `413` for an oversized payload). Expected typed client outcomes keep
    /// their bounded public classification; remaining failures are logged before their
    /// mapped status is passed through.
    fn from(err: anyhow::Error) -> Self {
        if matches!(
            err.downcast_ref::<storage::MediaError>(),
            Some(storage::MediaError::UploadsDisabled)
        ) {
            return HandlerError::UploadsDisabled;
        }

        tracing::error!(error = %err, "AtomPub media upload failed");
        HandlerError::Status(crate::media::map_error(&err))
    }
}

#[cfg(test)]
mod tests {
    use super::{HandlerError, MediaDeleteConflict};
    use axum::http::{StatusCode, header};
    use axum::response::IntoResponse;
    use common::ids::PostId;
    use storage::{DeleteMediaError, PerformCreationError, PerformUpdateError, TaggingError};

    /// The status an error maps to through the single `IntoResponse` boundary.
    fn status(err: HandlerError) -> StatusCode {
        err.into_response().status()
    }

    #[tokio::test]
    async fn owner_media_delete_conflict_is_a_problem_document_with_sorted_unique_post_ids() {
        let response = MediaDeleteConflict::owner_references(
            [
                PostId::from(7),
                PostId::from(2),
                PostId::from(7),
                PostId::from(12),
                PostId::from(2),
            ],
            3,
        )
        .into_response();

        assert_eq!(response.status(), StatusCode::CONFLICT);
        assert_eq!(
            response.headers()[header::CONTENT_TYPE],
            "application/problem+json"
        );
        let body = axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .expect("read problem response body");
        assert_eq!(
            body.as_ref(),
            br#"{"type":"https://jaunder.org/problems/media-delete-conflict","title":"Media deletion refused","status":409,"detail":"Media is referenced by retained Posts or revisions. Use Jaunder's web media library to review references before deleting.","post_ids":[2,7,12],"theme_reference_count":3}"#
        );
    }

    #[tokio::test]
    async fn global_media_delete_conflict_is_a_problem_document_without_post_ids() {
        let response = MediaDeleteConflict::global_safety(0).into_response();

        assert_eq!(response.status(), StatusCode::CONFLICT);
        assert_eq!(
            response.headers()[header::CONTENT_TYPE],
            "application/problem+json"
        );
        let body = axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .expect("read problem response body");
        assert_eq!(
            body.as_ref(),
            br#"{"type":"https://jaunder.org/problems/media-delete-conflict","title":"Media deletion refused","status":409,"detail":"Media deletion is blocked because Jaunder cannot prove that removing this record would preserve referenced media.","post_ids":[],"theme_reference_count":0}"#
        );
    }

    #[test]
    fn an_unparseable_document_is_a_bad_request() {
        let err = host::atompub::Error::InvalidStartTag;
        assert_eq!(status(err.into()), StatusCode::BAD_REQUEST);
    }

    #[test]
    fn a_serialization_failure_is_internal_not_a_bad_request() {
        // Writing a document is the server's job, so a failure there must not be
        // reported as the client having sent something wrong.
        let err = host::atompub::AtomPubError::Utf8(
            String::from_utf8(vec![0xff]).expect_err("invalid UTF-8"),
        );
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
    fn disabled_upload_error_is_forbidden() {
        let error = anyhow::anyhow!(storage::MediaError::UploadsDisabled);
        assert_eq!(status(error.into()), StatusCode::FORBIDDEN);
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
            status(HandlerError::Invariant),
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
            status(host::atompub::Error::InvalidStartTag.into()),
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
            status(PerformCreationError::BookkeepingMismatch.into()),
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
            status(PerformUpdateError::BookkeepingMismatch.into()),
            StatusCode::BAD_REQUEST
        );
        assert_eq!(
            status(PerformUpdateError::StaleContent.into()),
            StatusCode::PRECONDITION_FAILED
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
    fn delete_media_error_masks_internal_failures() {
        assert_eq!(
            status(DeleteMediaError::Internal(sqlx::Error::PoolClosed).into()),
            StatusCode::INTERNAL_SERVER_ERROR
        );
    }
}
