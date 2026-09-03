use std::sync::Arc;

use axum::{
    Extension,
    extract::Path,
    http::{HeaderMap, HeaderValue, StatusCode, header},
    response::{IntoResponse, Response},
};
use common::{
    feed::{FeedFormat, FeedSurface},
    tag::Tag,
    username::Username,
};
use host::feed::FeedPath;
use host::metrics;
use storage::{FeedCacheStorage, PostStorage, SiteConfigStorage, WriteScope};

use super::regenerate;
use crate::soft_path::SoftPath;
use web::error::InternalError;

/// Retains a cache-read failure as the typed source of the sanitized boundary
/// carrier.
#[must_use]
pub fn map_feed_cache_failure(error: storage::FeedCacheError) -> InternalError {
    InternalError::storage(error).with_context("boundary", "server.feed.cache_read")
}

/// Retains a regeneration failure as the typed source of the sanitized boundary
/// carrier.
#[must_use]
pub fn map_regeneration_failure(error: super::regenerate::RegenerateError) -> InternalError {
    InternalError::storage(error).with_context("boundary", "server.feed.regenerate")
}

async fn serve(
    feed_cache: Arc<dyn FeedCacheStorage>,
    site_config: Arc<dyn SiteConfigStorage>,
    posts: Arc<dyn PostStorage>,
    write_scope: WriteScope,
    headers: HeaderMap,
    surface: FeedSurface,
    format: FeedFormat,
) -> Response {
    let feed_path = FeedPath::canonical(&surface, format);
    let row = match feed_cache.get(&feed_path).await {
        Ok(Some(row)) => {
            metrics::feed_cache(metrics::CacheResult::Hit);
            row
        }
        Ok(None) => {
            metrics::feed_cache(metrics::CacheResult::Miss);
            // Cache miss: build the feed inline rather than 404. The background
            // worker only refreshes feeds that have pending events, so a cold or
            // evicted cache entry has no other path back to being populated.
            match regenerate::feed(
                site_config.as_ref(),
                posts.as_ref(),
                Arc::clone(&feed_cache),
                &write_scope,
                feed_path.clone(),
            )
            .await
            {
                Ok(row) => row,
                Err(error) => {
                    let error = map_regeneration_failure(error);
                    error.emit_boundary_failure();
                    return StatusCode::INTERNAL_SERVER_ERROR.into_response();
                }
            }
        }
        Err(error) => {
            let error = map_feed_cache_failure(error);
            error.emit_boundary_failure();
            return StatusCode::INTERNAL_SERVER_ERROR.into_response();
        }
    };

    if let Some(etag) = headers.get(header::IF_NONE_MATCH)
        && etag.to_str().ok() == Some(row.etag.as_ref())
    {
        return StatusCode::NOT_MODIFIED.into_response();
    }
    if let Some(ims) = headers.get(header::IF_MODIFIED_SINCE)
        && let Some(t) = ims
            .to_str()
            .ok()
            .and_then(|s| chrono::DateTime::parse_from_rfc2822(s).ok())
        && row.updated_at.value() <= t.with_timezone(&chrono::Utc)
    {
        return StatusCode::NOT_MODIFIED.into_response();
    } // cov:ignore fall-through brace; llvm-cov leaves it unmarked though the row-newer (200, not 304) path is tested

    let mut resp_headers = HeaderMap::new();
    if let Ok(ct) = HeaderValue::from_str(&row.representation().content_type()) {
        resp_headers.insert(header::CONTENT_TYPE, ct);
    }
    if let Ok(etag) = HeaderValue::from_str(&row.etag) {
        resp_headers.insert(header::ETAG, etag);
    }
    if let Ok(lm) = HeaderValue::from_str(&row.updated_at.value().to_rfc2822()) {
        resp_headers.insert(header::LAST_MODIFIED, lm);
    }
    resp_headers.insert(
        header::CACHE_CONTROL,
        HeaderValue::from_static("public, max-age=300"),
    );
    (
        StatusCode::OK,
        resp_headers,
        row.into_representation().into_body(),
    )
        .into_response()
}

