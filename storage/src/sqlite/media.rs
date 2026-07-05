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
