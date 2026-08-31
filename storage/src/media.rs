//! Media file metadata storage.

use async_trait::async_trait;
use common::ids::UserId;
use common::media::{ByteSize, ContentHash, ContentType, Filename, MediaRef, MediaSource};
use common::pagination::{PageOffset, RowLimit};
use common::tagged_url::MediaSourceUrl;
use common::time::UtcInstant;
use sqlx::{Database, FromRow, Pool};

use crate::InstanceId;
use crate::backend::Backend;
use crate::helpers;
use crate::posts::MediaReferenceEvidence;
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

/// Errors that can occur when deleting a media record.
#[derive(Debug, Error)]
pub enum DeleteMediaError {
    /// The specified media record does not exist.
    #[error("media not found")]
    NotFound,
    /// An unexpected database error occurred.
    #[error(transparent)]
    Internal(#[from] sqlx::Error),
}

/// What [`MediaStorage::try_delete_media`] did.
///
/// A bare `bool` cannot carry the third case — the record was never there — which
/// [`DeleteMediaError::NotFound`] keeps distinct (spec D8).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TryDeleteOutcome {
    /// The record was removed.
    Deleted,
    /// The record was left in place because deleting it would violate either the
    /// caller's unforced own-reference guard or the global rowless-reference guard.
    RefusedReferenced,
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
    async fn create_media(&self, record: &MediaRecord) -> Result<(), CreateMediaError>;

    /// Fetches a single media record by its composite key.
    async fn get_media(
        &self,
        user_id: UserId,
        sha256: &ContentHash,
        filename: &Filename,
        source: &MediaSource,
    ) -> sqlx::Result<Option<MediaRecord>>;

    /// Lists media records for a user, with optional filtering and pagination.
    // Explicit `'a` for `mockall::automock` — see
    // `PostStorage::list_published_by_user`.
    async fn list_media<'a>(
        &self,
        user_id: UserId,
        source: Option<&'a MediaSource>,
        limit: RowLimit,
        offset: PageOffset,
    ) -> sqlx::Result<Vec<MediaRecord>>;

    /// Deletes a media record, refusing when `force` is absent and one of
    /// `user_id`'s live posts references it, or when deleting it would leave a
    /// live Post anywhere naming a file with no remaining media row.
    ///
    /// The guards and the delete are **one statement**, so the storage decision has
    /// no check-then-delete window (spec D8, #721).
    ///
    /// # Errors
    ///
    /// Returns [`DeleteMediaError::NotFound`] if no such record exists — the case a
    /// refusal is distinguished from by a follow-up existence check on the cold path.
    async fn try_delete_media(
        &self,
        user_id: UserId,
        media: &MediaRef,
        current_instance_id: &InstanceId,
        evidence: &MediaReferenceEvidence,
        force: bool,
    ) -> Result<TryDeleteOutcome, DeleteMediaError>;

    /// Whether the physical file named by `media` can be unlinked after a row
    /// delete: no remaining media row and no live Post anywhere still names it.
    async fn media_entry_is_reclaimable(
        &self,
        media: &MediaRef,
        current_instance_id: &InstanceId,
        evidence: &MediaReferenceEvidence,
    ) -> sqlx::Result<bool>;

    /// Calculates the total storage used by a user's uploads (in bytes).
    async fn get_user_upload_usage(&self, user_id: UserId) -> sqlx::Result<ByteSize>;

    /// Calculates total storage used by all local uploads (in bytes).
    async fn total_upload_bytes(&self) -> sqlx::Result<ByteSize>;

    /// Finds a media record by its content hash and source across all users.
    ///
    /// This is used to avoid duplicate downloads of remote content.
    async fn find_by_hash(
        &self,
        sha256: &ContentHash,
        source: &MediaSource,
    ) -> sqlx::Result<Option<MediaRecord>>;
}

/// Backend-specific divergence for [`MediaStore`].
///
/// [`get_user_upload_usage`][MediaDialect::get_user_upload_usage] diverges
/// because Postgres requires an explicit `::bigint` cast on the
/// `COALESCE(SUM(…), 0)` expression while `SQLite` does not support that syntax.
/// It is the only divergence: the delete is shared on [`MediaStore`] because
/// `RETURNING` + `fetch_optional` asks "did a row match" generically, with no
/// need to monomorphise over `.rows_affected()` on the per-backend
/// `DB::QueryResult` types ([`MediaStorage::try_delete_media`], #711).
#[async_trait]
pub trait MediaDialect: Backend {
    /// Returns the total upload bytes for `user_id` using backend-appropriate SQL.
    async fn get_user_upload_usage(pool: &Pool<Self>, user_id: UserId) -> sqlx::Result<ByteSize>;

