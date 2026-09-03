use async_trait::async_trait;
use common::media::{ByteSize, MediaRef};
use sqlx::{Pool, Postgres, QueryBuilder};

use crate::InstanceId;
use crate::media::{MediaDeleteMode, MediaDialect, MediaStore};
use crate::posts::media::{self, MediaReferenceEvidence};
use crate::sql::{QueryBuilderStorageExt, QueryStorageExt};
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

    async fn try_delete_media(
        conn: &mut <Self as sqlx::Database>::Connection,
        user_id: UserId,
        media: &MediaRef,
        current_instance_id: &InstanceId,
        evidence: &MediaReferenceEvidence,
        mode: MediaDeleteMode,
    ) -> sqlx::Result<bool> {
        Self::lock_media_reference(conn, media).await?;
        let mut query = QueryBuilder::<Postgres>::new(String::new());
        media::push_media_reference_evidence_cte(&mut query, evidence);
        query.push("DELETE FROM media WHERE user_id = ");
        query
            .push_storage_bind(user_id)
            .push(" AND source = ")
            .push_storage_bind(media.source)
            .push(" AND sha256 = ")
            .push_storage_bind(media.sha256.clone())
            .push(" AND filename = ")
            .push_storage_bind(media.filename.clone())
            .push(" AND (")
            .push_storage_bind(mode);
        query.push(" OR NOT EXISTS (SELECT 1");
        media::push_owner_media_reference_from_where(&mut query, user_id, media);
        media::push_live_media_reference_predicate(&mut query, current_instance_id);
        query.push(")) AND (NOT EXISTS (SELECT 1");
        media::push_other_owner_media_reference_from_where(&mut query, user_id, media);
        media::push_live_media_reference_predicate(&mut query, current_instance_id);
        query.push(") OR EXISTS (SELECT 1 FROM media m2 WHERE m2.source = ");
        query
            .push_storage_bind(media.source)
            .push(" AND m2.sha256 = ")
            .push_storage_bind(media.sha256.clone())
            .push(" AND m2.filename = ")
            .push_storage_bind(media.filename.clone())
            .push(" AND m2.user_id <> ")
            .push_storage_bind(user_id)
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
        Self::lock_media_reference(conn, media).await?;
        let mut query = QueryBuilder::<Postgres>::new(String::new());
        media::push_media_reference_evidence_cte(&mut query, evidence);
        query.push("SELECT 1 WHERE NOT EXISTS (SELECT 1 FROM media WHERE source = ");
        query
            .push_storage_bind(media.source)
            .push(" AND sha256 = ")
            .push_storage_bind(media.sha256.clone())
            .push(" AND filename = ")
            .push_storage_bind(media.filename.clone());
        query.push(") AND NOT EXISTS (SELECT 1");
        media::push_any_media_reference_from_where(&mut query, media);
        media::push_live_media_reference_predicate(&mut query, current_instance_id);
        query.push(")");
        Ok(query
            .build_query_scalar::<i32>()
            .fetch_optional(&mut *conn)
            .await?
            .is_some())
    }
}
