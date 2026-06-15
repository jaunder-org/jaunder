use async_trait::async_trait;
use sqlx::{Pool, Sqlite};

use crate::media::{DeleteMediaError, MediaDialect, MediaStore};

/// SQLite-backed media storage.
pub type SqliteMediaStorage = MediaStore<Sqlite>;

#[async_trait]
impl MediaDialect for Sqlite {
    async fn get_user_upload_usage(pool: &Pool<Sqlite>, user_id: i64) -> sqlx::Result<i64> {
        let row = sqlx::query_as::<_, (i64,)>(
            "SELECT COALESCE(SUM(size_bytes), 0) FROM media WHERE user_id = $1 AND source = 'upload'",
        )
        .bind(user_id)
        .fetch_one(pool)
        .await?;

        Ok(row.0)
    }

    async fn delete_media_row(
        pool: &Pool<Sqlite>,
        user_id: i64,
        sha256: &str,
        filename: &str,
        source: &str,
    ) -> Result<(), DeleteMediaError> {
        let result = sqlx::query(
            "DELETE FROM media WHERE user_id = $1 AND sha256 = $2 AND filename = $3 AND source = $4",
        )
        .bind(user_id)
        .bind(sha256)
        .bind(filename)
        .bind(source)
        .execute(pool)
        .await?;

        if result.rows_affected() == 0 {
            return Err(DeleteMediaError::NotFound);
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::super::sqlite_pool;
    use super::*;
    use crate::{MediaRecord, MediaSource, MediaStorage as _};

    #[tokio::test]
    async fn create_media_with_closed_pool_returns_error() {
        let pool = sqlite_pool().await;
        let storage = SqliteMediaStorage::new(pool.clone());
        pool.close().await;
        let record = MediaRecord {
            user_id: 1,
            sha256: "abc123".to_string(),
            filename: "test.jpg".to_string(),
            source: MediaSource::Upload,
            content_type: "image/jpeg".to_string(),
            size_bytes: 1024,
            source_url: None,
            created_at: chrono::Utc::now(),
        };
        let result = storage.create_media(&record).await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn get_media_with_closed_pool_returns_error() {
        let pool = sqlite_pool().await;
        let storage = SqliteMediaStorage::new(pool.clone());
        pool.close().await;
        let result = storage
            .get_media(1, "abc123", "test.jpg", &MediaSource::Upload)
            .await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn list_media_with_closed_pool_returns_error() {
        let pool = sqlite_pool().await;
        let storage = SqliteMediaStorage::new(pool.clone());
        pool.close().await;
        let result = storage.list_media(1, None, 10, 0).await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn delete_media_with_closed_pool_returns_error() {
        let pool = sqlite_pool().await;
        let storage = SqliteMediaStorage::new(pool.clone());
        pool.close().await;
        let result = storage
            .delete_media(1, "abc123", "test.jpg", &MediaSource::Upload)
            .await;
        assert!(matches!(result, Err(DeleteMediaError::Internal(_))));
    }

    #[tokio::test]
    async fn get_user_upload_usage_with_closed_pool_returns_error() {
        let pool = sqlite_pool().await;
        let storage = SqliteMediaStorage::new(pool.clone());
        pool.close().await;
        let result = storage.get_user_upload_usage(1).await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn find_by_hash_with_closed_pool_returns_error() {
        let pool = sqlite_pool().await;
        let storage = SqliteMediaStorage::new(pool.clone());
        pool.close().await;
        let result = storage.find_by_hash("abc123", &MediaSource::Upload).await;
        assert!(result.is_err());
    }
}
