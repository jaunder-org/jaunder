//! Immutable filesystem lifecycle for compiled public Theme content.
//!
//! The package compiler is the only producer of [`CompiledThemeRevision`]. This
//! service materializes those exact bytes before making their digest eligible for
//! public serving, and keeps filesystem cleanup outside short database writes.

use std::{collections::BTreeSet, fmt::Write as _, io, path::PathBuf, sync::Arc};

use common::{
    MutationOutcome,
    ids::ThemeId,
    theme::{ThemeAssetDigest, ThemeContentDigest, ThemeRevisionDigest, ThemeStylesheetDigest},
};
use host::theme_package::CompiledThemeRevision;
use sha2::{Digest, Sha256};
use thiserror::Error;
use tokio::{fs, io::AsyncWriteExt};

use crate::{
    ThemeContentCharge, ThemeContentEligibility, ThemeOwner, ThemePackageAsset,
    ThemePublicationAdmission, ThemeQuotaLimits, ThemeRevision, ThemeStorage, WriteScope,
    WriteScopeError,
};

/// The immutable-cache lifetime plus the public document freshness allowance.
pub const THEME_CONTENT_RETENTION_SECONDS: i64 = 31_536_300;

/// A durable immutable-content publisher and collector.
pub struct ThemeAssetManager {
    themes: Arc<dyn ThemeStorage>,
    write_scope: WriteScope,
    storage_path: Arc<PathBuf>,
    #[cfg(test)]
    fail_unlink_for_test: bool,
}

/// Result of checking the durable content set during process startup.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct ThemeContentReconciliation {
    /// Immutable content files which did not have a serving/retention row.
    pub orphan_files: usize,
    /// Orphan immutable content files successfully reclaimed during reconciliation.
    pub reclaimed_files: usize,
}

/// A publication error whose cleanup semantics remain visible to the caller.
#[derive(Debug, Error)]
pub enum ThemeAssetError {
    #[error("theme content filesystem operation failed: {0}")]
    Filesystem(#[from] io::Error),
    #[error("theme storage operation failed: {0}")]
    Storage(#[from] sqlx::Error),
    #[error("compiler produced an invalid content digest")]
    InvalidDigest,
    #[error("theme content path has no parent directory")]
    InvalidContentPath,
    #[error("eligible theme content is missing or corrupt: {0}")]
    IneligibleContent(ThemeContentDigest),
}

#[derive(Clone)]
struct Blob<'a> {
    digest: ThemeContentDigest,
    mime: &'a str,
    bytes: &'a [u8],
}

impl ThemeAssetManager {
    #[must_use]
    pub fn new(
        themes: Arc<dyn ThemeStorage>,
        write_scope: WriteScope,
        storage_path: Arc<PathBuf>,
    ) -> Self {
        Self {
            themes,
            write_scope,
            storage_path,
            #[cfg(test)]
            fail_unlink_for_test: false,
        }
    }

    #[cfg(test)]
    fn with_unlink_failure_for_test(mut self) -> Self {
        self.fail_unlink_for_test = true;
        self
    }

    /// Reconciles the filesystem with durable serving eligibility before public
    /// content can be served. A referenced blob must be present and exact;
    /// unreferenced complete content paths are reclaimable and are retried here
    /// after a prior unlink failure or commit-indeterminate publication.
    ///
    /// # Errors
    ///
    /// Returns an error if storage lookup, filesystem inspection, or orphan reclamation fails.
    pub async fn reconcile_startup(&self) -> Result<ThemeContentReconciliation, ThemeAssetError> {
        // Discovery is durable: owner charges left after a restart are collected
        // through the same eligibility-first, lock-held detachment path. Do this
        // before checking bytes, because an expired zero-reference row can safely
        // be detached even when its now-unservable file is missing or corrupt.
        let now_unix_seconds = chrono::Utc::now().timestamp();
        for (owner, digest) in self
            .themes
            .expired_retained_content(now_unix_seconds)
            .await?
        {
            let _ = self.collect(owner, &digest, now_unix_seconds).await;
        }

        // Reload after collection: only content that remains eligible may block
        // serving startup when its immutable bytes are absent or corrupt.
        let known = self
            .themes
            .list_content_eligibility()
            .await?
            .into_iter()
            .map(|eligibility| eligibility.digest)
            .collect::<BTreeSet<_>>();
        for digest in &known {
            let bytes = fs::read(self.content_path(digest.as_ref()))
                .await
                .map_err(|_| ThemeAssetError::IneligibleContent(digest.clone()))?;
            Self::verify(&bytes, digest)
                .map_err(|_| ThemeAssetError::IneligibleContent(digest.clone()))?;
        }

        self.reclaim_orphan_content().await
    }

