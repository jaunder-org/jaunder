use std::{sync::Arc, time::SystemTime};

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
use storage::{CacheCommitOutcome, FeedCacheError, FeedCacheRow, FeedCacheStorage, PostStorage};

use super::{
    conditional,
    regenerate::{self, RegenerateError},
};
use crate::publisher::PublisherService;
use crate::soft_path::SoftPath;
use web::error::InternalError;

/// Retains a cache-read failure as the typed source of the sanitized boundary
/// carrier.
#[must_use]
pub fn map_feed_cache_failure(error: FeedCacheError) -> InternalError {
    InternalError::storage(error).with_context("boundary", "server.feed.cache_read")
}

/// Retains a regeneration failure as the typed source of the sanitized boundary
/// carrier.
#[must_use]
pub fn map_regeneration_failure(error: RegenerateError) -> InternalError {
    InternalError::storage(error).with_context("boundary", "server.feed.regenerate")
}

async fn regenerate_cache_miss(
    publisher: &PublisherService,
    posts: &dyn PostStorage,
    feed_path: FeedPath,
) -> Result<FeedCacheRow, RegenerateError> {
    loop {
        let snapshot = publisher
            .snapshot()
            .await
            .map_err(RegenerateError::Publisher)?;
        let row = regenerate::render(&snapshot, posts, feed_path.clone()).await?;
        let guard = publisher
            .finalization_guard()
            .await
            .map_err(RegenerateError::Publisher)?;
        match guard.commit_cache(snapshot.generation, row).await {
            Ok(CacheCommitOutcome::Committed(effective_row)) => return Ok(effective_row),
            Ok(CacheCommitOutcome::StaleGeneration) => {}
            Err(error) => return Err(RegenerateError::Storage(Box::new(error))),
        }
    }
}

async fn serve(
    feed_cache: Arc<dyn FeedCacheStorage>,
    publisher: Arc<PublisherService>,
    posts: Arc<dyn PostStorage>,
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
            let regeneration =
                regenerate_cache_miss(publisher.as_ref(), posts.as_ref(), feed_path.clone()).await;
            match regeneration {
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

    let last_modified: SystemTime = row.representation_modified_at.value().into();
    let status =
        if conditional::is_not_modified(&headers, row.etag.as_ref().as_bytes(), last_modified) {
            StatusCode::NOT_MODIFIED
        } else {
            StatusCode::OK
        };
    finish_cached_response(cached_response(row, status))
}

fn finish_cached_response(result: Result<Response, InternalError>) -> Response {
    match result {
        Ok(response) => response,
        Err(error) => {
            error.emit_boundary_failure();
            StatusCode::INTERNAL_SERVER_ERROR.into_response()
        }
    }
}

fn cached_response(row: FeedCacheRow, status: StatusCode) -> Result<Response, InternalError> {
    let mut headers = cache_headers(&row)?;
    if status == StatusCode::OK {
        let content_type = HeaderValue::from_str(&row.representation().content_type())
            .map_err(map_feed_response_metadata_failure)?;
        headers.insert(header::CONTENT_TYPE, content_type);
        return Ok((status, headers, row.into_representation().into_body()).into_response());
    }

    Ok((status, headers).into_response())
}

fn cache_headers(row: &FeedCacheRow) -> Result<HeaderMap, InternalError> {
    let mut headers = HeaderMap::new();
    let etag = HeaderValue::from_bytes(row.etag.as_ref().as_bytes())
        .map_err(map_feed_response_metadata_failure)?;
    headers.insert(header::ETAG, etag);
    let representation_modified_at: SystemTime = row.representation_modified_at.value().into();
    let last_modified = HeaderValue::from_str(&httpdate::fmt_http_date(representation_modified_at))
        .map_err(map_feed_response_metadata_failure)?;
    headers.insert(header::LAST_MODIFIED, last_modified);
    headers.insert(
        header::CACHE_CONTROL,
        HeaderValue::from_static("public, max-age=300"),
    );
    Ok(headers)
}

fn map_feed_response_metadata_failure(
    error: axum::http::header::InvalidHeaderValue,
) -> InternalError {
    InternalError::server(error).with_context("boundary", "server.feed.response_metadata")
}

pub async fn feed_site(
    Extension(feed_cache): Extension<Arc<dyn FeedCacheStorage>>,
    Extension(publisher): Extension<Arc<PublisherService>>,
    Extension(posts): Extension<Arc<dyn PostStorage>>,
    headers: HeaderMap,
    Path(format): Path<SoftPath<FeedFormat>>,
) -> Response {
    let Some(format) = format.into() else {
        return StatusCode::NOT_FOUND.into_response();
    };
    serve(
        feed_cache,
        publisher,
        posts,
        headers,
        FeedSurface::Site,
        format,
    )
    .await
}

pub async fn feed_site_tag(
    Extension(feed_cache): Extension<Arc<dyn FeedCacheStorage>>,
    Extension(publisher): Extension<Arc<PublisherService>>,
    Extension(posts): Extension<Arc<dyn PostStorage>>,
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
        publisher,
        posts,
        headers,
        FeedSurface::SiteTag { tag },
        format,
    )
    .await
}

