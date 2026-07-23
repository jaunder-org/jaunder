//! Media wire types and `#[server]` endpoints (ADR-0070, amended #530).
//!
//! The DTOs and the three media `#[server]` fns live here; `media/mod.rs` is
//! wiring only and re-exports these under the stable `crate::media::…` paths that
//! external call sites and the server-fn registrar depend on.

use common::media::{
    ByteSize, ContentHash, ContentType, Filename, MaxFileSize, MediaSource, UserQuota,
};
use leptos::prelude::*;
// `MultipartData`/`MultipartFormData` are named in the `upload_media` signature,
// which compiles for both the wasm client stub and the server build, so this import
// is ungated. (#517)
use leptos::server_fn::codec::{MultipartData, MultipartFormData};
use serde::{Deserialize, Serialize};

// `upload_media`'s return type; ungated so it is nameable on the wasm client stub
// (where `storage` is not compiled). (#517)
use common::media::UploadResponse;

#[cfg(feature = "server")]
use {
    crate::auth::require_auth,
    crate::error::InternalError,
    leptos_axum::extract,
    std::path::PathBuf,
    std::sync::Arc,
    storage::{MediaManager, MediaStorage, PostStorage, SiteConfigStorage},
};

use common::ids::PostId;
use common::pagination::{PageOffset, PageSize};
use common::time::UtcInstant;

use crate::error::WebResult;

/// A media item returned by [`list_my_media`].
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct MediaItem {
    pub sha256: ContentHash,
    pub filename: Filename,
    pub source: MediaSource,
    pub content_type: ContentType,
    pub size_bytes: ByteSize,
    pub url: String,
    pub created_at: UtcInstant,
}

/// Storage usage returned by [`media_usage`].
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct MediaUsageData {
    pub used_bytes: ByteSize,
    pub quota_bytes: UserQuota,
    pub max_file_size_bytes: MaxFileSize,
}

/// Result returned by [`delete_media`].
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct DeleteMediaResult {
    pub deleted: bool,
    pub referenced_in_posts: Vec<PostId>,
}

/// Lists media items owned by the authenticated user.
#[server(endpoint = "/list_my_media")]
pub async fn list_my_media(
    source: Option<MediaSource>,
    limit: Option<PageSize>,
    offset: Option<PageOffset>,
) -> WebResult<Vec<MediaItem>> {
    boundary!("list_my_media", {
        let auth = require_auth().await?;
        let media = expect_context::<Arc<dyn MediaStorage>>();

        let records = media
            .list_media(
                auth.user_id,
                source.as_ref(),
                limit.unwrap_or_default().value(),
                offset.unwrap_or_default(),
            )
            .await?;

        Ok(records
            .into_iter()
            .map(|r| {
                let url = common::media::media_url(r.source.as_str(), &r.sha256, &r.filename);
                MediaItem {
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
    })
}

/// Returns storage usage for the authenticated user.
#[server(endpoint = "/media_usage")]
pub async fn media_usage() -> WebResult<MediaUsageData> {
    boundary!("media_usage", {
        let auth = require_auth().await?;
        let media = expect_context::<Arc<dyn MediaStorage>>();
        let site_config = expect_context::<Arc<dyn SiteConfigStorage>>();

        let used_bytes = media.get_user_upload_usage(auth.user_id).await?;
        let quota_bytes = site_config.get_media_user_quota().await?;
        let max_file_size_bytes = site_config.get_media_max_file_size().await?;

        Ok(MediaUsageData {
            used_bytes,
            quota_bytes,
            max_file_size_bytes,
        })
    })
}

/// Deletes a media item owned by the authenticated user.
///
/// If the item is referenced in any posts, it will not be deleted unless
/// `force` is `Some(true)`.
#[server(endpoint = "/delete_media")]
pub async fn delete_media(
    sha256: ContentHash,
    filename: Filename,
    source: MediaSource,
    force: Option<bool>,
) -> WebResult<DeleteMediaResult> {
    boundary!("delete_media", {
        let auth = require_auth().await?;
        let media = expect_context::<Arc<dyn MediaStorage>>();
        let posts = expect_context::<Arc<dyn PostStorage>>();

        let url = common::media::media_url(source.as_str(), &sha256, &filename);

        let published = posts
            .list_published_by_user(
                &auth.username,
                None,
                1000,
                &crate::viewer::viewer_identity().await,
                chrono::Utc::now(),
            )
            .await?;

        let drafts = posts
            .list_drafts_by_user(auth.user_id, None, 1000, chrono::Utc::now())
            .await?;

        let referenced_in_posts: Vec<PostId> = published
            .iter()
            .chain(drafts.iter())
            .filter(|post| post.body.contains(&url) || post.rendered_html.contains(&url))
            .map(|post| post.post_id)
            .collect();

        if !referenced_in_posts.is_empty() && !force.unwrap_or(false) {
            return Ok(DeleteMediaResult {
                deleted: false,
                referenced_in_posts,
            });
        }

        media
            .delete_media(auth.user_id, &sha256, &filename, &source)
            .await
            .map_err(InternalError::storage)?;

        Ok(DeleteMediaResult {
            deleted: true,
            referenced_in_posts,
        })
    })
}

/// Maps a media upload `anyhow::Error` (carrying a `storage::MediaError`) to an
/// `InternalError`, so `boundary!` projects it to the right `WebError`: a bad
/// request / too-large / over-quota is client validation (`WebError::Validation`),
/// an internal or unknown failure masks as a server error (`WebError::Server`). The
/// upload metric is already emitted inside `storage::MediaManager`, so this is a
/// pure classification.
#[cfg(feature = "server")]
fn map_media_error(err: &anyhow::Error) -> InternalError {
    match err.downcast_ref::<storage::MediaError>() {
        Some(storage::MediaError::BadRequest(message)) => {
            InternalError::validation(message.clone())
        }
        Some(storage::MediaError::PayloadTooLarge) => {
            InternalError::validation("payload too large")
        }
        Some(storage::MediaError::InsufficientStorage) => {
            InternalError::validation("insufficient storage")
        }
        // cov:ignore-start — defensive server-error fallback, reached only by a
        // non-`MediaError` upload failure (e.g. a mid-request DB/IO fault). `require_auth`
        // would trip such a fault first, so it is unreachable from an integration test.
        Some(storage::MediaError::Internal(_)) | None => {
            InternalError::server_message(err.to_string())
        } // cov:ignore-stop
    }
}

/// Streams a multipart file upload to storage and returns its stored URL/metadata.
/// The multipart `#[server]` fn replacing the old `POST /media/upload` glue (#517).
#[server(input = MultipartFormData, endpoint = "/upload_media")]
pub async fn upload_media(data: MultipartData) -> WebResult<UploadResponse> {
    boundary!("upload_media", {
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
    })
}
