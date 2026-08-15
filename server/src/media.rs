use std::io;
use std::path::PathBuf;
use std::sync::Arc;

use axum::extract::{Path, Query};
use axum::http::{HeaderValue, StatusCode};
use axum::response::{IntoResponse, Redirect, Response};
use axum::routing::get;
use axum::{Extension, Router};
use serde::Deserialize;
use thiserror::Error;

use tokio::fs;
use tokio_util::io::ReaderStream;

use common::etag::ETag;
use common::ids::UserId;
use common::media::{
    ContentHash, ContentType, Filename, MediaSource, ProfferedFilename, detect_content_type,
    media_path, should_inline,
};
use storage::{MediaError, MediaStorage};
use web::auth::AuthUser;
use web::error::InternalError;

/// Builds the media routes (content-addressed serve, remote proxy). Upload lives
/// in the `web::media::upload` `#[server]` fn (#517).
///
/// The handlers read shared state via `Extension`, so the routes are generic
/// over the application's router state type.
pub fn router<S>() -> Router<S>
where
    S: Clone + Send + Sync + 'static,
{
    Router::new()
        .route(
            "/media/{source}/{p1}/{p2}/{hash}/{filename}",
            get(serve_handler),
        )
        .route("/media/proxy", get(proxy_handler))
}

// ---------------------------------------------------------------------------
// Error mapping
// ---------------------------------------------------------------------------

/// Maps a media upload `anyhow::Error` to the client-facing HTTP status. The
/// upload metric is emitted inside `storage::MediaManager`, so this is a pure map.
#[must_use]
pub fn map_error(err: &anyhow::Error) -> StatusCode {
    match err.downcast_ref::<MediaError>() {
        Some(MediaError::BadRequest(_)) => StatusCode::BAD_REQUEST,
        Some(MediaError::PayloadTooLarge) => StatusCode::PAYLOAD_TOO_LARGE,
        Some(MediaError::InsufficientStorage) => StatusCode::INSUFFICIENT_STORAGE,
        Some(MediaError::Internal(_)) | None => StatusCode::INTERNAL_SERVER_ERROR,
    }
}

/// Pure classification of a media-file open failure.
#[derive(Debug, Error)]
pub enum MediaOpenError {
    #[error("media file not found")]
    NotFound,
    #[error("failed to open media file")]
    Internal(#[source] io::Error),
}

impl MediaOpenError {
    /// The public status selected by the open-error classification.
    #[must_use]
    pub fn status(&self) -> StatusCode {
        match self {
            Self::NotFound => StatusCode::NOT_FOUND,
            Self::Internal(_) => StatusCode::INTERNAL_SERVER_ERROR,
        }
    }

    /// Emits the internal branch at the raw-Axum boundary. Expected absence is
    /// represented by `NotFound` and intentionally produces no error event.
    pub fn emit_boundary_failure(self) {
        if let Self::Internal(error) = self {
            InternalError::server(error)
                .with_context("boundary", "server.media.open")
                .emit_boundary_failure();
        }
    }
}

/// Maps only `io::ErrorKind::NotFound` to public absence. Every other I/O error
/// remains typed for the internal 500 boundary.
#[must_use]
pub fn classify_media_open_error(error: io::Error) -> MediaOpenError {
    if error.kind() == io::ErrorKind::NotFound {
        MediaOpenError::NotFound
    } else {
        MediaOpenError::Internal(error)
    }
}

// ---------------------------------------------------------------------------
// Serve handler  GET /media/{source}/{p1}/{p2}/{hash}/{filename}
// ---------------------------------------------------------------------------

/// Fully validated path address for the public content-addressed media route.
#[derive(Deserialize)]
struct RawServeAddress {
    source: MediaSource,
    p1: String,
    p2: String,
    hash: ContentHash,
    /// Axum has percent-decoded this segment; this door restores its canonical encoding.
    filename: ProfferedFilename,
}

/// Fully validated path address for the public content-addressed media route.
struct ServeAddress {
    source: MediaSource,
    hash: ContentHash,
    filename: Filename,
}

impl<'de> Deserialize<'de> for ServeAddress {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let RawServeAddress {
            source,
            p1,
            p2,
            hash,
            filename,
        } = RawServeAddress::deserialize(deserializer)?;

