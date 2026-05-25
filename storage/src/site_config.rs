//! Site-wide configuration storage.

use async_trait::async_trait;
use common::backup::{BackupConfig, BackupMode, BackupSchedule, DEFAULT_BACKUP_RETENTION_COUNT};

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

    /// Returns the backup configuration from stored values, using defaults for missing/invalid fields.
    async fn get_backup_config(&self) -> sqlx::Result<BackupConfig> {
        let destination_path = self.get(BACKUP_DESTINATION_PATH_KEY).await?.and_then(|v| {
            let v = v.trim().to_owned();
            if v.is_empty() {
                None
            } else {
                Some(v)
            }
        });
        let schedule = self
            .get(BACKUP_SCHEDULE_KEY)
            .await?
            .as_deref()
            .and_then(BackupSchedule::parse)
            .unwrap_or_default();
        let retention_count = self
            .get(BACKUP_RETENTION_COUNT_KEY)
            .await?
            .as_deref()
            .and_then(|v| v.trim().parse::<usize>().ok())
            .unwrap_or(DEFAULT_BACKUP_RETENTION_COUNT);
        let mode = self
            .get(BACKUP_MODE_KEY)
            .await?
            .as_deref()
            .and_then(parse_backup_mode)
            .unwrap_or_default();
        Ok(BackupConfig {
            destination_path,
            schedule,
            retention_count,
            mode,
        })
    }

    /// Stores the backup configuration to the site config storage.
    async fn set_backup_config(&self, config: &BackupConfig) -> sqlx::Result<()> {
        self.set(
            BACKUP_DESTINATION_PATH_KEY,
            config.destination_path.as_deref().unwrap_or(""),
        )
        .await?;
        self.set(BACKUP_SCHEDULE_KEY, config.schedule.as_str())
            .await?;
        self.set(
            BACKUP_RETENTION_COUNT_KEY,
            &config.retention_count.to_string(),
        )
        .await?;
        self.set(BACKUP_MODE_KEY, backup_mode_str(config.mode))
            .await?;
        Ok(())
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

fn parse_backup_mode(value: &str) -> Option<BackupMode> {
    match value.trim() {
        "directory" => Some(BackupMode::Directory),
        "archive" => Some(BackupMode::Archive),
        _ => None,
    }
}

fn backup_mode_str(mode: BackupMode) -> &'static str {
    match mode {
        BackupMode::Directory => "directory",
        BackupMode::Archive => "archive",
    }
}

#[cfg(test)]
mod tests {
    use super::SiteConfigStorage;
    use crate::sqlite::SqliteSiteConfigStorage;
    use common::backup::{BackupConfig, BackupMode, BackupSchedule};
    use sqlx::SqlitePool;

    async fn test_pool() -> SqlitePool {
        let pool = SqlitePool::connect("sqlite::memory:").await.unwrap();
        sqlx::migrate!("./migrations/sqlite")
            .run(&pool)
            .await
            .unwrap();
        pool
    }

    #[tokio::test]
    async fn get_backup_config_returns_defaults_when_unconfigured() {
        let pool = test_pool().await;
        let storage = SqliteSiteConfigStorage::new(pool);
        let config = storage.get_backup_config().await.unwrap();
        assert_eq!(config, BackupConfig::default());
    }

    #[tokio::test]
    async fn set_and_get_backup_config_round_trips() {
        let pool = test_pool().await;
        let storage = SqliteSiteConfigStorage::new(pool);
        let config = BackupConfig {
            destination_path: Some("/srv/backups".to_owned()),
            schedule: BackupSchedule::parse("0 30 2 * * *").unwrap(),
            retention_count: 14,
            mode: BackupMode::Archive,
        };
        storage.set_backup_config(&config).await.unwrap();
        assert_eq!(storage.get_backup_config().await.unwrap(), config);
    }

    #[tokio::test]
    async fn get_backup_config_ignores_invalid_stored_values() {
        let pool = test_pool().await;
        let storage = SqliteSiteConfigStorage::new(pool);
        storage.set("backup.schedule", "not a cron").await.unwrap();
        storage
            .set("backup.retention_count", "daily")
            .await
            .unwrap();
        storage.set("backup.mode", "floppy").await.unwrap();
        let config = storage.get_backup_config().await.unwrap();
        assert_eq!(config, BackupConfig::default());
    }

    #[tokio::test]
    async fn get_backup_config_treats_empty_destination_as_none() {
        let pool = test_pool().await;
        let storage = SqliteSiteConfigStorage::new(pool);
        storage.set("backup.destination_path", "").await.unwrap();
        let config = storage.get_backup_config().await.unwrap();
        assert_eq!(config.destination_path, None);
    }
}
