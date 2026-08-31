use async_trait::async_trait;
use common::ids::UserId;
use common::media::{ByteSize, MediaRef};
use sqlx::{Pool, QueryBuilder, Sqlite};

use crate::InstanceId;
use crate::media::{MediaDeleteMode, MediaDialect, MediaStore};
use crate::posts::{self, MediaReferenceEvidence};

/// SQLite-backed media storage.
pub type SqliteMediaStorage = MediaStore<Sqlite>;

#[async_trait]
impl MediaDialect for Sqlite {
    async fn get_user_upload_usage(pool: &Pool<Sqlite>, user_id: UserId) -> sqlx::Result<ByteSize> {
        let row = sqlx::query_as::<_, (ByteSize,)>(
            "SELECT COALESCE(SUM(size_bytes), 0) FROM media WHERE user_id = $1 AND source = 'upload'",
        )
        .bind(user_id)
        .fetch_one(pool)
        .await?;

        Ok(row.0)
    }

    async fn lock_media_reference(
        _conn: &mut <Self as sqlx::Database>::Connection,
        _media: &MediaRef,
    ) -> sqlx::Result<()> {
        // SQLite serializes writers; the guarded statement is the first write.
        Ok(())
    }

    async fn total_upload_bytes(pool: &Pool<Sqlite>) -> sqlx::Result<ByteSize> {
        let row = sqlx::query_as::<_, (ByteSize,)>(
            "SELECT COALESCE(SUM(size_bytes), 0) FROM media WHERE source = 'upload'",
        )
        .fetch_one(pool)
        .await?;

        Ok(row.0)
    }

    async fn try_delete_media(
        conn: &mut <Self as sqlx::Database>::Connection,
        user_id: UserId,
        media: &MediaRef,
        current_instance_id: &InstanceId,
        evidence: &MediaReferenceEvidence,
        mode: MediaDeleteMode,
    ) -> sqlx::Result<bool> {
        Self::lock_media_reference(conn, media).await?;
        let mut query = QueryBuilder::<Sqlite>::new(String::new());
        posts::push_media_reference_evidence_cte(&mut query, evidence);
        query.push("DELETE FROM media WHERE user_id = ");
        query
            .push_bind(user_id)
            .push(" AND source = ")
            .push_bind(media.source)
            .push(" AND sha256 = ")
            .push_bind(media.sha256.clone())
            .push(" AND filename = ")
            .push_bind(media.filename.clone())
            .push(" AND (")
            .push_bind(mode);
        query.push(" OR NOT EXISTS (SELECT 1");
        posts::push_owner_media_reference_from_where(&mut query, user_id, media);
        posts::push_live_media_reference_predicate(&mut query, current_instance_id);
        query.push(")) AND (NOT EXISTS (SELECT 1");
        posts::push_other_owner_media_reference_from_where(&mut query, user_id, media);
        posts::push_live_media_reference_predicate(&mut query, current_instance_id);
        query.push(") OR EXISTS (SELECT 1 FROM media m2 WHERE m2.source = ");
        query
            .push_bind(media.source)
            .push(" AND m2.sha256 = ")
            .push_bind(media.sha256.clone())
            .push(" AND m2.filename = ")
            .push_bind(media.filename.clone())
            .push(" AND m2.user_id <> ")
            .push_bind(user_id)
            .push(")) RETURNING 1");
        let removed = query
            .build_query_scalar::<i32>()
            .fetch_optional(&mut *conn)
            .await?
            .is_some();
        Ok(removed)
    }

    async fn media_entry_is_reclaimable(
        conn: &mut <Self as sqlx::Database>::Connection,
        media: &MediaRef,
        current_instance_id: &InstanceId,
        evidence: &MediaReferenceEvidence,
    ) -> sqlx::Result<bool> {
        // The caller-owned WriteScope has begun a SQLite write transaction, which
        // serializes Post writers until its callback has completed the unlink.
        Self::lock_media_reference(conn, media).await?;
        let mut query = QueryBuilder::<Sqlite>::new(String::new());
        posts::push_media_reference_evidence_cte(&mut query, evidence);
        query.push("SELECT 1 WHERE NOT EXISTS (SELECT 1 FROM media WHERE source = ");
        query
            .push_bind(media.source)
            .push(" AND sha256 = ")
            .push_bind(media.sha256.clone())
            .push(" AND filename = ")
            .push_bind(media.filename.clone());
        query.push(") AND NOT EXISTS (SELECT 1");
        posts::push_any_media_reference_from_where(&mut query, media);
        posts::push_live_media_reference_predicate(&mut query, current_instance_id);
        query.push(")");
        Ok(query
            .build_query_scalar::<i32>()
            .fetch_optional(&mut *conn)
            .await?
            .is_some())
    }
}
