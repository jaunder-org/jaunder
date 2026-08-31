//! The `serve` runtime-info file — a small JSON file recording the bound address
//! so an ephemeral (`--bind …:0`) server is discoverable by an out-of-process
//! caller (the elisp test harness). See ADR-0035.
//!
//! Contents: `{ "ip": <ip>, "port": <port>, "pid": <pid>, "start_time": <jiffies> }`.
//! The JSON file remains the out-of-process discovery contract. The adjacent
//! OS-backed `.lock` file is the startup mutex: its kernel lock, not its
//! on-disk existence, determines whether another live instance owns the data
//! directory. We still inspect the JSON identity during rollout so a live legacy
//! process without the lock remains protected.

use anyhow::{Context, Result};
use host::error;
use std::fs::{self, File, OpenOptions};
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

/// Reads a process's start-time from `/proc/<pid>/stat` (the `/proc` binding over
/// [`read_start_time_at`]). `Ok(None)` = no such pid (dead); `Err` = unusable `/proc`.
pub(crate) fn read_proc_start_time(pid: u32) -> std::io::Result<Option<u64>> {
    read_start_time_at(Path::new(&format!("/proc/{pid}/stat")))
}

/// `Ok(true)` iff `pid` is live **and** its start-time matches `recorded` — i.e. the
/// exact process that wrote the runtime file is still running. `Ok(false)` = dead pid
/// or start-time mismatch (a recycled pid). `Err` = unusable `/proc` (hard fail).
pub(crate) fn holder_is_live(pid: u32, recorded: u64) -> std::io::Result<bool> {
    Ok(match read_proc_start_time(pid)? {
        Some(actual) => actual == recorded,
        None => false,
    })
}

/// The canonical runtime-file path under a storage directory. This is the
/// legacy-visible reservation and the basis for the process-wide startup lock;
/// `--runtime-file` affects only additional discovery output.
pub(crate) fn canonical_runtime_path(storage_path: &Path) -> PathBuf {
    storage_path.join("runtime.json")
}

/// The optional discovery-file path: the explicit `override_path`, else the
/// canonical legacy-visible runtime file.
pub(crate) fn resolve_runtime_path(override_path: Option<PathBuf>, storage_path: &Path) -> PathBuf {
    override_path.unwrap_or_else(|| canonical_runtime_path(storage_path))
}

/// Outcome of the start-up mutex check.
pub(crate) enum StartupCheck {
    /// A live writer holds the file — refuse to start (its pid).
    Refuse { pid: u32 },
    /// The recorded process is gone, or the file is unusable — warn and overwrite.
    Stale,
    /// No runtime file — a fresh start.
    Proceed,
}

/// Reads the runtime file at `path` and decides whether a live writer holds it.
/// A corrupt / legacy / missing-field `runtime.json` is `Stale` (our own file,
/// non-authoritative if damaged); a `/proc` failure while probing the holder
/// propagates as `Err` (hard fail). Emits the stale `WARN` here, so callers only
/// map the outcome.
pub(crate) fn check_startup_mutex(path: &Path) -> std::io::Result<StartupCheck> {
    let contents = match std::fs::read_to_string(path) {
        Ok(contents) => contents,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            return Ok(StartupCheck::Proceed);
        }
        Err(error) => {
            error::report_swallowed(
                host::error::ErrorKind::Internal,
                host::error::ErrorClass::Transient,
                "server.runtime_file.read",
                host::error::SwallowedSource::Error(&error),
            );
            return Ok(StartupCheck::Stale);
        }
    };
    let decoded = serde_json::from_str::<serde_json::Value>(&contents);
    let (pid, recorded) = match decoded {
        Ok(value) => {
            let pid = value["pid"]
                .as_u64()
                .and_then(|pid| u32::try_from(pid).ok());
            let recorded = value["start_time"].as_u64();
            if let (Some(pid), Some(recorded)) = (pid, recorded) {
                (pid, recorded)
            } else {
                let error = std::io::Error::new(
                    std::io::ErrorKind::InvalidData,
                    "runtime file has invalid identity fields",
                );
                report_runtime_decode(&error);
                return Ok(StartupCheck::Stale);
            }
        }
        Err(error) => {
            report_runtime_decode(&error);
            return Ok(StartupCheck::Stale);
        }
    };
    Ok(if holder_is_live(pid, recorded)? {
        StartupCheck::Refuse { pid }
    } else {
        tracing::warn!(path = %path.display(), pid, "stale runtime file (gone); overwriting");
        StartupCheck::Stale
    })
}

