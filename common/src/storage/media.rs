use async_trait::async_trait;
use chrono::{DateTime, Utc};
use thiserror::Error;

/// Source of a media record.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum MediaSource {
    Upload,
    Cached,
}

impl MediaSource {
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

#[derive(Debug, Error)]
#[error("media source must be \"upload\" or \"cached\"")]
pub struct InvalidMediaSource;

#[derive(Clone, Debug)]
pub struct MediaRecord {
    pub user_id: i64,
    pub sha256: String,
    pub filename: String,
    pub source: MediaSource,
    pub content_type: String,
    pub size_bytes: i64,
    pub source_url: Option<String>,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Error)]
pub enum CreateMediaError {
    #[error("media already exists")]
    AlreadyExists,
    #[error(transparent)]
    Internal(#[from] sqlx::Error),
}

#[derive(Debug, Error)]
pub enum DeleteMediaError {
    #[error("media not found")]
    NotFound,
    #[error(transparent)]
    Internal(#[from] sqlx::Error),
}

#[async_trait]
pub trait MediaStorage: Send + Sync {
    /// Insert a new media record.
    ///
    /// # Errors
    ///
    /// Returns `CreateMediaError::AlreadyExists` if the record already exists,
    /// or `CreateMediaError::Internal` on database failure.
    async fn create_media(&self, record: &MediaRecord) -> Result<(), CreateMediaError>;

    /// Fetch a single media record by its composite key.
    ///
    /// # Errors
    ///
    /// Returns `Err` on database failure.
    async fn get_media(
        &self,
        user_id: i64,
        sha256: &str,
        filename: &str,
        source: &MediaSource,
    ) -> sqlx::Result<Option<MediaRecord>>;

    /// List media records for a user, optionally filtered by source.
    ///
    /// # Errors
    ///
    /// Returns `Err` on database failure.
    async fn list_media(
        &self,
        user_id: i64,
        source: Option<&MediaSource>,
        limit: u32,
        offset: u32,
    ) -> sqlx::Result<Vec<MediaRecord>>;

    /// Delete a media record by its composite key.
    ///
    /// # Errors
    ///
    /// Returns `DeleteMediaError::NotFound` if the record does not exist,
    /// or `DeleteMediaError::Internal` on database failure.
    async fn delete_media(
        &self,
        user_id: i64,
        sha256: &str,
        filename: &str,
        source: &MediaSource,
    ) -> Result<(), DeleteMediaError>;

    /// Sum of `size_bytes` for all upload records belonging to `user_id`.
    ///
    /// # Errors
    ///
    /// Returns `Err` on database failure.
    async fn get_user_upload_usage(&self, user_id: i64) -> sqlx::Result<i64>;

    /// Find a media record by hash and source (across all users).
    ///
    /// # Errors
    ///
    /// Returns `Err` on database failure.
    async fn find_by_hash(
        &self,
        sha256: &str,
        source: &MediaSource,
    ) -> sqlx::Result<Option<MediaRecord>>;
}

pub const MEDIA_MAX_FILE_SIZE_BYTES_KEY: &str = "media.max_file_size_bytes";
pub const MEDIA_USER_QUOTA_BYTES_KEY: &str = "media.user_quota_bytes";
pub const MEDIA_CACHE_POLICY_DEFAULT_KEY: &str = "media.cache_policy_default";
pub const DEFAULT_MAX_FILE_SIZE_BYTES: i64 = 52_428_800;
pub const DEFAULT_USER_QUOTA_BYTES: i64 = 1_073_741_824;
