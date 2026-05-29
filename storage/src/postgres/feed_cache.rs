use async_trait::async_trait;
use chrono::{DateTime, Utc};
use sqlx::PgPool;

use crate::feed_cache::{FeedCacheError, FeedCacheRow, FeedCacheStorage};

pub struct PostgresFeedCacheStorage {
    pool: PgPool,
}

impl PostgresFeedCacheStorage {
    #[must_use]
    pub fn new(pool: PgPool) -> Self {
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
impl FeedCacheStorage for PostgresFeedCacheStorage {
    #[tracing::instrument(name = "storage.postgres.feed_cache.get", skip(self))]
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

    #[tracing::instrument(name = "storage.postgres.feed_cache.upsert", skip(self, row))]
    async fn upsert(&self, row: FeedCacheRow) -> Result<(), FeedCacheError> {
        sqlx::query(
            "INSERT INTO feed_cache (feed_url, body, etag, content_type, updated_at, generated_at) \
             VALUES ($1, $2, $3, $4, $5, $6) \
             ON CONFLICT (feed_url) DO UPDATE SET \
               body = EXCLUDED.body, \
               etag = EXCLUDED.etag, \
               content_type = EXCLUDED.content_type, \
               updated_at = EXCLUDED.updated_at, \
               generated_at = EXCLUDED.generated_at",
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

    #[tracing::instrument(name = "storage.postgres.feed_cache.delete", skip(self))]
    async fn delete(&self, feed_url: &str) -> Result<(), FeedCacheError> {
        sqlx::query("DELETE FROM feed_cache WHERE feed_url = $1")
            .bind(feed_url)
            .execute(&self.pool)
            .await?;
        Ok(())
    }
}
