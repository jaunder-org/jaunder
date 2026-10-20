//! Two-slot process ownership. The tracked owner, not the HTTP future, holds
//! the spool and permit. Dropping the caller closes its lease; the owner kills
//! and waits before cleanup. Shutdown closes admission and joins every owner.
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use tokio::io::AsyncReadExt;
use tokio::process::{Child, Command};
use tokio::sync::{Semaphore, oneshot, watch};
use tokio::task::JoinSet;
use tokio::time::{Instant, timeout_at};

use super::native::WORKER_ARGUMENT;
use super::wire::{PreparationError, Status, private_io};

const RESPONSE_LIMIT: u64 = 128 * 1024;
const DEADLINE: Duration = Duration::from_secs(30);

pub struct UploadPreparer {
    executable: PathBuf,
    slots: Arc<Semaphore>,
    shutdown: watch::Sender<bool>,
    owners: Mutex<JoinSet<()>>,
    #[cfg(any(test, feature = "test-utils"))]
    test_control: Option<Arc<super::WorkerTestControl>>,
}

impl UploadPreparer {
    /// The composition root supplies the same deployed executable. Tests supply
    /// Cargo's explicit application binary, never their test harness executable.
    #[must_use]
    pub fn new(executable: PathBuf) -> Arc<Self> {
        Arc::new(Self::state(executable))
    }

    fn state(executable: PathBuf) -> Self {
        Self {
            executable,
            slots: Arc::new(Semaphore::new(2)),
            shutdown: watch::channel(false).0,
            owners: Mutex::new(JoinSet::new()),
            #[cfg(any(test, feature = "test-utils"))]
            test_control: None,
        }
    }

    #[cfg(any(test, feature = "test-utils"))]
    #[must_use]
    pub fn new_for_test(executable: PathBuf, control: Arc<super::WorkerTestControl>) -> Arc<Self> {
        let mut state = Self::state(executable);
        state.test_control = Some(control);
        Arc::new(state)
    }

