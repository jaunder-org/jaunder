//! `AtomPub` media collection upload/fetch/delete handlers.

use std::path::PathBuf;
use std::sync::Arc;

use axum::Extension;
use axum::body::Bytes;
use axum::extract::Path;
use axum::http::{HeaderMap, StatusCode, header};
use axum::response::{IntoResponse, Response};
use sha2::{Digest, Sha256};

use common::atompub::{MediaLinkEntry, render_media_link_entry};
use common::media::{ContentHash, Filename, MediaRef, MediaSource, ProfferedFilename, media_url};
use common::root_relative_url::RootRelativeUrl;
use common::tagged_url::{BaseUrl, EditMediaUriUrl, EditUriUrl, compose};
use common::time::UtcInstant;
use common::username::Username;
use storage::{MediaRecord, MediaStorage, SiteConfigStorage};
use web::auth::AuthUser;

use super::{HandlerError, required_base_url};

const ENTRY_CONTENT_TYPE: &str = "application/atom+xml;type=entry;charset=utf-8";

/// Builds the media-link entry for a stored media record.
fn media_link_entry(record: &MediaRecord, base: &BaseUrl, username: &Username) -> MediaLinkEntry {
    let binary_path = media_url(&MediaSource::Upload, &record.sha256, &record.filename);
    let binary: EditMediaUriUrl = compose(base, &binary_path);
    // The member URL is a *different* layout from the serve path (it is the AtomPub
    // collection's, not the content-addressed store's), so it is built here rather than by
    // `media_path`. Since #720 the filename needs no encoding at either site: a `Filename`
    // *is* the canonical percent-encoded segment, so this interpolates it verbatim and the
    // two layouts cannot spell one file differently. This URL is also the entry's
    // `atom:id`, so a malformed spelling would be the entry's permanent identity.
    //
    // Typed like `binary_path` rather than left a bare `String`: two spellings of the same
    // concept side by side is how one of them drifts.
    let edit_path: RootRelativeUrl = {
        let path = format!(
            "/atompub/{username}/media/{}/{}",
            record.sha256, record.filename
        );
        let Ok(url) = path.parse() else {
            // Unreachable: a leading `/`, a hex digest, a validated `Username`, and a
            // percent-encoded filename — no whitespace or delimiter can survive.
            unreachable!("the AtomPub media member path is a valid root-relative path");
        };
        url
    };
    let edit: EditUriUrl = compose(base, &edit_path);
    let timestamp = UtcInstant::from(record.created_at);
    MediaLinkEntry {
        // The member URL *is* the entry's atom:id — the edit URI is the canonical
        // identifier in the AtomPub member representation.
        id: edit.clone().retag(),
        title: record.filename.clone(),
        edit_uri: edit,
        edit_media_uri: binary.clone(),
        // The content source *is* the media binary; one resource, two link rels.
        content_src: binary.retag(),
        content_type: record.content_type.clone(),
        published: timestamp,
        updated: timestamp,
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

    let manager = storage::MediaManager::new(media.clone(), site_config.clone(), storage_path);
    let upload = manager
        .upload_bytes(auth_user.user_id, &filename, &content_type, &body)
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

    let base = required_base_url(site_config.as_ref()).await?;
    let entry = media_link_entry(&record, &base, &username);
    let xml = render_media_link_entry(&entry)?;
    let status = if existed {
        StatusCode::OK
    } else {
        StatusCode::CREATED
    };

    Ok((
        status,
        [
            (header::CONTENT_TYPE, ENTRY_CONTENT_TYPE.to_string()),
            (header::LOCATION, entry.edit_uri.to_string()),
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
    Path((username, sha, filename)): Path<(Username, ContentHash, ProfferedFilename)>,
) -> Result<Response, HandlerError> {
    super::require_user_match(&auth_user, &username)?;
    // `sha` and `filename` are parsed by the typed extractor: a malformed segment is a
    // pre-handler 400. The URL is one we minted in the media-link entry, so a bad segment
    // is the caller's fault, not a missing resource.
    //
    // The filename arrives percent-*decoded* (axum decodes path parameters), so it comes
    // in through the proffered door and is rewrapped here into the stored spelling (#720).
    let filename = Filename::from(filename);
    let record = media
        .get_media(auth_user.user_id, &sha, &filename, &MediaSource::Upload)
        .await?
        .ok_or(HandlerError::NotFound)?;

    let base = required_base_url(site_config.as_ref()).await?;
    let entry = media_link_entry(&record, &base, &username);
    let xml = render_media_link_entry(&entry)?;
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
    Path((username, sha, filename)): Path<(Username, ContentHash, ProfferedFilename)>,
) -> Result<Response, HandlerError> {
    super::require_user_match(&auth_user, &username)?;
    // `sha` and `filename` are parsed by the typed extractor (a malformed segment is a
    // pre-handler 400); a well-formed but absent record still yields `NotFound` below.
    // As in `member_get`, the segment arrives decoded and is rewrapped here (#720).
    let filename = Filename::from(filename);
    // `force = true`: AtomPub has no confirmation UI to carry a refusal back to the
    // client, and this endpoint's behaviour today is an unconditional delete, which
    // the guard must not change (#711).
    media
        .try_delete_media(
            auth_user.user_id,
            &MediaRef {
                source: MediaSource::Upload,
                sha256: sha,
                filename,
            },
            true,
        )
        .await?;
    Ok(StatusCode::NO_CONTENT.into_response())
}
