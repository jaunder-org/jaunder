//! `AtomPub` media collection upload/fetch/delete handlers.

use std::path::PathBuf;
use std::sync::Arc;

use axum::Extension;
use axum::body::Bytes;
use axum::extract::Path;
use axum::http::{HeaderMap, StatusCode, header};
use axum::response::{IntoResponse, Response};
use serde::Deserialize;
use sha2::{Digest, Sha256};

use common::atompub::{MediaLinkEntry, render_media_link_entry};
use common::media::{ContentHash, Filename, MediaRef, MediaSource, media_url};
use common::root_relative_url::RootRelativeUrl;
use common::tagged_url::{BaseUrl, EditMediaUriUrl, EditUriUrl, compose};
use common::username::Username;
use storage::{
    InstanceId, MediaManager, MediaRecord, MediaReferenceOwnershipResolver, MediaStorage,
    PostStorage, SiteConfigStorage, resolve_media_reference_ownership,
};
use web::auth;

use super::{HandlerError, required_base_url};

const ENTRY_CONTENT_TYPE: &str = "application/atom+xml;type=entry;charset=utf-8";

type MemberDeleteExtensions = (
    Extension<Arc<dyn MediaStorage>>,
    Extension<Arc<dyn PostStorage>>,
    Extension<Arc<dyn SiteConfigStorage>>,
    Extension<Arc<PathBuf>>,
    Extension<InstanceId>,
    Extension<Arc<dyn MediaReferenceOwnershipResolver>>,
);

/// Builds the media-link entry for a stored media record.
fn media_link_entry(record: &MediaRecord, base: &BaseUrl, username: &Username) -> MediaLinkEntry {
    let binary_path = media_url(&MediaSource::Upload, &record.sha256, &record.filename);
    let binary: EditMediaUriUrl = compose(base, &binary_path);
    // The member URL is a *different* layout from the serve path (it is the AtomPub
    // collection's, not the content-addressed store's), so it is built here rather than by
    // `media_path`. The filename needs no encoding at either site (#720): a `Filename`
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
        published: record.created_at,
        updated: record.created_at,
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
    auth_user: auth::User,
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
    let content_type = if let Some(value) = headers.get(header::CONTENT_TYPE) {
        value
            .to_str()
            .ok()
            .and_then(|value| value.parse().ok())
            .ok_or_else(|| {
                host::metrics::media_upload(host::metrics::UploadOutcome::Invalid);
                HandlerError::BadRequest
            })?
    } else {
        "application/octet-stream"
            .parse()
            .map_err(|_| HandlerError::Invariant)?
    };

    // Determine whether this exact resource already exists (idempotent re-upload).
    let sha = ContentHash::from_digest(Sha256::digest(&body).into());
    let existed = media
        .get_media(auth_user.user_id, &sha, &filename, &MediaSource::Upload)
        .await?
        .is_some();

    let manager = storage::MediaManager::new(media.clone(), site_config.clone(), storage_path);
    let upload = manager
        .upload_bytes(auth_user.user_id, &filename, content_type, &body)
        .await?;

    let record = media
        .get_media(
            auth_user.user_id,
            &upload.sha256,
            &upload.filename,
            &MediaSource::Upload,
        )
        .await?
        .ok_or(HandlerError::Invariant)?;

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

#[derive(Deserialize)]
struct RawMediaMemberAddress {
    username: Username,
    sha: ContentHash,
    filename: String,
}

pub(super) struct MediaMemberAddress {
    username: Username,
    sha: ContentHash,
    filename: Filename,
}

impl<'de> Deserialize<'de> for MediaMemberAddress {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let RawMediaMemberAddress {
            username,
            sha,
            filename,
        } = RawMediaMemberAddress::deserialize(deserializer)?;
        let filename =
            Filename::from_decoded_segment(&filename).map_err(serde::de::Error::custom)?;

        Ok(Self {
            username,
            sha,
            filename,
        })
    }
}

/// `GET /atompub/{username}/media/{sha}/{filename}` — fetch a media-link entry.
///
/// # Errors
/// `403` wrong user; `404` unknown; `500` on storage failure.
#[tracing::instrument(name = "atompub.media.member_get", skip_all)]
pub(super) async fn member_get(
    Extension(media): Extension<Arc<dyn MediaStorage>>,
    Extension(site_config): Extension<Arc<dyn SiteConfigStorage>>,
    auth_user: auth::User,
    Path(address): Path<MediaMemberAddress>,
) -> Result<Response, HandlerError> {
    let MediaMemberAddress {
        username,
        sha,
        filename,
    } = address;
    super::require_user_match(&auth_user, &username)?;
    // The private address extractor parses `sha` and converts the Axum-decoded
    // filename segment back to canonical storage spelling before handler logic runs.
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
pub(super) async fn member_delete(
    (
        Extension(media),
        Extension(posts),
        Extension(site_config),
        Extension(storage_path),
        Extension(instance_id),
        Extension(resolver),
    ): MemberDeleteExtensions,
    auth_user: auth::User,
    Path(address): Path<MediaMemberAddress>,
) -> Result<Response, HandlerError> {
    let MediaMemberAddress {
        username,
        sha,
        filename,
    } = address;
    super::require_user_match(&auth_user, &username)?;
    // The private address extractor rejects malformed segments before handler logic;
    // a well-formed but absent record still yields `NotFound` below.
    // `force = true`: AtomPub has no confirmation UI. The storage guard still refuses
    // deletes that would leave referenced bytes without any remaining media row (#721).
    let media_ref = MediaRef {
        source: MediaSource::Upload,
        sha256: sha,
        filename,
    };
    // Take one identity/global-reference snapshot, resolve network ownership
    // before storage locking, and carry the resulting evidence through both
    // forced deletion and potential last-row reclamation.
    let identity = site_config.get_identity().await?;
    let references = posts.list_media_references(&media_ref).await?;
    let evidence = resolve_media_reference_ownership(
        resolver.as_ref(),
        references.references(),
        &instance_id,
        identity.base_url.as_ref(),
    )
    .await;
    let manager = MediaManager::new(media, site_config, storage_path);
    let outcome = manager
        .delete_media(auth_user.user_id, &media_ref, &instance_id, &evidence, true)
        .await
        .map_err(map_delete_error)?;
    if outcome == storage::TryDeleteOutcome::RefusedReferenced {
        return Err(StatusCode::CONFLICT.into());
    }
    Ok(StatusCode::NO_CONTENT.into_response())
}

fn map_delete_error(err: anyhow::Error) -> HandlerError {
    match err.downcast::<storage::DeleteMediaError>() {
        Ok(storage::DeleteMediaError::NotFound) => HandlerError::NotFound,
        Ok(error) => HandlerError::Internal(Box::new(error)),
        Err(err) => err.into(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn map_delete_error_preserves_storage_internal_errors() {
        let error =
            map_delete_error(storage::DeleteMediaError::Internal(sqlx::Error::RowNotFound).into());

        assert!(matches!(error, HandlerError::Internal(_)));
    }

    #[test]
    fn map_delete_error_preserves_non_delete_errors() {
        let error = map_delete_error(anyhow::anyhow!("media delete failed"));

        assert!(matches!(
            error,
            HandlerError::Status(StatusCode::INTERNAL_SERVER_ERROR)
        ));
    }
}
