//! Coherent publisher configuration snapshots and generation-fenced cache commits.

use async_trait::async_trait;
use common::etag::ETag;
use common::media::ContentType;
use common::site::{SiteIdentity, SiteTitle};
use common::tagged_url::{BaseUrl, HubUrl};
use common::time::UtcInstant;
use host::config_key::SiteConfigKey;
use host::feed::{FeedMinDays, FeedMinItems, FeedPath, FeedsConfig};
use sqlx::{ColumnIndex, Database, Decode, Encode, Error, Executor, FromRow, Pool, Type};

use crate::feed_cache::{FeedCacheError, FeedCacheRow, StoredFeedCacheRow, upsert_on_connection};
use crate::site_config;
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

/// Typed change to one member of the publisher-owned feed-window snapshot.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum FeedWindowMutation {
    SetMinItems(FeedMinItems),
    UnsetMinItems,
    SetMinDays(FeedMinDays),
    UnsetMinDays,
}

/// Result of an accepted feed-window mutation.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum FeedWindowMutationOutcome {
    Applied { generation: PublisherGeneration },
}

/// Whether a hub mutation changed durable publisher state.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum HubMutationOutcome {
    Changed { generation: PublisherGeneration },
    Unchanged { generation: PublisherGeneration },
}

/// Result of a generation-fenced cache write.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum CacheCommitOutcome {
    Committed(FeedCacheRow),
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

    /// Applies one feed-window change as a new publisher generation.
    async fn mutate_feed_window(
        &self,
        transaction: &mut WriteTransaction,
        mutation: FeedWindowMutation,
    ) -> Result<FeedWindowMutationOutcome, PublisherStorageError>;

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