pub async fn feed_site(
    Extension(feed_cache): Extension<Arc<dyn FeedCacheStorage>>,
    Extension(site_config): Extension<Arc<dyn SiteConfigStorage>>,
    Extension(posts): Extension<Arc<dyn PostStorage>>,
    Extension(write_scope): Extension<WriteScope>,
    headers: HeaderMap,
    Path(format): Path<SoftPath<FeedFormat>>,
) -> Response {
    let Some(format) = format.into() else {
        return StatusCode::NOT_FOUND.into_response();
    };
    serve(
        feed_cache,
        site_config,
        posts,
        write_scope,
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
    Extension(write_scope): Extension<WriteScope>,
    headers: HeaderMap,
    Path((tag, format)): Path<(SoftPath<Tag>, SoftPath<FeedFormat>)>,
) -> Response {
    let Some(format) = format.into() else {
        return StatusCode::NOT_FOUND.into_response();
    };
    let Some(tag) = tag.into() else {
        return StatusCode::NOT_FOUND.into_response();
    };
    serve(
        feed_cache,
        site_config,
        posts,
        write_scope,
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
    Extension(write_scope): Extension<WriteScope>,
    headers: HeaderMap,
    Path((username, format)): Path<(SoftPath<Username>, SoftPath<FeedFormat>)>,
) -> Response {
    let Some(format) = format.into() else {
        return StatusCode::NOT_FOUND.into_response();
    };
    let Some(username) = username.into() else {
        return StatusCode::NOT_FOUND.into_response();
    };
    serve(
        feed_cache,
        site_config,
        posts,
        write_scope,
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
    Extension(write_scope): Extension<WriteScope>,
    headers: HeaderMap,
    Path((username, tag, format)): Path<(SoftPath<Username>, SoftPath<Tag>, SoftPath<FeedFormat>)>,
) -> Response {
    let Some(format) = format.into() else {
        return StatusCode::NOT_FOUND.into_response();
    };
    let (Some(username), Some(tag)) = (username.into(), tag.into()) else {
        return StatusCode::NOT_FOUND.into_response();
    };
    serve(
        feed_cache,
        site_config,
        posts,
        write_scope,
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
    use common::{feed::FeedFormat, test_support::parse_etag, time::UtcInstant};
    use host::feed::SyndicationFeedRepresentation;
    use storage::{FeedCacheError, FeedCacheRow};

    fn sample_row(etag: &str, updated_at: UtcInstant) -> FeedCacheRow {
        FeedCacheRow::new(
            "/feed.rss".parse().expect("valid feed path"),
            SyndicationFeedRepresentation::try_from_stored(
                FeedFormat::Rss,
                FeedFormat::Rss.content_type(),
                "<rss/>".to_owned(),
            )
            .expect("matching stored representation metadata"),
            parse_etag(etag),
            updated_at,
            updated_at,
        )
        .expect("matching cache row formats")
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
            storage::test_support::mock_write_scope(),
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
            storage::test_support::mock_write_scope(),
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
            .returning(|_| Ok(Some(sample_row("\"etag-1\"", UtcInstant::now()))));

        let mut headers = HeaderMap::new();
        headers.insert(
            header::IF_NONE_MATCH,
            HeaderValue::from_static("\"etag-1\""),
        );

        let resp = serve(
            Arc::new(cache),
            empty_site_config(),
            empty_posts(),
            storage::test_support::mock_write_scope(),
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
            .returning(|_| Ok(Some(sample_row("\"etag-1\"", UtcInstant::now()))));

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
            storage::test_support::mock_write_scope(),
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
        let updated_at = UtcInstant::now();
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
            storage::test_support::mock_write_scope(),
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
        let updated_at = UtcInstant::from(Utc::now() - Duration::days(1));
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
            storage::test_support::mock_write_scope(),
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
            Extension(storage::test_support::mock_write_scope()),
            HeaderMap::new(),
            Path(SoftPath::parse("bogus")),
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
            .returning(|_| Ok(Some(sample_row("\"etag-1\"", UtcInstant::now()))));

        let resp = feed_site(
            Extension(Arc::new(cache) as Arc<dyn FeedCacheStorage>),
            Extension(empty_site_config()),
            Extension(empty_posts()),
            Extension(storage::test_support::mock_write_scope()),
            HeaderMap::new(),
            Path(SoftPath::parse("rss")),
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
            Extension(storage::test_support::mock_write_scope()),
            HeaderMap::new(),
            Path((SoftPath::parse("rust"), SoftPath::parse("bogus"))),
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
            Extension(storage::test_support::mock_write_scope()),
            HeaderMap::new(),
            Path((
                SoftPath::parse("alice"),
                SoftPath::parse("rust"),
                SoftPath::parse("bogus"),
            )),
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
            .returning(|_| Ok(Some(sample_row("\"etag-1\"", UtcInstant::now()))));

        let resp = feed_user_tag(
            Extension(Arc::new(cache) as Arc<dyn FeedCacheStorage>),
            Extension(empty_site_config()),
            Extension(empty_posts()),
            Extension(storage::test_support::mock_write_scope()),
            HeaderMap::new(),
            Path((
                SoftPath::parse("alice"),
                SoftPath::parse("rust"),
                SoftPath::parse("rss"),
            )),
        )
        .await;
        assert_eq!(resp.status(), StatusCode::OK);
    }
}
