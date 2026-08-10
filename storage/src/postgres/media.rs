use async_trait::async_trait;
use common::media::ByteSize;
use sqlx::{Pool, Postgres};

use crate::error::{StorageError, fetch_exactly_one_scalar};
use crate::media::{MediaDialect, MediaStore};
use common::ids::UserId;

/// Postgres-backed media storage.
pub type PostgresMediaStorage = MediaStore<Postgres>;

#[async_trait]
impl MediaDialect for Postgres {
    async fn get_user_upload_usage(
        pool: &Pool<Postgres>,
        user_id: UserId,
    ) -> Result<ByteSize, StorageError> {
        // The `SQLite` twin, modulo the explicit `::bigint` cast; same
        // row-guaranteed aggregate, same wrapper, same `what` string.
        fetch_exactly_one_scalar(
            sqlx::query_scalar::<_, ByteSize>(
                "SELECT COALESCE(SUM(size_bytes), 0)::bigint FROM media WHERE user_id = $1 AND source = 'upload'",
            )
            .bind(user_id),
            pool,
            "the media upload-usage total",
        )
        .await
    }
}
