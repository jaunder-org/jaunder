//! Site-wide configuration storage.

use crate::backend::Backend;
use async_trait::async_trait;
use common::backup::{BackupConfig, BackupMode, BackupSchedule, DestinationPath, RetentionCount};
use host::smtp_config::SmtpConfig;
// The closed registry of site-config keys (#687) — re-exported so
// `storage::SiteConfigKey` resolves for the call sites that name one.
pub use common::config_key::SiteConfigKey;
use common::feed::{FeedMinDays, FeedMinItems, FeedsConfig};
use common::media::{MaxFileSize, UserQuota};
// Re-exported so `storage::RegistrationPolicy` keeps resolving for call sites, and
// used by `get_registration_policy` below (the typed config accessor, #607).
pub use common::registration::RegistrationPolicy;
use common::site::{SiteIdentity, SiteTitle};
use common::smtp_host::SmtpHost;
use common::smtp_password::SmtpPassword;
use common::smtp_port::SmtpPort;
use common::smtp_sender::SmtpSender;
use common::smtp_tls_mode::SmtpTlsMode;
use common::smtp_username::SmtpUsername;
use common::tagged_url::{BaseUrl, HubUrl};
use common::visibility::{AudienceTarget, default_audience_str, parse_default_audience};
use sqlx::{Database, Pool};

/// Async operations on the `site_config` key-value table.
///
/// This trait manages instance-wide settings that are not specific to any
/// individual user.
#[cfg_attr(any(test, feature = "test-utils"), mockall::automock)]
#[async_trait]
pub trait SiteConfigStorage: Send + Sync {
    /// Returns the value for a specific configuration key.
    async fn get(&self, key: SiteConfigKey) -> sqlx::Result<Option<String>>;

    /// Sets or updates the value for a configuration key.
    async fn set(&self, key: SiteConfigKey, value: &str) -> sqlx::Result<()>;

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
    async fn delete(&self, key: SiteConfigKey) -> sqlx::Result<bool>;

    /// Reads the whole SMTP block as one typed [`SmtpConfig`], or `None` when
    /// `smtp.host` is unset (which is how an instance says "no outbound mail").
    ///
    /// A **required** method rather than a `get`-based default, for two reasons. Every
    /// value decodes through its own validating sqlx bridge, so a garbage stored value is
    /// rejected as a `ColumnDecode` at the query boundary rather than re-parsed (badly) by
    /// each caller — and the gate's decode scanner does not read trait *default* bodies
    /// (#787), so a decode written there would be invisible to it rather than approved.
    ///
    /// The optional fields fall back to their types' own defaults
    /// ([`SmtpPort`] 587, [`SmtpTlsMode::StartTls`], [`SmtpSender`]
    /// `Jaunder <noreply@localhost>`).
    async fn get_smtp_config(&self) -> sqlx::Result<Option<SmtpConfig>>;

    /// Returns the configured media max upload size, falling back to the
    /// [`MaxFileSize`] default (50 MiB) if unset or unparseable (including a stored
    /// `0`/negative, which the positive invariant rejects).
    async fn get_media_max_file_size(&self) -> sqlx::Result<MaxFileSize> {
        Ok(self
            .get(SiteConfigKey::MediaMaxFileSizeBytes)
            .await?
            .as_deref()
            .and_then(|v| v.parse().ok())
            .unwrap_or_default())
    }

    /// Returns the configured per-user media quota, falling back to the
    /// [`UserQuota`] default (1 GiB) if unset or unparseable (including a stored
    /// `0`/negative, which the positive invariant rejects).
    async fn get_media_user_quota(&self) -> sqlx::Result<UserQuota> {
        Ok(self
            .get(SiteConfigKey::MediaUserQuotaBytes)
            .await?
            .as_deref()
            .and_then(|v| v.parse().ok())
            .unwrap_or_default())
    }

