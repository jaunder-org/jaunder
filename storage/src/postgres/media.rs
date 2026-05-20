use async_trait::async_trait;
use sqlx::PgPool;

use crate::helpers::{media_record_from_row, MediaRow};
use crate::{CreateMediaError, DeleteMediaError, MediaRecord, MediaSource, MediaStorage};

pub struct PostgresMediaStorage {
    pool: PgPool,
}

impl PostgresMediaStorage {
    #[must_use]
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }
}

#[async_trait]
impl MediaStorage for PostgresMediaStorage {
    #[tracing::instrument(name = "storage.postgres.media.create", skip(self, record))]
    async fn create_media(&self, record: &MediaRecord) -> Result<(), CreateMediaError> {
        let result = sqlx::query(
            "INSERT INTO media (user_id, sha256, filename, source, content_type, size_bytes, source_url, created_at)
             VALUES ($1, $2, $3, $4, $5, $6, $7, $8)",
        )
        .bind(record.user_id)
        .bind(&record.sha256)
        .bind(&record.filename)
        .bind(record.source.as_str())
        .bind(&record.content_type)
        .bind(record.size_bytes)
        .bind(&record.source_url)
        .bind(record.created_at)
        .execute(&self.pool)
        .await;

        match result {
            Ok(_) => Ok(()),
            Err(e)
                if e.as_database_error()
                    .is_some_and(sqlx::error::DatabaseError::is_unique_violation) =>
            {
                Err(CreateMediaError::AlreadyExists)
            }
            Err(e) => Err(CreateMediaError::Internal(e)),
        }
    }

    #[tracing::instrument(name = "storage.postgres.media.get", skip(self))]
    async fn get_media(
        &self,
        user_id: i64,
        sha256: &str,
        filename: &str,
        source: &MediaSource,
    ) -> sqlx::Result<Option<MediaRecord>> {
        let row = sqlx::query_as::<_, MediaRow>(
            "SELECT user_id, sha256, filename, source, content_type, size_bytes, source_url, created_at
             FROM media
             WHERE user_id = $1 AND sha256 = $2 AND filename = $3 AND source = $4",
        )
        .bind(user_id)
        .bind(sha256)
        .bind(filename)
        .bind(source.as_str())
        .fetch_optional(&self.pool)
        .await?;

        row.map(media_record_from_row).transpose()
    }

    #[tracing::instrument(name = "storage.postgres.media.list", skip(self))]
    async fn list_media(
        &self,
        user_id: i64,
        source: Option<&MediaSource>,
        limit: u32,
        offset: u32,
    ) -> sqlx::Result<Vec<MediaRecord>> {
        let rows = if let Some(src) = source {
            sqlx::query_as::<_, MediaRow>(
                "SELECT user_id, sha256, filename, source, content_type, size_bytes, source_url, created_at
                 FROM media
                 WHERE user_id = $1 AND source = $2
                 ORDER BY created_at DESC
                 LIMIT $3 OFFSET $4",
            )
            .bind(user_id)
            .bind(src.as_str())
            .bind(i64::from(limit))
            .bind(i64::from(offset))
            .fetch_all(&self.pool)
            .await?
        } else {
            sqlx::query_as::<_, MediaRow>(
                "SELECT user_id, sha256, filename, source, content_type, size_bytes, source_url, created_at
                 FROM media
                 WHERE user_id = $1
                 ORDER BY created_at DESC
                 LIMIT $2 OFFSET $3",
            )
            .bind(user_id)
            .bind(i64::from(limit))
            .bind(i64::from(offset))
            .fetch_all(&self.pool)
            .await?
        };

        rows.into_iter().map(media_record_from_row).collect()
    }

