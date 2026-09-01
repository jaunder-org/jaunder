//! Site-wide configuration storage.

use crate::backend::Backend;
use crate::sql::QueryStorageExt;
use async_trait::async_trait;
use common::backup::{BackupConfig, BackupMode, BackupSchedule, DestinationPath, RetentionCount};
use common::media::{MaxFileSize, UserQuota};
use common::text;
use host::config_key::SiteConfigKey;
use host::feed::{FeedMinDays, FeedMinItems, FeedsConfig};
use host::smtp_config::SmtpConfig;
// Re-exported so `storage::RegistrationPolicy` keeps resolving for call sites, and
// used by `get_registration_policy` below (the typed config accessor, #607).
use crate::WriteTransaction;
pub use common::registration::RegistrationPolicy;
use common::site::{SiteIdentity, SiteTitle};
use common::smtp_host::SmtpHost;
use common::smtp_password::SmtpPassword;
use common::smtp_port::SmtpPort;
use common::smtp_sender::SmtpSender;
use common::smtp_tls_mode::SmtpTlsMode;
use common::smtp_username::SmtpUsername;
use common::tagged_url::{BaseUrl, HubUrl};
use common::visibility::DefaultAudience;
use sqlx::{Database, Encode, Executor, Pool, Result, Type};

/// Async operations on the `site_config` key-value table.
///
/// This trait manages instance-wide settings that are not specific to any
/// individual user.
#[cfg_attr(any(test, feature = "test-utils"), mockall::automock)]
#[async_trait]
pub trait SiteConfigStorage: Send + Sync {
    /// Returns the raw stored text for a specific configuration key.
    async fn get_raw(&self, key: SiteConfigKey) -> Result<Option<String>>;

    /// Sets or updates the value for a configuration key within the caller-owned write scope.
    async fn set(
        &self,
        transaction: &mut WriteTransaction,
        key: SiteConfigKey,
        value: &str,
    ) -> Result<()>;

    /// Enumerates every `site_config` entry as `(key, value)`, ordered by key.
    ///
    /// A third primitive alongside [`get_raw`](Self::get_raw)/[`set`](Self::set) (no
    /// default: a `vec![]` default would silently under-report for any
    /// implementor). Backs `jaunder site-config list`.
    async fn list(&self) -> Result<Vec<(String, String)>>;
    /// Deletes a `site_config` entry within the caller-owned write scope, returning whether a row was removed.
    ///
    /// Idempotent: deleting an absent key is a no-op that returns `false`. Backs
    /// `jaunder site-config unset`.
    async fn delete(&self, transaction: &mut WriteTransaction, key: SiteConfigKey) -> Result<bool>;

    /// Reads the whole SMTP block as one typed [`SmtpConfig`], or `None` when
    /// `smtp.host` is unset (which is how an instance says "no outbound mail").
    ///
    /// A **required** method rather than a `get`-based default: every value decodes through
    /// its own validating sqlx bridge, so a garbage stored value is rejected as a
    /// `ColumnDecode` at the query boundary rather than re-parsed (badly) by each caller.
    ///
    /// The optional fields fall back to their types' own defaults
    /// ([`SmtpPort`] 587, [`SmtpTlsMode::StartTls`], [`SmtpSender`]
    /// `Jaunder <noreply@localhost>`).
    async fn get_smtp_config(&self) -> Result<Option<SmtpConfig>>;

    /// Returns the configured media max upload size, falling back to the
    /// [`MaxFileSize`] default (50 MiB) if unset or unparseable (including a stored
    /// `0`/negative, which the positive invariant rejects).
    async fn get_media_max_file_size(&self) -> Result<MaxFileSize> {
        Ok(self
            .get_raw(SiteConfigKey::MediaMaxFileSizeBytes)
            .await?
            .as_deref()
            .and_then(|v| v.parse().ok())
            .unwrap_or_default())
    }

    /// Returns the configured per-user media quota, falling back to the
    /// [`UserQuota`] default (1 GiB) if unset or unparseable (including a stored
    /// `0`/negative, which the positive invariant rejects).
    async fn get_media_user_quota(&self) -> Result<UserQuota> {
        Ok(self
            .get_raw(SiteConfigKey::MediaUserQuotaBytes)
            .await?
            .as_deref()
            .and_then(|v| v.parse().ok())
            .unwrap_or_default())
    }

