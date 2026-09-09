//! Media file metadata storage.

use async_trait::async_trait;
use common::ids::{PostId, UserId};
use common::media::{ByteSize, ContentHash, ContentType, Filename, MediaRef, MediaSource};
use common::pagination::{PageOffset, RowLimit};
use common::tagged_url::MediaSourceUrl;
use common::time::UtcInstant;
use sqlx::{
    ColumnIndex, Database, Decode, Encode, Executor, FromRow, Pool, QueryBuilder, Result, Row, Type,
};

use crate::InstanceId;
use crate::WriteTransaction;
use crate::backend::Backend;
use crate::posts::media::MediaReferenceEvidence;
use crate::sql::{QueryBuilderStorageExt, QueryStorageExt, RowCount};
use thiserror::Error;

/// A media metadata record returned by [`MediaStorage`] queries.
#[derive(Clone, Debug)]
pub struct MediaRecord {
    /// ID of the user who owns or triggered the caching of this media.
    pub user_id: UserId,
    /// SHA-256 content hash of the file (used for content-addressing and dedup).
    pub sha256: ContentHash,
    /// Original filename or a generated unique name.
    pub filename: Filename,
    /// Whether the media is a local upload or a remote cache.
    pub source: MediaSource,
    /// MIME type (e.g., "image/jpeg").
    pub content_type: ContentType,
    /// Size of the file in bytes.
    pub size_bytes: ByteSize,
    /// For cached media, the original remote URL; `None` for a local upload.
    ///
    /// Typed as [`MediaSourceUrl`] ahead of any writer: every construction site currently
    /// passes `None`, because the remote-caching ingest that would populate it does not
    /// exist yet. The type is therefore the **contract for that path** — whoever builds it
    /// must supply a validated, normalized `http(s)` URL rather than whatever a feed handed
    /// them. An unparseable value would be useless by definition, since caching means
    /// fetching this URL, so rejecting it at ingest is strictly better than storing
    /// something no code can act on (#675).
    pub source_url: Option<MediaSourceUrl>,
    /// When the record was created.
    pub created_at: UtcInstant,
}

/// Decodes the media projection directly into its storage record.
impl<'r, R> sqlx::FromRow<'r, R> for MediaRecord
where
    R: Row,
    &'r str: sqlx::ColumnIndex<R>,
    UserId: Decode<'r, R::Database> + Type<R::Database>,
    ContentHash: Decode<'r, R::Database> + Type<R::Database>,
    Filename: Decode<'r, R::Database> + Type<R::Database>,
    MediaSource: Decode<'r, R::Database> + Type<R::Database>,
    ContentType: Decode<'r, R::Database> + Type<R::Database>,
    ByteSize: Decode<'r, R::Database> + Type<R::Database>,
    Option<MediaSourceUrl>: Decode<'r, R::Database> + Type<R::Database>,
    UtcInstant: Decode<'r, R::Database> + Type<R::Database>,
{
    fn from_row(row: &'r R) -> Result<Self> {
        let user_id = row.try_get::<UserId, _>("user_id")?;
        let sha256 = row.try_get::<ContentHash, _>("sha256")?;
        let filename = row.try_get::<Filename, _>("filename")?;
        let source = row.try_get::<MediaSource, _>("source")?;
        let content_type = row.try_get::<ContentType, _>("content_type")?;
        let size_bytes = row.try_get::<ByteSize, _>("size_bytes")?;
        let source_url = row.try_get::<Option<MediaSourceUrl>, _>("source_url")?;
        let created_at = row.try_get::<UtcInstant, _>("created_at")?;

        Ok(Self {
            user_id,
            sha256,
            filename,
            source,
            content_type,
            size_bytes,
            source_url,
            created_at,
        })
    }
}

/// Errors that can occur when creating a media record.
#[derive(Debug, Error)]
pub enum CreateMediaError {
    /// A record with the same composite key already exists.
    #[error("media already exists")]
    AlreadyExists,
    /// An unexpected database error occurred.
    #[error(transparent)]
    Internal(#[from] sqlx::Error),
}

/// Errors that can occur while deciding a media deletion.
#[derive(Debug, Error)]
pub enum DeleteMediaError {
    /// An unexpected database error occurred.
    #[error(transparent)]
    Internal(#[from] sqlx::Error),
}

/// The locked, public classification of a media deletion attempt.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TryDeleteOutcome {
    /// The record was removed.
    Deleted,
    /// The requested record was absent.
    Missing,
    /// Retained history belonging only to the authenticated owner prevents deletion.
    OwnerRetainedHistory(Vec<PostId>),
    /// Evidence outside reportable owner history prevents safe deletion.
    GlobalSafety,
}

/// A storage-issued lease proving the caller still holds the transaction and media
/// lock that made filesystem reclamation safe.
///
/// Only storage can mint the lease. The caller's write-scope callback keeps the
/// transaction alive until this token is explicitly finalized after unlink.
pub struct ReclaimGuard {
    _private: (),
}

impl ReclaimGuard {
    pub(crate) const fn new() -> Self {
        Self { _private: () }
    }

    /// Releases the lease after the caller's filesystem action has completed.
    pub fn finalize(self) {}
}

/// Whether media deletion may knowingly break reportable owner retained history.
#[derive(Clone, Copy, Debug, PartialEq, Eq, macros::SqlxBridge)]
pub struct MediaDeleteMode(bool);

impl MediaDeleteMode {
    /// Refuse deletion when the requesting owner's retained history names media.
    pub const GUARDED: Self = Self(false);
    /// Permit deletion despite the requesting owner's retained history.
    pub const FORCED: Self = Self(true);
}

/// Async operations on the `media` table.
///
/// This trait manages the metadata for media files, supporting both user
/// uploads and cached remote content.
#[cfg_attr(any(test, feature = "test-utils"), mockall::automock)]
#[async_trait]
pub trait MediaStorage: Send + Sync {
    /// Inserts a new media record.
    ///
    /// # Errors
    ///
    /// Returns [`CreateMediaError::AlreadyExists`] if a record with the same
    /// hash, filename, and source exists for the user.
    async fn create_media(
        &self,
        transaction: &mut WriteTransaction,
        record: &MediaRecord,
    ) -> Result<(), CreateMediaError>;

    /// Acquires the caller-owned transaction's lock for one media identity.
    ///
    /// This is an internal coordination operation, deliberately excluded from the
    /// audited media-storage operation census. Callers retain the transaction while
    /// performing every filesystem mutation that depends on this identity.
    async fn lock_media_reference(
        &self,
        transaction: &mut WriteTransaction,
        media: &MediaRef,
    ) -> Result<()>;

    /// Fetches a single media record by its composite key.
    async fn get_media(
        &self,
        user_id: UserId,
        sha256: &ContentHash,
        filename: &Filename,
        source: &MediaSource,
    ) -> Result<Option<MediaRecord>>;

    /// Lists media records for a user, with optional filtering and pagination.
    async fn list_media<'a>(
        &self,
        user_id: UserId,
        source: Option<&'a MediaSource>,
        limit: RowLimit,
        offset: PageOffset,
    ) -> Result<Vec<MediaRecord>>;

    /// Atomically deletes a record or returns its locked safety classification.
    ///
    /// The conditional delete and all cold-path classification queries run under
    /// the same write transaction and media lock.
    async fn try_delete_media(
        &self,
        transaction: &mut WriteTransaction,
        user_id: UserId,
        media: &MediaRef,
        current_instance_id: &InstanceId,
        evidence: &MediaReferenceEvidence,
        mode: MediaDeleteMode,
    ) -> Result<TryDeleteOutcome, DeleteMediaError>;

    async fn reclaim_guard(
        &self,
        transaction: &mut WriteTransaction,
        media: &MediaRef,
        current_instance_id: &InstanceId,
        evidence: &MediaReferenceEvidence,
    ) -> Result<Option<ReclaimGuard>>;

    /// Checks whether an already-deleted entry is reclaimable under the caller's
    /// media lock. Kept for focused storage probes; production uses
    /// [`MediaStorage::reclaim_guard`] to hold the lease through unlink.
    async fn media_entry_is_reclaimable(
        &self,
        transaction: &mut WriteTransaction,
        media: &MediaRef,
        current_instance_id: &InstanceId,
        evidence: &MediaReferenceEvidence,
    ) -> Result<bool>;

    /// Calculates the total storage used by a user's uploads (in bytes).
    async fn get_user_upload_usage(&self, user_id: UserId) -> Result<ByteSize>;

    /// Calculates total storage used by all local uploads (in bytes).
    async fn total_upload_bytes(&self) -> Result<ByteSize>;

    /// Finds a media record by its content hash and source across all users.
    ///
    /// This is used to avoid duplicate downloads of remote content.
    async fn find_by_hash(
        &self,
        sha256: &ContentHash,
        source: &MediaSource,
    ) -> Result<Option<MediaRecord>>;

    /// Counts this owner's live theme bindings for an exact Media identity.
    async fn theme_reference_count(&self, user_id: UserId, media: &MediaRef) -> Result<u64>;
}

/// Backend-specific divergence for [`MediaStore`].
///
/// Aggregate casts and transaction-scoped media locking diverge by backend.
/// The guarded delete, its locked classification, and reclamation decision use
/// portable `RETURNING`/`EXISTS` SQL in this generic owner, keeping the policy
/// identical across the two dialects (ADR-0019).
#[async_trait]
pub trait MediaDialect: Backend {
    /// Returns the total upload bytes for `user_id` using backend-appropriate SQL.
    async fn get_user_upload_usage(pool: &Pool<Self>, user_id: UserId) -> Result<ByteSize>;

