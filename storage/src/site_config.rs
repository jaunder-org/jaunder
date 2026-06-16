//! Site-wide configuration storage.

use crate::backend::Backend;
use async_trait::async_trait;
use common::backup::{BackupConfig, BackupMode, BackupSchedule, DEFAULT_BACKUP_RETENTION_COUNT};
use common::site::{SiteIdentity, DEFAULT_SITE_TITLE};
use sqlx::{Database, Pool};

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
        let destination_path = self
            .get(BACKUP_DESTINATION_PATH_KEY)
            .await?
            .and_then(common::text::non_empty_owned);
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

    /// Returns the configured `feeds.min_items` value, falling back to the
    /// default ([`DEFAULT_FEEDS_MIN_ITEMS`]) if unset or unparseable.
    async fn get_feeds_min_items(&self) -> sqlx::Result<u32> {
        Ok(self
            .get(FEEDS_MIN_ITEMS_KEY)
            .await?
            .as_deref()
            .and_then(|v| v.trim().parse::<u32>().ok())
            .unwrap_or(DEFAULT_FEEDS_MIN_ITEMS))
    }

    /// Returns the configured `feeds.min_days` value, falling back to the
    /// default ([`DEFAULT_FEEDS_MIN_DAYS`]) if unset or unparseable.
    async fn get_feeds_min_days(&self) -> sqlx::Result<u32> {
        Ok(self
            .get(FEEDS_MIN_DAYS_KEY)
            .await?
            .as_deref()
            .and_then(|v| v.trim().parse::<u32>().ok())
            .unwrap_or(DEFAULT_FEEDS_MIN_DAYS))
    }

    /// Returns the configured `WebSub` hub URL, if any. An empty stored value
    /// is treated as unset.
    async fn get_feeds_websub_hub_url(&self) -> sqlx::Result<Option<String>> {
        Ok(self
            .get(FEEDS_WEBSUB_HUB_URL_KEY)
            .await?
            .and_then(common::text::non_empty_owned))
    }

    /// Returns the site identity (title and base URL).
    async fn get_identity(&self) -> sqlx::Result<SiteIdentity> {
        let title = self
            .get(SITE_TITLE_KEY)
            .await?
            .and_then(common::text::non_empty_owned)
            .unwrap_or_else(|| DEFAULT_SITE_TITLE.to_owned());
        let base_url = self.get(SITE_BASE_URL_KEY).await?.and_then(|v| {
            let trimmed = v.trim().trim_end_matches('/').to_owned();
            if trimmed.is_empty() {
                None
            } else {
                Some(trimmed)
            }
        });
        Ok(SiteIdentity { title, base_url })
    }

    /// Stores the site identity (title and base URL).
    /// For `base_url`, an empty string is stored when `None` is provided.
    /// Trailing slashes on the base URL are stripped on write.
    async fn set_identity(&self, config: &SiteIdentity) -> sqlx::Result<()> {
        self.set(SITE_TITLE_KEY, &config.title).await?;
        let base_url_value = config
            .base_url
            .as_deref()
            .map_or("", |v| v.trim_end_matches('/'));
        self.set(SITE_BASE_URL_KEY, base_url_value).await?;
        Ok(())
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

/// Key for the minimum number of items to include in any feed, regardless of age.
pub const FEEDS_MIN_ITEMS_KEY: &str = "feeds.min_items";
/// Key for the minimum age window (in days) of items to include in any feed,
/// regardless of count.
pub const FEEDS_MIN_DAYS_KEY: &str = "feeds.min_days";
/// Key for the absolute URL of the configured `WebSub` hub. Unset (or empty)
/// disables `WebSub` pings.
pub const FEEDS_WEBSUB_HUB_URL_KEY: &str = "feeds.websub_hub_url";

/// Default for [`FEEDS_MIN_ITEMS_KEY`]: include at least 20 items in every feed.
pub const DEFAULT_FEEDS_MIN_ITEMS: u32 = 20;
/// Default for [`FEEDS_MIN_DAYS_KEY`]: include items from the last 30 days.
pub const DEFAULT_FEEDS_MIN_DAYS: u32 = 30;

/// Key for the human-facing site title used in feed metadata and similar.
pub const SITE_TITLE_KEY: &str = "site.title";
/// Key for the public-facing base URL of the site (scheme + host, no
/// trailing slash). Unset (or empty) means callers should emit
/// root-relative URLs.
pub const SITE_BASE_URL_KEY: &str = "site.base_url";

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

/// Generic [`SiteConfigStorage`] backed by any [`Backend`] database.
///
/// Zero backend divergence (shared `ON CONFLICT` upsert), so it is implemented
/// once here; see ADR-0019.
pub struct SiteConfigStore<DB: Database> {
    pool: Pool<DB>,
}

impl<DB: Database> SiteConfigStore<DB> {
    #[must_use]
    pub fn new(pool: Pool<DB>) -> Self {
        Self { pool }
    }
}

#[async_trait]
impl<DB> SiteConfigStorage for SiteConfigStore<DB>
where
    DB: Backend,
    (String,): for<'r> sqlx::FromRow<'r, DB::Row>,
    for<'q> &'q str: sqlx::Encode<'q, DB> + sqlx::Type<DB>,
    for<'c> &'c Pool<DB>: sqlx::Executor<'c, Database = DB>,
    for<'q> DB::Arguments<'q>: sqlx::IntoArguments<'q, DB>,
{
    async fn get(&self, key: &str) -> sqlx::Result<Option<String>> {
        let row = sqlx::query_as::<_, (String,)>("SELECT value FROM site_config WHERE key = $1")
            .bind(key)
            .fetch_optional(&self.pool)
            .await?;
        Ok(row.map(|(value,)| value))
    }

    async fn set(&self, key: &str, value: &str) -> sqlx::Result<()> {
        sqlx::query(
            "INSERT INTO site_config (key, value) VALUES ($1, $2)
             ON CONFLICT (key) DO UPDATE SET value = excluded.value",
        )
        .bind(key)
        .bind(value)
        .execute(&self.pool)
        .await?;
        Ok(())
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
    async fn feeds_min_items_returns_default_when_unset() {
        let storage = SqliteSiteConfigStorage::new(test_pool().await);
        assert_eq!(
            storage.get_feeds_min_items().await.unwrap(),
            super::DEFAULT_FEEDS_MIN_ITEMS
        );
    }

    #[tokio::test]
    async fn feeds_min_items_returns_override_value() {
        let storage = SqliteSiteConfigStorage::new(test_pool().await);
        storage.set(super::FEEDS_MIN_ITEMS_KEY, "50").await.unwrap();
        assert_eq!(storage.get_feeds_min_items().await.unwrap(), 50);
    }

    #[tokio::test]
    async fn feeds_min_items_falls_back_when_invalid() {
        let storage = SqliteSiteConfigStorage::new(test_pool().await);
        storage
            .set(super::FEEDS_MIN_ITEMS_KEY, "not a number")
            .await
            .unwrap();
        assert_eq!(
            storage.get_feeds_min_items().await.unwrap(),
            super::DEFAULT_FEEDS_MIN_ITEMS
        );
    }

    #[tokio::test]
    async fn feeds_min_days_returns_default_when_unset() {
        let storage = SqliteSiteConfigStorage::new(test_pool().await);
        assert_eq!(
            storage.get_feeds_min_days().await.unwrap(),
            super::DEFAULT_FEEDS_MIN_DAYS
        );
    }

    #[tokio::test]
    async fn feeds_min_days_returns_override_value() {
        let storage = SqliteSiteConfigStorage::new(test_pool().await);
        storage.set(super::FEEDS_MIN_DAYS_KEY, "60").await.unwrap();
        assert_eq!(storage.get_feeds_min_days().await.unwrap(), 60);
    }

    #[tokio::test]
    async fn feeds_websub_hub_url_returns_none_when_unset() {
        let storage = SqliteSiteConfigStorage::new(test_pool().await);
        assert!(storage.get_feeds_websub_hub_url().await.unwrap().is_none());
    }

    #[tokio::test]
    async fn feeds_websub_hub_url_returns_some_when_set() {
        let storage = SqliteSiteConfigStorage::new(test_pool().await);
        storage
            .set(super::FEEDS_WEBSUB_HUB_URL_KEY, "https://hub.example.com/")
            .await
            .unwrap();
        assert_eq!(
            storage.get_feeds_websub_hub_url().await.unwrap().as_deref(),
            Some("https://hub.example.com/")
        );
    }

    #[tokio::test]
    async fn feeds_websub_hub_url_treats_empty_as_none() {
        let storage = SqliteSiteConfigStorage::new(test_pool().await);
        storage
            .set(super::FEEDS_WEBSUB_HUB_URL_KEY, "")
            .await
            .unwrap();
        assert!(storage.get_feeds_websub_hub_url().await.unwrap().is_none());
    }

    #[tokio::test]
    async fn identity_returns_defaults_when_unset() {
        let storage = SqliteSiteConfigStorage::new(test_pool().await);
        let identity = storage.get_identity().await.expect("get_identity");
        assert_eq!(identity.title, common::site::DEFAULT_SITE_TITLE);
        assert_eq!(identity.base_url, None);
    }

    #[tokio::test]
    async fn identity_returns_override_when_title_set() {
        let storage = SqliteSiteConfigStorage::new(test_pool().await);
        storage.set(super::SITE_TITLE_KEY, "My Blog").await.unwrap();
        let identity = storage.get_identity().await.expect("get_identity");
        assert_eq!(identity.title, "My Blog");
        assert_eq!(identity.base_url, None);
    }

    #[tokio::test]
    async fn identity_returns_some_base_url_when_set_with_trailing_slash_stripped() {
        let storage = SqliteSiteConfigStorage::new(test_pool().await);
        storage
            .set(super::SITE_BASE_URL_KEY, "https://example.com/")
            .await
            .unwrap();
        let identity = storage.get_identity().await.expect("get_identity");
        assert_eq!(identity.title, common::site::DEFAULT_SITE_TITLE);
        assert_eq!(identity.base_url.as_deref(), Some("https://example.com"));
    }

    #[tokio::test]
    async fn identity_treats_empty_title_as_unset() {
        let storage = SqliteSiteConfigStorage::new(test_pool().await);
        storage.set(super::SITE_TITLE_KEY, "   ").await.unwrap();
        let identity = storage.get_identity().await.expect("get_identity");
        assert_eq!(identity.title, common::site::DEFAULT_SITE_TITLE);
    }

    #[tokio::test]
    async fn identity_treats_empty_base_url_as_none() {
        let storage = SqliteSiteConfigStorage::new(test_pool().await);
        storage.set(super::SITE_BASE_URL_KEY, "").await.unwrap();
        let identity = storage.get_identity().await.expect("get_identity");
        assert_eq!(identity.base_url, None);
    }

    #[tokio::test]
    async fn set_identity_round_trips_via_get_identity() {
        let storage = SqliteSiteConfigStorage::new(test_pool().await);
        let original = common::site::SiteIdentity {
            title: "Test Site".to_string(),
            base_url: Some("https://test.example.com".to_string()),
        };
        storage.set_identity(&original).await.expect("set_identity");
        let retrieved = storage.get_identity().await.expect("get_identity");
        assert_eq!(retrieved, original);
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