    async fn reclaim_orphan_content(&self) -> Result<ThemeContentReconciliation, ThemeAssetError> {
        self.reclaim_staging().await?;
        let known = self
            .themes
            .list_content_eligibility()
            .await?
            .into_iter()
            .map(|eligibility| eligibility.digest)
            .collect::<BTreeSet<_>>();
        let mut reconciliation = ThemeContentReconciliation::default();
        for digest in self.enumerate_content_digests().await? {
            if known.contains(&digest) {
                continue;
            }
            reconciliation.orphan_files += 1;
            let _locks = self.acquire_locks([&digest]).await?;
            if self.themes.content_eligibility(&digest).await?.is_none()
                && matches!(self.unlink_if_present(&digest).await, Ok(true))
            {
                reconciliation.reclaimed_files += 1;
            }
        }
        Ok(reconciliation)
    }

    /// Publishes compiler-minted content. Files are installed under sorted digest
    /// locks before the sole database write. A transaction failure has a known
    /// rollback, so only files installed by this call are removed; an uncertain
    /// commit intentionally leaves them for startup reconciliation.
    ///
    /// # Errors
    ///
    /// Returns an error if compiler output is invalid, immutable content cannot be
    /// materialized, or the atomic storage admission fails.
    pub async fn publish(
        &self,
        owner: ThemeOwner,
        theme_id: ThemeId,
        compiled: &CompiledThemeRevision,
        limits: ThemeQuotaLimits,
        now_unix_seconds: i64,
    ) -> Result<MutationOutcome<ThemeRevision>, ThemeAssetError> {
        let blobs = Self::blobs(compiled)?;
        let _locks = self
            .acquire_locks(blobs.iter().map(|blob| &blob.digest))
            .await?;
        let mut installed = Vec::new();
        for blob in &blobs {
            if self.install(blob).await? {
                installed.push(blob.digest.clone());
            }
        }

        let revision = Self::revision(theme_id, compiled)?;
        let assets = Self::assets(compiled)?;
        let charges = Self::charges(&blobs)?;
        let eligibilities = blobs
            .iter()
            .map(|blob| ThemeContentEligibility {
                digest: blob.digest.clone(),
                mime: blob.mime.to_owned(),
                retained_until_unix_seconds: now_unix_seconds,
            })
            .collect::<Vec<_>>();
        let themes = Arc::clone(&self.themes);
        let outcome = self
            .write_scope
            .run(move |transaction| {
                Box::pin(async move {
                    themes
                        .admit_publication(
                            transaction,
                            ThemePublicationAdmission {
                                owner,
                                limits,
                                revision: &revision,
                                assets: &assets,
                                eligibilities: &eligibilities,
                                charges: &charges,
                            },
                        )
                        .await?;
                    Ok(revision)
                })
            })
            .await;

        match outcome {
            Ok(outcome) => Ok(outcome),
            Err(WriteScopeError::Operation(error) | WriteScopeError::Begin(error)) => {
                self.cleanup_newly_installed(&installed).await;
                Err(ThemeAssetError::Storage(error))
            }
        }
    }

    /// Detaches database eligibility and quota accounting before unlinking an
    /// expired, unreferenced blob. Failed unlinks are deliberately recoverable:
    /// the detached file is an orphan that reconciliation retries.
    ///
    /// # Errors
    ///
    /// Returns an error if locking or the storage detachment operation fails.
    pub async fn collect(
        &self,
        owner: ThemeOwner,
        digest: &ThemeContentDigest,
        now_unix_seconds: i64,
    ) -> Result<MutationOutcome<()>, ThemeAssetError> {
        let _locks = self.acquire_locks([digest]).await?;
        let transaction_digest = digest.clone();
        let themes = Arc::clone(&self.themes);
        let outcome = self
            .write_scope
            .run(move |transaction| {
                Box::pin(async move {
                    themes
                        .collect_retained_content(
                            transaction,
                            owner,
                            &transaction_digest,
                            now_unix_seconds,
                        )
                        .await
                })
            })
            .await
            .map_err(|error| match error {
                WriteScopeError::Operation(error) | WriteScopeError::Begin(error) => {
                    ThemeAssetError::Storage(error)
                }
            })?;
        if matches!(outcome, MutationOutcome::Confirmed(())) {
            let _ = self.unlink_if_present(digest).await;
        }
        Ok(outcome)
    }

