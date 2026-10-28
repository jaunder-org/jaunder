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

/// Park one child thread so its execution owner can test cancellation and reaping.
fn park_worker_once() {
    std::thread::park();
}

/// A fixed test-owned marker controls only the private worker, not uploads or
/// the program selected by a caller. Real successful privacy tests set no marker.
pub(super) fn worker_override(spool: &Path, output: &mut impl Write) -> io::Result<Option<bool>> {
    worker_override_with_pause_release(spool, output, || false)
}

fn worker_override_with_pause_release(
    spool: &Path,
    output: &mut impl Write,
    mut pause_release: impl FnMut() -> bool,
) -> io::Result<Option<bool>> {
    let mode = match std::fs::read_to_string(spool.join(MODE_FILE)) {
        Ok(mode) => mode,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(error),
    };
    private_file(&spool.join(READY_FILE), b"ready")?;
    match mode.as_str() {
        "pause" => loop {
            park_worker_once();
            if pause_release() {
                return Ok(Some(false));
            }
        },
        "oversize" => {
            // The JSON is otherwise usable. Invalid JSON would let a parser
            // failure pass the bound test even if the byte limit were absent.
            output.write_all(&vec![b' '; 128 * 1024])?;
            output.write_all(br#"{"version":1,"result":{"Ok":null}}"#)?;
            output.flush()?;
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
            serde_json::to_writer(&mut *output, &Status { version: 1, result })?;
            output.flush()?;
            Ok(Some(succeeded))
        }
        _ => Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "unknown image worker test control",
        )),
    }
}

#[cfg(test)]
mod tests {
    use super::{
        MODE_FILE, READY_FILE, WorkerTestControl, WorkerTestMode, private_file, worker_override,
        worker_override_with_pause_release,
    };
    use std::io;
    use std::sync::mpsc;

    fn spool() -> tempfile::TempDir {
        tempfile::tempdir().expect("private test spool")
    }

    fn set_mode(root: &std::path::Path, mode: &str) {
        private_file(&root.join(MODE_FILE), mode.as_bytes()).expect("write controlled mode");
    }

    #[test]
    fn missing_or_unreadable_control_marker_is_handled_explicitly() {
        let root = spool();
        let mut output = Vec::new();
        assert_eq!(worker_override(root.path(), &mut output).unwrap(), None);
        assert!(output.is_empty());

        std::fs::create_dir(root.path().join(MODE_FILE)).expect("directory blocks marker read");
        let error = worker_override(root.path(), &mut output).expect_err("read failure propagates");
        assert_eq!(error.kind(), io::ErrorKind::IsADirectory);
    }

    #[test]
    fn override_refuses_an_existing_ready_marker() {
        let root = spool();
        set_mode(root.path(), "oversize");
        std::fs::write(root.path().join(READY_FILE), b"already owned").expect("ready marker");
        let error = worker_override(root.path(), &mut Vec::new())
            .expect_err("the fixed marker must not be overwritten");
        assert_eq!(error.kind(), io::ErrorKind::AlreadyExists);
    }

    #[test]
    fn oversized_control_emits_valid_json_after_the_bounded_prefix() {
        let root = spool();
        set_mode(root.path(), "oversize");
        let mut output = Vec::new();
        assert_eq!(
            worker_override(root.path(), &mut output).unwrap(),
            Some(true)
        );
        assert_eq!(&output[..128 * 1024], vec![b' '; 128 * 1024]);
        let status: serde_json::Value = serde_json::from_slice(&output[128 * 1024..])
            .expect("oversized response still has valid wire JSON");
        assert_eq!(status["version"], 1);
        assert_eq!(status["result"]["Ok"], serde_json::Value::Null);
        assert!(root.path().join(READY_FILE).exists());
    }