    /// Returns the backup configuration from stored values, using defaults for missing/invalid fields.
    async fn get_backup_config(&self) -> Result<BackupConfig> {
        let destination_path = self
            .get_raw(SiteConfigKey::BackupDestinationPath)
            .await?
            .as_deref()
            .and_then(|v| v.parse::<DestinationPath>().ok());
        let schedule = self
            .get_raw(SiteConfigKey::BackupSchedule)
            .await?
            .as_deref()
            .and_then(|s| s.parse::<BackupSchedule>().ok())
            .unwrap_or_default();
        let retention_count = self
            .get_raw(SiteConfigKey::BackupRetentionCount)
            .await?
            .as_deref()
            .and_then(|v| v.parse::<RetentionCount>().ok())
            .unwrap_or_default();
        let mode = self
            .get_raw(SiteConfigKey::BackupMode)
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
    async fn get_registration_policy(&self) -> Result<RegistrationPolicy> {
        Ok(self
            .get_raw(SiteConfigKey::SiteRegistrationPolicy)
            .await?
            .as_deref()
            .and_then(|v| v.parse().ok())
            .unwrap_or(RegistrationPolicy::Closed))
    }

    /// Stores the site's user-registration policy.
    async fn set_registration_policy(
        &self,
        transaction: &mut WriteTransaction,
        policy: RegistrationPolicy,
    ) -> sqlx::Result<()> {
        self.set(
            transaction,
            SiteConfigKey::SiteRegistrationPolicy,
            policy.as_ref(),
        )
        .await
    }

    /// Stores the optional site base URL. `None` is represented by the existing
    /// empty-value convention used by [`set_identity`](Self::set_identity).
    async fn set_base_url(
        &self,
        transaction: &mut WriteTransaction,
        base_url: Option<BaseUrl>,
    ) -> sqlx::Result<()> {
        self.set(
            transaction,
            SiteConfigKey::SiteBaseUrl,
            base_url.as_ref().map_or("", AsRef::as_ref),
        )
        .await
    }

    /// Stores the two validated media limits together.
    async fn set_media_limits(
        &self,
        transaction: &mut WriteTransaction,
        max_file_size: MaxFileSize,
        user_quota: UserQuota,
    ) -> sqlx::Result<()> {
        self.set(
            transaction,
            SiteConfigKey::MediaMaxFileSizeBytes,
            &max_file_size.to_string(),
        )
        .await?;
        self.set(
            transaction,
            SiteConfigKey::MediaUserQuotaBytes,
            &user_quota.to_string(),
        )
        .await
    }

    /// Returns the configured `feeds.min_items` value, falling back to the
    /// [`FeedMinItems`] default (20) if unset or unparseable (including a stored `0`,
    /// which the min-1 invariant rejects).
    async fn get_feeds_min_items(&self) -> Result<FeedMinItems> {
        Ok(self
            .get_raw(SiteConfigKey::FeedsMinItems)
            .await?
            .as_deref()
            .and_then(|v| v.parse().ok())
            .unwrap_or_default())
    }

    /// Returns the configured `feeds.min_days` value, falling back to the
    /// [`FeedMinDays`] default (30) if unset or unparseable (including a stored `0`,
    /// which the min-1 invariant rejects).
    async fn get_feeds_min_days(&self) -> Result<FeedMinDays> {
        Ok(self
            .get_raw(SiteConfigKey::FeedsMinDays)
            .await?
            .as_deref()
            .and_then(|v| v.parse().ok())
            .unwrap_or_default())
    }

    /// Returns the configured `WebSub` hub URL, if any. An empty stored value is
    /// treated as unset; a non-empty value that no longer parses as an absolute
    /// `http(s)` URL (corruption, or legacy data pre-dating this validation) is
    /// read as unset.
    async fn get_feeds_websub_hub_url(&self) -> Result<Option<HubUrl>> {
        let Some(raw) = self
            .get_raw(SiteConfigKey::FeedsWebsubHubUrl)
            .await?
            .and_then(text::non_empty_owned)
        else {
            return Ok(None);
        };
        if let Ok(url) = raw.parse::<HubUrl>() {
            Ok(Some(url))
        } else {
            tracing::warn!("ignoring unparseable stored feeds.websub_hub_url");
            Ok(None)
        }
    }

    /// Returns the feed-generation configuration as a single group, applying
    /// the same per-field defaults as the granular getters it delegates to.
    /// The granular getters remain for single-value callers (e.g. the worker's
    /// hub-URL read).
    async fn get_feeds_config(&self) -> Result<FeedsConfig> {
        Ok(FeedsConfig {
            min_items: self.get_feeds_min_items().await?,
            min_days: self.get_feeds_min_days().await?,
            websub_hub_url: self.get_feeds_websub_hub_url().await?,
        })
    }

    /// Returns the site identity (title and base URL).
    async fn get_identity(&self) -> Result<SiteIdentity> {
        let title = self
            .get_raw(SiteConfigKey::SiteTitle)
            .await?
            .and_then(|v| v.parse::<SiteTitle>().ok())
            .unwrap_or_default();
        let base_url = match self
            .get_raw(SiteConfigKey::SiteBaseUrl)
            .await?
            .and_then(text::non_empty_owned)
        {
            None => None,
            Some(raw) => {
                if let Ok(url) = raw.parse::<BaseUrl>() {
                    Some(url)
                } else {
                    // Do not mutate while reading: callers that choose to repair this
                    // legacy value must acquire their own write capability.
                    tracing::warn!("ignoring unparseable stored site.base_url");
                    None
                }
            }
        };
        Ok(SiteIdentity { title, base_url })
    }
    /// Stores the site identity (title and base URL).
    /// For `base_url`, an empty string is stored when `None` is provided; a set
    /// value is stored in its canonical form (the `BaseUrl` normalized it).
    async fn set_identity(
        &self,
        transaction: &mut WriteTransaction,
        config: &SiteIdentity,
    ) -> Result<()> {
        self.set(transaction, SiteConfigKey::SiteTitle, &config.title)
            .await?;
        self.set_base_url(transaction, config.base_url.clone())
            .await
    }

    async fn set_backup_config(
        &self,
        transaction: &mut WriteTransaction,
        config: &BackupConfig,
    ) -> Result<()> {
        self.set(
            transaction,
            SiteConfigKey::BackupDestinationPath,
            config.destination_path.as_deref().unwrap_or(""),
        )
        .await?;
        self.set(transaction, SiteConfigKey::BackupSchedule, &config.schedule)
            .await?;
        self.set(
            transaction,
            SiteConfigKey::BackupRetentionCount,
            &config.retention_count.to_string(),
        )
        .await?;
        self.set(transaction, SiteConfigKey::BackupMode, config.mode.as_ref())
            .await?;
        Ok(())
    }

    /// Returns the configured site-wide Default Audience, falling back to
    /// [`DefaultAudience::Private`] when unset or unparseable. A Default
    /// Audience is a closed instance-wide value, distinct from the
    /// payload-bearing per-Post `AudienceTarget`.
    async fn get_default_audience(&self) -> Result<DefaultAudience> {
        Ok(self
            .get_raw(SiteConfigKey::PostsDefaultAudience)
            .await?
            .as_deref()
            .and_then(|value| value.parse().ok())
            .unwrap_or(DefaultAudience::Private))
    }

    /// Stores the closed instance-wide Default Audience using its standard
    /// string representation.
    async fn set_default_audience(
        &self,
        transaction: &mut WriteTransaction,
        audience: &DefaultAudience,
    ) -> Result<()> {
        self.set(
            transaction,
            SiteConfigKey::PostsDefaultAudience,
            audience.as_ref(),
        )
        .await
    }

    async fn set_feeds_config(
        &self,
        transaction: &mut WriteTransaction,
        config: &FeedsConfig,
    ) -> Result<()> {
        self.set(
            transaction,
            SiteConfigKey::FeedsMinItems,
            &config.min_items.to_string(),
        )
        .await?;
        self.set(
            transaction,
            SiteConfigKey::FeedsMinDays,
            &config.min_days.to_string(),
        )
        .await?;
        self.set(
            transaction,
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

/// A site-config value preserved exactly until its key-specific read policy parses it.
#[derive(Debug, macros::SqlxBridge)]
pub(crate) struct StoredSiteConfigValue(String);

impl StoredSiteConfigValue {
    fn into_inner(self) -> String {
        self.0
    }
}

/// A physically stored site-config key, including an unknown or orphan key.
#[derive(Debug, macros::SqlxBridge)]
struct StoredSiteConfigKey(String);

impl StoredSiteConfigKey {
    fn into_inner(self) -> String {
        self.0
    }
}

type SiteConfigExportRow = (StoredSiteConfigKey, StoredSiteConfigValue);

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
    (StoredSiteConfigValue,): for<'r> sqlx::FromRow<'r, DB::Row>,
    (StoredSiteConfigKey,): for<'r> sqlx::FromRow<'r, DB::Row>,
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
    for<'q> &'q str: Encode<'q, DB> + Type<DB>,
    // `SiteConfigKey`'s sqlx bridge reports `String` as its type (the token is bound as
    // borrowed text), so binding a key directly needs `String: Type<DB>` in scope.
    String: Type<DB>,
    for<'q> String: Encode<'q, DB>,
    for<'c> &'c Pool<DB>: Executor<'c, Database = DB>,
    for<'c> &'c mut DB::Connection: Executor<'c, Database = DB>,
    for<'q> DB::Arguments<'q>: sqlx::IntoArguments<'q, DB>,
{
    async fn get_raw(&self, key: SiteConfigKey) -> Result<Option<String>> {
        let row = sqlx::query_as::<_, (StoredSiteConfigValue,)>(
            "SELECT value FROM site_config WHERE key = $1",
        )
        .bind_storage(key)
        .fetch_optional(&self.pool)
        .await?;
        Ok(row.map(|(value,)| value.into_inner()))
    }

    #[tracing::instrument(
        name = "storage.site_config.set",
        skip(self, transaction, value),
        fields(db.system = DB::DB_SYSTEM)
    )]
    async fn set(
        &self,
        transaction: &mut WriteTransaction,
        key: SiteConfigKey,
        value: &str,
    ) -> Result<()> {
        set_stored::<DB>(transaction, key, StoredSiteConfigValue(value.to_owned())).await
    }

    async fn get_smtp_config(&self) -> Result<Option<SmtpConfig>> {
        // Six direct reads, each decoding the `value` column straight into its newtype via
        // that type's sqlx bridge: a garbage stored value fails `FromStr` and surfaces as a
        // `ColumnDecode` labelled with the key (see `read_value`), never as a silently
        // coerced default.
        let host = sqlx::query_as::<_, (SmtpHost,)>(SELECT_VALUE_SQL)
            .bind_storage(SiteConfigKey::SmtpHost)
            .fetch_optional(&self.pool)
            .await
            .map_err(|e| label_decode_error(SiteConfigKey::SmtpHost, e))?
            .map(|(host,)| host);
        let Some(host) = host else {
            // No host is not a misconfiguration: it is how an instance says "no SMTP".
            return Ok(None);
        };

        let port = sqlx::query_as::<_, (SmtpPort,)>(SELECT_VALUE_SQL)
            .bind_storage(SiteConfigKey::SmtpPort)
            .fetch_optional(&self.pool)
            .await
            .map_err(|e| label_decode_error(SiteConfigKey::SmtpPort, e))?
            .map_or_else(SmtpPort::default, |(port,)| port);

        let tls_mode = sqlx::query_as::<_, (SmtpTlsMode,)>(SELECT_VALUE_SQL)
            .bind_storage(SiteConfigKey::SmtpTlsMode)
            .fetch_optional(&self.pool)
            .await
            .map_err(|e| label_decode_error(SiteConfigKey::SmtpTlsMode, e))?
            .map_or_else(SmtpTlsMode::default, |(tls_mode,)| tls_mode);

        let sender = sqlx::query_as::<_, (SmtpSender,)>(SELECT_VALUE_SQL)
            .bind_storage(SiteConfigKey::SmtpSender)
            .fetch_optional(&self.pool)
            .await
            .map_err(|e| label_decode_error(SiteConfigKey::SmtpSender, e))?
            .map_or_else(SmtpSender::default, |(sender,)| sender);

        let username = sqlx::query_as::<_, (SmtpUsername,)>(SELECT_VALUE_SQL)
            .bind_storage(SiteConfigKey::SmtpUsername)
            .fetch_optional(&self.pool)
            .await
            .map_err(|e| label_decode_error(SiteConfigKey::SmtpUsername, e))?
            .map(|(username,)| username);

        let password = sqlx::query_as::<_, (SmtpPassword,)>(SELECT_VALUE_SQL)
            .bind_storage(SiteConfigKey::SmtpPassword)
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

    async fn list(&self) -> Result<Vec<(String, String)>> {
        let rows = sqlx::query_as::<_, SiteConfigExportRow>(
            "SELECT key, value FROM site_config ORDER BY key",
        )
        .fetch_all(&self.pool)
        .await?;
        Ok(rows
            .into_iter()
            .map(|(key, value)| (key.into_inner(), value.into_inner()))
            .collect())
    }

    async fn delete(&self, transaction: &mut WriteTransaction, key: SiteConfigKey) -> Result<bool> {
        let connection = DB::write_connection(transaction)?;
        // `RETURNING` + `fetch_optional` detects a no-match generically (a `None`),
        // avoiding `rows_affected()` which sqlx exposes only on concrete results
        // (mirrors `audiences::rename_audience`). Both backends support RETURNING.
        let removed = sqlx::query_as::<_, (StoredSiteConfigKey,)>(
            "DELETE FROM site_config WHERE key = $1 RETURNING key",
        )
        .bind_storage(key)
        .fetch_optional(connection)
        .await?;
        Ok(removed.is_some())
    }
}
async fn set_stored<DB>(
    transaction: &mut WriteTransaction,
    key: SiteConfigKey,
    value: StoredSiteConfigValue,
) -> Result<()>
where
    DB: Database + Backend,
    SiteConfigKey: Type<DB>,
    for<'q> SiteConfigKey: Encode<'q, DB>,
    String: Type<DB>,
    for<'q> String: Encode<'q, DB>,
    StoredSiteConfigValue: Type<DB>,
    for<'q> StoredSiteConfigValue: Encode<'q, DB>,
    for<'c> &'c mut DB::Connection: Executor<'c, Database = DB>,
    for<'q> DB::Arguments<'q>: sqlx::IntoArguments<'q, DB>,
{
    let connection = DB::write_connection(transaction)?;
    sqlx::query(
        "INSERT INTO site_config (key, value) VALUES ($1, $2)
             ON CONFLICT (key) DO UPDATE SET value = excluded.value",
    )
    .bind_storage(key)
    .bind_storage(value)
    .execute(connection)
    .await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::{SiteConfigKey, SmtpTlsMode};
    use crate::test_support::{
        Backend, TestEnv, backends, backends_matrix, confirmed, inject_invalid_site_config,
    };
    use common::backup::{BackupConfig, BackupMode, RetentionCount};
    use common::media::{MaxFileSize, UserQuota};
    use common::registration::RegistrationPolicy;
    use common::tagged_url::HubUrl;
    use common::test_support::{
        parse_destination_path, parse_max_file_size, parse_retention_count, parse_site_title,
        parse_smtp_username, parse_url, parse_user_quota,
    };
    use common::visibility::DefaultAudience;
    use host::feed::{FeedMinDays, FeedMinItems, FeedsConfig};
    use host::test_support::{parse_feed_min_days, parse_feed_min_items};
    use rstest::*;
    use rstest_reuse::*;

    async fn set_config(env: &TestEnv, key: SiteConfigKey, value: &str) -> anyhow::Result<()> {
        let storage = std::sync::Arc::clone(&env.state.site_config);
        let value = value.to_owned();
        confirmed(
            env.state
                .write_scope
                .run(move |transaction| {
                    Box::pin(async move { storage.set(transaction, key, &value).await })
                })
                .await?,
        );
        Ok(())
    }

    async fn delete_config(env: &TestEnv, key: SiteConfigKey) -> anyhow::Result<bool> {
        let storage = std::sync::Arc::clone(&env.state.site_config);
        Ok(confirmed(
            env.state
                .write_scope
                .run(move |transaction| {
                    Box::pin(async move { storage.delete(transaction, key).await })
                })
                .await?,
        ))
    }

    #[apply(backends)]
    #[tokio::test]
    async fn site_config_primitives_round_trip(#[case] backend: Backend) {
        let env = backend.setup().pristine().await;
        let store = &*env.state.site_config;
        set_config(&env, SiteConfigKey::SiteTitle, "T")
            .await
            .unwrap();
        set_config(&env, SiteConfigKey::BackupMode, "archive")
            .await
            .unwrap();
        assert_eq!(
            store.get_raw(SiteConfigKey::SiteTitle).await.unwrap(),
            Some("T".to_string())
        );
        set_config(&env, SiteConfigKey::FeedsMinItems, "9")
            .await
            .unwrap();
        assert_eq!(
            store.list().await.unwrap(),
            vec![
                ("backup.mode".to_string(), "archive".to_string()),
                ("feeds.min_items".to_string(), "9".to_string()),
                ("site.title".to_string(), "T".to_string()),
            ],
        );
        assert!(delete_config(&env, SiteConfigKey::SiteTitle).await.unwrap());
        assert!(!delete_config(&env, SiteConfigKey::SiteTitle).await.unwrap());
        assert_eq!(store.get_raw(SiteConfigKey::SiteTitle).await.unwrap(), None);
    }

    #[apply(backends)]
    #[tokio::test]
    async fn list_preserves_unknown_keys_for_export_and_raw_cleanup(#[case] backend: Backend) {
        let env = backend.setup().pristine().await;
        let store = &*env.state.site_config;
        let unknown_key = "legacy.unregistered_key";
        let opaque_value = "value retained verbatim";
        env.base
            .pool()
            .execute(
                "INSERT INTO site_config (key, value) \
                 VALUES ('legacy.unregistered_key', 'value retained verbatim')",
            )
            .await
            .unwrap();

        assert_eq!(
            store.list().await.unwrap(),
            vec![(unknown_key.to_owned(), opaque_value.to_owned())]
        );

        env.base
            .pool()
            .execute("DELETE FROM site_config WHERE key = 'legacy.unregistered_key'")
            .await
            .unwrap();
        assert!(store.list().await.unwrap().is_empty());
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
        let config_storage = std::sync::Arc::clone(&env.state.site_config);
        let expected = config.clone();
        confirmed(
            env.state
                .write_scope
                .run(move |transaction| {
                    Box::pin(
                        async move { config_storage.set_backup_config(transaction, &config).await },
                    )
                })
                .await
                .unwrap(),
        );
        assert_eq!(storage.get_backup_config().await.unwrap(), expected);
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
        let config_storage = std::sync::Arc::clone(&env.state.site_config);
        let expected = config.clone();
        confirmed(
            env.state
                .write_scope
                .run(move |transaction| {
                    Box::pin(
                        async move { config_storage.set_feeds_config(transaction, &config).await },
                    )
                })
                .await
                .unwrap(),
        );
        let loaded = storage.get_feeds_config().await.unwrap();
        assert_eq!(loaded, expected);
        // Exercise the derived Clone/Debug so the aggregate struct is covered.
        assert_eq!(loaded.clone(), expected);
        assert!(!format!("{expected:?}").is_empty());
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
            set_config(&env, key, value).await.unwrap();
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
        set_config(&env, SiteConfigKey::SmtpHost, "mail.example.com")
            .await
            .unwrap();
        set_config(&env, SiteConfigKey::SmtpPort, "not-a-port")
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
        set_config(&env, SiteConfigKey::SmtpHost, "mail.example.com")
            .await
            .unwrap();
        inject_invalid_site_config(&env, SiteConfigKey::SmtpPort, "0")
            .await
            .unwrap();
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
        set_config(&env, SiteConfigKey::SmtpHost, "").await.unwrap();
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
        set_config(&env, SiteConfigKey::SmtpHost, "mail.example.com")
            .await
            .unwrap();
        // An empty stored credential bypasses the non-empty invariant only via tampering;
        // the bridge decode rejects it. Symmetric for username and password — and neither
        // error echoes the value, unlike the sibling keys above.
        for key in [SiteConfigKey::SmtpUsername, SiteConfigKey::SmtpPassword] {
            let dotted = key.as_ref();
            set_config(&env, key, "").await.unwrap();
            let err = storage.get_smtp_config().await.unwrap_err();
            assert!(
                matches!(&err, sqlx::Error::ColumnDecode { index, .. } if index == dotted),
                "expected a column-decode error for {dotted}, got: {err:?}"
            );
            delete_config(&env, key).await.unwrap();
        }
    }

    #[apply(backends)]
    #[tokio::test]
    async fn get_backup_config_ignores_invalid_stored_values(#[case] backend: Backend) {
        let env = backend.setup().await;
        let storage = &*env.state.site_config;
        set_config(&env, SiteConfigKey::BackupSchedule, "not a cron")
            .await
            .unwrap();
        set_config(&env, SiteConfigKey::BackupRetentionCount, "daily")
            .await
            .unwrap();
        set_config(&env, SiteConfigKey::BackupMode, "floppy")
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
        inject_invalid_site_config(&env, SiteConfigKey::BackupRetentionCount, "0")
            .await
            .unwrap();
        let config = storage.get_backup_config().await.unwrap();
        assert_eq!(config.retention_count, RetentionCount::default());
    }

    #[apply(backends)]
    #[tokio::test]
    async fn list_returns_all_entries_ordered_by_key(#[case] backend: Backend) {
        let env = backend.setup().pristine().await;
        let storage = &*env.state.site_config;
        // Insert out of key order to prove the ORDER BY, not insertion order.
        set_config(&env, SiteConfigKey::SiteTitle, "T")
            .await
            .unwrap();
        set_config(&env, SiteConfigKey::FeedsWebsubHubUrl, "https://h/")
            .await
            .unwrap();
        set_config(&env, SiteConfigKey::BackupMode, "archive")
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
        set_config(&env, SiteConfigKey::SiteTitle, "T")
            .await
            .unwrap();

        // Deleting a present key reports true and the row is gone.
        assert!(
            delete_config(&env, SiteConfigKey::SiteTitle).await.unwrap(),
            "deleting a present key reports true",
        );
        assert_eq!(
            storage.get_raw(SiteConfigKey::SiteTitle).await.unwrap(),
            None,
            "the row is removed",
        );

        // Deleting an absent key is an idempotent no-op reporting false.
        assert!(
            !delete_config(&env, SiteConfigKey::SiteTitle).await.unwrap(),
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
        set_config(&env, SiteConfigKey::FeedsMinItems, "50")
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
        inject_invalid_site_config(&env, SiteConfigKey::FeedsMinItems, "not a number")
            .await
            .unwrap();
        assert_eq!(
            storage.get_feeds_min_items().await.unwrap(),
            FeedMinItems::default()
        );
        // A stored `0` is rejected by the min-1 invariant and also falls back.
        inject_invalid_site_config(&env, SiteConfigKey::FeedsMinItems, "0")
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
        set_config(&env, SiteConfigKey::FeedsMinDays, "60")
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
        set_config(&env, SiteConfigKey::MediaMaxFileSizeBytes, "1024")
            .await
            .unwrap();
        assert_eq!(
            storage.get_media_max_file_size().await.unwrap(),
            parse_max_file_size("1024")
        );
        // A stored 0/negative is rejected by the positive invariant → falls back.
        inject_invalid_site_config(&env, SiteConfigKey::MediaMaxFileSizeBytes, "0")
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
        set_config(&env, SiteConfigKey::MediaUserQuotaBytes, "2048")
            .await
            .unwrap();
        assert_eq!(
            storage.get_media_user_quota().await.unwrap(),
            parse_user_quota("2048")
        );
        inject_invalid_site_config(&env, SiteConfigKey::MediaUserQuotaBytes, "-5")
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
        set_config(
            &env,
            SiteConfigKey::FeedsWebsubHubUrl,
            "https://hub.example.com/",
        )
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
        set_config(&env, SiteConfigKey::FeedsWebsubHubUrl, "")
            .await
            .unwrap();
        assert!(storage.get_feeds_websub_hub_url().await.unwrap().is_none());
    }

    #[apply(backends)]
    #[tokio::test]
    async fn feeds_websub_hub_url_ignores_unparseable_stored_value(#[case] backend: Backend) {
        // Reads do not acquire write capabilities merely to repair legacy data.
        let env = backend.setup().await;
        let storage = &*env.state.site_config;
        inject_invalid_site_config(&env, SiteConfigKey::FeedsWebsubHubUrl, "not-a-url")
            .await
            .unwrap();
        assert_eq!(storage.get_feeds_websub_hub_url().await.unwrap(), None);
        assert_eq!(
            storage
                .get_raw(SiteConfigKey::FeedsWebsubHubUrl)
                .await
                .unwrap(),
            Some("not-a-url".to_owned())
        );
    }

    #[apply(backends)]
    #[tokio::test]
    async fn identity_returns_defaults_when_unset(#[case] backend: Backend) {
        let env = backend.setup().base_url(None).await;
        let storage = &*env.state.site_config;
        let identity = storage.get_identity().await.expect("get_identity");
        assert_eq!(identity.title, common::site::DEFAULT_SITE_TITLE);
        assert_eq!(identity.base_url, None);
    }

    #[apply(backends)]
    #[tokio::test]
    async fn identity_returns_override_when_title_set(#[case] backend: Backend) {
        let env = backend.setup().base_url(None).await;
        let storage = &*env.state.site_config;
        set_config(&env, SiteConfigKey::SiteTitle, "My Blog")
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
        set_config(&env, SiteConfigKey::SiteBaseUrl, "https://example.com")
            .await
            .unwrap();
        let identity = storage.get_identity().await.expect("get_identity");
        assert_eq!(identity.title, common::site::DEFAULT_SITE_TITLE);
        assert_eq!(identity.base_url.as_deref(), Some("https://example.com/"));
    }

    #[apply(backends)]
    #[tokio::test]
    async fn identity_ignores_unparseable_stored_base_url(#[case] backend: Backend) {
        // Reads do not acquire write capabilities merely to repair legacy data.
        let env = backend.setup().await;
        let storage = &*env.state.site_config;
        inject_invalid_site_config(&env, SiteConfigKey::SiteBaseUrl, "not-a-url")
            .await
            .unwrap();
        assert_eq!(storage.get_identity().await.unwrap().base_url, None);
        assert_eq!(
            storage.get_raw(SiteConfigKey::SiteBaseUrl).await.unwrap(),
            Some("not-a-url".to_owned())
        );
    }

    #[apply(backends)]
    #[tokio::test]
    async fn identity_treats_empty_title_as_unset(#[case] backend: Backend) {
        let env = backend.setup().await;
        let storage = &*env.state.site_config;
        set_config(&env, SiteConfigKey::SiteTitle, "   ")
            .await
            .unwrap();
        let identity = storage.get_identity().await.expect("get_identity");
        assert_eq!(identity.title, common::site::DEFAULT_SITE_TITLE);
    }

    #[apply(backends)]
    #[tokio::test]
    async fn identity_treats_empty_base_url_as_none(#[case] backend: Backend) {
        let env = backend.setup().await;
        let storage = &*env.state.site_config;
        set_config(&env, SiteConfigKey::SiteBaseUrl, "")
            .await
            .unwrap();
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
        let config_storage = std::sync::Arc::clone(&env.state.site_config);
        let expected = original.clone();
        confirmed(
            env.state
                .write_scope
                .run(move |transaction| {
                    Box::pin(
                        async move { config_storage.set_identity(transaction, &original).await },
                    )
                })
                .await
                .unwrap(),
        );
        let retrieved = storage.get_identity().await.expect("get_identity");
        assert_eq!(retrieved, expected);
    }

    #[apply(backends)]
    #[tokio::test]
    async fn get_backup_config_treats_empty_destination_as_none(#[case] backend: Backend) {
        let env = backend.setup().await;
        let storage = &*env.state.site_config;
        set_config(&env, SiteConfigKey::BackupDestinationPath, "")
            .await
            .unwrap();
        let config = storage.get_backup_config().await.unwrap();
        assert_eq!(config.destination_path, None);
    }

    #[apply(backends)]
    #[tokio::test]
    async fn default_audience_returns_private_when_unset(#[case] backend: Backend) {
        let env = backend.setup().await;
        let storage = &*env.state.site_config;
        assert_eq!(
            storage.get_default_audience().await.unwrap(),
            DefaultAudience::Private
        );
    }

    #[apply(backends_matrix)]
    #[case(DefaultAudience::Public)]
    #[case(DefaultAudience::Subscribers)]
    #[case(DefaultAudience::Private)]
    #[tokio::test]
    async fn default_audience_round_trips_each_value(
        backend: Backend,
        #[case] audience: DefaultAudience,
    ) {
        let env = backend.setup().await;
        let storage = &*env.state.site_config;
        let config_storage = std::sync::Arc::clone(&env.state.site_config);
        let expected = audience;
        confirmed(
            env.state
                .write_scope
                .run(move |transaction| {
                    Box::pin(async move {
                        config_storage
                            .set_default_audience(transaction, &audience)
                            .await
                    })
                })
                .await
                .unwrap(),
        );
        assert_eq!(
            storage
                .get_raw(SiteConfigKey::PostsDefaultAudience)
                .await
                .unwrap(),
            Some(expected.as_ref().to_owned())
        );
        assert_eq!(storage.get_default_audience().await.unwrap(), expected);
    }

    #[apply(backends)]
    #[tokio::test]
    async fn default_audience_returns_private_for_invalid_stored_values(#[case] backend: Backend) {
        let env = backend.setup().await;
        let storage = &*env.state.site_config;
        for value in ["named", "not a real value", " private "] {
            inject_invalid_site_config(&env, SiteConfigKey::PostsDefaultAudience, value)
                .await
                .unwrap();
            assert_eq!(
                storage.get_default_audience().await.unwrap(),
                DefaultAudience::Private
            );
        }
    }

    #[apply(backends)]
    #[tokio::test]
    async fn default_audience_propagates_database_errors(#[case] backend: Backend) {
        let env = backend.setup().await;
        let storage = &*env.state.site_config;
        env.base.pool().close().await;
        assert!(matches!(
            storage.get_default_audience().await,
            Err(sqlx::Error::PoolClosed)
        ));
    }

    // --- get_registration_policy (typed config accessor, #607) ---
    // (type-behavior tests — FromStr / Display / serde — live with the type in
    // `common::registration`.)

    #[apply(backends)]
    #[tokio::test]
    async fn registration_policy_defaults_to_closed_when_absent(#[case] backend: Backend) {
        let env = backend.setup().pristine().await;
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
        for policy in [
            RegistrationPolicy::Open,
            RegistrationPolicy::InviteOnly,
            RegistrationPolicy::Closed,
        ] {
            let config_storage = std::sync::Arc::clone(&env.state.site_config);
            confirmed(
                env.state
                    .write_scope
                    .run(move |transaction| {
                        Box::pin(async move {
                            config_storage
                                .set_registration_policy(transaction, policy)
                                .await
                        })
                    })
                    .await
                    .unwrap(),
            );
            assert_eq!(storage.get_registration_policy().await.unwrap(), policy);
        }
    }

    #[apply(backends)]
    #[tokio::test]
    async fn registration_policy_falls_back_to_closed_when_garbage(#[case] backend: Backend) {
        let env = backend.setup().await;
        let storage = &*env.state.site_config;
        inject_invalid_site_config(&env, SiteConfigKey::SiteRegistrationPolicy, "garbage")
            .await
            .unwrap();
        assert_eq!(
            storage.get_registration_policy().await.unwrap(),
            RegistrationPolicy::Closed
        );
    }
}
