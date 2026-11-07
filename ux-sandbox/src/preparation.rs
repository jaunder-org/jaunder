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

pub async fn prepare(server: bool, signals: &mut Signals) -> anyhow::Result<Prepared> {
    let root = std::env::current_dir()?.canonicalize()?;
    workspace::directory(&root.join(".xtask"))?;
    let lock_path = root.join(".xtask/host-artifacts.lock");
    workspace::regular(&lock_path)?;
    let lock = OpenOptions::new()
        .create(true)
        .truncate(false)
        .read(true)
        .write(true)
        .open(&lock_path)?;
    let mut waiting = false;
    loop {
        match lock.try_lock() {
            Ok(()) => break,
            Err(fs::TryLockError::WouldBlock) => {
                if !waiting {
                    eprintln!("Waiting for host artifacts: {}", lock_path.display());
                    waiting = true;
                }
                tokio::select! {
                    code = signals.next() => return Ok(Prepared::Exited(code)),
                    () = tokio::time::sleep(std::time::Duration::from_millis(50)) => {}
                }
            }
            Err(fs::TryLockError::Error(error)) => return Err(error.into()),
        }
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
            lock.unlock()?;
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
    lock.unlock()?;
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