    /// Acquires this transaction's stable lock for one media identity.
    async fn lock_media_reference(
        conn: &mut Self::Connection,
        media: &MediaRef,
    ) -> sqlx::Result<()>;

    /// Returns the total upload bytes across all users using backend-appropriate SQL.
    async fn total_upload_bytes(pool: &Pool<Self>) -> sqlx::Result<ByteSize>;

    /// Executes the locked, conditional delete for a concrete `SQLx` dialect.
    /// `true` means the row was deleted; `false` preserves the caller's
    /// NotFound-versus-refusal classification.
    async fn try_delete_media(
        pool: &Pool<Self>,
        user_id: UserId,
        media: &MediaRef,
        current_instance_id: &InstanceId,
        evidence: &MediaReferenceEvidence,
        force: bool,
    ) -> sqlx::Result<bool>;

    /// Executes the locked global reclaimability decision for a concrete dialect.
    async fn media_entry_is_reclaimable(
        pool: &Pool<Self>,
        media: &MediaRef,
        current_instance_id: &InstanceId,
        evidence: &MediaReferenceEvidence,
    ) -> sqlx::Result<bool>;
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
    helpers::MediaRow: for<'r> sqlx::FromRow<'r, DB::Row>,
    for<'q> i64: sqlx::Encode<'q, DB> + sqlx::Type<DB>,
    for<'q> &'q str: sqlx::Encode<'q, DB> + sqlx::Type<DB>,
    // `ContentHash`/`Filename` bind and decode as themselves via the ADR-0071 sqlx
    // bridge (the `sha256`/`filename` columns in `MediaRow` decode into their
    // newtypes, and the write/lookup binds encode `&ContentHash`/`&Filename`).
    String: sqlx::Type<DB>,
    for<'q> String: sqlx::Encode<'q, DB>,
    // `source_url` binds as `Option<MediaSourceUrl>` (#675). The newtype's own `Type`/`Encode`
    // follow from the `String` bounds above via the generic `StrNewtype` bridge, but the
    // `Option` wrapper has to be named explicitly — same reason the `Option<String>` bound
    // it replaces was spelled out.
    for<'q> Option<MediaSourceUrl>: sqlx::Encode<'q, DB> + sqlx::Type<DB>,
    // `RowLimit`/`PageOffset` bind as themselves via the ADR-0071 sqlx bridge (both
    // delegate to `i64`) — the listing's `LIMIT`/`OFFSET` placeholders (#696).
    for<'q> RowLimit: sqlx::Encode<'q, DB> + sqlx::Type<DB>,
    for<'q> PageOffset: sqlx::Encode<'q, DB> + sqlx::Type<DB>,
    for<'q> UtcInstant: sqlx::Encode<'q, DB> + sqlx::Type<DB>,
    // `try_delete_media` binds `force` into the guard's boolean expressions.
    for<'q> bool: sqlx::Encode<'q, DB> + sqlx::Type<DB>,
    for<'q> i64: sqlx::Decode<'q, DB>,
    usize: sqlx::ColumnIndex<DB::Row>,
    for<'c> &'c mut DB::Connection: sqlx::Executor<'c, Database = DB>,
    for<'c> &'c Pool<DB>: sqlx::Executor<'c, Database = DB>,
    for<'q> DB::Arguments<'q>: sqlx::IntoArguments<'q, DB>,
{
    #[tracing::instrument(
        name = "storage.media.create",
        skip(self, record),
        fields(db.system = DB::DB_SYSTEM)
    )]
    async fn create_media(&self, record: &MediaRecord) -> Result<(), CreateMediaError> {
        let result = sqlx::query(
            "INSERT INTO media (user_id, sha256, filename, source, content_type, size_bytes, source_url, created_at)
             VALUES ($1, $2, $3, $4, $5, $6, $7, $8)",
        )
        .bind(record.user_id)
        .bind(&record.sha256)
        .bind(&record.filename)
        .bind(record.source)
        .bind(&record.content_type)
        .bind(record.size_bytes)
        .bind(record.source_url.clone())
        .bind(record.created_at)
        .execute(&self.pool)
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
    ) -> sqlx::Result<Option<MediaRecord>> {
        let row = sqlx::query_as::<_, helpers::MediaRow>(
            "SELECT user_id, sha256, filename, source, content_type, size_bytes, source_url, created_at
             FROM media
             WHERE user_id = $1 AND sha256 = $2 AND filename = $3 AND source = $4",
        )
        .bind(user_id)
        .bind(sha256)
        .bind(filename)
        .bind(*source)
        .fetch_optional(&self.pool)
        .await?;

        Ok(row.map(helpers::media_record_from_row))
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
    ) -> sqlx::Result<Vec<MediaRecord>> {
        // Fetch raw rows (not `query_as::<MediaRow>`) so each row decodes
        // independently: the `sha256`/`filename` columns decode into their newtypes
        // *inside* `MediaRow::from_row` (#438), so a single corrupt row would fail a
        // whole `query_as` `fetch_all`. Decoding per row (as the feed-event claim
        // mapper does) lets us skip the bad one and keep the rest.
        let rows = if let Some(src) = source {
            sqlx::query(
                "SELECT user_id, sha256, filename, source, content_type, size_bytes, source_url, created_at
                 FROM media
                 WHERE user_id = $1 AND source = $2
                 ORDER BY created_at DESC
                 LIMIT $3 OFFSET $4",
            )
            .bind(user_id)
            .bind(*src)
            .bind(limit)
            .bind(offset)
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
            .bind(user_id)
            .bind(limit)
            .bind(offset)
            .fetch_all(&self.pool)
            .await?
        };

        // Skip (don't fail the whole list on) a row that fails to decode; direct
        // lookups (`get_media`/`find_by_hash`) stay strict
        // (docs/adr/0122-one-bad-row-must-not-stop-the-scan.md).
        Ok(rows
            .iter()
            .filter_map(|row| {
                match helpers::MediaRow::from_row(row).map(helpers::media_record_from_row) {
                    Ok(record) => Some(record),
                    Err(error) => {
                        tracing::warn!(%error, "skipping undecodable media row in list_media");
                        None
                    }
                }
            })
            .collect())
    }

    #[tracing::instrument(
        name = "storage.media.try_delete",
        skip(self, media),
        fields(db.system = DB::DB_SYSTEM)
    )]
    async fn try_delete_media(
        &self,
        user_id: UserId,
        media: &MediaRef,
        current_instance_id: &InstanceId,
        evidence: &MediaReferenceEvidence,
        force: bool,
    ) -> Result<TryDeleteOutcome, DeleteMediaError> {
        let removed = DB::try_delete_media(
            &self.pool,
            user_id,
            media,
            current_instance_id,
            evidence,
            force,
        )
        .await?;
        if removed {
            return Ok(TryDeleteOutcome::Deleted);
        }

        let present = sqlx::query(
            "SELECT 1 FROM media \
             WHERE user_id = $1 AND source = $2 AND sha256 = $3 AND filename = $4",
        )
        .bind(user_id)
        .bind(media.source)
        .bind(&media.sha256)
        .bind(&media.filename)
        .fetch_optional(&self.pool)
        .await?;

        if present.is_some() {
            Ok(TryDeleteOutcome::RefusedReferenced)
        } else {
            Err(DeleteMediaError::NotFound)
        }
    }

    #[tracing::instrument(
        name = "storage.media.entry_reclaimable",
        skip(self, media),
        fields(db.system = DB::DB_SYSTEM)
    )]
    async fn media_entry_is_reclaimable(
        &self,
        media: &MediaRef,
        current_instance_id: &InstanceId,
        evidence: &MediaReferenceEvidence,
    ) -> sqlx::Result<bool> {
        DB::media_entry_is_reclaimable(&self.pool, media, current_instance_id, evidence).await
    }

    #[tracing::instrument(
        name = "storage.media.upload_usage",
        skip(self),
        fields(db.system = DB::DB_SYSTEM)
    )]
    async fn get_user_upload_usage(&self, user_id: UserId) -> sqlx::Result<ByteSize> {
        // The dialect twin decodes `COALESCE(SUM(…), 0)` straight into `ByteSize`; the
        // bridge's bound-checking `Decode` rejects a negative total at the column.
        DB::get_user_upload_usage(&self.pool, user_id).await
    }

    #[tracing::instrument(
        name = "storage.media.total_upload_bytes",
        skip(self),
        fields(db.system = DB::DB_SYSTEM)
    )]
    async fn total_upload_bytes(&self) -> sqlx::Result<ByteSize> {
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
    ) -> sqlx::Result<Option<MediaRecord>> {
        let row = sqlx::query_as::<_, helpers::MediaRow>(
            "SELECT user_id, sha256, filename, source, content_type, size_bytes, source_url, created_at
             FROM media
             WHERE sha256 = $1 AND source = $2
             LIMIT 1",
        )
        .bind(sha256)
        .bind(*source)
        .fetch_optional(&self.pool)
        .await?;

        Ok(row.map(helpers::media_record_from_row))
    }
}

