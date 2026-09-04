//! Cross-process serialization for publisher finalization and hub mutations.

use std::fs::{self, File, OpenOptions, TryLockError};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use anyhow::Context;
use common::MutationOutcome;
use common::tagged_url::HubUrl;
use sqlx::Error;
use storage::{
    CacheCommitOutcome, FeedCacheRow, FeedWindowMutation, FeedWindowMutationOutcome,
    HubMutationOutcome, PublisherGeneration, PublisherSnapshot, PublisherStorage,
    PublisherStorageError, WriteScope, WriteScopeError,
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
                // cov:ignore-start -- advisory-lock backends expose no deterministic
                // way to make a successfully opened regular file return this OS error.
                Err(TryLockError::Error(error)) => {
                    return Err(error).with_context(|| {
                        format!("cannot acquire publisher gate {}", path.display())
                    });
                } // cov:ignore-stop
            }
        }
    }
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
        // cov:ignore-start -- downstream test scopes deliberately expose only confirmed
        // commits or operation/begin failures, not post-commit acknowledgement loss.
        if matches!(committed, MutationOutcome::CommitIndeterminate(_)) {
            return Err(anyhow::anyhow!(
                "malformed hub repair commit acknowledgement was indeterminate"
            ));
        }
        // cov:ignore-stop
        Ok(self.publisher.snapshot().await?)
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
            // cov:ignore-start -- downstream test scopes cannot synthesize a
            // post-commit acknowledgement loss; the write-scope crate owns that fault.
            MutationOutcome::CommitIndeterminate(_) => Err(anyhow::anyhow!(
                "hub mutation commit acknowledgement was indeterminate"
            )),
            // cov:ignore-stop
        }
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
            // cov:ignore-start — write scopes expose operation/begin failures but
            // cannot synthesize post-commit acknowledgement loss.
            MutationOutcome::CommitIndeterminate(_) => Err(PublisherStorageError::Db(
                Error::Protocol("cache commit acknowledgement was indeterminate".to_owned()),
            )),
            // cov:ignore-stop
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
        test_support::{Backend, backends, inject_invalid_site_config},
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
        let directory = tempfile::tempdir().expect("temporary storage directory");
        let service = PublisherService::new(
            directory.path().to_owned(),
            Arc::clone(&env.state.publisher),
            env.state.write_scope.clone(),
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
        let directory = tempfile::tempdir().expect("temporary storage directory");
        let service = PublisherService::new(
            directory.path().to_owned(),
            Arc::clone(&env.state.publisher),
            env.state.write_scope.clone(),
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
            .state
            .publisher
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
        let generation = env
            .state
            .publisher
            .snapshot()
            .await
            .expect("snapshot")
            .generation;
        let directory = tempfile::tempdir().expect("temporary storage directory");
        let service = PublisherService::new(
            directory.path().to_owned(),
            Arc::clone(&env.state.publisher),
            env.state.write_scope.clone(),
        );
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
    async fn snapshot_repairs_malformed_hub_before_exposure(#[case] backend: Backend) {
        let env = backend.setup().await;
        inject_invalid_site_config(&env, SiteConfigKey::FeedsWebsubHubUrl, "malformed")
            .await
            .expect("seed malformed hub");
        let before = env.state.publisher.snapshot().await.unwrap().generation;
        let directory = tempfile::tempdir().expect("temporary storage directory");
        let service = PublisherService::new(
            directory.path().to_owned(),
            Arc::clone(&env.state.publisher),
            env.state.write_scope.clone(),
        );

        let snapshot = service.snapshot().await.expect("repairing snapshot");

        assert_eq!(snapshot.feeds.websub_hub_url, None);
        assert!(snapshot.malformed_hub().is_none());
        assert!(snapshot.generation > before);
        assert_eq!(
            env.state
                .site_config
                .get_raw(SiteConfigKey::FeedsWebsubHubUrl)
                .await
                .unwrap(),
            None
        );
    }
}
