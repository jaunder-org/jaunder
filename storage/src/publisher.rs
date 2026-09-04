//! Coherent publisher configuration snapshots and generation-fenced cache commits.

use async_trait::async_trait;
use common::etag::ETag;
use common::media::ContentType;
use common::site::{SiteIdentity, SiteTitle};
use common::tagged_url::{BaseUrl, HubUrl};
use common::time::UtcInstant;
use host::config_key::SiteConfigKey;
use host::feed::{FeedPath, FeedsConfig};
use sqlx::{ColumnIndex, Database, Decode, Encode, Error, Executor, FromRow, Pool, Type};

use crate::feed_cache::{FeedCacheError, FeedCacheRow, upsert_on_connection};
use crate::site_config::StoredSiteConfigValue;
use crate::sql::QueryStorageExt;
use crate::{Backend, WriteTransaction};

/// Opaque durable version of the configured `WebSub` hub.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Ord, PartialOrd, macros::SqlxBridge)]
pub struct PublisherGeneration(i64);

/// Opaque compare token for an invalid stored hub value.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct MalformedHubToken(String);

/// Configuration read once for a publisher attempt.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PublisherSnapshot {
    pub feeds: FeedsConfig,
    pub identity: SiteIdentity,
    pub generation: PublisherGeneration,
    malformed_hub: Option<MalformedHubToken>,
}

impl PublisherSnapshot {
    /// Returns a token only when the stored hub needs compare-safe repair.
    #[must_use]
    pub fn malformed_hub(&self) -> Option<MalformedHubToken> {
        self.malformed_hub.clone()
    }
}

/// Whether a hub mutation changed durable publisher state.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum HubMutationOutcome {
    Changed { generation: PublisherGeneration },
    Unchanged { generation: PublisherGeneration },
}

/// Result of a generation-fenced cache write.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum CacheCommitOutcome {
    Committed,
    StaleGeneration,
}

