use async_trait::async_trait;
use common::media::ByteSize;
use sqlx::{Pool, Sqlite};

use crate::media::{MediaDialect, MediaStore};
use common::ids::UserId;

/// SQLite-backed media storage.
pub type SqliteMediaStorage = MediaStore<Sqlite>;

#[async_trait]
impl MediaDialect for Sqlite {
    async fn get_user_upload_usage(pool: &Pool<Sqlite>, user_id: UserId) -> sqlx::Result<ByteSize> {
        let row = sqlx::query_as::<_, (ByteSize,)>(
            "SELECT COALESCE(SUM(size_bytes), 0) FROM media WHERE user_id = $1 AND source = 'upload'",
        )
        .bind(user_id)
        // `fetch_optional`, not the banned `fetch_one` (#343): a bare aggregate
        // always yields one row, and the impossible `None` folds into the same
        // zero a user with no uploads gives.
        .fetch_optional(pool)
        .await?;

        let Some((bytes,)) = row else {
            unreachable!("a bare COALESCE(SUM(...)) aggregate always yields one row");
        };
        Ok(bytes)
    }
}
