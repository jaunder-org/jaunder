use sqlx::Sqlite;

use crate::feed_cache::FeedCacheStore;

/// SQLite-backed feed-cache storage.
pub type SqliteFeedCacheStorage = FeedCacheStore<Sqlite>;

#[cfg(test)]
mod tests {
    use chrono::Utc;
    use sqlx::SqlitePool;

    use super::*;
    use crate::feed_cache::FeedCacheStorage;

    async fn pool() -> SqlitePool {
        let pool = SqlitePool::connect("sqlite::memory:").await.unwrap();
        sqlx::migrate!("./migrations/sqlite")
            .run(&pool)
            .await
            .unwrap();
        pool
    }

    fn sample(url: &str) -> crate::feed_cache::FeedCacheRow {
        crate::feed_cache::FeedCacheRow {
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