        if p1 != hash[..2] || p2 != hash[2..4] {
            return Err(serde::de::Error::custom("media hash prefixes do not match"));
        }

        Ok(Self {
            source,
            hash,
            filename: Filename::from(filename),
        })
    }
}

/// Serves a stored media file.
///
/// # Errors
///
/// Returns `4xx` status codes for a valid but missing file.
#[tracing::instrument(name = "media.serve", skip_all)]
async fn serve_handler(
    media: Extension<Arc<dyn MediaStorage>>,
    storage_path: Extension<Arc<PathBuf>>,
    Path(address): Path<ServeAddress>,
    req_headers: axum::http::HeaderMap,
) -> Result<Response, StatusCode> {
    let result = serve_response(media, storage_path, address, req_headers).await;
    if let Some(outcome) = serve_result(&result) {
        host::metrics::media_served(outcome);
    }
    result
}

/// Maps a serve outcome to its bounded `result` attribute.
fn serve_result(result: &Result<Response, StatusCode>) -> Option<host::metrics::ServeResult> {
    match result {
        Ok(response) if response.status() == StatusCode::NOT_MODIFIED => {
            Some(host::metrics::ServeResult::NotModified)
        }
        Ok(_) => Some(host::metrics::ServeResult::Ok),
        Err(status) if *status == StatusCode::NOT_FOUND => {
            Some(host::metrics::ServeResult::NotFound)
        }
        Err(_) => None,
    }
}

/// Serves a stored media file with long-lived cache headers and `ETag` support.
async fn serve_response(
    Extension(media): Extension<Arc<dyn MediaStorage>>,
    Extension(storage_path): Extension<Arc<PathBuf>>,
    address: ServeAddress,
    req_headers: axum::http::HeaderMap,
) -> Result<Response, StatusCode> {
    let (source, hash, filename, file_path) = resolve_media_path(&storage_path, address);
    let file = match fs::File::open(&file_path).await {
        Ok(file) => file,
        Err(error) => {
            let error = classify_media_open_error(error);
            let status = error.status();
            error.emit_boundary_failure();
            return Err(status);
        }
    };

    // ETag / If-None-Match check.
    let etag = ETag::from_content_hash(&hash);
    if let Some(if_none_match) = req_headers.get(axum::http::header::IF_NONE_MATCH) {
        // `ETag: PartialEq<&str>` (the reverse `str: PartialEq<ETag>` isn't derived).
        if etag == if_none_match.to_str().unwrap_or("") {
            return Ok(StatusCode::NOT_MODIFIED.into_response());
        }
    }

    // Look up content_type from DB; fall back to extension detection.
    let content_type = media
        .find_by_hash(&hash, &source)
        .await
        .map_err(serve_internal_error)?
        // Both read inside the typed filename, which decodes only at the display
        // boundary. Extension detection must not inspect its encoded spelling.
        .map_or_else(|| detect_content_type(&filename), |r| r.content_type);

    let disposition = content_disposition(&content_type, &filename);

    let stream = ReaderStream::new(file);
    let body = axum::body::Body::from_stream(stream);

    let mut response = Response::new(body);
    let headers = response.headers_mut();

    headers.insert(
        axum::http::header::CONTENT_TYPE,
        // A `ContentType` is always a valid header value (its invariant), so — like the
        // sibling etag/disposition inserts below — the `Err` arm is unreachable (#495).
        HeaderValue::from_str(content_type.as_ref())
            .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?,
    );
    headers.insert(
        axum::http::header::CACHE_CONTROL,
        HeaderValue::from_static("public, max-age=31536000, immutable"),
    );
    headers.insert(
        axum::http::header::ETAG,
        HeaderValue::from_str(etag.as_ref()).map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?,
    );
    headers.insert(
        axum::http::header::CONTENT_DISPOSITION,
        HeaderValue::from_str(&disposition).map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?,
    );

    Ok(response)
}

// ---------------------------------------------------------------------------
// Proxy handler stub  GET /media/proxy
// ---------------------------------------------------------------------------

