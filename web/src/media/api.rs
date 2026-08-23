//! Media wire types and `#[server]` endpoints (ADR-0070, amended #530).
//!
//! The DTOs and the three media `#[server]` fns live here; `media/mod.rs` is
//! wiring only and re-exports these under the stable `crate::media::…` paths that
//! external call sites and the server-fn registrar depend on.

use common::media::{
    ByteSize, ContentHash, ContentType, Filename, MaxFileSize, MediaSource, UserQuota,
};
// `MultipartData`/`MultipartFormData` are named in the `upload` signature,
// which compiles for both the wasm client stub and the server build, so this import
// is ungated. (#517)
use leptos::server_fn::codec::{MultipartData, MultipartFormData};
use serde::{Deserialize, Serialize};

// `upload`'s return type; ungated so it is nameable on the wasm client stub
// (where `storage` is not compiled). (#517)
use common::media::UploadResponse;

#[cfg(feature = "server")]
use {
    crate::auth::require_auth,
    crate::error::{ErrorClass, ErrorKind, InternalError},
    crate::server_fn_adapter::extract,
    common::media::MediaRef,
    // Server-only: the delete guard's key. The CSR build never runs a query.
    leptos::prelude::*,
    leptos::server_fn::error::ServerFnErrorErr,
    std::path::PathBuf,
    std::sync::Arc,
    storage::{
        MediaError, MediaManager, MediaStorage, PostStorage, SiteConfigStorage, TryDeleteOutcome,
    },
};

use common::ids::PostId;
use common::pagination::{PageOffset, PageSize};
use common::root_relative_url::RootRelativeUrl;
use common::time::UtcInstant;

use crate::error::WebResult;

/// A media item returned by [`list_mine`].
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Item {
    pub sha256: ContentHash,
    pub filename: Filename,
    pub source: MediaSource,
    pub content_type: ContentType,
    pub size_bytes: ByteSize,
    pub url: RootRelativeUrl,
    pub created_at: UtcInstant,
}

/// Storage usage returned by [`get_usage`].
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct UsageData {
    pub used_bytes: ByteSize,
    pub quota_bytes: UserQuota,
    pub max_file_size_bytes: MaxFileSize,
}

