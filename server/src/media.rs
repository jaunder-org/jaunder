use std::path::PathBuf;
use std::sync::Arc;

use axum::extract::{Multipart, Path, Query};
use axum::http::{HeaderValue, StatusCode};
use axum::response::{IntoResponse, Redirect, Response};
use axum::routing::{get, post};
use axum::{Extension, Json, Router};
use serde::Deserialize;
use tokio::fs;
use tokio_util::io::ReaderStream;

use common::ids::UserId;
use common::media::{detect_content_type, should_inline, ContentHash, Filename, MediaSource};
use storage::{MediaStorage, SiteConfigStorage};
use web::auth::AuthUser;

use crate::soft_path::SoftPath;

/// Builds the media routes (upload, content-addressed serve, remote proxy).
///
/// The handlers read shared state via `Extension`, so the routes are generic
/// over the application's router state type.
pub fn router<S>() -> Router<S>
where
    S: Clone + Send + Sync + 'static,
{
    Router::new()
        .route("/media/upload", post(upload_handler))
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
    match err.downcast_ref::<storage::MediaError>() {
        Some(storage::MediaError::BadRequest(_)) => StatusCode::BAD_REQUEST,
        Some(storage::MediaError::PayloadTooLarge) => StatusCode::PAYLOAD_TOO_LARGE,
        Some(storage::MediaError::InsufficientStorage) => StatusCode::INSUFFICIENT_STORAGE,
        Some(storage::MediaError::Internal(_)) | None => StatusCode::INTERNAL_SERVER_ERROR,
    }
}

// ---------------------------------------------------------------------------
// Upload handler  POST /media/upload
// ---------------------------------------------------------------------------

/// Accepts a multipart upload, stores the file content-addressed under
/// `<storage_path>/media/upload/`, deduplicates via hard-links, inserts a DB
/// record, and returns 201 JSON.
///
/// # Errors
///
/// Returns `4xx`/`5xx` status codes on validation failures or I/O errors.
#[tracing::instrument(name = "media.upload", skip_all)]
pub async fn upload_handler(
    Extension(media): Extension<Arc<dyn MediaStorage>>,
    Extension(site_config): Extension<Arc<dyn SiteConfigStorage>>,
    Extension(storage_path): Extension<Arc<PathBuf>>,
    auth_user: AuthUser,
    mut multipart: Multipart,
) -> Result<Response, StatusCode> {
    let Some(field) = multipart
        .next_field()
        .await
        .map_err(|_| StatusCode::BAD_REQUEST)?
    else {
        return Err(StatusCode::BAD_REQUEST);
    };

    // Extract the field metadata before the field is consumed as the byte stream
    // (`Field` borrows `file_name()`/`content_type()` from itself, so those borrows
    // must end before it is moved into `upload`).
    // cov:ignore-start — e2e-only browser upload handler; exercised by media.spec.ts,
    // not host-covered, and deleted in #517 Task 4 (superseded by the upload_media server fn).
    let filename =
        storage::MediaManager::validate_filename(field.file_name()).map_err(|e| map_error(&e))?;
    // cov:ignore-stop
    let content_type = field.content_type().map(ToOwned::to_owned);

    let manager = storage::MediaManager::new(media, site_config, storage_path);
    let response = manager
        .upload(auth_user.user_id, &filename, content_type.as_deref(), field)
        .await
        .map_err(|e| {
            tracing::error!(error = %e, "upload failed");
            map_error(&e)
        })?;

    Ok((StatusCode::CREATED, Json(response)).into_response())
}

// ---------------------------------------------------------------------------
// Serve handler  GET /media/{source}/{p1}/{p2}/{hash}/{filename}
// ---------------------------------------------------------------------------

/// Path parameters for the media serve route.
#[derive(Deserialize)]
pub struct ServeParams {
    pub source: SoftPath<MediaSource>,
    pub p1: String,
    pub p2: String,
    pub hash: SoftPath<ContentHash>,
    pub filename: SoftPath<Filename>,
}

/// Serves a stored media file, recording the `jaunder.media.served` outcome.
///
/// # Errors
///
/// Returns `4xx` status codes for missing files or invalid parameters.
#[tracing::instrument(name = "media.serve", skip_all)]
pub async fn serve_handler(
    media: Extension<Arc<dyn MediaStorage>>,
    storage_path: Extension<Arc<PathBuf>>,
    params: Path<ServeParams>,
    req_headers: axum::http::HeaderMap,
) -> Result<Response, StatusCode> {
    let result = serve_response(media, storage_path, params, req_headers).await;
    if let Some(outcome) = serve_result(&result) {
        host::metrics::media_served(outcome);
    }
    result
}