/// Query parameters for the proxy route.
#[derive(Deserialize)]
pub struct ProxyParams {
    pub url: String,
    pub user_id: UserId,
}

/// Stub proxy handler: redirects to the remote URL.
///
/// Full caching implementation is deferred to a future milestone.
///
/// # Errors
///
/// Returns 401 if the authenticated user does not match `user_id`.
#[tracing::instrument(name = "media.proxy", skip_all)]
pub async fn proxy_handler(
    auth_user: AuthUser,
    Query(params): Query<ProxyParams>,
) -> Result<Redirect, StatusCode> {
    if auth_user.user_id != params.user_id {
        return Err(StatusCode::UNAUTHORIZED);
    }
    // TODO(M9/M17): implement actual fetch, cache, and redirect to local URL
    Ok(Redirect::temporary(&params.url))
}

// ---------------------------------------------------------------------------
// Internal helpers
// ---------------------------------------------------------------------------
/// Resolves the already-validated media address into its storage path.
fn resolve_media_path(
    storage_path: &std::path::Path,
    address: ServeAddress,
) -> (MediaSource, ContentHash, Filename, PathBuf) {
    let file_path = storage_path.join("media").join(media_path(
        &address.source,
        &address.hash,
        &address.filename,
    ));

    (address.source, address.hash, address.filename, file_path)
}

/// Builds a header-safe `Content-Disposition` value for a typed content type and filename.
///
/// The decoded filename is only for this display header; its canonical spelling remains on
/// the storage and URL paths. `inline` vs `attachment` follows [`should_inline`].
fn content_disposition(content_type: &ContentType, filename: &Filename) -> String {
    use percent_encoding::{NON_ALPHANUMERIC, utf8_percent_encode};

    let disposition = if should_inline(content_type) {
        "inline"
    } else {
        "attachment"
    };

    let decoded = filename.decoded();
    let mut fallback = String::with_capacity(decoded.len());
    for c in decoded.chars() {
        if !c.is_ascii() || c.is_control() {
            continue;
        }
        if c == '"' || c == '\\' {
            fallback.push('\\');
        }
        fallback.push(c);
    }

    let encoded = utf8_percent_encode(&decoded, NON_ALPHANUMERIC);
    format!("{disposition}; filename=\"{fallback}\"; filename*=UTF-8''{encoded}")
}

/// Logs a genuine media-serve internal failure (a storage lookup error) and maps
/// it to `500`. Without this the error was discarded, producing a blank 500 with
/// nothing logged. The error is infrastructure detail, not user content, so it
/// carries no PII.
fn serve_internal_error<E: std::error::Error>(err: E) -> StatusCode {
    tracing::error!(error = %err, "media serve internal error");
    StatusCode::INTERNAL_SERVER_ERROR
}

#[cfg(test)]
mod tests {
    use super::*;
    use common::test_support::{parse_content_hash, parse_content_type};
    use std::path::Path;

    #[test]
    fn serve_result_maps_each_outcome() {
        use host::metrics::ServeResult;
        let ok: Result<Response, StatusCode> = Ok(StatusCode::OK.into_response());
        assert!(matches!(serve_result(&ok), Some(ServeResult::Ok)));
        let not_modified: Result<Response, StatusCode> =
            Ok(StatusCode::NOT_MODIFIED.into_response());
        assert!(matches!(
            serve_result(&not_modified),
            Some(ServeResult::NotModified)
        ));
        let not_found: Result<Response, StatusCode> = Err(StatusCode::NOT_FOUND);
        assert!(matches!(
            serve_result(&not_found),
            Some(ServeResult::NotFound)
        ));
        let internal: Result<Response, StatusCode> = Err(StatusCode::INTERNAL_SERVER_ERROR);
        assert!(serve_result(&internal).is_none());
    }

    #[test]
    fn serve_internal_error_maps_to_500() {
        assert_eq!(
            serve_internal_error(sqlx::Error::PoolClosed),
            StatusCode::INTERNAL_SERVER_ERROR
        );
    }

