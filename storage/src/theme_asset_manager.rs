//! Immutable filesystem lifecycle for compiled public Theme content.
//!
//! The package compiler is the only producer of [`CompiledThemeRevision`]. This
//! service materializes those exact bytes before making their digest eligible for
//! public serving, and keeps filesystem cleanup outside short database writes.

use std::{collections::BTreeSet, io, path::PathBuf, sync::Arc};

use common::{
    MutationOutcome,
    ids::ThemeId,
    theme::{ThemeAssetDigest, ThemeContentDigest, ThemeRevisionDigest, ThemeStylesheetDigest},
};
use host::{system_theme::SystemArtifactInventory, theme_package::CompiledThemeRevision};
use jiff::Timestamp;
use sha2::{Digest, Sha256};
use thiserror::Error;
use tokio::{fs, io::AsyncWriteExt};

use crate::{
    SystemThemeAdmission, ThemeContentCharge, ThemeContentEligibility, ThemeOwner,
    ThemePackageAsset, ThemePublicationAdmission, ThemeQuotaLimits, ThemeRevision, ThemeStorage,
    WriteScope, WriteScopeError,
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
        let now_unix_seconds = Timestamp::now().as_second();
        for (owner, digest) in self
            .themes
            .expired_retained_content(now_unix_seconds)
            .await?
        {
            self.collect(owner, &digest, now_unix_seconds).await?;
        }

        for digest in self.themes.expired_system_content(now_unix_seconds).await? {
            self.collect_system(&digest, now_unix_seconds).await?;
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

    /// Installs the complete compiler-minted system inventory before atomically
    /// advancing all four release roles. The inventory lock serializes release
    /// changes; sorted old/new digest locks join the ordinary content lifecycle.
    /// No filesystem I/O occurs inside the short database transaction.
    ///
    /// # Errors
    ///
    /// Rejects invalid deadlines, filesystem failures, corrupt existing bytes, or
    /// failed admission. Commit-indeterminate outcomes retain installed bytes and
    /// must not be treated as confirmed startup readiness.
    pub async fn install_system(
        &self,
        inventory: &SystemArtifactInventory,
        now_unix_seconds: i64,
    ) -> Result<MutationOutcome<()>, ThemeAssetError> {
        let admission = SystemThemeAdmission::from_inventory(inventory, now_unix_seconds)?;
        let _inventory_lock = self.acquire_inventory_lock().await?;
        let application = inventory.application().content();
        let mut blobs = vec![Blob {
            digest: ThemeContentDigest::from_digest(application.digest()),
            mime: application.mime(),
            bytes: application.bytes(),
        }];
        blobs.extend(
            inventory
                .themes()
                .flat_map(|package| package.revision().contents())
                .map(|content| Blob {
                    digest: ThemeContentDigest::from_digest(content.digest()),
                    mime: content.mime(),
                    bytes: content.bytes(),
                }),
        );
        blobs.sort_by(|left, right| left.digest.cmp(&right.digest));
        blobs.dedup_by(|left, right| left.digest == right.digest);
        let previous = self.themes.live_system_content_references().await?;
        let _locks = self
            .acquire_locks(
                blobs.iter().map(|blob| &blob.digest).chain(
                    previous
                        .iter()
                        .filter(|reference| reference.live_references > 0)
                        .map(|reference| &reference.digest),
                ),
            )
            .await?;
        let mut installed = Vec::new();
        for blob in &blobs {
            match self.install(blob).await {
                Ok(true) => installed.push(blob.digest.clone()),
                Ok(false) => {}
                Err(error) => {
                    self.cleanup_newly_installed(&installed).await;
                    return Err(error);
                }
            }
        }
        let themes = Arc::clone(&self.themes);
        let outcome = self
            .write_scope
            .run(move |transaction| {
                Box::pin(
                    async move { themes.admit_system_inventory(transaction, &admission).await },
                )
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
        self.collect_content(Some(owner), digest, now_unix_seconds)
            .await
    }

    /// Collects expired system-only content through the ordinary lock-held,
    /// eligibility-first detachment and unlink path, without custom quota charges.
    ///
    /// # Errors
    ///
    /// Returns an error while any reference/retention guarantee/owner charge
    /// prevents collection, or if locking/storage detachment fails.
    pub async fn collect_system(
        &self,
        digest: &ThemeContentDigest,
        now_unix_seconds: i64,
    ) -> Result<MutationOutcome<()>, ThemeAssetError> {
        self.collect_content(None, digest, now_unix_seconds).await
    }

    async fn collect_content(
        &self,
        owner: Option<ThemeOwner>,
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
                    match owner {
                        Some(owner) => {
                            themes
                                .collect_retained_content(
                                    transaction,
                                    owner,
                                    &transaction_digest,
                                    now_unix_seconds,
                                )
                                .await
                        }
                        None => {
                            themes
                                .collect_system_content(
                                    transaction,
                                    &transaction_digest,
                                    now_unix_seconds,
                                )
                                .await
                        }
                    }
                })
            })
            .await
            .map_err(|error| match error {
                WriteScopeError::Operation(error) | WriteScopeError::Begin(error) => {
                    ThemeAssetError::Storage(error)
                }
            })?;
        if matches!(outcome, MutationOutcome::Confirmed(()))
            && self.themes.content_eligibility(digest).await?.is_none()
            && let Err(error) = self.unlink_if_present(digest).await
        {
            host::error::report_swallowed(
                host::error::ErrorKind::Internal,
                host::error::ErrorClass::Transient,
                "storage.theme_asset.collect_unlink",
                host::error::SwallowedSource::Error(&error),
            );
        }
        Ok(outcome)
    }

    fn blobs(compiled: &CompiledThemeRevision) -> Result<Vec<Blob<'_>>, ThemeAssetError> {
        let mut blobs = compiled
            .contents()
            .map(|content| {
                Ok(Blob {
                    digest: Self::digest(content.digest())?,
                    mime: content.mime(),
                    bytes: content.bytes(),
                })
            })
            .collect::<Result<Vec<_>, ThemeAssetError>>()?;
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
            stylesheet_digest: Self::stylesheet_digest(compiled.stylesheet_content().digest())?,
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

    async fn acquire_inventory_lock(&self) -> Result<std::fs::File, ThemeAssetError> {
        let directory = self.storage_path.join("themes").join(".locks");
        fs::create_dir_all(&directory).await?;
        self.sync_content_directories(&directory).await?;
        let path = directory.join("system-inventory.lock");
        tokio::task::spawn_blocking(move || {
            let file = std::fs::OpenOptions::new()
                .create(true)
                .append(true)
                .open(path)?;
            file.lock()?;
            Ok::<_, io::Error>(file)
        })
        .await
        .map_err(io::Error::other)?
        .map_err(ThemeAssetError::Filesystem)
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
            // cov:ignore-start: Tokio's Linux rename replaces an existing destination atomically, so this portable AlreadyExists recovery cannot execute on the authoritative host.
            Err(error) if error.kind() == io::ErrorKind::AlreadyExists => {
                let existing = fs::read(&path).await?;
                fs::remove_file(&temporary).await?;
                Self::sync_directory(staging).await?;
                Self::verify(&existing, &blob.digest).map(|()| false)
            }
            // cov:ignore-stop
            // cov:ignore-start: after the missing-destination check, this rename failure needs a post-check OS race or host fault injection unavailable to authoritative coverage.
            Err(error) => Err(finish_install_failure(
                error,
                fs::remove_file(&temporary).await,
            )),
            // cov:ignore-stop
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
            match self.themes.content_eligibility(digest).await {
                Ok(None) => {
                    if let Err(error) = self.unlink_if_present(digest).await {
                        host::error::report_swallowed(
                            host::error::ErrorKind::Internal,
                            host::error::ErrorClass::Transient,
                            "storage.theme_asset.publish_cleanup_unlink",
                            host::error::SwallowedSource::Error(&error),
                        );
                    }
                }
                Ok(Some(_)) => {}
                Err(error) => host::error::report_swallowed(
                    host::error::ErrorKind::Storage,
                    host::error::ErrorClass::Transient,
                    "storage.theme_asset.publish_cleanup_eligibility",
                    host::error::SwallowedSource::Error(&error),
                ),
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
                    let Some(digest) = Self::canonical_content_digest(&prefix, &shard, &digest)
                    else {
                        continue;
                    };
                    digests.push(digest);
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

    fn canonical_content_digest(
        prefix: &str,
        shard: &str,
        digest: &str,
    ) -> Option<ThemeContentDigest> {
        if !Self::is_canonical_digest_path(prefix, shard, digest) {
            return None;
        }
        let Ok(digest) = digest.parse() else {
            unreachable!("a canonical content path is a valid theme digest");
        };
        Some(digest)
    }

    fn hex(bytes: &[u8]) -> String {
        let mut output = String::with_capacity(bytes.len() * 2);
        for byte in bytes {
            let _ = std::fmt::Write::write_fmt(&mut output, format_args!("{byte:02x}"));
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

fn finish_install_failure(primary: io::Error, cleanup: Result<(), io::Error>) -> ThemeAssetError {
    crate::helpers::preserve_after_secondary(
        primary,
        cleanup,
        host::error::ErrorKind::Internal,
        host::error::ErrorClass::Transient,
        "storage.theme_asset.install_cleanup_unlink",
    )
    .into()
}

#[cfg(test)]
mod tests {
    use std::{fs, io, sync::Arc};

    use common::{MutationOutcome, ids::ThemeId, theme::ThemeContentDigest};
    use jiff::Timestamp;
    use rstest::*;
    use rstest_reuse::*;
    use sha2::{Digest, Sha256};
    use tempfile::TempDir;

    use super::{ThemeAssetError, ThemeAssetManager, finish_install_failure};
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
    fn install_failure_preserves_primary_and_reports_cleanup_failure() {
        let (error, trace) = crate::helpers::swallowed_test::capture(|| {
            finish_install_failure(
                io::Error::other("primary rename failure"),
                Err(io::Error::other("cleanup unlink failure")),
            )
        });
        assert!(matches!(error, ThemeAssetError::Filesystem(_)));
        assert!(error.to_string().contains("primary rename failure"));
        crate::helpers::swallowed_test::assert_one_report(
            &trace,
            "storage.theme_asset.install_cleanup_unlink",
        );
    }

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
            let _ = std::fmt::Write::write_fmt(&mut hex, format_args!("{byte:02x}"));
        }
        hex.parse().expect("SHA-256 is a valid theme digest")
    }

    // guard:no-backend — directory cleanup and census are filesystem-only invariants
    #[tokio::test]
    async fn staging_cleanup_removes_only_files_and_content_census_skips_noncanonical_nodes() {
        let fixture = TempDir::new().expect("create fixture");
        let manager = ThemeAssetManager::new(
            Arc::new(MockThemeStorage::new()),
            mock_write_scope(),
            Arc::new(fixture.path().to_path_buf()),
        );
        let staging = fixture.path().join("themes").join(".staging");
        fs::create_dir_all(staging.join("nested")).expect("create staging directory");
        fs::write(staging.join("abandoned.tmp"), b"partial").expect("write temporary content");
        manager.reclaim_staging().await.expect("clean staging");
        assert!(!staging.join("abandoned.tmp").exists());
        assert!(staging.join("nested").is_dir());

        let digest = digest(b"canonical content");
        let root = fixture.path().join("themes");
        let shard = root
            .join(&digest.as_ref()[..2])
            .join(&digest.as_ref()[2..4]);
        fs::create_dir_all(&shard).expect("create canonical shard");
        fs::write(shard.join(digest.as_ref()), b"canonical content").expect("write content");
        fs::write(shard.join("not-a-digest"), b"ignore").expect("write noncanonical digest");
        fs::write(root.join("not-a-prefix"), b"ignore").expect("write non-directory prefix");
        fs::create_dir_all(root.join("aa").join("not-a-shard")).expect("create invalid shard");
        assert_eq!(
            manager
                .enumerate_content_digests()
                .await
                .expect("enumerate canonical content"),
            vec![digest.clone()]
        );
        assert!(
            manager
                .unlink_if_present(&digest)
                .await
                .expect("remove existing content")
        );
        assert!(
            !manager
                .unlink_if_present(&digest)
                .await
                .expect("an absent content path is not an error")
        );
    }

    #[apply(backends)]
    #[tokio::test]
    async fn system_install_is_idempotent_and_outside_custom_quotas(#[case] backend: Backend) {
        let env = backend.setup().await;
        let inventory = host::system_theme::compile_system_artifact_inventory().unwrap();
        let manager = ThemeAssetManager::new(
            env.themes(),
            env.write_scope().clone(),
            Arc::new(env.base.path().to_path_buf()),
        );
        let quotas = env.themes().site_quota().await.unwrap();
        assert!(
            env.themes()
                .system_application_content()
                .await
                .unwrap()
                .is_none()
        );
        for package in inventory.themes() {
            assert!(
                env.themes()
                    .system_theme_revision(package.theme())
                    .await
                    .unwrap()
                    .is_none()
            );
        }
        confirmed(manager.install_system(&inventory, 100).await.unwrap());
        let references = env.themes().system_content_references().await.unwrap();
        assert_eq!(references.len(), 4);
        assert!(
            references
                .iter()
                .all(|reference| reference.live_references == 1)
        );
        confirmed(manager.install_system(&inventory, 200).await.unwrap());
        assert_eq!(
            env.themes().system_content_references().await.unwrap(),
            references
        );
        assert_eq!(env.themes().site_quota().await.unwrap(), quotas);
        assert!(
            env.themes()
                .list_themes(ThemeOwner::Site)
                .await
                .unwrap()
                .is_empty()
        );
        assert!(
            env.themes()
                .owner_quota(ThemeOwner::Site)
                .await
                .unwrap()
                .is_none()
        );
        assert_eq!(
            env.themes()
                .system_application_content()
                .await
                .unwrap()
                .unwrap()
                .digest,
            inventory.application().content_digest()
        );
        for package in inventory.themes() {
            let revision = env
                .themes()
                .system_theme_revision(package.theme())
                .await
                .unwrap()
                .unwrap();
            assert_eq!(revision.digest, package.revision_digest());
            assert_eq!(revision.source_digest, package.source_digest());
            assert_eq!(revision.stylesheet_digest, package.stylesheet_digest());
            assert_eq!(revision.manifest, package.revision().canonical_manifest());
        }
        let mut expected = vec![inventory.application().content()];
        expected.extend(
            inventory
                .themes()
                .flat_map(|package| package.revision().contents()),
        );
        for content in expected {
            let digest = common::theme::ThemeContentDigest::from_digest(content.digest());
            assert_eq!(
                fs::read(manager.content_path(digest.as_ref())).unwrap(),
                content.bytes()
            );
            assert_eq!(
                env.themes()
                    .content_eligibility(&digest)
                    .await
                    .unwrap()
                    .unwrap()
                    .mime,
                content.mime()
            );
        }
    }

    #[apply(backends)]
    #[tokio::test]
    async fn system_upgrade_rollback_and_later_detachment_preserve_retention(
        #[case] backend: Backend,
    ) {
        use host::system_theme::qualification::{self, Fixture};
        let env = backend.setup().await;
        let a = qualification::compile(Fixture::A).unwrap();
        let b = qualification::compile(Fixture::BApplication).unwrap();
        let manager = ThemeAssetManager::new(
            env.themes(),
            env.write_scope().clone(),
            Arc::new(env.base.path().to_path_buf()),
        );
        let digest_a = a.application().content_digest();
        let digest_b = b.application().content_digest();
        let retention = super::THEME_CONTENT_RETENTION_SECONDS;
        confirmed(manager.install_system(&a, 100).await.unwrap());
        confirmed(manager.install_system(&b, 200).await.unwrap());
        let deadline = env
            .themes()
            .content_eligibility(&digest_a)
            .await
            .unwrap()
            .unwrap()
            .retained_until_unix_seconds;
        assert_eq!(deadline, 200 + retention);
        confirmed(manager.install_system(&a, 300).await.unwrap());
        assert_eq!(
            env.themes()
                .content_eligibility(&digest_a)
                .await
                .unwrap()
                .unwrap()
                .retained_until_unix_seconds,
            deadline
        );
        assert!(
            manager.collect_system(&digest_a, deadline).await.is_err(),
            "live rollback bytes cannot collect even past their old deadline"
        );
        assert_eq!(
            fs::read(manager.content_path(digest_a.as_ref())).unwrap(),
            a.application().content().bytes()
        );
        let later = deadline + 100;
        confirmed(manager.install_system(&b, later).await.unwrap());
        assert_eq!(
            env.themes()
                .content_eligibility(&digest_a)
                .await
                .unwrap()
                .unwrap()
                .retained_until_unix_seconds,
            later + retention
        );
        assert!(
            manager
                .collect_system(&digest_a, later + retention - 1)
                .await
                .is_err()
        );
        confirmed(
            manager
                .collect_system(&digest_a, later + retention)
                .await
                .unwrap(),
        );
        assert!(!manager.content_path(digest_a.as_ref()).exists());
        assert!(
            env.themes()
                .content_eligibility(&digest_a)
                .await
                .unwrap()
                .is_none()
        );
        assert_eq!(
            fs::read(manager.content_path(digest_b.as_ref())).unwrap(),
            b.application().content().bytes()
        );
        assert_eq!(
            env.themes()
                .system_application_content()
                .await
                .unwrap()
                .unwrap()
                .digest,
            digest_b
        );
        for package in b.themes() {
            assert_eq!(
                env.themes()
                    .system_theme_revision(package.theme())
                    .await
                    .unwrap()
                    .unwrap()
                    .digest,
                package.revision_digest()
            );
        }
    }

    #[apply(backends)]
    #[tokio::test]
    async fn system_admission_leaves_unrelated_retained_history_untouched_and_preserves_rollback_deadlines(
        #[case] backend: Backend,
    ) {
        use host::system_theme::qualification::{self, Fixture};

        let env = backend.setup().await;
        let a = qualification::compile(Fixture::A).expect("compile release A");
        let b_application =
            qualification::compile(Fixture::BApplication).expect("compile application release B");
        let b_theme = qualification::compile(Fixture::BTheme).expect("compile theme release B");
        let manager = ThemeAssetManager::new(
            env.themes(),
            env.write_scope().clone(),
            Arc::new(env.base.path().to_path_buf()),
        );
        let now = Timestamp::now().as_second();
        let first = now.checked_add(1).expect("advance time");
        let second = first.checked_add(1).expect("advance time");
        let rollback = second.checked_add(1).expect("advance time");
        let retention = super::THEME_CONTENT_RETENTION_SECONDS;
        let b_application_digest = b_application.application().content_digest();
        confirmed(manager.install_system(&a, now).await.expect("install A"));
        confirmed(
            manager
                .install_system(&b_application, first)
                .await
                .expect("install application B"),
        );
        confirmed(
            manager
                .install_system(&b_theme, second)
                .await
                .expect("install theme B"),
        );
        let retained_before_repeat = env
            .themes()
            .system_content_references()
            .await
            .expect("read retained history")
            .into_iter()
            .find(|reference| reference.digest == b_application_digest)
            .expect("application B is retained history");
        assert_eq!(retained_before_repeat.live_references, 0);
        assert_eq!(
            retained_before_repeat.retained_until_unix_seconds,
            second
                .checked_add(retention)
                .expect("retention deadline fits")
        );

        // Repeating B-theme does not include retained B-application content;
        // admission must not visit or extend unrelated detached history.
        confirmed(
            manager
                .install_system(&b_theme, now)
                .await
                .expect("repeat theme B"),
        );
        assert_eq!(
            env.themes()
                .system_content_references()
                .await
                .expect("read repeated retained history")
                .into_iter()
                .find(|reference| reference.digest == b_application_digest)
                .expect("unrelated retained application remains")
                .retained_until_unix_seconds,
            retained_before_repeat.retained_until_unix_seconds
        );

        // B-application is an incoming retained digest on rollback: it must
        // reattach without resetting the deadline established while detached.
        confirmed(
            manager
                .install_system(&b_application, rollback)
                .await
                .expect("rollback to application B"),
        );
        let reattached = env
            .themes()
            .system_content_references()
            .await
            .expect("read reattached application")
            .into_iter()
            .find(|reference| reference.digest == b_application_digest)
            .expect("application B remains tracked");
        assert_eq!(reattached.live_references, 1);
        assert_eq!(
            reattached.retained_until_unix_seconds,
            retained_before_repeat.retained_until_unix_seconds
        );
    }

    #[apply(backends)]
    #[tokio::test]
    async fn system_theme_upgrade_changes_only_selected_package_identity(#[case] backend: Backend) {
        use host::system_theme::qualification::{self, Fixture};
        let env = backend.setup().await;
        let a = qualification::compile(Fixture::A).unwrap();
        let b = qualification::compile(Fixture::BTheme).unwrap();
        let manager = ThemeAssetManager::new(
            env.themes(),
            env.write_scope().clone(),
            Arc::new(env.base.path().to_path_buf()),
        );
        confirmed(manager.install_system(&a, 100).await.unwrap());
        confirmed(manager.install_system(&b, 200).await.unwrap());
        assert_eq!(
            env.themes()
                .system_application_content()
                .await
                .unwrap()
                .unwrap()
                .digest,
            a.application().content_digest()
        );
        for package in b.themes() {
            let revision = env
                .themes()
                .system_theme_revision(package.theme())
                .await
                .unwrap()
                .unwrap();
            assert_eq!(revision.digest, package.revision_digest());
            assert_eq!(revision.source_digest, package.source_digest());
        }
        let old_studio = common::theme::ThemeContentDigest::from_digest(
            a.theme(common::theme::Theme::Studio)
                .stylesheet_content()
                .digest(),
        );
        assert_eq!(
            env.themes()
                .content_eligibility(&old_studio)
                .await
                .unwrap()
                .unwrap()
                .retained_until_unix_seconds,
            200 + super::THEME_CONTENT_RETENTION_SECONDS
        );
        assert_eq!(
            fs::read(manager.content_path(old_studio.as_ref())).unwrap(),
            a.theme(common::theme::Theme::Studio)
                .stylesheet_content()
                .bytes()
        );
        confirmed(manager.install_system(&a, 300).await.unwrap());
        assert_eq!(
            env.themes()
                .system_theme_revision(common::theme::Theme::Studio)
                .await
                .unwrap()
                .unwrap()
                .digest,
            a.theme(common::theme::Theme::Studio).revision_digest()
        );
    }

    #[apply(backends)]
    #[tokio::test]
    async fn system_admission_callback_failure_rolls_back_the_entire_inventory(
        #[case] backend: Backend,
    ) {
        use host::system_theme::qualification::{self, Fixture};
        let env = backend.setup().await;
        let a = qualification::compile(Fixture::A).unwrap();
        let b = qualification::compile(Fixture::BTheme).unwrap();
        let manager = ThemeAssetManager::new(
            env.themes(),
            env.write_scope().clone(),
            Arc::new(env.base.path().to_path_buf()),
        );
        confirmed(manager.install_system(&a, 100).await.unwrap());
        let references = env.themes().system_content_references().await.unwrap();
        let eligibilities = env.themes().list_content_eligibility().await.unwrap();
        let admission = crate::SystemThemeAdmission::from_inventory(&b, 200).unwrap();
        let themes = env.themes();
        let result = env
            .write_scope()
            .run(move |transaction| {
                Box::pin(async move {
                    themes
                        .admit_system_inventory(transaction, &admission)
                        .await?;
                    Err::<(), sqlx::Error>(sqlx::Error::RowNotFound)
                })
            })
            .await;
        assert!(result.is_err());
        assert_eq!(
            env.themes().system_content_references().await.unwrap(),
            references
        );
        assert_eq!(
            env.themes().list_content_eligibility().await.unwrap(),
            eligibilities
        );
        assert_eq!(
            env.themes()
                .system_application_content()
                .await
                .unwrap()
                .unwrap()
                .digest,
            a.application().content_digest()
        );
        for package in a.themes() {
            assert_eq!(
                env.themes()
                    .system_theme_revision(package.theme())
                    .await
                    .unwrap()
                    .unwrap()
                    .digest,
                package.revision_digest()
            );
        }
    }

    #[apply(backends)]
    #[tokio::test]
    async fn system_materialization_failure_cleans_new_files_without_publishing_any_role(
        #[case] backend: Backend,
    ) {
        let env = backend.setup().await;
        let inventory = host::system_theme::compile_system_artifact_inventory().unwrap();
        let manager = ThemeAssetManager::new(
            env.themes(),
            env.write_scope().clone(),
            Arc::new(env.base.path().to_path_buf()),
        );
        let mut digests = vec![inventory.application().content_digest()];
        digests.extend(
            inventory
                .themes()
                .flat_map(|package| package.revision().contents())
                .map(|content| ThemeContentDigest::from_digest(content.digest())),
        );
        digests.sort();
        let blocked = digests.last().unwrap();
        fs::create_dir_all(manager.content_path(blocked.as_ref())).unwrap();
        assert!(manager.install_system(&inventory, 100).await.is_err());
        assert!(
            env.themes()
                .system_content_references()
                .await
                .unwrap()
                .is_empty()
        );
        assert!(
            env.themes()
                .list_content_eligibility()
                .await
                .unwrap()
                .is_empty()
        );
        assert!(
            env.themes()
                .system_application_content()
                .await
                .unwrap()
                .is_none()
        );
        for package in inventory.themes() {
            assert!(
                env.themes()
                    .system_theme_revision(package.theme())
                    .await
                    .unwrap()
                    .is_none()
            );
        }
        for digest in digests.iter().filter(|digest| *digest != blocked) {
            assert!(!manager.content_path(digest.as_ref()).exists());
        }
    }

    #[apply(backends)]
    #[tokio::test]
    async fn shared_system_and_custom_content_cannot_collect_the_other_live_owner(
        #[case] backend: Backend,
    ) {
        use host::system_theme::qualification::{self, Fixture};
        let env = backend.setup().await;
        let a = qualification::compile(Fixture::A).unwrap();
        let b = qualification::compile(Fixture::BTheme).unwrap();
        let compiled = a.theme(common::theme::Theme::Studio).revision();
        let manager = ThemeAssetManager::new(
            env.themes(),
            env.write_scope().clone(),
            Arc::new(env.base.path().to_path_buf()),
        );
        confirmed(manager.install_system(&a, 100).await.unwrap());
        let theme_id = create_theme(env.themes(), env.write_scope().clone(), compiled).await;
        confirmed(
            manager
                .publish(ThemeOwner::Site, theme_id, compiled, limits(i64::MAX), 100)
                .await
                .unwrap(),
        );
        let digest = ThemeContentDigest::from_digest(compiled.stylesheet_content().digest());
        let retention = super::THEME_CONTENT_RETENTION_SECONDS;
        confirmed(manager.install_system(&b, 200).await.unwrap());
        assert!(
            manager
                .collect_system(&digest, 200 + retention)
                .await
                .is_err(),
            "detached system content must not remove custom-live bytes"
        );
        assert_eq!(
            fs::read(manager.content_path(digest.as_ref())).unwrap(),
            compiled.stylesheet_content().bytes()
        );
        confirmed(manager.install_system(&a, 300 + retention).await.unwrap());
        let themes = env.themes();
        let custom_deadline = 400 + 2 * retention;
        confirmed(
            env.write_scope()
                .run(move |transaction| {
                    Box::pin(async move {
                        themes
                            .remove_theme(transaction, ThemeOwner::Site, theme_id, custom_deadline)
                            .await
                    })
                })
                .await
                .unwrap(),
        );
        assert!(
            manager
                .collect(ThemeOwner::Site, &digest, custom_deadline)
                .await
                .is_err(),
            "expired custom charges must not remove system-live bytes"
        );
        assert_eq!(
            fs::read(manager.content_path(digest.as_ref())).unwrap(),
            compiled.stylesheet_content().bytes()
        );
        let last_detach = custom_deadline + 100;
        confirmed(manager.install_system(&b, last_detach).await.unwrap());
        assert!(
            manager
                .collect(ThemeOwner::Site, &digest, last_detach + retention - 1)
                .await
                .is_err()
        );
        confirmed(
            manager
                .collect(ThemeOwner::Site, &digest, last_detach + retention)
                .await
                .unwrap(),
        );
        assert!(
            env.themes()
                .content_eligibility(&digest)
                .await
                .unwrap()
                .is_none()
        );
        assert!(!manager.content_path(digest.as_ref()).exists());
        assert!(
            !env.themes()
                .system_content_references()
                .await
                .unwrap()
                .iter()
                .any(|reference| reference.digest == digest)
        );
        assert_eq!(env.themes().site_quota().await.unwrap().physical_bytes, 0);
    }

    #[apply(backends)]
    #[tokio::test]
    async fn concurrent_system_installs_publish_one_complete_release_inventory(
        #[case] backend: Backend,
    ) {
        use host::system_theme::qualification::{self, Fixture};

        let env = backend.setup().await;
        let a = qualification::compile(Fixture::A).expect("compile release A");
        let b_application = qualification::compile(Fixture::BApplication)
            .expect("compile application-only release B");
        let b_theme =
            qualification::compile(Fixture::BTheme).expect("compile theme-only release B");
        let root = Arc::new(env.base.path().to_path_buf());
        let first =
            ThemeAssetManager::new(env.themes(), env.write_scope().clone(), Arc::clone(&root));
        let second = ThemeAssetManager::new(env.themes(), env.write_scope().clone(), root);
        let quotas = env.themes().site_quota().await.expect("read initial quota");
        let now = Timestamp::now().as_second();
        confirmed(
            first
                .install_system(&a, now)
                .await
                .expect("install release A"),
        );

        let (application_result, theme_result) = tokio::join!(
            first.install_system(&b_application, now.checked_add(1).expect("advance time")),
            second.install_system(&b_theme, now.checked_add(2).expect("advance time")),
        );
        confirmed(application_result.expect("install application release B"));
        confirmed(theme_result.expect("install theme release B"));

        let application = env
            .themes()
            .system_application_content()
            .await
            .expect("read current application")
            .expect("current application exists");
        let winner = if application.digest == b_application.application().content_digest() {
            &b_application
        } else {
            assert_eq!(application.digest, b_theme.application().content_digest());
            &b_theme
        };
        for package in winner.themes() {
            let persisted = env
                .themes()
                .system_theme_revision(package.theme())
                .await
                .expect("read current bundled revision")
                .expect("current bundled revision exists");
            assert_eq!(persisted.digest, package.revision_digest());
            assert_eq!(persisted.source_digest, package.source_digest());
            assert_eq!(persisted.stylesheet_digest, package.stylesheet_digest());
        }
        let references = env
            .themes()
            .system_content_references()
            .await
            .expect("read references");
        assert_eq!(
            references.len(),
            6,
            "both competing replacement digests remain retained"
        );
        let live = references
            .iter()
            .filter(|reference| reference.live_references == 1)
            .collect::<Vec<_>>();
        assert_eq!(live.len(), 4, "only the complete winning inventory is live");
        assert_eq!(
            live.iter()
                .map(|reference| reference.digest.clone())
                .collect::<std::collections::BTreeSet<_>>(),
            std::iter::once(winner.application().content_digest())
                .chain(winner.themes().flat_map(|package| {
                    package
                        .revision()
                        .contents()
                        .map(|content| ThemeContentDigest::from_digest(content.digest()))
                }))
                .collect(),
            "positive references must be exactly the complete winning inventory"
        );
        let detached = references
            .iter()
            .filter(|reference| reference.live_references == 0)
            .collect::<Vec<_>>();
        assert_eq!(
            detached.len(),
            2,
            "both non-winning replacement digests are retained"
        );
        assert!(
            detached
                .iter()
                .all(|reference| reference.retained_until_unix_seconds > now),
            "non-winning release bytes remain retained rather than becoming collectable"
        );
        for content in std::iter::once(winner.application().content()).chain(
            winner
                .themes()
                .flat_map(|package| package.revision().contents()),
        ) {
            let digest = ThemeContentDigest::from_digest(content.digest());
            assert_eq!(
                fs::read(first.content_path(digest.as_ref())).expect("read installed bytes"),
                content.bytes()
            );
            assert!(
                env.themes()
                    .content_eligibility(&digest)
                    .await
                    .expect("read current eligibility")
                    .is_some()
            );
        }
        assert_eq!(
            env.themes().site_quota().await.expect("read final quota"),
            quotas
        );
        assert!(
            env.themes()
                .owner_quota(ThemeOwner::Site)
                .await
                .expect("read site quota")
                .is_none()
        );
        assert!(
            env.themes()
                .list_themes(ThemeOwner::Site)
                .await
                .expect("read custom themes")
                .is_empty()
        );
    }

    #[apply(backends)]
    #[tokio::test]
    async fn expired_system_collection_racing_reinstallation_preserves_reinstalled_bytes(
        #[case] backend: Backend,
    ) {
        use host::system_theme::qualification::{self, Fixture};

        let env = backend.setup().await;
        let a = qualification::compile(Fixture::A).expect("compile release A");
        let b = qualification::compile(Fixture::BApplication).expect("compile release B");
        let root = Arc::new(env.base.path().to_path_buf());
        let collector =
            ThemeAssetManager::new(env.themes(), env.write_scope().clone(), Arc::clone(&root));
        let installer = ThemeAssetManager::new(env.themes(), env.write_scope().clone(), root);
        let now = Timestamp::now().as_second();
        let replacement = now.checked_add(1).expect("advance time");
        let deadline = replacement
            .checked_add(super::THEME_CONTENT_RETENTION_SECONDS)
            .expect("retention deadline fits");
        let digest = a.application().content_digest();
        confirmed(
            installer
                .install_system(&a, now)
                .await
                .expect("install release A"),
        );
        confirmed(
            installer
                .install_system(&b, replacement)
                .await
                .expect("replace release A"),
        );

        let (collected, reinstalled) = tokio::join!(
            collector.collect_system(&digest, deadline),
            installer.install_system(&a, deadline),
        );
        match collected {
            Ok(MutationOutcome::Confirmed(()))
            | Err(ThemeAssetError::Storage(sqlx::Error::RowNotFound)) => {}
            Err(error) => panic!("collector failed unexpectedly: {error}"),
            Ok(MutationOutcome::CommitIndeterminate(())) => {
                panic!("collector did not have a confirmed outcome")
            }
        }
        confirmed(reinstalled.expect("reinstall release A"));
        assert_eq!(
            env.themes()
                .system_application_content()
                .await
                .expect("read current application")
                .expect("current application exists")
                .digest,
            digest
        );
        assert_eq!(
            fs::read(installer.content_path(digest.as_ref())).expect("read reinstalled bytes"),
            a.application().content().bytes()
        );
        assert!(
            env.themes()
                .content_eligibility(&digest)
                .await
                .expect("read eligibility")
                .is_some()
        );
        assert_eq!(
            env.themes()
                .system_content_references()
                .await
                .expect("read references")
                .into_iter()
                .find(|reference| reference.digest == digest)
                .expect("reinstalled digest remains tracked")
                .live_references,
            1
        );
    }

    #[apply(backends)]
    #[tokio::test]
    async fn system_install_commit_acknowledgement_loss_retains_recoverable_inventory(
        #[case] backend: Backend,
    ) {
        use host::system_theme::qualification::{self, Fixture};

        let env = backend.setup().await;
        let a = qualification::compile(Fixture::A).expect("compile release A");
        let b = qualification::compile(Fixture::BApplication).expect("compile release B");
        let root = Arc::new(env.base.path().to_path_buf());
        let normal =
            ThemeAssetManager::new(env.themes(), env.write_scope().clone(), Arc::clone(&root));
        let acknowledgement_lost = ThemeAssetManager::new(
            env.themes(),
            env.write_scope()
                .with_commit_acknowledgement_loss_after_commit_for_test(),
            root,
        );
        let now = Timestamp::now().as_second();
        confirmed(
            normal
                .install_system(&a, now)
                .await
                .expect("install release A"),
        );
        let outcome = acknowledgement_lost
            .install_system(&b, now.checked_add(1).expect("advance time"))
            .await
            .expect("lost acknowledgement remains a mutation outcome");
        assert!(matches!(outcome, MutationOutcome::CommitIndeterminate(())));

        assert_eq!(
            env.themes()
                .system_application_content()
                .await
                .expect("read persisted application")
                .expect("persisted application exists")
                .digest,
            b.application().content_digest()
        );
        for package in b.themes() {
            assert_eq!(
                env.themes()
                    .system_theme_revision(package.theme())
                    .await
                    .expect("read persisted bundled revision")
                    .expect("persisted bundled revision exists")
                    .digest,
                package.revision_digest()
            );
        }
        let retained = a.application().content_digest();
        assert_eq!(
            fs::read(normal.content_path(retained.as_ref())).expect("read retained bytes"),
            a.application().content().bytes()
        );
        for content in std::iter::once(b.application().content())
            .chain(b.themes().flat_map(|package| package.revision().contents()))
        {
            let digest = ThemeContentDigest::from_digest(content.digest());
            assert_eq!(
                fs::read(normal.content_path(digest.as_ref()))
                    .expect("read uncertain current bytes"),
                content.bytes()
            );
        }
        let references_before_retry = env
            .themes()
            .system_content_references()
            .await
            .expect("read uncertain references");
        let eligibility_before_retry = env
            .themes()
            .list_content_eligibility()
            .await
            .expect("read uncertain eligibility");
        confirmed(
            normal
                .install_system(&b, now.checked_add(2).expect("advance time"))
                .await
                .expect("idempotent retry"),
        );
        assert_eq!(
            env.themes()
                .system_content_references()
                .await
                .expect("read retry references"),
            references_before_retry
        );
        assert_eq!(
            env.themes()
                .list_content_eligibility()
                .await
                .expect("read retry eligibility"),
            eligibility_before_retry
        );
        normal
            .reconcile_startup()
            .await
            .expect("reconcile current and retained bytes");
        assert_eq!(
            fs::read(normal.content_path(retained.as_ref()))
                .expect("read retained bytes after reconciliation"),
            a.application().content().bytes()
        );
        for content in std::iter::once(b.application().content())
            .chain(b.themes().flat_map(|package| package.revision().contents()))
        {
            let digest = ThemeContentDigest::from_digest(content.digest());
            assert_eq!(
                fs::read(normal.content_path(digest.as_ref())).expect("read current bytes"),
                content.bytes()
            );
        }
    }

    #[apply(backends)]
    #[tokio::test]
    async fn system_reconciliation_reclaims_complete_orphans_and_rejects_installed_corruption(
        #[case] backend: Backend,
    ) {
        use host::system_theme::qualification::{self, Fixture};

        let env = backend.setup().await;
        let inventory = qualification::compile(Fixture::A).expect("compile release A");
        let manager = ThemeAssetManager::new(
            env.themes(),
            env.write_scope().clone(),
            Arc::new(env.base.path().to_path_buf()),
        );
        let orphan_bytes = b"uncommitted complete system orphan";
        let orphan = digest(orphan_bytes);
        let orphan_path = manager.content_path(orphan.as_ref());
        fs::create_dir_all(orphan_path.parent().expect("content path has parent"))
            .expect("create orphan shard");
        fs::write(&orphan_path, orphan_bytes).expect("write complete uncommitted orphan");
        let reconciliation = manager.reconcile_startup().await.expect("reconcile orphan");
        assert_eq!(reconciliation.reclaimed_files, 1);
        assert!(!orphan_path.exists());

        confirmed(
            manager
                .install_system(&inventory, Timestamp::now().as_second())
                .await
                .expect("install release A"),
        );
        let digest = inventory.application().content_digest();
        fs::write(
            manager.content_path(digest.as_ref()),
            b"corrupt installed bytes",
        )
        .expect("corrupt installed bytes");
        assert!(
            matches!(manager.reconcile_startup().await, Err(ThemeAssetError::IneligibleContent(found)) if found == digest)
        );
        fs::remove_file(manager.content_path(digest.as_ref())).expect("remove installed bytes");
        assert!(
            matches!(manager.reconcile_startup().await, Err(ThemeAssetError::IneligibleContent(found)) if found == digest)
        );
        assert_eq!(
            env.themes()
                .system_application_content()
                .await
                .expect("read persisted application")
                .expect("persisted application remains")
                .digest,
            digest
        );
    }

    #[apply(backends)]
    #[tokio::test]
    async fn shared_system_package_asset_tracks_roles_and_retention_boundaries(
        #[case] backend: Backend,
    ) {
        use host::system_theme::shared_asset_fixture::{self, SharingFixture};

        let env = backend.setup().await;
        let png = compiled()
            .assets()
            .find(|(_, mime, _, _)| *mime == "image/png")
            .map(|(_, _, bytes, _)| bytes.to_vec())
            .expect("existing validated fixture supplies a PNG");
        let both = shared_asset_fixture::compile(SharingFixture::Both, &png)
            .expect("compile both-role inventory");
        let one = shared_asset_fixture::compile(SharingFixture::One, &png)
            .expect("compile one-role inventory");
        let neither = shared_asset_fixture::compile(SharingFixture::Neither, &png)
            .expect("compile no-role inventory");
        let package = both.theme(common::theme::Theme::Terminal);
        let validated = host::theme_package::validate_theme_package(
            package.package_bytes(),
            host::theme_package::ThemePackageLimits::default(),
        )
        .expect("shared package archive revalidates");
        let asset_urls = validated
            .asset_digests()
            .map(|(path, digest)| {
                (
                    path.to_owned(),
                    common::theme::ThemeAssetDigest::from_digest(digest)
                        .content_url()
                        .to_string(),
                )
            })
            .collect();
        let replay = validated
            .compile(
                &asset_urls,
                host::theme_package::ThemePackageLimits::default(),
            )
            .expect("shared package recompiles with validator-minted URLs");
        assert_eq!(
            replay.revision_digest(),
            package.revision().revision_digest()
        );
        assert_eq!(
            replay.asset("assets/shared-a.png"),
            package.revision().asset("assets/shared-a.png")
        );
        let manager = ThemeAssetManager::new(
            env.themes(),
            env.write_scope().clone(),
            Arc::new(env.base.path().to_path_buf()),
        );
        let quotas = env.themes().site_quota().await.expect("read initial quota");
        let now = Timestamp::now().as_second();
        let partial = now.checked_add(1).expect("advance time");
        let later = partial.checked_add(1).expect("advance time");
        let retention = super::THEME_CONTENT_RETENTION_SECONDS;
        let shared_asset = both
            .theme(common::theme::Theme::Terminal)
            .revision()
            .asset("assets/shared-a.png")
            .expect("shared terminal asset")
            .2;
        let shared = ThemeContentDigest::from_digest(shared_asset);
        confirmed(
            manager
                .install_system(&both, now)
                .await
                .expect("install both roles"),
        );
        assert_eq!(
            env.themes()
                .system_content_references()
                .await
                .expect("read both references")
                .into_iter()
                .find(|reference| reference.digest == shared)
                .expect("shared digest tracked")
                .live_references,
            2
        );
        for theme in [common::theme::Theme::Terminal, common::theme::Theme::Studio] {
            let package = both.theme(theme);
            let persisted = env
                .themes()
                .system_theme_revision(theme)
                .await
                .expect("read persisted shared package")
                .expect("persisted shared package exists");
            for path in ["assets/shared-a.png", "assets/shared-b.png"] {
                let (mime, bytes, digest) = package
                    .revision()
                    .asset(path)
                    .expect("compiled shared asset");
                assert_eq!(mime, "image/png");
                assert_eq!(digest, shared_asset);
                assert_eq!(bytes, png);
                let asset = persisted
                    .assets
                    .iter()
                    .find(|asset| asset.path == path)
                    .expect("persisted shared asset");
                assert_eq!(asset.mime, mime);
                assert_eq!(
                    asset.digest,
                    common::theme::ThemeAssetDigest::from_digest(shared_asset)
                );
            }
        }
        assert_eq!(
            fs::read(manager.content_path(shared.as_ref())).expect("read one physical shared file"),
            png
        );
        assert!(
            manager.collect_system(&shared, now).await.is_err(),
            "live shared bytes cannot collect"
        );

        confirmed(
            manager
                .install_system(&one, partial)
                .await
                .expect("detach one role"),
        );
        let partial_deadline = partial
            .checked_add(retention)
            .expect("retention deadline fits");
        let reference = env
            .themes()
            .system_content_references()
            .await
            .expect("read partial references")
            .into_iter()
            .find(|reference| reference.digest == shared)
            .expect("shared digest remains tracked");
        assert_eq!(reference.live_references, 1);
        assert_eq!(reference.retained_until_unix_seconds, partial_deadline);
        confirmed(
            manager
                .install_system(&both, now)
                .await
                .expect("reattach at earlier clock"),
        );
        confirmed(
            manager
                .install_system(&one, now)
                .await
                .expect("repeat at earlier clock"),
        );
        assert_eq!(
            env.themes()
                .system_content_references()
                .await
                .expect("read repeated references")
                .into_iter()
                .find(|reference| reference.digest == shared)
                .expect("shared digest remains tracked")
                .retained_until_unix_seconds,
            partial_deadline
        );
        confirmed(
            manager
                .install_system(&neither, later)
                .await
                .expect("detach final role"),
        );
        let final_deadline = later
            .checked_add(retention)
            .expect("retention deadline fits");
        let reference = env
            .themes()
            .system_content_references()
            .await
            .expect("read final references")
            .into_iter()
            .find(|reference| reference.digest == shared)
            .expect("shared digest remains retained");
        assert_eq!(reference.live_references, 0);
        assert_eq!(reference.retained_until_unix_seconds, final_deadline);
        confirmed(
            manager
                .install_system(&neither, now)
                .await
                .expect("repeat earlier release"),
        );
        assert_eq!(
            env.themes()
                .system_content_references()
                .await
                .expect("read earlier repeated references")
                .into_iter()
                .find(|reference| reference.digest == shared)
                .expect("shared digest remains retained")
                .retained_until_unix_seconds,
            final_deadline,
            "an earlier supplied clock cannot shorten the final guarantee"
        );
        assert!(
            manager
                .collect_system(&shared, final_deadline - 1)
                .await
                .is_err()
        );
        assert_eq!(
            fs::read(manager.content_path(shared.as_ref()))
                .expect("shared file remains before deadline"),
            png
        );
        confirmed(
            manager
                .collect_system(&shared, final_deadline)
                .await
                .expect("collect at deadline"),
        );
        assert!(!manager.content_path(shared.as_ref()).exists());
        assert!(
            env.themes()
                .content_eligibility(&shared)
                .await
                .expect("read final eligibility")
                .is_none()
        );
        assert_eq!(
            env.themes().site_quota().await.expect("read final quota"),
            quotas
        );
        assert!(
            env.themes()
                .owner_quota(ThemeOwner::Site)
                .await
                .expect("read site quota")
                .is_none()
        );
        assert!(
            env.themes()
                .list_themes(ThemeOwner::Site)
                .await
                .expect("read custom themes")
                .is_empty()
        );
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
            Arc::clone(&env.themes()),
            env.write_scope().clone(),
            &compiled,
        )
        .await;
        let manager = ThemeAssetManager::new(
            Arc::clone(&env.themes()),
            env.write_scope().clone(),
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
            env.themes()
                .content_eligibility(&digest)
                .await
                .expect("read eligibility")
                .is_some()
        );
        assert_eq!(
            env.themes()
                .list_revisions(ThemeOwner::Site, theme_id)
                .await
                .expect("read revisions")
                .len(),
            1
        );
        assert_eq!(
            env.themes()
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
        storage
            .expect_expired_system_content()
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
        storage
            .expect_expired_system_content()
            .once()
            .returning(|_| Ok(Vec::new()));
        let manager = ThemeAssetManager::new(
            Arc::new(storage),
            mock_write_scope(),
            Arc::new(fixture.path().to_path_buf()),
        );

        assert!(manager.reconcile_startup().await.is_err());
    }

    // guard:no-backend — mocked storage isolates startup collection failure propagation
    #[tokio::test]
    async fn reconciliation_propagates_expired_content_collection_failure() {
        let digest = digest(b"expired content");
        let mut storage = MockThemeStorage::new();
        let expired = digest.clone();
        storage
            .expect_expired_retained_content()
            .once()
            .returning(move |_| Ok(vec![(ThemeOwner::Site, expired.clone())]));
        storage
            .expect_collect_retained_content()
            .once()
            .returning(|_, _, _, _| Err(sqlx::Error::RowNotFound));
        let fixture = TempDir::new().expect("create fixture");
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
            .expect_content_eligibility()
            .once()
            .returning(|_| Ok(None));
        storage
            .expect_expired_system_content()
            .once()
            .returning(|_| Ok(Vec::new()));
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
        let themes = Arc::clone(&env.themes());
        confirmed(
            env.write_scope()
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
        let themes = Arc::clone(&env.themes());
        let eligibility = ThemeContentEligibility {
            digest: digest.clone(),
            mime: "image/png".into(),
            retained_until_unix_seconds: 0,
        };
        confirmed(
            env.write_scope()
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
        let themes = Arc::clone(&env.themes());
        let attached = charge.clone();
        confirmed(
            env.write_scope()
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
        let themes = Arc::clone(&env.themes());
        confirmed(
            env.write_scope()
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
            Arc::clone(&env.themes()),
            env.write_scope().clone(),
            Arc::new(env.base.path().to_path_buf()),
        );

        let outcome = manager
            .collect(ThemeOwner::Site, &digest, 10)
            .await
            .expect("collect elapsed content");

        assert!(matches!(outcome, MutationOutcome::Confirmed(())));
        assert!(
            env.themes()
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
        detached
            .expect_content_eligibility()
            .once()
            .returning(|_| Ok(None));
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
            .expect_expired_system_content()
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
    // guard:no-backend — mocked eligibility isolates the cleanup branches.
    #[tokio::test]
    async fn cleanup_reports_only_failed_eligibility_and_unlink_work() {
        let missing = digest(b"missing");
        let retained = digest(b"retained");
        let failed = digest(b"lookup failure");
        let missing_for_lookup = missing.clone();
        let retained_for_lookup = retained.clone();
        let mut storage = MockThemeStorage::new();
        storage
            .expect_content_eligibility()
            .times(3)
            .returning(move |digest| {
                if digest == &missing_for_lookup {
                    Ok(None)
                } else if digest == &retained_for_lookup {
                    Ok(Some(ThemeContentEligibility {
                        digest: retained_for_lookup.clone(),
                        mime: "image/png".into(),
                        retained_until_unix_seconds: 0,
                    }))
                } else {
                    Err(sqlx::Error::RowNotFound)
                }
            });
        let fixture = TempDir::new().expect("create fixture");
        let manager = ThemeAssetManager::new(
            Arc::new(storage),
            mock_write_scope(),
            Arc::new(fixture.path().to_path_buf()),
        )
        .with_unlink_failure_for_test();

        let ((), trace) = crate::helpers::swallowed_test::capture_async(
            manager.cleanup_newly_installed(&[missing, retained, failed]),
        )
        .await;

        assert_eq!(
            trace.matches(r#""error.disposition":"swallowed""#).count(),
            2
        );
        assert!(trace.contains(r#""error.context":"storage.theme_asset.publish_cleanup_unlink""#));
        assert!(
            trace.contains(r#""error.context":"storage.theme_asset.publish_cleanup_eligibility""#)
        );
    }

    // guard:no-backend — filesystem census and cleanup are storage-independent.
    #[tokio::test]
    async fn filesystem_census_handles_non_directory_nodes_and_reports_invalid_roots() {
        let fixture = TempDir::new().expect("create fixture");
        let manager = ThemeAssetManager::new(
            Arc::new(MockThemeStorage::new()),
            mock_write_scope(),
            Arc::new(fixture.path().to_path_buf()),
        );
        let themes = fixture.path().join("themes");
        fs::create_dir_all(&themes).expect("create themes root");
        fs::write(themes.join(".staging"), b"not a directory").expect("create staging file");
        assert!(manager.reclaim_staging().await.is_err());
        fs::remove_file(themes.join(".staging")).expect("remove staging file");

        let digest = digest(b"directory entry");
        let prefix_file = format!(
            "{:02x}",
            u8::from_str_radix(&digest.as_ref()[..2], 16).expect("digest prefix is hex") ^ 1
        );
        fs::write(themes.join(prefix_file), b"not a prefix directory").expect("create prefix file");
        let shard_prefix = format!(
            "{:02x}",
            u8::from_str_radix(&digest.as_ref()[..2], 16).expect("digest prefix is hex") ^ 2
        );
        let shard = themes.join(shard_prefix);
        fs::create_dir_all(&shard).expect("create prefix directory");
        let shard_file = format!(
            "{:02x}",
            u8::from_str_radix(&digest.as_ref()[2..4], 16).expect("digest shard is hex") ^ 1
        );
        fs::write(shard.join(shard_file), b"not a shard directory").expect("create shard file");
        let digest_directory = themes
            .join(&digest.as_ref()[..2])
            .join(&digest.as_ref()[2..4])
            .join(digest.as_ref());
        fs::create_dir_all(digest_directory).expect("create non-file digest entry");
        assert_eq!(
            manager
                .enumerate_content_digests()
                .await
                .expect("skip non-file nodes"),
            Vec::<ThemeContentDigest>::new()
        );

        let root_as_file = TempDir::new().expect("create root fixture");
        fs::write(root_as_file.path().join("themes"), b"not a directory")
            .expect("create root file");
        let root_manager = ThemeAssetManager::new(
            Arc::new(MockThemeStorage::new()),
            mock_write_scope(),
            Arc::new(root_as_file.path().to_path_buf()),
        );
        assert!(root_manager.enumerate_content_digests().await.is_err());

        let path = manager.content_path(digest.as_ref());
        fs::remove_dir(&path).expect("replace directory with content path");

        fs::create_dir_all(&path).expect("create unlink target directory");
        assert!(manager.unlink_if_present(&digest).await.is_err());
    }
    // guard:no-backend — dangling directory symlinks deterministically exercise census errors.
    #[cfg(unix)]
    #[tokio::test]
    async fn filesystem_census_reports_dangling_prefix_and_shard_symlinks() {
        let fixture = TempDir::new().expect("create fixture");
        let manager = ThemeAssetManager::new(
            Arc::new(MockThemeStorage::new()),
            mock_write_scope(),
            Arc::new(fixture.path().to_path_buf()),
        );
        let themes = fixture.path().join("themes");
        fs::create_dir_all(&themes).expect("create themes root");
        std::os::unix::fs::symlink("missing-prefix", themes.join("ab"))
            .expect("create dangling prefix symlink");
        assert!(matches!(
            manager.enumerate_content_digests().await,
            Err(ThemeAssetError::Filesystem(error)) if error.kind() == io::ErrorKind::NotFound
        ));
        fs::remove_file(themes.join("ab")).expect("remove dangling prefix symlink");

        let prefix = themes.join("ab");
        fs::create_dir_all(&prefix).expect("create valid prefix directory");
        std::os::unix::fs::symlink("missing-shard", prefix.join("cd"))
            .expect("create dangling shard symlink");
        assert!(matches!(
            manager.enumerate_content_digests().await,
            Err(ThemeAssetError::Filesystem(error)) if error.kind() == io::ErrorKind::NotFound
        ));
    }

    // guard:no-backend — filesystem content validation is storage-independent.
    #[tokio::test]
    async fn reconciliation_preserves_known_complete_content() {
        let fixture = TempDir::new().expect("create fixture");
        let bytes = b"known content";
        let digest = digest(bytes);
        let path = fixture
            .path()
            .join("themes")
            .join(&digest.as_ref()[..2])
            .join(&digest.as_ref()[2..4]);
        fs::create_dir_all(&path).expect("create content shard");
        fs::write(path.join(digest.as_ref()), bytes).expect("write known content");
        let known = digest.clone();
        let mut storage = MockThemeStorage::new();
        storage
            .expect_expired_retained_content()
            .once()
            .returning(|_| Ok(Vec::new()));
        storage
            .expect_expired_system_content()
            .once()
            .returning(|_| Ok(Vec::new()));
        storage
            .expect_list_content_eligibility()
            .times(2)
            .returning(move || {
                Ok(vec![ThemeContentEligibility {
                    digest: known.clone(),
                    mime: "image/png".into(),
                    retained_until_unix_seconds: 0,
                }])
            });
        storage.expect_content_eligibility().never();
        let manager = ThemeAssetManager::new(
            Arc::new(storage),
            mock_write_scope(),
            Arc::new(fixture.path().to_path_buf()),
        );

        assert_eq!(
            manager
                .reconcile_startup()
                .await
                .expect("reconcile known content"),
            super::ThemeContentReconciliation::default()
        );
        assert!(path.join(digest.as_ref()).exists());
    }
}