    /// Acquires this transaction's stable lock for one media identity.
    async fn lock_media_reference(conn: &mut Self::Connection, media: &MediaRef) -> Result<()>;

    /// Returns the total upload bytes across all users using backend-appropriate SQL.
    async fn total_upload_bytes(pool: &Pool<Self>) -> Result<ByteSize>;

    /// Performs the portable conditional deletion under the dialect's media lock.
    // cov:ignore-start — generic declaration header is attributed to backend monomorphizations.
    async fn try_delete_media(
        conn: &mut Self::Connection,
        user_id: UserId,
        media: &MediaRef,
        current_instance_id: &InstanceId,
        evidence: &MediaReferenceEvidence,
        mode: MediaDeleteMode,
    ) -> Result<bool>
    where
        for<'q> i64: Encode<'q, Self> + Type<Self>,
        String: Type<Self>,
        for<'q> String: Encode<'q, Self>,
        for<'q> MediaDeleteMode: Encode<'q, Self> + Type<Self>,
        for<'q> &'q str: Encode<'q, Self> + Type<Self>,
        for<'q> &'q InstanceId: Encode<'q, Self> + Type<Self>,
        for<'q> i32: Decode<'q, Self> + Type<Self>,
        usize: ColumnIndex<Self::Row>,
        for<'c> &'c mut Self::Connection: Executor<'c, Database = Self>,
        Self::Arguments: sqlx::IntoArguments<Self>,
    {
        // cov:ignore-stop
        // cov:ignore
        Self::lock_media_reference(conn, media).await?;
        let mut query = QueryBuilder::<Self>::new(String::new());
        crate::posts::media::push_media_reference_evidence_cte(&mut query, evidence);
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
        crate::posts::media::push_owner_media_reference_from_where(&mut query, user_id, media);
        crate::posts::media::push_live_media_reference_predicate(&mut query, current_instance_id);
        query.push(")) AND (NOT EXISTS (SELECT 1");
        crate::posts::media::push_other_owner_media_reference_from_where(
            &mut query, user_id, media,
        );
        crate::posts::media::push_live_media_reference_predicate(&mut query, current_instance_id);
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
        Ok(query
            .build_query_scalar::<i32>()
            .fetch_optional(&mut *conn)
            .await?
            .is_some())
    } // cov:ignore

    /// Lists the authenticated owner's retained Post IDs after foreign evidence
    /// exemptions, in ascending order.
    async fn owner_retained_post_ids(
        conn: &mut Self::Connection,
        user_id: UserId,
        media: &MediaRef,
        current_instance_id: &InstanceId,
        evidence: &MediaReferenceEvidence,
    ) -> Result<Vec<PostId>>
    where
        for<'q> i64: Decode<'q, Self> + Encode<'q, Self> + Type<Self>,
        String: Type<Self>,
        for<'q> String: Encode<'q, Self>,
        for<'q> &'q str: Encode<'q, Self> + Type<Self>,
        for<'q> &'q InstanceId: Encode<'q, Self> + Type<Self>,
        for<'q> PostId: Decode<'q, Self> + Type<Self>,
        usize: ColumnIndex<Self::Row>,
        for<'c> &'c mut Self::Connection: Executor<'c, Database = Self>,
        Self::Arguments: sqlx::IntoArguments<Self>,
    {
        let mut query = QueryBuilder::<Self>::new(String::new());
        crate::posts::media::push_media_reference_evidence_cte(&mut query, evidence);
        query.push("SELECT DISTINCT pm.post_id");
        crate::posts::media::push_owner_media_reference_from_where(&mut query, user_id, media);
        crate::posts::media::push_live_media_reference_predicate(&mut query, current_instance_id);
        query.push(" ORDER BY pm.post_id ASC");
        query
            .build_query_scalar::<PostId>()
            .fetch_all(&mut *conn)
            .await
    }

    /// Whether unreportable global evidence still makes deletion unsafe.
    async fn has_global_media_safety(
        conn: &mut Self::Connection,
        user_id: UserId,
        media: &MediaRef,
        current_instance_id: &InstanceId,
        evidence: &MediaReferenceEvidence,
    ) -> Result<bool>
    where
        for<'q> i64: Encode<'q, Self> + Type<Self>,
        String: Type<Self>,
        for<'q> String: Encode<'q, Self>,
        for<'q> &'q str: Encode<'q, Self> + Type<Self>,
        for<'q> &'q InstanceId: Encode<'q, Self> + Type<Self>,
        for<'q> i32: Decode<'q, Self> + Type<Self>,
        usize: ColumnIndex<Self::Row>,
        for<'c> &'c mut Self::Connection: Executor<'c, Database = Self>,
        Self::Arguments: sqlx::IntoArguments<Self>,
    {
        let mut query = QueryBuilder::<Self>::new(String::new());
        crate::posts::media::push_media_reference_evidence_cte(&mut query, evidence);
        query.push("SELECT 1 WHERE EXISTS (SELECT 1");
        crate::posts::media::push_other_owner_media_reference_from_where(
            &mut query, user_id, media,
        );
        crate::posts::media::push_live_media_reference_predicate(&mut query, current_instance_id);
        query.push(") AND NOT EXISTS (SELECT 1 FROM media m2 WHERE m2.source = ");
        query
            .push_storage_bind(media.source)
            .push(" AND m2.sha256 = ")
            .push_storage_bind(media.sha256.clone())
            .push(" AND m2.filename = ")
            .push_storage_bind(media.filename.clone())
            .push(" AND m2.user_id <> ")
            .push_storage_bind(user_id)
            .push(")");
        Ok(query
            .build_query_scalar::<i32>()
            .fetch_optional(&mut *conn)
            .await?
            .is_some())
    }

    /// Executes the portable locked global reclaimability decision.
    // cov:ignore-start — generic declaration header is attributed to backend monomorphizations.
    async fn media_entry_is_reclaimable(
        conn: &mut Self::Connection,
        media: &MediaRef,
        current_instance_id: &InstanceId,
        evidence: &MediaReferenceEvidence,
    ) -> Result<bool>
    where
        for<'q> i64: Encode<'q, Self> + Type<Self>,
        String: Type<Self>,
        for<'q> String: Encode<'q, Self>,
        for<'q> &'q str: Encode<'q, Self> + Type<Self>,
        for<'q> &'q InstanceId: Encode<'q, Self> + Type<Self>,
        for<'q> i32: Decode<'q, Self> + Type<Self>,
        usize: ColumnIndex<Self::Row>,
        for<'c> &'c mut Self::Connection: Executor<'c, Database = Self>,
        Self::Arguments: sqlx::IntoArguments<Self>,
    {
        // cov:ignore-stop
        // cov:ignore
        Self::lock_media_reference(conn, media).await?;
        let mut query = QueryBuilder::<Self>::new(String::new());
        crate::posts::media::push_media_reference_evidence_cte(&mut query, evidence);
        query.push("SELECT 1 WHERE NOT EXISTS (SELECT 1 FROM media WHERE source = ");
        query
            .push_storage_bind(media.source)
            .push(" AND sha256 = ")
            .push_storage_bind(media.sha256.clone())
            .push(" AND filename = ")
            .push_storage_bind(media.filename.clone());
        query.push(") AND NOT EXISTS (SELECT 1");
        crate::posts::media::push_any_media_reference_from_where(&mut query, media);
        crate::posts::media::push_live_media_reference_predicate(&mut query, current_instance_id);
        query.push(")");
        Ok(query
            .build_query_scalar::<i32>()
            .fetch_optional(&mut *conn)
            .await?
            .is_some())
    } // cov:ignore
}

/// Generic [`MediaStorage`] backed by any [`MediaDialect`] database.
///
/// All methods except `get_user_upload_usage` are shared here; that one
/// delegates to [`MediaDialect::get_user_upload_usage`].  See ADR-0019.
pub struct MediaStore<DB: Database> {
    pool: Pool<DB>,
}

impl<DB: Database> MediaStore<DB> {
    #[must_use]
    pub fn new(pool: Pool<DB>) -> Self {
        Self { pool }
    }
}

