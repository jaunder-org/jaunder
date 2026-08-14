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
    crate::error::InternalError,
    // Server-only: the delete guard's key. The CSR build never runs a query.
    common::media::MediaRef,
    leptos::prelude::*,
    leptos_axum::extract,
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

/// Maps a media upload `anyhow::Error` (carrying a `storage::MediaError`) to an
/// `InternalError`, so the error boundary projects it to the right `WebError`: a bad
/// request / too-large / over-quota is client validation (`WebError::Validation`),
/// an internal or unknown failure masks as a server error (`WebError::Server`). The
/// upload metric is already emitted inside `storage::MediaManager`, so this is a
/// pure classification.
#[cfg(feature = "server")]
fn map_media_error(err: &anyhow::Error) -> InternalError {
    match err.downcast_ref::<MediaError>() {
        Some(MediaError::BadRequest(message)) => InternalError::validation(message.clone()),
        Some(MediaError::PayloadTooLarge) => InternalError::validation("payload too large"),
        Some(MediaError::InsufficientStorage) => InternalError::validation("insufficient storage"),
        // A `MediaError::Internal` or a non-`MediaError` upload failure (e.g. a mid-stream
        // IO fault, which downcasts to `None`) masks as a generic server error.
        Some(MediaError::Internal(_)) | None => InternalError::server_message(err.to_string()),
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
        .map_err(|e| InternalError::server_message(format!("storage_path extract: {e}")))?;

    // `into_inner()` is `Some` on the server (the parsed multipart body).
    let mut multipart = data
        .into_inner()
        .ok_or_else(|| InternalError::validation("missing multipart body"))?;

    let field = multipart
        .next_field()
        .await
        .map_err(|e| InternalError::validation(format!("bad multipart: {e}")))?
        .ok_or_else(|| InternalError::validation("no file field"))?;

    // The `file_name()`/`content_type()` borrows must end before `field` is moved
    // into `upload` as the byte stream.
    let filename =
        MediaManager::validate_filename(field.file_name()).map_err(|e| map_media_error(&e))?;
    // `multer::Field::content_type()` yields `Option<&mime::Mime>`; render it to a
    // `String` so it outlives the field being moved into `upload` as the stream.
    let content_type = field.content_type().map(ToString::to_string);

    let manager = MediaManager::new(media, site_config, storage_path);
    manager
        .upload(auth.user_id, &filename, content_type.as_deref(), field)
        .await
        .map_err(|e| map_media_error(&e))
}

#[cfg(all(test, feature = "server"))]
mod tests {
    use super::{MediaError, map_media_error};
    use crate::error::ErrorKind;

    #[test]
    fn map_media_error_classifies_each_arm() {
        // A bad request / too-large / over-quota is client validation.
        assert_eq!(
            map_media_error(&anyhow::anyhow!(MediaError::BadRequest("bad".to_owned()))).kind(),
            ErrorKind::Validation
        );
        assert_eq!(
            map_media_error(&anyhow::anyhow!(MediaError::PayloadTooLarge)).kind(),
            ErrorKind::Validation
        );
        assert_eq!(
            map_media_error(&anyhow::anyhow!(MediaError::InsufficientStorage)).kind(),
            ErrorKind::Validation
        );
        // An internal storage fault masks as a generic server error.
        assert_eq!(
            map_media_error(&anyhow::anyhow!(MediaError::Internal("boom".to_owned()))).kind(),
            ErrorKind::Internal
        );
        // A non-`MediaError` failure (e.g. a mid-stream IO fault) downcasts to `None`
        // and also masks as a server error — the fallback arm.
        assert_eq!(
            map_media_error(&anyhow::anyhow!("io boom")).kind(),
            ErrorKind::Internal
        );
    }
}
