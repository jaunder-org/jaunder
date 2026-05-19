use std::path::PathBuf;
use std::sync::Arc;

use axum::extract::{Multipart, Path, Query};
use axum::http::{HeaderValue, StatusCode};
use axum::response::{IntoResponse, Redirect, Response};
use axum::{Extension, Json};
use chrono::Utc;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use tokio::fs;
use tokio::io::AsyncWriteExt;
use tokio_util::io::ReaderStream;

use common::media::{detect_content_type, media_url, sanitize_filename, should_inline};
use storage::{
    AppState, CreateMediaError, MediaRecord, MediaSource, DEFAULT_MAX_FILE_SIZE_BYTES,
    DEFAULT_USER_QUOTA_BYTES, MEDIA_MAX_FILE_SIZE_BYTES_KEY, MEDIA_USER_QUOTA_BYTES_KEY,
};
use web::auth::AuthUser;

// ---------------------------------------------------------------------------
// Response types
// ---------------------------------------------------------------------------

/// JSON body returned on a successful upload.
#[derive(Debug, Serialize)]
pub struct UploadResponse {
    pub sha256: String,
    pub filename: String,
    pub content_type: String,
    pub size_bytes: i64,
    pub url: String,
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
#[allow(clippy::too_many_lines)]
pub async fn upload_handler(
    Extension(state): Extension<Arc<AppState>>,
    Extension(storage_path): Extension<Arc<PathBuf>>,
    auth_user: AuthUser,
    mut multipart: Multipart,
) -> Result<Response, StatusCode> {
    // Read max_file_size from site config (default 50 MiB).
    let max_file_size: i64 = state
        .site_config
        .get(MEDIA_MAX_FILE_SIZE_BYTES_KEY)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?
        .and_then(|v| v.parse::<i64>().ok())
        .unwrap_or(DEFAULT_MAX_FILE_SIZE_BYTES);

    let user_quota: i64 = state
        .site_config
        .get(MEDIA_USER_QUOTA_BYTES_KEY)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?
        .and_then(|v| v.parse::<i64>().ok())
        .unwrap_or(DEFAULT_USER_QUOTA_BYTES);

    // Get the first multipart field.
    let Some(mut field) = multipart
        .next_field()
        .await
        .map_err(|_| StatusCode::BAD_REQUEST)?
    else {
        return Err(StatusCode::BAD_REQUEST);
    };

    let raw_filename = field.file_name().unwrap_or("upload").to_owned();
    let filename = sanitize_filename(&raw_filename);
    if filename.is_empty() {
        return Err(StatusCode::BAD_REQUEST);
    }

    let content_type = field
        .content_type()
        .map_or_else(|| detect_content_type(&filename).to_owned(), str::to_owned);

    // Stream to a temp file, computing SHA-256 along the way.
    let tmp_dir = storage_path.join("media").join("tmp");
    fs::create_dir_all(&tmp_dir)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

    let tmp_id = uuid::Uuid::new_v4();
    let tmp_path = tmp_dir.join(tmp_id.to_string());

    let result = stream_to_temp(&mut field, &tmp_path, max_file_size).await;

    let (sha256_hex, size_bytes) = match result {
        Ok(ok) => ok,
        Err(StreamError::TooLarge) => {
            let _ = fs::remove_file(&tmp_path).await;
            return Err(StatusCode::PAYLOAD_TOO_LARGE);
        }
        Err(StreamError::Io) => {
            let _ = fs::remove_file(&tmp_path).await;
            return Err(StatusCode::INTERNAL_SERVER_ERROR);
        }
    };

    // Check user quota.
    let current_usage = state
        .media
        .get_user_upload_usage(auth_user.user_id)
        .await
        .map_err(|_| {
            let _ = std::fs::remove_file(&tmp_path);
            StatusCode::INTERNAL_SERVER_ERROR
        })?;

    if current_usage + size_bytes > user_quota {
        let _ = fs::remove_file(&tmp_path).await;
        return Err(StatusCode::INSUFFICIENT_STORAGE);
    }

    // Determine target path:  <storage_path>/media/upload/<p1>/<p2>/<sha256>/<filename>
    let p1 = &sha256_hex[..2];
    let p2 = &sha256_hex[2..4];
    let hash_dir = storage_path
        .join("media")
        .join("upload")
        .join(p1)
        .join(p2)
        .join(&sha256_hex);
    let target_path = hash_dir.join(&filename);

    if target_path.exists() {
        // Exact duplicate — drop temp.
        let _ = fs::remove_file(&tmp_path).await;
    } else {
        // Check whether hash dir has any existing file (for hard-link dedup).
        let existing_file = first_file_in_dir(&hash_dir).await;

        fs::create_dir_all(&hash_dir)
            .await
            .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

        if let Some(existing) = existing_file {
            // Hard-link the existing file to the new name, drop temp.
            fs::hard_link(&existing, &target_path)
                .await
                .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
            let _ = fs::remove_file(&tmp_path).await;
        } else {
            // Move temp to target.
            fs::rename(&tmp_path, &target_path)
                .await
                .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
        }
    }

    // Insert DB record (ignore AlreadyExists).
    let record = MediaRecord {
        user_id: auth_user.user_id,
        sha256: sha256_hex.clone(),
        filename: filename.clone(),
        source: MediaSource::Upload,
        content_type: content_type.clone(),
        size_bytes,
        source_url: None,
        created_at: Utc::now(),
    };
    match state.media.create_media(&record).await {
        Ok(()) | Err(CreateMediaError::AlreadyExists) => {}
        Err(CreateMediaError::Internal(e)) => {
            tracing::error!(error = %e, "create_media failed");
            return Err(StatusCode::INTERNAL_SERVER_ERROR);
        }
    }

    let url = media_url("upload", &sha256_hex, &filename);
    let body = UploadResponse {
        sha256: sha256_hex,
        filename,
        content_type,
        size_bytes,
        url,
    };

    Ok((StatusCode::CREATED, Json(body)).into_response())
}

// ---------------------------------------------------------------------------
// Serve handler  GET /media/{source}/{p1}/{p2}/{hash}/{filename}
// ---------------------------------------------------------------------------

/// Path parameters for the media serve route.
#[derive(Deserialize)]
pub struct ServeParams {
    pub source: String,
    pub p1: String,
    pub p2: String,
    pub hash: String,
    pub filename: String,
}

/// Serves a stored media file with long-lived cache headers and `ETag` support.
///
/// # Errors
///
/// Returns `4xx` status codes for missing files or invalid parameters.
pub async fn serve_handler(
    Extension(state): Extension<Arc<AppState>>,
    Extension(storage_path): Extension<Arc<PathBuf>>,
    Path(params): Path<ServeParams>,
    req_headers: axum::http::HeaderMap,
) -> Result<Response, StatusCode> {
    // Validate source.
    let source: MediaSource = params.source.parse().map_err(|_| StatusCode::NOT_FOUND)?;

    // Validate prefix segments match hash.
    if !params.hash.starts_with(&params.p1) || !params.hash[2..].starts_with(&params.p2) {
        return Err(StatusCode::NOT_FOUND);
    }

    let file_path = storage_path
        .join("media")
        .join(source.as_str())
        .join(&params.p1)
        .join(&params.p2)
        .join(&params.hash)
        .join(&params.filename);

    if !file_path.exists() {
        return Err(StatusCode::NOT_FOUND);
    }

    // ETag / If-None-Match check.
    let etag_value = format!("\"{hash}\"", hash = params.hash);
    if let Some(if_none_match) = req_headers.get(axum::http::header::IF_NONE_MATCH) {
        if if_none_match.to_str().unwrap_or("") == etag_value {
            return Ok(StatusCode::NOT_MODIFIED.into_response());
        }
    }

    // Look up content_type from DB; fall back to extension detection.
    let content_type = state
        .media
        .find_by_hash(&params.hash, &source)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?
        .map_or_else(
            || detect_content_type(&params.filename).to_owned(),
            |r| r.content_type,
        );

    let disposition = if should_inline(&content_type) {
        format!("inline; filename=\"{}\"", params.filename)
    } else {
        format!("attachment; filename=\"{}\"", params.filename)
    };

    let file = fs::File::open(&file_path)
        .await
        .map_err(|_| StatusCode::NOT_FOUND)?;
    let stream = ReaderStream::new(file);
    let body = axum::body::Body::from_stream(stream);

    let mut response = Response::new(body);
    let headers = response.headers_mut();

    headers.insert(
        axum::http::header::CONTENT_TYPE,
        HeaderValue::from_str(&content_type)
            .unwrap_or_else(|_| HeaderValue::from_static("application/octet-stream")),
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
    pub user_id: i64,
}

/// Stub proxy handler: redirects to the remote URL.
///
/// Full caching implementation is deferred to a future milestone.
///
/// # Errors
///
/// Returns 401 if the authenticated user does not match `user_id`.
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

enum StreamError {
    TooLarge,
    Io,
}

/// Streams `field` to `tmp_path`, computing SHA-256 and counting bytes.
/// Returns `(hex_digest, size_bytes)` or an error.
async fn stream_to_temp(
    field: &mut axum::extract::multipart::Field<'_>,
    tmp_path: &std::path::Path,
    max_file_size: i64,
) -> Result<(String, i64), StreamError> {
    let mut file = fs::File::create(tmp_path)
        .await
        .map_err(|_| StreamError::Io)?;
    let mut hasher = Sha256::new();
    let mut bytes_written: i64 = 0;

    loop {
        let chunk = field.chunk().await.map_err(|_| StreamError::Io)?;
        let Some(data) = chunk else { break };

        bytes_written += i64::try_from(data.len()).unwrap_or(i64::MAX);
        if bytes_written > max_file_size {
            return Err(StreamError::TooLarge);
        }

        hasher.update(&data);
        file.write_all(&data).await.map_err(|_| StreamError::Io)?;
    }

    file.flush().await.map_err(|_| StreamError::Io)?;
    drop(file);

    let digest = hasher.finalize();
    let sha256_hex = format!("{digest:x}");
    Ok((sha256_hex, bytes_written))
}

/// Returns the first regular file found directly inside `dir`, if any.
async fn first_file_in_dir(dir: &std::path::Path) -> Option<PathBuf> {
    let mut read_dir = fs::read_dir(dir).await.ok()?;
    while let Ok(Some(entry)) = read_dir.next_entry().await {
        let path = entry.path();
        if path.is_file() {
            return Some(path);
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;
    use tokio::fs;

    #[tokio::test]
    async fn test_first_file_in_dir() {
        let temp = TempDir::new().unwrap();
        let dir = temp.path();

        // Empty dir
        assert_eq!(first_file_in_dir(dir).await, None);

        // Dir with a subdir (should be ignored by is_file())
        let subdir = dir.join("subdir");
        fs::create_dir(&subdir).await.unwrap();
        assert_eq!(first_file_in_dir(dir).await, None);

        // Dir with a file
        let file = dir.join("test.txt");
        fs::write(&file, "hello").await.unwrap();
        assert_eq!(first_file_in_dir(dir).await, Some(file));
    }
}
