//! A storage-directory lock around offline database phases, separate from the
//! server-lifetime runtime lock. Its on-disk file is never deleted: the kernel
//! releases the lock on process death, not on removal of its path.

use std::{fs, io, path::Path};

/// Holds the exclusive lock at `<storage>/database.lock` until dropped.
/// Ordinary CLI work and server serving continue after the guard is dropped.
pub struct DatabaseLockGuard {
    _file: fs::File,
}

impl DatabaseLockGuard {
    /// Waits for the holder of this storage directory's database phase to finish.
    ///
    /// # Errors
    ///
    /// Returns an error if the lock directory/file cannot be created, the lock
    /// cannot be acquired, or its blocking task cannot complete.
    pub async fn acquire(storage_path: &Path) -> io::Result<Self> {
        let path = storage_path.to_path_buf();
        tokio::task::spawn_blocking(move || {
            fs::create_dir_all(&path)?;
            let file = fs::OpenOptions::new()
                .create(true)
                .append(true)
                .open(path.join("database.lock"))?;
            file.lock()?;
            Ok(Self { _file: file })
        })
        .await
        .map_err(io::Error::other)?
    }
}

#[cfg(test)]
mod tests {
    use std::fs::OpenOptions;

    use super::DatabaseLockGuard;

    // guard:no-backend — exercises the OS file lock across processes, without a database.
    #[test]
    fn database_lock_is_exclusive_across_processes() {
        const CHILD_PATH: &str = "JAUNDER_TEST_DB_LOCK_CHILD_PATH";
        const EXPECT_FREE: &str = "JAUNDER_TEST_DB_LOCK_EXPECT_FREE";
        if let Ok(path) = std::env::var(CHILD_PATH) {
            let file = OpenOptions::new()
                .append(true)
                .open(std::path::Path::new(&path).join("database.lock"))
                .unwrap();
            let expected_free = std::env::var(EXPECT_FREE).unwrap() == "true";
            if expected_free {
                file.try_lock().expect("child must acquire a released lock");
            } else {
                assert!(
                    matches!(file.try_lock(), Err(std::fs::TryLockError::WouldBlock)),
                    "child must see an occupied lock"
                );
            }
            return;
        }

        let dir = tempfile::tempdir().unwrap();
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        let guard = runtime
            .block_on(DatabaseLockGuard::acquire(dir.path()))
            .unwrap();
        let assert_child = |expected_free: bool| {
            let status = std::process::Command::new(std::env::current_exe().unwrap())
                .arg("--exact")
                .arg("database_lock::tests::database_lock_is_exclusive_across_processes")
                .env(CHILD_PATH, dir.path())
                .env(EXPECT_FREE, expected_free.to_string())
                .status()
                .unwrap();
            assert!(status.success(), "child lock assertion failed");
        };
        assert_child(false);
        drop(guard);
        assert_child(true);
    }

    // guard:no-backend — tests only the storage-directory file lock.
    #[tokio::test]
    async fn storage_directory_guard_excludes_other_openers_until_drop() {
        let dir = tempfile::tempdir().unwrap();
        let guard = DatabaseLockGuard::acquire(dir.path()).await.unwrap();
        let rival = OpenOptions::new()
            .append(true)
            .open(dir.path().join("database.lock"))
            .unwrap();
        assert!(rival.try_lock().is_err());
        drop(guard);
        rival.try_lock().unwrap();
    }
}
