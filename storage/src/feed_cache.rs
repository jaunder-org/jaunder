//! Cached, fully-rendered feed bodies keyed by their canonical (decoded) path
//! form. The cache layer is the single source of truth for what bytes get
//! served by `GET /feed.{rss,atom,json}` and the other feed endpoints.

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use thiserror::Error;

/// A single cached feed body.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FeedCacheRow {
    pub feed_url: String,
    pub body: String,
    pub etag: String,
    pub content_type: String,
    pub updated_at: DateTime<Utc>,
    pub generated_at: DateTime<Utc>,
}

#[derive(Debug, Error)]
pub enum FeedCacheError {
    #[error("database error: {0}")]
    Db(#[from] sqlx::Error),
}

#[async_trait]
pub trait FeedCacheStorage: Send + Sync {
    async fn get(&self, feed_url: &str) -> Result<Option<FeedCacheRow>, FeedCacheError>;
    async fn upsert(&self, row: FeedCacheRow) -> Result<(), FeedCacheError>;
    async fn delete(&self, feed_url: &str) -> Result<(), FeedCacheError>;
}
