//! Site-wide configuration storage.

use crate::backend::Backend;
use async_trait::async_trait;
use common::backup::{BackupConfig, BackupMode, BackupSchedule, RetentionCount};
use common::feed::FeedsConfig;
use common::site::{SiteIdentity, DEFAULT_SITE_TITLE};
use common::visibility::AudienceTarget;
use sqlx::{Database, Pool};

/// Async operations on the `site_config` key-value table.
///
/// This trait manages instance-wide settings that are not specific to any
/// individual user.
#[cfg_attr(feature = "test-utils", mockall::automock)]
#[async_trait]
pub trait SiteConfigStorage: Send + Sync {
    /// Returns the value for a specific configuration key.
    async fn get(&self, key: &str) -> sqlx::Result<Option<String>>;

    /// Sets or updates the value for a configuration key.
    async fn set(&self, key: &str, value: &str) -> sqlx::Result<()>;

    /// Enumerates every `site_config` entry as `(key, value)`, ordered by key.
    ///
    /// A third primitive alongside [`get`](Self::get)/[`set`](Self::set) (no
    /// default: a `vec![]` default would silently under-report for any
    /// implementor). Backs `jaunder site-config list`.
    async fn list(&self) -> sqlx::Result<Vec<(String, String)>>;

    /// Deletes a `site_config` entry, returning whether a row was removed.
    ///
    /// Idempotent: deleting an absent key is a no-op that returns `false`. Backs
    /// `jaunder site-config unset`.
    async fn delete(&self, key: &str) -> sqlx::Result<bool>;

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
            .and_then(|s| s.parse::<BackupSchedule>().ok())
            .unwrap_or_default();
        let retention_count = self
            .get(BACKUP_RETENTION_COUNT_KEY)
            .await?
            .as_deref()
            .and_then(|v| v.parse::<RetentionCount>().ok())
            .unwrap_or_default();
        let mode = self
            .get(BACKUP_MODE_KEY)
            .await?
            .as_deref()
            .and_then(|v| v.trim().parse::<BackupMode>().ok())
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

    /// Returns the feed-generation configuration as a single group, applying
    /// the same per-field defaults as the granular getters it delegates to.
    /// The granular getters remain for single-value callers (e.g. the worker's
    /// hub-URL read).
    async fn get_feeds_config(&self) -> sqlx::Result<FeedsConfig> {
        Ok(FeedsConfig {
            min_items: self.get_feeds_min_items().await?,
            min_days: self.get_feeds_min_days().await?,
            websub_hub_url: self.get_feeds_websub_hub_url().await?,
        })
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
        self.set(BACKUP_SCHEDULE_KEY, &config.schedule).await?;
        self.set(
            BACKUP_RETENTION_COUNT_KEY,
            &config.retention_count.to_string(),
        )
        .await?;
        self.set(BACKUP_MODE_KEY, config.mode.as_ref()).await?;
        Ok(())
    }

    /// Returns the configured site-wide default post audience, falling back to
    /// [`AudienceTarget::Public`] when unset or unparseable. Only the built-in
    /// audiences (`public`/`subscribers`/`private`) are valid site-wide
    /// defaults; a `Named` audience is per-author and never returned here.
    async fn get_default_audience(&self) -> sqlx::Result<AudienceTarget> {
        Ok(self
            .get(POSTS_DEFAULT_AUDIENCE_KEY)
            .await?
            .as_deref()
            .and_then(parse_default_audience)
            .unwrap_or(AudienceTarget::Public))
    }

    /// Stores the site-wide default post audience as its string form. A `Named`
    /// audience has no site-wide string form and is stored as `public`.
    async fn set_default_audience(&self, audience: &AudienceTarget) -> sqlx::Result<()> {
        self.set(POSTS_DEFAULT_AUDIENCE_KEY, default_audience_str(audience))
            .await
    }

