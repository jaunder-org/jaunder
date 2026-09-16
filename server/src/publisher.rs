//! Cross-process serialization for publisher finalization and hub mutations.

use std::fs::{self, File, OpenOptions, TryLockError};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use anyhow::Context;
use common::MutationOutcome;
use common::tagged_url::HubUrl;
use common::{
    site::{SiteIdentity, SiteTagline, SiteTitle},
    tagged_url::BaseUrl,
};
use host::config_key::SiteConfigKey;
use sqlx::Error;
use storage::{
    BaseUrlMutationError, CacheCommitOutcome, FeedCacheRow, FeedWindowMutation,
    FeedWindowMutationOutcome, HubMutationOutcome, PasskeyStorage, PublisherGeneration,
    PublisherSnapshot, PublisherStorage, PublisherStorageError, SiteConfigStorage, WriteScope,
    WriteScopeError, clear_base_url_with_passkey_guard, set_base_url_with_passkey_guard,
};
use web::websub::{WebsubPublisher, WebsubPublisherError};
///
/// The lock file may remain after a process exits; the kernel releases its advisory
/// lock on close, cancellation unwinding, panic, and process death.
pub struct PublisherGateGuard {
    _file: File,
}

impl PublisherGateGuard {
    async fn acquire(storage_path: &Path) -> anyhow::Result<Self> {
        fs::create_dir_all(storage_path).with_context(|| {
            format!(
                "cannot create publisher gate directory {}",
                storage_path.display()
            )
        })?;
        let path = storage_path.join("publisher.lock");
        loop {
            let file = OpenOptions::new()
                .create(true)
                .append(true)
                .open(&path)
                .with_context(|| format!("cannot open publisher gate {}", path.display()))?;
            match file.try_lock() {
                Ok(()) => return Ok(Self { _file: file }),
                Err(TryLockError::WouldBlock) => {
                    tokio::time::sleep(Duration::from_millis(10)).await;
                }
                // cov:ignore-start: The regular-file advisory-lock backend has no deterministic seam for this post-open OS error.
                Err(TryLockError::Error(error)) => {
                    return Err(error).with_context(|| {
                        format!("cannot acquire publisher gate {}", path.display())
                    });
                } // cov:ignore-stop
            }
        }
    }
}

fn require_confirmed_malformed_hub_repair<T>(committed: &MutationOutcome<T>) -> anyhow::Result<()> {
    if matches!(committed, MutationOutcome::CommitIndeterminate(_)) {
        return Err(anyhow::anyhow!(
            "malformed hub repair commit acknowledgement was indeterminate"
        ));
    }
    Ok(())
}