    #[test]
    fn map_error_maps_each_media_error() {
        assert_eq!(
            map_error(&anyhow::anyhow!(MediaError::BadRequest("bad".to_owned()))),
            StatusCode::BAD_REQUEST
        );
        assert_eq!(
            map_error(&anyhow::anyhow!(MediaError::PayloadTooLarge)),
            StatusCode::PAYLOAD_TOO_LARGE
        );
        assert_eq!(
            map_error(&anyhow::anyhow!(MediaError::InsufficientStorage)),
            StatusCode::INSUFFICIENT_STORAGE
        );
        assert_eq!(
            map_error(&anyhow::anyhow!(MediaError::Internal(Box::new(
                std::io::Error::other("error"),
            )))),
            StatusCode::INTERNAL_SERVER_ERROR
        );
        assert_eq!(
            map_error(&anyhow::anyhow!("unknown")),
            StatusCode::INTERNAL_SERVER_ERROR
        );
    }

    #[test]
    fn resolve_media_path_builds_path_for_valid_address() {
        let hash: ContentHash = "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855"
            .parse()
            .unwrap();
        let address = ServeAddress {
            source: MediaSource::Upload,
            hash: hash.clone(),
            filename: Filename::sanitized("photo.jpg").unwrap(),
        };

        let (source, resolved_hash, _filename, path) =
            resolve_media_path(Path::new("/data"), address);

        assert_eq!(source, MediaSource::Upload);
        assert_eq!(resolved_hash, hash);
        assert_eq!(
            path,
            Path::new("/data")
                .join("media")
                .join("upload")
                .join("e3")
                .join("b0")
                .join(hash.as_ref())
                .join("photo.jpg")
        );
    }

    #[test]
    fn content_disposition_carries_decoded_and_rfc5987_forms() {
        // The argument is the *decoded* name (#720). The helper's own `NON_ALPHANUMERIC`
        // encode is the RFC 5987 one and is a different set from the media segment's —
        // both correct in place. Handing it the already-encoded name would double-encode
        // into a header that still looks well-formed, which is exactly the failure class
        // this issue exists to remove, so both parameters are pinned as exact strings.
        let value = content_disposition(
            &"image/png".parse().unwrap(),
            &Filename::sanitized("my photo.jpg").unwrap(),
        );
        assert!(value.contains("filename=\"my photo.jpg\""), "{value}");
        // Note `%2E`, not `.`: the RFC 5987 parameter uses the **bare** `NON_ALPHANUMERIC`
        // set, which encodes the unreserved marks the media path segment deliberately
        // keeps. The two sets genuinely differ and are each correct in place (ADR-0080);
        // pinning the exact string here is what stops one being swapped for the other.
        assert!(
            value.contains("filename*=UTF-8''my%20photo%2Ejpg"),
            "{value}"
        );
        assert!(!value.contains("%2520"), "double-encoded: {value}");
    }

    #[test]
    fn content_disposition_picks_inline_or_attachment_by_type() {
        assert!(
            content_disposition(
                &"image/png".parse().unwrap(),
                &Filename::sanitized("p.png").unwrap()
            )
            .starts_with("inline; ")
        );
        assert!(
            content_disposition(
                &"application/octet-stream".parse().unwrap(),
                &Filename::sanitized("p.bin").unwrap()
            )
            .starts_with("attachment; ")
        );
    }

