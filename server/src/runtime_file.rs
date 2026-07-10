//! The `serve` runtime-info file — a small JSON file recording the bound address
//! so an ephemeral (`--bind …:0`) server is discoverable by an out-of-process
//! caller (the elisp test harness). See ADR-0035.
//!
//! Contents: `{ "ip": <ip>, "port": <port>, "pid": <pid>, "start_time": <jiffies> }`.
//! The `pid` + `start_time` (from `/proc/<pid>/stat` field 22) identify the exact
//! writer *process* — the start-up mutex (#141) refuses to start when they name a
//! live instance and treats a dead/mismatched holder as stale. A further follow-on
//! adds an `admin_token` (admin channel, #142).

use std::net::SocketAddr;
use std::path::{Path, PathBuf};

/// Serializes `{ "ip", "port", "pid", "start_time" }` and writes it to `path`
/// atomically: write a sibling `.tmp`, then rename (atomic on the same filesystem)
/// so a reader never observes a half-written file. `start_time` is the writer's
/// `/proc/self/stat` field 22, read by the caller so a failure hard-fails startup.
fn write_atomic(path: &Path, addr: SocketAddr, start_time: u64) -> std::io::Result<()> {
    let body = serde_json::json!({
        "ip": addr.ip().to_string(),
        "port": addr.port(),
        "pid": std::process::id(),
        "start_time": start_time,
    })
    .to_string();
    let tmp = path.with_extension("tmp");
    std::fs::write(&tmp, body)?;
    std::fs::rename(&tmp, path)
}

/// Field 22 (start-time, jiffies since boot) of a `/proc/<pid>/stat` line;
/// `InvalidData` if malformed — a malformed stat is a hard failure in every caller.
/// Field 2 (`comm`) is paren-wrapped and may contain spaces and `)`, so parse from
/// the **last** `)` (via `rsplit_once`, not slice-indexing, so it can never panic on
/// a char boundary); after it, `split_whitespace` coalesces the leading space and
/// start-time is index 19 (the 20th field after `comm`).
pub(crate) fn parse_stat_start_time(stat: &str) -> std::io::Result<u64> {
    stat.rsplit_once(')')
        .and_then(|(_, after)| after.split_whitespace().nth(19))
        .and_then(|field| field.parse().ok())
        .ok_or_else(|| {
            std::io::Error::new(std::io::ErrorKind::InvalidData, "unparseable /proc stat")
        })
}

/// Reads a start-time from `path`. `Ok(Some)` when it reads and parses; `Ok(None)`
/// when it does not exist (`NotFound` — a dead pid for `/proc/<pid>/stat`); `Err`
/// on any other I/O error **or** an unparseable read (the `/proc` mechanism is
/// unusable → the caller hard-fails). Path is a parameter so tests exercise every
/// arm with planted files.
pub(crate) fn read_start_time_at(path: &Path) -> std::io::Result<Option<u64>> {
    match std::fs::read_to_string(path) {
        Ok(s) => Ok(Some(parse_stat_start_time(&s)?)),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(e) => Err(e),
    }
}

