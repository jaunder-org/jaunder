//! The `serve` runtime-info file — a small JSON file recording the bound address
//! so an ephemeral (`--bind …:0`) server is discoverable by an out-of-process
//! caller (the elisp test harness). See ADR-0035.
//!
//! Contents are intentionally minimal for now: `{ "ip": <ip>, "port": <port> }`.
//! Follow-ons add a `pid` (start-up mutex) and an `admin_token` (admin channel).

use std::net::SocketAddr;
use std::path::{Path, PathBuf};

/// Serializes `{ "ip", "port" }` and writes it to `path` atomically: write a
/// sibling `.tmp`, then rename (atomic on the same filesystem) so a reader never
/// observes a half-written file.
fn write_atomic(path: &Path, addr: SocketAddr) -> std::io::Result<()> {
    let body = serde_json::json!({
        "ip": addr.ip().to_string(),
        "port": addr.port(),
    })
    .to_string();
    let tmp = path.with_extension("tmp");
    std::fs::write(&tmp, body)?;
    std::fs::rename(&tmp, path)
}

/// Best-effort removal of the runtime file at `path`, ignoring errors (it may
/// already be gone). Shared by `RuntimeFileGuard::drop` and the forced-shutdown
/// path in `cmd_serve`, which must remove explicitly because `process::exit`
/// skips `Drop`.
pub(crate) fn remove_runtime_file(path: &Path) {
    let _ = std::fs::remove_file(path);
}

/// RAII guard: writes the runtime file on construction, removes it on `Drop`.
///
/// Removal is signal-robust on a normal service stop (#140): the graceful
/// shutdown hook in `cmd_serve` lets the serve loop return so `Drop` runs on
/// `SIGINT`/`SIGTERM`, and its forced-exit path removes the file explicitly via
/// [`remove_runtime_file`] before `process::exit`. A hard `SIGKILL` still skips
/// both (recovered by the #141 stale-detection follow-on).
pub struct RuntimeFileGuard {
    path: Option<PathBuf>,
}

impl RuntimeFileGuard {
    /// Writes the runtime file at `path` recording `addr`.
    ///
    /// Best-effort: on a write failure this logs and returns an inert guard, so
    /// a runtime-file problem never stops the server from serving.
    #[must_use]
    pub fn write(path: PathBuf, addr: SocketAddr) -> Self {
        match write_atomic(&path, addr) {
            Ok(()) => Self { path: Some(path) },
            Err(e) => {
                tracing::warn!(path = %path.display(), error = %e, "failed to write runtime file");
                Self { path: None }
            }
        }
    }

    /// Resolves the runtime-file path — the explicit `override_path`, else
    /// `<storage_path>/runtime.json` — and writes `addr` to it. Keeps the
    /// path-resolution branch out of the caller (`prepare_server`).
    #[must_use]
    pub fn for_serve(
        override_path: Option<PathBuf>,
        storage_path: &Path,
        addr: SocketAddr,
    ) -> Self {
        let path = override_path.unwrap_or_else(|| storage_path.join("runtime.json"));
        Self::write(path, addr)
    }

    /// The active runtime-file path, or `None` for an inert guard (write failed).
    /// Lets the shutdown supervisor clone the path before the guard is moved into
    /// the serve future, so the forced-exit path can remove it without the guard.
    #[must_use]
    pub fn path(&self) -> Option<&Path> {
        self.path.as_deref()
    }
}

impl Drop for RuntimeFileGuard {
    fn drop(&mut self) {
        if let Some(p) = &self.path {
            remove_runtime_file(p);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::net::{IpAddr, Ipv4Addr, SocketAddr};
    use tempfile::TempDir;

    fn addr() -> SocketAddr {
        SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), 34567)
    }

    #[test]
    fn writes_ip_and_port_json() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("runtime.json");
        let _guard = RuntimeFileGuard::write(path.clone(), addr());
        let v: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
        assert_eq!(v["ip"], "127.0.0.1");
        assert_eq!(v["port"], 34567);
    }

    #[test]
    fn removes_file_on_drop() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("runtime.json");
        let guard = RuntimeFileGuard::write(path.clone(), addr());
        assert!(path.exists());
        drop(guard);
        assert!(!path.exists());
    }

    #[test]
    fn write_failure_yields_inert_guard() {
        // A non-existent parent directory makes the atomic write fail.
        let path = std::path::Path::new("/nonexistent-jaunder-xyz/sub/runtime.json").to_path_buf();
        let guard = RuntimeFileGuard::write(path.clone(), addr());
        assert!(!path.exists());
        drop(guard); // inert: must not panic and must not create the file
        assert!(!path.exists());
    }

    #[test]
    fn for_serve_defaults_into_storage_dir() {
        let dir = TempDir::new().unwrap();
        let _guard = RuntimeFileGuard::for_serve(None, dir.path(), addr());
        assert!(dir.path().join("runtime.json").exists());
    }

    #[test]
    fn for_serve_honors_override() {
        let dir = TempDir::new().unwrap();
        let custom = dir.path().join("custom-runtime.json");
        let _guard = RuntimeFileGuard::for_serve(Some(custom.clone()), dir.path(), addr());
        assert!(custom.exists());
        assert!(!dir.path().join("runtime.json").exists());
    }

    #[test]
    fn remove_runtime_file_deletes_when_present() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("runtime.json");
        std::fs::write(&path, "{}").unwrap();
        assert!(path.exists());
        remove_runtime_file(&path);
        assert!(!path.exists());
    }

    #[test]
    fn remove_runtime_file_is_noop_when_absent() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("runtime.json");
        // Must not panic and must not create the file; idempotent on repeat.
        remove_runtime_file(&path);
        remove_runtime_file(&path);
        assert!(!path.exists());
    }

    #[test]
    fn path_is_some_for_active_guard_and_none_for_inert() {
        let dir = TempDir::new().unwrap();
        let active = RuntimeFileGuard::write(dir.path().join("runtime.json"), addr());
        assert!(active.path().is_some());
        let inert = RuntimeFileGuard::write(
            std::path::Path::new("/nonexistent-jaunder-xyz/sub/runtime.json").to_path_buf(),
            addr(),
        );
        assert!(inert.path().is_none());
    }
}
