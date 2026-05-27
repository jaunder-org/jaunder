use std::sync::Arc;

use axum::{
    extract::Path,
    http::{header, HeaderMap, HeaderValue, StatusCode},
    response::{IntoResponse, Response},
    Extension,
};
use common::feed::{canonicalize, FeedFormat, FeedSurface};
use storage::AppState;

use super::regenerate::regenerate_feed;

fn parse_format(ext: &str) -> Option<FeedFormat> {
    match ext {
        "rss" => Some(FeedFormat::Rss),
        "atom" => Some(FeedFormat::Atom),
        "json" => Some(FeedFormat::Json),
        _ => None,
    }
}

async fn serve(
    state: Arc<AppState>,
    headers: HeaderMap,
    surface: FeedSurface,
    format: FeedFormat,
) -> Response {
    let feed_url = canonicalize(&surface, format);
    let row = match state.feed_cache.get(&feed_url).await {
        Ok(Some(row)) => row,
        Ok(None) => match regenerate_feed(&state, &feed_url).await {
            Ok(row) => row,
            Err(e) => return (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()).into_response(),
        },
        Err(e) => return (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()).into_response(),
    };

    if let Some(etag) = headers.get(header::IF_NONE_MATCH) {
        if etag.to_str().ok() == Some(row.etag.as_str()) {
            return StatusCode::NOT_MODIFIED.into_response();
        }
    }
    if let Some(ims) = headers.get(header::IF_MODIFIED_SINCE) {
        if let Some(t) = ims
            .to_str()
            .ok()
            .and_then(|s| chrono::DateTime::parse_from_rfc2822(s).ok())
        {
            if row.updated_at <= t.with_timezone(&chrono::Utc) {
                return StatusCode::NOT_MODIFIED.into_response();
            }
        }
    }

    let mut resp_headers = HeaderMap::new();
    if let Ok(ct) = HeaderValue::from_str(&row.content_type) {
        resp_headers.insert(header::CONTENT_TYPE, ct);
    }
    if let Ok(etag) = HeaderValue::from_str(&row.etag) {
        resp_headers.insert(header::ETAG, etag);
    }
    if let Ok(lm) = HeaderValue::from_str(&row.updated_at.to_rfc2822()) {
        resp_headers.insert(header::LAST_MODIFIED, lm);
    }
    resp_headers.insert(
        header::CACHE_CONTROL,
        HeaderValue::from_static("public, max-age=300"),
    );
    (StatusCode::OK, resp_headers, row.body).into_response()
}

pub async fn feed_site(
    Extension(state): Extension<Arc<AppState>>,
    headers: HeaderMap,
    Path(ext): Path<String>,
) -> Response {
    let Some(format) = parse_format(&ext) else {
        return StatusCode::NOT_FOUND.into_response();
    };
    serve(state, headers, FeedSurface::Site, format).await
}

pub async fn feed_site_tag(
    Extension(state): Extension<Arc<AppState>>,
    headers: HeaderMap,
    Path((tag, ext)): Path<(String, String)>,
) -> Response {
    let Some(format) = parse_format(&ext) else {
        return StatusCode::NOT_FOUND.into_response();
    };
    serve(state, headers, FeedSurface::SiteTag { tag }, format).await
}

pub async fn feed_user(
    Extension(state): Extension<Arc<AppState>>,
    headers: HeaderMap,
    Path((username, ext)): Path<(String, String)>,
) -> Response {
    let Some(format) = parse_format(&ext) else {
        return StatusCode::NOT_FOUND.into_response();
    };
    serve(state, headers, FeedSurface::User { username }, format).await
}

pub async fn feed_user_tag(
    Extension(state): Extension<Arc<AppState>>,
    headers: HeaderMap,
    Path((username, tag, ext)): Path<(String, String, String)>,
) -> Response {
    let Some(format) = parse_format(&ext) else {
        return StatusCode::NOT_FOUND.into_response();
    };
    serve(
        state,
        headers,
        FeedSurface::UserTag { username, tag },
        format,
    )
    .await
}
