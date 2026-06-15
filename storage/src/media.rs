//! Media file metadata storage.

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use sqlx::{Database, Pool};
use thiserror::Error;

use crate::backend::Backend;

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

/// A media metadata record returned by [`MediaStorage`] queries.
#[derive(Clone, Debug)]
pub struct MediaRecord {
    /// ID of the user who owns or triggered the caching of this media.
    pub user_id: i64,
    /// SHA-256 hash of the file content (used for deduplication).
    pub sha256: String,
    /// Original filename or a generated unique name.
    pub filename: String,
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
        user_id: i64,
        sha256: &str,
        filename: &str,
        source: &MediaSource,
    ) -> sqlx::Result<Option<MediaRecord>>;

    /// Lists media records for a user, with optional filtering and pagination.
    async fn list_media(
        &self,
        user_id: i64,
        source: Option<&MediaSource>,
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
        user_id: i64,
        sha256: &str,
        filename: &str,
        source: &MediaSource,
    ) -> Result<(), DeleteMediaError>;

    /// Calculates the total storage used by a user's uploads (in bytes).
    async fn get_user_upload_usage(&self, user_id: i64) -> sqlx::Result<i64>;

    /// Finds a media record by its content hash and source across all users.
    ///
    /// This is used to avoid duplicate downloads of remote content.
    async fn find_by_hash(
        &self,
        sha256: &str,
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
    async fn get_user_upload_usage(pool: &Pool<Self>, user_id: i64) -> sqlx::Result<i64>;

    /// Deletes a media record; returns `NotFound` when no row was matched.
    async fn delete_media_row(
        pool: &Pool<Self>,
        user_id: i64,
        sha256: &str,
        filename: &str,
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
        .bind(record.user_id)
        .bind(record.sha256.as_str())
        .bind(record.filename.as_str())
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
        user_id: i64,
        sha256: &str,
        filename: &str,
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
    async fn list_media(
        &self,
        user_id: i64,
        source: Option<&MediaSource>,
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
            .bind(user_id)
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
            .bind(user_id)
            .bind(i64::from(limit))
            .bind(i64::from(offset))
            .fetch_all(&self.pool)
            .await?
        };

        rows.into_iter()
            .map(crate::helpers::media_record_from_row)
            .collect()
    }

    #[tracing::instrument(
        name = "storage.media.delete",
        skip(self),
        fields(db.system = DB::DB_SYSTEM)
    )]
    async fn delete_media(
        &self,
        user_id: i64,
        sha256: &str,
        filename: &str,
        source: &MediaSource,
    ) -> Result<(), DeleteMediaError> {
        DB::delete_media_row(&self.pool, user_id, sha256, filename, source.as_str()).await
    }

    #[tracing::instrument(
        name = "storage.media.upload_usage",
        skip(self),
        fields(db.system = DB::DB_SYSTEM)
    )]
    async fn get_user_upload_usage(&self, user_id: i64) -> sqlx::Result<i64> {
        DB::get_user_upload_usage(&self.pool, user_id).await
    }

    #[tracing::instrument(
        name = "storage.media.find_by_hash",
        skip(self),
        fields(db.system = DB::DB_SYSTEM)
    )]
    async fn find_by_hash(
        &self,
        sha256: &str,
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
/// Default maximum file size (50 MiB) if not configured.
pub const DEFAULT_MAX_FILE_SIZE_BYTES: i64 = 52_428_800;
/// Default per-user quota (1 GiB) if not configured.
pub const DEFAULT_USER_QUOTA_BYTES: i64 = 1_073_741_824;

#[cfg(test)]
mod tests {
    use super::*;

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
}
