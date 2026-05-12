use async_trait::async_trait;

/// Async operations on the `site_config` key-value table.
#[async_trait]
pub trait SiteConfigStorage: Send + Sync {
    /// Returns the value for `key`, or `None` if the key is not set.
    async fn get(&self, key: &str) -> sqlx::Result<Option<String>>;

    /// Inserts or replaces the value for `key`.
    async fn set(&self, key: &str, value: &str) -> sqlx::Result<()>;
}

pub const BACKUP_DESTINATION_PATH_KEY: &str = "backup.destination_path";
pub const BACKUP_SCHEDULE_KEY: &str = "backup.schedule";
pub const BACKUP_RETENTION_COUNT_KEY: &str = "backup.retention_count";
pub const BACKUP_MODE_KEY: &str = "backup.mode";
