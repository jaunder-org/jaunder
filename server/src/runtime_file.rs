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

/// RAII guard: writes the runtime file on construction, removes it on `Drop`.
///
/// Removal is best-effort and only runs on a normal unwind (a `SIGKILL` skips
/// `Drop`); making removal signal-robust is a deferred follow-on (#140).
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
}

impl Drop for RuntimeFileGuard {
    fn drop(&mut self) {
        if let Some(p) = &self.path {
            let _ = std::fs::remove_file(p);
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
}
