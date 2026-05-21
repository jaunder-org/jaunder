//! Site-wide configuration storage.

use async_trait::async_trait;

/// Async operations on the `site_config` key-value table.
///
/// This trait manages instance-wide settings that are not specific to any
/// individual user.
#[async_trait]
pub trait SiteConfigStorage: Send + Sync {
    /// Returns the value for a specific configuration key.
    async fn get(&self, key: &str) -> sqlx::Result<Option<String>>;

    /// Sets or updates the value for a configuration key.
    async fn set(&self, key: &str, value: &str) -> sqlx::Result<()>;

    /// Returns the integer value for a configuration key, or the default if not set/invalid.
    async fn get_int(&self, key: &str, default: i64) -> i64 {
        self.get(key)
            .await
            .ok()
            .flatten()
            .and_then(|v| v.parse::<i64>().ok())
            .unwrap_or(default)
    }
}

/// Key for the site configuration setting for backup destination path.
pub const BACKUP_DESTINATION_PATH_KEY: &str = "backup.destination_path";
/// Key for the site configuration setting for the backup schedule (cron).
pub const BACKUP_SCHEDULE_KEY: &str = "backup.schedule";
/// Key for the site configuration setting for the number of backups to retain.
pub const BACKUP_RETENTION_COUNT_KEY: &str = "backup.retention_count";
/// Key for the site configuration setting for the backup mode (Archive/Directory).
pub const BACKUP_MODE_KEY: &str = "backup.mode";