#[derive(Debug, thiserror::Error)]
pub enum PublisherStorageError {
    #[error("database error: {0}")]
    Db(#[from] Error),
    #[error("feed cache error: {0}")]
    Cache(#[from] FeedCacheError),
}

/// The persistence seam shared by worker and configuration surfaces.
#[cfg_attr(any(test, feature = "test-utils"), mockall::automock)]
#[async_trait]
pub trait PublisherStorage: Send + Sync {
    /// Reads feed configuration, site identity, and generation from one database statement.
    async fn snapshot(&self) -> Result<PublisherSnapshot, PublisherStorageError>;

    /// Applies a normalized hub mutation and invalidates every cached feed when it changes.
    async fn mutate_hub(
        &self,
        transaction: &mut WriteTransaction,
        hub: Option<HubUrl>,
    ) -> Result<HubMutationOutcome, PublisherStorageError>;

    /// Conditionally repairs exactly the malformed value represented by `token`.
    async fn repair_malformed_hub(
        &self,
        transaction: &mut WriteTransaction,
        token: MalformedHubToken,
    ) -> Result<HubMutationOutcome, PublisherStorageError>;

    /// Checks whether `generation` remains current without mutating cache state.
    async fn is_current_generation(
        &self,
        generation: PublisherGeneration,
    ) -> Result<bool, PublisherStorageError>;

    /// Writes a rendered cache entry only if `generation` is still current.
    async fn commit_cache(
        &self,
        transaction: &mut WriteTransaction,
        generation: PublisherGeneration,
        row: FeedCacheRow,
    ) -> Result<CacheCommitOutcome, PublisherStorageError>;
}

/// Generic dual-backend publisher persistence.
pub struct PublisherStore<DB: Database> {
    pool: Pool<DB>,
}

impl<DB: Database> PublisherStore<DB> {
    #[must_use]
    pub fn new(pool: Pool<DB>) -> Self {
        Self { pool }
    }
}

#[derive(FromRow)]
struct SnapshotRow {
    min_items: Option<StoredSiteConfigValue>,
    min_days: Option<StoredSiteConfigValue>,
    hub: Option<StoredSiteConfigValue>,
    title: Option<StoredSiteConfigValue>,
    base_url: Option<StoredSiteConfigValue>,
    generation: PublisherGeneration,
}

fn parse_hub(raw: Option<String>) -> Option<HubUrl> {
    raw.and_then(|value| (!value.is_empty()).then_some(value))
        .and_then(|value| value.parse().ok())
}

fn snapshot_from_row(row: SnapshotRow) -> PublisherSnapshot {
    let min_items = row.min_items.map(StoredSiteConfigValue::into_inner);
    let min_days = row.min_days.map(StoredSiteConfigValue::into_inner);
    let hub = row.hub.map(StoredSiteConfigValue::into_inner);
    let title = row.title.map(StoredSiteConfigValue::into_inner);
    let base_url = row.base_url.map(StoredSiteConfigValue::into_inner);
    let malformed_hub = hub
        .as_ref()
        .filter(|value| parse_hub(Some((*value).clone())).is_none())
        .map(|value| MalformedHubToken(value.clone()));
    let base_url = base_url
        .and_then(|value| (!value.is_empty()).then_some(value))
        .and_then(|value| value.parse::<BaseUrl>().ok());
    PublisherSnapshot {
        feeds: FeedsConfig {
            min_items: min_items
                .and_then(|value| value.parse().ok())
                .unwrap_or_default(),
            min_days: min_days
                .and_then(|value| value.parse().ok())
                .unwrap_or_default(),
            websub_hub_url: parse_hub(hub),
        },
        identity: SiteIdentity {
            title: title
                .and_then(|value| value.parse::<SiteTitle>().ok())
                .unwrap_or_default(),
            base_url,
        },
        generation: row.generation,
        malformed_hub,
    }
}

#[async_trait]
impl<DB> PublisherStorage for PublisherStore<DB>
where
    DB: Backend,
    SnapshotRow: for<'r> FromRow<'r, DB::Row>,
    for<'q> &'q str: Encode<'q, DB> + Type<DB>,
    for<'q> String: Encode<'q, DB>,
    String: Type<DB>,
    for<'q> StoredSiteConfigValue: Encode<'q, DB> + Type<DB>,
    for<'r> StoredSiteConfigValue: Decode<'r, DB> + Type<DB>,
    for<'q> PublisherGeneration: Encode<'q, DB> + Type<DB>,
    for<'r> PublisherGeneration: Decode<'r, DB> + Type<DB>,
    usize: ColumnIndex<DB::Row>,
    for<'q> ETag: Encode<'q, DB> + Type<DB>,
    for<'q> ContentType: Encode<'q, DB> + Type<DB>,
    for<'q> UtcInstant: Encode<'q, DB> + Type<DB>,
    for<'q> FeedPath: Encode<'q, DB> + Type<DB>,
    for<'c> &'c Pool<DB>: Executor<'c, Database = DB>,
    for<'c> &'c mut DB::Connection: Executor<'c, Database = DB>,
    for<'q> DB::Arguments<'q>: sqlx::IntoArguments<'q, DB>,
{
    async fn snapshot(&self) -> Result<PublisherSnapshot, PublisherStorageError> {
        let row = sqlx::query_as::<_, SnapshotRow>(
            "SELECT \
             MAX(CASE WHEN c.key = 'feeds.min_items' THEN c.value END) AS min_items, \
             MAX(CASE WHEN c.key = 'feeds.min_days' THEN c.value END) AS min_days, \
             MAX(CASE WHEN c.key = 'feeds.websub_hub_url' THEN c.value END) AS hub, \
             MAX(CASE WHEN c.key = 'site.title' THEN c.value END) AS title, \
             MAX(CASE WHEN c.key = 'site.base_url' THEN c.value END) AS base_url, \
             s.generation AS generation \
             FROM publisher_state s LEFT JOIN site_config c ON c.key IN \
             ('feeds.min_items', 'feeds.min_days', 'feeds.websub_hub_url', 'site.title', 'site.base_url') \
             WHERE s.id = 1 GROUP BY s.generation",
        )
        .fetch_one(&self.pool)
        .await?;
        Ok(snapshot_from_row(row))
    }

    async fn mutate_hub(
        &self,
        transaction: &mut WriteTransaction,
        hub: Option<HubUrl>,
    ) -> Result<HubMutationOutcome, PublisherStorageError> {
        let connection = DB::write_connection(transaction)?;
        let stored = sqlx::query_as::<_, (StoredSiteConfigValue,)>(
            "SELECT value FROM site_config WHERE key = $1",
        )
        .bind_storage(SiteConfigKey::FeedsWebsubHubUrl)
        .fetch_optional(&mut *connection)
        .await?
        .map(|(value,)| value.into_inner());
        let parsed = parse_hub(stored.clone());
        let desired = hub.map(|hub| hub.to_string());
        let malformed = stored.is_some() && parsed.is_none();
        if parsed.as_ref().map(ToString::to_string) == desired && !malformed {
            let generation = sqlx::query_scalar::<_, PublisherGeneration>(
                "SELECT generation FROM publisher_state WHERE id = 1",
            )
            .fetch_one(&mut *connection)
            .await?;
            return Ok(HubMutationOutcome::Unchanged { generation });
        }
        if let Some(value) = desired {
            sqlx::query(
                "INSERT INTO site_config (key, value) VALUES ($1, $2) \
                 ON CONFLICT(key) DO UPDATE SET value = excluded.value",
            )
            .bind_storage(SiteConfigKey::FeedsWebsubHubUrl)
            .bind_storage(StoredSiteConfigValue::new(value))
            .execute(&mut *connection)
            .await?;
        } else {
            // A legacy malformed row is repairable, but the exact raw value is
            // the compare token: never erase a valid replacement that won after
            // our coherent read on PostgreSQL.
            let Some(stored) = stored else {
                let generation = sqlx::query_scalar::<_, PublisherGeneration>(
                    "SELECT generation FROM publisher_state WHERE id = 1",
                )
                .fetch_one(&mut *connection)
                .await?;
                return Ok(HubMutationOutcome::Unchanged { generation });
            };
            let removed = sqlx::query_as::<_, (StoredSiteConfigValue,)>(
                "DELETE FROM site_config WHERE key = $1 AND value = $2 RETURNING value",
            )
            .bind_storage(SiteConfigKey::FeedsWebsubHubUrl)
            .bind_storage(StoredSiteConfigValue::new(stored))
            .fetch_optional(&mut *connection)
            .await?;
            if removed.is_none() {
                let generation = sqlx::query_scalar::<_, PublisherGeneration>(
                    "SELECT generation FROM publisher_state WHERE id = 1",
                )
                .fetch_one(&mut *connection)
                .await?;
                return Ok(HubMutationOutcome::Unchanged { generation });
            }
        }
        let generation = sqlx::query_scalar::<_, PublisherGeneration>(
            "UPDATE publisher_state SET generation = generation + 1 WHERE id = 1 RETURNING generation",
        )
        .fetch_one(&mut *connection)
        .await?;
        sqlx::query("DELETE FROM feed_cache")
            .execute(&mut *connection)
            .await?;
        Ok(HubMutationOutcome::Changed { generation })
    }

    async fn repair_malformed_hub(
        &self,
        transaction: &mut WriteTransaction,
        token: MalformedHubToken,
    ) -> Result<HubMutationOutcome, PublisherStorageError> {
        let connection = DB::write_connection(transaction)?;
        let removed = sqlx::query_as::<_, (StoredSiteConfigValue,)>(
            "DELETE FROM site_config WHERE key = $1 AND value = $2 RETURNING value",
        )
        .bind_storage(SiteConfigKey::FeedsWebsubHubUrl)
        .bind_storage(StoredSiteConfigValue::new(token.0))
        .fetch_optional(&mut *connection)
        .await?;
        if removed.is_none() {
            let generation = sqlx::query_scalar::<_, PublisherGeneration>(
                "SELECT generation FROM publisher_state WHERE id = 1",
            )
            .fetch_one(&mut *connection)
            .await?;
            return Ok(HubMutationOutcome::Unchanged { generation });
        }
        let generation = sqlx::query_scalar::<_, PublisherGeneration>(
            "UPDATE publisher_state SET generation = generation + 1 WHERE id = 1 RETURNING generation",
        )
        .fetch_one(&mut *connection)
        .await?;
        sqlx::query("DELETE FROM feed_cache")
            .execute(&mut *connection)
            .await?;
        Ok(HubMutationOutcome::Changed { generation })
    }

    async fn is_current_generation(
        &self,
        generation: PublisherGeneration,
    ) -> Result<bool, PublisherStorageError> {
        let stored_generation = sqlx::query_scalar::<_, PublisherGeneration>(
            "SELECT generation FROM publisher_state WHERE id = 1",
        )
        .fetch_one(&self.pool)
        .await?;
        Ok(stored_generation == generation)
    }

    async fn commit_cache(
        &self,
        transaction: &mut WriteTransaction,
        generation: PublisherGeneration,
        row: FeedCacheRow,
    ) -> Result<CacheCommitOutcome, PublisherStorageError> {
        let connection = DB::write_connection(transaction)?;
        let current = sqlx::query_scalar::<_, PublisherGeneration>(
            "UPDATE publisher_state SET generation = generation \
             WHERE id = 1 AND generation = $1 RETURNING generation",
        )
        .bind_storage(generation)
        .fetch_optional(&mut *connection)
        .await?;
        if current.is_none() {
            return Ok(CacheCommitOutcome::StaleGeneration);
        }
        upsert_on_connection::<DB>(connection, row).await?;
        Ok(CacheCommitOutcome::Committed)
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use super::*;
    use crate::test_support::fp;
    use crate::test_support::{Backend, TestEnv, backends, confirmed, inject_invalid_site_config};
    use common::test_support::parse_url;
    use common::{feed::FeedFormat, test_support::parse_etag, time::UtcInstant};
    use host::feed::SyndicationFeedRepresentation;
    use host::feed::{FeedMinDays, FeedMinItems};
    use rstest::*;
    use rstest_reuse::*;

    async fn mutate(env: &TestEnv, hub: Option<&HubUrl>) -> HubMutationOutcome {
        let publisher = Arc::clone(&env.state.publisher);
        let hub = hub.cloned();
        confirmed(
            env.state
                .write_scope
                .run(move |transaction| {
                    Box::pin(async move { publisher.mutate_hub(transaction, hub).await })
                })
                .await
                .expect("mutate publisher hub"),
        )
    }

    fn cache_row() -> FeedCacheRow {
        let path = fp("/feed.rss");
        let (_, format) = path.parts().expect("valid feed path");
        let now = UtcInstant::now();
        FeedCacheRow::new(
            path,
            SyndicationFeedRepresentation::try_from_stored(
                format,
                FeedFormat::Rss.content_type(),
                "<rss/>".to_owned(),
            )
            .expect("valid representation"),
            parse_etag("\"sha256-deadbeef\""),
            now,
            now,
        )
        .expect("matching cache row")
    }

    #[apply(backends)]
    #[tokio::test]
    async fn normalized_hub_noop_preserves_generation(#[case] backend: Backend) {
        let env = backend.setup().await;
        let hub: HubUrl = parse_url("https://hub.example.test/");
        let first = match mutate(&env, Some(&hub)).await {
            HubMutationOutcome::Changed { generation } => generation,
            HubMutationOutcome::Unchanged { .. } => panic!("first hub write must change"),
        };

        assert_eq!(
            mutate(&env, Some(&hub)).await,
            HubMutationOutcome::Unchanged { generation: first }
        );
        assert_eq!(
            env.state.publisher.snapshot().await.unwrap().generation,
            first
        );
    }

    #[apply(backends)]
    #[tokio::test]
    async fn snapshot_decodes_defaults_as_one_publisher_value(#[case] backend: Backend) {
        let env = backend.setup().await;
        let snapshot = env.state.publisher.snapshot().await.unwrap();

        assert_eq!(snapshot.feeds.min_items, FeedMinItems::default());
        assert_eq!(snapshot.feeds.min_days, FeedMinDays::default());
        assert_eq!(snapshot.feeds.websub_hub_url, None);
        assert_eq!(snapshot.identity.title, SiteTitle::default());
        assert_eq!(
            snapshot.identity.base_url.as_ref().map(ToString::to_string),
            Some("https://example.com/".to_owned())
        );
        assert_eq!(snapshot.generation, PublisherGeneration(0));
    }

    #[apply(backends)]
    #[tokio::test]
    async fn hub_change_advances_snapshot_generation(#[case] backend: Backend) {
        let env = backend.setup().await;
        let before = env.state.publisher.snapshot().await.unwrap();
        let hub: HubUrl = parse_url("https://hub.example.test/");

        let generation = match mutate(&env, Some(&hub)).await {
            HubMutationOutcome::Changed { generation } => generation,
            HubMutationOutcome::Unchanged { .. } => panic!("absent hub must change"),
        };
        let after = env.state.publisher.snapshot().await.unwrap();

        assert!(generation > before.generation);
        assert_eq!(after.generation, generation);
        assert_eq!(after.feeds.websub_hub_url, Some(hub));
    }

    #[apply(backends)]
    #[tokio::test]
    async fn malformed_hub_repair_advances_generation(#[case] backend: Backend) {
        let env = backend.setup().await;
        inject_invalid_site_config(&env, SiteConfigKey::FeedsWebsubHubUrl, "not a hub URL")
            .await
            .expect("seed malformed hub");
        let before = env.state.publisher.snapshot().await.unwrap().generation;

        let generation = match mutate(&env, None).await {
            HubMutationOutcome::Changed { generation } => generation,
            HubMutationOutcome::Unchanged { .. } => panic!("malformed hub must repair"),
        };

        assert!(generation > before);
        assert_eq!(
            env.state
                .site_config
                .get_raw(SiteConfigKey::FeedsWebsubHubUrl)
                .await
                .unwrap(),
            None
        );
    }

    #[apply(backends)]
    #[tokio::test]
    async fn empty_hub_row_is_repaired(#[case] backend: Backend) {
        let env = backend.setup().await;
        inject_invalid_site_config(&env, SiteConfigKey::FeedsWebsubHubUrl, "")
            .await
            .expect("seed empty hub row");

        assert!(matches!(
            mutate(&env, None).await,
            HubMutationOutcome::Changed { .. }
        ));
        assert_eq!(
            env.state
                .site_config
                .get_raw(SiteConfigKey::FeedsWebsubHubUrl)
                .await
                .unwrap(),
            None
        );
    }

    #[apply(backends)]
    #[tokio::test]
    async fn stale_generation_does_not_commit_cache(#[case] backend: Backend) {
        let env = backend.setup().await;
        let stale = env.state.publisher.snapshot().await.unwrap().generation;
        let hub: HubUrl = parse_url("https://hub.example.test/");
        let _ = mutate(&env, Some(&hub)).await;
        let publisher = Arc::clone(&env.state.publisher);
        let row = cache_row();
        let outcome = confirmed(
            env.state
                .write_scope
                .run(move |transaction| {
                    Box::pin(async move { publisher.commit_cache(transaction, stale, row).await })
                })
                .await
                .expect("fence cache"),
        );

        assert_eq!(outcome, CacheCommitOutcome::StaleGeneration);
        assert!(
            env.state
                .feed_cache
                .get(&fp("/feed.rss"))
                .await
                .unwrap()
                .is_none()
        );
    }

    #[apply(backends)]
    #[tokio::test]
    async fn changed_hub_invalidates_cached_feed(#[case] backend: Backend) {
        let env = backend.setup().await;
        let row = cache_row();
        let path = row.feed_path().clone();
        let cache = Arc::clone(&env.state.feed_cache);
        confirmed(
            env.state
                .write_scope
                .run(move |transaction| {
                    Box::pin(async move { cache.upsert(transaction, row).await })
                })
                .await
                .expect("seed cache"),
        );
        let hub: HubUrl = parse_url("https://hub.example.test/");
        let _ = mutate(&env, Some(&hub)).await;
        assert!(env.state.feed_cache.get(&path).await.unwrap().is_none());
    }

    #[apply(backends)]
    #[tokio::test]
    async fn normalized_hub_noop_preserves_cached_feed(#[case] backend: Backend) {
        let env = backend.setup().await;
        let hub: HubUrl = parse_url("https://hub.example.test/");
        let _ = mutate(&env, Some(&hub)).await;
        let row = cache_row();
        let path = row.feed_path().clone();
        let cache = Arc::clone(&env.state.feed_cache);
        confirmed(
            env.state
                .write_scope
                .run(move |transaction| {
                    Box::pin(async move { cache.upsert(transaction, row).await })
                })
                .await
                .expect("seed cache"),
        );
        let _ = mutate(&env, Some(&hub)).await;
        assert!(env.state.feed_cache.get(&path).await.unwrap().is_some());
    }

    #[apply(backends)]
    #[tokio::test]
    async fn compare_token_repair_preserves_valid_replacement(#[case] backend: Backend) {
        let env = backend.setup().await;
        inject_invalid_site_config(&env, SiteConfigKey::FeedsWebsubHubUrl, "invalid A")
            .await
            .expect("seed invalid hub");
        let snapshot = env.state.publisher.snapshot().await.unwrap();
        let token = snapshot
            .malformed_hub()
            .expect("invalid row exposes repair token");
        let before = snapshot.generation;
        let valid = "https://replacement.example.test/";
        inject_invalid_site_config(&env, SiteConfigKey::FeedsWebsubHubUrl, valid)
            .await
            .expect("concurrent valid replacement");
        let publisher = Arc::clone(&env.state.publisher);
        let outcome = confirmed(
            env.state
                .write_scope
                .run(move |transaction| {
                    Box::pin(
                        async move { publisher.repair_malformed_hub(transaction, token).await },
                    )
                })
                .await
                .expect("conditional repair"),
        );

        assert_eq!(
            outcome,
            HubMutationOutcome::Unchanged { generation: before }
        );
        assert_eq!(
            env.state
                .site_config
                .get_raw(SiteConfigKey::FeedsWebsubHubUrl)
                .await
                .unwrap(),
            Some(valid.to_owned())
        );
        assert_eq!(
            env.state.publisher.snapshot().await.unwrap().generation,
            before
        );
    }
}
