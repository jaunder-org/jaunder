use std::path::PathBuf;
use std::sync::Arc;

use axum::extract::{Multipart, Path, Query};
use axum::http::{HeaderValue, StatusCode};
use axum::response::{IntoResponse, Redirect, Response};
use axum::{Extension, Json};
use serde::{Deserialize, Serialize};
use tokio::fs;
use tokio_util::io::ReaderStream;

use common::media::{detect_content_type, should_inline};
use storage::{AppState, MediaSource};
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
    // Get the first multipart field.
    let Some(field) = multipart
        .next_field()
        .await
        .map_err(|_| StatusCode::BAD_REQUEST)?
    else {
        return Err(StatusCode::BAD_REQUEST);
    };

    let manager = crate::media_manager::MediaManager::new(state, storage_path);
    let response = manager.upload(&auth_user, field).await.map_err(|e| {
        tracing::error!(error = %e, "upload failed");
        crate::media_manager::MediaManager::map_error(&e)
    })?;

    Ok((StatusCode::CREATED, Json(response)).into_response())
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
