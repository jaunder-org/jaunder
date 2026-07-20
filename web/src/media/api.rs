//! Media wire types and `#[server]` endpoints (ADR-0070, amended #530).
//!
//! The DTOs and the three media `#[server]` fns live here; `media/mod.rs` is
//! wiring only and re-exports these under the stable `crate::media::…` paths that
//! external call sites and the server-fn registrar depend on.

use common::media::{ContentHash, Filename, MaxFileSize, UserQuota};
use leptos::prelude::*;
use serde::{Deserialize, Serialize};

#[cfg(feature = "server")]
use {
    crate::auth::require_auth,
    crate::error::InternalError,
    std::sync::Arc,
    storage::{MediaSource, MediaStorage, PostStorage, SiteConfigStorage},
};

use common::ids::PostId;

use crate::error::WebResult;

/// A media item returned by [`list_my_media`].
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct MediaItem {
    pub sha256: ContentHash,
    pub filename: Filename,
    pub source: String,
    pub content_type: String,
    pub size_bytes: i64,
    pub url: String,
    pub created_at: String,
}

/// Storage usage returned by [`media_usage`].
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct MediaUsageData {
    pub used_bytes: i64,
    pub quota_bytes: UserQuota,
    pub max_file_size_bytes: MaxFileSize,
}

/// Result returned by [`delete_media`].
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct DeleteMediaResult {
    pub deleted: bool,
    pub referenced_in_posts: Vec<PostId>,
}

/// Extracts the `url` field from the `/media/upload` JSON response body.
///
/// The upload endpoint (`server/src/media.rs`) returns `{"url": "/media/…", …}`;
/// the wasm-only upload glue in [`super::component`] calls this to pull the media
/// URL out of the response text. It lives here — ungated, host-compiled, and
/// coverage-measured — so the parse is unit-tested off the browser (ADR-0055: pure
/// logic is extracted before the surrounding code is wasm-gated).
///
/// # Errors
///
/// Returns `Err` with a human-readable message when the body is not valid JSON or
/// has no string `url` field.
///
/// `pub` (and re-exported from `mod.rs`) so it is an *exported* item: its only
/// callers are the wasm-only `component` leaf and the `#[cfg(test)]` tests below,
/// so a `pub(crate)` fn would be `dead_code` on the host non-test build. This
/// mirrors `auth::marker`'s public host-tested codec.
pub fn extract_upload_url(body: &str) -> Result<String, String> {
    let parsed: serde_json::Value =
        serde_json::from_str(body).map_err(|_| "invalid JSON in response".to_string())?;
    parsed["url"]
        .as_str()
        .map(ToString::to_string)
        .ok_or_else(|| "response JSON missing 'url' field".to_string())
}

/// Lists media items owned by the authenticated user.
#[server(endpoint = "/list_my_media")]
pub async fn list_my_media(
    source: Option<String>,
    limit: Option<u32>,
    offset: Option<u32>,
) -> WebResult<Vec<MediaItem>> {
    boundary!("list_my_media", {
        let auth = require_auth().await?;
        let media = expect_context::<Arc<dyn MediaStorage>>();

        let source_filter = source
            .as_deref()
            .map(str::parse::<MediaSource>)
            .transpose()?;

        let records = media
            .list_media(
                auth.user_id,
                source_filter.as_ref(),
                limit.unwrap_or(50),
                offset.unwrap_or(0),
            )
            .await?;

        Ok(records
            .into_iter()
            .map(|r| {
                let url = common::media::media_url(r.source.as_str(), &r.sha256, &r.filename);
                MediaItem {
                    sha256: r.sha256,
                    filename: r.filename,
                    source: r.source.to_string(),
                    content_type: r.content_type,
                    size_bytes: r.size_bytes,
                    url,
                    created_at: r.created_at.to_rfc3339(),
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
    source: String,
    force: Option<bool>,
) -> WebResult<DeleteMediaResult> {
    boundary!("delete_media", {
        let auth = require_auth().await?;
        let media = expect_context::<Arc<dyn MediaStorage>>();
        let posts = expect_context::<Arc<dyn PostStorage>>();

        let source_enum = source.parse::<MediaSource>()?;

        let url = common::media::media_url(source_enum.as_str(), &sha256, &filename);

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
            .delete_media(auth.user_id, &sha256, &filename, &source_enum)
            .await
            .map_err(InternalError::storage)?;

        Ok(DeleteMediaResult {
            deleted: true,
            referenced_in_posts,
        })
    })
}

#[cfg(test)]
mod tests {
    use super::extract_upload_url;

    #[test]
    fn extracts_url_field() {
        assert_eq!(
            extract_upload_url(r#"{"url":"/media/upload/ab/cd/hash/pic.png"}"#),
            Ok("/media/upload/ab/cd/hash/pic.png".to_string())
        );
    }

    #[test]
    fn extracts_url_ignoring_other_fields() {
        assert_eq!(
            extract_upload_url(r#"{"size_bytes":11,"url":"/media/x","content_type":"image/png"}"#),
            Ok("/media/x".to_string())
        );
    }

    #[test]
    fn rejects_missing_url_field() {
        assert_eq!(
            extract_upload_url(r#"{"size_bytes":11}"#),
            Err("response JSON missing 'url' field".to_string())
        );
    }

    #[test]
    fn rejects_non_string_url_field() {
        assert_eq!(
            extract_upload_url(r#"{"url":42}"#),
            Err("response JSON missing 'url' field".to_string())
        );
    }

    #[test]
    fn rejects_invalid_json() {
        assert_eq!(
            extract_upload_url("not json"),
            Err("invalid JSON in response".to_string())
        );
    }
}