    fn blobs(compiled: &CompiledThemeRevision) -> Result<Vec<Blob<'_>>, ThemeAssetError> {
        let mut blobs = Vec::new();
        let css_digest = Self::digest(compiled.css().digest())?;
        blobs.push(Blob {
            digest: css_digest,
            mime: "text/css; charset=utf-8",
            bytes: compiled.css().bytes(),
        });
        for (_, mime, bytes, digest) in compiled.assets() {
            blobs.push(Blob {
                digest: Self::digest(digest)?,
                mime,
                bytes,
            });
        }
        blobs.sort_by(|left, right| left.digest.as_ref().cmp(right.digest.as_ref()));
        blobs.dedup_by(|left, right| left.digest == right.digest);
        Ok(blobs)
    }

    fn revision(
        theme_id: ThemeId,
        compiled: &CompiledThemeRevision,
    ) -> Result<ThemeRevision, ThemeAssetError> {
        Ok(ThemeRevision {
            theme_id,
            digest: Self::revision_digest(compiled.revision_digest())?,
            stylesheet_digest: Self::stylesheet_digest(compiled.css().digest())?,
            manifest: compiled.canonical_manifest().to_vec(),
        })
    }

    fn assets(compiled: &CompiledThemeRevision) -> Result<Vec<ThemePackageAsset>, ThemeAssetError> {
        compiled
            .assets()
            .map(|(path, mime, _, digest)| {
                Ok(ThemePackageAsset {
                    path: path.to_owned(),
                    digest: Self::asset_digest(digest)?,
                    mime: mime.to_owned(),
                })
            })
            .collect()
    }

    fn charges(blobs: &[Blob<'_>]) -> Result<Vec<ThemeContentCharge>, ThemeAssetError> {
        blobs
            .iter()
            .map(|blob| {
                Ok(ThemeContentCharge {
                    digest: blob.digest.clone(),
                    logical_bytes: i64::try_from(blob.bytes.len())
                        .map_err(|_| ThemeAssetError::InvalidDigest)?,
                    physical_bytes: i64::try_from(blob.bytes.len())
                        .map_err(|_| ThemeAssetError::InvalidDigest)?,
                })
            })
            .collect()
    }

    async fn acquire_locks<'a>(
        &self,
        digests: impl IntoIterator<Item = &'a ThemeContentDigest>,
    ) -> Result<Vec<std::fs::File>, ThemeAssetError> {
        let lock_dir = self.storage_path.join("themes").join(".locks");
        fs::create_dir_all(&lock_dir).await?;
        self.sync_content_directories(&lock_dir).await?;
        let mut paths = digests
            .into_iter()
            .map(|digest| lock_dir.join(format!("{digest}.lock")))
            .collect::<Vec<_>>();
        paths.sort();
        paths.dedup();
        tokio::task::spawn_blocking(move || {
            paths
                .into_iter()
                .map(|path| {
                    let file = std::fs::OpenOptions::new()
                        .create(true)
                        .append(true)
                        .open(path)?;
                    file.lock()?;
                    Ok(file)
                })
                .collect::<io::Result<Vec<_>>>()
        })
        .await
        .map_err(io::Error::other)?
        .map_err(ThemeAssetError::Filesystem)
    }

    async fn install(&self, blob: &Blob<'_>) -> Result<bool, ThemeAssetError> {
        let path = self.content_path(blob.digest.as_ref());
        match fs::read(&path).await {
            Ok(existing) => return Self::verify(&existing, &blob.digest).map(|()| false),
            Err(error) if error.kind() == io::ErrorKind::NotFound => {}
            Err(error) => return Err(error.into()),
        }
        let parent = path.parent().ok_or(ThemeAssetError::InvalidContentPath)?;
        fs::create_dir_all(parent).await?;
        self.sync_content_directories(parent).await?;
        let staging = self.storage_path.join("themes").join(".staging");
        fs::create_dir_all(&staging).await?;
        self.sync_content_directories(&staging).await?;
        let temporary = staging.join(format!("{}.tmp", uuid::Uuid::new_v4()));
        let mut file = fs::OpenOptions::new()
            .create_new(true)
            .write(true)
            .open(&temporary)
            .await?;
        file.write_all(blob.bytes).await?;
        file.sync_all().await?;
        drop(file);
        match fs::rename(&temporary, &path).await {
            Ok(()) => {
                Self::sync_directory(parent.to_path_buf()).await?;
                Self::sync_directory(staging).await?;
                Ok(true)
            }
            Err(error) if error.kind() == io::ErrorKind::AlreadyExists => {
                let existing = fs::read(&path).await?;
                fs::remove_file(&temporary).await?;
                Self::sync_directory(staging).await?;
                Self::verify(&existing, &blob.digest).map(|()| false)
            }
            Err(error) => {
                let _ = fs::remove_file(&temporary).await;
                Err(error.into())
            }
        }
    }

