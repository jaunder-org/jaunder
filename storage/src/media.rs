//! Media file metadata storage.

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use common::media::{ContentHash, ContentType, Filename, MediaSource};
use sqlx::{Database, FromRow, Pool};
use thiserror::Error;

use crate::backend::Backend;
use common::ids::UserId;

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
    pub size_bytes: i64,
    /// For cached media, the original remote URL.
    pub source_url: Option<String>,
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

/// Async operations on the `media` table.
///
/// This trait manages the metadata for media files, supporting both user
/// uploads and cached remote content.
#[cfg_attr(feature = "test-utils", mockall::automock)]
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
        limit: u32,
        offset: u32,
    ) -> sqlx::Result<Vec<MediaRecord>>;

    /// Deletes a media record from the database.
    ///
    /// # Errors
    ///
    /// Returns [`DeleteMediaError::NotFound`] if the record does not exist.
    async fn delete_media(
        &self,
        user_id: UserId,
        sha256: &ContentHash,
        filename: &Filename,
        source: &MediaSource,
    ) -> Result<(), DeleteMediaError>;

    /// Calculates the total storage used by a user's uploads (in bytes).
    async fn get_user_upload_usage(&self, user_id: UserId) -> sqlx::Result<i64>;

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
///
/// [`delete_media`][MediaDialect::delete_media] is also here because calling
/// `.rows_affected()` on the generic `DB::QueryResult` associated type
/// requires no trait in sqlx 0.8 — the method exists only on the concrete
/// per-backend result types, so the implementation must be monomorphised.
/// The SQL itself is identical across backends.
#[async_trait]
pub trait MediaDialect: Backend {
    /// Returns the total upload bytes for `user_id` using backend-appropriate SQL.
    async fn get_user_upload_usage(pool: &Pool<Self>, user_id: UserId) -> sqlx::Result<i64>;

    /// Deletes a media record; returns `NotFound` when no row was matched.
    async fn delete_media_row(
        pool: &Pool<Self>,
        user_id: UserId,
        sha256: &ContentHash,
        filename: &Filename,
        source: &MediaSource,
    ) -> Result<(), DeleteMediaError>;
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
    for<'q> Option<String>: sqlx::Encode<'q, DB> + sqlx::Type<DB>,
    for<'q> DateTime<Utc>: sqlx::Encode<'q, DB> + sqlx::Type<DB>,
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
        .bind(i64::from(record.user_id))
        .bind(&record.sha256)
        .bind(&record.filename)
        .bind(record.source.as_str())
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
        .bind(i64::from(user_id))
        .bind(sha256)
        .bind(filename)
        .bind(source.as_str())
        .fetch_optional(&self.pool)
        .await?;

        row.map(crate::helpers::media_record_from_row).transpose()
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
        limit: u32,
        offset: u32,
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
            .bind(i64::from(user_id))
            .bind(src.as_str())
            .bind(i64::from(limit))
            .bind(i64::from(offset))
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
            .bind(i64::from(user_id))
            .bind(i64::from(limit))
            .bind(i64::from(offset))
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
                    .and_then(crate::helpers::media_record_from_row)
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
        name = "storage.media.delete",
        skip(self),
        fields(db.system = DB::DB_SYSTEM)
    )]
    async fn delete_media(
        &self,
        user_id: UserId,
        sha256: &ContentHash,
        filename: &Filename,
        source: &MediaSource,
    ) -> Result<(), DeleteMediaError> {
        DB::delete_media_row(&self.pool, user_id, sha256, filename, source).await
    }

    #[tracing::instrument(
        name = "storage.media.upload_usage",
        skip(self),
        fields(db.system = DB::DB_SYSTEM)
    )]
    async fn get_user_upload_usage(&self, user_id: UserId) -> sqlx::Result<i64> {
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
        .bind(source.as_str())
        .fetch_optional(&self.pool)
        .await?;

        row.map(crate::helpers::media_record_from_row).transpose()
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
    use crate::test_support::{backends, seed_user, Backend, TestEnv};
    use common::test_support::{parse_content_hash, parse_content_type, parse_filename};
    use rstest::*;
    use rstest_reuse::*;

    /// A canonical 64-char lowercase-hex content hash for fixtures.
    const HASH: &str = "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855";

    #[apply(backends)]
    #[tokio::test]
    async fn content_hash_and_filename_round_trip_through_create_and_get(#[case] backend: Backend) {
        let env = backend.setup().await;
        let user_id = seed_user(&env.state).await;
        let record = MediaRecord {
            user_id,
            sha256: parse_content_hash(HASH),
            filename: parse_filename("photo.jpg"),
            source: MediaSource::Upload,
            content_type: parse_content_type("image/jpeg"),
            size_bytes: 2048,
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
        let user_id = seed_user(&env.state).await;
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

    #[apply(backends)]
    #[tokio::test]
    async fn list_media_skips_a_row_with_a_malformed_sha256_column(#[case] backend: Backend) {
        let env = backend.setup().await;
        let user_id = seed_user(&env.state).await;
        // A valid record is stored normally.
        let good = MediaRecord {
            user_id,
            sha256: parse_content_hash(HASH),
            filename: parse_filename("good.jpg"),
            source: MediaSource::Upload,
            content_type: parse_content_type("image/jpeg"),
            size_bytes: 1,
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
            .list_media(user_id, None, 10, 0)
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
            size_bytes: 1024,
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
        let result = state.media.list_media(UserId::from(1), None, 10, 0).await;
        assert!(result.is_err());
    }

    #[apply(backends)]
    #[tokio::test]
    async fn delete_media_with_closed_pool_returns_error(#[case] backend: Backend) {
        let TestEnv { state, base } = backend.setup().await;
        base.close_pool().await;
        let result = state
            .media
            .delete_media(
                UserId::from(1),
                &parse_content_hash(HASH),
                &parse_filename("test.jpg"),
                &MediaSource::Upload,
            )
            .await;
        assert!(matches!(result, Err(DeleteMediaError::Internal(_))));
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
