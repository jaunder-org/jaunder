//! `AtomPub` media collection upload/fetch/delete handlers.

use std::path::PathBuf;
use std::sync::Arc;

use axum::body::Bytes;
use axum::extract::Path;
use axum::http::{header, HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::Extension;
use sha2::{Digest, Sha256};

use common::atompub::{render_media_link_entry, MediaLinkEntry};
use common::media::{media_url, sanitize_filename};
use storage::{AppState, MediaRecord, MediaSource};
use web::auth::AuthUser;

use super::base_url;

const ENTRY_CONTENT_TYPE: &str = "application/atom+xml;type=entry;charset=utf-8";

/// Builds the media-link entry for a stored media record.
fn media_link_entry(record: &MediaRecord, base: &str, username: &str) -> MediaLinkEntry {
    let binary = format!(
        "{base}{}",
        media_url("upload", &record.sha256, &record.filename)
    );
    let edit = format!(
        "{base}/atompub/{username}/media/{}/{}",
        record.sha256, record.filename
    );
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
/// `403` wrong user; `4xx`/`5xx` from the upload pipeline; `500` on storage/serialization failure.
pub async fn collection_post(
    Extension(state): Extension<Arc<AppState>>,
    Extension(storage_path): Extension<Arc<PathBuf>>,
    auth_user: AuthUser,
    Path(username): Path<String>,
    headers: HeaderMap,
    body: Bytes,
) -> Result<Response, StatusCode> {
    super::require_user_match(&auth_user, &username)?;

    let raw_name = headers
        .get("slug")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("upload");
    let filename = sanitize_filename(raw_name);
    if filename.is_empty() {
        return Err(StatusCode::BAD_REQUEST);
    }
    let content_type = headers
        .get(header::CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .unwrap_or("application/octet-stream")
        .to_string();

    // Determine whether this exact resource already exists (idempotent re-upload).
    let sha = format!("{:x}", Sha256::digest(&body));
    let existed = state
        .media
        .get_media(auth_user.user_id, &sha, &filename, &MediaSource::Upload)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?
        .is_some();

    let manager = crate::media_manager::MediaManager::new(state.clone(), storage_path);
    let upload = manager
        .upload_bytes(&auth_user, &filename, &content_type, &body)
        .await
        .map_err(|e| crate::media_manager::MediaManager::map_error(&e))?;

    let record = state
        .media
        .get_media(
            auth_user.user_id,
            &upload.sha256,
            &upload.filename,
            &MediaSource::Upload,
        )
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?
        .ok_or(StatusCode::INTERNAL_SERVER_ERROR)?;

    let base = base_url(&state).await;
    let entry = media_link_entry(&record, &base, &username);
    let xml = render_media_link_entry(&entry).map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
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
/// `403` wrong user; `404` unknown; `500` on storage/serialization failure.
pub async fn member_get(
    Extension(state): Extension<Arc<AppState>>,
    auth_user: AuthUser,
    Path((username, sha, filename)): Path<(String, String, String)>,
) -> Result<Response, StatusCode> {
    super::require_user_match(&auth_user, &username)?;
    let record = state
        .media
        .get_media(auth_user.user_id, &sha, &filename, &MediaSource::Upload)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?
        .ok_or(StatusCode::NOT_FOUND)?;

    let base = base_url(&state).await;
    let entry = media_link_entry(&record, &base, &username);
    let xml = render_media_link_entry(&entry).map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    Ok(([(header::CONTENT_TYPE, ENTRY_CONTENT_TYPE)], xml).into_response())
}

/// `DELETE /atompub/{username}/media/{sha}/{filename}` — remove a media record.
///
/// # Errors
/// `403` wrong user; `404` unknown; `500` on storage failure.
pub async fn member_delete(
    Extension(state): Extension<Arc<AppState>>,
    auth_user: AuthUser,
    Path((username, sha, filename)): Path<(String, String, String)>,
) -> Result<Response, StatusCode> {
    super::require_user_match(&auth_user, &username)?;
    state
        .media
        .delete_media(auth_user.user_id, &sha, &filename, &MediaSource::Upload)
        .await
        .map_err(|e| delete_status(&e))?;
    Ok(StatusCode::NO_CONTENT.into_response())
}

/// Maps a `DeleteMediaError` to the appropriate HTTP status code.
fn delete_status(err: &storage::DeleteMediaError) -> StatusCode {
    match err {
        storage::DeleteMediaError::NotFound => StatusCode::NOT_FOUND,
        storage::DeleteMediaError::Internal(_) => StatusCode::INTERNAL_SERVER_ERROR,
    }
}

#[cfg(test)]
mod tests {
    use super::delete_status;
    use axum::http::StatusCode;
    use storage::DeleteMediaError;

    #[test]
    fn delete_status_maps_not_found_and_internal() {
        assert_eq!(
            delete_status(&DeleteMediaError::NotFound),
            StatusCode::NOT_FOUND
        );
        assert_eq!(
            delete_status(&DeleteMediaError::Internal(sqlx::Error::PoolClosed)),
            StatusCode::INTERNAL_SERVER_ERROR
        );
    }
}