    async fn sync_directory(path: PathBuf) -> Result<(), ThemeAssetError> {
        tokio::task::spawn_blocking(move || std::fs::File::open(path)?.sync_all())
            .await
            .map_err(io::Error::other)?
            .map_err(ThemeAssetError::Filesystem)
    }

    async fn sync_content_directories(
        &self,
        directory: &std::path::Path,
    ) -> Result<(), ThemeAssetError> {
        let mut current = Some(directory);
        while let Some(path) = current {
            Self::sync_directory(path.to_path_buf()).await?;
            if path == self.storage_path.as_path() {
                break;
            }
            current = path.parent();
        }
        Ok(())
    }

    async fn reclaim_staging(&self) -> Result<(), ThemeAssetError> {
        let staging = self.storage_path.join("themes").join(".staging");
        let mut entries = match fs::read_dir(&staging).await {
            Ok(entries) => entries,
            Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(()),
            Err(error) => return Err(error.into()),
        };
        while let Some(entry) = entries.next_entry().await? {
            if entry.file_type().await?.is_file() {
                fs::remove_file(entry.path()).await?;
            }
        }
        Self::sync_directory(staging).await
    }
    async fn cleanup_newly_installed(&self, digests: &[ThemeContentDigest]) {
        for digest in digests {
            if self
                .themes
                .content_eligibility(digest)
                .await
                .ok()
                .flatten()
                .is_none()
            {
                let _ = fs::remove_file(self.content_path(digest.as_ref())).await;
            }
        }
    }

    /// Enumerates only complete, canonical content-address paths. Lock and
    /// temporary directories are deliberately outside this namespace.
    async fn enumerate_content_digests(&self) -> Result<Vec<ThemeContentDigest>, ThemeAssetError> {
        let root = self.storage_path.join("themes");
        let mut digests = Vec::new();
        let mut first = match fs::read_dir(&root).await {
            Ok(entries) => entries,
            Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(digests),
            Err(error) => return Err(error.into()),
        };
        while let Some(prefix_entry) = first.next_entry().await? {
            let prefix_name = prefix_entry.file_name();
            let prefix = prefix_name.to_string_lossy();
            if !Self::is_hex_component(&prefix, 2) {
                continue;
            }
            let mut second = match fs::read_dir(prefix_entry.path()).await {
                Ok(entries) => entries,
                Err(error) if error.kind() == io::ErrorKind::NotADirectory => continue,
                Err(error) => return Err(error.into()),
            };
            while let Some(shard_entry) = second.next_entry().await? {
                let shard_name = shard_entry.file_name();
                let shard = shard_name.to_string_lossy();
                if !Self::is_hex_component(&shard, 2) {
                    continue;
                }
                let mut files = match fs::read_dir(shard_entry.path()).await {
                    Ok(entries) => entries,
                    Err(error) if error.kind() == io::ErrorKind::NotADirectory => continue,
                    Err(error) => return Err(error.into()),
                };
                while let Some(file) = files.next_entry().await? {
                    if !file.file_type().await?.is_file() {
                        continue;
                    }
                    let name = file.file_name();
                    let digest = name.to_string_lossy();
                    if Self::is_canonical_digest_path(&prefix, &shard, &digest) {
                        digests.push(digest.parse().map_err(|_| ThemeAssetError::InvalidDigest)?);
                    }
                }
            }
        }
        digests.sort_by(|left, right| left.as_ref().cmp(right.as_ref()));
        digests.dedup();
        Ok(digests)
    }

    async fn unlink_if_present(
        &self,
        digest: &ThemeContentDigest,
    ) -> Result<bool, ThemeAssetError> {
        #[cfg(test)]
        if self.fail_unlink_for_test {
            return Err(
                io::Error::new(io::ErrorKind::PermissionDenied, "injected unlink failure").into(),
            );
        }
        match fs::remove_file(self.content_path(digest.as_ref())).await {
            Ok(()) => Ok(true),
            Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(false),
            Err(error) => Err(error.into()),
        }
    }

    fn is_hex_component(value: &str, length: usize) -> bool {
        value.len() == length
            && value
                .bytes()
                .all(|byte| byte.is_ascii_digit() || byte.is_ascii_lowercase())
    }

    fn is_canonical_digest_path(first: &str, second: &str, digest: &str) -> bool {
        Self::is_hex_component(digest, 64)
            && digest.starts_with(first)
            && digest[2..].starts_with(second)
    }

    fn content_path(&self, digest: &str) -> PathBuf {
        self.storage_path
            .join("themes")
            .join(&digest[..2])
            .join(&digest[2..4])
            .join(digest)
    }