/// Result returned by [`delete`].
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct DeleteResult {
    pub deleted: bool,
    pub referenced_in_posts: Vec<PostId>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DeleteMediaRequest {
    pub sha256: ContentHash,
    pub filename: Filename,
    pub source: MediaSource,
    pub force: Option<bool>,
}

/// Lists media items owned by the authenticated user.
#[macros::server]
pub async fn list_mine(
    source: Option<MediaSource>,
    limit: Option<PageSize>,
    offset: Option<PageOffset>,
) -> WebResult<Vec<Item>> {
    let auth = require_auth().await?;
    let media = expect_context::<Arc<dyn MediaStorage>>();

    let records = media
        .list_media(
            auth.user_id,
            source.as_ref(),
            limit.unwrap_or_default().exact_limit(),
            offset.unwrap_or_default(),
        )
        .await?;

    Ok(records
        .into_iter()
        .map(|r| {
            let url = common::media::media_url(&r.source, &r.sha256, &r.filename);
            Item {
                sha256: r.sha256,
                filename: r.filename,
                source: r.source,
                content_type: r.content_type,
                size_bytes: r.size_bytes,
                url,
                created_at: UtcInstant::from(r.created_at),
            }
        })
        .collect())
}

/// Returns storage usage for the authenticated user.
#[macros::server]
pub async fn get_usage() -> WebResult<UsageData> {
    let auth = require_auth().await?;
    let media = expect_context::<Arc<dyn MediaStorage>>();
    let site_config = expect_context::<Arc<dyn SiteConfigStorage>>();

    let used_bytes = media.get_user_upload_usage(auth.user_id).await?;
    let quota_bytes = site_config.get_media_user_quota().await?;
    let max_file_size_bytes = site_config.get_media_max_file_size().await?;

    Ok(UsageData {
        used_bytes,
        quota_bytes,
        max_file_size_bytes,
    })
}

/// Deletes a media item owned by the authenticated user.
///
/// If the item is referenced in any posts, it will not be deleted unless
/// `request.force` is `Some(true)`.
#[macros::server(skip_all)]
pub async fn delete(request: DeleteMediaRequest) -> WebResult<DeleteResult> {
    let DeleteMediaRequest {
        sha256,
        filename,
        source,
        force,
    } = request;
    let auth = require_auth().await?;
    let media = expect_context::<Arc<dyn MediaStorage>>();
    let posts = expect_context::<Arc<dyn PostStorage>>();

    let media_ref = MediaRef {
        source,
        sha256,
        filename,
    };

    // The decision is made first and made in SQL: `try_delete_media`'s guard and delete
    // are one statement, so this handler holds no check-then-delete window of its own
    // (spec D8).
    let outcome = media
        .try_delete_media(auth.user_id, &media_ref, force.unwrap_or(false))
        .await
        .map_err(InternalError::storage)?;

    // Pure reporting, and only for a refusal — the one outcome that has to be explained.
    // A successful delete therefore reports an empty list even when it was forced: the
    // references it overrode did not block it, and the UI reads this field only on the
    // `deleted == false` branch. Asking unconditionally would spend a second query on
    // every happy-path delete to fill a field nothing reads.
    let referenced_in_posts: Vec<PostId> = if outcome == TryDeleteOutcome::RefusedReferenced {
        posts
            .list_posts_referencing_media(auth.user_id, &media_ref)
            .await?
    } else {
        Vec::new()
    };

    Ok(DeleteResult {
        deleted: outcome == TryDeleteOutcome::Deleted,
        referenced_in_posts,
    })
}

/// Maps an owned media upload failure to its bounded public classification while
/// retaining the complete `anyhow` chain for the operator boundary.
#[cfg(feature = "server")]
fn map_media_error(err: anyhow::Error) -> InternalError {
    let (kind, class, public_message) = match err.downcast_ref::<MediaError>() {
        Some(MediaError::BadRequest(message)) => {
            (ErrorKind::Validation, ErrorClass::Client, message.clone())
        }
        Some(MediaError::PayloadTooLarge) => (
            ErrorKind::Validation,
            ErrorClass::Client,
            "payload too large".to_owned(),
        ),
        Some(MediaError::InsufficientStorage) => (
            ErrorKind::Validation,
            ErrorClass::Client,
            "insufficient storage".to_owned(),
        ),
        Some(MediaError::Internal(_)) | None => (
            ErrorKind::Internal,
            ErrorClass::Bug,
            "server operation failed".to_owned(),
        ),
    };
    InternalError::masked(kind, class, public_message, err)
}

/// Preserves the typed server-fn extractor rejection for a missing storage-path
/// extension. Missing composition state is a server invariant, never validation.
#[cfg(feature = "server")]
fn map_storage_path_extract_error(error: ServerFnErrorErr) -> InternalError {
    InternalError::server(error).with_context("stage", "storage_path_extension")
}

/// Classifies every current multer error semantically. Malformed client framing
/// and configured size limits are validation; stream I/O, poisoned shared state,
/// and future unknown variants are server failures. Both paths retain the typed
/// multer error by ownership.
#[cfg(feature = "server")]
fn map_multipart_error(error: multer::Error) -> InternalError {
    let client = matches!(
        &error,
        multer::Error::UnknownField { .. }
            | multer::Error::IncompleteFieldData { .. }
            | multer::Error::IncompleteHeaders
            | multer::Error::ReadHeaderFailed(_)
            | multer::Error::DecodeHeaderName { .. }
            | multer::Error::DecodeHeaderValue { .. }
            | multer::Error::IncompleteStream
            | multer::Error::FieldSizeExceeded { .. }
            | multer::Error::StreamSizeExceeded { .. }
            | multer::Error::NoMultipart
            | multer::Error::DecodeContentType(_)
            | multer::Error::NoBoundary
    );
    if client {
        let message = format!("bad multipart: {error}");
        InternalError::validation_source(message, error)
    } else {
        InternalError::server(error).with_context("stage", "multipart")
    }
}

/// Streams a multipart file upload to storage and returns its stored URL/metadata.
/// The multipart `#[server]` fn (#517).
#[macros::server(input = MultipartFormData, skip_all)]
pub async fn upload(data: MultipartData) -> WebResult<UploadResponse> {
    let auth = require_auth().await?;
    let media = expect_context::<Arc<dyn MediaStorage>>();
    let site_config = expect_context::<Arc<dyn SiteConfigStorage>>();

    // `storage_path` is an axum `Extension` (server/src/lib.rs), not a leptos
    // context value, so pull it via the request extractor rather than expect_context.
    let axum::Extension(storage_path) = extract::<axum::Extension<Arc<PathBuf>>>()
        .await
        .map_err(map_storage_path_extract_error)?;

    // `into_inner()` is `Some` on the server (the parsed multipart body).
    let mut multipart = data
        .into_inner()
        .ok_or_else(|| InternalError::validation("missing multipart body"))?;

    let field = multipart
        .next_field()
        .await
        .map_err(map_multipart_error)?
        .ok_or_else(|| InternalError::validation("no file field"))?;

    // The `file_name()`/`content_type()` borrows must end before `field` is moved
    // into `upload` as the byte stream.
    let filename = MediaManager::validate_filename(field.file_name()).map_err(map_media_error)?;
    let content_type = field
        .content_type()
        .map(|value| {
            value.to_string().parse::<ContentType>().map_err(|_| {
                // multer only exposes parsed `mime::Mime` values, a strict subset of
                // `ContentType`; retain the defensive mapping if either contract changes.
                // cov:ignore-start
                map_media_error(anyhow::anyhow!(MediaError::BadRequest(
                    "Invalid content type".to_owned()
                )))
                // cov:ignore-stop
            }) // cov:ignore
        })
        .transpose()?; // cov:ignore

    let manager = MediaManager::new(media, site_config, storage_path);
    manager
        .upload(auth.user_id, &filename, content_type, field)
        .await
        .map_err(map_media_error)
}

#[cfg(all(test, feature = "server"))]
mod tests {
    use super::{MediaError, map_media_error, map_multipart_error, map_storage_path_extract_error};
    use crate::error::{ErrorKind, InternalError};
    use leptos::server_fn::error::ServerFnErrorErr;
    use std::error::Error;
    use std::fmt;

    fn typed_source<T: Error + 'static>(error: &InternalError) -> Option<&T> {
        let mut current: &(dyn Error + 'static) = error;
        loop {
            if let Some(source) = current.downcast_ref::<T>() {
                return Some(source);
            }
            current = current.source()?;
        }
    }

    #[derive(Debug)]
    struct UploadSource;

    impl fmt::Display for UploadSource {
        fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
            f.write_str("upload ownership sentinel")
        }
    }

    impl Error for UploadSource {}

    #[test]
    fn map_media_error_classifies_each_arm() {
        // A bad request / too-large / over-quota is client validation.
        assert_eq!(
            map_media_error(anyhow::anyhow!(MediaError::BadRequest("bad".to_owned()))).kind(),
            ErrorKind::Validation
        );
        assert_eq!(
            map_media_error(anyhow::anyhow!(MediaError::PayloadTooLarge)).kind(),
            ErrorKind::Validation
        );
        assert_eq!(
            map_media_error(anyhow::anyhow!(MediaError::InsufficientStorage)).kind(),
            ErrorKind::Validation
        );
        // An internal storage fault masks as a generic server error.
        assert_eq!(
            map_media_error(anyhow::anyhow!(MediaError::Internal(Box::new(
                std::io::Error::other("boom"),
            ))))
            .kind(),
            ErrorKind::Internal
        );
        // A non-`MediaError` failure (e.g. a mid-stream IO fault) downcasts to `None`
        // and also masks as a server error — the fallback arm.
        assert_eq!(
            map_media_error(anyhow::anyhow!("io boom")).kind(),
            ErrorKind::Internal
        );
    }

    #[test]
    fn missing_storage_path_extension_retains_extractor_rejection() {
        let rejection =
            ServerFnErrorErr::ServerError("missing storage path Extension sentinel".to_owned());

        let mapped = map_storage_path_extract_error(rejection);

        assert_eq!(mapped.kind(), ErrorKind::Internal);
        let source = typed_source::<ServerFnErrorErr>(&mapped)
            .expect("extractor rejection remains a typed source");
        assert!(
            source
                .to_string()
                .contains("missing storage path Extension sentinel")
        );
    }

    #[test]
    fn multipart_error_classification_is_exhaustive() {
        let client_errors = [
            multer::Error::UnknownField {
                field_name: Some("file".to_owned()),
            },
            multer::Error::IncompleteFieldData {
                field_name: Some("file".to_owned()),
            },
            multer::Error::IncompleteHeaders,
            multer::Error::ReadHeaderFailed(httparse::Error::HeaderName),
            multer::Error::DecodeHeaderName {
                name: "bad name".to_owned(),
                cause: Box::new(std::io::Error::other("header-name sentinel")),
            },
            multer::Error::DecodeHeaderValue {
                value: vec![0xff],
                cause: Box::new(std::io::Error::other("header-value sentinel")),
            },
            multer::Error::IncompleteStream,
            multer::Error::FieldSizeExceeded {
                limit: 1,
                field_name: Some("file".to_owned()),
            },
            multer::Error::StreamSizeExceeded { limit: 1 },
            multer::Error::NoMultipart,
            multer::Error::DecodeContentType(
                "not a content type"
                    .parse::<mime::Mime>()
                    .expect_err("invalid MIME"),
            ),
            multer::Error::NoBoundary,
        ];

        for error in client_errors {
            let mapped = map_multipart_error(error);
            assert_eq!(mapped.kind(), ErrorKind::Validation);
        }

        let infrastructure_errors = [
            multer::Error::StreamReadFailed(Box::new(std::io::Error::new(
                std::io::ErrorKind::ConnectionReset,
                "stream-read sentinel",
            ))),
            multer::Error::LockFailure,
        ];
        for error in infrastructure_errors {
            let expected = error.to_string();
            let mapped = map_multipart_error(error);
            assert_eq!(mapped.kind(), ErrorKind::Internal);
            let source = typed_source::<multer::Error>(&mapped)
                .expect("infrastructure multipart error remains typed");
            assert_eq!(source.to_string(), expected);
        }
    }

    #[test]
    fn upload_mapping_takes_ownership_and_retains_typed_source() {
        let mapped = map_media_error(anyhow::Error::new(UploadSource));

        assert_eq!(mapped.kind(), ErrorKind::Internal);
        assert!(
            typed_source::<UploadSource>(&mapped).is_some(),
            "owned upload source reaches InternalError"
        );
        assert!(
            mapped
                .operator_message()
                .contains("upload ownership sentinel"),
            "operator message renders the owned source"
        );
    }
}