/// Reads a **required** start-time (our own, at startup): a missing file or read
/// error is a hard fail — a runtime that can't read `/proc` can't enforce the
/// start-up mutex, so it must refuse rather than serve with a broken guard.
pub(crate) fn require_start_time_at(path: &Path) -> anyhow::Result<u64> {
    read_start_time_at(path)?
        .ok_or_else(|| anyhow::anyhow!("cannot read own start-time from {}", path.display()))
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
    /// Writes the runtime file at `path` recording `addr` + our pid + `start_time`.
    ///
    /// Best-effort: on a write failure this logs and returns an inert guard, so
    /// a runtime-file problem never stops the server from serving.
    #[must_use]
    pub fn write(path: PathBuf, addr: SocketAddr, start_time: u64) -> Self {
        match write_atomic(&path, addr, start_time) {
            Ok(()) => Self { path: Some(path) },
            Err(e) => {
                tracing::warn!(path = %path.display(), error = %e, "failed to write runtime file");
                Self { path: None }
            }
        }
    }

    /// Resolves the runtime-file path — the explicit `override_path`, else
    /// `<storage_path>/runtime.json` — and writes `addr` + `start_time` to it.
    /// Keeps the path-resolution branch out of the caller (`prepare_server`).
    #[must_use]
    pub fn for_serve(
        override_path: Option<PathBuf>,
        storage_path: &Path,
        addr: SocketAddr,
        start_time: u64,
    ) -> Self {
        let path = override_path.unwrap_or_else(|| storage_path.join("runtime.json"));
        Self::write(path, addr, start_time)
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
        let _guard = RuntimeFileGuard::write(path.clone(), addr(), 0);
        let v: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
        assert_eq!(v["ip"], "127.0.0.1");
        assert_eq!(v["port"], 34567);
    }

    #[test]
    fn removes_file_on_drop() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("runtime.json");
        let guard = RuntimeFileGuard::write(path.clone(), addr(), 0);
        assert!(path.exists());
        drop(guard);
        assert!(!path.exists());
    }

    #[test]
    fn write_failure_yields_inert_guard() {
        // A non-existent parent directory makes the atomic write fail.
        let path = std::path::Path::new("/nonexistent-jaunder-xyz/sub/runtime.json").to_path_buf();
        let guard = RuntimeFileGuard::write(path.clone(), addr(), 0);
        assert!(!path.exists());
        drop(guard); // inert: must not panic and must not create the file
        assert!(!path.exists());
    }

    #[test]
    fn for_serve_defaults_into_storage_dir() {
        let dir = TempDir::new().unwrap();
        let _guard = RuntimeFileGuard::for_serve(None, dir.path(), addr(), 0);
        assert!(dir.path().join("runtime.json").exists());
    }

    #[test]
    fn for_serve_honors_override() {
        let dir = TempDir::new().unwrap();
        let custom = dir.path().join("custom-runtime.json");
        let _guard = RuntimeFileGuard::for_serve(Some(custom.clone()), dir.path(), addr(), 0);
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
        let active = RuntimeFileGuard::write(dir.path().join("runtime.json"), addr(), 0);
        assert!(active.path().is_some());
        let inert = RuntimeFileGuard::write(
            std::path::Path::new("/nonexistent-jaunder-xyz/sub/runtime.json").to_path_buf(),
            addr(),
            0,
        );
        assert!(inert.path().is_none());
    }

    #[test]
    fn parse_stat_start_time_reads_field_22() {
        // pid (comm may contain spaces and ')') state ppid ... field22 = starttime.
        let line = "1234 (jaunder blog) S 1 1234 1234 0 -1 4194560 100 0 0 0 \
                    1 2 0 0 20 0 1 0 987654 12345 0";
        assert_eq!(parse_stat_start_time(line).unwrap(), 987_654);
    }

    #[test]
    fn parse_stat_start_time_rejects_malformed() {
        assert!(parse_stat_start_time("").is_err());
        assert!(parse_stat_start_time("no parens here").is_err());
        assert!(parse_stat_start_time("1 (x) S 1").is_err()); // too few fields
                                                              // Non-numeric value AT field 22 (index 19: state + 18 fillers + token).
        assert!(
            parse_stat_start_time("1 (x) S 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 notnum").is_err()
        );
    }

    #[test]
    fn read_start_time_at_arms() {
        let dir = TempDir::new().unwrap();
        let ok = dir.path().join("stat");
        std::fs::write(&ok, "1 (x) S 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 555").unwrap();
        assert_eq!(read_start_time_at(&ok).unwrap(), Some(555));
        // Absent path -> Ok(None) (the dead-pid signal for /proc/<pid>/stat).
        assert_eq!(read_start_time_at(&dir.path().join("nope")).unwrap(), None);
        // Read succeeds but is unparseable -> Err (hard fail).
        let bad = dir.path().join("bad");
        std::fs::write(&bad, "garbage").unwrap();
        assert!(read_start_time_at(&bad).is_err());
        // A directory is a non-NotFound I/O error -> Err (hard fail).
        assert!(read_start_time_at(dir.path()).is_err());
    }

    #[test]
    fn require_start_time_at_arms() {
        let dir = TempDir::new().unwrap();
        let ok = dir.path().join("stat");
        std::fs::write(&ok, "1 (x) S 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 777").unwrap();
        assert_eq!(require_start_time_at(&ok).unwrap(), 777);
        // Absent -> Err (the None -> hard-fail mapping).
        assert!(require_start_time_at(&dir.path().join("nope")).is_err());
        // Our own real stat parses.
        assert!(require_start_time_at(std::path::Path::new("/proc/self/stat")).is_ok());
    }

    #[test]
    fn writes_pid_and_start_time_json() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("runtime.json");
        let _guard = RuntimeFileGuard::write(path.clone(), addr(), 4242);
        let v: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
        assert_eq!(v["ip"], "127.0.0.1");
        assert_eq!(v["port"], 34567);
        assert_eq!(v["pid"], std::process::id());
        assert_eq!(v["start_time"], 4242);
    }
}