    #[test]
    fn output_growth_and_verification_faults_use_the_worker_wire() {
        for (mode, expected_success) in [("expanded-output", true), ("verification-failure", false)]
        {
            let root = spool();
            set_mode(root.path(), mode);
            std::fs::write(root.path().join("input"), b"private image bytes")
                .expect("controlled input");
            let mut output = Vec::new();
            assert_eq!(
                worker_override(root.path(), &mut output).unwrap(),
                Some(expected_success)
            );
            let status: serde_json::Value =
                serde_json::from_slice(&output).expect("controlled response is valid JSON");
            if expected_success {
                assert_eq!(
                    std::fs::read(root.path().join("output")).unwrap(),
                    b"private image bytes\0"
                );
                assert_eq!(status["result"]["Ok"], "image/jpeg");
            } else {
                assert_eq!(status["result"]["Err"], "Verification");
            }
        }
    }

    #[test]
    fn unknown_control_is_rejected() {
        let root = spool();
        set_mode(root.path(), "unknown");
        assert_eq!(
            worker_override(root.path(), &mut Vec::new())
                .expect_err("closed test control set rejects unknown markers")
                .kind(),
            io::ErrorKind::InvalidInput
        );
    }

    #[test]
    fn paused_override_releases_only_through_its_private_unit_test_predicate() {
        let root = spool();
        set_mode(root.path(), "pause");
        let spool = root.path().to_path_buf();
        let (entered_tx, entered_rx) = mpsc::sync_channel(0);
        let (release_tx, release_rx) = mpsc::sync_channel(0);
        let worker = std::thread::spawn(move || {
            let mut output = Vec::new();
            let result = worker_override_with_pause_release(&spool, &mut output, || {
                entered_tx
                    .send(())
                    .expect("unit-test owner observes the actual pause arm");
                let release: bool = release_rx
                    .recv_timeout(std::time::Duration::from_secs(5))
                    .expect("unit-test owner answers pause predicate");
                if !release {
                    // Set the token after channel synchronization, so channel
                    // internals cannot consume the next loop's unpark token.
                    std::thread::current().unpark();
                }
                release
            });
            (result, output)
        });

        // The unpark token is retained if the worker has not reached park yet.
        worker.thread().unpark();
        entered_rx
            .recv_timeout(std::time::Duration::from_secs(5))
            .expect("ready marker is written and the Paused arm reaches its park");
        assert_eq!(
            std::fs::read(root.path().join(READY_FILE)).unwrap(),
            b"ready"
        );
        release_tx.send(false).expect("retain the paused worker");
        entered_rx
            .recv_timeout(std::time::Duration::from_secs(5))
            .expect("false release parks the same worker again");
        assert!(
            !worker.is_finished(),
            "false release cannot complete preparation"
        );
        release_tx
            .send(true)
            .expect("release the actual paused arm");
        let (result, output) = worker.join().expect("paused unit worker exits cleanly");
        assert_eq!(result.unwrap(), Some(false));
        assert!(output.is_empty());
    }

    #[test]
    fn before_reap_returns_immediately_for_non_paused_modes() {
        use std::future::Future;
        use std::pin::pin;
        use std::task::{Context, Poll, Waker};

        let mut context = Context::from_waker(Waker::noop());
        for mode in [
            WorkerTestMode::OversizedResponse,
            WorkerTestMode::ExpandedOutput,
            WorkerTestMode::VerificationFailure,
        ] {
            let control = WorkerTestControl::new(mode);
            let mut before_reap = pin!(control.before_reap());
            assert_eq!(before_reap.as_mut().poll(&mut context), Poll::Ready(()));
        }

        let control = WorkerTestControl::new(WorkerTestMode::Paused);
        let mut before_reap = pin!(control.before_reap());
        assert_eq!(before_reap.as_mut().poll(&mut context), Poll::Pending);
    }

    #[test]
    fn every_test_mode_installs_one_fixed_marker() {
        for mode in [
            WorkerTestMode::Paused,
            WorkerTestMode::OversizedResponse,
            WorkerTestMode::ExpandedOutput,
            WorkerTestMode::VerificationFailure,
        ] {
            let root = spool();
            WorkerTestControl::new(mode)
                .install(root.path())
                .expect("fixed enum mode installs");
            assert!(root.path().join(MODE_FILE).is_file());
        }
    }
}