    /// Takes a slot without queuing, before any private copy is created.
    ///
    /// # Errors
    ///
    /// Returns `Busy` or `Stopping` when admission is unavailable, and propagates
    /// errors from owner reaping or private-spool filesystem operations.
    pub fn admit(self: &Arc<Self>, root: &Path) -> Result<UploadAttempt, PreparationError> {
        let permit = self
            .slots
            .clone()
            .try_acquire_owned()
            .map_err(|error| match error {
                tokio::sync::TryAcquireError::Closed => PreparationError::Stopping,
                tokio::sync::TryAcquireError::NoPermits => PreparationError::Busy,
            })?;
        let mut owners = self.owners.lock().map_err(|_| PreparationError::Protocol)?;
        if self.slots.is_closed() {
            return Err(PreparationError::Stopping);
        }
        while let Some(result) = owners.try_join_next() {
            result?;
        }
        let tmp = root.join("media/tmp");
        std::fs::create_dir_all(&tmp)?;
        let spool = tempfile::Builder::new()
            .prefix("upload-")
            .tempdir_in(&tmp)
            .map_err(|error| PreparationError::Io(private_io(&error)))?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(spool.path(), std::fs::Permissions::from_mode(0o700))?;
        }
        let input = spool.path().join("input");
        let output = spool.path().join("output");
        let mut options = std::fs::OpenOptions::new();
        options.write(true).create_new(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(0o600);
        }
        let file = options.open(input)?;
        // The native writer replaces this file, but the enclosing private spool
        // protects it even if a library temporary inherits a wider umask.
        options.open(&output)?;
        #[cfg(any(test, feature = "test-utils"))]
        if let Some(control) = &self.test_control {
            control.install(spool.path())?;
        }
        let (start, start_rx) = oneshot::channel();
        let (lease, lease_rx) = oneshot::channel();
        let (result_tx, result) = oneshot::channel();
        let (done_tx, done) = oneshot::channel();
        let owner = Arc::clone(self);
        let mut shutdown = self.shutdown.subscribe();
        owners.spawn(async move {
            let mut result_tx = Some(result_tx);
            let mut lease_rx = lease_rx;
            let job = tokio::select! {
                biased;
                () = stopped(&mut shutdown) => {
                    // A start may already be queued. Unblock its caller before
                    // waiting for that caller to close reception/its lease.
                    if let Some(sender) = result_tx.take() {
                        let _ = sender.send(Err(PreparationError::Stopping));
                    }
                    let _ = (&mut lease_rx).await;
                    None
                },
                _ = &mut lease_rx => None,
                start = start_rx => start.ok(),
            };
            if let Some(mime) = job {
                let prepared = owner
                    .process(spool.path(), mime, &mut lease_rx, &mut shutdown)
                    .await;
                let succeeded = prepared.is_ok();
                if let Some(sender) = result_tx.take() {
                    let _ = sender.send(prepared);
                }
                if succeeded {
                    // Successful bytes remain private and owned through hashing,
                    // placement and registration, not just native processing.
                    let _ = lease_rx.await;
                }
            }
            let cleanup = cleanup_spool(spool).await;
            drop(permit);
            if let Err(Err(error)) = done_tx.send(cleanup) {
                crate::error::report_swallowed(
                    crate::error::ErrorKind::Internal,
                    crate::error::ErrorClass::Transient,
                    "media.private_spool.cleanup",
                    crate::error::SwallowedSource::Error(&error),
                );
            }
        });
        Ok(UploadAttempt {
            file: Some(file),
            start: Some(start),
            lease: Some(lease),
            result: Some(result),
            done: Some(done),
            output,
            stopping: self.shutdown.subscribe(),
        })
    }

    async fn process(
        &self,
        spool: &Path,
        mime: String,
        lease: &mut oneshot::Receiver<()>,
        shutdown: &mut watch::Receiver<bool>,
    ) -> Result<Option<String>, PreparationError> {
        // One clock includes spawn, edit, post-read, response and process exit.
        let deadline = Instant::now() + DEADLINE;
        if *shutdown.borrow() {
            return Err(PreparationError::Stopping);
        }
        let mut child = Command::new(&self.executable)
            .arg(WORKER_ARGUMENT)
            .arg(spool)
            .arg(mime)
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .kill_on_drop(true)
            .spawn()?;
        #[cfg(any(test, feature = "test-utils"))]
        if let Some(control) = &self.test_control
            && let Some(pid) = child.id()
        {
            control.spawned(pid);
        }
        let response = tokio::select! {
            biased;
            () = stopped(shutdown) => Err(PreparationError::Stopping),
            _ = lease => Err(PreparationError::Stopping),
            result = timeout_at(deadline, read_response(&mut child)) => {
                result.unwrap_or(Err(PreparationError::Deadline))
            }
        };
        match response {
            Err(primary) => {
                // start_kill alone is not reaping. Preserve both failures, but
                // return neither until actual process exit has been acknowledged.
                match self.stop_and_wait(&mut child).await {
                    Ok(()) => Err(primary),
                    Err(reap) => Err(PreparationError::Reap {
                        primary: Box::new(primary),
                        reap,
                    }),
                }
            }
            success => success,
        }
    }

    async fn stop_and_wait(&self, child: &mut Child) -> io::Result<()> {
        let mut failure = match child.try_wait() {
            Ok(Some(_)) => return Ok(()),
            Ok(None) => None,
            Err(error) => Some(error),
        };
        if let Err(error) = child.start_kill() {
            failure.get_or_insert(error);
        }
        #[cfg(any(test, feature = "test-utils"))]
        if let Some(control) = &self.test_control {
            control.before_reap().await;
        }
        loop {
            match child.wait().await {
                Ok(_) => {
                    return match failure {
                        Some(error) => Err(error),
                        None => Ok(()),
                    };
                }
                Err(error) => {
                    failure.get_or_insert(error);
                    // An uncertain wait is not an exit acknowledgement. Retain
                    // the child, spool and permit while retrying only reaping.
                    tokio::time::sleep(Duration::from_millis(100)).await;
                }
            }
        }
    }

    /// Refuses new uploads, cancels native jobs, and drains every tracked owner.
    /// The server holds its runtime lock until this returns.
    pub fn stop(&self) {
        self.slots.close();
        self.shutdown.send_replace(true);
    }

    /// Waits until every tracked owner has completed its cleanup.
    ///
    /// # Errors
    ///
    /// Returns `Protocol` if owner tracking is poisoned, or a join error if an
    /// owned job panicked.
    pub async fn shutdown(&self) -> Result<(), PreparationError> {
        self.stop();
        let mut owners = {
            let mut guard = self.owners.lock().map_err(|_| PreparationError::Protocol)?;
            std::mem::take(&mut *guard)
        };
        let mut failure = None;
        while let Some(result) = owners.join_next().await {
            if let Err(error) = result {
                failure = Some(error);
            }
        }
        match failure {
            Some(error) => Err(error.into()),
            None => Ok(()),
        }
    }
}

