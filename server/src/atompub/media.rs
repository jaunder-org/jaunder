//! `AtomPub` media collection upload/fetch/delete handlers.

use std::path::PathBuf;
use std::sync::Arc;

use axum::body::Bytes;
use axum::extract::Path;
use axum::http::{header, HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::Extension;
use sha2::{Digest, Sha256};

use common::absolute_url::{compose, AbsoluteUrl};
use common::atompub::{render_media_link_entry, MediaLinkEntry};
use common::media::{media_url, ContentHash, Filename};
use common::username::Username;
use storage::{MediaRecord, MediaSource, MediaStorage, SiteConfigStorage};
use web::auth::AuthUser;

use super::{base_url, HandlerError};

const ENTRY_CONTENT_TYPE: &str = "application/atom+xml;type=entry;charset=utf-8";

/// Builds the media-link entry for a stored media record.
fn media_link_entry(
    record: &MediaRecord,
    base: Option<&AbsoluteUrl>,
    username: &Username,
) -> MediaLinkEntry {
    let binary_path = media_url("upload", &record.sha256, &record.filename);
    let binary = compose(base, &binary_path);
    let edit_path = format!(
        "/atompub/{username}/media/{}/{}",
        record.sha256, record.filename
    );
    let edit = compose(base, &edit_path);
    let timestamp = record.created_at.to_rfc3339();
    MediaLinkEntry {
        id: edit.clone(),
        title: record.filename.clone(),
        edit_uri: edit,
        edit_media_uri: binary.clone(),
        content_src: binary,
        content_type: record.content_type.clone(),
        published_rfc3339: timestamp.clone(),
        updated_rfc3339: timestamp,
    }
}

/// `POST /atompub/{username}/media` — upload a binary as a new media resource.
///
/// The `Slug` header (when present) is the requested filename. Responds `201`
/// for a new resource or `200` when identical content was already stored.
///
/// # Errors
/// `403` wrong user; `4xx`/`5xx` from the upload pipeline; `500` on storage failure.
#[tracing::instrument(name = "atompub.media.collection_post", skip_all)]
pub async fn collection_post(
    Extension(media): Extension<Arc<dyn MediaStorage>>,
    Extension(site_config): Extension<Arc<dyn SiteConfigStorage>>,
    Extension(storage_path): Extension<Arc<PathBuf>>,
    auth_user: AuthUser,
    Path(username): Path<Username>,
    headers: HeaderMap,
    body: Bytes,
) -> Result<Response, HandlerError> {
    super::require_user_match(&auth_user, &username)?;

    let raw_name = headers
        .get("slug")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("upload");
    // Door B: normalize the requested `Slug` to a safe leaf, rejecting empty as a 400.
    let filename = Filename::sanitized(raw_name).map_err(|_| HandlerError::BadRequest)?;
    let content_type = headers
        .get(header::CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .unwrap_or("application/octet-stream")
        .to_string();

    // Determine whether this exact resource already exists (idempotent re-upload).
    let sha = ContentHash::from_digest(Sha256::digest(&body).into());
    let existed = media
        .get_media(auth_user.user_id, &sha, &filename, &MediaSource::Upload)
        .await?
        .is_some();

    let manager =
        crate::media_manager::MediaManager::new(media.clone(), site_config.clone(), storage_path);
    let upload = manager
        .upload_bytes(&auth_user, &filename, &content_type, &body)
        .await?;

    let record = media
        .get_media(
            auth_user.user_id,
            &upload.sha256,
            &upload.filename,
            &MediaSource::Upload,
        )
        .await?
        .ok_or(HandlerError::Internal)?;

    let base = base_url(site_config.as_ref()).await;
    let entry = media_link_entry(&record, base.as_ref(), &username);
    let xml = render_media_link_entry(&entry);
    let status = if existed {
        StatusCode::OK
    } else {
        StatusCode::CREATED
    };

    Ok((
        status,
        [
            (header::CONTENT_TYPE, ENTRY_CONTENT_TYPE.to_string()),
            (header::LOCATION, entry.edit_uri),
        ],
        xml,
    )
        .into_response())
}

/// `GET /atompub/{username}/media/{sha}/{filename}` — fetch a media-link entry.
///
/// # Errors
/// `403` wrong user; `404` unknown; `500` on storage failure.
#[tracing::instrument(name = "atompub.media.member_get", skip_all)]
pub async fn member_get(
    Extension(media): Extension<Arc<dyn MediaStorage>>,
    Extension(site_config): Extension<Arc<dyn SiteConfigStorage>>,
    auth_user: AuthUser,
    Path((username, sha, filename)): Path<(Username, ContentHash, Filename)>,
) -> Result<Response, HandlerError> {
    super::require_user_match(&auth_user, &username)?;
    // `sha` and `filename` are parsed by the typed extractor: a malformed segment is a
    // pre-handler 400. The URL is one we minted in the media-link entry, so a bad segment
    // is the caller's fault, not a missing resource.
    let record = media
        .get_media(auth_user.user_id, &sha, &filename, &MediaSource::Upload)
        .await?
        .ok_or(HandlerError::NotFound)?;

    let base = base_url(site_config.as_ref()).await;
    let entry = media_link_entry(&record, base.as_ref(), &username);
    let xml = render_media_link_entry(&entry);
    Ok(([(header::CONTENT_TYPE, ENTRY_CONTENT_TYPE)], xml).into_response())
}

/// `DELETE /atompub/{username}/media/{sha}/{filename}` — remove a media record.
///
/// # Errors
/// `403` wrong user; `404` unknown; `500` on storage failure.
#[tracing::instrument(name = "atompub.media.member_delete", skip_all)]
pub async fn member_delete(
    Extension(media): Extension<Arc<dyn MediaStorage>>,
    auth_user: AuthUser,
    Path((username, sha, filename)): Path<(Username, ContentHash, Filename)>,
) -> Result<Response, HandlerError> {
    super::require_user_match(&auth_user, &username)?;
    // `sha` and `filename` are parsed by the typed extractor (a malformed segment is a
    // pre-handler 400); a well-formed but absent record still yields `NotFound` below.
    media
        .delete_media(auth_user.user_id, &sha, &filename, &MediaSource::Upload)
        .await?;
    Ok(StatusCode::NO_CONTENT.into_response())
}