    fn verify(bytes: &[u8], digest: &ThemeContentDigest) -> Result<(), ThemeAssetError> {
        let actual = Sha256::digest(bytes);
        if Self::hex(&actual) == digest.as_ref() {
            Ok(())
        } else {
            Err(ThemeAssetError::InvalidDigest)
        }
    }

    fn hex(bytes: &[u8]) -> String {
        let mut output = String::with_capacity(bytes.len() * 2);
        for byte in bytes {
            let _ = write!(output, "{byte:02x}");
        }
        output
    }
    fn digest(bytes: [u8; 32]) -> Result<ThemeContentDigest, ThemeAssetError> {
        Self::hex(&bytes)
            .parse()
            .map_err(|_| ThemeAssetError::InvalidDigest)
    }
    fn stylesheet_digest(bytes: [u8; 32]) -> Result<ThemeStylesheetDigest, ThemeAssetError> {
        Self::hex(&bytes)
            .parse()
            .map_err(|_| ThemeAssetError::InvalidDigest)
    }
    fn asset_digest(bytes: [u8; 32]) -> Result<ThemeAssetDigest, ThemeAssetError> {
        Self::hex(&bytes)
            .parse()
            .map_err(|_| ThemeAssetError::InvalidDigest)
    }
    fn revision_digest(bytes: [u8; 32]) -> Result<ThemeRevisionDigest, ThemeAssetError> {
        Self::hex(&bytes)
            .parse()
            .map_err(|_| ThemeAssetError::InvalidDigest)
    }
}

#[cfg(test)]
mod tests {
    use std::{fmt::Write as _, fs, sync::Arc};

    use common::{MutationOutcome, ids::ThemeId, theme::ThemeContentDigest};
    use rstest::*;
    use rstest_reuse::*;
    use sha2::{Digest, Sha256};
    use tempfile::TempDir;

    use super::ThemeAssetManager;
    use crate::{
        MockThemeStorage, ThemeContentCharge, ThemeContentEligibility, ThemeOwner,
        ThemeQuotaLimits,
        test_support::{
            Backend, backends, compiled_theme_fixture as compiled, confirmed,
            create_site_theme as create_theme, mock_write_scope,
            mock_write_scope_with_commit_acknowledgement_loss, theme_quota_limits as limits,
        },
    };
    #[test]
    fn complete_content_path_requires_matching_lowercase_digest_shards() {
        let digest = "ab12aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
        assert!(ThemeAssetManager::is_canonical_digest_path(
            "ab", "12", digest
        ));
        assert!(!ThemeAssetManager::is_canonical_digest_path(
            "ab", "13", digest
        ));
        assert!(!ThemeAssetManager::is_canonical_digest_path(
            "AB", "12", digest
        ));
    }

    #[test]
    fn incomplete_or_non_hex_content_paths_are_not_reconciliation_candidates() {
        assert!(!ThemeAssetManager::is_canonical_digest_path(
            "aa", "bb", "aabb"
        ));
        assert!(!ThemeAssetManager::is_canonical_digest_path(
            "aa",
            "bb",
            "aabbzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzz",
        ));
    }

    fn digest(bytes: &[u8]) -> ThemeContentDigest {
        let digest = Sha256::digest(bytes);
        let mut hex = String::with_capacity(digest.len() * 2);
        for byte in digest {
            let _ = write!(hex, "{byte:02x}");
        }
        hex.parse().expect("SHA-256 is a valid theme digest")
    }

    #[apply(backends)]
    #[tokio::test]
    async fn publication_is_durable_and_identical_republish_is_counter_idempotent(
        #[case] backend: Backend,
    ) {
        let env = backend.setup().await;
        let compiled = compiled();
        let bytes = compiled
            .css()
            .bytes()
            .len()
            .checked_add(compiled.assets().map(|(_, _, bytes, _)| bytes.len()).sum())
            .and_then(|bytes| i64::try_from(bytes).ok())
            .expect("fixture bytes fit");
        let theme_id = create_theme(
            Arc::clone(&env.state.themes),
            env.state.write_scope.clone(),
            &compiled,
        )
        .await;
        let manager = ThemeAssetManager::new(
            Arc::clone(&env.state.themes),
            env.state.write_scope.clone(),
            Arc::new(env.base.path().to_path_buf()),
        );

        let outcome = manager
            .publish(ThemeOwner::Site, theme_id, &compiled, limits(bytes), 100)
            .await
            .expect("publish");

        assert!(matches!(outcome, MutationOutcome::Confirmed(_)));
        let replay = manager
            .publish(ThemeOwner::Site, theme_id, &compiled, limits(bytes), 100)
            .await
            .expect("idempotent republish");
        assert!(matches!(replay, MutationOutcome::Confirmed(_)));
        let digest = digest(compiled.css().bytes());
        assert!(manager.content_path(digest.as_ref()).exists());
        assert!(
            env.state
                .themes
                .content_eligibility(&digest)
                .await
                .expect("read eligibility")
                .is_some()
        );
        assert_eq!(
            env.state
                .themes
                .list_revisions(ThemeOwner::Site, theme_id)
                .await
                .expect("read revisions")
                .len(),
            1
        );
        assert_eq!(
            env.state
                .themes
                .owner_quota(ThemeOwner::Site)
                .await
                .expect("read owner quota")
                .expect("owner quota exists")
                .active_themes,
            1
        );
    }