#[async_trait]
impl<DB> MediaStorage for MediaStore<DB>
where
    DB: MediaDialect,
    MediaRecord: for<'r> sqlx::FromRow<'r, DB::Row>,
    for<'q> i64: Encode<'q, DB> + Type<DB>,
    for<'q> &'q str: Encode<'q, DB> + Type<DB>,
    // `ContentHash`/`Filename` bind and decode as themselves via the ADR-0071 sqlx
    // bridge. The write/lookup binds encode `&ContentHash`/`&Filename`.
    String: Type<DB>,
    for<'q> String: Encode<'q, DB>,
    // `source_url` binds as `Option<MediaSourceUrl>` (#675). The newtype's own `Type`/`Encode`
    // follow from the `String` bounds above via the generic `StrNewtype` bridge, but the
    // `Option` wrapper has to be named explicitly — same reason the `Option<String>` bound
    // it replaces was spelled out.
    for<'q> Option<MediaSourceUrl>: Encode<'q, DB> + Type<DB>,
    // `RowLimit`/`PageOffset` bind as themselves via the ADR-0071 sqlx bridge (both
    // delegate to `i64`) — the listing's `LIMIT`/`OFFSET` placeholders (#696).
    for<'q> RowLimit: Encode<'q, DB> + Type<DB>,
    for<'q> PageOffset: Encode<'q, DB> + Type<DB>,
    for<'q> UtcInstant: Encode<'q, DB> + Type<DB>,
    // `MediaDeleteMode` binds directly into the guarded-delete expression.
    for<'q> MediaDeleteMode: Encode<'q, DB> + Type<DB>,
    for<'q> i64: sqlx::Decode<'q, DB>,
    for<'q> i32: Decode<'q, DB> + Type<DB>,
    usize: sqlx::ColumnIndex<DB::Row>,
    for<'c> &'c mut DB::Connection: Executor<'c, Database = DB>,
    for<'c> &'c Pool<DB>: Executor<'c, Database = DB>,
    DB::Arguments: sqlx::IntoArguments<DB>,
{
    #[tracing::instrument(
        name = "storage.media.create",
        skip(self, transaction, record),
        fields(db.system = DB::DB_SYSTEM)
    )]
    async fn create_media(
        &self,
        transaction: &mut WriteTransaction,
        record: &MediaRecord,
    ) -> Result<(), CreateMediaError> {
        // Keep direct storage callers serialized even when the manager has already
        // acquired this reentrant transaction lock for placement and insertion.
        let media = MediaRef {
            source: record.source,
            sha256: record.sha256.clone(),
            filename: record.filename.clone(),
        };
        let connection = DB::write_connection(transaction)?;
        DB::lock_media_reference(connection, &media)
            .await
            .map_err(CreateMediaError::Internal)?;

        let result = sqlx::query(
            "INSERT INTO media (user_id, sha256, filename, source, content_type, size_bytes, source_url, created_at)
             VALUES ($1, $2, $3, $4, $5, $6, $7, $8)",
        )
        .bind_storage(record.user_id)
        .bind_storage(&record.sha256)
        .bind_storage(&record.filename)
        .bind_storage(record.source)
        .bind_storage(&record.content_type)
        .bind_storage(record.size_bytes)
        .bind_storage(record.source_url.clone())
        .bind_storage(record.created_at)
        .execute(connection)
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

    async fn lock_media_reference(
        &self,
        transaction: &mut WriteTransaction,
        media: &MediaRef,
    ) -> Result<()> {
        DB::lock_media_reference(DB::write_connection(transaction)?, media).await
    }

    #[tracing::instrument(
        name = "storage.media.get",
        skip(self),
        fields(db.system = DB::DB_SYSTEM)
    )]
    async fn get_media(
        &self,
        user_id: UserId,
        sha256: &ContentHash,
        filename: &Filename,
        source: &MediaSource,
    ) -> Result<Option<MediaRecord>> {
        sqlx::query_as::<_, MediaRecord>(
            "SELECT user_id, sha256, filename, source, content_type, size_bytes, source_url, created_at
             FROM media
             WHERE user_id = $1 AND sha256 = $2 AND filename = $3 AND source = $4",
        )
        .bind_storage(user_id)
        .bind_storage(sha256)
        .bind_storage(filename)
        .bind_storage(*source)
        .fetch_optional(&self.pool)
        .await
    }

    async fn theme_reference_count(&self, user_id: UserId, media: &MediaRef) -> Result<u64> {
        let count: RowCount = sqlx::query_scalar(
            "SELECT COUNT(*) FROM (
                SELECT media_user_id, media_source, media_digest, media_filename
                FROM theme_role_bindings
                UNION ALL
                SELECT media_user_id, media_source, media_digest, media_filename
                FROM theme_header_pool
            ) theme_media
            WHERE media_user_id = $1
              AND media_source = $2
              AND media_digest = $3
              AND media_filename = $4",
        )
        .bind_storage(user_id)
        .bind_storage(media.source)
        .bind_storage(&media.sha256)
        .bind_storage(&media.filename)
        .fetch_one(&self.pool)
        .await?;
        Ok(count.into_u64())
    }

    #[tracing::instrument(
        name = "storage.media.list",
        skip(self),
        fields(db.system = DB::DB_SYSTEM)
    )]
    async fn list_media<'a>(
        &self,
        user_id: UserId,
        source: Option<&'a MediaSource>,
        limit: RowLimit,
        offset: PageOffset,
    ) -> Result<Vec<MediaRecord>> {
        // Fetch raw rows so each row decodes independently: a corrupt domain
        // column must not fail the whole `fetch_all`. Decoding per row (as the
        // feed-event claim mapper does) lets us skip the bad one and keep the rest.
        let rows = if let Some(src) = source {
            sqlx::query(
                "SELECT user_id, sha256, filename, source, content_type, size_bytes, source_url, created_at
                 FROM media
                 WHERE user_id = $1 AND source = $2
                 ORDER BY created_at DESC
                 LIMIT $3 OFFSET $4",
            )
            .bind_storage(user_id)
            .bind_storage(*src)
            .bind_storage(limit)
            .bind_storage(offset)
            .fetch_all(&self.pool)
            .await?
        } else {
            sqlx::query(
                "SELECT user_id, sha256, filename, source, content_type, size_bytes, source_url, created_at
                 FROM media
                 WHERE user_id = $1
                 ORDER BY created_at DESC
                 LIMIT $2 OFFSET $3",
            )
            .bind_storage(user_id)
            .bind_storage(limit)
            .bind_storage(offset)
            .fetch_all(&self.pool)
            .await?
        };

        // Skip (don't fail the whole list on) a row that fails to decode; direct
        // lookups (`get_media`/`find_by_hash`) stay strict
        // (docs/adr/0122-one-bad-row-must-not-stop-the-scan.md).
        Ok(rows
            .iter()
            .filter_map(|row| match MediaRecord::from_row(row) {
                Ok(record) => Some(record),
                Err(error) => {
                    tracing::warn!(%error, "skipping undecodable media row in list_media");
                    None
                }
            })
            .collect())
    }

    #[tracing::instrument(
        name = "storage.media.try_delete",
        skip(self, transaction, media),
        fields(db.system = DB::DB_SYSTEM)
    )]
    async fn try_delete_media(
        &self,
        transaction: &mut WriteTransaction,
        user_id: UserId,
        media: &MediaRef,
        current_instance_id: &InstanceId,
        evidence: &MediaReferenceEvidence,
        mode: MediaDeleteMode,
    ) -> Result<TryDeleteOutcome, DeleteMediaError> {
        let connection = DB::write_connection(transaction)?;
        if DB::try_delete_media(
            connection,
            user_id,
            media,
            current_instance_id,
            evidence,
            mode,
        )
        .await?
        {
            return Ok(TryDeleteOutcome::Deleted);
        }

        let present = sqlx::query(
            "SELECT 1 FROM media \
             WHERE user_id = $1 AND source = $2 AND sha256 = $3 AND filename = $4",
        )
        .bind_storage(user_id)
        .bind_storage(media.source)
        .bind_storage(&media.sha256)
        .bind_storage(&media.filename)
        .fetch_optional(DB::write_connection(transaction)?)
        .await?;
        if present.is_none() {
            return Ok(TryDeleteOutcome::Missing);
        }

        let owner_post_ids = DB::owner_retained_post_ids(
            DB::write_connection(transaction)?,
            user_id,
            media,
            current_instance_id,
            evidence,
        )
        .await?;
        if DB::has_global_media_safety(
            DB::write_connection(transaction)?,
            user_id,
            media,
            current_instance_id,
            evidence,
        )
        .await?
            || owner_post_ids.is_empty()
        {
            Ok(TryDeleteOutcome::GlobalSafety)
        } else {
            Ok(TryDeleteOutcome::OwnerRetainedHistory(owner_post_ids))
        }
    }

    async fn reclaim_guard(
        &self,
        transaction: &mut WriteTransaction,
        media: &MediaRef,
        current_instance_id: &InstanceId,
        evidence: &MediaReferenceEvidence,
    ) -> Result<Option<ReclaimGuard>> {
        if DB::media_entry_is_reclaimable(
            DB::write_connection(transaction)?,
            media,
            current_instance_id,
            evidence,
        )
        .await?
        {
            Ok(Some(ReclaimGuard::new()))
        } else {
            Ok(None)
        }
    }

    async fn media_entry_is_reclaimable(
        &self,
        transaction: &mut WriteTransaction,
        media: &MediaRef,
        current_instance_id: &InstanceId,
        evidence: &MediaReferenceEvidence,
    ) -> Result<bool> {
        DB::media_entry_is_reclaimable(
            DB::write_connection(transaction)?,
            media,
            current_instance_id,
            evidence,
        )
        .await
    }

    #[tracing::instrument(
        name = "storage.media.upload_usage",
        skip(self),
        fields(db.system = DB::DB_SYSTEM)
    )]
    async fn get_user_upload_usage(&self, user_id: UserId) -> Result<ByteSize> {
        // The dialect twin decodes `COALESCE(SUM(…), 0)` straight into `ByteSize`; the
        // bridge's bound-checking `Decode` rejects a negative total at the column.
        DB::get_user_upload_usage(&self.pool, user_id).await
    }

    #[tracing::instrument(
        name = "storage.media.total_upload_bytes",
        skip(self),
        fields(db.system = DB::DB_SYSTEM)
    )]
    async fn total_upload_bytes(&self) -> Result<ByteSize> {
        // Same shape as per-user usage, but intentionally all-users: this is the
        // DB-declared upload footprint exported by observability, not filesystem usage.
        DB::total_upload_bytes(&self.pool).await
    }

    #[tracing::instrument(
        name = "storage.media.find_by_hash",
        skip(self),
        fields(db.system = DB::DB_SYSTEM)
    )]
    async fn find_by_hash(
        &self,
        sha256: &ContentHash,
        source: &MediaSource,
    ) -> Result<Option<MediaRecord>> {
        sqlx::query_as::<_, MediaRecord>(
            "SELECT user_id, sha256, filename, source, content_type, size_bytes, source_url, created_at
             FROM media
             WHERE sha256 = $1 AND source = $2
             LIMIT 1",
        )
        .bind_storage(sha256)
        .bind_storage(*source)
        .fetch_optional(&self.pool)
        .await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::posts::media::{
        PersistedMediaReference, PersistedMediaSubject, ProvenForeignReference,
    };
    use crate::test_support::{
        Backend, MEDIA_TEST_SHA256, SeedUser, TestEnv, backends, confirmed,
        create_post_via_service, media_ref_for, media_row_exists, media_url_for, seed_media,
        seed_users,
    };
    use common::media::{MediaReferenceForm, MediaReferenceKind};
    use common::test_support::{
        parse_byte_size, parse_content_hash, parse_content_type, parse_filename, parse_page_offset,
        parse_post_body, parse_row_limit,
    };
    use rstest::*;
    use rstest_reuse::*;
    use std::{sync::Arc, time::Duration};
    use tokio::{sync::oneshot, time::timeout};

    async fn create_media_confirmed(
        media: Arc<dyn MediaStorage>,
        write_scope: crate::WriteScope,
        record: MediaRecord,
    ) {
        let outcome = write_scope
            .run(move |transaction| {
                Box::pin(async move { media.create_media(transaction, &record).await })
            })
            .await
            .expect("media fixture write succeeds");
        confirmed(outcome);
    }

    async fn delete_media_accounting_row(
        env: &TestEnv,
        user_id: UserId,
        media: &MediaRef,
    ) -> Result<(), sqlx::Error> {
        crate::with_closeable_pool!(env.base.pool(), pool, {
            sqlx::query(
                "DELETE FROM media WHERE user_id = $1 AND source = $2 AND sha256 = $3 AND filename = $4",
            )
            .bind_storage(user_id)
            .bind_storage(media.source)
            .bind_storage(&media.sha256)
            .bind_storage(&media.filename)
            .execute(pool)
            .await
            .map(|_| ())
        })
    }

    async fn insert_corrupt_media(
        env: &TestEnv,
        user_id: UserId,
        sql: &'static str,
    ) -> Result<(), sqlx::Error> {
        let sha256 = parse_content_hash(MEDIA_TEST_SHA256);
        crate::with_closeable_pool!(env.base.pool(), pool, {
            sqlx::query(sql)
                .bind_storage(user_id)
                .bind_storage(&sha256)
                .execute(pool)
                .await
                .map(|_| ())
        })
    }

    async fn try_delete_media_scoped(
        media: Arc<dyn MediaStorage>,
        write_scope: crate::WriteScope,
        user_id: UserId,
        media_ref: &MediaRef,
        instance_id: &InstanceId,
        evidence: &MediaReferenceEvidence,
        mode: MediaDeleteMode,
    ) -> Result<common::MutationOutcome<TryDeleteOutcome>, crate::WriteScopeError<DeleteMediaError>>
    {
        let media_ref = media_ref.clone();
        let instance_id = instance_id.clone();
        let evidence = evidence.clone();
        write_scope
            .run(move |transaction| {
                Box::pin(async move {
                    media
                        .try_delete_media(
                            transaction,
                            user_id,
                            &media_ref,
                            &instance_id,
                            &evidence,
                            mode,
                        )
                        .await
                })
            })
            .await
    }

    async fn media_entry_is_reclaimable_scoped(
        media: Arc<dyn MediaStorage>,
        write_scope: crate::WriteScope,
        media_ref: &MediaRef,
        instance_id: &InstanceId,
        evidence: &MediaReferenceEvidence,
    ) -> Result<common::MutationOutcome<bool>, crate::WriteScopeError<sqlx::Error>> {
        let media_ref = media_ref.clone();
        let instance_id = instance_id.clone();
        let evidence = evidence.clone();
        write_scope
            .run(move |transaction| {
                Box::pin(async move {
                    media
                        .media_entry_is_reclaimable(
                            transaction,
                            &media_ref,
                            &instance_id,
                            &evidence,
                        )
                        .await
                })
            })
            .await
    }

    /// A reference writer cannot pass a held media lock. Rolling that lock back
    /// releases the waiter; its newly-live row then defeats evidence collected for
    /// the earlier foreign reference instead of permitting the owner-row delete.
    #[apply(backends)]
    #[tokio::test]
    async fn post_reference_insert_serializes_with_foreign_evidence_delete(
        #[case] backend: Backend,
    ) {
        let env = backend.setup().await;
        let [owner, accounting_owner] = seed_users::<2>(
            std::sync::Arc::clone(&env.users()),
            env.write_scope().clone(),
        )
        .await;
        let media = seed_media(
            std::sync::Arc::clone(&env.media()),
            env.write_scope().clone(),
            owner,
            "serialized.jpg",
        )
        .await;
        seed_media(
            std::sync::Arc::clone(&env.media()),
            env.write_scope().clone(),
            accounting_owner,
            "serialized.jpg",
        )
        .await;
        let form: MediaReferenceForm = media_url_for("serialized.jpg")
            .parse()
            .expect("valid media reference form");
        let foreign_post = create_post_via_service(
            env.posts().clone(),
            env.feed_events().clone(),
            env.write_scope().clone(),
            owner,
            parse_post_body(&format!("<img src=\"{form}\">")),
        )
        .await;
        let mut evidence = MediaReferenceEvidence::new(env.base.instance_id().clone());
        assert!(evidence.insert(ProvenForeignReference::new(
            PersistedMediaReference::new(
                foreign_post,
                media.clone(),
                MediaReferenceKind::Local,
                form.clone(),
            ),
            env.base.instance_id().clone(),
        )));

        let held = env
            .base
            .pool()
            .lock_media_reference_for_write(&media)
            .await
            .expect("take the shared media lock");
        let (started_tx, started_rx) = oneshot::channel();
        let (finished_tx, mut finished_rx) = oneshot::channel();
        let writer = tokio::spawn({
            let posts = env.posts();
            let feed_events = env.feed_events();
            let write_scope = env.write_scope();
            let body = parse_post_body(&format!("new reference\n\n<img src=\"{form}\">"));
            async move {
                started_tx.send(()).expect("parent waits for writer start");
                let post_id =
                    create_post_via_service(posts, feed_events, write_scope, owner, body).await;
                finished_tx
                    .send(post_id)
                    .expect("parent waits for writer completion");
            }
        });
        started_rx.await.expect("writer started");
        assert!(
            timeout(Duration::from_millis(100), &mut finished_rx)
                .await
                .is_err(),
            "the writer must wait for the held target lock"
        );

        held.rollback()
            .await
            .expect("rollback releases the shared media lock");
        let new_post_id = finished_rx.await.expect("writer completed after rollback");
        writer.await.expect("writer task does not panic");

        let held = env
            .base
            .pool()
            .lock_media_reference_for_write(&media)
            .await
            .expect("take the shared media lock for deletion");
        let (delete_started_tx, delete_started_rx) = oneshot::channel();
        let (delete_finished_tx, mut delete_finished_rx) = oneshot::channel();
        let delete = tokio::spawn({
            let media_storage = env.media();
            let write_scope = env.write_scope();
            let media = media.clone();
            let instance_id = env.base.instance_id().clone();
            let evidence = evidence.clone();
            async move {
                delete_started_tx
                    .send(())
                    .expect("parent waits for delete start");
                let result = try_delete_media_scoped(
                    media_storage,
                    write_scope,
                    owner,
                    &media,
                    &instance_id,
                    &evidence,
                    MediaDeleteMode::GUARDED,
                )
                .await
                .map(confirmed);
                delete_finished_tx
                    .send(result)
                    .expect("parent waits for delete completion");
            }
        });
        delete_started_rx.await.expect("delete started");
        assert!(
            timeout(Duration::from_millis(100), &mut delete_finished_rx)
                .await
                .is_err(),
            "the guarded delete must wait for the same target lock"
        );
        held.commit()
            .await
            .expect("commit releases the shared media lock");

        assert_eq!(
            delete_finished_rx
                .await
                .expect("delete completed after lock release")
                .expect("guarded delete query succeeds"),
            TryDeleteOutcome::OwnerRetainedHistory(vec![new_post_id]),
            "the new unevidenced owner reference must prevent the owner-row delete"
        );
        delete.await.expect("delete task does not panic");
        assert!(media_row_exists(env.media().clone(), owner, &media).await);
    }

    /// Reclamation takes the same target lock as writes and deletion, so it cannot
    /// decide a file is orphaned while a reference writer is waiting on that target.
    #[apply(backends)]
    #[tokio::test]
    async fn reclamation_serializes_on_the_media_reference_lock(#[case] backend: Backend) {
        let env = backend.setup().await;
        let [user] = seed_users::<1>(
            std::sync::Arc::clone(&env.users()),
            env.write_scope().clone(),
        )
        .await;
        let media = seed_media(
            std::sync::Arc::clone(&env.media()),
            env.write_scope().clone(),
            user,
            "reclaim-lock.jpg",
        )
        .await;
        delete_media_accounting_row(&env, user, &media)
            .await
            .expect("remove the only accounting row");

        let held = env
            .base
            .pool()
            .lock_media_reference_for_write(&media)
            .await
            .expect("take the shared media lock");
        let (started_tx, started_rx) = oneshot::channel();
        let (finished_tx, mut finished_rx) = oneshot::channel();
        let media_storage = env.media();
        let write_scope = env.write_scope();
        let instance_id = env.base.instance_id().clone();
        let reclaim = tokio::spawn(async move {
            started_tx.send(()).expect("parent waits for reclaim start");
            let result = media_entry_is_reclaimable_scoped(
                media_storage,
                write_scope,
                &media,
                &instance_id,
                &MediaReferenceEvidence::new(instance_id.clone()),
            )
            .await
            .map(confirmed);
            finished_tx
                .send(result)
                .expect("parent waits for reclaim completion");
        });
        started_rx.await.expect("reclaim started");
        assert!(
            timeout(Duration::from_millis(100), &mut finished_rx)
                .await
                .is_err(),
            "reclamation must wait for the target lock"
        );

        held.commit()
            .await
            .expect("commit releases the shared media lock");
        assert!(
            finished_rx
                .await
                .expect("reclaim completed after lock release")
                .expect("reclaim query succeeds"),
            "with no rows or references, the file is reclaimable"
        );
        reclaim.await.expect("reclaim task does not panic");
    }

    /// The reclaimability decision and unlink share one write callback, so a
    /// newly-created Post cannot become live after the decision but before unlink.
    #[apply(backends)]
    #[tokio::test]
    async fn reclamation_holds_reference_lock_through_unlink(#[case] backend: Backend) {
        let env = backend.setup().await;
        let [user] = seed_users::<1>(
            std::sync::Arc::clone(&env.users()),
            env.write_scope().clone(),
        )
        .await;
        let media = seed_media(
            std::sync::Arc::clone(&env.media()),
            env.write_scope().clone(),
            user,
            "reclaim-unlink-lock.jpg",
        )
        .await;
        delete_media_accounting_row(&env, user, &media)
            .await
            .expect("remove the only accounting row");
        let form: MediaReferenceForm = media_url_for("reclaim-unlink-lock.jpg")
            .parse()
            .expect("valid media reference form");
        let (checked_tx, checked_rx) = oneshot::channel();
        let (unlink_tx, unlink_rx) = oneshot::channel();
        let reclaim_storage = env.media();
        let reclaim_scope = env.write_scope();
        let instance_id = env.base.instance_id().clone();
        let reclaim = tokio::spawn(async move {
            reclaim_scope
                .run(move |transaction| {
                    Box::pin(async move {
                        assert!(
                            reclaim_storage
                                .media_entry_is_reclaimable(
                                    transaction,
                                    &media,
                                    &instance_id,
                                    &MediaReferenceEvidence::new(instance_id.clone()),
                                )
                                .await?
                        );
                        checked_tx.send(()).expect("parent waits for reclaim check");
                        unlink_rx.await.expect("parent permits unlink");
                        Ok::<(), sqlx::Error>(())
                    })
                })
                .await
                .expect("reclaim write scope completes");
        });
        checked_rx.await.expect("reclaimability decision completed");

        let writer_posts = env.posts();
        let writer_feed_events = env.feed_events();
        let writer_scope = env.write_scope();
        let mut writer = tokio::spawn(async move {
            create_post_via_service(
                writer_posts,
                writer_feed_events,
                writer_scope,
                user,
                parse_post_body(&format!("<img src=\"{form}\">")),
            )
            .await
        });
        assert!(
            timeout(Duration::from_millis(100), &mut writer)
                .await
                .is_err(),
            "a fresh Post writer must wait until unlink finishes"
        );
        unlink_tx.send(()).expect("permit unlink");
        reclaim.await.expect("reclaim task does not panic");
        writer.await.expect("writer task does not panic");
    }

    /// A media create takes the reclamation lock before its insert. Therefore, a
    /// successful create starts only after an in-flight reclaim has unlinked the old bytes.
    #[apply(backends)]
    #[tokio::test]
    async fn create_media_serializes_with_reclamation_through_unlink(#[case] backend: Backend) {
        let env = backend.setup().await;
        let [user] = seed_users::<1>(
            std::sync::Arc::clone(&env.users()),
            env.write_scope().clone(),
        )
        .await;
        let media = seed_media(
            std::sync::Arc::clone(&env.media()),
            env.write_scope().clone(),
            user,
            "create-reclaim-lock.jpg",
        )
        .await;
        delete_media_accounting_row(&env, user, &media)
            .await
            .expect("remove the only accounting row");
        let record = MediaRecord {
            user_id: user,
            sha256: media.sha256.clone(),
            filename: media.filename.clone(),
            source: media.source,
            content_type: parse_content_type("image/jpeg"),
            size_bytes: parse_byte_size("1"),
            source_url: None,
            created_at: UtcInstant::now(),
        };

        let (checked_tx, checked_rx) = oneshot::channel();
        let (unlink_tx, unlink_rx) = oneshot::channel();
        let reclaim_storage = env.media();
        let reclaim_scope = env.write_scope();
        let instance_id = env.base.instance_id().clone();
        let media_for_reclaim = media.clone();
        let reclaim = tokio::spawn(async move {
            reclaim_scope
                .run(move |transaction| {
                    Box::pin(async move {
                        assert!(
                            reclaim_storage
                                .media_entry_is_reclaimable(
                                    transaction,
                                    &media_for_reclaim,
                                    &instance_id,
                                    &MediaReferenceEvidence::new(instance_id.clone()),
                                )
                                .await?
                        );
                        checked_tx.send(()).expect("parent waits for reclaim check");
                        unlink_rx.await.expect("parent permits unlink");
                        Ok::<(), sqlx::Error>(())
                    })
                })
                .await
                .expect("reclaim write scope completes");
        });
        checked_rx.await.expect("reclaimability decision completed");

        let media_storage = env.media();
        let write_scope = env.write_scope();
        let mut create = tokio::spawn(async move {
            create_media_confirmed(media_storage, write_scope, record).await;
        });
        assert!(
            timeout(Duration::from_millis(100), &mut create)
                .await
                .is_err(),
            "the create must wait until the reclaim transaction finishes unlinking"
        );

        unlink_tx.send(()).expect("permit unlink");
        reclaim.await.expect("reclaim task does not panic");
        create.await.expect("create task does not panic");
        assert!(media_row_exists(env.media().clone(), user, &media).await);
    }

    /// How many posts the concurrency exercise writes while the guard is hammered, and
    /// how many unforced deletes it attempts against them. Large enough that the two
    /// interleave for the whole run on both backends; small enough to stay a unit test.
    const ROUNDS: usize = 100;

    #[apply(backends)]
    #[tokio::test]
    async fn content_hash_and_filename_round_trip_through_create_and_get(#[case] backend: Backend) {
        let env = backend.setup().await;
        let user_id = SeedUser::new()
            .seed(
                std::sync::Arc::clone(&env.users()),
                env.write_scope().clone(),
            )
            .await
            .user_id;
        let record = MediaRecord {
            user_id,
            sha256: parse_content_hash(MEDIA_TEST_SHA256),
            filename: parse_filename("photo.jpg"),
            source: MediaSource::Upload,
            content_type: parse_content_type("image/jpeg"),
            size_bytes: parse_byte_size("2048"),
            source_url: None,
            created_at: UtcInstant::now(),
        };
        create_media_confirmed(env.media().clone(), env.write_scope().clone(), record).await;
        let got = env
            .media()
            .get_media(
                user_id,
                &parse_content_hash(MEDIA_TEST_SHA256),
                &parse_filename("photo.jpg"),
                &MediaSource::Upload,
            )
            .await
            .unwrap()
            .expect("present");
        // `sha256`/`filename` decode straight into their newtypes via the sqlx bridge (#438).
        assert_eq!(got.sha256, parse_content_hash(MEDIA_TEST_SHA256));
        assert_eq!(got.filename, parse_filename("photo.jpg"));
    }

    #[apply(backends)]
    #[tokio::test]
    async fn find_by_hash_surfaces_a_column_decode_error_for_a_malformed_filename(
        #[case] backend: Backend,
    ) {
        let env = backend.setup().await;
        let user_id = SeedUser::new()
            .seed(
                std::sync::Arc::clone(&env.users()),
                env.write_scope().clone(),
            )
            .await
            .user_id;
        // A non-canonical filename (`../evil`) bypasses `Filename` validation — only
        // reachable via DB tampering. The `sha256`/`source` keys stay valid so the row
        // is found; the validating bridge `Decode` then rejects the `filename` column
        // on read as a column-decode error (`find_by_hash` is strict, unlike `list_media`).
        insert_corrupt_media(
            &env,
            user_id,
            "INSERT INTO media (user_id, sha256, filename, source, content_type, size_bytes) \
             VALUES ($1, $2, '../evil', 'upload', 'image/jpeg', 1)",
        )
        .await
        .unwrap();
        let err = env
            .media()
            .find_by_hash(&parse_content_hash(MEDIA_TEST_SHA256), &MediaSource::Upload)
            .await
            .unwrap_err();
        assert!(
            matches!(err, sqlx::Error::ColumnDecode { .. }),
            "expected a column-decode error, got: {err:?}"
        );
    }

    // No `list_media`/`find_by_hash` invalid-`source` decode-error test: the `media`
    // table's `source TEXT NOT NULL CHECK (source IN ('upload', 'cached'))` constraint
    // makes a non-token value structurally unstorable (an INSERT is rejected), so the
    // `MediaSource` text-enum bridge `Decode` error branch is unreachable at the DB layer.
    // That branch is the shared `macros::text_enum` bridge, exercised by every other
    // adopting enum; the unknown-token rejection itself is asserted in `common::media`'s
    // `media_source_unknown_token_is_rejected_with_message`.

    #[apply(backends)]
    #[tokio::test]
    async fn find_by_hash_surfaces_a_column_decode_error_for_a_negative_size(
        #[case] backend: Backend,
    ) {
        let env = backend.setup().await;
        let user_id = SeedUser::new()
            .seed(
                std::sync::Arc::clone(&env.users()),
                env.write_scope().clone(),
            )
            .await
            .user_id;
        // A negative `size_bytes` bypasses `ByteSize` validation — only reachable via DB
        // tampering. On read, `MediaRecord::from_row` decodes the column through the
        // validating `ByteSize` bridge, which rejects it as a column-decode error.
        insert_corrupt_media(
            &env,
            user_id,
            "INSERT INTO media (user_id, sha256, filename, source, content_type, size_bytes) \
             VALUES ($1, $2, 'photo.jpg', 'upload', 'image/jpeg', -1)",
        )
        .await
        .unwrap();
        let err = env
            .media()
            .find_by_hash(&parse_content_hash(MEDIA_TEST_SHA256), &MediaSource::Upload)
            .await
            .unwrap_err();
        assert!(
            matches!(err, sqlx::Error::ColumnDecode { .. }),
            "expected a column-decode error, got: {err:?}"
        );
    }

    #[apply(backends)]
    #[tokio::test]
    async fn get_user_upload_usage_surfaces_a_column_decode_error_for_a_negative_sum(
        #[case] backend: Backend,
    ) {
        let env = backend.setup().await;
        let user_id = SeedUser::new()
            .seed(
                std::sync::Arc::clone(&env.users()),
                env.write_scope().clone(),
            )
            .await
            .user_id;
        // A negative `size_bytes` upload row (DB tampering) makes `SUM(size_bytes)` negative;
        // the sum decodes into `ByteSize`, whose bound-checking `Decode` rejects the negative
        // total as a column-decode error.
        insert_corrupt_media(
            &env,
            user_id,
            "INSERT INTO media (user_id, sha256, filename, source, content_type, size_bytes) \
             VALUES ($1, $2, 'photo.jpg', 'upload', 'image/jpeg', -5)",
        )
        .await
        .unwrap();
        let err = env
            .media()
            .get_user_upload_usage(user_id)
            .await
            .unwrap_err();
        assert!(
            matches!(err, sqlx::Error::ColumnDecode { .. }),
            "expected a column-decode error, got: {err:?}"
        );
    }

    #[apply(backends)]
    #[tokio::test]
    async fn list_media_skips_a_row_with_a_malformed_sha256_column(#[case] backend: Backend) {
        let env = backend.setup().await;
        let user_id = SeedUser::new()
            .seed(
                std::sync::Arc::clone(&env.users()),
                env.write_scope().clone(),
            )
            .await
            .user_id;
        // A valid record is stored normally.
        let good = MediaRecord {
            user_id,
            sha256: parse_content_hash(MEDIA_TEST_SHA256),
            filename: parse_filename("good.jpg"),
            source: MediaSource::Upload,
            content_type: parse_content_type("image/jpeg"),
            size_bytes: parse_byte_size("1"),
            source_url: None,
            created_at: UtcInstant::now(),
        };
        create_media_confirmed(env.media().clone(), env.write_scope().clone(), good).await;
        // A second row's `sha256` is tampered to a non-hex value — only reachable via
        // direct DB access, since `ContentHash::from_str` requires 64 lowercase hex chars.
        // Every media read keys the query *on* `sha256`, so the observable behavior is
        // `list_media`'s per-row skip: the validating bridge `Decode` rejects the
        // non-canonical hash and the row is dropped rather than surfaced (mirrors the
        // `filename` decode handling; #438). The skip *is* the proof `Decode` rejected it.
        crate::with_closeable_pool!(env.base.pool(), pool, {
            sqlx::query(
                "INSERT INTO media (user_id, sha256, filename, source, content_type, size_bytes) \
                 VALUES ($1, 'not-a-valid-hash', 'bad.jpg', 'upload', 'image/jpeg', 1)",
            )
            .bind_storage(user_id)
            .execute(pool)
            .await
            .map(|_| ())
        })
        .unwrap();
        let listed = env
            .media()
            .list_media(user_id, None, parse_row_limit("10"), parse_page_offset("0"))
            .await
            .unwrap();
        assert_eq!(
            listed.len(),
            1,
            "the malformed-sha256 row must be skipped and the valid row kept"
        );
        assert_eq!(listed[0].sha256, parse_content_hash(MEDIA_TEST_SHA256));
    }

    #[apply(backends)]
    #[tokio::test]
    async fn get_media_with_closed_pool_returns_error(#[case] backend: Backend) {
        let env = backend.setup().await;
        env.base.close_pool().await;
        let result = env
            .media()
            .get_media(
                UserId::from(1),
                &parse_content_hash(MEDIA_TEST_SHA256),
                &parse_filename("test.jpg"),
                &MediaSource::Upload,
            )
            .await;
        assert!(result.is_err());
    }

    #[apply(backends)]
    #[tokio::test]
    async fn list_media_with_closed_pool_returns_error(#[case] backend: Backend) {
        let env = backend.setup().await;
        env.base.close_pool().await;
        let result = env
            .media()
            .list_media(
                UserId::from(1),
                None,
                parse_row_limit("10"),
                parse_page_offset("0"),
            )
            .await;
        assert!(result.is_err());
    }

    #[apply(backends)]
    #[tokio::test]
    async fn try_delete_media_refuses_a_referenced_item_without_force(#[case] backend: Backend) {
        // A17b.
        let env = backend.setup().await;
        let [user] = seed_users::<1>(
            std::sync::Arc::clone(&env.users()),
            env.write_scope().clone(),
        )
        .await;
        let media = seed_media(
            std::sync::Arc::clone(&env.media()),
            env.write_scope().clone(),
            user,
            "photo.jpg",
        )
        .await;
        let embed = format!("<img src=\"{}\">", media_url_for("photo.jpg"));
        let post_id = create_post_via_service(
            env.posts().clone(),
            env.feed_events().clone(),
            env.write_scope().clone(),
            user,
            parse_post_body(&embed),
        )
        .await;

        assert_eq!(
            confirmed(
                try_delete_media_scoped(
                    env.media().clone(),
                    env.write_scope().clone(),
                    user,
                    &media,
                    env.base.instance_id(),
                    &MediaReferenceEvidence::new(env.base.instance_id().clone()),
                    MediaDeleteMode::GUARDED,
                )
                .await
                .expect("the guarded delete succeeds as a scoped write"),
            ),
            TryDeleteOutcome::OwnerRetainedHistory(vec![post_id])
        );
        assert!(
            media_row_exists(env.media().clone(), user, &media).await,
            "refusal leaves the row"
        );
    }

    #[apply(backends)]
    #[tokio::test]
    async fn foreign_evidence_exempts_only_the_exact_persisted_reference(#[case] backend: Backend) {
        let env = backend.setup().await;
        let [user] = seed_users::<1>(
            std::sync::Arc::clone(&env.users()),
            env.write_scope().clone(),
        )
        .await;
        let media = seed_media(
            std::sync::Arc::clone(&env.media()),
            env.write_scope().clone(),
            user,
            "exact.jpg",
        )
        .await;
        let form: MediaReferenceForm = media_url_for("exact.jpg")
            .parse()
            .expect("valid media reference form");
        let post_id = create_post_via_service(
            env.posts().clone(),
            env.feed_events().clone(),
            env.write_scope().clone(),
            user,
            parse_post_body(&format!("<img src=\"{form}\">")),
        )
        .await;

        let wrong_form = PersistedMediaReference::new(
            post_id,
            media.clone(),
            MediaReferenceKind::Absolute,
            format!("https://foreign.example{form}")
                .parse()
                .expect("valid media reference form"),
        );
        let mut near_match = MediaReferenceEvidence::new(env.base.instance_id().clone());
        assert!(near_match.insert(ProvenForeignReference::new(
            wrong_form,
            env.base.instance_id().clone(),
        )));
        assert_eq!(
            confirmed(
                try_delete_media_scoped(
                    env.media().clone(),
                    env.write_scope().clone(),
                    user,
                    &media,
                    env.base.instance_id(),
                    &near_match,
                    MediaDeleteMode::GUARDED,
                )
                .await
                .expect("near-match guarded delete succeeds"),
            ),
            TryDeleteOutcome::OwnerRetainedHistory(vec![post_id]),
            "different kind/form evidence must not exempt the owner's local row"
        );

        let exact =
            PersistedMediaReference::new(post_id, media.clone(), MediaReferenceKind::Local, form);
        let mut exact_match = MediaReferenceEvidence::new(env.base.instance_id().clone());
        assert!(exact_match.insert(ProvenForeignReference::new(
            exact,
            env.base.instance_id().clone(),
        )));
        assert_eq!(
            confirmed(
                try_delete_media_scoped(
                    env.media().clone(),
                    env.write_scope().clone(),
                    user,
                    &media,
                    env.base.instance_id(),
                    &exact_match,
                    MediaDeleteMode::GUARDED,
                )
                .await
                .expect("exact-evidence delete succeeds"),
            ),
            TryDeleteOutcome::Deleted
        );
    }

    #[apply(backends)]
    #[tokio::test]
    async fn reclaimability_uses_the_same_exact_evidence_guard(#[case] backend: Backend) {
        let env = backend.setup().await;
        let [user] = seed_users::<1>(
            std::sync::Arc::clone(&env.users()),
            env.write_scope().clone(),
        )
        .await;
        let media = seed_media(
            std::sync::Arc::clone(&env.media()),
            env.write_scope().clone(),
            user,
            "reclaim.jpg",
        )
        .await;
        let form: MediaReferenceForm = media_url_for("reclaim.jpg")
            .parse()
            .expect("valid media reference form");
        let post_id = create_post_via_service(
            env.posts().clone(),
            env.feed_events().clone(),
            env.write_scope().clone(),
            user,
            parse_post_body(&format!("<img src=\"{form}\">")),
        )
        .await;
        delete_media_accounting_row(&env, user, &media)
            .await
            .expect("remove accounting row");

        let empty = MediaReferenceEvidence::new(env.base.instance_id().clone());
        assert!(
            !confirmed(
                media_entry_is_reclaimable_scoped(
                    env.media().clone(),
                    env.write_scope().clone(),
                    &media,
                    env.base.instance_id(),
                    &empty,
                )
                .await
                .expect("live reference check succeeds"),
            ),
            "live reference prevents reclamation"
        );
        let mut exact = MediaReferenceEvidence::new(env.base.instance_id().clone());
        assert!(exact.insert(ProvenForeignReference::new(
            PersistedMediaReference::new(post_id, media.clone(), MediaReferenceKind::Local, form),
            env.base.instance_id().clone(),
        )));
        assert!(
            confirmed(
                media_entry_is_reclaimable_scoped(
                    env.media().clone(),
                    env.write_scope().clone(),
                    &media,
                    env.base.instance_id(),
                    &exact,
                )
                .await
                .expect("exact foreign evidence check succeeds"),
            ),
            "exact foreign evidence makes row reclaimable"
        );
    }
    /// A foreign result for a current row cannot authorize deleting an unseen
    /// retained revision of the same Post. The exact revision proof may.
    #[apply(backends)]
    #[tokio::test]
    async fn revision_subject_requires_its_own_exact_foreign_evidence(#[case] backend: Backend) {
        let env = backend.setup().await;
        let [owner] = seed_users::<1>(
            std::sync::Arc::clone(&env.users()),
            env.write_scope().clone(),
        )
        .await;
        let media = seed_media(
            std::sync::Arc::clone(&env.media()),
            env.write_scope().clone(),
            owner,
            "revision-evidence.jpg",
        )
        .await;
        let form: MediaReferenceForm = media_url_for("revision-evidence.jpg")
            .parse()
            .expect("valid media reference form");
        let post_id = create_post_via_service(
            env.posts().clone(),
            env.feed_events().clone(),
            env.write_scope().clone(),
            owner,
            parse_post_body(&format!("<img src=\"{form}\">")),
        )
        .await;
        let posts = Arc::clone(&env.posts());
        let outcome = env
            .write_scope()
            .run(move |transaction| {
                Box::pin(async move {
                    posts
                        .soft_delete_post(
                            transaction,
                            post_id,
                            owner,
                            common::time::UtcInstant::now(),
                        )
                        .await
                        .map(|_| ())
                })
            })
            .await
            .expect("deletion captures the prior media subject");
        assert!(matches!(outcome, common::MutationOutcome::Confirmed(())));

        let references = env
            .posts()
            .list_media_references(&media)
            .await
            .expect("retained references load");
        let current = references
            .references()
            .iter()
            .find(|reference| matches!(reference.subject(), PersistedMediaSubject::Current))
            .expect("deleted current subject remains retained")
            .clone();
        let revision = references
            .references()
            .iter()
            .find(|reference| matches!(reference.subject(), PersistedMediaSubject::Revision(_)))
            .expect("captured revision subject remains retained")
            .clone();

        assert_eq!(
            env.posts()
                .list_posts_referencing_media(
                    owner,
                    &media,
                    env.base.instance_id(),
                    &MediaReferenceEvidence::new(env.base.instance_id().clone()),
                )
                .await
                .expect("owner advisory query succeeds"),
            vec![post_id],
            "current and revision subjects report their Post only once"
        );

        let mut current_evidence = MediaReferenceEvidence::new(env.base.instance_id().clone());
        assert!(current_evidence.insert(ProvenForeignReference::new(
            current.clone(),
            env.base.instance_id().clone(),
        )));
        assert_eq!(
            confirmed(
                try_delete_media_scoped(
                    env.media().clone(),
                    env.write_scope().clone(),
                    owner,
                    &media,
                    env.base.instance_id(),
                    &current_evidence,
                    MediaDeleteMode::GUARDED,
                )
                .await
                .expect("guarded delete succeeds"),
            ),
            TryDeleteOutcome::OwnerRetainedHistory(vec![post_id]),
            "current evidence cannot exempt the owner's retained revision subject"
        );

        let mut revision_evidence = MediaReferenceEvidence::new(env.base.instance_id().clone());
        assert!(revision_evidence.insert(ProvenForeignReference::new(
            revision.clone(),
            env.base.instance_id().clone(),
        )));
        assert_eq!(
            confirmed(
                try_delete_media_scoped(
                    env.media().clone(),
                    env.write_scope().clone(),
                    owner,
                    &media,
                    env.base.instance_id(),
                    &revision_evidence,
                    MediaDeleteMode::GUARDED,
                )
                .await
                .expect("guarded delete succeeds"),
            ),
            TryDeleteOutcome::OwnerRetainedHistory(vec![post_id]),
            "revision evidence cannot exempt the owner's deleted-current subject"
        );

        let mut complete_evidence = MediaReferenceEvidence::new(env.base.instance_id().clone());
        assert!(complete_evidence.insert(ProvenForeignReference::new(
            current,
            env.base.instance_id().clone(),
        )));
        assert!(complete_evidence.insert(ProvenForeignReference::new(
            revision,
            env.base.instance_id().clone(),
        )));
        assert_eq!(
            confirmed(
                try_delete_media_scoped(
                    env.media().clone(),
                    env.write_scope().clone(),
                    owner,
                    &media,
                    env.base.instance_id(),
                    &complete_evidence,
                    MediaDeleteMode::GUARDED,
                )
                .await
                .expect("guarded delete succeeds"),
            ),
            TryDeleteOutcome::Deleted,
            "every retained subject needs and accepts its own exact foreign proof"
        );
    }

    #[apply(backends)]
    #[tokio::test]
    async fn try_delete_media_force_overrides_own_retained_reference(#[case] backend: Backend) {
        let env = backend.setup().await;
        let [user] = seed_users::<1>(
            std::sync::Arc::clone(&env.users()),
            env.write_scope().clone(),
        )
        .await;
        let media = seed_media(
            std::sync::Arc::clone(&env.media()),
            env.write_scope().clone(),
            user,
            "photo.jpg",
        )
        .await;
        let embed = format!("<img src=\"{}\">", media_url_for("photo.jpg"));
        create_post_via_service(
            env.posts().clone(),
            env.feed_events().clone(),
            env.write_scope().clone(),
            user,
            parse_post_body(&embed),
        )
        .await;

        assert_eq!(
            confirmed(
                try_delete_media_scoped(
                    env.media().clone(),
                    env.write_scope().clone(),
                    user,
                    &media,
                    env.base.instance_id(),
                    &MediaReferenceEvidence::new(env.base.instance_id().clone()),
                    MediaDeleteMode::FORCED,
                )
                .await
                .expect("forced delete succeeds"),
            ),
            TryDeleteOutcome::Deleted
        );
        assert!(
            !media_row_exists(env.media().clone(), user, &media).await,
            "force deliberately permits losing the owner's reconstruction"
        );
    }

    #[apply(backends)]
    #[tokio::test]
    async fn try_delete_media_allows_force_when_another_row_accounts_for_reference(
        #[case] backend: Backend,
    ) {
        let env = backend.setup().await;
        let [owner, other] = seed_users::<2>(
            std::sync::Arc::clone(&env.users()),
            env.write_scope().clone(),
        )
        .await;
        let media = seed_media(
            std::sync::Arc::clone(&env.media()),
            env.write_scope().clone(),
            owner,
            "photo.jpg",
        )
        .await;
        seed_media(
            std::sync::Arc::clone(&env.media()),
            env.write_scope().clone(),
            other,
            "photo.jpg",
        )
        .await;
        let embed = format!("<img src=\"{}\">", media_url_for("photo.jpg"));
        create_post_via_service(
            env.posts().clone(),
            env.feed_events().clone(),
            env.write_scope().clone(),
            owner,
            parse_post_body(&embed),
        )
        .await;

        assert_eq!(
            confirmed(
                try_delete_media_scoped(
                    env.media().clone(),
                    env.write_scope().clone(),
                    owner,
                    &media,
                    env.base.instance_id(),
                    &MediaReferenceEvidence::new(env.base.instance_id().clone()),
                    MediaDeleteMode::FORCED,
                )
                .await
                .expect("forced delete succeeds"),
            ),
            TryDeleteOutcome::Deleted
        );
        assert!(!media_row_exists(env.media().clone(), owner, &media).await);
        assert!(media_row_exists(env.media().clone(), other, &media).await);
    }

    #[apply(backends)]
    #[tokio::test]
    async fn try_delete_media_deletes_an_unreferenced_item(#[case] backend: Backend) {
        // A17b, the other half: nothing references it, so an unforced delete goes through.
        let env = backend.setup().await;
        let [user] = seed_users::<1>(
            std::sync::Arc::clone(&env.users()),
            env.write_scope().clone(),
        )
        .await;
        let media = seed_media(
            std::sync::Arc::clone(&env.media()),
            env.write_scope().clone(),
            user,
            "photo.jpg",
        )
        .await;

        assert_eq!(
            confirmed(
                try_delete_media_scoped(
                    env.media().clone(),
                    env.write_scope().clone(),
                    user,
                    &media,
                    env.base.instance_id(),
                    &MediaReferenceEvidence::new(env.base.instance_id().clone()),
                    MediaDeleteMode::GUARDED,
                )
                .await
                .expect("delete succeeds"),
            ),
            TryDeleteOutcome::Deleted
        );
        assert!(!media_row_exists(env.media().clone(), user, &media).await);
    }

    #[apply(backends)]
    #[tokio::test]
    async fn try_delete_media_reports_missing_distinctly_from_refusal(#[case] backend: Backend) {
        let env = backend.setup().await;
        let [user] = seed_users::<1>(
            std::sync::Arc::clone(&env.users()),
            env.write_scope().clone(),
        )
        .await;
        let result = try_delete_media_scoped(
            env.media().clone(),
            env.write_scope().clone(),
            user,
            &media_ref_for("never-uploaded.jpg"),
            env.base.instance_id(),
            &MediaReferenceEvidence::new(env.base.instance_id().clone()),
            MediaDeleteMode::GUARDED,
        )
        .await
        .expect("missing classification succeeds");

        assert_eq!(confirmed(result), TryDeleteOutcome::Missing);
    }

    #[apply(backends)]
    #[tokio::test]
    async fn try_delete_media_holds_under_concurrent_reference_writes(#[case] backend: Backend) {
        // A17d/A17e. Be honest about what this establishes: a stress test cannot *prove*
        // atomicity — that would need controlled interleaving inside the statement, which
        // SQL gives no hook for. Atomicity here is structural: it is one statement. What
        // this does establish is (a) the statement survives sustained concurrency without
        // SQLITE_BUSY (A17e), and (b) the guard does not ignore references under load.
        //
        // Written monotone — the writer only ever ADDS references — so it cannot false-fail
        // the way an add/remove churn would, where a reference legitimately appearing between
        // the delete and a separate verification read looks identical to a violation.
        let env = backend.setup().await;
        let [user] = seed_users::<1>(
            std::sync::Arc::clone(&env.users()),
            env.write_scope().clone(),
        )
        .await;
        let media = seed_media(
            std::sync::Arc::clone(&env.media()),
            env.write_scope().clone(),
            user,
            "photo.jpg",
        )
        .await;
        // Each body carries a distinct leading line so the service path derives a distinct
        // title, and hence a distinct slug: identical bodies would collide on the slug and
        // exhaust the creator's attempt budget long before the round count here. The embed
        // — the only part the guard reads — is the same in every one.
        let embed = format!("<img src=\"{}\">", media_url_for("photo.jpg"));

        // One reference exists before any delete is attempted, and none is ever removed, so
        // every unforced delete from here on must refuse.
        create_post_via_service(
            env.posts().clone(),
            env.feed_events().clone(),
            env.write_scope().clone(),
            user,
            parse_post_body(&format!("reference 0\n\n{embed}")),
        )
        .await;

        let writer = tokio::spawn({
            let posts = env.posts();
            let feed_events = env.feed_events();
            let write_scope = env.write_scope();
            async move {
                for round in 1..=ROUNDS {
                    create_post_via_service(
                        posts.clone(),
                        feed_events.clone(),
                        write_scope.clone(),
                        user,
                        parse_post_body(&format!("reference {round}\n\n{embed}")),
                    )
                    .await;
                }
            }
        });

        for _ in 0..ROUNDS {
            let outcome = confirmed(
                try_delete_media_scoped(
                    env.media().clone(),
                    env.write_scope().clone(),
                    user,
                    &media,
                    env.base.instance_id(),
                    &MediaReferenceEvidence::new(env.base.instance_id().clone()),
                    MediaDeleteMode::GUARDED,
                )
                .await
                .expect("no SQLite busy under concurrent scoped writes"),
            );
            assert!(
                matches!(
                    &outcome,
                    TryDeleteOutcome::OwnerRetainedHistory(post_ids) if !post_ids.is_empty()
                ),
                "a retained owner reference exists throughout, so no guarded delete may succeed"
            );
        }
        writer.await.expect("the concurrent writer does not panic");
        assert!(media_row_exists(env.media().clone(), user, &media).await);
    }

    #[apply(backends)]
    #[tokio::test]
    async fn delete_rolls_back_when_the_scoped_operation_fails(#[case] backend: Backend) {
        let env = backend.setup().await;
        let [user] = seed_users::<1>(
            std::sync::Arc::clone(&env.users()),
            env.write_scope().clone(),
        )
        .await;
        let media_ref = seed_media(
            std::sync::Arc::clone(&env.media()),
            env.write_scope().clone(),
            user,
            "rollback.jpg",
        )
        .await;
        let instance_id = env.base.instance_id().clone();
        let evidence = MediaReferenceEvidence::new(instance_id.clone());
        let media = Arc::clone(&env.media());

        let delete_media_ref = media_ref.clone();
        let result = env
            .write_scope()
            .run(move |transaction| {
                Box::pin(async move {
                    assert_eq!(
                        media
                            .try_delete_media(
                                transaction,
                                user,
                                &delete_media_ref,
                                &instance_id,
                                &evidence,
                                MediaDeleteMode::GUARDED,
                            )
                            .await?,
                        TryDeleteOutcome::Deleted
                    );
                    Err::<TryDeleteOutcome, DeleteMediaError>(DeleteMediaError::Internal(
                        sqlx::Error::RowNotFound,
                    ))
                })
            })
            .await;

        assert!(matches!(
            result,
            Err(crate::WriteScopeError::Operation(
                DeleteMediaError::Internal(_)
            ))
        ));
        assert!(
            media_row_exists(env.media().clone(), user, &media_ref).await,
            "the storage delete participates in the caller's rollback"
        );
    }

    #[apply(backends)]
    #[tokio::test]
    async fn delete_commit_acknowledgement_loss_is_indeterminate(#[case] backend: Backend) {
        let env = backend.setup().await;
        let [user] = seed_users::<1>(
            std::sync::Arc::clone(&env.users()),
            env.write_scope().clone(),
        )
        .await;
        let media_ref = seed_media(
            std::sync::Arc::clone(&env.media()),
            env.write_scope().clone(),
            user,
            "indeterminate.jpg",
        )
        .await;
        let instance_id = env.base.instance_id().clone();
        let evidence = MediaReferenceEvidence::new(instance_id.clone());
        let media = Arc::clone(&env.media());
        let delete_media_ref = media_ref.clone();
        let scope = env
            .write_scope()
            .with_commit_acknowledgement_loss_after_commit_for_test();

        let outcome = scope
            .run(move |transaction| {
                Box::pin(async move {
                    media
                        .try_delete_media(
                            transaction,
                            user,
                            &delete_media_ref,
                            &instance_id,
                            &evidence,
                            MediaDeleteMode::GUARDED,
                        )
                        .await
                })
            })
            .await
            .expect("delete operation succeeds before acknowledgement loss");

        assert_eq!(
            outcome,
            common::MutationOutcome::CommitIndeterminate(TryDeleteOutcome::Deleted)
        );
        assert!(
            !media_row_exists(env.media().clone(), user, &media_ref).await,
            "the commit may have succeeded despite acknowledgement loss"
        );
    }

    #[apply(backends)]
    #[tokio::test]
    async fn get_user_upload_usage_with_closed_pool_returns_error(#[case] backend: Backend) {
        let env = backend.setup().await;
        env.base.close_pool().await;
        let result = env.media().get_user_upload_usage(UserId::from(1)).await;
        assert!(result.is_err());
    }

    #[apply(backends)]
    #[tokio::test]
    async fn total_upload_bytes_sums_upload_rows(#[case] backend: Backend) {
        let env = backend.setup().await;
        let [alice] = seed_users(
            std::sync::Arc::clone(&env.users()),
            env.write_scope().clone(),
        )
        .await;
        seed_media(
            std::sync::Arc::clone(&env.media()),
            env.write_scope().clone(),
            alice,
            "a.jpg",
        )
        .await;
        seed_media(
            std::sync::Arc::clone(&env.media()),
            env.write_scope().clone(),
            alice,
            "b.jpg",
        )
        .await;

        let total = env.media().total_upload_bytes().await.unwrap();

        assert_eq!(total, parse_byte_size("2"));
    }

    #[apply(backends)]
    #[tokio::test]
    async fn total_upload_bytes_excludes_non_upload_sources(#[case] backend: Backend) {
        let env = backend.setup().await;
        let [alice] = seed_users(
            std::sync::Arc::clone(&env.users()),
            env.write_scope().clone(),
        )
        .await;
        seed_media(
            std::sync::Arc::clone(&env.media()),
            env.write_scope().clone(),
            alice,
            "upload.jpg",
        )
        .await;
        env.base
            .pool()
            .execute(
                "INSERT INTO media (user_id, sha256, filename, source, content_type, size_bytes) \
                 VALUES (1, 'aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa', \
                         'remote.jpg', 'cached', 'image/jpeg', 99)",
            )
            .await
            .unwrap();

        let total = env.media().total_upload_bytes().await.unwrap();

        assert_eq!(total, parse_byte_size("1"));
    }

    #[apply(backends)]
    #[tokio::test]
    async fn find_by_hash_with_closed_pool_returns_error(#[case] backend: Backend) {
        let env = backend.setup().await;
        env.base.close_pool().await;
        let result = env
            .media()
            .find_by_hash(&parse_content_hash(MEDIA_TEST_SHA256), &MediaSource::Upload)
            .await;
        assert!(result.is_err());
    }
}
