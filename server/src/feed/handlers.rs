use std::sync::Arc;

use axum::{
    extract::Path,
    http::{header, HeaderMap, HeaderValue, StatusCode},
    response::{IntoResponse, Response},
    Extension,
};
use common::feed::{canonicalize, FeedFormat, FeedSurface};
use common::{tag::Tag, username::Username};
use storage::{FeedCacheStorage, PostStorage, SiteConfigStorage};

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
    feed_cache: Arc<dyn FeedCacheStorage>,
    site_config: Arc<dyn SiteConfigStorage>,
    posts: Arc<dyn PostStorage>,
    headers: HeaderMap,
    surface: FeedSurface,
    format: FeedFormat,
) -> Response {
    let feed_url = canonicalize(&surface, format);
    let row = match feed_cache.get(&feed_url).await {
        Ok(Some(row)) => {
            common::metrics::feed_cache(common::metrics::CacheResult::Hit);
            row
        }
        Ok(None) => {
            common::metrics::feed_cache(common::metrics::CacheResult::Miss);
            // Cache miss: build the feed inline rather than 404. The background
            // worker only refreshes feeds that have pending events, so a cold or
            // evicted cache entry has no other path back to being populated.
            match regenerate_feed(
                site_config.as_ref(),
                posts.as_ref(),
                feed_cache.as_ref(),
                &feed_url,
            )
            .await
            {
                Ok(row) => row,
                Err(e) => {
                    return (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()).into_response();
                }
            }
        }
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
        } // cov:ignore fall-through brace when if-modified-since parses but the row is newer (304 + no-header paths are tested)
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
    Extension(feed_cache): Extension<Arc<dyn FeedCacheStorage>>,
    Extension(site_config): Extension<Arc<dyn SiteConfigStorage>>,
    Extension(posts): Extension<Arc<dyn PostStorage>>,
    headers: HeaderMap,
    Path(ext): Path<String>,
) -> Response {
    let Some(format) = parse_format(&ext) else {
        return StatusCode::NOT_FOUND.into_response();
    };
    serve(
        feed_cache,
        site_config,
        posts,
        headers,
        FeedSurface::Site,
        format,
    )
    .await
}

pub async fn feed_site_tag(
    Extension(feed_cache): Extension<Arc<dyn FeedCacheStorage>>,
    Extension(site_config): Extension<Arc<dyn SiteConfigStorage>>,
    Extension(posts): Extension<Arc<dyn PostStorage>>,
    headers: HeaderMap,
    Path((tag, ext)): Path<(String, String)>,
) -> Response {
    let Some(format) = parse_format(&ext) else {
        return StatusCode::NOT_FOUND.into_response();
    };
    let Ok(tag) = tag.parse::<Tag>() else {
        return StatusCode::NOT_FOUND.into_response();
    };
    serve(
        feed_cache,
        site_config,
        posts,
        headers,
        FeedSurface::SiteTag { tag },
        format,
    )
    .await
}

pub async fn feed_user(
    Extension(feed_cache): Extension<Arc<dyn FeedCacheStorage>>,
    Extension(site_config): Extension<Arc<dyn SiteConfigStorage>>,
    Extension(posts): Extension<Arc<dyn PostStorage>>,
    headers: HeaderMap,
    Path((username, ext)): Path<(String, String)>,
) -> Response {
    let Some(format) = parse_format(&ext) else {
        return StatusCode::NOT_FOUND.into_response();
    };
    let Ok(username) = username.parse::<Username>() else {
        return StatusCode::NOT_FOUND.into_response();
    };
    serve(
        feed_cache,
        site_config,
        posts,
        headers,
        FeedSurface::User { username },
        format,
    )
    .await
}