async fn cleanup_spool(spool: tempfile::TempDir) -> io::Result<()> {
    let mut failure = None;
    loop {
        match std::fs::remove_dir_all(spool.path()) {
            Ok(()) => break,
            Err(error) if error.kind() == io::ErrorKind::NotFound => break,
            Err(error) => {
                if failure.is_none() {
                    // Retain ownership and its slot until removal actually
                    // succeeds. Report the first safe cause, not private paths.
                    crate::error::report_swallowed(
                        crate::error::ErrorKind::Internal,
                        crate::error::ErrorClass::Transient,
                        "media.private_spool.cleanup_retry",
                        crate::error::SwallowedSource::Error(&error),
                    );
                    failure = Some(error);
                }
                tokio::time::sleep(Duration::from_millis(100)).await;
            }
        }
    }
    // No path remains to clean. Disarm the TempDir without another removal
    // attempt, while retaining the original typed failure for the caller.
    drop(spool.keep());
    match failure {
        Some(error) => Err(error),
        None => Ok(()),
    }
}

async fn stopped(receiver: &mut watch::Receiver<bool>) {
    let _ = receiver.wait_for(|stopping| *stopping).await;
}

async fn read_response(child: &mut Child) -> Result<Option<String>, PreparationError> {
    let stdout = child.stdout.take().ok_or(PreparationError::Protocol)?;
    let mut bytes = Vec::new();
    stdout
        .take(RESPONSE_LIMIT + 1)
        .read_to_end(&mut bytes)
        .await?;
    if bytes.len() as u64 > RESPONSE_LIMIT {
        return Err(PreparationError::Protocol);
    }
    let exit = child.wait().await?;
    decode_response(&bytes, exit)
}

// Decode only after bounded reception and acknowledged exit. Keeping wire
// consistency pure avoids launching another child for each malformed JSON case.
fn decode_response(
    bytes: &[u8],
    exit: std::process::ExitStatus,
) -> Result<Option<String>, PreparationError> {
    if !exit.success() && bytes.is_empty() {
        return Err(PreparationError::WorkerExit(exit));
    }
    let status: Status = serde_json::from_slice(bytes).map_err(|_| PreparationError::Protocol)?;
    if status.version != 1 {
        return Err(PreparationError::Protocol);
    }
    match (exit.success(), status.result) {
        (true, Ok(mime))
            if mime.as_deref().is_none_or(|value| {
                matches!(
                    value,
                    "image/jpeg"
                        | "image/png"
                        | "image/gif"
                        | "image/webp"
                        | "image/heic"
                        | "image/heif"
                )
            }) =>
        {
            Ok(mime)
        }
        (false, Err(error)) => Err(error.into()),
        _ => Err(PreparationError::Protocol),
    }
}

pub struct UploadAttempt {
    file: Option<std::fs::File>,
    start: Option<oneshot::Sender<String>>,
    lease: Option<oneshot::Sender<()>>,
    result: Option<oneshot::Receiver<Result<Option<String>, PreparationError>>>,
    done: Option<oneshot::Receiver<io::Result<()>>>,
    output: PathBuf,
    stopping: watch::Receiver<bool>,
}

impl UploadAttempt {
    #[must_use]
    pub fn stopping(&self) -> watch::Receiver<bool> {
        self.stopping.clone()
    }

    /// Writes a chunk to the private input file synchronously so cancellation
    /// cannot leave a blocking-file operation using its path during cleanup.
    ///
    /// # Errors
    ///
    /// Returns an I/O error if the attempt is no longer receiving or the private
    /// input write fails.
    pub fn write(&mut self, bytes: &[u8]) -> io::Result<()> {
        self.file
            .as_mut()
            .ok_or_else(|| io::Error::other("upload already prepared"))?
            .write_all(bytes)
    }