    #[test]
    fn content_disposition_escapes_quotes_and_strips_control_chars() {
        // A quote in the name must be backslash-escaped, never break the
        // quoted-string; control chars are dropped from the ASCII fallback.
        let value = content_disposition(
            &"application/octet-stream".parse().unwrap(),
            &Filename::sanitized("a\"b\n.txt").unwrap(),
        );
        assert!(
            value.contains(r#"filename="a\"b.txt""#),
            "fallback not escaped/stripped: {value}"
        );
        assert!(!value.contains('\n'), "control char leaked: {value:?}");
        // Header construction must succeed (all-ASCII, no controls).
        assert!(HeaderValue::from_str(&value).is_ok());
    }

    #[test]
    fn content_disposition_percent_encodes_non_ascii_in_filename_star() {
        let value = content_disposition(
            &"image/png".parse().unwrap(),
            &Filename::sanitized("café.png").unwrap(),
        );
        // Non-ASCII dropped from the ASCII fallback...
        assert!(value.contains(r#"filename="caf.png""#), "{value}");
        // ...but carried, percent-encoded, in filename*.
        assert!(value.contains("filename*=UTF-8''caf%C3%A9"), "{value}");
        assert!(HeaderValue::from_str(&value).is_ok());
    }

    const SAMPLE_HASH: &str = "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855";

    /// Materializes a stored media file under a fresh temp storage root and
    /// returns the root plus the matching strict serve address.
    fn stored_file(filename: &str) -> (tempfile::TempDir, ServeAddress) {
        let temp = tempfile::TempDir::new().expect("temp dir");
        let dir = temp
            .path()
            .join("media")
            .join("upload")
            .join("e3")
            .join("b0")
            .join(SAMPLE_HASH);
        std::fs::create_dir_all(&dir).expect("create media dirs");
        std::fs::write(dir.join(filename), b"file-bytes").expect("write file");
        (
            temp,
            ServeAddress {
                source: MediaSource::Upload,
                hash: SAMPLE_HASH.parse().unwrap(),
                filename: Filename::sanitized(filename).unwrap(),
            },
        )
    }

    // guard:no-backend — mock store
    #[tokio::test]
    async fn serve_response_returns_304_on_matching_if_none_match() {
        let (temp, address) = stored_file("photo.jpg");
        let media = storage::MockMediaStorage::new();
        let etag = ETag::from_content_hash(&parse_content_hash(SAMPLE_HASH));
        let mut headers = axum::http::HeaderMap::new();
        headers.insert(
            axum::http::header::IF_NONE_MATCH,
            HeaderValue::from_str(etag.as_ref()).unwrap(),
        );

        let response = serve_response(
            Extension(Arc::new(media) as Arc<dyn MediaStorage>),
            Extension(Arc::new(temp.path().to_path_buf())),
            address,
            headers,
        )
        .await
        .expect("serve response");

        assert_eq!(response.status(), StatusCode::NOT_MODIFIED);
    }

    // guard:no-backend — mock store
    #[tokio::test]
    async fn serve_response_serves_body_when_if_none_match_does_not_match() {
        let (temp, address) = stored_file("photo.jpg");
        let mut media = storage::MockMediaStorage::new();
        media
            .expect_find_by_hash()
            .times(1)
            .returning(|_, _| Ok(None));
        let mut headers = axum::http::HeaderMap::new();
        headers.insert(
            axum::http::header::IF_NONE_MATCH,
            HeaderValue::from_static("\"not-the-hash\""),
        );

        let response = serve_response(
            Extension(Arc::new(media) as Arc<dyn MediaStorage>),
            Extension(Arc::new(temp.path().to_path_buf())),
            address,
            headers,
        )
        .await
        .expect("serve response");

        assert_eq!(response.status(), StatusCode::OK);
    }

    // guard:no-backend — mock store
    #[tokio::test]
    async fn serve_response_falls_back_to_extension_content_type_when_db_has_no_record() {
        let (temp, address) = stored_file("photo.jpg");
        let mut media = storage::MockMediaStorage::new();
        media
            .expect_find_by_hash()
            .times(1)
            .returning(|_, _| Ok(None));

        let response = serve_response(
            Extension(Arc::new(media) as Arc<dyn MediaStorage>),
            Extension(Arc::new(temp.path().to_path_buf())),
            address,
            axum::http::HeaderMap::new(),
        )
        .await
        .expect("serve response");

        assert_eq!(response.status(), StatusCode::OK);
        let expected = detect_content_type(&Filename::sanitized("photo.jpg").unwrap());
        assert_eq!(
            response
                .headers()
                .get(axum::http::header::CONTENT_TYPE)
                .and_then(|value| value.to_str().ok()),
            Some(expected.as_ref())
        );
    }

    #[test]
    fn every_accepted_content_type_is_header_constructible() {
        // The D4 invariant — every accepted content type is header-constructible —
        // observed against the real `HeaderValue::from_str` oracle (#495).
        for s in [
            "image/png",
            "text/html; charset=utf-8",
            "application/octet-stream",
        ] {
            let ct = parse_content_type(s);
            assert!(
                HeaderValue::from_str(ct.as_ref()).is_ok(),
                "header value for {s:?}"
            );
        }
    }
}
