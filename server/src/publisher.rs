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
    CacheCommitOutcome, FeedCacheRow, HubMutationOutcome, PublisherGeneration, PublisherSnapshot,
    PublisherStorage, PublisherStorageError, WriteScope, WriteScopeError,
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
                Err(TryLockError::Error(error)) => {
                    return Err(error).with_context(|| {
                        format!("cannot acquire publisher gate {}", path.display())
                    });
                }
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
        if matches!(committed, MutationOutcome::CommitIndeterminate(_)) {
            return Err(anyhow::anyhow!(
                "malformed hub repair commit acknowledgement was indeterminate"
            ));
        }
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
    use host::config_key::SiteConfigKey;
    use rstest::*;
    use rstest_reuse::*;
    use storage::test_support::{Backend, backends, inject_invalid_site_config};

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
