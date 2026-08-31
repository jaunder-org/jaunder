//! Cross-process serialization for content-addressed media filesystem changes.

use std::collections::BTreeSet;
use std::fs::{File, OpenOptions};
use std::io;
use std::path::PathBuf;
use std::sync::Arc;

use common::media::{ContentHash, MediaRef};
use tokio::fs;

/// Composition-root-owned media locks shared by filesystem and Post writers.
#[derive(Clone)]
pub struct MediaContentLocks {
    storage_path: Arc<PathBuf>,
}

/// Held per-content operating-system locks, released when dropped.
pub(crate) struct MediaContentGuard {
    _files: Vec<File>,
}

impl MediaContentLocks {
    #[must_use]
    pub fn new(storage_path: Arc<PathBuf>) -> Self {
        Self { storage_path }
    }

    pub(crate) fn storage_path(&self) -> &Arc<PathBuf> {
        &self.storage_path
    }

    /// Acquires each content hash in stable order.
    ///
    /// # Errors
    ///
    /// Returns an I/O error when the lock directory or a lock file cannot be
    /// opened, or when the operating system refuses a lock.
    pub(crate) async fn acquire<'a>(
        &self,
        media: impl IntoIterator<Item = &'a MediaRef>,
    ) -> io::Result<MediaContentGuard> {
        let hashes = media
            .into_iter()
            .map(|media| media.sha256.clone())
            .collect::<BTreeSet<_>>();
        self.acquire_hashes(hashes).await
    }

    pub(crate) async fn acquire_one(&self, hash: &ContentHash) -> io::Result<MediaContentGuard> {
        self.acquire_hashes([hash.clone()]).await
    }

    async fn acquire_hashes(
        &self,
        hashes: impl IntoIterator<Item = ContentHash>,
    ) -> io::Result<MediaContentGuard> {
        let lock_dir = self.storage_path.join("media").join(".locks");
        fs::create_dir_all(&lock_dir).await?;
        let lock_paths = hashes
            .into_iter()
            .map(|hash| lock_dir.join(format!("{hash}.lock")))
            .collect::<Vec<_>>();
        let files = tokio::task::spawn_blocking(move || {
            lock_paths
                .into_iter()
                .map(|path| {
                    let file = OpenOptions::new().create(true).append(true).open(path)?;
                    file.lock()?;
                    Ok(file)
                })
                .collect::<io::Result<Vec<_>>>()
        })
        .await
        .map_err(io::Error::other)??;
        Ok(MediaContentGuard { _files: files })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;
    use tokio::sync::Barrier;

    // guard:no-backend — operating-system file locks only; no database is involved.
    #[tokio::test]
    async fn same_content_hash_is_serialized_across_coordinator_instances() {
        let temp = tempfile::tempdir().unwrap();
        let storage_path = Arc::new(temp.path().to_path_buf());
        let first_locks = MediaContentLocks::new(Arc::clone(&storage_path));
        let second_locks = MediaContentLocks::new(storage_path);
        let hash = ContentHash::from_digest([7; 32]);
        let first = first_locks.acquire_one(&hash).await.unwrap();

        let barrier = Arc::new(Barrier::new(2));
        let waiter_barrier = Arc::clone(&barrier);
        let waiter_hash = hash.clone();
        let waiter = tokio::spawn(async move {
            waiter_barrier.wait().await;
            second_locks.acquire_one(&waiter_hash).await.unwrap()
        });
        barrier.wait().await;
        tokio::task::yield_now().await;
        assert!(!waiter.is_finished());

        drop(first);
        let _second = tokio::time::timeout(Duration::from_secs(1), waiter)
            .await
            .expect("second coordinator should acquire after release")
            .unwrap();
    }
}