/// Maps a serve outcome to its bounded `result` attribute, or `None` for
/// internal failures (not one of the served outcomes). Exhaustively tested so
/// every arm is covered independent of handler call paths.
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
    Path(params): Path<ServeParams>,
    req_headers: axum::http::HeaderMap,
) -> Result<Response, StatusCode> {
    let (source, hash, filename, file_path) = resolve_media_path(&storage_path, &params)?;

    if !file_path.exists() {
        return Err(StatusCode::NOT_FOUND);
    }

    // ETag / If-None-Match check.
    let etag_value = format!("\"{hash}\"");
    if let Some(if_none_match) = req_headers.get(axum::http::header::IF_NONE_MATCH) {
        if if_none_match.to_str().unwrap_or("") == etag_value {
            return Ok(StatusCode::NOT_MODIFIED.into_response());
        }
    }

    // Look up content_type from DB; fall back to extension detection.
    let content_type = media
        .find_by_hash(&hash, &source)
        .await
        .map_err(serve_internal_error)?
        .map_or_else(|| detect_content_type(&filename), |r| r.content_type);

    let disposition = content_disposition(&content_type, &filename);

    let file = fs::File::open(&file_path)
        .await
        .map_err(|_| StatusCode::NOT_FOUND)?;
    let stream = ReaderStream::new(file);
    let body = axum::body::Body::from_stream(stream);

    let mut response = Response::new(body);
    let headers = response.headers_mut();

    headers.insert(
        axum::http::header::CONTENT_TYPE,
        // A `ContentType` is always a valid header value (its invariant), so — like the
        // sibling etag/disposition inserts below — the `Err` arm is unreachable (#495,
        // retiring the former octet-stream fallback).
        HeaderValue::from_str(content_type.as_ref())
            .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?,
    );
    headers.insert(
        axum::http::header::CACHE_CONTROL,
        HeaderValue::from_static("public, max-age=31536000, immutable"),
    );
    headers.insert(
        axum::http::header::ETAG,
        HeaderValue::from_str(&etag_value).map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?,
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

/// Validates the serve route's attacker-controlled path parameters, returning
/// the parsed [`MediaSource`], [`ContentHash`], and [`Filename`], or `NOT_FOUND`
/// for any invalid component.
///
/// The `source`/`hash`/`filename` segments arrive as [`SoftPath`] — soft-parsed into
/// [`MediaSource`]/[`ContentHash`]/[`Filename`] at extraction, `None` on a parse miss
/// (never axum's pre-handler 400, keeping this public content route a DoS-safe 404 for a
/// bad segment, #504). A missing (unparseable) component is `NOT_FOUND` here. `ContentHash`'s
/// `FromStr` enforces canonical 64-char lowercase hex, so a malformed hash is rejected before
/// it is sliced (`hash[2..]`/path-join can never panic); `p1`/`p2` must be its matching leading
/// hex pairs. `Filename`'s validating `FromStr` accepts only a single safe leaf, so a
/// traversal value (`..`, `a/b`) is a miss → `NOT_FOUND`.
fn validate_serve_params(
    params: &ServeParams,
) -> Result<(MediaSource, ContentHash, Filename), StatusCode> {
    let Some(source) = params.source.value() else {
        return Err(StatusCode::NOT_FOUND);
    };
    let Some(hash) = params.hash.value() else {
        return Err(StatusCode::NOT_FOUND);
    };

    if !hash.starts_with(&params.p1) || !hash[2..].starts_with(&params.p2) {
        return Err(StatusCode::NOT_FOUND);
    }

    let Some(filename) = params.filename.value() else {
        return Err(StatusCode::NOT_FOUND);
    };

    Ok((*source, hash.clone(), filename.clone()))
}

/// Validates the serve route's path parameters and resolves the on-disk file
/// path, returning `NOT_FOUND` for any invalid component.
fn resolve_media_path(
    storage_path: &std::path::Path,
    params: &ServeParams,
) -> Result<(MediaSource, ContentHash, Filename, PathBuf), StatusCode> {
    let (source, hash, filename) = validate_serve_params(params)?;
    let file_path = storage_path
        .join("media")
        .join(source.as_str())
        .join(&params.p1)
        .join(&params.p2)
        // Join the parsed hash and filename, not the raw `params.*`: identical bytes
        // today (`FromStr` rejects rather than normalizes), but keeps the on-disk path
        // built from the validated values should that ever change.
        .join(hash.as_ref())
        .join(filename.as_ref());

    Ok((source, hash, filename, file_path))
}

/// Builds a header-safe `Content-Disposition` value for serving `filename`.
///
/// The filename is attacker-influenced (it round-trips through the URL), so it
/// is emitted in two forms: a quote/backslash-escaped, ASCII-only `filename=`
/// fallback (control and non-ASCII bytes dropped, so the value can never break
/// the quoted string or be rejected as a header), and an RFC 5987
/// `filename*=UTF-8''…` carrying the full percent-encoded name for modern
/// clients. `inline` vs `attachment` follows [`should_inline`].
fn content_disposition(content_type: &str, filename: &str) -> String {
    use percent_encoding::{utf8_percent_encode, NON_ALPHANUMERIC};

    let disposition = if should_inline(content_type) {
        "inline"
    } else {
        "attachment"
    };

    let mut fallback = String::with_capacity(filename.len());
    for c in filename.chars() {
        if !c.is_ascii() || c.is_control() {
            continue;
        }
        if c == '"' || c == '\\' {
            fallback.push('\\');
        }
        fallback.push(c);
    }

    let encoded = utf8_percent_encode(filename, NON_ALPHANUMERIC);
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
    use common::test_support::parse_content_type;
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
            map_error(&anyhow::anyhow!(storage::MediaError::BadRequest(
                "bad".to_owned()
            ))),
            StatusCode::BAD_REQUEST
        );
        assert_eq!(
            map_error(&anyhow::anyhow!(storage::MediaError::PayloadTooLarge)),
            StatusCode::PAYLOAD_TOO_LARGE
        );
        assert_eq!(
            map_error(&anyhow::anyhow!(storage::MediaError::InsufficientStorage)),
            StatusCode::INSUFFICIENT_STORAGE
        );
        assert_eq!(
            map_error(&anyhow::anyhow!(storage::MediaError::Internal(
                "error".to_owned()
            ))),
            StatusCode::INTERNAL_SERVER_ERROR
        );
        assert_eq!(
            map_error(&anyhow::anyhow!("unknown")),
            StatusCode::INTERNAL_SERVER_ERROR
        );
    }

    fn params(source: &str, p1: &str, p2: &str, hash: &str, filename: &str) -> ServeParams {
        ServeParams {
            source: SoftPath::parse(source),
            p1: p1.to_string(),
            p2: p2.to_string(),
            hash: SoftPath::parse(hash),
            filename: SoftPath::parse(filename),
        }
    }

    #[test]
    fn resolve_media_path_builds_path_for_valid_params() {
        let hash = "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855";
        let p = params("upload", "e3", "b0", hash, "photo.jpg");

        let (source, resolved_hash, _filename, path) =
            resolve_media_path(Path::new("/data"), &p).expect("valid params should resolve");

        assert_eq!(source, MediaSource::Upload);
        assert_eq!(resolved_hash, hash);
        assert_eq!(
            path,
            Path::new("/data")
                .join("media")
                .join("upload")
                .join("e3")
                .join("b0")
                .join(hash)
                .join("photo.jpg")
        );
    }

    #[test]
    fn validate_serve_params_returns_typed_filename_for_canonical_name() {
        let hash = "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855";
        let p = params("upload", "e3", "b0", hash, "photo.jpg");
        let (source, h, filename) = validate_serve_params(&p).expect("valid params");
        assert_eq!(source, MediaSource::Upload);
        assert_eq!(h, hash);
        assert_eq!(filename, "photo.jpg"); // Filename: PartialEq<str>
    }

    #[test]
    fn resolve_media_path_rejects_unknown_source() {
        let hash = "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855";
        let p = params("bogus", "e3", "b0", hash, "photo.jpg");
        assert_eq!(
            resolve_media_path(Path::new("/data"), &p),
            Err(StatusCode::NOT_FOUND)
        );
    }

    #[test]
    fn resolve_media_path_rejects_short_hash() {
        // The historical panic input: shorter than 2 bytes.
        let p = params("upload", "a", "a", "a", "photo.jpg");
        assert_eq!(
            resolve_media_path(Path::new("/data"), &p),
            Err(StatusCode::NOT_FOUND)
        );
    }

    #[test]
    fn resolve_media_path_rejects_non_hex_hash() {
        let hash = "z".repeat(64);
        let p = params("upload", "zz", "zz", &hash, "photo.jpg");
        assert_eq!(
            resolve_media_path(Path::new("/data"), &p),
            Err(StatusCode::NOT_FOUND)
        );
    }

    #[test]
    fn resolve_media_path_rejects_filename_with_traversal_or_separators() {
        let hash = "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855";
        for bad in ["..", ".", "../../etc/passwd", "a/b", "sub/file.txt"] {
            let p = params("upload", "e3", "b0", hash, bad);
            assert_eq!(
                resolve_media_path(Path::new("/data"), &p),
                Err(StatusCode::NOT_FOUND),
                "filename {bad:?} must be rejected"
            );
        }
    }

    #[test]
    fn resolve_media_path_rejects_p1_prefix_mismatch() {
        let hash = "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855";
        let p = params("upload", "ff", "b0", hash, "photo.jpg");
        assert_eq!(
            resolve_media_path(Path::new("/data"), &p),
            Err(StatusCode::NOT_FOUND)
        );
    }

    #[test]
    fn resolve_media_path_rejects_p2_prefix_mismatch() {
        let hash = "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855";
        let p = params("upload", "e3", "ff", hash, "photo.jpg");
        assert_eq!(
            resolve_media_path(Path::new("/data"), &p),
            Err(StatusCode::NOT_FOUND)
        );
    }

    #[test]
    fn content_disposition_picks_inline_or_attachment_by_type() {
        assert!(content_disposition("image/png", "p.png").starts_with("inline; "));
        assert!(
            content_disposition("application/octet-stream", "p.bin").starts_with("attachment; ")
        );
    }

    #[test]
    fn content_disposition_escapes_quotes_and_strips_control_chars() {
        // A quote in the name must be backslash-escaped, never break the
        // quoted-string; control chars are dropped from the ASCII fallback.
        let value = content_disposition("application/octet-stream", "a\"b\n.txt");
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
        let value = content_disposition("image/png", "café.png");
        // Non-ASCII dropped from the ASCII fallback...
        assert!(value.contains(r#"filename="caf.png""#), "{value}");
        // ...but carried, percent-encoded, in filename*.
        assert!(value.contains("filename*=UTF-8''caf%C3%A9"), "{value}");
        assert!(HeaderValue::from_str(&value).is_ok());
    }

    const SAMPLE_HASH: &str = "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855";

    /// Materializes a stored media file under a fresh temp storage root and
    /// returns the root plus the matching serve params.
    fn stored_file(filename: &str) -> (tempfile::TempDir, ServeParams) {
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
        let p = params("upload", "e3", "b0", SAMPLE_HASH, filename);
        (temp, p)
    }

    // guard:no-backend — mock store
    #[tokio::test]
    async fn serve_response_returns_304_on_matching_if_none_match() {
        let (temp, p) = stored_file("photo.jpg");
        // No DB lookup happens: the ETag match short-circuits before find_by_hash.
        let media = storage::MockMediaStorage::new();

        let mut headers = axum::http::HeaderMap::new();
        headers.insert(
            axum::http::header::IF_NONE_MATCH,
            HeaderValue::from_str(&format!("\"{SAMPLE_HASH}\"")).unwrap(),
        );

        let resp = serve_response(
            Extension(Arc::new(media) as Arc<dyn MediaStorage>),
            Extension(Arc::new(temp.path().to_path_buf())),
            Path(p),
            headers,
        )
        .await
        .expect("serve response");
        assert_eq!(resp.status(), StatusCode::NOT_MODIFIED);
    }

    // guard:no-backend — mock store
    #[tokio::test]
    async fn serve_response_serves_body_when_if_none_match_does_not_match() {
        let (temp, p) = stored_file("photo.jpg");
        let mut media = storage::MockMediaStorage::new();
        // ETag present but not matching: the conditional falls through to the
        // normal serve path (DB lookup + 200) rather than returning 304.
        media
            .expect_find_by_hash()
            .times(1)
            .returning(|_, _| Ok(None));

        let mut headers = axum::http::HeaderMap::new();
        headers.insert(
            axum::http::header::IF_NONE_MATCH,
            HeaderValue::from_static("\"not-the-hash\""),
        );

        let resp = serve_response(
            Extension(Arc::new(media) as Arc<dyn MediaStorage>),
            Extension(Arc::new(temp.path().to_path_buf())),
            Path(p),
            headers,
        )
        .await
        .expect("serve response");
        assert_eq!(resp.status(), StatusCode::OK);
    }

    // guard:no-backend — mock store
    #[tokio::test]
    async fn serve_response_falls_back_to_extension_content_type_when_db_has_no_record() {
        let (temp, p) = stored_file("photo.jpg");
        let mut media = storage::MockMediaStorage::new();
        // No DB record -> content type is detected from the filename extension.
        media
            .expect_find_by_hash()
            .times(1)
            .returning(|_, _| Ok(None));

        let resp = serve_response(
            Extension(Arc::new(media) as Arc<dyn MediaStorage>),
            Extension(Arc::new(temp.path().to_path_buf())),
            Path(p),
            axum::http::HeaderMap::new(),
        )
        .await
        .expect("serve response");

        assert_eq!(resp.status(), StatusCode::OK);
        let expected = detect_content_type("photo.jpg");
        assert_eq!(
            resp.headers()
                .get(axum::http::header::CONTENT_TYPE)
                .and_then(|v| v.to_str().ok()),
            Some(expected.as_ref())
        );
    }

    #[test]
    fn every_accepted_content_type_is_header_constructible() {
        // The D4 invariant the retired `:178` octet-stream fallback relied on, observed
        // against the real `HeaderValue::from_str` oracle (#495).
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
