//! Media file metadata storage.

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use common::media::{ContentHash, Filename};
use sqlx::{Database, Pool};
use thiserror::Error;

use crate::backend::Backend;
use common::ids::UserId;

/// Source of a media record.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum MediaSource {
    /// File uploaded directly by a local user.
    Upload,
    /// Remote file cached locally by the system.
    Cached,
}

impl MediaSource {
    /// Returns the string representation used in the database.
    #[must_use]
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Upload => "upload",
            Self::Cached => "cached",
        }
    }
}

impl std::fmt::Display for MediaSource {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

impl std::str::FromStr for MediaSource {
    type Err = InvalidMediaSource;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "upload" => Ok(Self::Upload),
            "cached" => Ok(Self::Cached),
            _ => Err(InvalidMediaSource),
        }
    }
}

/// Error returned when a string cannot be parsed as a [`MediaSource`].
#[derive(Debug, Error)]
#[error("media source must be \"upload\" or \"cached\"")]
pub struct InvalidMediaSource;

impl From<InvalidMediaSource> for host::error::InternalError {
    /// Reproduces the former `web::media` lift `(kind, class, public_message)`:
    /// a client validation error whose wire message is the error's `Display`,
    /// carrying the typed source instead of flattening it to a string (A19).
    fn from(error: InvalidMediaSource) -> Self {
        host::error::InternalError::validation_source(error.to_string(), error)
    }
}

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
    pub content_type: String,
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
        source: &str,
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
        .bind(record.sha256.as_ref())
        .bind(record.filename.as_ref())
        .bind(record.source.as_str())
        .bind(record.content_type.as_str())
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
        .bind(sha256.as_ref())
        .bind(filename.as_ref())
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
        let rows = if let Some(src) = source {
            sqlx::query_as::<_, crate::helpers::MediaRow>(
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
            sqlx::query_as::<_, crate::helpers::MediaRow>(
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
        // or hand-edited `sha256`/`source`/`filename` column that no longer satisfies
        // its newtype invariant. `get_media`/`find_by_hash` stay strict (a direct
        // lookup surfaces the error), but a single bad row must not 500 a user's entire
        // media list and hide every other item.
        Ok(rows
            .into_iter()
            .filter_map(|row| match crate::helpers::media_record_from_row(row) {
                Ok(record) => Some(record),
                Err(error) => {
                    tracing::warn!(%error, "skipping undecodable media row in list_media");
                    None
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
        DB::delete_media_row(&self.pool, user_id, sha256, filename, source.as_str()).await
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
        .bind(sha256.as_ref())
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
/// Default maximum file size (50 MiB) if not configured.
pub const DEFAULT_MAX_FILE_SIZE_BYTES: i64 = 52_428_800;
/// Default per-user quota (1 GiB) if not configured.
pub const DEFAULT_USER_QUOTA_BYTES: i64 = 1_073_741_824;

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::{backends, Backend, TestEnv};
    use common::test_support::{parse_content_hash, parse_filename};
    use rstest::*;
    use rstest_reuse::*;

    /// A canonical 64-char lowercase-hex content hash for fixtures.
    const HASH: &str = "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855";

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
            content_type: "image/jpeg".to_string(),
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

    #[test]
    fn media_source_as_str_returns_correct_value_for_each_variant() {
        assert_eq!(MediaSource::Upload.as_str(), "upload");
        assert_eq!(MediaSource::Cached.as_str(), "cached");
    }

    #[test]
    fn media_source_display_produces_correct_string_for_each_variant() {
        assert_eq!(MediaSource::Upload.to_string(), "upload");
        assert_eq!(MediaSource::Cached.to_string(), "cached");
    }

    #[test]
    fn media_source_from_str_parses_cached_variant() {
        assert_eq!(
            "cached".parse::<MediaSource>().unwrap(),
            MediaSource::Cached
        );
    }

    // Behavior-preserving translation of the former `web::media` lift: a client
    // validation error whose wire message is the error's `Display`, with the
    // typed source preserved on the operator side.
    #[test]
    fn from_invalid_media_source_maps_to_validation() {
        use host::error::{ErrorKind, InternalError};

        let error: InternalError = InvalidMediaSource.into();
        assert_eq!(error.kind(), ErrorKind::Validation);
        assert_eq!(error.public_message(), InvalidMediaSource.to_string());
        assert!(error
            .operator_message()
            .contains(&InvalidMediaSource.to_string()));
    }
}
