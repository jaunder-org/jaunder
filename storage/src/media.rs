//! Media file metadata storage.

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use thiserror::Error;

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
