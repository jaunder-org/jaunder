use async_trait::async_trait;
use common::media::ByteSize;
use sqlx::{Pool, Postgres};

use crate::media::{MediaDialect, MediaStore};
use common::ids::UserId;

/// Postgres-backed media storage.
pub type PostgresMediaStorage = MediaStore<Postgres>;

#[async_trait]
impl MediaDialect for Postgres {
    async fn get_user_upload_usage(
        pool: &Pool<Postgres>,
        user_id: UserId,
    ) -> sqlx::Result<ByteSize> {
        let row = sqlx::query_as::<_, (ByteSize,)>(
            "SELECT COALESCE(SUM(size_bytes), 0)::bigint FROM media WHERE user_id = $1 AND source = 'upload'",
        )
        .bind(user_id)
        .fetch_one(pool)
        .await?;

        Ok(row.0)
    }

    async fn total_upload_bytes(pool: &Pool<Postgres>) -> sqlx::Result<ByteSize> {
        let row = sqlx::query_as::<_, (ByteSize,)>(
            "SELECT COALESCE(SUM(size_bytes), 0)::bigint FROM media WHERE source = 'upload'",
        )
        .fetch_one(pool)
        .await?;

        Ok(row.0)
    }
}
