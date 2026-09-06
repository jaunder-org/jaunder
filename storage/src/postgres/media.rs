use async_trait::async_trait;
use common::ids::UserId;
use common::media::{ByteSize, MediaRef};
use sqlx::{Pool, Postgres};

use crate::media::{MediaDialect, MediaStore};
use crate::posts::media;
use crate::sql::QueryStorageExt;

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
        .bind_storage(user_id)
        .fetch_one(pool)
        .await?;

        Ok(row.0)
    }

    async fn lock_media_reference(
        conn: &mut <Self as sqlx::Database>::Connection,
        media: &MediaRef,
    ) -> sqlx::Result<()> {
        let key = media::media_advisory_lock_key(media);
        sqlx::query("SELECT pg_advisory_xact_lock($1)")
            .bind_storage(key)
            .execute(conn)
            .await?;
        Ok(())
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
