use async_trait::async_trait;

#[async_trait]
pub trait UserConfigStorage: Send + Sync {
    /// Get a user configuration value by key.
    ///
    /// # Errors
    ///
    /// Returns `Err` on database failure.
    async fn get(&self, user_id: i64, key: &str) -> sqlx::Result<Option<String>>;

    /// Set a user configuration value.
    ///
    /// # Errors
    ///
    /// Returns `Err` on database failure.
    async fn set(&self, user_id: i64, key: &str, value: &str) -> sqlx::Result<()>;

    /// Delete a user configuration value.
    ///
    /// # Errors
    ///
    /// Returns `Err` on database failure.
    async fn delete(&self, user_id: i64, key: &str) -> sqlx::Result<()>;
}

pub const USER_MEDIA_CACHE_POLICY_KEY: &str = "media.cache_policy";