fn snapshot_from_row(row: SnapshotRow) -> Result<PublisherSnapshot, Error> {
    let min_items = site_config::parse_feed_minimum(SiteConfigKey::FeedsMinItems, row.min_items)?;
    let min_days = site_config::parse_feed_minimum(SiteConfigKey::FeedsMinDays, row.min_days)?;
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
    Ok(PublisherSnapshot {
        feeds: FeedsConfig {
            min_items,
            min_days,
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
    })
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
    StoredFeedCacheRow: for<'r> FromRow<'r, DB::Row>,
    for<'c> &'c Pool<DB>: Executor<'c, Database = DB>,
    for<'c> &'c mut DB::Connection: Executor<'c, Database = DB>,
    DB::Arguments: sqlx::IntoArguments<DB>,
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
        Ok(snapshot_from_row(row)?)
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

    async fn mutate_feed_window(
        &self,
        transaction: &mut WriteTransaction,
        mutation: FeedWindowMutation,
    ) -> Result<FeedWindowMutationOutcome, PublisherStorageError> {
        let connection = DB::write_connection(transaction)?;
        match mutation {
            FeedWindowMutation::SetMinItems(value) => {
                sqlx::query(
                    "INSERT INTO site_config (key, value) VALUES ($1, $2) \
                     ON CONFLICT(key) DO UPDATE SET value = excluded.value",
                )
                .bind_storage(SiteConfigKey::FeedsMinItems)
                .bind_storage(StoredSiteConfigValue::new(value.to_string()))
                .execute(&mut *connection)
                .await?;
            }
            FeedWindowMutation::UnsetMinItems => {
                sqlx::query("DELETE FROM site_config WHERE key = $1")
                    .bind_storage(SiteConfigKey::FeedsMinItems)
                    .execute(&mut *connection)
                    .await?;
            }
            FeedWindowMutation::SetMinDays(value) => {
                sqlx::query(
                    "INSERT INTO site_config (key, value) VALUES ($1, $2) \
                     ON CONFLICT(key) DO UPDATE SET value = excluded.value",
                )
                .bind_storage(SiteConfigKey::FeedsMinDays)
                .bind_storage(StoredSiteConfigValue::new(value.to_string()))
                .execute(&mut *connection)
                .await?;
            }
            FeedWindowMutation::UnsetMinDays => {
                sqlx::query("DELETE FROM site_config WHERE key = $1")
                    .bind_storage(SiteConfigKey::FeedsMinDays)
                    .execute(&mut *connection)
                    .await?;
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
        Ok(FeedWindowMutationOutcome::Applied { generation })
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
        let effective_row = upsert_on_connection::<DB>(connection, row).await?;
        Ok(CacheCommitOutcome::Committed(effective_row))
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;
    use tokio::sync::Barrier;

    use super::*;
    use crate::test_support::fp;
    use crate::test_support::{Backend, TestEnv, backends, confirmed, inject_invalid_site_config};
    use common::{
        MutationOutcome,
        test_support::{parse_etag, parse_url},
        time::UtcInstant,
    };
    use host::feed::{FeedMinDays, FeedMinItems, SyndicationFeedRepresentation};
    use host::test_support::{parse_feed_min_days, parse_feed_min_items};
    use rstest::*;
    use rstest_reuse::*;

    async fn mutate_feed_window(
        env: &TestEnv,
        mutation: FeedWindowMutation,
    ) -> FeedWindowMutationOutcome {
        let publisher = Arc::clone(&env.state.publisher);
        confirmed(
            env.state
                .write_scope
                .run(move |transaction| {
                    Box::pin(
                        async move { publisher.mutate_feed_window(transaction, mutation).await },
                    )
                })
                .await
                .expect("mutate feed window"),
        )
    }

    async fn seed_cache(env: &TestEnv, row: FeedCacheRow) {
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
    }

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
        cache_row_at("/feed.rss")
    }

    fn cache_row_at(path: &str) -> FeedCacheRow {
        let path = fp(path);
        let (_, format) = path.parts().expect("valid feed path");
        let now = UtcInstant::now();
        FeedCacheRow::new(
            path,
            SyndicationFeedRepresentation::try_from_stored(
                format,
                format.content_type(),
                "<rss/>".to_owned(),
            )
            .expect("valid representation"),
            parse_etag("\"sha256-deadbeef\""),
            now,
            now,
            "0000000000000000000000000000000000000000000000000000000000000000"
                .parse()
                .expect("valid fingerprint"),
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

    /// Each typed mutation changes only its requested minimum; unsetting restores
    /// that minimum's absent-value default without disturbing its companion.
    #[apply(backends)]
    #[tokio::test]
    async fn feed_window_mutations_retain_companions_and_restore_defaults(
        #[case] backend: Backend,
    ) {
        let env = backend.setup().await;

        mutate_feed_window(
            &env,
            FeedWindowMutation::SetMinItems(parse_feed_min_items("42")),
        )
        .await;
        let snapshot = env.state.publisher.snapshot().await.unwrap();
        assert_eq!(snapshot.feeds.min_items, parse_feed_min_items("42"));
        assert_eq!(snapshot.feeds.min_days, FeedMinDays::default());

        mutate_feed_window(
            &env,
            FeedWindowMutation::SetMinDays(parse_feed_min_days("7")),
        )
        .await;
        let snapshot = env.state.publisher.snapshot().await.unwrap();
        assert_eq!(snapshot.feeds.min_items, parse_feed_min_items("42"));
        assert_eq!(snapshot.feeds.min_days, parse_feed_min_days("7"));

        mutate_feed_window(&env, FeedWindowMutation::UnsetMinItems).await;
        let snapshot = env.state.publisher.snapshot().await.unwrap();
        assert_eq!(snapshot.feeds.min_items, FeedMinItems::default());
        assert_eq!(snapshot.feeds.min_days, parse_feed_min_days("7"));

        mutate_feed_window(&env, FeedWindowMutation::UnsetMinDays).await;
        let snapshot = env.state.publisher.snapshot().await.unwrap();
        assert_eq!(snapshot.feeds.min_items, FeedMinItems::default());
        assert_eq!(snapshot.feeds.min_days, FeedMinDays::default());
    }

    /// Even an unchanged request establishes a new generation fence and removes
    /// every public representation from the cache in the same durable mutation.
    #[apply(backends)]
    #[tokio::test]
    async fn feed_window_noop_advances_generation_and_invalidates_every_cache_path(
        #[case] backend: Backend,
    ) {
        let env = backend.setup().await;
        for path in ["/feed.rss", "/feed.atom", "/feed.json"] {
            seed_cache(&env, cache_row_at(path)).await;
        }
        let stale = env.state.publisher.snapshot().await.unwrap().generation;

        let outcome = mutate_feed_window(
            &env,
            FeedWindowMutation::SetMinItems(FeedMinItems::default()),
        )
        .await;
        let FeedWindowMutationOutcome::Applied { generation } = outcome;
        assert!(generation > stale, "accepted no-op must advance generation");
        for path in ["/feed.rss", "/feed.atom", "/feed.json"] {
            assert!(
                env.state.feed_cache.get(&fp(path)).await.unwrap().is_none(),
                "{path} must be invalidated"
            );
        }

        let publisher = Arc::clone(&env.state.publisher);
        let outcome = confirmed(
            env.state
                .write_scope
                .run(move |transaction| {
                    Box::pin(async move {
                        publisher
                            .commit_cache(transaction, stale, cache_row())
                            .await
                    })
                })
                .await
                .expect("fence stale cache commit"),
        );
        assert_eq!(outcome, CacheCommitOutcome::StaleGeneration);
    }

    #[apply(backends)]
    #[tokio::test]
    async fn cache_commit_returns_the_effective_existing_row_for_matching_identity(
        #[case] backend: Backend,
    ) {
        let env = backend.setup().await;
        seed_cache(&env, cache_row()).await;
        let existing = env
            .state
            .feed_cache
            .get(&fp("/feed.rss"))
            .await
            .unwrap()
            .expect("seeded cache row");
        let generation = env.state.publisher.snapshot().await.unwrap().generation;
        let publisher = Arc::clone(&env.state.publisher);
        let candidate = FeedCacheRow::new(
            fp("/feed.rss"),
            SyndicationFeedRepresentation::try_from_stored(
                common::feed::FeedFormat::Rss,
                common::feed::FeedFormat::Rss.content_type(),
                "<rss>discarded</rss>".to_owned(),
            )
            .expect("valid representation"),
            parse_etag("\"sha256-discarded\""),
            UtcInstant::from(
                existing.representation_modified_at.value() + chrono::Duration::seconds(1),
            ),
            UtcInstant::from(existing.generated_at.value() + chrono::Duration::seconds(1)),
            existing.semantic_fingerprint().clone(),
        )
        .expect("matching cache row");
        let outcome = confirmed(
            env.state
                .write_scope
                .run(move |transaction| {
                    Box::pin(async move {
                        publisher
                            .commit_cache(transaction, generation, candidate)
                            .await
                    })
                })
                .await
                .expect("commit cache"),
        );

        assert_eq!(
            outcome,
            CacheCommitOutcome::Committed(
                FeedCacheRow::new(
                    fp("/feed.rss"),
                    existing.representation().clone(),
                    existing.etag.clone(),
                    existing.representation_modified_at,
                    UtcInstant::from(existing.generated_at.value() + chrono::Duration::seconds(1)),
                    existing.semantic_fingerprint().clone(),
                )
                .expect("matching effective row")
            )
        );
    }

    #[apply(backends)]
    #[tokio::test]
    async fn concurrent_matching_fingerprint_commits_preserve_one_identity(
        #[case] backend: Backend,
    ) {
        let env = backend.setup().await;
        let existing = cache_row();
        seed_cache(&env, existing).await;
        let existing = env
            .state
            .feed_cache
            .get(&fp("/feed.rss"))
            .await
            .unwrap()
            .expect("seeded cache row");
        let generation = env.state.publisher.snapshot().await.unwrap().generation;
        let candidate = |body: &str, offset| {
            FeedCacheRow::new(
                fp("/feed.rss"),
                SyndicationFeedRepresentation::try_from_stored(
                    common::feed::FeedFormat::Rss,
                    common::feed::FeedFormat::Rss.content_type(),
                    body.to_owned(),
                )
                .expect("valid representation"),
                parse_etag("\"sha256-candidate\""),
                UtcInstant::from(
                    existing.representation_modified_at.value() + chrono::Duration::seconds(offset),
                ),
                UtcInstant::from(existing.generated_at.value() + chrono::Duration::seconds(offset)),
                existing.semantic_fingerprint().clone(),
            )
            .expect("matching cache row")
        };
        let first = candidate("<rss>first candidate</rss>", 1);
        let latest_generated_at =
            UtcInstant::from(existing.generated_at.value() + chrono::Duration::seconds(2));
        let second = candidate("<rss>second candidate</rss>", 2);
        let barrier = Arc::new(Barrier::new(2));
        let one_publisher = Arc::clone(&env.state.publisher);
        let two_publisher = Arc::clone(&env.state.publisher);
        let one_scope = env.state.write_scope.clone();
        let two_scope = env.state.write_scope.clone();
        let one_barrier = Arc::clone(&barrier);
        let two_barrier = Arc::clone(&barrier);
        let (one, two) = tokio::join!(
            async move {
                one_barrier.wait().await;
                one_scope
                    .run(move |transaction| {
                        Box::pin(async move {
                            one_publisher
                                .commit_cache(transaction, generation, first)
                                .await
                        })
                    })
                    .await
            },
            async move {
                two_barrier.wait().await;
                two_scope
                    .run(move |transaction| {
                        Box::pin(async move {
                            two_publisher
                                .commit_cache(transaction, generation, second)
                                .await
                        })
                    })
                    .await
            },
        );
        let CacheCommitOutcome::Committed(one) = confirmed(one.expect("first commit")) else {
            panic!("first commit must be current")
        };
        let CacheCommitOutcome::Committed(two) = confirmed(two.expect("second commit")) else {
            panic!("second commit must be current")
        };
        for row in [one, two] {
            assert_eq!(
                row.representation().body(),
                existing.representation().body()
            );
            assert_eq!(row.etag, existing.etag);
            assert_eq!(
                row.representation_modified_at,
                existing.representation_modified_at
            );
            assert!(row.generated_at > existing.generated_at);
            assert!(row.generated_at <= latest_generated_at);
        }
        let persisted = env
            .state
            .feed_cache
            .get(&fp("/feed.rss"))
            .await
            .unwrap()
            .unwrap();
        assert_eq!(
            persisted.representation().body(),
            existing.representation().body()
        );
        assert_eq!(persisted.etag, existing.etag);
        assert_eq!(
            persisted.representation_modified_at,
            existing.representation_modified_at
        );
        assert_eq!(persisted.generated_at, latest_generated_at);
    }

    /// A callback failure rolls back configuration, generation, and cache
    /// invalidation together, leaving the prior publisher snapshot observable.
    #[apply(backends)]
    #[tokio::test]
    async fn failed_feed_window_mutation_preserves_the_prior_coherent_snapshot(
        #[case] backend: Backend,
    ) {
        let env = backend.setup().await;
        seed_cache(&env, cache_row()).await;
        let before = env.state.publisher.snapshot().await.unwrap();
        let publisher = Arc::clone(&env.state.publisher);

        let error = env
            .state
            .write_scope
            .run(move |transaction| {
                Box::pin(async move {
                    publisher
                        .mutate_feed_window(
                            transaction,
                            FeedWindowMutation::SetMinItems(parse_feed_min_items("42")),
                        )
                        .await?;
                    Err::<(), _>(PublisherStorageError::Db(Error::PoolClosed))
                })
            })
            .await
            .expect_err("operation error rolls back the enclosing write scope");
        assert!(matches!(
            error,
            crate::WriteScopeError::Operation(PublisherStorageError::Db(Error::PoolClosed))
        ));
        assert_eq!(env.state.publisher.snapshot().await.unwrap(), before);
        assert!(
            env.state
                .feed_cache
                .get(&fp("/feed.rss"))
                .await
                .unwrap()
                .is_some()
        );
    }

    /// Losing the acknowledgement after the durable mutation never lets callers
    /// report the new snapshot as confirmed.
    #[apply(backends)]
    #[tokio::test]
    async fn feed_window_commit_acknowledgement_loss_is_indeterminate(#[case] backend: Backend) {
        let env = backend.setup().await;
        let publisher = Arc::clone(&env.state.publisher);
        let outcome = env
            .state
            .write_scope
            .with_commit_acknowledgement_loss_after_commit_for_test()
            .run(move |transaction| {
                Box::pin(async move {
                    publisher
                        .mutate_feed_window(
                            transaction,
                            FeedWindowMutation::SetMinDays(parse_feed_min_days("7")),
                        )
                        .await
                })
            })
            .await
            .expect("acknowledgement loss is an outcome");
        assert!(matches!(
            outcome,
            MutationOutcome::CommitIndeterminate(FeedWindowMutationOutcome::Applied { .. })
        ));
    }

    /// Publisher work must never begin from a snapshot that silently substituted a default
    /// for a corrupt feed minimum.
    #[apply(backends)]
    #[tokio::test]
    async fn snapshot_rejects_a_corrupt_feed_minimum(#[case] backend: Backend) {
        let env = backend.setup().await;
        let corrupt = "corrupt-min-items-value";
        inject_invalid_site_config(&env, SiteConfigKey::FeedsMinItems, corrupt)
            .await
            .expect("seed corrupt feed minimum");

        let err = env.state.publisher.snapshot().await.unwrap_err();
        let diagnostic = err.to_string();
        assert!(
            diagnostic.contains("feeds.min_items"),
            "publisher snapshot diagnostics must identify the corrupt key",
        );
        assert!(
            diagnostic.contains("invalid"),
            "publisher snapshot diagnostics must retain the validation reason",
        );
        assert!(
            !diagnostic.contains(corrupt),
            "publisher snapshot diagnostics must redact the corrupt stored value",
        );
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
