//! The bootstrap is separate; only protected artifact preparation holds this lock.
use crate::session::{self, Signals};
use anyhow::{Context, ensure};
use std::{
    fs::{self, OpenOptions},
    os::unix::fs::PermissionsExt,
    path::PathBuf,
};
use tokio::process::Command;

pub async fn prepare(signals: &mut Signals) -> anyhow::Result<(PathBuf, i32)> {
    let root = std::env::current_dir()?.canonicalize()?;
    fs::create_dir_all(root.join(".xtask"))?;
    let lock = OpenOptions::new()
        .create(true)
        .truncate(false)
        .read(true)
        .write(true)
        .open(root.join(".xtask/host-artifacts.lock"))?;
    // Nonblocking attempts keep signal handling on the one runtime while a peer owns builds.
    loop {
        match lock.try_lock() {
            Ok(()) => break,
            Err(fs::TryLockError::WouldBlock) => {
                // No children exist yet; cancellation can return without teardown.
                tokio::select! {
                    code = signals.next() => return Ok((root, code)),
                    () = tokio::time::sleep(std::time::Duration::from_millis(50)) => {}
                }
            }
            Err(fs::TryLockError::Error(error)) => return Err(error.into()),
        }
    }
    for argv in [
        vec!["just", "build-csr", "debug"],
        vec!["cargo", "build", "--locked", "-p", "jaunder"],
        vec!["cargo", "build", "--locked", "-p", "test-support"],
    ] {
        let mut command = Command::new(argv[0]);
        command
            .args(&argv[1..])
            .env_remove("CARGO_TARGET_DIR")
            .env_remove("JAUNDER_CSR_BUNDLE_DIR")
            .env_remove("JAUNDER_PUBLIC_DIR");
        let code = session::run(command, None, signals).await?;
        if code != 0 {
            lock.unlock()?;
            return Ok((root, code));
        }
    }
    for name in ["jaunder", "test-support"] {
        let path = root.join("target/debug").join(name);
        let metadata =
            fs::metadata(&path).with_context(|| format!("selecting {}", path.display()))?;
        ensure!(
            metadata.is_file() && metadata.permissions().mode() & 0o111 != 0,
            "not an executable: {}",
            path.display()
        );
        eprintln!("Selected {}", path.display());
    }
    ensure!(
        root.join("target/site/.vite/manifest.json").is_file(),
        "CSR build did not publish its debug site"
    );
    lock.unlock()?;
    Ok((root.join("target/debug/jaunder"), 0))
}
