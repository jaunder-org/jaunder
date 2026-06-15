use sqlx::Postgres;

use crate::feed_cache::FeedCacheStore;

/// Postgres-backed feed-cache storage.
pub type PostgresFeedCacheStorage = FeedCacheStore<Postgres>;
