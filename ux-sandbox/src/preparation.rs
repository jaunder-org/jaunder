//! Build/select compatible debug artifacts under the existing checkout artifact lock.
use crate::{
    session::{self, Signals},
    workspace,
};
use anyhow::{Context, ensure};
use sha2::{Digest, Sha256};
use std::{
    fs::{self, File, OpenOptions},
    io::Read,
    os::unix::fs::PermissionsExt,
    path::{Path, PathBuf},
};
use tokio::process::Command;

pub struct Artifacts {
    pub jaunder: PathBuf,
    pub support: Option<PathBuf>,
    pub digest: String,
}
pub enum Prepared {
    Ready(Artifacts),
    Exited(i32),
}

struct ArtifactLock {
    file: File,
    #[cfg(test)]
    fail_acquire: bool,
}
impl ArtifactLock {
    fn try_lock(&mut self) -> Result<(), fs::TryLockError> {
        #[cfg(test)]
        if std::mem::take(&mut self.fail_acquire) {
            return Err(fs::TryLockError::Error(std::io::Error::new(
                std::io::ErrorKind::PermissionDenied,
                "one-shot artifact lock failure",
            )));
        }
        self.file.try_lock()
    }
    async fn acquire(&mut self, path: &Path, signals: &mut Signals) -> anyhow::Result<Option<i32>> {
        let mut waiting = false;
        loop {
            match self.try_lock() {
                Ok(()) => return Ok(None),
                Err(fs::TryLockError::WouldBlock) => {
                    if !waiting {
                        eprintln!("Waiting for host artifacts: {}", path.display());
                        waiting = true;
                    }
                    tokio::select! {
                        code = signals.next() => return Ok(Some(code)),
                        () = tokio::time::sleep(std::time::Duration::from_millis(50)) => {}
                    }
                }
                Err(fs::TryLockError::Error(error)) => return Err(error.into()),
            }
        }
    }
}

pub async fn prepare(server: bool, signals: &mut Signals) -> anyhow::Result<Prepared> {
    let root = std::env::current_dir()?.canonicalize()?;
    workspace::directory(&root.join(".xtask"))?;
    let lock_path = root.join(".xtask/host-artifacts.lock");
    workspace::regular(&lock_path)?;
    let file = OpenOptions::new()
        .create(true)
        .truncate(false)
        .read(true)
        .write(true)
        .open(&lock_path)?;
    let mut lock = ArtifactLock {
        file,
        #[cfg(test)]
        fail_acquire: false,
    };
    if let Some(code) = lock.acquire(&lock_path, signals).await? {
        return Ok(Prepared::Exited(code));
    }
    let mut builds = Vec::new();
    if server {
        builds.push(vec!["just", "build-csr", "debug"]);
    }
    builds.push(vec!["cargo", "build", "--locked", "-p", "jaunder"]);
    if server {
        builds.push(vec!["cargo", "build", "--locked", "-p", "test-support"]);
    }
    for argv in builds {
        let mut command = Command::new(argv[0]);
        command
            .args(&argv[1..])
            .env("CARGO_TARGET_DIR", root.join("target"))
            .env_remove("JAUNDER_CSR_BUNDLE_DIR")
            .env_remove("JAUNDER_PUBLIC_DIR");
        let code = session::run(command, None, signals).await?;
        if code != 0 {
            lock.file.unlock()?;
            return Ok(Prepared::Exited(code));
        }
    }
    let jaunder = root.join("target/debug/jaunder");
    let support = server.then(|| root.join("target/debug/test-support"));
    for path in std::iter::once(&jaunder).chain(support.iter()) {
        let metadata =
            fs::metadata(path).with_context(|| format!("selecting {}", path.display()))?;
        ensure!(
            metadata.is_file() && metadata.permissions().mode() & 0o111 != 0,
            "not an executable: {}",
            path.display()
        );
        eprintln!("Selected {}", path.display());
    }
    if server {
        ensure!(
            root.join("target/site/.vite/manifest.json").is_file(),
            "CSR build did not publish its debug site"
        );
    }
    // Fingerprint the selected generation while the producer still has exclusion.
    let digest = fingerprint(&jaunder)?;
    lock.file.unlock()?;
    Ok(Prepared::Ready(Artifacts {
        jaunder,
        support,
        digest,
    }))
}

fn fingerprint(path: &Path) -> anyhow::Result<String> {
    let mut file =
        File::open(path).with_context(|| format!("fingerprinting {}", path.display()))?;
    let mut digest = Sha256::new();
    let mut buffer = [0_u8; 16 * 1024];
    loop {
        let count = file.read(&mut buffer)?;
        if count == 0 {
            break;
        }
        digest.update(&buffer[..count]);
    }
    let mut hex = String::with_capacity(64);
    for byte in digest.finalize() {
        use std::fmt::Write;
        write!(&mut hex, "{byte:02x}")?;
    }
    Ok(hex)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn artifact_lock_error_is_not_cancellation_and_releases_the_descriptor() {
        let file = tempfile::NamedTempFile::new().unwrap();
        let mut lock = ArtifactLock {
            file: file.reopen().unwrap(),
            fail_acquire: true,
        };
        let mut signals = Signals::new().unwrap();
        let error = lock.acquire(file.path(), &mut signals).await.unwrap_err();
        assert_eq!(
            error.downcast_ref::<std::io::Error>().unwrap().kind(),
            std::io::ErrorKind::PermissionDenied
        );
        assert!(signals.status().is_none());
        drop(lock);
        let mut retry = ArtifactLock {
            file: file.reopen().unwrap(),
            fail_acquire: false,
        };
        assert_eq!(
            retry.acquire(file.path(), &mut signals).await.unwrap(),
            None
        );
        assert!(matches!(
            file.as_file().try_lock(),
            Err(fs::TryLockError::WouldBlock)
        ));
        retry.file.unlock().unwrap();
        file.as_file().try_lock().unwrap();
    }
}
