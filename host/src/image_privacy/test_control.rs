//! Test-only controls for the real application child. Production builds do not
//! compile these controls; a test-built application still refuses ordinary serve.
use std::io::{self, Write};
use std::path::Path;
use std::sync::Arc;

use tokio::sync::{Notify, watch};

use super::PreparationError;
use super::wire::{Failure, Status};

pub const READY_FILE: &str = "worker-test-ready";
const MODE_FILE: &str = "worker-test-mode";

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum WorkerTestMode {
    Paused,
    OversizedResponse,
    ExpandedOutput,
    VerificationFailure,
}

pub struct WorkerTestControl {
    mode: WorkerTestMode,
    pid: watch::Sender<Option<u32>>,
    before_wait: Notify,
    resume_wait: Notify,
}

impl WorkerTestControl {
    #[must_use]
    pub fn new(mode: WorkerTestMode) -> Arc<Self> {
        Arc::new(Self {
            mode,
            pid: watch::channel(None).0,
            before_wait: Notify::new(),
            resume_wait: Notify::new(),
        })
    }

    /// Waits until the real worker reports its process ID.
    ///
    /// # Errors
    ///
    /// Returns `Protocol` if the worker owner ends before reporting its process ID.
    pub async fn started(&self) -> Result<u32, PreparationError> {
        let mut receiver = self.pid.subscribe();
        let pid = *receiver
            .wait_for(Option::is_some)
            .await
            .map_err(|_| PreparationError::Protocol)?;
        pid.ok_or(PreparationError::Protocol)
    }

    pub async fn waiting_for_reap(&self) {
        self.before_wait.notified().await;
    }
    pub fn release_reap(&self) {
        self.resume_wait.notify_one();
    }

    pub(super) fn install(&self, spool: &Path) -> io::Result<()> {
        private_file(
            &spool.join(MODE_FILE),
            match self.mode {
                WorkerTestMode::Paused => b"pause",
                WorkerTestMode::OversizedResponse => b"oversize",
                WorkerTestMode::ExpandedOutput => b"expanded-output",
                WorkerTestMode::VerificationFailure => b"verification-failure",
            },
        )
    }

    pub(super) fn spawned(&self, pid: u32) {
        let _ = self.pid.send_replace(Some(pid));
    }

    pub(super) async fn before_reap(&self) {
        if self.mode == WorkerTestMode::Paused {
            self.before_wait.notify_one();
            self.resume_wait.notified().await;
        }
    }
}

fn private_file(path: &Path, bytes: &[u8]) -> io::Result<()> {
    let mut options = std::fs::OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    options.open(path)?.write_all(bytes)
}

/// A fixed test-owned marker controls only the private worker, not uploads or
/// the program selected by a caller. Real successful privacy tests set no marker.
pub(super) fn worker_override(spool: &Path) -> io::Result<Option<bool>> {
    let mode = match std::fs::read_to_string(spool.join(MODE_FILE)) {
        Ok(mode) => mode,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(error),
    };
    private_file(&spool.join(READY_FILE), b"ready")?;
    match mode.as_str() {
        "pause" => loop {
            std::thread::park();
        },
        "oversize" => {
            // The JSON is otherwise usable. Invalid JSON would let a parser
            // failure pass the bound test even if the byte limit were absent.
            let mut stdout = io::stdout().lock();
            stdout.write_all(&vec![b' '; 128 * 1024])?;
            stdout.write_all(br#"{"version":1,"result":{"Ok":null}}"#)?;
            stdout.flush()?;
            Ok(Some(true))
        }
        "expanded-output" | "verification-failure" => {
            // Fixed fault witnesses in the real child, never alternate production
            // preparation. Expansion is exactly one byte, not a growth search.
            let result = if mode == "expanded-output" {
                std::fs::copy(spool.join("input"), spool.join("output"))?;
                std::fs::OpenOptions::new()
                    .append(true)
                    .open(spool.join("output"))?
                    .write_all(&[0])?;
                Ok(Some("image/jpeg".to_owned()))
            } else {
                Err(Failure::Verification)
            };
            let succeeded = result.is_ok();
            let mut stdout = io::stdout().lock();
            serde_json::to_writer(&mut stdout, &Status { version: 1, result })?;
            stdout.flush()?;
            Ok(Some(succeeded))
        }
        _ => Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "unknown image worker test control",
        )),
    }
}
