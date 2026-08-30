use async_trait::async_trait;
use common::ids::UserId;
use common::media::{ByteSize, MediaRef};
use sqlx::{Pool, QueryBuilder, Sqlite};

use crate::InstanceId;
use crate::media::{MediaDialect, MediaStore};
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
        pool: &Pool<Self>,
        user_id: UserId,
        media: &MediaRef,
        current_instance_id: &InstanceId,
        evidence: &MediaReferenceEvidence,
        force: bool,
    ) -> sqlx::Result<bool> {
        let mut tx = pool.begin().await?;
        Self::lock_media_reference(&mut *tx, media).await?;
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
            .push_bind(force);
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
            .fetch_optional(&mut *tx)
            .await?
            .is_some();
        tx.commit().await?;
        Ok(removed)
    }

    async fn media_entry_is_reclaimable(
        pool: &Pool<Self>,
        media: &MediaRef,
        current_instance_id: &InstanceId,
        evidence: &MediaReferenceEvidence,
    ) -> sqlx::Result<bool> {
        // Reclamation may unlink the physical bytes immediately after this query, so
        // it must exclude a concurrent Post reference writer just like deletion. A
        // deferred read transaction would observe an old WAL snapshot while that
        // writer holds the lock; take SQLite's writer lock before the read instead.
        let mut conn = pool.acquire().await?;
        sqlx::query("BEGIN IMMEDIATE").execute(&mut *conn).await?;
        let result: sqlx::Result<bool> = async {
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
        .await;
        match result {
            Ok(reclaimable) => {
                sqlx::query("COMMIT").execute(&mut *conn).await?;
                Ok(reclaimable)
            }
            Err(error) => {
                let rollback = sqlx::query("ROLLBACK").execute(&mut *conn).await.map(drop);
                crate::helpers::preserve_after_secondary(
                    Err(error),
                    rollback,
                    host::error::ErrorKind::Storage,
                    host::error::ErrorClass::Transient,
                    "storage.sqlite.media.reclaimability.rollback",
                )
            }
        }
    }
}
