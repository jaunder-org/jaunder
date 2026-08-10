use async_trait::async_trait;
use common::media::ByteSize;
use sqlx::{Pool, Sqlite};

use crate::error::{StorageError, fetch_exactly_one_scalar};
use crate::media::{MediaDialect, MediaStore};
use common::ids::UserId;

/// SQLite-backed media storage.
pub type SqliteMediaStorage = MediaStore<Sqlite>;

#[async_trait]
impl MediaDialect for Sqlite {
    async fn get_user_upload_usage(
        pool: &Pool<Sqlite>,
        user_id: UserId,
    ) -> Result<ByteSize, StorageError> {
        // A bare aggregate always returns exactly one row, so the `MissingRow`
        // arm is unreachable today. Routed through the wrapper anyway: it is
        // `fetch_one` that #343 removes, and the `what` string names the row the
        // day a `GROUP BY` makes it optional.
        fetch_exactly_one_scalar(
            sqlx::query_scalar::<_, ByteSize>(
                "SELECT COALESCE(SUM(size_bytes), 0) FROM media WHERE user_id = $1 AND source = 'upload'",
            )
            .bind(user_id),
            pool,
            "the media upload-usage total",
        )
        .await
    }
}
