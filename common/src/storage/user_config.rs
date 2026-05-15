//! Per-user preference storage.

use async_trait::async_trait;

/// Async operations on the `user_config` key-value table.
///
/// This trait manages individual user preferences and settings, which are
/// separate from site-wide configuration.
#[async_trait]
pub trait UserConfigStorage: Send + Sync {
    /// Returns a user's configuration value for a specific key.
    async fn get(&self, user_id: i64, key: &str) -> sqlx::Result<Option<String>>;

    /// Sets or updates a user's configuration value.
    async fn set(&self, user_id: i64, key: &str, value: &str) -> sqlx::Result<()>;

    /// Deletes a specific configuration key for a user.
    async fn delete(&self, user_id: i64, key: &str) -> sqlx::Result<()>;
}

/// Key for a user's media cache policy (e.g., whether to cache remote content).
pub const USER_MEDIA_CACHE_POLICY_KEY: &str = "media.cache_policy";