pub async fn feed_user(
    Extension(feed_cache): Extension<Arc<dyn FeedCacheStorage>>,
    Extension(publisher): Extension<Arc<PublisherService>>,
    Extension(posts): Extension<Arc<dyn PostStorage>>,
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
        publisher,
        posts,
        headers,
        FeedSurface::User { username },
        format,
    )
    .await
}

pub async fn feed_user_tag(
    Extension(feed_cache): Extension<Arc<dyn FeedCacheStorage>>,
    Extension(publisher): Extension<Arc<PublisherService>>,
    Extension(posts): Extension<Arc<dyn PostStorage>>,
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
        publisher,
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
    use chrono::{TimeZone, Utc};
    use common::{test_support::parse_etag, time::UtcInstant};
    use http_body_util::BodyExt;
    use rstest::*;
    use rstest_reuse::*;
    use sqlx::Error;
    use storage::{
        FeedCacheError, FeedCacheRow, MockFeedCacheStorage, MockPostStorage, MockPublisherStorage,
        PublisherStorageError,
        test_support::{Backend, SeedFeedCache, backends},
    };
    fn sample_row(etag: &str, updated_at: UtcInstant) -> FeedCacheRow {
        SeedFeedCache::new("/feed.rss".parse().expect("valid feed path"))
            .body("<rss/>".to_owned())
            .etag(parse_etag(etag))
            .representation_modified_at(updated_at)
            .generated_at(updated_at)
            .build()
    }

    fn empty_publisher() -> Arc<PublisherService> {
        Arc::new(PublisherService::new(
            std::env::temp_dir(),
            Arc::new(MockPublisherStorage::new()),
            storage::test_support::mock_write_scope(),
        ))
    }

    async fn body_bytes(response: Response) -> Vec<u8> {
        response
            .into_body()
            .collect()
            .await
            .expect("response body is readable")
            .to_bytes()
            .to_vec()
    }
    fn empty_posts() -> Arc<dyn PostStorage> {
        Arc::new(MockPostStorage::new())
    }

    #[apply(backends)]
    #[tokio::test]
    async fn regenerate_cache_miss_preserves_cache_commit_storage_errors(#[case] backend: Backend) {
        let env = backend.setup().await;
        let snapshot = env
            .state
            .publisher
            .snapshot()
            .await
            .expect("valid snapshot");
        let directory = tempfile::tempdir().expect("temporary storage directory");
        let mut publisher = MockPublisherStorage::new();
        publisher
            .expect_snapshot()
            .return_once(move || Ok(snapshot));
        publisher
            .expect_commit_cache()
            .returning(|_, _, _| Err(PublisherStorageError::Db(Error::PoolClosed)));
        let service = PublisherService::new(
            directory.path().to_owned(),
            Arc::new(publisher),
            storage::test_support::mock_write_scope(),
        );
        let mut posts = MockPostStorage::new();
        posts
            .expect_list_published_in_window()
            .returning(|_, _, _, _| Ok(Vec::new()));

        let error = regenerate_cache_miss(
            &service,
            &posts,
            FeedPath::canonical(&FeedSurface::Site, FeedFormat::Rss),
        )
        .await
        .expect_err("cache commit failure");

        assert!(matches!(error, RegenerateError::Storage(_)));
    }

    // guard:no-backend — mock store
    #[tokio::test]
    async fn serve_returns_500_when_regeneration_fails() {
        let mut cache = MockFeedCacheStorage::new();
        cache.expect_get().returning(|_| Ok(None));
        let mut publisher = MockPublisherStorage::new();
        publisher
            .expect_snapshot()
            .returning(|| Err(PublisherStorageError::Db(Error::PoolClosed)));

        let resp = serve(
            Arc::new(cache),
            Arc::new(PublisherService::new(
                std::env::temp_dir(),
                Arc::new(publisher),
                storage::test_support::mock_write_scope(),
            )),
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
        let mut cache = MockFeedCacheStorage::new();
        cache
            .expect_get()
            .returning(|_| Err(FeedCacheError::Db(Error::PoolClosed)));

        let resp = serve(
            Arc::new(cache),
            empty_publisher(),
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
    async fn serve_returns_a_metadata_complete_body_free_304_on_if_none_match() {
        let updated_at = UtcInstant::from(
            Utc.with_ymd_and_hms(1994, 11, 6, 8, 49, 37)
                .single()
                .expect("valid timestamp"),
        );
        let mut cache = MockFeedCacheStorage::new();
        cache
            .expect_get()
            .returning(move |_| Ok(Some(sample_row("\"etag-1\"", updated_at))));

        let mut headers = HeaderMap::new();
        headers.insert(
            header::IF_NONE_MATCH,
            HeaderValue::from_static("W/\"etag-1\""),
        );

        let resp = serve(
            Arc::new(cache),
            empty_publisher(),
            empty_posts(),
            headers,
            FeedSurface::Site,
            FeedFormat::Rss,
        )
        .await;
        assert_eq!(resp.status(), StatusCode::NOT_MODIFIED);
        assert_eq!(resp.headers()[header::ETAG], "\"etag-1\"");
        assert_eq!(
            resp.headers()[header::LAST_MODIFIED],
            "Sun, 06 Nov 1994 08:49:37 GMT"
        );
        assert_eq!(resp.headers()[header::CACHE_CONTROL], "public, max-age=300");
        assert!(!resp.headers().contains_key(header::CONTENT_TYPE));
        assert_eq!(body_bytes(resp).await, b"");
    }
    // guard:no-backend — mock store
    #[tokio::test]
    async fn serve_returns_a_metadata_complete_200_on_nonmatching_if_none_match() {
        let updated_at = UtcInstant::from(
            Utc.with_ymd_and_hms(1994, 11, 6, 8, 49, 37)
                .single()
                .expect("valid timestamp"),
        );
        let mut cache = MockFeedCacheStorage::new();
        cache
            .expect_get()
            .returning(move |_| Ok(Some(sample_row("\"etag-1\"", updated_at))));

        let mut headers = HeaderMap::new();
        headers.insert(
            header::IF_NONE_MATCH,
            HeaderValue::from_static("\"etag-other\""),
        );

        let resp = serve(
            Arc::new(cache),
            empty_publisher(),
            empty_posts(),
            headers,
            FeedSurface::Site,
            FeedFormat::Rss,
        )
        .await;
        assert_eq!(resp.status(), StatusCode::OK);
        assert_eq!(
            resp.headers()[header::CONTENT_TYPE],
            "application/rss+xml; charset=utf-8"
        );
        assert_eq!(resp.headers()[header::ETAG], "\"etag-1\"");
        assert_eq!(
            resp.headers()[header::LAST_MODIFIED],
            "Sun, 06 Nov 1994 08:49:37 GMT"
        );
        assert_eq!(resp.headers()[header::CACHE_CONTROL], "public, max-age=300");
        assert_eq!(body_bytes(resp).await, b"<rss/>");
    }

    // guard:no-backend — mock store
    #[tokio::test]
    async fn serve_does_not_fall_back_to_if_modified_since_when_if_none_match_is_present() {
        let updated_at = UtcInstant::from(
            Utc.with_ymd_and_hms(1994, 11, 6, 8, 49, 37)
                .single()
                .expect("valid timestamp"),
        );
        let mut cache = MockFeedCacheStorage::new();
        cache
            .expect_get()
            .returning(move |_| Ok(Some(sample_row("\"etag-1\"", updated_at))));

        let mut headers = HeaderMap::new();
        headers.insert(header::IF_NONE_MATCH, HeaderValue::from_static("malformed"));
        headers.insert(
            header::IF_MODIFIED_SINCE,
            HeaderValue::from_static("Sun, 06 Nov 1994 08:49:37 GMT"),
        );

        let resp = serve(
            Arc::new(cache),
            empty_publisher(),
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
        let updated_at = UtcInstant::from(
            Utc.with_ymd_and_hms(1994, 11, 6, 8, 49, 37)
                .single()
                .expect("valid timestamp"),
        );
        let mut cache = MockFeedCacheStorage::new();
        cache
            .expect_get()
            .returning(move |_| Ok(Some(sample_row("\"etag-1\"", updated_at))));

        let mut headers = HeaderMap::new();
        headers.insert(
            header::IF_MODIFIED_SINCE,
            HeaderValue::from_static("Sat, 05 Nov 1994 08:49:37 GMT"),
        );

        let resp = serve(
            Arc::new(cache),
            empty_publisher(),
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
        let updated_at = UtcInstant::from(
            Utc.with_ymd_and_hms(1994, 11, 6, 8, 49, 37)
                .single()
                .expect("valid timestamp"),
        );
        let mut cache = MockFeedCacheStorage::new();
        cache
            .expect_get()
            .returning(move |_| Ok(Some(sample_row("\"etag-1\"", updated_at))));

        let mut headers = HeaderMap::new();
        headers.insert(
            header::IF_MODIFIED_SINCE,
            HeaderValue::from_static("Sun, 06 Nov 1994 08:49:37 GMT"),
        );

        let resp = serve(
            Arc::new(cache),
            empty_publisher(),
            empty_posts(),
            headers,
            FeedSurface::Site,
            FeedFormat::Rss,
        )
        .await;
        assert_eq!(resp.status(), StatusCode::NOT_MODIFIED);
    }

    #[test]
    fn invalid_response_metadata_returns_a_sanitized_internal_error() {
        let invalid_header = HeaderValue::from_bytes(b"\n").expect_err("newline is invalid");
        let response =
            finish_cached_response(Err(map_feed_response_metadata_failure(invalid_header)));

        assert_eq!(response.status(), StatusCode::INTERNAL_SERVER_ERROR);
    }

    // guard:no-backend — mock store
    #[tokio::test]
    async fn feed_site_returns_404_on_bad_format() {
        let resp = feed_site(
            Extension(Arc::new(MockFeedCacheStorage::new()) as Arc<dyn FeedCacheStorage>),
            Extension(empty_publisher()),
            Extension(empty_posts()),
            HeaderMap::new(),
            Path(SoftPath::parse("bogus")),
        )
        .await;
        assert_eq!(resp.status(), StatusCode::NOT_FOUND);
    }

    // guard:no-backend — mock store
    #[tokio::test]
    async fn feed_site_delegates_to_serve_on_valid_format() {
        let mut cache = MockFeedCacheStorage::new();
        cache
            .expect_get()
            .returning(|_| Ok(Some(sample_row("\"etag-1\"", UtcInstant::now()))));

        let resp = feed_site(
            Extension(Arc::new(cache) as Arc<dyn FeedCacheStorage>),
            Extension(empty_publisher()),
            Extension(empty_posts()),
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
            Extension(Arc::new(MockFeedCacheStorage::new()) as Arc<dyn FeedCacheStorage>),
            Extension(empty_publisher()),
            Extension(empty_posts()),
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
            Extension(Arc::new(MockFeedCacheStorage::new()) as Arc<dyn FeedCacheStorage>),
            Extension(empty_publisher()),
            Extension(empty_posts()),
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
        let mut cache = MockFeedCacheStorage::new();
        cache
            .expect_get()
            .returning(|_| Ok(Some(sample_row("\"etag-1\"", UtcInstant::now()))));

        let resp = feed_user_tag(
            Extension(Arc::new(cache) as Arc<dyn FeedCacheStorage>),
            Extension(empty_publisher()),
            Extension(empty_posts()),
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