pub async fn feed_user_tag(
    Extension(feed_cache): Extension<Arc<dyn FeedCacheStorage>>,
    Extension(site_config): Extension<Arc<dyn SiteConfigStorage>>,
    Extension(posts): Extension<Arc<dyn PostStorage>>,
    headers: HeaderMap,
    Path((username, tag, ext)): Path<(String, String, String)>,
) -> Response {
    let Some(format) = parse_format(&ext) else {
        return StatusCode::NOT_FOUND.into_response();
    };
    let (Ok(username), Ok(tag)) = (username.parse::<Username>(), tag.parse::<Tag>()) else {
        return StatusCode::NOT_FOUND.into_response();
    };
    serve(
        feed_cache,
        site_config,
        posts,
        headers,
        FeedSurface::UserTag { username, tag },
        format,
    )
    .await
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::{Duration, Utc};
    use storage::{FeedCacheError, FeedCacheRow};

    fn sample_row(etag: &str, updated_at: chrono::DateTime<chrono::Utc>) -> FeedCacheRow {
        FeedCacheRow {
            feed_url: "/feed.rss".to_owned(),
            body: "<rss/>".to_owned(),
            etag: etag.to_owned(),
            content_type: "application/rss+xml; charset=utf-8".to_owned(),
            updated_at,
            generated_at: updated_at,
        }
    }

    fn empty_site_config() -> Arc<dyn SiteConfigStorage> {
        Arc::new(storage::MockSiteConfigStorage::new())
    }

    fn empty_posts() -> Arc<dyn PostStorage> {
        Arc::new(storage::MockPostStorage::new())
    }

    // guard:no-backend — mock store
    #[tokio::test]
    async fn serve_returns_500_when_regeneration_fails() {
        let mut cache = storage::MockFeedCacheStorage::new();
        cache.expect_get().returning(|_| Ok(None));
        let mut site_config = storage::MockSiteConfigStorage::new();
        // A storage failure during regeneration surfaces as a 500.
        site_config
            .expect_get_feeds_config()
            .returning(|| Err(sqlx::Error::PoolClosed));

        let resp = serve(
            Arc::new(cache),
            Arc::new(site_config),
            empty_posts(),
            HeaderMap::new(),
            FeedSurface::Site,
            FeedFormat::Rss,
        )
        .await;
        assert_eq!(resp.status(), StatusCode::INTERNAL_SERVER_ERROR);
    }

    // guard:no-backend — mock store
    #[tokio::test]
    async fn serve_returns_500_when_cache_get_errors() {
        let mut cache = storage::MockFeedCacheStorage::new();
        cache
            .expect_get()
            .returning(|_| Err(FeedCacheError::Db(sqlx::Error::PoolClosed)));

        let resp = serve(
            Arc::new(cache),
            empty_site_config(),
            empty_posts(),
            HeaderMap::new(),
            FeedSurface::Site,
            FeedFormat::Rss,
        )
        .await;
        assert_eq!(resp.status(), StatusCode::INTERNAL_SERVER_ERROR);
    }

    // guard:no-backend — mock store
    #[tokio::test]
    async fn serve_returns_304_on_if_none_match() {
        let mut cache = storage::MockFeedCacheStorage::new();
        cache
            .expect_get()
            .returning(|_| Ok(Some(sample_row("\"etag-1\"", Utc::now()))));

        let mut headers = HeaderMap::new();
        headers.insert(
            header::IF_NONE_MATCH,
            HeaderValue::from_static("\"etag-1\""),
        );

        let resp = serve(
            Arc::new(cache),
            empty_site_config(),
            empty_posts(),
            headers,
            FeedSurface::Site,
            FeedFormat::Rss,
        )
        .await;
        assert_eq!(resp.status(), StatusCode::NOT_MODIFIED);
    }

    // guard:no-backend — mock store
    #[tokio::test]
    async fn serve_returns_200_when_if_none_match_does_not_match() {
        let mut cache = storage::MockFeedCacheStorage::new();
        cache
            .expect_get()
            .returning(|_| Ok(Some(sample_row("\"etag-1\"", Utc::now()))));

        // IF_NONE_MATCH present but a different etag: the conditional falls
        // through to a normal 200 rather than returning 304.
        let mut headers = HeaderMap::new();
        headers.insert(
            header::IF_NONE_MATCH,
            HeaderValue::from_static("\"etag-other\""),
        );

        let resp = serve(
            Arc::new(cache),
            empty_site_config(),
            empty_posts(),
            headers,
            FeedSurface::Site,
            FeedFormat::Rss,
        )
        .await;
        assert_eq!(resp.status(), StatusCode::OK);
    }

    // guard:no-backend — mock store
    #[tokio::test]
    async fn serve_returns_200_when_modified_since_is_stale() {
        // Row updated *after* the client's If-Modified-Since date: the
        // conditional falls through to a 200 rather than returning 304.
        let updated_at = Utc::now();
        let mut cache = storage::MockFeedCacheStorage::new();
        cache
            .expect_get()
            .returning(move |_| Ok(Some(sample_row("\"etag-1\"", updated_at))));

        let mut headers = HeaderMap::new();
        let ims = (Utc::now() - Duration::days(1)).to_rfc2822();
        headers.insert(
            header::IF_MODIFIED_SINCE,
            HeaderValue::from_str(&ims).unwrap(),
        );

        let resp = serve(
            Arc::new(cache),
            empty_site_config(),
            empty_posts(),
            headers,
            FeedSurface::Site,
            FeedFormat::Rss,
        )
        .await;
        assert_eq!(resp.status(), StatusCode::OK);
    }

    // guard:no-backend — mock store
    #[tokio::test]
    async fn serve_returns_304_on_if_modified_since() {
        let updated_at = Utc::now() - Duration::days(1);
        let mut cache = storage::MockFeedCacheStorage::new();
        cache
            .expect_get()
            .returning(move |_| Ok(Some(sample_row("\"etag-1\"", updated_at))));

        let mut headers = HeaderMap::new();
        let ims = (Utc::now() + Duration::days(1)).to_rfc2822();
        headers.insert(
            header::IF_MODIFIED_SINCE,
            HeaderValue::from_str(&ims).unwrap(),
        );

        let resp = serve(
            Arc::new(cache),
            empty_site_config(),
            empty_posts(),
            headers,
            FeedSurface::Site,
            FeedFormat::Rss,
        )
        .await;
        assert_eq!(resp.status(), StatusCode::NOT_MODIFIED);
    }

    // guard:no-backend — mock store
    #[tokio::test]
    async fn feed_site_returns_404_on_bad_format() {
        let resp = feed_site(
            Extension(Arc::new(storage::MockFeedCacheStorage::new()) as Arc<dyn FeedCacheStorage>),
            Extension(empty_site_config()),
            Extension(empty_posts()),
            HeaderMap::new(),
            Path("bogus".to_owned()),
        )
        .await;
        assert_eq!(resp.status(), StatusCode::NOT_FOUND);
    }

    // guard:no-backend — mock store
    #[tokio::test]
    async fn feed_site_delegates_to_serve_on_valid_format() {
        let mut cache = storage::MockFeedCacheStorage::new();
        cache
            .expect_get()
            .returning(|_| Ok(Some(sample_row("\"etag-1\"", Utc::now()))));

        let resp = feed_site(
            Extension(Arc::new(cache) as Arc<dyn FeedCacheStorage>),
            Extension(empty_site_config()),
            Extension(empty_posts()),
            HeaderMap::new(),
            Path("rss".to_owned()),
        )
        .await;
        assert_eq!(resp.status(), StatusCode::OK);
    }

    // guard:no-backend — mock store
    #[tokio::test]
    async fn feed_site_tag_returns_404_on_bad_ext() {
        let resp = feed_site_tag(
            Extension(Arc::new(storage::MockFeedCacheStorage::new()) as Arc<dyn FeedCacheStorage>),
            Extension(empty_site_config()),
            Extension(empty_posts()),
            HeaderMap::new(),
            Path(("rust".to_owned(), "bogus".to_owned())),
        )
        .await;
        assert_eq!(resp.status(), StatusCode::NOT_FOUND);
    }

    // guard:no-backend — mock store
    #[tokio::test]
    async fn feed_user_tag_returns_404_on_bad_ext() {
        let resp = feed_user_tag(
            Extension(Arc::new(storage::MockFeedCacheStorage::new()) as Arc<dyn FeedCacheStorage>),
            Extension(empty_site_config()),
            Extension(empty_posts()),
            HeaderMap::new(),
            Path(("alice".to_owned(), "rust".to_owned(), "bogus".to_owned())),
        )
        .await;
        assert_eq!(resp.status(), StatusCode::NOT_FOUND);
    }

    // guard:no-backend — mock store
    #[tokio::test]
    async fn feed_user_tag_delegates_to_serve_on_valid() {
        let mut cache = storage::MockFeedCacheStorage::new();
        cache
            .expect_get()
            .returning(|_| Ok(Some(sample_row("\"etag-1\"", Utc::now()))));

        let resp = feed_user_tag(
            Extension(Arc::new(cache) as Arc<dyn FeedCacheStorage>),
            Extension(empty_site_config()),
            Extension(empty_posts()),
            HeaderMap::new(),
            Path(("alice".to_owned(), "rust".to_owned(), "rss".to_owned())),
        )
        .await;
        assert_eq!(resp.status(), StatusCode::OK);
    }
}