fn report_runtime_decode(error: &(dyn std::error::Error + 'static)) {
    error::report_swallowed(
        host::error::ErrorKind::Internal,
        host::error::ErrorClass::Bug,
        "server.runtime_file.decode",
        host::error::SwallowedSource::Error(error),
    );
}

fn remove_file(path: &Path) -> std::io::Result<()> {
    std::fs::remove_file(path)
}

/// Best-effort removal of the runtime file at `path`, ignoring errors (it may
/// already be gone). Shared by `RuntimeFileGuard::drop` and the forced-shutdown
/// path in `cmd_serve`, which must remove explicitly because `process::exit`
/// skips `Drop`.
pub(crate) fn remove_runtime_file(path: &Path) {
    remove_runtime_file_with(path, remove_file);
}

fn remove_runtime_file_with(path: &Path, remove: fn(&Path) -> std::io::Result<()>) {
    match remove(path) {
        Ok(()) => {}
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(error) => error::report_swallowed(
            host::error::ErrorKind::Internal,
            host::error::ErrorClass::Transient,
            "server.runtime_file.remove",
            host::error::SwallowedSource::Error(&error),
        ),
    }
}

/// The advisory-lock path associated with the canonical storage runtime file.
///
/// Unlike the discoverability file, this path is never removed: kernel-managed
/// file locks are released when a process dies, so a leftover file is harmless.
fn lock_path(storage_path: &Path) -> PathBuf {
    canonical_runtime_path(storage_path).with_extension("lock")
}

/// An OS-backed exclusive lock held from before temporary-upload cleanup until
/// shutdown. Its file may outlive its holder; only the live kernel lock matters.
pub struct StartupLockGuard {
    _file: File,
}

impl StartupLockGuard {
    /// Acquires the exclusive lock for `storage_path`, failing rather than
    /// continuing when another live process owns that data directory.
    ///
    /// # Errors
    ///
    /// Returns an error when the lock directory or file cannot be created, or
    /// when another live process already holds the exclusive lock.
    pub fn acquire(storage_path: &Path) -> Result<Self> {
        fs::create_dir_all(storage_path).with_context(|| {
            format!(
                "cannot create startup lock directory {}",
                storage_path.display()
            )
        })?;
        let path = lock_path(storage_path);
        let file = OpenOptions::new()
            .create(true)
            .append(true)
            .open(&path)
            .with_context(|| format!("cannot open startup lock {}", path.display()))?;
        file.try_lock()
            .with_context(|| format!("cannot acquire exclusive startup lock {}", path.display()))?;
        Ok(Self { _file: file })
    }

    /// Publishes the canonical reservation and, when distinct, the discovery
    /// override while retaining the startup lock for their lifetime.
    ///
    /// # Errors
    ///
    /// Returns an error if either runtime file cannot be reserved.
    pub fn reserve(
        self,
        reservation_path: PathBuf,
        discovery_path: PathBuf,
        addr: SocketAddr,
        start_time: u64,
    ) -> Result<RuntimeGuard> {
        let reservation = RuntimeFileGuard::reserve(reservation_path, addr, start_time)?;
        let discovery = if reservation.path() == Some(discovery_path.as_path()) {
            None
        } else {
            Some(RuntimeFileGuard::reserve(discovery_path, addr, start_time)?)
        };
        Ok(RuntimeGuard {
            reservation,
            discovery,
            _lock: self,
        })
    }
}

/// Holds both the OS-backed startup lock and runtime-file publication. The
/// reservation drops before the lock, so legacy discovery disappears before a
/// new process may acquire the data-directory lock.
pub struct RuntimeGuard {
    reservation: RuntimeFileGuard,
    discovery: Option<RuntimeFileGuard>,
    _lock: StartupLockGuard,
}

impl RuntimeGuard {
    /// Updates the reservations with the bound listener address. Address
    /// publication is best-effort only after both live identities exist.
    pub fn update_addresses(&self, addr: SocketAddr, start_time: u64) {
        self.reservation.update(addr, start_time);
        if let Some(discovery) = &self.discovery {
            discovery.update(addr, start_time);
        }
    }

    /// Every active runtime-file path, for forced-shutdown removal.
    #[must_use]
    pub fn paths(&self) -> Vec<PathBuf> {
        [
            self.reservation.path(),
            self.discovery.as_ref().and_then(RuntimeFileGuard::path),
        ]
        .into_iter()
        .flatten()
        .map(Path::to_path_buf)
        .collect()
    }
}

/// RAII guard for the discoverability file: writes it on construction and
/// removes it on `Drop`.
///
/// Removal is signal-robust on a normal service stop (#140): the graceful
/// shutdown hook in `cmd_serve` lets the serve loop return so `Drop` runs on
/// `SIGINT`/`SIGTERM`, and its forced-exit path removes the file explicitly via
/// [`remove_runtime_file`] before `process::exit`. A hard `SIGKILL` still skips
/// both, but releases [`StartupLockGuard`]'s OS lock automatically.
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
            Err(error) => {
                error::report_swallowed(
                    host::error::ErrorKind::Internal,
                    host::error::ErrorClass::Transient,
                    "server.runtime_file.write",
                    host::error::SwallowedSource::Error(&error),
                );
                Self { path: None }
            }
        }
    }

    /// Writes the initial legacy-visible runtime reservation. Unlike ordinary
    /// discovery publication, failure is fatal because startup cleanup must not
    /// run unless another process can observe this live identity.
    fn reserve(path: PathBuf, addr: SocketAddr, start_time: u64) -> Result<Self> {
        write_atomic(&path, addr, start_time).with_context(|| {
            format!("cannot publish live runtime reservation {}", path.display())
        })?;
        Ok(Self { path: Some(path) })
    }

    /// Replaces the reservation's provisional address after listener binding.
    /// The existing live reservation remains valid if the atomic replacement
    /// fails, so preserve ordinary discovery publication's best-effort behavior.
    fn update(&self, addr: SocketAddr, start_time: u64) {
        let Some(path) = self.path() else {
            return;
        };
        if let Err(error) = write_atomic(path, addr, start_time) {
            error::report_swallowed(
                host::error::ErrorKind::Internal,
                host::error::ErrorClass::Transient,
                "server.runtime_file.write",
                host::error::SwallowedSource::Error(&error),
            );
        }
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

    #[derive(Clone)]
    struct SharedWriter(std::sync::Arc<std::sync::Mutex<Vec<u8>>>);

    impl std::io::Write for SharedWriter {
        fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
            self.0.lock().expect("trace lock").extend_from_slice(bytes);
            Ok(bytes.len())
        }

        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }

    impl<'writer> tracing_subscriber::fmt::MakeWriter<'writer> for SharedWriter {
        type Writer = Self;

        fn make_writer(&'writer self) -> Self::Writer {
            self.clone()
        }
    }

    fn capture<T>(operation: impl FnOnce() -> T) -> (T, String) {
        let output = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
        let subscriber = tracing_subscriber::fmt()
            .json()
            .with_ansi(false)
            .with_writer(SharedWriter(output.clone()))
            .finish();
        let value = tracing::subscriber::with_default(subscriber, operation);
        std::io::Write::flush(&mut SharedWriter(output.clone())).expect("flush trace");
        let text =
            String::from_utf8(output.lock().expect("trace lock").clone()).expect("utf8 trace");
        (value, text)
    }

    fn assert_one_report(trace: &str, context: &str) {
        assert_eq!(
            trace.matches(r#""error.disposition":"swallowed""#).count(),
            1,
            "trace: {trace}"
        );
        assert!(
            trace.contains(&format!(r#""error.context":"{context}""#)),
            "trace: {trace}"
        );
    }

    #[test]
    fn startup_lock_excludes_concurrent_contender() {
        let dir = TempDir::new().unwrap();
        let first = StartupLockGuard::acquire(dir.path()).expect("first lock");

        let second = StartupLockGuard::acquire(dir.path());
        assert!(
            second.is_err(),
            "a live holder must prevent another contender from starting"
        );

        drop(first);
        StartupLockGuard::acquire(dir.path()).expect("released lock is acquirable");
    }

    #[test]
    fn startup_lock_ignores_distinct_discovery_overrides() {
        let dir = TempDir::new().unwrap();
        let first_path = resolve_runtime_path(Some(dir.path().join("first.json")), dir.path());
        let second_path = resolve_runtime_path(Some(dir.path().join("second.json")), dir.path());
        assert_ne!(
            first_path, second_path,
            "test needs distinct discovery paths"
        );

        let first = StartupLockGuard::acquire(dir.path()).expect("first lock");
        assert!(
            StartupLockGuard::acquire(dir.path()).is_err(),
            "distinct discovery overrides must contend for one storage lock"
        );
        drop(first);
    }

    #[test]
    fn stale_startup_lock_file_does_not_block_acquisition() {
        let dir = TempDir::new().unwrap();
        let lock_path = lock_path(dir.path());
        fs::write(&lock_path, "left by a dead process").unwrap();

        let lock = StartupLockGuard::acquire(dir.path()).expect("stale file is not a lock");
        drop(lock);
        assert!(
            lock_path.exists(),
            "lock-file cleanup is neither needed nor safe"
        );
        StartupLockGuard::acquire(dir.path()).expect("stale file remains harmless");
    }

    #[test]
    fn startup_lock_acquire_keeps_directory_creation_error_context() {
        let dir = TempDir::new().unwrap();
        let storage_path = dir.path().join("not-a-directory");
        fs::write(&storage_path, "ordinary file").unwrap();

        let error = StartupLockGuard::acquire(&storage_path)
            .err()
            .expect("a file cannot serve as the startup lock directory");

        assert_eq!(
            error.to_string(),
            format!(
                "cannot create startup lock directory {}",
                storage_path.display()
            )
        );
        assert!(
            error.chain().any(|source| matches!(
                source.downcast_ref::<std::io::Error>(),
                Some(error) if error.kind() == std::io::ErrorKind::AlreadyExists
            )),
            "directory creation failure must retain its typed I/O source: {error:#}"
        );
        assert!(
            storage_path.is_file(),
            "acquisition must not replace the file"
        );
    }

    #[test]
    fn startup_lock_reservation_write_failure_is_fatal_and_contextual() {
        let dir = TempDir::new().unwrap();
        let reservation_path = dir.path().join("reservation");
        let discovery_path = dir.path().join("discovery.json");
        fs::create_dir(&reservation_path).unwrap();
        let lock = StartupLockGuard::acquire(dir.path()).expect("startup lock");

        let error = lock
            .reserve(
                reservation_path.clone(),
                discovery_path.clone(),
                addr(),
                own_start_time(),
            )
            .err()
            .expect("a directory cannot receive a runtime reservation");

        assert_eq!(
            error.to_string(),
            format!(
                "cannot publish live runtime reservation {}",
                reservation_path.display()
            )
        );
        assert!(
            error
                .chain()
                .any(|source| source.downcast_ref::<std::io::Error>().is_some()),
            "fatal reservation failure must retain its typed I/O source: {error:#}"
        );
        assert!(
            reservation_path.is_dir(),
            "failed publication must not replace the reservation target"
        );
        assert!(
            !discovery_path.exists(),
            "a failed canonical reservation must not publish discovery output"
        );
        StartupLockGuard::acquire(dir.path())
            .expect("failed reservation drops its lock instead of blocking retry");
    }

    #[test]
    fn runtime_guard_removes_discovery_file_before_releasing_lock() {
        let dir = TempDir::new().unwrap();
        let runtime_path = canonical_runtime_path(dir.path());
        let lock = StartupLockGuard::acquire(dir.path()).expect("startup lock");
        let guard = lock
            .reserve(
                runtime_path.clone(),
                runtime_path.clone(),
                addr(),
                own_start_time(),
            )
            .expect("reservation");
        let reservation: serde_json::Value =
            serde_json::from_str(&fs::read_to_string(&runtime_path).expect("reservation file"))
                .expect("valid reservation");
        assert_eq!(reservation["pid"], std::process::id());
        assert!(reservation["start_time"].as_u64().is_some());

        drop(guard);
        assert!(
            !runtime_path.exists(),
            "guard preserves removal-on-drop behavior"
        );
        StartupLockGuard::acquire(dir.path()).expect("drop releases startup lock");
    }
    fn denied_remove(_: &Path) -> std::io::Result<()> {
        Err(std::io::Error::new(
            std::io::ErrorKind::PermissionDenied,
            "remove denied",
        ))
    }

    fn missing_remove(_: &Path) -> std::io::Result<()> {
        Err(std::io::Error::new(
            std::io::ErrorKind::NotFound,
            "already absent",
        ))
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
    fn removal_failure_reports_once_but_not_found_is_silent() {
        let path = Path::new("/private/runtime.json");
        let ((), trace) = capture(|| remove_runtime_file_with(path, denied_remove));
        assert_one_report(&trace, "server.runtime_file.remove");

        let ((), trace) = capture(|| remove_runtime_file_with(path, missing_remove));
        assert!(trace.is_empty(), "NotFound is expected: {trace}");
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
    fn inert_guard_update_is_a_silent_noop() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("missing").join("runtime.json");
        let guard = RuntimeFileGuard::write(path.clone(), addr(), 0);
        assert!(
            guard.path().is_none(),
            "failed initial publication is inert"
        );

        let ((), trace) = capture(|| guard.update(addr(), 1));

        assert!(
            trace.is_empty(),
            "an inert guard must not report updates: {trace}"
        );
        assert!(
            !path.exists(),
            "an inert update must not create its unavailable runtime file"
        );
        assert!(
            !path.with_extension("tmp").exists(),
            "an inert update must not attempt an atomic replacement"
        );
    }

    #[test]
    fn active_guard_update_failure_reports_and_preserves_live_reservation() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("runtime.json");
        let start_time = own_start_time();
        let guard = RuntimeFileGuard::reserve(path.clone(), addr(), start_time)
            .expect("initial live reservation");
        let original = fs::read(&path).expect("read initial live reservation");
        let tmp_path = path.with_extension("tmp");
        fs::create_dir(&tmp_path).expect("block atomic replacement write");
        let replacement_addr = SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), 45678);

        let ((), trace) = capture(|| guard.update(replacement_addr, start_time));

        assert_one_report(&trace, "server.runtime_file.write");
        assert_eq!(
            fs::read(&path).expect("read preserved live reservation"),
            original,
            "a failed address update must preserve the existing live identity"
        );
        assert!(
            tmp_path.is_dir(),
            "the failed update must not replace the blocking temporary path"
        );
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

    fn own_start_time() -> u64 {
        require_start_time_at(std::path::Path::new("/proc/self/stat")).unwrap()
    }

    fn write_runtime(path: &std::path::Path, json: &serde_json::Value) {
        std::fs::write(path, json.to_string()).unwrap();
    }

    #[test]
    fn read_proc_start_time_self_and_dead() {
        assert!(read_proc_start_time(std::process::id()).unwrap().is_some());
        // u32::MAX is above pid_max on any Linux system => /proc entry never exists.
        assert_eq!(read_proc_start_time(u32::MAX).unwrap(), None);
    }

    #[test]
    fn holder_is_live_matrix() {
        let me = std::process::id();
        assert!(holder_is_live(me, own_start_time()).unwrap()); // exact writer
        assert!(!holder_is_live(me, own_start_time() + 1).unwrap()); // pid reuse (start-time differs)
        assert!(!holder_is_live(u32::MAX, 0).unwrap()); // dead pid
    }

    #[test]
    fn check_startup_mutex_outcomes() {
        let dir = TempDir::new().unwrap();
        let p = dir.path().join("runtime.json");
        let me = std::process::id();

        // absent -> Proceed
        assert!(matches!(
            check_startup_mutex(&p).unwrap(),
            StartupCheck::Proceed
        ));

        // live writer (our pid + real start-time) -> Refuse { pid: me }
        write_runtime(
            &p,
            &serde_json::json!({"ip":"127.0.0.1","port":1,"pid":me,"start_time":own_start_time()}),
        );
        assert!(
            matches!(check_startup_mutex(&p).unwrap(), StartupCheck::Refuse { pid } if pid == me)
        );

        // our pid + wrong start-time (reuse) -> Stale
        write_runtime(
            &p,
            &serde_json::json!({"ip":"127.0.0.1","port":1,"pid":me,"start_time":own_start_time()+1}),
        );
        assert!(matches!(
            check_startup_mutex(&p).unwrap(),
            StartupCheck::Stale
        ));

        // dead pid -> Stale
        write_runtime(
            &p,
            &serde_json::json!({"ip":"127.0.0.1","port":1,"pid":u32::MAX,"start_time":0}),
        );
        assert!(matches!(
            check_startup_mutex(&p).unwrap(),
            StartupCheck::Stale
        ));

        // legacy {ip,port}-only -> Stale
        write_runtime(&p, &serde_json::json!({"ip":"127.0.0.1","port":1}));
        assert!(matches!(
            check_startup_mutex(&p).unwrap(),
            StartupCheck::Stale
        ));

        // corrupt JSON -> Stale
        std::fs::write(&p, "not json").unwrap();
        assert!(matches!(
            check_startup_mutex(&p).unwrap(),
            StartupCheck::Stale
        ));
    }

    #[test]
    fn startup_mutex_read_failure_is_stale_and_reports_once() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("runtime.json");
        std::fs::create_dir(&path).unwrap();
        let (result, trace) = capture(|| check_startup_mutex(&path));
        assert!(matches!(result.unwrap(), StartupCheck::Stale));
        assert_one_report(&trace, "server.runtime_file.read");
    }

    #[test]
    fn startup_mutex_decode_failure_is_stale_and_reports_once() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("runtime.json");
        std::fs::write(&path, "not json").unwrap();
        let (result, trace) = capture(|| check_startup_mutex(&path));
        assert!(matches!(result.unwrap(), StartupCheck::Stale));
        assert_one_report(&trace, "server.runtime_file.decode");
    }

    #[test]
    fn startup_mutex_not_found_proceeds_without_report() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("runtime.json");
        let (result, trace) = capture(|| check_startup_mutex(&path));
        assert!(matches!(result.unwrap(), StartupCheck::Proceed));
        assert!(trace.is_empty(), "NotFound is expected: {trace}");
    }
}