// The media site-config keys live in the closed host registry
// (`host::config_key::SiteConfigKey::{MediaMaxFileSizeBytes, MediaUserQuotaBytes}`, #687).
// The defaults (50 MiB / 1 GiB) live on the `common::media::MaxFileSize` /
// `UserQuota` newtypes' `#[num_newtype(default = …)]`, applied by the
// `SiteConfigStorage::get_media_*` getters.

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::{
        Backend, MEDIA_TEST_SHA256, SeedUser, TestEnv, backends, create_post_via_service,
        media_ref_for, media_row_exists, media_url_for, seed_media, seed_users,
    };
    use crate::{PersistedMediaReference, PersistedMediaSubject, ProvenForeignReference};
    use common::media::{MediaReferenceForm, MediaReferenceKind};
    use common::test_support::{
        parse_byte_size, parse_content_hash, parse_content_type, parse_filename, parse_page_offset,
        parse_post_body, parse_row_limit,
    };
    use rstest::*;
    use rstest_reuse::*;
    use std::{sync::Arc, time::Duration};
    use tokio::{sync::oneshot, time::timeout};

    /// A reference writer cannot pass a held media lock. Rolling that lock back
    /// releases the waiter; its newly-live row then defeats evidence collected for
    /// the earlier foreign reference instead of permitting the owner-row delete.
    #[apply(backends)]
    #[tokio::test]
    async fn post_reference_insert_serializes_with_foreign_evidence_delete(
        #[case] backend: Backend,
    ) {
        let env = backend.setup().await;
        let [owner, accounting_owner] = seed_users::<2>(&env.state).await;
        let media = seed_media(&env.state, owner, "serialized.jpg").await;
        seed_media(&env.state, accounting_owner, "serialized.jpg").await;
        let form: MediaReferenceForm = media_url_for("serialized.jpg")
            .parse()
            .expect("valid media reference form");
        let foreign_post = create_post_via_service(
            &env.state,
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
            let state = Arc::clone(&env.state);
            let body = parse_post_body(&format!("new reference\n\n<img src=\"{form}\">"));
            async move {
                started_tx.send(()).expect("parent waits for writer start");
                create_post_via_service(&state, owner, body).await;
                finished_tx
                    .send(())
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
        finished_rx.await.expect("writer completed after rollback");
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
            let state = Arc::clone(&env.state);
            let media = media.clone();
            let instance_id = env.base.instance_id().clone();
            let evidence = evidence.clone();
            async move {
                delete_started_tx
                    .send(())
                    .expect("parent waits for delete start");
                let result = state
                    .media
                    .try_delete_media(owner, &media, &instance_id, &evidence, false)
                    .await;
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
            TryDeleteOutcome::RefusedReferenced,
            "the new unevidenced reference must prevent the owner-row delete"
        );
        delete.await.expect("delete task does not panic");
        assert!(media_row_exists(&env.state, owner, &media).await);
    }

    /// Reclamation takes the same target lock as writes and deletion, so it cannot
    /// decide a file is orphaned while a reference writer is waiting on that target.
    #[apply(backends)]
    #[tokio::test]
    async fn reclamation_serializes_on_the_media_reference_lock(#[case] backend: Backend) {
        let env = backend.setup().await;
        let [user] = seed_users::<1>(&env.state).await;
        let media = seed_media(&env.state, user, "reclaim-lock.jpg").await;
        env.base
            .pool()
            .execute(&format!(
                "DELETE FROM media WHERE user_id = {user} AND source = '{}' \
                 AND sha256 = '{}' AND filename = '{}'",
                media.source, media.sha256, media.filename
            ))
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
        let state = Arc::clone(&env.state);
        let instance_id = env.base.instance_id().clone();
        let reclaim = tokio::spawn(async move {
            started_tx.send(()).expect("parent waits for reclaim start");
            let result = state
                .media
                .media_entry_is_reclaimable(
                    &media,
                    &instance_id,
                    &MediaReferenceEvidence::new(instance_id.clone()),
                )
                .await;
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

    /// How many posts the concurrency exercise writes while the guard is hammered, and
    /// how many unforced deletes it attempts against them. Large enough that the two
    /// interleave for the whole run on both backends; small enough to stay a unit test.
    const ROUNDS: usize = 100;

    #[apply(backends)]
    #[tokio::test]
    async fn content_hash_and_filename_round_trip_through_create_and_get(#[case] backend: Backend) {
        let env = backend.setup().await;
        let user_id = SeedUser::new().seed(&env.state).await.user_id;
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
        env.state.media.create_media(&record).await.unwrap();
        let got = env
            .state
            .media
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
        let user_id = SeedUser::new().seed(&env.state).await.user_id;
        // A non-canonical filename (`../evil`) bypasses `Filename` validation — only
        // reachable via DB tampering. The `sha256`/`source` keys stay valid so the row
        // is found; the validating bridge `Decode` then rejects the `filename` column
        // on read as a column-decode error (`find_by_hash` is strict, unlike `list_media`).
        env.base
            .pool()
            .execute(&format!(
                "INSERT INTO media (user_id, sha256, filename, source, content_type, size_bytes) \
                 VALUES ({}, '{MEDIA_TEST_SHA256}', '../evil', 'upload', 'image/jpeg', 1)",
                i64::from(user_id)
            ))
            .await
            .unwrap();
        let err = env
            .state
            .media
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
        let user_id = SeedUser::new().seed(&env.state).await.user_id;
        // A negative `size_bytes` bypasses `ByteSize` validation — only reachable via DB
        // tampering. On read, `media_record_from_row` wraps the column through the validating
        // `ByteSize::try_from`, which rejects it as a column-decode error.
        env.base
            .pool()
            .execute(&format!(
                "INSERT INTO media (user_id, sha256, filename, source, content_type, size_bytes) \
                 VALUES ({}, '{MEDIA_TEST_SHA256}', 'photo.jpg', 'upload', 'image/jpeg', -1)",
                i64::from(user_id)
            ))
            .await
            .unwrap();
        let err = env
            .state
            .media
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
        let user_id = SeedUser::new().seed(&env.state).await.user_id;
        // A negative `size_bytes` upload row (DB tampering) makes `SUM(size_bytes)` negative;
        // the sum decodes into `ByteSize`, whose bound-checking `Decode` rejects the negative
        // total as a column-decode error.
        env.base
            .pool()
            .execute(&format!(
                "INSERT INTO media (user_id, sha256, filename, source, content_type, size_bytes) \
                 VALUES ({}, '{MEDIA_TEST_SHA256}', 'photo.jpg', 'upload', 'image/jpeg', -5)",
                i64::from(user_id)
            ))
            .await
            .unwrap();
        let err = env
            .state
            .media
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
        let user_id = SeedUser::new().seed(&env.state).await.user_id;
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
        env.state.media.create_media(&good).await.unwrap();
        // A second row's `sha256` is tampered to a non-hex value — only reachable via
        // direct DB access, since `ContentHash::from_str` requires 64 lowercase hex chars.
        // Every media read keys the query *on* `sha256`, so the observable behavior is
        // `list_media`'s per-row skip: the validating bridge `Decode` rejects the
        // non-canonical hash and the row is dropped rather than surfaced (mirrors the
        // `filename` decode handling; #438). The skip *is* the proof `Decode` rejected it.
        env.base
            .pool()
            .execute(&format!(
                "INSERT INTO media (user_id, sha256, filename, source, content_type, size_bytes) \
                 VALUES ({}, 'not-a-valid-hash', 'bad.jpg', 'upload', 'image/jpeg', 1)",
                i64::from(user_id)
            ))
            .await
            .unwrap();
        let listed = env
            .state
            .media
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
    async fn create_media_with_closed_pool_returns_error(#[case] backend: Backend) {
        let TestEnv { state, base } = backend.setup().await;
        base.close_pool().await;
        let record = MediaRecord {
            user_id: UserId::from(1),
            sha256: parse_content_hash(MEDIA_TEST_SHA256),
            filename: parse_filename("test.jpg"),
            source: MediaSource::Upload,
            content_type: parse_content_type("image/jpeg"),
            size_bytes: parse_byte_size("1024"),
            source_url: None,
            created_at: UtcInstant::now(),
        };
        let result = state.media.create_media(&record).await;
        assert!(result.is_err());
    }

    #[apply(backends)]
    #[tokio::test]
    async fn get_media_with_closed_pool_returns_error(#[case] backend: Backend) {
        let TestEnv { state, base } = backend.setup().await;
        base.close_pool().await;
        let result = state
            .media
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
        let TestEnv { state, base } = backend.setup().await;
        base.close_pool().await;
        let result = state
            .media
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
    async fn try_delete_media_with_closed_pool_returns_error(#[case] backend: Backend) {
        let TestEnv { state, base } = backend.setup().await;
        base.close_pool().await;
        let result = state
            .media
            .try_delete_media(
                UserId::from(1),
                &media_ref_for("test.jpg"),
                base.instance_id(),
                &MediaReferenceEvidence::new(base.instance_id().clone()),
                false,
            )
            .await;
        assert!(matches!(result, Err(DeleteMediaError::Internal(_))));
    }

    #[apply(backends)]
    #[tokio::test]
    async fn try_delete_media_refuses_a_referenced_item_without_force(#[case] backend: Backend) {
        // A17b.
        let env = backend.setup().await;
        let [user] = seed_users::<1>(&env.state).await;
        let media = seed_media(&env.state, user, "photo.jpg").await;
        let embed = format!("<img src=\"{}\">", media_url_for("photo.jpg"));
        create_post_via_service(&env.state, user, parse_post_body(&embed)).await;

        assert_eq!(
            env.state
                .media
                .try_delete_media(
                    user,
                    &media,
                    env.base.instance_id(),
                    &MediaReferenceEvidence::new(env.base.instance_id().clone()),
                    false,
                )
                .await
                .expect("the guarded delete succeeds as a query"),
            TryDeleteOutcome::RefusedReferenced
        );
        assert!(
            media_row_exists(&env.state, user, &media).await,
            "refusal leaves the row"
        );
    }

    #[apply(backends)]
    #[tokio::test]
    async fn foreign_evidence_exempts_only_the_exact_persisted_reference(#[case] backend: Backend) {
        let env = backend.setup().await;
        let [user] = seed_users::<1>(&env.state).await;
        let media = seed_media(&env.state, user, "exact.jpg").await;
        let form: MediaReferenceForm = media_url_for("exact.jpg")
            .parse()
            .expect("valid media reference form");
        let post_id = create_post_via_service(
            &env.state,
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
            env.state
                .media
                .try_delete_media(user, &media, env.base.instance_id(), &near_match, false)
                .await
                .expect("near-match guard query succeeds"),
            TryDeleteOutcome::RefusedReferenced,
            "different kind/form evidence must not exempt the local row"
        );

        let exact =
            PersistedMediaReference::new(post_id, media.clone(), MediaReferenceKind::Local, form);
        let mut exact_match = MediaReferenceEvidence::new(env.base.instance_id().clone());
        assert!(exact_match.insert(ProvenForeignReference::new(
            exact,
            env.base.instance_id().clone(),
        )));
        assert_eq!(
            env.state
                .media
                .try_delete_media(user, &media, env.base.instance_id(), &exact_match, false)
                .await
                .expect("exact-evidence delete succeeds"),
            TryDeleteOutcome::Deleted
        );
    }

    #[apply(backends)]
    #[tokio::test]
    async fn reclaimability_uses_the_same_exact_evidence_guard(#[case] backend: Backend) {
        let env = backend.setup().await;
        let [user] = seed_users::<1>(&env.state).await;
        let media = seed_media(&env.state, user, "reclaim.jpg").await;
        let form: MediaReferenceForm = media_url_for("reclaim.jpg")
            .parse()
            .expect("valid media reference form");
        let post_id = create_post_via_service(
            &env.state,
            user,
            parse_post_body(&format!("<img src=\"{form}\">")),
        )
        .await;
        env.base
            .pool()
            .execute(&format!(
                "DELETE FROM media WHERE user_id = {user} AND source = '{}' \
                 AND sha256 = '{}' AND filename = '{}'",
                media.source, media.sha256, media.filename
            ))
            .await
            .expect("remove accounting row");

        let empty = MediaReferenceEvidence::new(env.base.instance_id().clone());
        assert!(
            !env.state
                .media
                .media_entry_is_reclaimable(&media, env.base.instance_id(), &empty)
                .await
                .expect("live reference prevents reclamation")
        );
        let mut exact = MediaReferenceEvidence::new(env.base.instance_id().clone());
        assert!(exact.insert(ProvenForeignReference::new(
            PersistedMediaReference::new(post_id, media.clone(), MediaReferenceKind::Local, form),
            env.base.instance_id().clone(),
        )));
        assert!(
            env.state
                .media
                .media_entry_is_reclaimable(&media, env.base.instance_id(), &exact)
                .await
                .expect("exact foreign evidence makes row reclaimable")
        );
    }
    /// A foreign result for a current row cannot authorize deleting an unseen
    /// retained revision of the same Post. The exact revision proof may.
    #[apply(backends)]
    #[tokio::test]
    async fn revision_subject_requires_its_own_exact_foreign_evidence(#[case] backend: Backend) {
        let env = backend.setup().await;
        let [owner] = seed_users::<1>(&env.state).await;
        let media = seed_media(&env.state, owner, "revision-evidence.jpg").await;
        let form: MediaReferenceForm = media_url_for("revision-evidence.jpg")
            .parse()
            .expect("valid media reference form");
        let post_id = create_post_via_service(
            &env.state,
            owner,
            parse_post_body(&format!("<img src=\"{form}\">")),
        )
        .await;
        let posts = Arc::clone(&env.state.posts);
        let outcome = env
            .state
            .write_scope
            .run(move |transaction| {
                Box::pin(async move { posts.soft_delete_post(transaction, post_id, owner).await })
            })
            .await
            .expect("deletion captures the prior media subject");
        assert!(matches!(outcome, common::MutationOutcome::Confirmed(())));

        let references = env
            .state
            .posts
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
            env.state
                .posts
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
            env.state
                .media
                .try_delete_media(
                    owner,
                    &media,
                    env.base.instance_id(),
                    &current_evidence,
                    false,
                )
                .await
                .expect("guard query succeeds"),
            TryDeleteOutcome::RefusedReferenced,
            "current evidence cannot exempt a retained revision subject"
        );

        let mut revision_evidence = MediaReferenceEvidence::new(env.base.instance_id().clone());
        assert!(revision_evidence.insert(ProvenForeignReference::new(
            revision.clone(),
            env.base.instance_id().clone(),
        )));
        assert_eq!(
            env.state
                .media
                .try_delete_media(
                    owner,
                    &media,
                    env.base.instance_id(),
                    &revision_evidence,
                    false,
                )
                .await
                .expect("guard query succeeds"),
            TryDeleteOutcome::RefusedReferenced,
            "the unexamined deleted-current subject remains protected"
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
            env.state
                .media
                .try_delete_media(
                    owner,
                    &media,
                    env.base.instance_id(),
                    &complete_evidence,
                    false,
                )
                .await
                .expect("guard query succeeds"),
            TryDeleteOutcome::Deleted,
            "every retained subject needs and accepts its own exact foreign proof"
        );
    }

    #[apply(backends)]
    #[tokio::test]
    async fn try_delete_media_force_overrides_own_retained_reference(#[case] backend: Backend) {
        let env = backend.setup().await;
        let [user] = seed_users::<1>(&env.state).await;
        let media = seed_media(&env.state, user, "photo.jpg").await;
        let embed = format!("<img src=\"{}\">", media_url_for("photo.jpg"));
        create_post_via_service(&env.state, user, parse_post_body(&embed)).await;

        assert_eq!(
            env.state
                .media
                .try_delete_media(
                    user,
                    &media,
                    env.base.instance_id(),
                    &MediaReferenceEvidence::new(env.base.instance_id().clone()),
                    true,
                )
                .await
                .expect("the forced delete succeeds as a query"),
            TryDeleteOutcome::Deleted
        );
        assert!(
            !media_row_exists(&env.state, user, &media).await,
            "force deliberately permits losing the owner's reconstruction"
        );
    }

    #[apply(backends)]
    #[tokio::test]
    async fn try_delete_media_allows_force_when_another_row_accounts_for_reference(
        #[case] backend: Backend,
    ) {
        let env = backend.setup().await;
        let [owner, other] = seed_users::<2>(&env.state).await;
        let media = seed_media(&env.state, owner, "photo.jpg").await;
        seed_media(&env.state, other, "photo.jpg").await;
        let embed = format!("<img src=\"{}\">", media_url_for("photo.jpg"));
        create_post_via_service(&env.state, owner, parse_post_body(&embed)).await;

        assert_eq!(
            env.state
                .media
                .try_delete_media(
                    owner,
                    &media,
                    env.base.instance_id(),
                    &MediaReferenceEvidence::new(env.base.instance_id().clone()),
                    true,
                )
                .await
                .expect("the forced delete succeeds"),
            TryDeleteOutcome::Deleted
        );
        assert!(!media_row_exists(&env.state, owner, &media).await);
        assert!(media_row_exists(&env.state, other, &media).await);
    }

    #[apply(backends)]
    #[tokio::test]
    async fn try_delete_media_deletes_an_unreferenced_item(#[case] backend: Backend) {
        // A17b, the other half: nothing references it, so an unforced delete goes through.
        let env = backend.setup().await;
        let [user] = seed_users::<1>(&env.state).await;
        let media = seed_media(&env.state, user, "photo.jpg").await;

        assert_eq!(
            env.state
                .media
                .try_delete_media(
                    user,
                    &media,
                    env.base.instance_id(),
                    &MediaReferenceEvidence::new(env.base.instance_id().clone()),
                    false,
                )
                .await
                .expect("the delete succeeds"),
            TryDeleteOutcome::Deleted
        );
        assert!(!media_row_exists(&env.state, user, &media).await);
    }

    #[apply(backends)]
    #[tokio::test]
    async fn try_delete_media_reports_not_found_distinctly_from_refusal(#[case] backend: Backend) {
        // A17c — the conditional statement returns no row in both cases, so this pins
        // that the follow-up existence check still separates them, preserving today's
        // `DeleteMediaError::NotFound`.
        let env = backend.setup().await;
        let [user] = seed_users::<1>(&env.state).await;

        let result = env
            .state
            .media
            .try_delete_media(
                user,
                &media_ref_for("never-uploaded.jpg"),
                env.base.instance_id(),
                &MediaReferenceEvidence::new(env.base.instance_id().clone()),
                false,
            )
            .await;

        assert!(
            matches!(result, Err(DeleteMediaError::NotFound)),
            "expected NotFound, got {result:?}"
        );
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
        let [user] = seed_users::<1>(&env.state).await;
        let media = seed_media(&env.state, user, "photo.jpg").await;
        // Each body carries a distinct leading line so the service path derives a distinct
        // title, and hence a distinct slug: identical bodies would collide on the slug and
        // exhaust the creator's attempt budget long before the round count here. The embed
        // — the only part the guard reads — is the same in every one.
        let embed = format!("<img src=\"{}\">", media_url_for("photo.jpg"));

        // One reference exists before any delete is attempted, and none is ever removed, so
        // every unforced delete from here on must refuse.
        create_post_via_service(
            &env.state,
            user,
            parse_post_body(&format!("reference 0\n\n{embed}")),
        )
        .await;

        let writer = tokio::spawn({
            let state = Arc::clone(&env.state);
            async move {
                for round in 1..=ROUNDS {
                    create_post_via_service(
                        &state,
                        user,
                        parse_post_body(&format!("reference {round}\n\n{embed}")),
                    )
                    .await;
                }
            }
        });

        for _ in 0..ROUNDS {
            let outcome = env
                .state
                .media
                .try_delete_media(
                    user,
                    &media,
                    env.base.instance_id(),
                    &MediaReferenceEvidence::new(env.base.instance_id().clone()),
                    false,
                )
                .await
                .expect("no SQLITE_BUSY under concurrency");
            assert_eq!(
                outcome,
                TryDeleteOutcome::RefusedReferenced,
                "a live reference exists throughout, so no unforced delete may succeed"
            );
        }
        writer.await.expect("the concurrent writer does not panic");
        assert!(media_row_exists(&env.state, user, &media).await);
    }

    #[apply(backends)]
    #[tokio::test]
    async fn get_user_upload_usage_with_closed_pool_returns_error(#[case] backend: Backend) {
        let TestEnv { state, base } = backend.setup().await;
        base.close_pool().await;
        let result = state.media.get_user_upload_usage(UserId::from(1)).await;
        assert!(result.is_err());
    }

    #[apply(backends)]
    #[tokio::test]
    async fn total_upload_bytes_sums_upload_rows(#[case] backend: Backend) {
        let env = backend.setup().await;
        let [alice] = seed_users(&env.state).await;
        seed_media(&env.state, alice, "a.jpg").await;
        seed_media(&env.state, alice, "b.jpg").await;

        let total = env.state.media.total_upload_bytes().await.unwrap();

        assert_eq!(total, parse_byte_size("2"));
    }

    #[apply(backends)]
    #[tokio::test]
    async fn total_upload_bytes_excludes_non_upload_sources(#[case] backend: Backend) {
        let env = backend.setup().await;
        let [alice] = seed_users(&env.state).await;
        seed_media(&env.state, alice, "upload.jpg").await;
        env.base
            .pool()
            .execute(
                "INSERT INTO media (user_id, sha256, filename, source, content_type, size_bytes) \
                 VALUES (1, 'aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa', \
                         'remote.jpg', 'cached', 'image/jpeg', 99)",
            )
            .await
            .unwrap();

        let total = env.state.media.total_upload_bytes().await.unwrap();

        assert_eq!(total, parse_byte_size("1"));
    }

    #[apply(backends)]
    #[tokio::test]
    async fn find_by_hash_with_closed_pool_returns_error(#[case] backend: Backend) {
        let TestEnv { state, base } = backend.setup().await;
        base.close_pool().await;
        let result = state
            .media
            .find_by_hash(&parse_content_hash(MEDIA_TEST_SHA256), &MediaSource::Upload)
            .await;
        assert!(result.is_err());
    }
}