#[derive(Debug, thiserror::Error)]
enum SiteIdentityMutationError {
    #[error(transparent)]
    BaseUrl(#[from] BaseUrlMutationError),
    #[error(transparent)]
    Config(#[from] sqlx::Error),
    #[error(transparent)]
    Publisher(#[from] PublisherStorageError),
    #[error(transparent)]
    Aggregate(#[from] storage::SiteIdentityMutationError),
}

pub enum SiteIdentityMutation {
    SetTitle(SiteTitle),
    SetTagline(Option<SiteTagline>),
    SetBaseUrl(Option<BaseUrl>),
    UnsetTitle,
    UnsetTagline,
    UnsetBaseUrl,
}

/// Shared publisher operation seam. The gate is acquired before every write scope.
#[derive(Clone)]
pub struct PublisherService {
    storage_path: PathBuf,
    publisher: Arc<dyn PublisherStorage>,
    write_scope: WriteScope,
}

impl PublisherService {
    #[must_use]
    pub fn new(
        storage_path: PathBuf,
        publisher: Arc<dyn PublisherStorage>,
        write_scope: WriteScope,
    ) -> Self {
        Self {
            storage_path,
            publisher,
            write_scope,
        }
    }

    /// Acquires the finalization region. Task 4 must retain the returned guard
    /// from generation-checked cache commit through the `WebSub` request.
    ///
    /// # Errors
    ///
    /// Returns an error if the publisher gate cannot be acquired.
    pub async fn finalization_guard(&self) -> anyhow::Result<PublisherFinalizationGuard> {
        Ok(PublisherFinalizationGuard {
            _gate: PublisherGateGuard::acquire(&self.storage_path).await?,
            publisher: Arc::clone(&self.publisher),
            write_scope: self.write_scope.clone(),
        })
    }

    /// Reads the attempt snapshot and repairs an invalid persisted hub before
    /// exposing it. The invalid raw text never crosses this service boundary.
    ///
    /// # Errors
    ///
    /// Returns an error if reading the snapshot, acquiring the publisher gate, or
    /// repairing the malformed hub fails. Also returns an error when the repair's
    /// commit acknowledgement is indeterminate.
    pub async fn snapshot(&self) -> anyhow::Result<PublisherSnapshot> {
        let snapshot = self.publisher.snapshot().await?;
        let Some(token) = snapshot.malformed_hub() else {
            return Ok(snapshot);
        };
        let _gate = PublisherGateGuard::acquire(&self.storage_path).await?;
        let publisher = Arc::clone(&self.publisher);
        let committed = self
            .write_scope
            .run(move |transaction| {
                Box::pin(async move { publisher.repair_malformed_hub(transaction, token).await })
            })
            .await?;
        require_confirmed_malformed_hub_repair(&committed)?;
        Ok(self.publisher.snapshot().await?)
    }

    /// Mutates one identity key under the publisher gate, invalidating cached
    /// Syndication Feeds in the same write scope.
    ///
    /// # Errors
    ///
    /// Returns an error when the publisher gate, write scope, or storage mutation fails.
    pub async fn mutate_identity_with_feedback(
        &self,
        site_config: Arc<dyn SiteConfigStorage>,
        passkeys: Arc<dyn PasskeyStorage>,
        mutation: SiteIdentityMutation,
    ) -> anyhow::Result<MutationOutcome<PublisherGeneration>> {
        let _gate = PublisherGateGuard::acquire(&self.storage_path).await?;
        let publisher = Arc::clone(&self.publisher);
        self.write_scope
            .run(move |transaction| {
                Box::pin(async move {
                    match mutation {
                        SiteIdentityMutation::SetTitle(title) => {
                            site_config
                                .set(transaction, SiteConfigKey::SiteTitle, &title)
                                .await?;
                        }
                        SiteIdentityMutation::SetTagline(tagline) => {
                            site_config
                                .set(
                                    transaction,
                                    SiteConfigKey::SiteTagline,
                                    tagline.as_ref().map_or("", AsRef::as_ref),
                                )
                                .await?;
                        }
                        SiteIdentityMutation::SetBaseUrl(base_url) => {
                            set_base_url_with_passkey_guard(
                                transaction,
                                site_config.as_ref(),
                                passkeys.as_ref(),
                                base_url,
                            )
                            .await?;
                        }
                        SiteIdentityMutation::UnsetTitle => {
                            site_config
                                .delete(transaction, SiteConfigKey::SiteTitle)
                                .await?;
                        }
                        SiteIdentityMutation::UnsetTagline => {
                            site_config
                                .delete(transaction, SiteConfigKey::SiteTagline)
                                .await?;
                        }
                        SiteIdentityMutation::UnsetBaseUrl => {
                            clear_base_url_with_passkey_guard(
                                transaction,
                                site_config.as_ref(),
                                passkeys.as_ref(),
                            )
                            .await?;
                        }
                    }
                    publisher
                        .invalidate_identity(transaction)
                        .await
                        .map_err(SiteIdentityMutationError::from)
                })
            })
            .await
            .map_err(Into::into)
    }

    /// Mutates the complete site identity under one publisher gate and write scope.
    ///
    /// # Errors
    ///
    /// Returns an error when the publisher gate, write scope, or aggregate mutation fails.
    pub async fn mutate_site_identity_with_feedback(
        &self,
        site_config: Arc<dyn SiteConfigStorage>,
        passkeys: Arc<dyn PasskeyStorage>,
        identity: SiteIdentity,
    ) -> anyhow::Result<MutationOutcome<()>> {
        let _gate = PublisherGateGuard::acquire(&self.storage_path).await?;
        let publisher = Arc::clone(&self.publisher);
        self.write_scope
            .run(move |transaction| {
                Box::pin(async move {
                    site_config
                        .set_identity(transaction, passkeys, publisher, &identity)
                        .await
                        .map_err(SiteIdentityMutationError::from)
                })
            })
            .await
            .map(|outcome| outcome.map(|()| ()))
            .map_err(Into::into)
    }

    /// Mutates the normalized hub under the same gate used by publication and
    /// preserves the write acknowledgement for operator feedback.
    ///
    /// # Errors
    ///
    /// Returns an error if the publisher gate cannot be acquired or the hub mutation
    /// transaction cannot be completed.
    pub async fn mutate_hub_with_feedback(
        &self,
        hub: Option<&HubUrl>,
    ) -> anyhow::Result<MutationOutcome<HubMutationOutcome>> {
        let _gate = PublisherGateGuard::acquire(&self.storage_path).await?;
        let publisher = Arc::clone(&self.publisher);
        let hub = hub.cloned();
        self.write_scope
            .run(move |transaction| {
                Box::pin(async move { publisher.mutate_hub(transaction, hub).await })
            })
            .await
            .map_err(Into::into)
    }

    /// Mutates one feed-window setting while preserving the durable commit
    /// acknowledgement for the CLI surface.
    ///
    /// # Errors
    ///
    /// Returns an error if the publisher gate cannot be acquired or the write
    /// scope cannot complete.
    pub async fn mutate_feed_window_with_feedback(
        &self,
        mutation: FeedWindowMutation,
    ) -> anyhow::Result<MutationOutcome<FeedWindowMutationOutcome>> {
        let _gate = PublisherGateGuard::acquire(&self.storage_path).await?;
        let publisher = Arc::clone(&self.publisher);
        self.write_scope
            .run(move |transaction| {
                Box::pin(async move { publisher.mutate_feed_window(transaction, mutation).await })
            })
            .await
            .map_err(Into::into)
    }

    /// Mutates the normalized hub under the same gate used by publication.
    ///
    /// # Errors
    ///
    /// Returns an error if the publisher gate cannot be acquired, the hub mutation
    /// transaction fails, or its commit acknowledgement is indeterminate.
    pub async fn mutate_hub(&self, hub: Option<&HubUrl>) -> anyhow::Result<HubMutationOutcome> {
        match self.mutate_hub_with_feedback(hub).await? {
            MutationOutcome::Confirmed(outcome) => Ok(outcome),
            MutationOutcome::CommitIndeterminate(_) => Err(anyhow::anyhow!(
                "hub mutation commit acknowledgement was indeterminate"
            )),
        }
    }
}
/// Server-side adapter for the web site's aggregate identity capability.
#[derive(Clone)]
pub struct SiteIdentityPublisherOperation {
    publisher: Arc<PublisherService>,
    site_config: Arc<dyn SiteConfigStorage>,
    passkeys: Arc<dyn PasskeyStorage>,
}

impl SiteIdentityPublisherOperation {
    #[must_use]
    pub fn new(
        publisher: Arc<PublisherService>,
        site_config: Arc<dyn SiteConfigStorage>,
        passkeys: Arc<dyn PasskeyStorage>,
    ) -> Self {
        Self {
            publisher,
            site_config,
            passkeys,
        }
    }
}

#[async_trait::async_trait]
impl web::site::SiteIdentityPublisher for SiteIdentityPublisherOperation {
    async fn mutate_identity(
        &self,
        identity: SiteIdentity,
    ) -> Result<MutationOutcome<()>, web::site::SiteIdentityPublisherError> {
        self.publisher
            .mutate_site_identity_with_feedback(
                Arc::clone(&self.site_config),
                Arc::clone(&self.passkeys),
                identity,
            )
            .await
            .map_err(|error| {
                web::site::SiteIdentityPublisherError::new(error.into_boxed_dyn_error())
            })
    }
}

#[async_trait::async_trait]
impl WebsubPublisher for PublisherService {
    async fn hub_url(&self) -> Result<Option<HubUrl>, WebsubPublisherError> {
        self.snapshot()
            .await
            .map(|snapshot| snapshot.feeds.websub_hub_url)
            .map_err(|error| WebsubPublisherError::new(error.into_boxed_dyn_error()))
    }

    async fn mutate_hub(
        &self,
        hub: Option<HubUrl>,
    ) -> Result<MutationOutcome<()>, WebsubPublisherError> {
        self.mutate_hub_with_feedback(hub.as_ref())
            .await
            .map(|outcome| outcome.map(|_| ()))
            .map_err(|error| WebsubPublisherError::new(error.into_boxed_dyn_error()))
    }
}

/// Held final cache-commit/publish region. Dropping releases the kernel lock.
pub struct PublisherFinalizationGuard {
    _gate: PublisherGateGuard,
    publisher: Arc<dyn PublisherStorage>,
    write_scope: WriteScope,
}

impl PublisherFinalizationGuard {
    /// Proves a publication-only attempt still holds the current hub generation.
    ///
    /// # Errors
    ///
    /// Returns an error if the generation cannot be read from publisher storage.
    pub async fn is_current(
        &self,
        generation: PublisherGeneration,
    ) -> Result<bool, PublisherStorageError> {
        self.publisher.is_current_generation(generation).await
    }

    /// Performs the brief transaction containing the generation fence and cache write.
    ///
    /// # Errors
    ///
    /// Returns an error if the transaction cannot begin or complete, or its commit
    /// acknowledgement is indeterminate.
    pub async fn commit_cache(
        &self,
        generation: PublisherGeneration,
        row: FeedCacheRow,
    ) -> Result<CacheCommitOutcome, PublisherStorageError> {
        let publisher = Arc::clone(&self.publisher);
        let committed = self
            .write_scope
            .run(move |transaction| {
                Box::pin(async move { publisher.commit_cache(transaction, generation, row).await })
            })
            .await
            .map_err(|error| match error {
                WriteScopeError::Operation(error) => error,
                WriteScopeError::Begin(error) => PublisherStorageError::Db(error),
            })?;
        match committed {
            MutationOutcome::Confirmed(outcome) => Ok(outcome),
            MutationOutcome::CommitIndeterminate(_) => Err(PublisherStorageError::Db(
                Error::Protocol("cache commit acknowledgement was indeterminate".to_owned()),
            )),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use common::{
        feed::FeedFormat,
        test_support::{parse_etag, parse_url},
        time::UtcInstant,
    };
    use host::config_key::SiteConfigKey;
    use host::feed::SyndicationFeedRepresentation;
    use rstest::*;
    use rstest_reuse::*;
    use sqlx::Error;
    use storage::{
        FeedCacheRow, FeedWindowMutation, MockPublisherStorage, PublisherStorageError,
        test_support::{Backend, backends, mock_write_scope_with_commit_acknowledgement_loss},
    };

    fn cache_row() -> FeedCacheRow {
        let now = UtcInstant::now();
        FeedCacheRow::new(
            "/feed.rss".parse().expect("valid feed path"),
            SyndicationFeedRepresentation::try_from_stored(
                FeedFormat::Rss,
                FeedFormat::Rss.content_type(),
                "<rss/>".to_owned(),
            )
            .expect("matching stored representation metadata"),
            parse_etag("\"etag\""),
            now,
            now,
            "0000000000000000000000000000000000000000000000000000000000000000"
                .parse()
                .expect("valid fingerprint"),
        )
        .expect("matching cache row formats")
    }

    #[tokio::test]
    async fn publisher_gate_reports_unusable_storage_path() {
        let file = tempfile::NamedTempFile::new().expect("temporary file");

        let error = PublisherGateGuard::acquire(file.path())
            .await
            .err()
            .expect("a file cannot become the gate directory");

        assert!(
            error
                .to_string()
                .contains("cannot create publisher gate directory")
        );
    }

    #[tokio::test]
    async fn websub_trait_maps_publisher_storage_errors() {
        let directory = tempfile::tempdir().expect("temporary storage directory");
        let mut publisher = MockPublisherStorage::new();
        publisher
            .expect_snapshot()
            .returning(|| Err(PublisherStorageError::Db(Error::PoolClosed)));
        let service = PublisherService::new(
            directory.path().to_owned(),
            Arc::new(publisher),
            storage::test_support::mock_write_scope(),
        );

        let error = WebsubPublisher::hub_url(&service)
            .await
            .expect_err("storage error crosses WebSub publisher seam");

        let source = std::error::Error::source(&error).expect("publisher error preserves source");
        assert!(source.to_string().contains("closed"));
    }

    #[apply(backends)]
    #[tokio::test]
    async fn websub_trait_preserves_confirmed_hub_mutation_outcome(#[case] backend: Backend) {
        let env = backend.setup().await;
        let publisher = env.publisher();
        let write_scope = env.write_scope();
        let directory = tempfile::tempdir().expect("temporary storage directory");
        let service = PublisherService::new(
            directory.path().to_owned(),
            Arc::clone(&publisher),
            write_scope,
        );

        let outcome =
            WebsubPublisher::mutate_hub(&service, Some(parse_url("https://example.com/hub")))
                .await
                .expect("confirmed mutation");

        assert!(matches!(outcome, MutationOutcome::Confirmed(())));
    }

    /// The service exposes the publisher mutation outcome directly so an
    /// acknowledgement loss cannot be mistaken for a confirmed CLI mutation.
    #[apply(backends)]
    #[tokio::test]
    async fn feed_window_service_preserves_the_publisher_mutation_outcome(
        #[case] backend: Backend,
    ) {
        let env = backend.setup().await;
        let publisher = env.publisher();
        let write_scope = env.write_scope();
        let directory = tempfile::tempdir().expect("temporary storage directory");
        let service = PublisherService::new(
            directory.path().to_owned(),
            Arc::clone(&publisher),
            write_scope,
        );

        let outcome = service
            .mutate_feed_window_with_feedback(FeedWindowMutation::SetMinItems(
                "42".parse().expect("valid feed minimum"),
            ))
            .await
            .expect("mutation outcome");
        assert!(matches!(
            outcome,
            MutationOutcome::Confirmed(FeedWindowMutationOutcome::Applied { .. })
        ));
    }

    #[apply(backends)]
    #[tokio::test]
    async fn mutation_rejects_indeterminate_commit_acknowledgements(#[case] backend: Backend) {
        let env = backend.setup().await;
        let generation = env
            .publisher()
            .snapshot()
            .await
            .expect("snapshot")
            .generation;
        let directory = tempfile::tempdir().expect("temporary storage directory");
        let mut publisher = MockPublisherStorage::new();
        publisher
            .expect_mutate_hub()
            .returning(move |_, _| Ok(HubMutationOutcome::Unchanged { generation }));
        let service = PublisherService::new(
            directory.path().to_owned(),
            Arc::new(publisher),
            mock_write_scope_with_commit_acknowledgement_loss(),
        );

        let error = service
            .mutate_hub(None)
            .await
            .expect_err("indeterminate acknowledgement is not a confirmed hub mutation");

        assert!(
            error
                .to_string()
                .contains("hub mutation commit acknowledgement was indeterminate")
        );
    }

    #[test]
    fn malformed_hub_repair_rejects_indeterminate_acknowledgements() {
        let error =
            require_confirmed_malformed_hub_repair(&MutationOutcome::CommitIndeterminate(()))
                .expect_err("indeterminate acknowledgement is not a repaired snapshot");

        assert_eq!(
            error.to_string(),
            "malformed hub repair commit acknowledgement was indeterminate"
        );
    }

    #[apply(backends)]
    #[tokio::test]
    async fn cache_commit_rejects_indeterminate_acknowledgements(#[case] backend: Backend) {
        let env = backend.setup().await;
        let generation = env
            .publisher()
            .snapshot()
            .await
            .expect("snapshot")
            .generation;
        let directory = tempfile::tempdir().expect("temporary storage directory");
        let mut publisher = MockPublisherStorage::new();
        publisher
            .expect_commit_cache()
            .returning(|_, _, _| Ok(CacheCommitOutcome::StaleGeneration));
        let service = PublisherService::new(
            directory.path().to_owned(),
            Arc::new(publisher),
            mock_write_scope_with_commit_acknowledgement_loss(),
        );

        let error = service
            .finalization_guard()
            .await
            .expect("gate acquired")
            .commit_cache(generation, cache_row())
            .await
            .expect_err("indeterminate acknowledgement is not a committed cache row");

        assert!(matches!(
            error,
            PublisherStorageError::Db(Error::Protocol(message))
                if message == "cache commit acknowledgement was indeterminate"
        ));
    }

    #[tokio::test]
    async fn websub_trait_maps_hub_mutation_errors() {
        let directory = tempfile::tempdir().expect("temporary storage directory");
        let mut publisher = MockPublisherStorage::new();
        publisher
            .expect_mutate_hub()
            .returning(|_, _| Err(PublisherStorageError::Db(Error::PoolClosed)));
        let service = PublisherService::new(
            directory.path().to_owned(),
            Arc::new(publisher),
            storage::test_support::mock_write_scope(),
        );

        let error = WebsubPublisher::mutate_hub(&service, None)
            .await
            .expect_err("storage error crosses WebSub publisher seam");

        let source = std::error::Error::source(&error).expect("publisher error preserves source");
        assert!(source.to_string().contains("closed"));
    }

    #[apply(backends)]
    #[tokio::test]
    async fn finalization_commit_maps_operation_errors(#[case] backend: Backend) {
        let env = backend.setup().await;
        let generation = env
            .publisher()
            .snapshot()
            .await
            .expect("snapshot")
            .generation;
        let directory = tempfile::tempdir().expect("temporary storage directory");
        let mut publisher = MockPublisherStorage::new();
        publisher
            .expect_commit_cache()
            .returning(|_, _, _| Err(PublisherStorageError::Db(Error::PoolClosed)));
        let service = PublisherService::new(
            directory.path().to_owned(),
            Arc::new(publisher),
            storage::test_support::mock_write_scope(),
        );

        let error = service
            .finalization_guard()
            .await
            .expect("gate acquired")
            .commit_cache(generation, cache_row())
            .await
            .expect_err("operation error");

        assert!(matches!(
            error,
            PublisherStorageError::Db(Error::PoolClosed)
        ));
    }

    #[apply(backends)]
    #[tokio::test]
    async fn finalization_commit_maps_begin_errors(#[case] backend: Backend) {
        let env = backend.setup().await;
        let publisher = env.publisher();
        let generation = publisher.snapshot().await.expect("snapshot").generation;
        let write_scope = env.write_scope();
        let directory = tempfile::tempdir().expect("temporary storage directory");
        let service = PublisherService::new(directory.path().to_owned(), publisher, write_scope);
        let guard = service.finalization_guard().await.expect("gate acquired");
        env.base.close_pool().await;

        let error = guard
            .commit_cache(generation, cache_row())
            .await
            .expect_err("closed pool prevents beginning write scope");

        assert!(matches!(error, PublisherStorageError::Db(_)));
    }

    #[tokio::test]
    async fn publisher_gate_waits_for_prior_finalization_region() {
        let directory = tempfile::tempdir().expect("temporary storage directory");
        let first = PublisherGateGuard::acquire(directory.path())
            .await
            .expect("first gate");
        let path = directory.path().to_owned();
        let second = tokio::spawn(async move {
            PublisherGateGuard::acquire(&path)
                .await
                .expect("second gate")
        });

        tokio::task::yield_now().await;
        assert!(
            !second.is_finished(),
            "second acquirer must wait for the first"
        );
        drop(first);
        let _second = second.await.expect("gate task joins");
    }

    #[apply(backends)]
    #[tokio::test]
    async fn aggregate_identity_operation_writes_one_fenced_snapshot(#[case] backend: Backend) {
        let env = backend.setup().await;
        let operation = SiteIdentityPublisherOperation::new(
            Arc::new(PublisherService::new(
                env.base.path().to_path_buf(),
                env.publisher(),
                env.write_scope(),
            )),
            env.site_config(),
            env.passkeys(),
        );
        let before = env.publisher().snapshot().await.unwrap().generation;
        let identity = SiteIdentity {
            title: "Configured".parse().unwrap(),
            tagline: Some("A tagline".parse().unwrap()),
            base_url: Some(parse_url("https://identity.example/")),
        };

        assert!(matches!(
            web::site::SiteIdentityPublisher::mutate_identity(&operation, identity.clone())
                .await
                .unwrap(),
            MutationOutcome::Confirmed(())
        ));
        let snapshot = env.publisher().snapshot().await.unwrap();
        assert_eq!(snapshot.identity, identity);
        assert!(snapshot.generation > before);
    }

    #[apply(backends)]
    #[tokio::test]
    async fn snapshot_repairs_malformed_hub_before_exposure(#[case] backend: Backend) {
        let env = backend.setup().await;
        env.inject_invalid_site_config(SiteConfigKey::FeedsWebsubHubUrl, "malformed")
            .await
            .expect("seed malformed hub");
        let publisher = env.publisher();
        let site_config = env.site_config();
        let before = publisher.snapshot().await.unwrap().generation;
        let write_scope = env.write_scope();
        let directory = tempfile::tempdir().expect("temporary storage directory");
        let service = PublisherService::new(
            directory.path().to_owned(),
            Arc::clone(&publisher),
            write_scope,
        );

        let snapshot = service.snapshot().await.expect("repairing snapshot");

        assert_eq!(snapshot.feeds.websub_hub_url, None);
        assert!(snapshot.malformed_hub().is_none());
        assert!(snapshot.generation > before);
        assert_eq!(
            site_config
                .get_raw(SiteConfigKey::FeedsWebsubHubUrl)
                .await
                .unwrap(),
            None
        );
    }
}
