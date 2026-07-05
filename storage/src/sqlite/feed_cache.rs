use sqlx::Sqlite;

use crate::feed_cache::FeedCacheStore;

/// SQLite-backed feed-cache storage.
pub type SqliteFeedCacheStorage = FeedCacheStore<Sqlite>;