    // guard:no-backend — filesystem materialization fails before any database operation
    #[tokio::test]
    async fn materialize_failure_precedes_database_visibility() {
        let fixture = TempDir::new().expect("create fixture");
        let compiled = compiled();
        let digest = digest(compiled.css().bytes());
        let path = fixture
            .path()
            .join("themes")
            .join(&digest.as_ref()[..2])
            .join(&digest.as_ref()[2..4])
            .join(digest.as_ref());
        fs::create_dir_all(&path).expect("make target a directory");
        let mut storage = MockThemeStorage::new();
        storage.expect_content_eligibility().never();
        storage.expect_admit_publication().never();
        let manager = ThemeAssetManager::new(
            Arc::new(storage),
            mock_write_scope(),
            Arc::new(fixture.path().to_path_buf()),
        );

        assert!(
            manager
                .publish(
                    ThemeOwner::Site,
                    ThemeId::from(1),
                    &compiled,
                    limits(i64::MAX),
                    0,
                )
                .await
                .is_err()
        );
    }

    // guard:no-backend — mocked storage isolates confirmed transaction rollback cleanup
    #[tokio::test]
    async fn transaction_rollback_removes_only_newly_installed_unreferenced_blobs_after_recheck() {
        let fixture = TempDir::new().expect("create fixture");
        let compiled = compiled();
        let digest = digest(compiled.css().bytes());
        let mut storage = MockThemeStorage::new();
        storage
            .expect_admit_publication()
            .once()
            .returning(|_, _| Err(sqlx::Error::RowNotFound));
        storage
            .expect_content_eligibility()
            .times(2)
            .returning(|_| Ok(None));
        let manager = ThemeAssetManager::new(
            Arc::new(storage),
            mock_write_scope(),
            Arc::new(fixture.path().to_path_buf()),
        );

        assert!(
            manager
                .publish(
                    ThemeOwner::Site,
                    ThemeId::from(1),
                    &compiled,
                    limits(i64::MAX),
                    0,
                )
                .await
                .is_err()
        );
        assert!(!manager.content_path(digest.as_ref()).exists());
    }

    // guard:no-backend — injected commit acknowledgement loss is backend-independent
    #[tokio::test]
    async fn commit_indeterminate_retains_installed_blobs_for_restart_reconciliation() {
        let fixture = TempDir::new().expect("create fixture");
        let compiled = compiled();
        let digest = digest(compiled.css().bytes());
        let mut storage = MockThemeStorage::new();
        storage
            .expect_admit_publication()
            .once()
            .returning(|_, _| Ok(()));
        let manager = ThemeAssetManager::new(
            Arc::new(storage),
            mock_write_scope_with_commit_acknowledgement_loss(),
            Arc::new(fixture.path().to_path_buf()),
        );

        let outcome = manager
            .publish(
                ThemeOwner::Site,
                ThemeId::from(1),
                &compiled,
                limits(i64::MAX),
                0,
            )
            .await
            .expect("indeterminate commit is a mutation outcome");

        assert!(matches!(outcome, MutationOutcome::CommitIndeterminate(_)));
        assert!(manager.content_path(digest.as_ref()).exists());
    }
    // guard:no-backend — mocked storage isolates orphan filesystem reconciliation
    #[tokio::test]
    async fn reconciliation_reclaims_orphan_file_after_rechecking_eligibility() {
        let fixture = TempDir::new().expect("create content fixture");
        let bytes = b"orphaned content";
        let digest = digest(bytes);
        let path = fixture
            .path()
            .join("themes")
            .join(&digest.as_ref()[..2])
            .join(&digest.as_ref()[2..4]);
        fs::create_dir_all(&path).expect("create content shard");
        fs::write(path.join(digest.as_ref()), bytes).expect("write orphan content");

        let mut storage = MockThemeStorage::new();
        storage
            .expect_list_content_eligibility()
            .returning(|| Ok(Vec::new()));
        storage
            .expect_expired_retained_content()
            .once()
            .returning(|_| Ok(Vec::new()));
        storage.expect_content_eligibility().returning(|_| Ok(None));
        let manager = ThemeAssetManager::new(
            Arc::new(storage),
            mock_write_scope(),
            Arc::new(fixture.path().to_path_buf()),
        );

        let reconciliation = manager.reconcile_startup().await.expect("reconcile");

        assert_eq!(reconciliation.orphan_files, 1);
        assert_eq!(reconciliation.reclaimed_files, 1);
        assert!(!path.join(digest.as_ref()).exists());
    }