    /// Returns the backup configuration from stored values, using defaults for missing/invalid fields.
    async fn get_backup_config(&self) -> sqlx::Result<BackupConfig> {
        let destination_path = self
            .get(SiteConfigKey::BackupDestinationPath)
            .await?
            .as_deref()
            .and_then(|v| v.parse::<DestinationPath>().ok());
        let schedule = self
            .get(SiteConfigKey::BackupSchedule)
            .await?
            .as_deref()
            .and_then(|s| s.parse::<BackupSchedule>().ok())
            .unwrap_or_default();
        let retention_count = self
            .get(SiteConfigKey::BackupRetentionCount)
            .await?
            .as_deref()
            .and_then(|v| v.parse::<RetentionCount>().ok())
            .unwrap_or_default();
        let mode = self
            .get(SiteConfigKey::BackupMode)
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

    /// Returns the site's user-registration policy, falling back to
    /// [`RegistrationPolicy::Closed`] when the value is unset or unparseable — the
    /// safe default that prevents unintended open registration on a freshly
    /// initialised instance. Like [`get_backup_config`](Self::get_backup_config), a
    /// genuine DB read error propagates (only the absent/garbage value defaults).
    async fn get_registration_policy(&self) -> sqlx::Result<RegistrationPolicy> {
        Ok(self
            .get(SiteConfigKey::SiteRegistrationPolicy)
            .await?
            .as_deref()
            .and_then(|v| v.parse().ok())
            .unwrap_or(RegistrationPolicy::Closed))
    }

    /// Returns the configured `feeds.min_items` value, falling back to the
    /// [`FeedMinItems`] default (20) if unset or unparseable (including a stored `0`,
    /// which the min-1 invariant rejects).
    async fn get_feeds_min_items(&self) -> sqlx::Result<FeedMinItems> {
        Ok(self
            .get(SiteConfigKey::FeedsMinItems)
            .await?
            .as_deref()
            .and_then(|v| v.parse().ok())
            .unwrap_or_default())
    }

    /// Returns the configured `feeds.min_days` value, falling back to the
    /// [`FeedMinDays`] default (30) if unset or unparseable (including a stored `0`,
    /// which the min-1 invariant rejects).
    async fn get_feeds_min_days(&self) -> sqlx::Result<FeedMinDays> {
        Ok(self
            .get(SiteConfigKey::FeedsMinDays)
            .await?
            .as_deref()
            .and_then(|v| v.parse().ok())
            .unwrap_or_default())
    }

    /// Returns the configured `WebSub` hub URL, if any. An empty stored value is
    /// treated as unset; a non-empty value that no longer parses as an absolute
    /// `http(s)` URL (corruption, or legacy data pre-dating this validation) is
    /// **purged** and read as unset — mirroring the `feed_events` unparseable-`feed_url`
    /// purge, so a bad stored value never hard-fails the read.
    async fn get_feeds_websub_hub_url(&self) -> sqlx::Result<Option<HubUrl>> {
        let Some(raw) = self
            .get(SiteConfigKey::FeedsWebsubHubUrl)
            .await?
            .and_then(common::text::non_empty_owned)
        else {
            return Ok(None);
        };
        if let Ok(url) = raw.parse::<HubUrl>() {
            Ok(Some(url))
        } else {
            tracing::warn!("purging unparseable stored feeds.websub_hub_url");
            self.delete(SiteConfigKey::FeedsWebsubHubUrl).await?;
            Ok(None)
        }
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
            .get(SiteConfigKey::SiteTitle)
            .await?
            .and_then(|v| v.parse::<SiteTitle>().ok())
            .unwrap_or_default();
        let base_url = match self
            .get(SiteConfigKey::SiteBaseUrl)
            .await?
            .and_then(common::text::non_empty_owned)
        {
            None => None,
            Some(raw) => {
                if let Ok(url) = raw.parse::<BaseUrl>() {
                    Some(url)
                } else {
                    // Purge a corrupt/legacy unparseable value and read as unset (as
                    // `get_feeds_websub_hub_url` does), so a bad `base_url` never bricks
                    // feed regeneration or the settings page it would otherwise 500.
                    tracing::warn!("purging unparseable stored site.base_url");
                    self.delete(SiteConfigKey::SiteBaseUrl).await?;
                    None
                }
            }
        };
        Ok(SiteIdentity { title, base_url })
    }

    /// Stores the site identity (title and base URL).
    /// For `base_url`, an empty string is stored when `None` is provided; a set
    /// value is stored in its canonical form (the `BaseUrl` normalized it).
    async fn set_identity(&self, config: &SiteIdentity) -> sqlx::Result<()> {
        self.set(SiteConfigKey::SiteTitle, &config.title).await?;
        let base_url_value = config.base_url.as_deref().unwrap_or("");
        self.set(SiteConfigKey::SiteBaseUrl, base_url_value).await?;
        Ok(())
    }

    /// Stores the backup configuration to the site config storage.
    async fn set_backup_config(&self, config: &BackupConfig) -> sqlx::Result<()> {
        self.set(
            SiteConfigKey::BackupDestinationPath,
            config.destination_path.as_deref().unwrap_or(""),
        )
        .await?;
        self.set(SiteConfigKey::BackupSchedule, &config.schedule)
            .await?;
        self.set(
            SiteConfigKey::BackupRetentionCount,
            &config.retention_count.to_string(),
        )
        .await?;
        self.set(SiteConfigKey::BackupMode, config.mode.as_ref())
            .await?;
        Ok(())
    }

    /// Returns the configured site-wide default post audience, falling back to
    /// [`AudienceTarget::Public`] when unset or unparseable. Only the built-in
    /// audiences (`public`/`subscribers`/`private`) are valid site-wide
    /// defaults; a `Named` audience is per-author and never returned here.
    async fn get_default_audience(&self) -> sqlx::Result<AudienceTarget> {
        Ok(self
            .get(SiteConfigKey::PostsDefaultAudience)
            .await?
            .as_deref()
            .and_then(parse_default_audience)
            .unwrap_or(AudienceTarget::Public))
    }

    /// Stores the site-wide default post audience as its string form. A `Named`
    /// audience has no site-wide string form and is stored as `public`.
    async fn set_default_audience(&self, audience: &AudienceTarget) -> sqlx::Result<()> {
        self.set(
            SiteConfigKey::PostsDefaultAudience,
            default_audience_str(audience),
        )
        .await
    }

    /// Stores the feed-generation configuration. An absent `websub_hub_url` is
    /// stored as the empty string (treated as unset on read).
    async fn set_feeds_config(&self, config: &FeedsConfig) -> sqlx::Result<()> {
        self.set(SiteConfigKey::FeedsMinItems, &config.min_items.to_string())
            .await?;
        self.set(SiteConfigKey::FeedsMinDays, &config.min_days.to_string())
            .await?;
        self.set(
            SiteConfigKey::FeedsWebsubHubUrl,
            config.websub_hub_url.as_deref().unwrap_or(""),
        )
        .await?;
        Ok(())
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

/// The one-row read behind every typed site-config value.
///
/// Named once, then written out per value type in [`SiteConfigStorage::get_smtp_config`]:
/// neither a generic helper (`query_as::<_, (T,)>`) nor a macro can carry the decode
/// target in a form the `sqlx-newtype-decode` gate can resolve, and six repetitions the
/// gate reads are worth more than one abstraction it cannot.
const SELECT_VALUE_SQL: &str = "SELECT value FROM site_config WHERE key = $1";

type SiteConfigExportRow = (String, String);

/// Re-labels a decode failure with the **key** it came from.
///
/// sqlx names the column by its position (`0`), which for six single-column reads of the
/// same table says nothing. The key is what makes a corrupt row actionable, and it is what
/// [`crate::load_smtp_config`] reads back to tell a credential failure (whose value is
/// never echoed) from a plain value one.
fn label_decode_error(key: SiteConfigKey, error: sqlx::Error) -> sqlx::Error {
    let sqlx::Error::ColumnDecode { source, .. } = error else {
        // A non-decode failure (pool closed, connection lost) has no key to add and passes
        // through unchanged; reaching it needs fault injection.
        return error; // cov:ignore
    };
    sqlx::Error::ColumnDecode {
        index: key.as_ref().to_owned(),
        source,
    }
}

#[async_trait]
impl<DB> SiteConfigStorage for SiteConfigStore<DB>
where
    DB: Backend,
    (String,): for<'r> sqlx::FromRow<'r, DB::Row>,
    SiteConfigExportRow: for<'r> sqlx::FromRow<'r, DB::Row>,
    // The SMTP value types decode from the `value` column via their validating sqlx
    // bridges (#438, #687); these bounds make the bridges available on the generic
    // backend.
    (SmtpHost,): for<'r> sqlx::FromRow<'r, DB::Row>,
    (SmtpPort,): for<'r> sqlx::FromRow<'r, DB::Row>,
    (SmtpTlsMode,): for<'r> sqlx::FromRow<'r, DB::Row>,
    (SmtpSender,): for<'r> sqlx::FromRow<'r, DB::Row>,
    (SmtpUsername,): for<'r> sqlx::FromRow<'r, DB::Row>,
    (SmtpPassword,): for<'r> sqlx::FromRow<'r, DB::Row>,
    for<'q> &'q str: sqlx::Encode<'q, DB> + sqlx::Type<DB>,
    // `SiteConfigKey`'s sqlx bridge reports `String` as its type (the token is bound as
    // borrowed text), so binding a key directly needs `String: Type<DB>` in scope.
    String: sqlx::Type<DB>,
    for<'c> &'c Pool<DB>: sqlx::Executor<'c, Database = DB>,
    for<'q> DB::Arguments<'q>: sqlx::IntoArguments<'q, DB>,
{
    async fn get(&self, key: SiteConfigKey) -> sqlx::Result<Option<String>> {
        let row = sqlx::query_as::<_, (String,)>("SELECT value FROM site_config WHERE key = $1")
            .bind(key)
            .fetch_optional(&self.pool)
            .await?;
        Ok(row.map(|(value,)| value))
    }

    #[tracing::instrument(
        name = "storage.site_config.set",
        skip(self, value),
        fields(db.system = DB::DB_SYSTEM)
    )]
    async fn set(&self, key: SiteConfigKey, value: &str) -> sqlx::Result<()> {
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

    async fn get_smtp_config(&self) -> sqlx::Result<Option<SmtpConfig>> {
        // Six direct reads, each decoding the `value` column straight into its newtype via
        // that type's sqlx bridge: a garbage stored value fails `FromStr` and surfaces as a
        // `ColumnDecode` labelled with the key (see `read_value`), never as a silently
        // coerced default.
        let host = sqlx::query_as::<_, (SmtpHost,)>(SELECT_VALUE_SQL)
            .bind(SiteConfigKey::SmtpHost)
            .fetch_optional(&self.pool)
            .await
            .map_err(|e| label_decode_error(SiteConfigKey::SmtpHost, e))?
            .map(|(host,)| host);
        let Some(host) = host else {
            // No host is not a misconfiguration: it is how an instance says "no SMTP".
            return Ok(None);
        };

        let port = sqlx::query_as::<_, (SmtpPort,)>(SELECT_VALUE_SQL)
            .bind(SiteConfigKey::SmtpPort)
            .fetch_optional(&self.pool)
            .await
            .map_err(|e| label_decode_error(SiteConfigKey::SmtpPort, e))?
            .map_or_else(SmtpPort::default, |(port,)| port);

        let tls_mode = sqlx::query_as::<_, (SmtpTlsMode,)>(SELECT_VALUE_SQL)
            .bind(SiteConfigKey::SmtpTlsMode)
            .fetch_optional(&self.pool)
            .await
            .map_err(|e| label_decode_error(SiteConfigKey::SmtpTlsMode, e))?
            .map_or_else(SmtpTlsMode::default, |(tls_mode,)| tls_mode);

        let sender = sqlx::query_as::<_, (SmtpSender,)>(SELECT_VALUE_SQL)
            .bind(SiteConfigKey::SmtpSender)
            .fetch_optional(&self.pool)
            .await
            .map_err(|e| label_decode_error(SiteConfigKey::SmtpSender, e))?
            .map_or_else(SmtpSender::default, |(sender,)| sender);

        let username = sqlx::query_as::<_, (SmtpUsername,)>(SELECT_VALUE_SQL)
            .bind(SiteConfigKey::SmtpUsername)
            .fetch_optional(&self.pool)
            .await
            .map_err(|e| label_decode_error(SiteConfigKey::SmtpUsername, e))?
            .map(|(username,)| username);

        let password = sqlx::query_as::<_, (SmtpPassword,)>(SELECT_VALUE_SQL)
            .bind(SiteConfigKey::SmtpPassword)
            .fetch_optional(&self.pool)
            .await
            .map_err(|e| label_decode_error(SiteConfigKey::SmtpPassword, e))?
            .map(|(password,)| password);

        Ok(Some(SmtpConfig {
            host,
            port,
            tls_mode,
            username,
            password,
            sender,
        }))
    }

    async fn list(&self) -> sqlx::Result<Vec<(String, String)>> {
        let rows = sqlx::query_as::<_, SiteConfigExportRow>(
            "SELECT key, value FROM site_config ORDER BY key",
        )
        .fetch_all(&self.pool)
        .await?;
        Ok(rows)
    }

    async fn delete(&self, key: SiteConfigKey) -> sqlx::Result<bool> {
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
    use super::{SiteConfigKey, SmtpTlsMode};
    use crate::test_support::{Backend, backends};
    use common::backup::{BackupConfig, BackupMode, RetentionCount};
    use common::feed::{FeedMinDays, FeedMinItems, FeedsConfig};
    use common::media::{MaxFileSize, UserQuota};
    use common::registration::RegistrationPolicy;
    use common::tagged_url::HubUrl;
    use common::test_support::{
        parse_destination_path, parse_feed_min_days, parse_feed_min_items, parse_max_file_size,
        parse_retention_count, parse_site_title, parse_smtp_username, parse_url, parse_user_quota,
    };
    use rstest::*;
    use rstest_reuse::*;

    #[apply(backends)]
    #[tokio::test]
    async fn site_config_primitives_round_trip(#[case] backend: Backend) {
        let env = backend.setup().await;
        let store = &*env.state.site_config;
        store.set(SiteConfigKey::SiteTitle, "T").await.unwrap();
        store
            .set(SiteConfigKey::BackupMode, "archive")
            .await
            .unwrap();
        assert_eq!(
            store.get(SiteConfigKey::SiteTitle).await.unwrap(),
            Some("T".to_string())
        );
        store.set(SiteConfigKey::FeedsMinItems, "9").await.unwrap();
        assert_eq!(
            store.list().await.unwrap(),
            vec![
                ("backup.mode".to_string(), "archive".to_string()),
                ("feeds.min_items".to_string(), "9".to_string()),
                ("site.title".to_string(), "T".to_string()),
            ],
        );
        assert!(store.delete(SiteConfigKey::SiteTitle).await.unwrap());
        assert!(!store.delete(SiteConfigKey::SiteTitle).await.unwrap());
        assert_eq!(store.get(SiteConfigKey::SiteTitle).await.unwrap(), None);
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
            destination_path: Some(parse_destination_path("/srv/backups")),
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
        assert_eq!(config.min_items, FeedMinItems::default());
        assert_eq!(config.min_days, FeedMinDays::default());
        assert_eq!(config.websub_hub_url, None);
    }

    #[apply(backends)]
    #[tokio::test]
    async fn set_and_get_feeds_config_round_trips(#[case] backend: Backend) {
        let env = backend.setup().await;
        let storage = &*env.state.site_config;
        let config = FeedsConfig {
            min_items: parse_feed_min_items("42"),
            min_days: parse_feed_min_days("7"),
            websub_hub_url: Some(parse_url("https://hub.example.com/")),
        };
        storage.set_feeds_config(&config).await.unwrap();
        let loaded = storage.get_feeds_config().await.unwrap();
        assert_eq!(loaded, config);
        // Exercise the derived Clone/Debug so the aggregate struct is covered.
        assert_eq!(loaded.clone(), config);
        assert!(!format!("{config:?}").is_empty());
    }

    /// An unset `smtp.host` is how an instance says "no outbound mail" — not an error,
    /// and not a half-populated config.
    #[apply(backends)]
    #[tokio::test]
    async fn get_smtp_config_returns_none_when_host_unset(#[case] backend: Backend) {
        let env = backend.setup().await;
        let storage = &*env.state.site_config;
        assert!(storage.get_smtp_config().await.unwrap().is_none());
    }

    #[apply(backends)]
    #[tokio::test]
    async fn get_smtp_config_reads_every_value_typed(#[case] backend: Backend) {
        let env = backend.setup().await;
        let storage = &*env.state.site_config;
        for (key, value) in [
            (SiteConfigKey::SmtpHost, "mail.example.com"),
            (SiteConfigKey::SmtpPort, "2525"),
            (SiteConfigKey::SmtpTlsMode, "tls"),
            (SiteConfigKey::SmtpSender, "Jaunder <noreply@example.com>"),
            (SiteConfigKey::SmtpUsername, "user@example.com"),
            (SiteConfigKey::SmtpPassword, "s3cr3t"),
        ] {
            storage.set(key, value).await.unwrap();
        }

        let got = storage
            .get_smtp_config()
            .await
            .unwrap()
            .expect("host is set");
        assert_eq!(got.host.as_ref(), "mail.example.com");
        assert_eq!(got.port.value(), 2525);
        assert_eq!(got.tls_mode, SmtpTlsMode::Tls);
        assert_eq!(got.sender.as_ref(), "Jaunder <noreply@example.com>");
        assert_eq!(got.username, Some(parse_smtp_username("user@example.com")));
        // Exercise the aggregate's derived Clone/Debug (which redacts the secret) before
        // reading the password out of it.
        assert!(format!("{got:?}").contains("[redacted]"));
        assert_eq!(
            got.clone().password.expect("password present").as_ref(),
            "s3cr3t"
        );
    }

    /// A bad stored value fails at the query boundary rather than silently defaulting —
    /// and the error names the key and echoes the offending value, which is what
    /// `load_smtp_config`'s error tests read back.
    ///
    /// Only reachable to set up because `set` takes a raw `&str`: the CLI validator would
    /// refuse it. Deliberate — the read path stays defensive about rows the CLI did not
    /// write.
    #[apply(backends)]
    #[tokio::test]
    async fn get_smtp_config_rejects_a_bad_stored_port(#[case] backend: Backend) {
        let env = backend.setup().await;
        let storage = &*env.state.site_config;
        storage
            .set(SiteConfigKey::SmtpHost, "mail.example.com")
            .await
            .unwrap();
        storage
            .set(SiteConfigKey::SmtpPort, "not-a-port")
            .await
            .unwrap();
        let err = storage.get_smtp_config().await.unwrap_err();
        assert!(
            matches!(&err, sqlx::Error::ColumnDecode { index, .. } if index == "smtp.port"),
            "the decode error must name the offending key; got {err:?}"
        );
        assert!(
            err.to_string().contains("not-a-port"),
            "the decode error must echo the offending value; got {err}"
        );
    }

    /// A row that is *syntactically* a `u16` but violates the newtype's own rule must still
    /// be rejected. This is the regression lock on the bridge decoding through
    /// `SmtpPort::from_str` rather than parsing a bare `u16` and wrapping it: the wrapping
    /// form accepts `"0"` here and hands back a `SmtpPort(0)` that the type's constructor
    /// would refuse, so the invariant would hold everywhere except coming out of the
    /// database — the one direction that matters for a corrupt row.
    #[apply(backends)]
    #[tokio::test]
    async fn get_smtp_config_rejects_a_stored_port_the_newtype_forbids(#[case] backend: Backend) {
        let env = backend.setup().await;
        let storage = &*env.state.site_config;
        storage
            .set(SiteConfigKey::SmtpHost, "mail.example.com")
            .await
            .unwrap();
        storage.set(SiteConfigKey::SmtpPort, "0").await.unwrap();
        let err = storage.get_smtp_config().await.unwrap_err();
        assert!(
            matches!(&err, sqlx::Error::ColumnDecode { index, .. } if index == "smtp.port"),
            "port 0 must be refused at the query boundary; got {err:?}"
        );
    }

    /// An empty `smtp.host` row is a misconfiguration, not a way to say "unset" — unset is
    /// the absent row. So it is rejected rather than read as `None`.
    #[apply(backends)]
    #[tokio::test]
    async fn get_smtp_config_rejects_an_empty_stored_host(#[case] backend: Backend) {
        let env = backend.setup().await;
        let storage = &*env.state.site_config;
        storage.set(SiteConfigKey::SmtpHost, "").await.unwrap();
        let err = storage.get_smtp_config().await.unwrap_err();
        assert!(
            matches!(&err, sqlx::Error::ColumnDecode { index, .. } if index == "smtp.host"),
            "the decode error must name the offending key; got {err:?}"
        );
    }

    #[apply(backends)]
    #[tokio::test]
    async fn get_smtp_config_rejects_an_empty_credential(#[case] backend: Backend) {
        let env = backend.setup().await;
        let storage = &*env.state.site_config;
        storage
            .set(SiteConfigKey::SmtpHost, "mail.example.com")
            .await
            .unwrap();
        // An empty stored credential bypasses the non-empty invariant only via tampering;
        // the bridge decode rejects it. Symmetric for username and password — and neither
        // error echoes the value, unlike the sibling keys above.
        for key in [SiteConfigKey::SmtpUsername, SiteConfigKey::SmtpPassword] {
            let dotted = key.as_ref();
            storage.set(key, "").await.unwrap();
            let err = storage.get_smtp_config().await.unwrap_err();
            assert!(
                matches!(&err, sqlx::Error::ColumnDecode { index, .. } if index == dotted),
                "expected a column-decode error for {dotted}, got: {err:?}"
            );
            storage.delete(key).await.unwrap();
        }
    }

    #[apply(backends)]
    #[tokio::test]
    async fn get_backup_config_ignores_invalid_stored_values(#[case] backend: Backend) {
        let env = backend.setup().await;
        let storage = &*env.state.site_config;
        storage
            .set(SiteConfigKey::BackupSchedule, "not a cron")
            .await
            .unwrap();
        storage
            .set(SiteConfigKey::BackupRetentionCount, "daily")
            .await
            .unwrap();
        storage
            .set(SiteConfigKey::BackupMode, "floppy")
            .await
            .unwrap();
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
        storage
            .set(SiteConfigKey::BackupRetentionCount, "0")
            .await
            .unwrap();
        let config = storage.get_backup_config().await.unwrap();
        assert_eq!(config.retention_count, RetentionCount::default());
    }

    #[apply(backends)]
    #[tokio::test]
    async fn list_returns_all_entries_ordered_by_key(#[case] backend: Backend) {
        let env = backend.setup().await;
        let storage = &*env.state.site_config;
        // Insert out of key order to prove the ORDER BY, not insertion order.
        storage.set(SiteConfigKey::SiteTitle, "T").await.unwrap();
        storage
            .set(SiteConfigKey::FeedsWebsubHubUrl, "https://h/")
            .await
            .unwrap();
        storage
            .set(SiteConfigKey::BackupMode, "archive")
            .await
            .unwrap();

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
        storage.set(SiteConfigKey::SiteTitle, "T").await.unwrap();

        // Deleting a present key reports true and the row is gone.
        assert!(
            storage.delete(SiteConfigKey::SiteTitle).await.unwrap(),
            "deleting a present key reports true",
        );
        assert_eq!(
            storage.get(SiteConfigKey::SiteTitle).await.unwrap(),
            None,
            "the row is removed",
        );

        // Deleting an absent key is an idempotent no-op reporting false.
        assert!(
            !storage.delete(SiteConfigKey::SiteTitle).await.unwrap(),
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
            FeedMinItems::default()
        );
    }

    #[apply(backends)]
    #[tokio::test]
    async fn feeds_min_items_returns_override_value(#[case] backend: Backend) {
        let env = backend.setup().await;
        let storage = &*env.state.site_config;
        storage
            .set(SiteConfigKey::FeedsMinItems, "50")
            .await
            .unwrap();
        assert_eq!(
            storage.get_feeds_min_items().await.unwrap(),
            parse_feed_min_items("50")
        );
    }

    #[apply(backends)]
    #[tokio::test]
    async fn feeds_min_items_falls_back_when_invalid_or_zero(#[case] backend: Backend) {
        let env = backend.setup().await;
        let storage = &*env.state.site_config;
        storage
            .set(SiteConfigKey::FeedsMinItems, "not a number")
            .await
            .unwrap();
        assert_eq!(
            storage.get_feeds_min_items().await.unwrap(),
            FeedMinItems::default()
        );
        // A stored `0` is rejected by the min-1 invariant and also falls back.
        storage
            .set(SiteConfigKey::FeedsMinItems, "0")
            .await
            .unwrap();
        assert_eq!(
            storage.get_feeds_min_items().await.unwrap(),
            FeedMinItems::default()
        );
    }

    #[apply(backends)]
    #[tokio::test]
    async fn feeds_min_days_returns_default_when_unset(#[case] backend: Backend) {
        let env = backend.setup().await;
        let storage = &*env.state.site_config;
        assert_eq!(
            storage.get_feeds_min_days().await.unwrap(),
            FeedMinDays::default()
        );
    }

    #[apply(backends)]
    #[tokio::test]
    async fn feeds_min_days_returns_override_value(#[case] backend: Backend) {
        let env = backend.setup().await;
        let storage = &*env.state.site_config;
        storage
            .set(SiteConfigKey::FeedsMinDays, "60")
            .await
            .unwrap();
        assert_eq!(
            storage.get_feeds_min_days().await.unwrap(),
            parse_feed_min_days("60")
        );
    }

    #[apply(backends)]
    #[tokio::test]
    async fn media_max_file_size_defaults_overrides_and_rejects_zero(#[case] backend: Backend) {
        let env = backend.setup().await;
        let storage = &*env.state.site_config;
        assert_eq!(
            storage.get_media_max_file_size().await.unwrap(),
            MaxFileSize::default()
        );
        storage
            .set(SiteConfigKey::MediaMaxFileSizeBytes, "1024")
            .await
            .unwrap();
        assert_eq!(
            storage.get_media_max_file_size().await.unwrap(),
            parse_max_file_size("1024")
        );
        // A stored 0/negative is rejected by the positive invariant → falls back.
        storage
            .set(SiteConfigKey::MediaMaxFileSizeBytes, "0")
            .await
            .unwrap();
        assert_eq!(
            storage.get_media_max_file_size().await.unwrap(),
            MaxFileSize::default()
        );
    }

    #[apply(backends)]
    #[tokio::test]
    async fn media_user_quota_defaults_overrides_and_rejects_negative(#[case] backend: Backend) {
        let env = backend.setup().await;
        let storage = &*env.state.site_config;
        assert_eq!(
            storage.get_media_user_quota().await.unwrap(),
            UserQuota::default()
        );
        storage
            .set(SiteConfigKey::MediaUserQuotaBytes, "2048")
            .await
            .unwrap();
        assert_eq!(
            storage.get_media_user_quota().await.unwrap(),
            parse_user_quota("2048")
        );
        storage
            .set(SiteConfigKey::MediaUserQuotaBytes, "-5")
            .await
            .unwrap();
        assert_eq!(
            storage.get_media_user_quota().await.unwrap(),
            UserQuota::default()
        );
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
            .set(SiteConfigKey::FeedsWebsubHubUrl, "https://hub.example.com/")
            .await
            .unwrap();
        // Asserted against a typed `HubUrl`, not its bytes: the column carries the
        // *role*, so a getter retyped to another role would fail here (#875).
        let want: HubUrl = parse_url("https://hub.example.com/");
        assert_eq!(
            storage.get_feeds_websub_hub_url().await.unwrap(),
            Some(want)
        );
    }

    #[apply(backends)]
    #[tokio::test]
    async fn feeds_websub_hub_url_treats_empty_as_none(#[case] backend: Backend) {
        let env = backend.setup().await;
        let storage = &*env.state.site_config;
        storage
            .set(SiteConfigKey::FeedsWebsubHubUrl, "")
            .await
            .unwrap();
        assert!(storage.get_feeds_websub_hub_url().await.unwrap().is_none());
    }

    #[apply(backends)]
    #[tokio::test]
    async fn feeds_websub_hub_url_purges_unparseable_stored_value(#[case] backend: Backend) {
        // A non-empty stored value that no longer parses as an absolute http(s) URL is
        // purged and read as unset (self-heal, mirroring the feed_events feed_url purge).
        let env = backend.setup().await;
        let storage = &*env.state.site_config;
        storage
            .set(SiteConfigKey::FeedsWebsubHubUrl, "not-a-url")
            .await
            .unwrap();
        assert_eq!(storage.get_feeds_websub_hub_url().await.unwrap(), None);
        // The corrupt value was deleted, not merely ignored.
        assert_eq!(
            storage.get(SiteConfigKey::FeedsWebsubHubUrl).await.unwrap(),
            None
        );
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
        storage
            .set(SiteConfigKey::SiteTitle, "My Blog")
            .await
            .unwrap();
        let identity = storage.get_identity().await.expect("get_identity");
        assert_eq!(identity.title, "My Blog");
        assert_eq!(identity.base_url, None);
    }

    #[apply(backends)]
    #[tokio::test]
    async fn identity_normalizes_stored_base_url_to_canonical_form(#[case] backend: Backend) {
        let env = backend.setup().await;
        let storage = &*env.state.site_config;
        // A value stored WITHOUT a trailing slash (representable in the column)
        // still parses; the type normalizes it to the canonical slashed form.
        storage
            .set(SiteConfigKey::SiteBaseUrl, "https://example.com")
            .await
            .unwrap();
        let identity = storage.get_identity().await.expect("get_identity");
        assert_eq!(identity.title, common::site::DEFAULT_SITE_TITLE);
        assert_eq!(identity.base_url.as_deref(), Some("https://example.com/"));
    }

    #[apply(backends)]
    #[tokio::test]
    async fn identity_purges_unparseable_stored_base_url(#[case] backend: Backend) {
        // A corrupt/legacy unparseable base_url is purged and read as unset, so it never
        // bricks the identity read (which feeds and the settings page depend on).
        let env = backend.setup().await;
        let storage = &*env.state.site_config;
        storage
            .set(SiteConfigKey::SiteBaseUrl, "not-a-url")
            .await
            .unwrap();
        assert_eq!(storage.get_identity().await.unwrap().base_url, None);
        // The corrupt value was deleted, not merely ignored.
        assert_eq!(storage.get(SiteConfigKey::SiteBaseUrl).await.unwrap(), None);
    }

    #[apply(backends)]
    #[tokio::test]
    async fn identity_treats_empty_title_as_unset(#[case] backend: Backend) {
        let env = backend.setup().await;
        let storage = &*env.state.site_config;
        storage.set(SiteConfigKey::SiteTitle, "   ").await.unwrap();
        let identity = storage.get_identity().await.expect("get_identity");
        assert_eq!(identity.title, common::site::DEFAULT_SITE_TITLE);
    }

    #[apply(backends)]
    #[tokio::test]
    async fn identity_treats_empty_base_url_as_none(#[case] backend: Backend) {
        let env = backend.setup().await;
        let storage = &*env.state.site_config;
        storage.set(SiteConfigKey::SiteBaseUrl, "").await.unwrap();
        let identity = storage.get_identity().await.expect("get_identity");
        assert_eq!(identity.base_url, None);
    }

    #[apply(backends)]
    #[tokio::test]
    async fn set_identity_round_trips_via_get_identity(#[case] backend: Backend) {
        let env = backend.setup().await;
        let storage = &*env.state.site_config;
        let original = common::site::SiteIdentity {
            title: parse_site_title("Test Site"),
            base_url: Some(parse_url("https://test.example.com/")),
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
        storage
            .set(SiteConfigKey::BackupDestinationPath, "")
            .await
            .unwrap();
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
                .get(SiteConfigKey::PostsDefaultAudience)
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
            .set(SiteConfigKey::PostsDefaultAudience, "named")
            .await
            .unwrap();
        assert_eq!(
            storage.get_default_audience().await.unwrap(),
            common::visibility::AudienceTarget::Public
        );
        storage
            .set(SiteConfigKey::PostsDefaultAudience, "not a real value")
            .await
            .unwrap();
        assert_eq!(
            storage.get_default_audience().await.unwrap(),
            common::visibility::AudienceTarget::Public
        );
    }

    // --- get_registration_policy (typed config accessor, #607) ---
    // (type-behavior tests — FromStr / Display / serde — live with the type in
    // `common::registration`.)

    #[apply(backends)]
    #[tokio::test]
    async fn registration_policy_defaults_to_closed_when_absent(#[case] backend: Backend) {
        let env = backend.setup().await;
        let storage = &*env.state.site_config;
        assert_eq!(
            storage.get_registration_policy().await.unwrap(),
            RegistrationPolicy::Closed
        );
    }

    #[apply(backends)]
    #[tokio::test]
    async fn registration_policy_round_trips_each_token(#[case] backend: Backend) {
        let env = backend.setup().await;
        let storage = &*env.state.site_config;
        for (token, expected) in [
            ("open", RegistrationPolicy::Open),
            ("invite_only", RegistrationPolicy::InviteOnly),
            ("closed", RegistrationPolicy::Closed),
        ] {
            storage
                .set(SiteConfigKey::SiteRegistrationPolicy, token)
                .await
                .unwrap();
            assert_eq!(storage.get_registration_policy().await.unwrap(), expected);
        }
    }

    #[apply(backends)]
    #[tokio::test]
    async fn registration_policy_falls_back_to_closed_when_garbage(#[case] backend: Backend) {
        let env = backend.setup().await;
        let storage = &*env.state.site_config;
        storage
            .set(SiteConfigKey::SiteRegistrationPolicy, "garbage")
            .await
            .unwrap();
        assert_eq!(
            storage.get_registration_policy().await.unwrap(),
            RegistrationPolicy::Closed
        );
    }
}