    /// Starts image processing and waits for verified worker output.
    ///
    /// # Errors
    ///
    /// Returns the worker's typed processing failure, or a protocol/stopping error
    /// if the owned worker cannot complete a valid response.
    pub async fn prepare(&mut self, mime: &str) -> Result<Option<String>, PreparationError> {
        if let Some(mut file) = self.file.take() {
            file.flush()?;
        }
        self.start
            .take()
            .ok_or(PreparationError::Protocol)?
            .send(mime.to_owned())
            .map_err(|_| PreparationError::Stopping)?;
        self.result
            .take()
            .ok_or(PreparationError::Protocol)?
            .await
            .map_err(|_| PreparationError::Protocol)?
    }

    #[must_use]
    pub fn output(&self) -> &Path {
        &self.output
    }

    /// Waits for owned cleanup on ordinary return. Drop uses the same lease
    /// closure but leaves the tracked owner responsible when the caller vanishes.
    ///
    /// # Errors
    ///
    /// Returns a protocol error if the owner acknowledgement is lost, or the
    /// private-spool cleanup error after the owner has completed cleanup.
    pub async fn finish(mut self) -> Result<(), PreparationError> {
        self.file.take();
        self.lease.take();
        self.done
            .take()
            .ok_or(PreparationError::Protocol)?
            .await
            .map_err(|_| PreparationError::Protocol)??;
        Ok(())
    }
}

impl Drop for UploadAttempt {
    fn drop(&mut self) {
        self.file.take();
        self.lease.take();
    }
}

#[cfg(all(test, unix))]
mod tests {
    use super::{PreparationError, decode_response};
    use std::error::Error as _;
    use std::os::unix::process::ExitStatusExt;
    use std::process::ExitStatus;

    #[test]
    fn worker_wire_requires_valid_version_mime_and_exit_agreement() {
        assert_eq!(
            decode_response(
                br#"{"version":1,"result":{"Ok":"image/jpeg"}}"#,
                ExitStatus::from_raw(0)
            )
            .unwrap()
            .as_deref(),
            Some("image/jpeg")
        );
        assert_eq!(
            decode_response(
                br#"{"version":1,"result":{"Ok":null}}"#,
                ExitStatus::from_raw(0)
            )
            .unwrap(),
            None
        );
        for (bytes, exit) in [
            (b"{".as_slice(), 0),
            (br#"{"version":2,"result":{"Ok":null}}"#.as_slice(), 0),
            (
                br#"{"version":1,"result":{"Ok":"text/plain"}}"#.as_slice(),
                0,
            ),
            (
                br#"{"version":1,"result":{"Ok":"image/jpeg"},"unknown":true}"#.as_slice(),
                0,
            ),
            (br#"{"version":1,"result":{"Other":null}}"#.as_slice(), 0),
            (
                br#"{"version":1,"result":{"Ok":"image/jpeg"}}"#.as_slice(),
                256,
            ),
            (
                br#"{"version":1,"result":{"Err":"Verification"}}"#.as_slice(),
                0,
            ),
            (
                br#"{"version":1,"result":{"Err":{"Io":{"kind":"FutureKind","os":null}}}}"#
                    .as_slice(),
                256,
            ),
        ] {
            assert!(matches!(
                decode_response(bytes, ExitStatus::from_raw(exit)),
                Err(PreparationError::Protocol)
            ));
        }
        assert!(matches!(
            decode_response(b"", ExitStatus::from_raw(256)),
            Err(PreparationError::WorkerExit(_))
        ));
        assert!(matches!(
            decode_response(
                br#"{"version":1,"result":{"Err":"Verification"}}"#,
                ExitStatus::from_raw(256)
            ),
            Err(PreparationError::Verification)
        ));
    }

    #[test]
    fn worker_wire_retains_the_underlying_native_io_cause() {
        let error = decode_response(
            br#"{"version":1,"result":{"Err":{"NativeIo":{"kind":"PermissionDenied","os":13}}}}"#,
            ExitStatus::from_raw(256),
        )
        .expect_err("native write/check I/O must stay typed");
        assert!(matches!(error, PreparationError::Native(_)));
        let io = error
            .source()
            .unwrap()
            .source()
            .unwrap()
            .downcast_ref::<std::io::Error>()
            .unwrap();
        assert_eq!(io.kind(), std::io::ErrorKind::PermissionDenied);
        assert_eq!(io.raw_os_error(), Some(13));
    }
}
