//! Cached, fully-rendered feed bodies keyed by their canonical (decoded) path
//! form. The cache layer is the single source of truth for what bytes get
//! served by `GET /feed.{rss,atom,json}` and the other feed endpoints.

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use sqlx::{Database, Pool};
use thiserror::Error;

use crate::backend::Backend;

/// A single cached feed body.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FeedCacheRow {
    pub feed_url: String,
    pub body: String,
    pub etag: String,
    pub content_type: String,
    pub updated_at: DateTime<Utc>,
    pub generated_at: DateTime<Utc>,
}

#[derive(Debug, Error)]
pub enum FeedCacheError {
    #[error("database error: {0}")]
    Db(#[from] sqlx::Error),
}

#[cfg_attr(feature = "test-utils", mockall::automock)]
#[async_trait]
pub trait FeedCacheStorage: Send + Sync {
    async fn get(&self, feed_url: &str) -> Result<Option<FeedCacheRow>, FeedCacheError>;
    async fn upsert(&self, row: FeedCacheRow) -> Result<(), FeedCacheError>;
    async fn delete(&self, feed_url: &str) -> Result<(), FeedCacheError>;
}

type CacheTuple = (String, String, String, String, DateTime<Utc>, DateTime<Utc>);

fn row_from_tuple(t: CacheTuple) -> FeedCacheRow {
    FeedCacheRow {
        feed_url: t.0,
        body: t.1,
        etag: t.2,
        content_type: t.3,
        updated_at: t.4,
        generated_at: t.5,
    }
}

/// Generic [`FeedCacheStorage`] backed by any [`Backend`] database.
///
/// Zero backend divergence (identical SQL across `SQLite` and Postgres),
/// so it is implemented once here; see ADR-0019.
pub struct FeedCacheStore<DB: Database> {
    pool: Pool<DB>,
}

impl<DB: Database> FeedCacheStore<DB> {
    #[must_use]
    pub fn new(pool: Pool<DB>) -> Self {
        Self { pool }
    }
}

#[async_trait]
impl<DB> FeedCacheStorage for FeedCacheStore<DB>
where
    DB: Backend,
    CacheTuple: for<'r> sqlx::FromRow<'r, DB::Row>,
    for<'q> &'q str: sqlx::Encode<'q, DB> + sqlx::Type<DB>,
    for<'q> DateTime<Utc>: sqlx::Encode<'q, DB> + sqlx::Type<DB>,
    for<'c> &'c Pool<DB>: sqlx::Executor<'c, Database = DB>,
    for<'q> DB::Arguments<'q>: sqlx::IntoArguments<'q, DB>,
{
    #[tracing::instrument(
        name = "storage.feed_cache.get",
        skip(self),
        fields(db.system = DB::DB_SYSTEM)
    )]
    async fn get(&self, feed_url: &str) -> Result<Option<FeedCacheRow>, FeedCacheError> {
        let row = sqlx::query_as::<_, CacheTuple>(
            "SELECT feed_url, body, etag, content_type, updated_at, generated_at \
             FROM feed_cache WHERE feed_url = $1",
        )
        .bind(feed_url)
        .fetch_optional(&self.pool)
        .await?;
        Ok(row.map(row_from_tuple))
    }

    #[tracing::instrument(
        name = "storage.feed_cache.upsert",
        skip(self, row),
        fields(db.system = DB::DB_SYSTEM)
    )]
    async fn upsert(&self, row: FeedCacheRow) -> Result<(), FeedCacheError> {
        sqlx::query(
            "INSERT INTO feed_cache (feed_url, body, etag, content_type, updated_at, generated_at) \
             VALUES ($1, $2, $3, $4, $5, $6) \
             ON CONFLICT(feed_url) DO UPDATE SET \
               body = excluded.body, \
               etag = excluded.etag, \
               content_type = excluded.content_type, \
               updated_at = excluded.updated_at, \
               generated_at = excluded.generated_at",
        )
        .bind(row.feed_url.as_str())
        .bind(row.body.as_str())
        .bind(row.etag.as_str())
        .bind(row.content_type.as_str())
        .bind(row.updated_at)
        .bind(row.generated_at)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    #[tracing::instrument(
        name = "storage.feed_cache.delete",
        skip(self),
        fields(db.system = DB::DB_SYSTEM)
    )]
    async fn delete(&self, feed_url: &str) -> Result<(), FeedCacheError> {
        sqlx::query("DELETE FROM feed_cache WHERE feed_url = $1")
            .bind(feed_url)
            .execute(&self.pool)
            .await?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::{backends, Backend};
    use rstest::*;
    use rstest_reuse::*;

    fn sample(url: &str) -> FeedCacheRow {
        FeedCacheRow {
            feed_url: url.into(),
            body: "<rss/>".into(),
            etag: "\"sha256-deadbeef\"".into(),
            content_type: "application/rss+xml".into(),
            updated_at: Utc::now(),
            generated_at: Utc::now(),
        }
    }

    #[apply(backends)]
    #[tokio::test]
    async fn upsert_then_get_returns_row(#[case] backend: Backend) {
        let env = backend.setup().await;
        env.state
            .feed_cache
            .upsert(sample("/feed.rss"))
            .await
            .unwrap();
        let got = env
            .state
            .feed_cache
            .get("/feed.rss")
            .await
            .unwrap()
            .expect("present");
        assert_eq!(got.feed_url, "/feed.rss");
        assert_eq!(got.body, "<rss/>");
    }

    #[apply(backends)]
    #[tokio::test]
    async fn second_upsert_updates_existing_body(#[case] backend: Backend) {
        let env = backend.setup().await;
        let mut row = sample("/feed.rss");
        env.state.feed_cache.upsert(row.clone()).await.unwrap();
        row.body = "<rss>updated</rss>".into();
        env.state.feed_cache.upsert(row).await.unwrap();
        let got = env
            .state
            .feed_cache
            .get("/feed.rss")
            .await
            .unwrap()
            .unwrap();
        assert_eq!(got.body, "<rss>updated</rss>");
    }

    #[apply(backends)]
    #[tokio::test]
    async fn get_missing_returns_none(#[case] backend: Backend) {
        let env = backend.setup().await;
        assert!(env
            .state
            .feed_cache
            .get("/missing")
            .await
            .unwrap()
            .is_none());
    }

    #[apply(backends)]
    #[tokio::test]
    async fn delete_removes_row(#[case] backend: Backend) {
        let env = backend.setup().await;
        env.state
            .feed_cache
            .upsert(sample("/feed.rss"))
            .await
            .unwrap();
        env.state.feed_cache.delete("/feed.rss").await.unwrap();
        assert!(env
            .state
            .feed_cache
            .get("/feed.rss")
            .await
            .unwrap()
            .is_none());
    }
}