    /// Stores the feed-generation configuration. An absent `websub_hub_url` is
    /// stored as the empty string (treated as unset on read).
    async fn set_feeds_config(&self, config: &FeedsConfig) -> sqlx::Result<()> {
        self.set(FEEDS_MIN_ITEMS_KEY, &config.min_items.to_string())
            .await?;
        self.set(FEEDS_MIN_DAYS_KEY, &config.min_days.to_string())
            .await?;
        self.set(
            FEEDS_WEBSUB_HUB_URL_KEY,
            config.websub_hub_url.as_deref().unwrap_or(""),
        )
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

/// Key for the site-wide default post audience. Valid stored values are the
/// built-in audiences `public`/`subscribers`/`private`; anything else (or
/// unset) is read back as [`AudienceTarget::Public`].
pub const POSTS_DEFAULT_AUDIENCE_KEY: &str = "posts.default_audience";

/// Key for the human-facing site title used in feed metadata and similar.
pub const SITE_TITLE_KEY: &str = "site.title";
/// Key for the public-facing base URL of the site (scheme + host, no
/// trailing slash). Unset (or empty) means callers should emit
/// root-relative URLs.
pub const SITE_BASE_URL_KEY: &str = "site.base_url";

/// Parses a stored site-wide default audience. Only the built-ins are valid:
/// `Named` is per-author and has no instance-wide form, so it is rejected here
/// (the caller falls back to `Public`).
fn parse_default_audience(value: &str) -> Option<AudienceTarget> {
    match value.trim() {
        "public" => Some(AudienceTarget::Public),
        "subscribers" => Some(AudienceTarget::Subscribers),
        "private" => Some(AudienceTarget::Private),
        _ => None,
    }
}

/// String form for a site-wide default audience. `Named` has no instance-wide
/// form, so it collapses to `public`.
fn default_audience_str(audience: &AudienceTarget) -> &'static str {
    match audience {
        AudienceTarget::Public | AudienceTarget::Named(_) => "public",
        AudienceTarget::Subscribers => "subscribers",
        AudienceTarget::Private => "private",
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
    (String, String): for<'r> sqlx::FromRow<'r, DB::Row>,
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

    async fn list(&self) -> sqlx::Result<Vec<(String, String)>> {
        let rows = sqlx::query_as::<_, (String, String)>(
            "SELECT key, value FROM site_config ORDER BY key",
        )
        .fetch_all(&self.pool)
        .await?;
        Ok(rows)
    }

    async fn delete(&self, key: &str) -> sqlx::Result<bool> {
        // `RETURNING` + `fetch_optional` detects a no-match generically (a `None`),
        // avoiding `rows_affected()` which sqlx exposes only on concrete results
        // (mirrors `audiences::rename_audience`). Both backends support RETURNING.
        let removed =
            sqlx::query_as::<_, (String,)>("DELETE FROM site_config WHERE key = $1 RETURNING key")
                .bind(key)
                .fetch_optional(&self.pool)
                .await?;
        Ok(removed.is_some())
    }
}

#[cfg(test)]
mod tests {
    use crate::test_support::{backends, Backend};
    use common::backup::{BackupConfig, BackupMode, RetentionCount};
    use common::feed::FeedsConfig;
    use common::test_support::parse_retention_count;
    use rstest::*;
    use rstest_reuse::*;

    // guard:no-backend — exercises the in-memory InMemorySiteConfig fixture; no live database backend
    #[tokio::test]
    async fn in_memory_site_config_round_trips() {
        use crate::test_support::InMemorySiteConfig;
        use crate::SiteConfigStorage;
        let store =
            InMemorySiteConfig::from_pairs([("site.title", "T"), ("backup.mode", "archive")]);
        assert_eq!(
            store.get("site.title").await.unwrap(),
            Some("T".to_string())
        );
        store.set("feeds.min_items", "9").await.unwrap();
        assert_eq!(
            store.list().await.unwrap(),
            vec![
                ("backup.mode".to_string(), "archive".to_string()),
                ("feeds.min_items".to_string(), "9".to_string()),
                ("site.title".to_string(), "T".to_string()),
            ],
        );
        assert!(store.delete("site.title").await.unwrap());
        assert!(!store.delete("site.title").await.unwrap());
        assert_eq!(store.get("site.title").await.unwrap(), None);
    }

    #[apply(backends)]
    #[tokio::test]
    async fn get_backup_config_returns_defaults_when_unconfigured(#[case] backend: Backend) {
        let env = backend.setup().await;
        let storage = &*env.state.site_config;
        let config = storage.get_backup_config().await.unwrap();
        assert_eq!(config, BackupConfig::default());
    }

    #[apply(backends)]
    #[tokio::test]
    async fn set_and_get_backup_config_round_trips(#[case] backend: Backend) {
        let env = backend.setup().await;
        let storage = &*env.state.site_config;
        let config = BackupConfig {
            destination_path: Some("/srv/backups".to_owned()),
            schedule: "0 30 2 * * *".parse().unwrap(),
            retention_count: parse_retention_count("14"),
            mode: BackupMode::Archive,
        };
        storage.set_backup_config(&config).await.unwrap();
        assert_eq!(storage.get_backup_config().await.unwrap(), config);
    }

    #[apply(backends)]
    #[tokio::test]
    async fn get_feeds_config_returns_defaults_when_unconfigured(#[case] backend: Backend) {
        let env = backend.setup().await;
        let storage = &*env.state.site_config;
        let config = storage.get_feeds_config().await.unwrap();
        assert_eq!(config.min_items, super::DEFAULT_FEEDS_MIN_ITEMS);
        assert_eq!(config.min_days, super::DEFAULT_FEEDS_MIN_DAYS);
        assert_eq!(config.websub_hub_url, None);
    }

    #[apply(backends)]
    #[tokio::test]
    async fn set_and_get_feeds_config_round_trips(#[case] backend: Backend) {
        let env = backend.setup().await;
        let storage = &*env.state.site_config;
        let config = FeedsConfig {
            min_items: 42,
            min_days: 7,
            websub_hub_url: Some("https://hub.example.com".to_owned()),
        };
        storage.set_feeds_config(&config).await.unwrap();
        let loaded = storage.get_feeds_config().await.unwrap();
        assert_eq!(loaded, config);
        // Exercise the derived Clone/Debug so the aggregate struct is covered.
        assert_eq!(loaded.clone(), config);
        assert!(!format!("{config:?}").is_empty());
    }

    #[apply(backends)]
    #[tokio::test]
    async fn get_backup_config_ignores_invalid_stored_values(#[case] backend: Backend) {
        let env = backend.setup().await;
        let storage = &*env.state.site_config;
        storage.set("backup.schedule", "not a cron").await.unwrap();
        storage
            .set("backup.retention_count", "daily")
            .await
            .unwrap();
        storage.set("backup.mode", "floppy").await.unwrap();
        let config = storage.get_backup_config().await.unwrap();
        assert_eq!(config, BackupConfig::default());
    }

    #[apply(backends)]
    #[tokio::test]
    async fn get_backup_config_treats_zero_retention_as_default(#[case] backend: Backend) {
        // A stored `0` is not a valid RetentionCount (min 1), so it falls back to the default
        // (7) rather than being kept — pruning can never be configured to remove every backup.
        let env = backend.setup().await;
        let storage = &*env.state.site_config;
        storage.set("backup.retention_count", "0").await.unwrap();
        let config = storage.get_backup_config().await.unwrap();
        assert_eq!(config.retention_count, RetentionCount::default());
    }

    #[apply(backends)]
    #[tokio::test]
    async fn list_returns_all_entries_ordered_by_key(#[case] backend: Backend) {
        let env = backend.setup().await;
        let storage = &*env.state.site_config;
        // Insert out of key order to prove the ORDER BY, not insertion order.
        storage.set("site.title", "T").await.unwrap();
        storage
            .set("feeds.websub_hub_url", "https://h/")
            .await
            .unwrap();
        storage.set("backup.mode", "archive").await.unwrap();

        assert_eq!(
            storage.list().await.unwrap(),
            vec![
                ("backup.mode".to_string(), "archive".to_string()),
                ("feeds.websub_hub_url".to_string(), "https://h/".to_string()),
                ("site.title".to_string(), "T".to_string()),
            ],
            "list() enumerates every entry ordered by key, both backends",
        );
    }

    #[apply(backends)]
    #[tokio::test]
    async fn delete_removes_a_key_and_reports_whether_present(#[case] backend: Backend) {
        let env = backend.setup().await;
        let storage = &*env.state.site_config;
        storage.set("site.title", "T").await.unwrap();

        // Deleting a present key reports true and the row is gone.
        assert!(
            storage.delete("site.title").await.unwrap(),
            "deleting a present key reports true",
        );
        assert_eq!(
            storage.get("site.title").await.unwrap(),
            None,
            "the row is removed",
        );

        // Deleting an absent key is an idempotent no-op reporting false.
        assert!(
            !storage.delete("site.title").await.unwrap(),
            "deleting an absent key reports false (no-op)",
        );
    }

    #[apply(backends)]
    #[tokio::test]
    async fn feeds_min_items_returns_default_when_unset(#[case] backend: Backend) {
        let env = backend.setup().await;
        let storage = &*env.state.site_config;
        assert_eq!(
            storage.get_feeds_min_items().await.unwrap(),
            super::DEFAULT_FEEDS_MIN_ITEMS
        );
    }

    #[apply(backends)]
    #[tokio::test]
    async fn feeds_min_items_returns_override_value(#[case] backend: Backend) {
        let env = backend.setup().await;
        let storage = &*env.state.site_config;
        storage.set(super::FEEDS_MIN_ITEMS_KEY, "50").await.unwrap();
        assert_eq!(storage.get_feeds_min_items().await.unwrap(), 50);
    }

    #[apply(backends)]
    #[tokio::test]
    async fn feeds_min_items_falls_back_when_invalid(#[case] backend: Backend) {
        let env = backend.setup().await;
        let storage = &*env.state.site_config;
        storage
            .set(super::FEEDS_MIN_ITEMS_KEY, "not a number")
            .await
            .unwrap();
        assert_eq!(
            storage.get_feeds_min_items().await.unwrap(),
            super::DEFAULT_FEEDS_MIN_ITEMS
        );
    }

    #[apply(backends)]
    #[tokio::test]
    async fn feeds_min_days_returns_default_when_unset(#[case] backend: Backend) {
        let env = backend.setup().await;
        let storage = &*env.state.site_config;
        assert_eq!(
            storage.get_feeds_min_days().await.unwrap(),
            super::DEFAULT_FEEDS_MIN_DAYS
        );
    }

    #[apply(backends)]
    #[tokio::test]
    async fn feeds_min_days_returns_override_value(#[case] backend: Backend) {
        let env = backend.setup().await;
        let storage = &*env.state.site_config;
        storage.set(super::FEEDS_MIN_DAYS_KEY, "60").await.unwrap();
        assert_eq!(storage.get_feeds_min_days().await.unwrap(), 60);
    }

    #[apply(backends)]
    #[tokio::test]
    async fn feeds_websub_hub_url_returns_none_when_unset(#[case] backend: Backend) {
        let env = backend.setup().await;
        let storage = &*env.state.site_config;
        assert!(storage.get_feeds_websub_hub_url().await.unwrap().is_none());
    }

    #[apply(backends)]
    #[tokio::test]
    async fn feeds_websub_hub_url_returns_some_when_set(#[case] backend: Backend) {
        let env = backend.setup().await;
        let storage = &*env.state.site_config;
        storage
            .set(super::FEEDS_WEBSUB_HUB_URL_KEY, "https://hub.example.com/")
            .await
            .unwrap();
        assert_eq!(
            storage.get_feeds_websub_hub_url().await.unwrap().as_deref(),
            Some("https://hub.example.com/")
        );
    }

    #[apply(backends)]
    #[tokio::test]
    async fn feeds_websub_hub_url_treats_empty_as_none(#[case] backend: Backend) {
        let env = backend.setup().await;
        let storage = &*env.state.site_config;
        storage
            .set(super::FEEDS_WEBSUB_HUB_URL_KEY, "")
            .await
            .unwrap();
        assert!(storage.get_feeds_websub_hub_url().await.unwrap().is_none());
    }

    #[apply(backends)]
    #[tokio::test]
    async fn identity_returns_defaults_when_unset(#[case] backend: Backend) {
        let env = backend.setup().await;
        let storage = &*env.state.site_config;
        let identity = storage.get_identity().await.expect("get_identity");
        assert_eq!(identity.title, common::site::DEFAULT_SITE_TITLE);
        assert_eq!(identity.base_url, None);
    }

    #[apply(backends)]
    #[tokio::test]
    async fn identity_returns_override_when_title_set(#[case] backend: Backend) {
        let env = backend.setup().await;
        let storage = &*env.state.site_config;
        storage.set(super::SITE_TITLE_KEY, "My Blog").await.unwrap();
        let identity = storage.get_identity().await.expect("get_identity");
        assert_eq!(identity.title, "My Blog");
        assert_eq!(identity.base_url, None);
    }

    #[apply(backends)]
    #[tokio::test]
    async fn identity_returns_some_base_url_when_set_with_trailing_slash_stripped(
        #[case] backend: Backend,
    ) {
        let env = backend.setup().await;
        let storage = &*env.state.site_config;
        storage
            .set(super::SITE_BASE_URL_KEY, "https://example.com/")
            .await
            .unwrap();
        let identity = storage.get_identity().await.expect("get_identity");
        assert_eq!(identity.title, common::site::DEFAULT_SITE_TITLE);
        assert_eq!(identity.base_url.as_deref(), Some("https://example.com"));
    }

    #[apply(backends)]
    #[tokio::test]
    async fn identity_treats_empty_title_as_unset(#[case] backend: Backend) {
        let env = backend.setup().await;
        let storage = &*env.state.site_config;
        storage.set(super::SITE_TITLE_KEY, "   ").await.unwrap();
        let identity = storage.get_identity().await.expect("get_identity");
        assert_eq!(identity.title, common::site::DEFAULT_SITE_TITLE);
    }

    #[apply(backends)]
    #[tokio::test]
    async fn identity_treats_empty_base_url_as_none(#[case] backend: Backend) {
        let env = backend.setup().await;
        let storage = &*env.state.site_config;
        storage.set(super::SITE_BASE_URL_KEY, "").await.unwrap();
        let identity = storage.get_identity().await.expect("get_identity");
        assert_eq!(identity.base_url, None);
    }

    #[apply(backends)]
    #[tokio::test]
    async fn set_identity_round_trips_via_get_identity(#[case] backend: Backend) {
        let env = backend.setup().await;
        let storage = &*env.state.site_config;
        let original = common::site::SiteIdentity {
            title: "Test Site".to_string(),
            base_url: Some("https://test.example.com".to_string()),
        };
        storage.set_identity(&original).await.expect("set_identity");
        let retrieved = storage.get_identity().await.expect("get_identity");
        assert_eq!(retrieved, original);
    }

    #[apply(backends)]
    #[tokio::test]
    async fn get_backup_config_treats_empty_destination_as_none(#[case] backend: Backend) {
        let env = backend.setup().await;
        let storage = &*env.state.site_config;
        storage.set("backup.destination_path", "").await.unwrap();
        let config = storage.get_backup_config().await.unwrap();
        assert_eq!(config.destination_path, None);
    }

    #[apply(backends)]
    #[tokio::test]
    async fn default_audience_returns_public_when_unset(#[case] backend: Backend) {
        let env = backend.setup().await;
        let storage = &*env.state.site_config;
        assert_eq!(
            storage.get_default_audience().await.unwrap(),
            common::visibility::AudienceTarget::Public
        );
    }

    #[apply(backends)]
    #[tokio::test]
    async fn default_audience_returns_private_when_set(#[case] backend: Backend) {
        let env = backend.setup().await;
        let storage = &*env.state.site_config;
        storage
            .set_default_audience(&common::visibility::AudienceTarget::Private)
            .await
            .unwrap();
        assert_eq!(
            storage.get_default_audience().await.unwrap(),
            common::visibility::AudienceTarget::Private
        );
    }

    #[apply(backends)]
    #[tokio::test]
    async fn default_audience_returns_subscribers_when_set(#[case] backend: Backend) {
        let env = backend.setup().await;
        let storage = &*env.state.site_config;
        storage
            .set_default_audience(&common::visibility::AudienceTarget::Subscribers)
            .await
            .unwrap();
        assert_eq!(
            storage.get_default_audience().await.unwrap(),
            common::visibility::AudienceTarget::Subscribers
        );
    }

    #[apply(backends)]
    #[tokio::test]
    async fn set_default_audience_collapses_named_to_public(#[case] backend: Backend) {
        // A `Named` audience has no instance-wide form; the setter stores it as
        // `public` and the getter reads it back as `Public`.
        let env = backend.setup().await;
        let storage = &*env.state.site_config;
        storage
            .set_default_audience(&common::visibility::AudienceTarget::Named(
                common::ids::AudienceId::from(7),
            ))
            .await
            .unwrap();
        assert_eq!(
            storage
                .get(super::POSTS_DEFAULT_AUDIENCE_KEY)
                .await
                .unwrap(),
            Some("public".to_owned())
        );
        assert_eq!(
            storage.get_default_audience().await.unwrap(),
            common::visibility::AudienceTarget::Public
        );
    }

    #[apply(backends)]
    #[tokio::test]
    async fn default_audience_falls_back_to_public_when_garbage(#[case] backend: Backend) {
        let env = backend.setup().await;
        let storage = &*env.state.site_config;
        storage
            .set(super::POSTS_DEFAULT_AUDIENCE_KEY, "named")
            .await
            .unwrap();
        assert_eq!(
            storage.get_default_audience().await.unwrap(),
            common::visibility::AudienceTarget::Public
        );
        storage
            .set(super::POSTS_DEFAULT_AUDIENCE_KEY, "not a real value")
            .await
            .unwrap();
        assert_eq!(
            storage.get_default_audience().await.unwrap(),
            common::visibility::AudienceTarget::Public
        );
    }
}
