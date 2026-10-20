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
        self.ensure_accepting()?;
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
            report_unobserved_cleanup_if_needed(done_tx.send(cleanup));
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

    fn ensure_accepting(&self) -> Result<(), PreparationError> {
        if self.slots.is_closed() {
            Err(PreparationError::Stopping)
        } else {
            Ok(())
        }
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
                Err(preserve_primary_after_reap(
                    primary,
                    self.stop_and_wait(&mut child).await,
                ))
            }
            success => success,
        }
    }

    async fn stop_and_wait(&self, child: &mut Child) -> io::Result<()> {
        let (exited, mut failure) = initial_reap_state(child.try_wait());
        if exited {
            return Ok(());
        }
        failure = preserve_first_reap_failure(failure, child.start_kill());
        #[cfg(any(test, feature = "test-utils"))]
        if let Some(control) = &self.test_control {
            control.before_reap().await;
        }
        loop {
            if let Some(result) =
                reap_wait_result(&mut failure, child.wait().await.map(|_| ())).await
            {
                return result;
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
        let owners = {
            let mut guard = self.owners.lock().map_err(|_| PreparationError::Protocol)?;
            std::mem::take(&mut *guard)
        };
        match join_finalizations(owners).await {
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

fn initial_reap_state(
    result: io::Result<Option<std::process::ExitStatus>>,
) -> (bool, Option<io::Error>) {
    match result {
        Ok(Some(_)) => (true, None),
        Ok(None) => (false, None),
        Err(error) => (false, Some(error)),
    }
}

async fn reap_wait_result(
    failure: &mut Option<io::Error>,
    result: io::Result<()>,
) -> Option<io::Result<()>> {
    match result {
        Ok(()) => Some(match failure.take() {
            Some(error) => Err(error),
            None => Ok(()),
        }),
        Err(error) => {
            *failure = Some(failure.take().unwrap_or(error));
            // An uncertain wait is not an exit acknowledgement. Retain the child,
            // spool and permit while retrying only reaping.
            retry_reap_delay().await;
            None
        }
    }
}

fn preserve_first_reap_failure(
    previous: Option<io::Error>,
    result: io::Result<()>,
) -> Option<io::Error> {
    match result {
        Ok(()) => previous,
        Err(error) => Some(previous.unwrap_or(error)),
    }
}

fn preserve_primary_after_reap(
    primary: PreparationError,
    reap: io::Result<()>,
) -> PreparationError {
    match reap {
        Ok(()) => primary,
        Err(reap) => PreparationError::Reap {
            primary: Box::new(primary),
            reap,
        },
    }
}

async fn retry_reap_delay() {
    tokio::time::sleep(Duration::from_millis(100)).await;
}

async fn join_finalizations(mut owners: JoinSet<()>) -> Option<tokio::task::JoinError> {
    let mut failure = None;
    while let Some(result) = owners.join_next().await {
        if let Err(error) = result {
            failure = Some(error);
        }
    }
    failure
}

fn report_unobserved_cleanup_if_needed(delivery: Result<(), io::Result<()>>) {
    if let Err(Err(_error)) = delivery {
        report_unobserved_cleanup();
    }
}

fn report_unobserved_cleanup() {
    crate::error::report_swallowed(
        crate::error::ErrorKind::Internal,
        crate::error::ErrorClass::Transient,
        "media.private_spool.cleanup",
        crate::error::SwallowedSource::Redacted,
    );
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
    use super::{
        PreparationError, UploadAttempt, UploadPreparer, decode_response, initial_reap_state,
        join_finalizations, preserve_first_reap_failure, preserve_primary_after_reap,
        reap_wait_result, report_unobserved_cleanup_if_needed,
    };
    use std::error::Error as _;
    use std::os::unix::process::ExitStatusExt;
    use std::process::ExitStatus;
    use std::sync::{Arc, Mutex};

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

    #[tokio::test]
    async fn process_state_classifiers_preserve_the_first_reap_failure() {
        assert!(matches!(
            initial_reap_state(Ok(Some(ExitStatus::from_raw(0)))),
            (true, None)
        ));
        assert!(matches!(initial_reap_state(Ok(None)), (false, None)));
        assert!(matches!(
            initial_reap_state(Err(std::io::Error::other("query"))),
            (false, Some(error)) if error.to_string() == "query"
        ));

        let mut failure = None;
        assert!(matches!(
            reap_wait_result(&mut failure, Ok(())).await,
            Some(Ok(()))
        ));
        assert!(failure.is_none());

        failure = Some(std::io::Error::other("kill"));
        assert!(
            reap_wait_result(&mut failure, Err(std::io::Error::other("wait")))
                .await
                .is_none()
        );
        assert_eq!(failure.as_ref().unwrap().to_string(), "kill");
        assert!(matches!(
            reap_wait_result(&mut failure, Ok(())).await,
            Some(Err(error)) if error.to_string() == "kill"
        ));

        failure = None;
        assert!(
            reap_wait_result(&mut failure, Err(std::io::Error::other("wait")))
                .await
                .is_none()
        );
        assert!(matches!(
            reap_wait_result(&mut failure, Ok(())).await,
            Some(Err(error)) if error.to_string() == "wait"
        ));

        let first = preserve_first_reap_failure(None, Err(std::io::Error::other("kill")));
        let first = preserve_first_reap_failure(first, Err(std::io::Error::other("wait")));
        assert_eq!(first.unwrap().to_string(), "kill");
        assert!(preserve_first_reap_failure(None, Ok(())).is_none());
    }

    #[test]
    fn primary_and_reap_failures_remain_both_typed() {
        assert!(matches!(
            preserve_primary_after_reap(PreparationError::Deadline, Ok(()),),
            PreparationError::Deadline
        ));
        assert!(matches!(
            preserve_primary_after_reap(
                PreparationError::Stopping,
                Err(std::io::Error::other("wait failed")),
            ),
            PreparationError::Reap { primary, reap }
                if matches!(*primary, PreparationError::Stopping)
                    && reap.to_string() == "wait failed"
        ));
    }

    #[test]
    fn closed_admission_is_rejected_and_process_does_not_spawn_after_shutdown() {
        let preparer = UploadPreparer::state("unused executable".into());
        assert!(preparer.ensure_accepting().is_ok());
        preparer.slots.close();
        assert!(matches!(
            preparer.ensure_accepting(),
            Err(PreparationError::Stopping)
        ));
    }

    #[tokio::test]
    async fn process_observes_shutdown_before_spawning_a_worker() {
        let preparer = UploadPreparer::state("unused executable".into());
        let (_shutdown_tx, mut shutdown) = tokio::sync::watch::channel(true);
        let (_lease_tx, mut lease) = tokio::sync::oneshot::channel();
        let error = preparer
            .process(
                std::path::Path::new("unused spool"),
                "image/jpeg".into(),
                &mut lease,
                &mut shutdown,
            )
            .await
            .expect_err("shutdown prevents child admission");
        assert!(matches!(error, PreparationError::Stopping));
    }

    #[tokio::test]
    async fn finalization_drain_returns_join_failures_after_draining_all_owners() {
        let mut owners = tokio::task::JoinSet::new();
        owners.spawn(async {});
        owners.spawn(std::future::pending::<()>());
        owners.abort_all();
        let failure = join_finalizations(owners)
            .await
            .expect("cancelled finalizer is reported");
        assert!(failure.is_cancelled());
    }

    #[tokio::test]
    async fn shutdown_reports_the_join_error_from_its_owned_finalizers() {
        let preparer = UploadPreparer::state("unused executable".into());
        {
            let mut owners = preparer.owners.lock().unwrap();
            owners.spawn(async { panic!("owned finalizer panic witness") });
            owners.spawn(async {});
        }
        let error = preparer
            .shutdown()
            .await
            .expect_err("shutdown reports an owned finalizer panic");
        assert!(matches!(error, PreparationError::Owner(join) if join.is_panic()));
    }

    #[tokio::test]
    async fn retry_reaping_retains_the_first_failure_until_exit_acknowledgement() {
        let mut failure = None;
        let started = tokio::time::Instant::now();
        assert!(
            reap_wait_result(
                &mut failure,
                Err(std::io::Error::other("first wait failure")),
            )
            .await
            .is_none()
        );
        assert!(started.elapsed() >= std::time::Duration::from_millis(100));
        assert_eq!(failure.as_ref().unwrap().to_string(), "first wait failure");
        assert!(matches!(
            reap_wait_result(&mut failure, Ok(())).await,
            Some(Err(error)) if error.to_string() == "first wait failure"
        ));
    }

    #[derive(Clone)]
    struct CapturedWriter(Arc<Mutex<Vec<u8>>>);

    impl std::io::Write for CapturedWriter {
        fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
            self.0
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .extend_from_slice(bytes);
            Ok(bytes.len())
        }

        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }

    impl<'writer> tracing_subscriber::fmt::MakeWriter<'writer> for CapturedWriter {
        type Writer = Self;

        fn make_writer(&'writer self) -> Self::Writer {
            self.clone()
        }
    }

    #[test]
    fn unobserved_cleanup_reports_only_disconnected_cleanup_failures() {
        let capture = |delivery: Result<(), std::io::Result<()>>| {
            let output = Arc::new(Mutex::new(Vec::new()));
            let subscriber = tracing_subscriber::fmt()
                .json()
                .with_ansi(false)
                .with_writer(CapturedWriter(output.clone()))
                .finish();
            let mut writer = CapturedWriter(output.clone());
            std::io::Write::flush(&mut writer).expect("captured diagnostic writer flushes");
            tracing::subscriber::with_default(subscriber, || {
                report_unobserved_cleanup_if_needed(delivery);
            });
            String::from_utf8_lossy(
                &output
                    .lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner),
            )
            .into_owned()
        };

        assert!(capture(Ok(())).is_empty(), "successful handback is silent");
        assert!(
            capture(Err(Ok(()))).is_empty(),
            "disconnected receiver with successful cleanup is silent"
        );

        let trace = capture(Err(Err(std::io::Error::other(
            "private upload /media/tmp/upload-secret cleanup failure",
        ))));
        assert_eq!(trace.matches("media.private_spool.cleanup").count(), 1);
        assert!(trace.contains("swallowed"));
        assert!(trace.contains("redacted"));
        assert!(!trace.contains("upload-secret"));
        assert!(!trace.contains("private upload"));
    }

    #[tokio::test]
    async fn upload_attempt_can_observe_an_already_completed_owner_result_without_a_file() {
        let (start, _start_rx) = tokio::sync::oneshot::channel();
        let (lease, _lease_rx) = tokio::sync::oneshot::channel();
        let (result_tx, result) = tokio::sync::oneshot::channel();
        result_tx.send(Ok(None)).unwrap();
        let (done_tx, done) = tokio::sync::oneshot::channel();
        done_tx.send(Ok(())).unwrap();
        let mut attempt = UploadAttempt {
            file: None,
            start: Some(start),
            lease: Some(lease),
            result: Some(result),
            done: Some(done),
            output: std::path::PathBuf::new(),
            stopping: tokio::sync::watch::channel(false).1,
        };
        assert_eq!(attempt.prepare("image/jpeg").await.unwrap(), None);
    }
}