    #[tracing::instrument(name = "storage.postgres.media.delete", skip(self))]
    async fn delete_media(
        &self,
        user_id: i64,
        sha256: &str,
        filename: &str,
        source: &MediaSource,
    ) -> Result<(), DeleteMediaError> {
        let result = sqlx::query(
            "DELETE FROM media WHERE user_id = $1 AND sha256 = $2 AND filename = $3 AND source = $4",
        )
        .bind(user_id)
        .bind(sha256)
        .bind(filename)
        .bind(source.as_str())
        .execute(&self.pool)
        .await?;

        if result.rows_affected() == 0 {
            return Err(DeleteMediaError::NotFound);
        }
        Ok(())
    }

    #[tracing::instrument(name = "storage.postgres.media.upload_usage", skip(self))]
    async fn get_user_upload_usage(&self, user_id: i64) -> sqlx::Result<i64> {
        let row = sqlx::query_as::<_, (i64,)>(
            "SELECT COALESCE(SUM(size_bytes), 0)::bigint FROM media WHERE user_id = $1 AND source = 'upload'",
        )
        .bind(user_id)
        .fetch_one(&self.pool)
        .await?;

        Ok(row.0)
    }

    #[tracing::instrument(name = "storage.postgres.media.find_by_hash", skip(self))]
    async fn find_by_hash(
        &self,
        sha256: &str,
        source: &MediaSource,
    ) -> sqlx::Result<Option<MediaRecord>> {
        let row = sqlx::query_as::<_, MediaRow>(
            "SELECT user_id, sha256, filename, source, content_type, size_bytes, source_url, created_at
             FROM media
             WHERE sha256 = $1 AND source = $2
             LIMIT 1",
        )
        .bind(sha256)
        .bind(source.as_str())
        .fetch_optional(&self.pool)
        .await?;

        row.map(media_record_from_row).transpose()
    }
}

#[cfg(test)]
mod tests {
    use super::super::postgres_pool;
    use super::*;

    #[tokio::test]
    #[ignore = "requires PostgreSQL test VM"]
    async fn create_media_with_closed_pool_returns_error() {
        let pool = postgres_pool().await;
        let storage = PostgresMediaStorage::new(pool.clone());
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
    #[ignore = "requires PostgreSQL test VM"]
    async fn get_media_with_closed_pool_returns_error() {
        let pool = postgres_pool().await;
        let storage = PostgresMediaStorage::new(pool.clone());
        pool.close().await;
        let result = storage
            .get_media(1, "abc123", "test.jpg", &MediaSource::Upload)
            .await;
        assert!(result.is_err());
    }

    #[tokio::test]
    #[ignore = "requires PostgreSQL test VM"]
    async fn list_media_with_closed_pool_returns_error() {
        let pool = postgres_pool().await;
        let storage = PostgresMediaStorage::new(pool.clone());
        pool.close().await;
        let result = storage.list_media(1, None, 10, 0).await;
        assert!(result.is_err());
    }

    #[tokio::test]
    #[ignore = "requires PostgreSQL test VM"]
    async fn delete_media_with_closed_pool_returns_error() {
        let pool = postgres_pool().await;
        let storage = PostgresMediaStorage::new(pool.clone());
        pool.close().await;
        let result = storage
            .delete_media(1, "abc123", "test.jpg", &MediaSource::Upload)
            .await;
        assert!(matches!(result, Err(DeleteMediaError::Internal(_))));
    }

    #[tokio::test]
    #[ignore = "requires PostgreSQL test VM"]
    async fn get_user_upload_usage_with_closed_pool_returns_error() {
        let pool = postgres_pool().await;
        let storage = PostgresMediaStorage::new(pool.clone());
        pool.close().await;
        let result = storage.get_user_upload_usage(1).await;
        assert!(result.is_err());
    }

    #[tokio::test]
    #[ignore = "requires PostgreSQL test VM"]
    async fn find_by_hash_with_closed_pool_returns_error() {
        let pool = postgres_pool().await;
        let storage = PostgresMediaStorage::new(pool.clone());
        pool.close().await;
        let result = storage.find_by_hash("abc123", &MediaSource::Upload).await;
        assert!(result.is_err());
    }
}