    // guard:no-backend — mocked storage isolates corrupt-file startup rejection
    #[tokio::test]
    async fn reconciliation_refuses_corrupt_referenced_content() {
        let fixture = TempDir::new().expect("create content fixture");
        let digest = digest(b"expected bytes");
        let path = fixture
            .path()
            .join("themes")
            .join(&digest.as_ref()[..2])
            .join(&digest.as_ref()[2..4]);
        fs::create_dir_all(&path).expect("create content shard");
        fs::write(path.join(digest.as_ref()), b"corrupt bytes").expect("write corrupt content");

        let mut storage = MockThemeStorage::new();
        let expected = digest.clone();
        storage
            .expect_list_content_eligibility()
            .returning(move || {
                Ok(vec![crate::ThemeContentEligibility {
                    digest: expected.clone(),
                    mime: "image/png".into(),
                    retained_until_unix_seconds: 0,
                }])
            });
        storage
            .expect_expired_retained_content()
            .once()
            .returning(|_| Ok(Vec::new()));
        let manager = ThemeAssetManager::new(
            Arc::new(storage),
            mock_write_scope(),
            Arc::new(fixture.path().to_path_buf()),
        );

        assert!(manager.reconcile_startup().await.is_err());
    }

    // guard:no-backend — expired unreferenced rows are detached before byte validation
    #[tokio::test]
    async fn reconciliation_collects_corrupt_expired_content_before_validation() {
        let fixture = TempDir::new().expect("create content fixture");
        let digest = digest(b"expected bytes");
        let path = fixture
            .path()
            .join("themes")
            .join(&digest.as_ref()[..2])
            .join(&digest.as_ref()[2..4]);
        fs::create_dir_all(&path).expect("create content shard");
        fs::write(path.join(digest.as_ref()), b"corrupt bytes").expect("write corrupt content");

        let mut storage = MockThemeStorage::new();
        let expired = digest.clone();
        storage
            .expect_expired_retained_content()
            .once()
            .returning(move |_| Ok(vec![(ThemeOwner::Site, expired.clone())]));
        storage
            .expect_collect_retained_content()
            .once()
            .returning(|_, _, _, _| Ok(()));
        storage
            .expect_list_content_eligibility()
            .times(2)
            .returning(|| Ok(Vec::new()));
        let manager = ThemeAssetManager::new(
            Arc::new(storage),
            mock_write_scope(),
            Arc::new(fixture.path().to_path_buf()),
        );

        manager.reconcile_startup().await.expect("reconcile");
        assert!(!path.join(digest.as_ref()).exists());
    }

    #[apply(backends)]
    #[tokio::test]
    async fn collection_detaches_expired_content_before_unlinking_on_both_backends(
        #[case] backend: Backend,
    ) {
        let env = backend.setup().await;
        let digest = digest(b"retained bytes");
        let charge = ThemeContentCharge {
            digest: digest.clone(),
            logical_bytes: 14,
            physical_bytes: 14,
        };
        let limits = ThemeQuotaLimits {
            active_themes: 1,
            retained_revisions: 1,
            logical_bytes: 14,
            site_retained_revisions: 1,
            site_physical_bytes: 14,
        };
        let themes = Arc::clone(&env.state.themes);
        confirmed(
            env.state
                .write_scope
                .run(move |transaction| {
                    Box::pin(async move {
                        themes
                            .admit_theme(transaction, ThemeOwner::Site, limits)
                            .await
                    })
                })
                .await
                .expect("admit owner"),
        );
        let themes = Arc::clone(&env.state.themes);
        let eligibility = ThemeContentEligibility {
            digest: digest.clone(),
            mime: "image/png".into(),
            retained_until_unix_seconds: 0,
        };
        confirmed(
            env.state
                .write_scope
                .run(move |transaction| {
                    Box::pin(async move {
                        themes
                            .upsert_content_eligibility(transaction, &eligibility)
                            .await
                    })
                })
                .await
                .expect("make bytes eligible"),
        );
        let themes = Arc::clone(&env.state.themes);
        let attached = charge.clone();
        confirmed(
            env.state
                .write_scope
                .run(move |transaction| {
                    Box::pin(async move {
                        themes
                            .attach_revision_content(
                                transaction,
                                ThemeOwner::Site,
                                limits,
                                &[attached],
                            )
                            .await
                    })
                })
                .await
                .expect("attach content"),
        );
        let themes = Arc::clone(&env.state.themes);
        confirmed(
            env.state
                .write_scope
                .run(move |transaction| {
                    Box::pin(async move {
                        themes
                            .detach_revision_content(transaction, ThemeOwner::Site, &[charge], 10)
                            .await
                    })
                })
                .await
                .expect("detach content"),
        );
        let path = env
            .base
            .path()
            .join("themes")
            .join(&digest.as_ref()[..2])
            .join(&digest.as_ref()[2..4]);
        fs::create_dir_all(&path).expect("create content shard");
        fs::write(path.join(digest.as_ref()), b"retained bytes").expect("write retained bytes");
        let manager = ThemeAssetManager::new(
            Arc::clone(&env.state.themes),
            env.state.write_scope.clone(),
            Arc::new(env.base.path().to_path_buf()),
        );

        let outcome = manager
            .collect(ThemeOwner::Site, &digest, 10)
            .await
            .expect("collect elapsed content");

        assert!(matches!(outcome, MutationOutcome::Confirmed(())));
        assert!(
            env.state
                .themes
                .content_eligibility(&digest)
                .await
                .expect("read eligibility")
                .is_none()
        );
        assert!(!path.join(digest.as_ref()).exists());
    }

