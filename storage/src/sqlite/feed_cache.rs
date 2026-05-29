use async_trait::async_trait;
use chrono::{DateTime, Utc};
use sqlx::SqlitePool;

use crate::feed_cache::{FeedCacheError, FeedCacheRow, FeedCacheStorage};

pub struct SqliteFeedCacheStorage {
    pool: SqlitePool,
}

impl SqliteFeedCacheStorage {
    #[must_use]
    pub fn new(pool: SqlitePool) -> Self {
        Self { pool }
    }
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

#[async_trait]
impl FeedCacheStorage for SqliteFeedCacheStorage {
    #[tracing::instrument(name = "storage.sqlite.feed_cache.get", skip(self))]
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

    #[tracing::instrument(name = "storage.sqlite.feed_cache.upsert", skip(self, row))]
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
        .bind(&row.feed_url)
        .bind(&row.body)
        .bind(&row.etag)
        .bind(&row.content_type)
        .bind(row.updated_at)
        .bind(row.generated_at)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    #[tracing::instrument(name = "storage.sqlite.feed_cache.delete", skip(self))]
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
    use chrono::Utc;
    use sqlx::SqlitePool;

    use super::*;

    async fn pool() -> SqlitePool {
        let pool = SqlitePool::connect("sqlite::memory:").await.unwrap();
        sqlx::migrate!("./migrations/sqlite")
            .run(&pool)
            .await
            .unwrap();
        pool
    }

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

    #[tokio::test]
    async fn upsert_then_get_returns_row() {
        let s = SqliteFeedCacheStorage::new(pool().await);
        s.upsert(sample("/feed.rss")).await.unwrap();
        let got = s.get("/feed.rss").await.unwrap().expect("present");
        assert_eq!(got.feed_url, "/feed.rss");
        assert_eq!(got.body, "<rss/>");
    }

    #[tokio::test]
    async fn second_upsert_updates_existing_body() {
        let s = SqliteFeedCacheStorage::new(pool().await);
        let mut row = sample("/feed.rss");
        s.upsert(row.clone()).await.unwrap();
        row.body = "<rss>updated</rss>".into();
        s.upsert(row).await.unwrap();
        let got = s.get("/feed.rss").await.unwrap().unwrap();
        assert_eq!(got.body, "<rss>updated</rss>");
    }

    #[tokio::test]
    async fn get_missing_returns_none() {
        let s = SqliteFeedCacheStorage::new(pool().await);
        assert!(s.get("/missing").await.unwrap().is_none());
    }

    #[tokio::test]
    async fn delete_removes_row() {
        let s = SqliteFeedCacheStorage::new(pool().await);
        s.upsert(sample("/feed.rss")).await.unwrap();
        s.delete("/feed.rss").await.unwrap();
        assert!(s.get("/feed.rss").await.unwrap().is_none());
    }
}
