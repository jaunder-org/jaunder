//! Media file metadata storage.

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use common::absolute_url::AbsoluteUrl;
use common::media::{ByteSize, ContentHash, ContentType, Filename, MediaRef, MediaSource};
use sqlx::{Database, FromRow, Pool};
use thiserror::Error;

use crate::backend::Backend;
use common::ids::UserId;
use common::pagination::{PageOffset, RowLimit};

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
    /// Typed as [`AbsoluteUrl`] ahead of any writer: every construction site currently
    /// passes `None`, because the remote-caching ingest that would populate it does not
    /// exist yet. The type is therefore the **contract for that path** — whoever builds it
    /// must supply a validated, normalized `http(s)` URL rather than whatever a feed handed
    /// them. An unparseable value would be useless by definition, since caching means
    /// fetching this URL, so rejecting it at ingest is strictly better than storing
    /// something no code can act on (#675).
    pub source_url: Option<AbsoluteUrl>,
    /// When the record was created.
    pub created_at: DateTime<Utc>,
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
    /// The record was left in place because one of the owner's live posts
    /// references it and the caller did not force the delete.
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

    /// Deletes a media record, refusing when one of `user_id`'s live posts references
    /// it unless `force`.
    ///
    /// The guard and the delete are **one statement**, so no post can start
    /// referencing the media between them (spec D8) — the reason the
    /// refuse-unless-forced policy lives in storage rather than in the caller.
    ///
    /// # Errors
    ///
    /// Returns [`DeleteMediaError::NotFound`] if no such record exists — the case a
    /// refusal is distinguished from by a follow-up existence check on the cold path.
    async fn try_delete_media(
        &self,
        user_id: UserId,
        media: &MediaRef,
        force: bool,
    ) -> Result<TryDeleteOutcome, DeleteMediaError>;

    /// Calculates the total storage used by a user's uploads (in bytes).
    async fn get_user_upload_usage(&self, user_id: UserId) -> sqlx::Result<ByteSize>;

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
/// It is the only divergence: the delete used to live here too, because reading
/// `.rows_affected()` off the generic `DB::QueryResult` associated type needs
/// monomorphising (the method exists only on the concrete per-backend result
/// types), but `RETURNING` + `fetch_optional` asks the same question generically,
/// so [`MediaStorage::try_delete_media`] is shared on [`MediaStore`] (#711).
#[async_trait]
pub trait MediaDialect: Backend {
    /// Returns the total upload bytes for `user_id` using backend-appropriate SQL.
    async fn get_user_upload_usage(pool: &Pool<Self>, user_id: UserId) -> sqlx::Result<ByteSize>;
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
    crate::helpers::MediaRow: for<'r> sqlx::FromRow<'r, DB::Row>,
    for<'q> i64: sqlx::Encode<'q, DB> + sqlx::Type<DB>,
    for<'q> &'q str: sqlx::Encode<'q, DB> + sqlx::Type<DB>,
    // `ContentHash`/`Filename` bind and decode as themselves via the sqlx bridge
    // (#438), which delegates to `String`; these bounds make that bridge available on
    // the generic backend (the `sha256`/`filename` columns in `MediaRow` decode into
    // their newtypes, and the write/lookup binds encode `&ContentHash`/`&Filename`).
    String: sqlx::Type<DB>,
    for<'q> String: sqlx::Encode<'q, DB>,
    // `source_url` binds as `Option<AbsoluteUrl>` (#675). The newtype's own `Type`/`Encode`
    // follow from the `String` bounds above via the generic `StrNewtype` bridge, but the
    // `Option` wrapper has to be named explicitly — same reason the `Option<String>` bound
    // it replaces was spelled out.
    for<'q> Option<AbsoluteUrl>: sqlx::Encode<'q, DB> + sqlx::Type<DB>,
    // `RowLimit`/`PageOffset` bind as themselves via the ADR-0071 sqlx bridge (both
    // delegate to `i64`) — the listing's `LIMIT`/`OFFSET` placeholders (#696).
    for<'q> RowLimit: sqlx::Encode<'q, DB> + sqlx::Type<DB>,
    for<'q> PageOffset: sqlx::Encode<'q, DB> + sqlx::Type<DB>,
    for<'q> DateTime<Utc>: sqlx::Encode<'q, DB> + sqlx::Type<DB>,
    // `try_delete_media` binds `force` into the guard's `($5 OR NOT EXISTS …)`.
    for<'q> bool: sqlx::Encode<'q, DB> + sqlx::Type<DB>,
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
        let row = sqlx::query_as::<_, crate::helpers::MediaRow>(
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

        Ok(row.map(crate::helpers::media_record_from_row))
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
        // independently: with the sqlx bridge (#438) the `sha256`/`filename` columns
        // now decode into their newtypes *inside* `MediaRow::from_row`, so a single
        // corrupt row would fail a whole `query_as` `fetch_all`. Decoding per row (as
        // the feed-event claim mapper does) lets us skip the bad one and keep the rest.
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

        // Skip (don't fail the whole list on) a row that fails to decode — a corrupt
        // or hand-edited `sha256`/`filename` column that no longer satisfies its
        // newtype invariant (rejected inside `from_row`), or an invalid `source`.
        // `get_media`/`find_by_hash` stay strict (a direct lookup surfaces the error),
        // but a single bad row must not 500 a user's entire media list.
        Ok(rows
            .iter()
            .filter_map(|row| {
                match crate::helpers::MediaRow::from_row(row)
                    .map(crate::helpers::media_record_from_row)
                {
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
        force: bool,
    ) -> Result<TryDeleteOutcome, DeleteMediaError> {
        // The guard and the delete are the same statement, so a `post_media` insert
        // cannot slip between them the way it could between a check and a delete
        // (spec D8). A single statement is atomic in both engines, so this needs no
        // transaction, no locking, and no isolation-level tuning. `RETURNING` is what
        // makes the outcome readable generically — `.rows_affected()` is not callable
        // on `DB::QueryResult`, which is why the delete used to be a dialect method.
        let removed = sqlx::query(
            "DELETE FROM media \
             WHERE user_id = $1 AND source = $2 AND sha256 = $3 AND filename = $4 \
               AND ($5 OR NOT EXISTS ( \
                     SELECT 1 FROM post_media pm \
                       JOIN posts p ON p.post_id = pm.post_id \
                      WHERE p.user_id = $1 AND p.deleted_at IS NULL \
                        AND pm.source = $2 AND pm.sha256 = $3 AND pm.filename = $4)) \
             RETURNING sha256",
        )
        .bind(user_id)
        .bind(media.source)
        .bind(&media.sha256)
        .bind(&media.filename)
        .bind(force)
        .fetch_optional(&self.pool)
        .await?;

        if removed.is_some() {
            return Ok(TryDeleteOutcome::Deleted);
        }

        // No row came back for one of two reasons: the record is still there and was
        // guarded, or it was never there. One existence check tells them apart, which
        // is what keeps today's `NotFound` behaviour intact. Advisory and on the cold
        // path only — the decision was already made atomically above, so asking
        // afterwards cannot reopen the race.
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
        name = "storage.media.find_by_hash",
        skip(self),
        fields(db.system = DB::DB_SYSTEM)
    )]
    async fn find_by_hash(
        &self,
        sha256: &ContentHash,
        source: &MediaSource,
    ) -> sqlx::Result<Option<MediaRecord>> {
        let row = sqlx::query_as::<_, crate::helpers::MediaRow>(
            "SELECT user_id, sha256, filename, source, content_type, size_bytes, source_url, created_at
             FROM media
             WHERE sha256 = $1 AND source = $2
             LIMIT 1",
        )
        .bind(sha256)
        .bind(*source)
        .fetch_optional(&self.pool)
        .await?;

        Ok(row.map(crate::helpers::media_record_from_row))
    }
}

/// Key for the site configuration setting for maximum file upload size.
pub const MEDIA_MAX_FILE_SIZE_BYTES_KEY: &str = "media.max_file_size_bytes";
/// Key for the site configuration setting for per-user upload quota.
pub const MEDIA_USER_QUOTA_BYTES_KEY: &str = "media.user_quota_bytes";
/// Key for the site-wide default media cache policy.
pub const MEDIA_CACHE_POLICY_DEFAULT_KEY: &str = "media.cache_policy_default";
// The defaults (50 MiB / 1 GiB) now live on the `common::media::MaxFileSize` /
// `UserQuota` newtypes' `#[num_newtype(default = …)]`, applied by the
// `SiteConfigStorage::get_media_*` getters.

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::{
        backends, create_post_via_service, media_ref_for, media_row_exists, media_url_for,
        seed_media, seed_users, Backend, SeedUser, TestEnv,
    };
    use common::test_support::{
        parse_byte_size, parse_content_hash, parse_content_type, parse_filename, parse_page_offset,
        parse_row_limit,
    };
    use rstest::*;
    use rstest_reuse::*;
    use std::sync::Arc;

    /// A canonical 64-char lowercase-hex content hash for fixtures.
    const HASH: &str = "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855";

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
            sha256: parse_content_hash(HASH),
            filename: parse_filename("photo.jpg"),
            source: MediaSource::Upload,
            content_type: parse_content_type("image/jpeg"),
            size_bytes: parse_byte_size("2048"),
            source_url: None,
            created_at: chrono::Utc::now(),
        };
        env.state.media.create_media(&record).await.unwrap();
        let got = env
            .state
            .media
            .get_media(
                user_id,
                &parse_content_hash(HASH),
                &parse_filename("photo.jpg"),
                &MediaSource::Upload,
            )
            .await
            .unwrap()
            .expect("present");
        // `sha256`/`filename` decode straight into their newtypes via the sqlx bridge (#438).
        assert_eq!(got.sha256, parse_content_hash(HASH));
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
                 VALUES ({}, '{HASH}', '../evil', 'upload', 'image/jpeg', 1)",
                i64::from(user_id)
            ))
            .await
            .unwrap();
        let err = env
            .state
            .media
            .find_by_hash(&parse_content_hash(HASH), &MediaSource::Upload)
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
    // That branch is shared with `PostFormat`'s `impl_text_column_enum!` instantiation and
    // is covered by its parse-error tests; the unknown-token rejection itself is asserted
    // in `common::media`'s `media_source_unknown_token_is_rejected_with_message`.

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
                 VALUES ({}, '{HASH}', 'photo.jpg', 'upload', 'image/jpeg', -1)",
                i64::from(user_id)
            ))
            .await
            .unwrap();
        let err = env
            .state
            .media
            .find_by_hash(&parse_content_hash(HASH), &MediaSource::Upload)
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
                 VALUES ({}, '{HASH}', 'photo.jpg', 'upload', 'image/jpeg', -5)",
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
            sha256: parse_content_hash(HASH),
            filename: parse_filename("good.jpg"),
            source: MediaSource::Upload,
            content_type: parse_content_type("image/jpeg"),
            size_bytes: parse_byte_size("1"),
            source_url: None,
            created_at: chrono::Utc::now(),
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
        assert_eq!(listed[0].sha256, parse_content_hash(HASH));
    }

    #[apply(backends)]
    #[tokio::test]
    async fn create_media_with_closed_pool_returns_error(#[case] backend: Backend) {
        let TestEnv { state, base } = backend.setup().await;
        base.close_pool().await;
        let record = MediaRecord {
            user_id: UserId::from(1),
            sha256: parse_content_hash(HASH),
            filename: parse_filename("test.jpg"),
            source: MediaSource::Upload,
            content_type: parse_content_type("image/jpeg"),
            size_bytes: parse_byte_size("1024"),
            source_url: None,
            created_at: chrono::Utc::now(),
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
                &parse_content_hash(HASH),
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
            .try_delete_media(UserId::from(1), &media_ref_for("test.jpg"), false)
            .await;
        assert!(matches!(result, Err(DeleteMediaError::Internal(_))));
    }

    #[apply(backends)]
    #[tokio::test]
    async fn try_delete_media_refuses_a_referenced_item_unless_forced(#[case] backend: Backend) {
        // A17b.
        let env = backend.setup().await;
        let [user] = seed_users::<1>(&env.state).await;
        let media = seed_media(&env.state, user, "photo.jpg").await;
        let embed = format!("<img src=\"{}\">", media_url_for("photo.jpg"));
        create_post_via_service(&env.state, user, &embed).await;

        assert_eq!(
            env.state
                .media
                .try_delete_media(user, &media, false)
                .await
                .expect("the guarded delete succeeds as a query"),
            TryDeleteOutcome::RefusedReferenced
        );
        assert!(
            media_row_exists(&env.state, user, &media).await,
            "refusal leaves the row"
        );

        assert_eq!(
            env.state
                .media
                .try_delete_media(user, &media, true)
                .await
                .expect("the forced delete succeeds"),
            TryDeleteOutcome::Deleted
        );
        assert!(!media_row_exists(&env.state, user, &media).await);
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
                .try_delete_media(user, &media, false)
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
            .try_delete_media(user, &media_ref_for("never-uploaded.jpg"), false)
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
        create_post_via_service(&env.state, user, &format!("reference 0\n\n{embed}")).await;

        let writer = tokio::spawn({
            let state = Arc::clone(&env.state);
            async move {
                for round in 1..=ROUNDS {
                    create_post_via_service(&state, user, &format!("reference {round}\n\n{embed}"))
                        .await;
                }
            }
        });

        for _ in 0..ROUNDS {
            let outcome = env
                .state
                .media
                .try_delete_media(user, &media, false)
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
    async fn find_by_hash_with_closed_pool_returns_error(#[case] backend: Backend) {
        let TestEnv { state, base } = backend.setup().await;
        base.close_pool().await;
        let result = state
            .media
            .find_by_hash(&parse_content_hash(HASH), &MediaSource::Upload)
            .await;
        assert!(result.is_err());
    }
}