    // guard:no-backend — mocked storage isolates eligibility-detach failure ordering
    #[tokio::test]
    async fn eligibility_detach_failure_leaves_file_for_retry() {
        let fixture = TempDir::new().expect("create fixture");
        let bytes = b"still eligible";
        let digest = digest(bytes);
        let path = fixture
            .path()
            .join("themes")
            .join(&digest.as_ref()[..2])
            .join(&digest.as_ref()[2..4]);
        fs::create_dir_all(&path).expect("create content shard");
        fs::write(path.join(digest.as_ref()), bytes).expect("write content");
        let mut storage = MockThemeStorage::new();
        storage
            .expect_collect_retained_content()
            .once()
            .returning(|_, _, _, _| Err(sqlx::Error::RowNotFound));
        let manager = ThemeAssetManager::new(
            Arc::new(storage),
            mock_write_scope(),
            Arc::new(fixture.path().to_path_buf()),
        );

        assert!(manager.collect(ThemeOwner::Site, &digest, 1).await.is_err());
        assert!(path.join(digest.as_ref()).exists());
    }

    // guard:no-backend — mocked storage isolates unlink failure and startup retry
    #[tokio::test]
    async fn unlink_failure_leaves_retryable_orphan_and_startup_retry_removes_it() {
        let fixture = TempDir::new().expect("create fixture");
        let bytes = b"reclaim me";
        let digest = digest(bytes);
        let path = fixture
            .path()
            .join("themes")
            .join(&digest.as_ref()[..2])
            .join(&digest.as_ref()[2..4]);
        fs::create_dir_all(&path).expect("create content shard");
        fs::write(path.join(digest.as_ref()), bytes).expect("write content");

        let mut detached = MockThemeStorage::new();
        detached
            .expect_collect_retained_content()
            .once()
            .returning(|_, _, _, _| Ok(()));
        let manager = ThemeAssetManager::new(
            Arc::new(detached),
            mock_write_scope(),
            Arc::new(fixture.path().to_path_buf()),
        )
        .with_unlink_failure_for_test();
        assert!(matches!(
            manager
                .collect(ThemeOwner::Site, &digest, 1)
                .await
                .expect("database detachment"),
            MutationOutcome::Confirmed(())
        ));
        assert!(path.join(digest.as_ref()).exists());

        let mut reconciliation_storage = MockThemeStorage::new();
        reconciliation_storage
            .expect_list_content_eligibility()
            .times(2)
            .returning(|| Ok(Vec::new()));
        reconciliation_storage
            .expect_expired_retained_content()
            .once()
            .returning(|_| Ok(Vec::new()));
        reconciliation_storage
            .expect_content_eligibility()
            .once()
            .returning(|_| Ok(None));
        let reconciliation_manager = ThemeAssetManager::new(
            Arc::new(reconciliation_storage),
            mock_write_scope(),
            Arc::new(fixture.path().to_path_buf()),
        );
        let reconciliation = reconciliation_manager
            .reconcile_startup()
            .await
            .expect("startup retry");

        assert_eq!(reconciliation.reclaimed_files, 1);
        assert!(!path.join(digest.as_ref()).exists());
    }
}
