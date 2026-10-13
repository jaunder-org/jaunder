//! Runtime-independent child and spool ownership, including supervisor unwinding.
//! No capacity or private file can be released without a confirmed child reap.

use std::io::Read;
use std::path::Path;
use std::process::{Child, ChildStdout, Command, Stdio};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use tempfile::NamedTempFile;
use tokio::sync::oneshot;

use super::{
    ImageSanitizerError, report_private_failure, worker_protocol, worker_request::WORKER_FLAG,
};

pub(super) fn run(
    executable: &Path,
    input: NamedTempFile,
    claim: bool,
    limit: u64,
    deadline: Instant,
    mut canceled: oneshot::Receiver<()>,
    reaped: &Arc<AtomicBool>,
) -> Result<(NamedTempFile, worker_protocol::Reply), ImageSanitizerError> {
    let mut spool = Spool {
        file: Some(input),
        reaped: reaped.clone(),
    };
    let result = process(
        executable,
        spool.file().path(),
        claim,
        limit,
        deadline,
        &mut canceled,
        reaped,
    );
    if !reaped.load(Ordering::Acquire) {
        // Drop retains the spool; the outer permit guard retains capacity too.
        return result.map(|reply| unreachable!("unreaped child cannot carry a reply: {reply:?}"));
    }
    match result {
        Ok(reply) => Ok((spool.take(), reply)),
        Err(primary) => match spool.take().close() {
            Ok(()) => Err(primary),
            Err(source) => Err(ImageSanitizerError::Cleanup {
                primary: Box::new(primary),
                source,
            }),
        },
    }
}

struct Spool {
    file: Option<NamedTempFile>,
    reaped: Arc<AtomicBool>,
}
impl Spool {
    fn file(&self) -> &NamedTempFile {
        match &self.file {
            Some(file) => file,
            None => unreachable!("spool ownership already transferred"),
        }
    }
    fn take(&mut self) -> NamedTempFile {
        match self.file.take() {
            Some(file) => file,
            None => unreachable!("spool transfers only once"),
        }
    }
}
impl Drop for Spool {
    fn drop(&mut self) {
        if let Some(file) = self.file.take() {
            if self.reaped.load(Ordering::Acquire) {
                if let Err(error) = file.close() {
                    report_private_failure(&error, "unwinding image spool cleanup");
                }
            } else {
                std::mem::forget(file);
            }
        }
    }
}

struct ChildOwner {
    child: Child,
    reaped: Arc<AtomicBool>,
    ownership_lost: bool,
}
impl ChildOwner {
    fn status(
        &mut self,
        result: std::io::Result<Option<std::process::ExitStatus>>,
    ) -> std::io::Result<Option<std::process::ExitStatus>> {
        match &result {
            Ok(Some(_)) => self.reaped.store(true, Ordering::Release),
            Err(error) if error.raw_os_error() == Some(rustix::io::Errno::CHILD.raw_os_error()) => {
                self.ownership_lost = true;
            }
            _ => {}
        }
        result
    }
    fn try_wait(&mut self) -> std::io::Result<Option<std::process::ExitStatus>> {
        let result = self.child.try_wait();
        self.status(result)
    }
    fn wait(&mut self) -> std::io::Result<()> {
        let result = self.child.wait().map(Some);
        self.status(result).map(|_| ())
    }
}
impl Drop for ChildOwner {
    fn drop(&mut self) {
        // ECHILD gives no authority to signal a PID that may now be reused.
        // Retain the unconfirmed resources rather than guessing termination.
        if self.reaped.load(Ordering::Acquire) || self.ownership_lost {
            return;
        }
        let kill = self.child.kill();
        let wait = self.wait();
        report_unwind_results(kill, wait);
    }
}

fn process(
    executable: &Path,
    path: &Path,
    claim: bool,
    limit: u64,
    deadline: Instant,
    canceled: &mut oneshot::Receiver<()>,
    reaped: &Arc<AtomicBool>,
) -> Result<worker_protocol::Reply, ImageSanitizerError> {
    if Instant::now() >= deadline {
        return Err(ImageSanitizerError::TimedOut);
    }
    let child = Command::new(executable)
        .arg(WORKER_FLAG)
        .arg(path)
        .arg(if claim { "1" } else { "0" })
        .arg(limit.to_string())
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .map_err(ImageSanitizerError::Processing)?;
    reaped.store(false, Ordering::Release);
    let mut child = ChildOwner {
        child,
        reaped: reaped.clone(),
        ownership_lost: false,
    };
    let Some(stdout) = child.child.stdout.take() else {
        unreachable!("spawned child owns configured stdout");
    };
    supervise(&mut child, stdout, deadline, canceled)
}

// Keep status observation and settlement together: lost wait authority must
// return before either normal settlement or Drop can signal a reused PID.
fn supervise(
    child: &mut ChildOwner,
    mut stdout: ChildStdout,
    deadline: Instant,
    canceled: &mut oneshot::Receiver<()>,
) -> Result<worker_protocol::Reply, ImageSanitizerError> {
    let work = (|| {
        let flags = rustix::fs::fcntl_getfl(&stdout).map_err(std::io::Error::from)?;
        rustix::fs::fcntl_setfl(&stdout, flags | rustix::fs::OFlags::NONBLOCK)
            .map_err(std::io::Error::from)?;
        let mut bytes = Vec::new();
        let mut eof = false;
        let mut status = None;
        loop {
            if !matches!(
                canceled.try_recv(),
                Err(oneshot::error::TryRecvError::Empty)
            ) {
                return Err(ImageSanitizerError::Canceled);
            }
            if Instant::now() >= deadline {
                return Err(ImageSanitizerError::TimedOut);
            }
            // Interrupted reads immediately begin the next supervision turn,
            // including its cancellation/deadline checks. Only completed reads
            // advance child-status observation or incur the poll interval.
            if read_reply_chunk(&mut stdout, &mut bytes, &mut eof)? == ReadProgress::Poll {
                if status.is_none() {
                    status = child.try_wait().map_err(ImageSanitizerError::Processing)?;
                }
                if let Some(status) = status {
                    if !status.success() {
                        return Err(ImageSanitizerError::WorkerExited(status));
                    }
                    if eof {
                        return worker_protocol::decode(&bytes)
                            .map_err(ImageSanitizerError::Protocol);
                    }
                }
                std::thread::sleep(Duration::from_millis(2));
            }
        }
    })();
    if child.ownership_lost {
        return work;
    }
    let kill = child.child.kill();
    let wait = child.wait();
    settled_child(kill, wait)?;
    work
}

fn settled_child(
    kill: std::io::Result<()>,
    wait: std::io::Result<()>,
) -> Result<(), ImageSanitizerError> {
    wait.map_err(ImageSanitizerError::Reap)?;
    if let Err(error) = kill
        && error.kind() != std::io::ErrorKind::InvalidInput
    {
        return Err(ImageSanitizerError::Processing(error));
    }
    Ok(())
}

fn report_unwind_results(kill: std::io::Result<()>, wait: std::io::Result<()>) {
    match wait {
        Ok(()) => {}
        Err(error) => report_private_failure(&error, "unwinding image child reap failed"),
    }
    if let Err(error) = kill
        && error.kind() != std::io::ErrorKind::InvalidInput
    {
        // Reaping has already been attempted, even after a kill failure.
        report_private_failure(&error, "unwinding image child kill failed");
    }
}

#[derive(Debug, PartialEq, Eq)]
enum ReadProgress {
    Poll,
    Retry,
}

fn read_reply_chunk(
    reader: &mut impl Read,
    bytes: &mut Vec<u8>,
    eof: &mut bool,
) -> Result<ReadProgress, ImageSanitizerError> {
    let mut buffer = [0; 4096];
    match reader.read(&mut buffer) {
        Ok(0) => *eof = true,
        Ok(count) => {
            bytes.extend_from_slice(&buffer[..count]);
            if bytes.len() > worker_protocol::MAX_REPLY_BYTES {
                return Err(ImageSanitizerError::Protocol(
                    worker_protocol::ProtocolError::TooLarge,
                ));
            }
        }
        Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {}
        Err(error) if error.kind() == std::io::ErrorKind::Interrupted => {
            return Ok(ReadProgress::Retry);
        }
        Err(error) => return Err(ImageSanitizerError::Processing(error)),
    }
    Ok(ReadProgress::Poll)
}

impl From<std::io::Error> for ImageSanitizerError {
    fn from(value: std::io::Error) -> Self {
        Self::Processing(value)
    }
}

#[cfg(test)]
mod tests {
    use super::super::owner_registry::PermitOwner;
    use super::*;
    use tokio::sync::Semaphore;

    fn worker(input: &NamedTempFile) -> Child {
        Command::new(std::env::var_os("JAUNDER_IMAGE_WORKER_BIN").expect("real worker fixture"))
            .arg(WORKER_FLAG)
            .arg(input.path())
            .args(["0", "134217728"])
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .expect("real native child")
    }
    fn pid(child: &Child) -> rustix::process::Pid {
        rustix::process::Pid::from_raw(i32::try_from(child.id()).expect("PID fits platform"))
            .expect("positive PID")
    }

    // Observer libtests are owners, not replacement workers. Capture to private
    // files so output volume cannot block settlement, and retain those files
    // whenever the parent's bounded wait cannot confirm its own child reap.
    struct ObserverOwner {
        child: Option<ChildOwner>,
        stdout: Spool,
        stderr: Spool,
    }

    impl ObserverOwner {
        fn spawn(mut command: Command) -> std::io::Result<Self> {
            let reaped = Arc::new(AtomicBool::new(true));
            let stdout = Spool {
                file: Some(NamedTempFile::new()?),
                reaped: reaped.clone(),
            };
            let stderr = Spool {
                file: Some(NamedTempFile::new()?),
                reaped: reaped.clone(),
            };
            command
                .stdin(Stdio::null())
                .stdout(Stdio::from(stdout.file().reopen()?))
                .stderr(Stdio::from(stderr.file().reopen()?));
            let child = command.spawn()?;
            reaped.store(false, Ordering::Release);
            Ok(Self {
                child: Some(ChildOwner {
                    child,
                    reaped,
                    ownership_lost: false,
                }),
                stdout,
                stderr,
            })
        }

        fn wait_until(&mut self, deadline: Instant) -> std::io::Result<std::process::ExitStatus> {
            loop {
                let child = self.child.as_mut().expect("owned observer child");
                if let Some(status) = child.try_wait()? {
                    self.child.take();
                    return Ok(status);
                }
                if Instant::now() >= deadline {
                    return Err(std::io::Error::new(
                        std::io::ErrorKind::TimedOut,
                        "observer child reap deadline",
                    ));
                }
                std::thread::sleep(Duration::from_millis(1));
            }
        }

        fn settle(&mut self) -> std::io::Result<()> {
            let Some(child) = self.child.as_mut() else {
                return Ok(());
            };
            if child.ownership_lost {
                return Err(std::io::Error::other("observer wait authority lost"));
            }
            if child.try_wait()?.is_some() {
                self.child.take();
                return Ok(());
            }
            // This parent's unreaped observer has no independent reaper. Kill
            // only while that wait authority remains, then confirm settlement.
            let kill = child.child.kill();
            let wait = self.wait_until(Instant::now() + Duration::from_secs(5));
            settled_child(kill, wait.map(|_| ())).map_err(std::io::Error::other)
        }
    }

    impl Drop for ObserverOwner {
        fn drop(&mut self) {
            if let Err(error) = self.settle() {
                if let Some(child) = self.child.take() {
                    // Do not enter ChildOwner's blocking production Drop after
                    // the observer's bounded wait has failed. Spools retain too.
                    std::mem::forget(child);
                }
                eprintln!("observer cleanup unconfirmed: {error}");
                assert!(std::thread::panicking(), "observer cleanup unconfirmed");
            }
        }
    }

    #[test]
    fn observer_deadline_cleanup_reaps_before_private_capture_release() {
        let mut command = Command::new("/bin/sh");
        command.args(["-c", "kill -STOP $$"]);
        let mut owner = ObserverOwner::spawn(command).expect("owned stopped observer");
        let stdout = owner.stdout.file().path().to_owned();
        let stderr = owner.stderr.file().path().to_owned();
        let pid = pid(&owner.child.as_ref().expect("owned observer").child);
        let error = owner
            .wait_until(Instant::now())
            .expect_err("expired observer wait");
        assert_eq!(error.kind(), std::io::ErrorKind::TimedOut);
        assert!(stdout.exists() && stderr.exists());
        drop(owner);
        assert_eq!(
            rustix::process::waitpid(Some(pid), rustix::process::WaitOptions::NOHANG).err(),
            Some(rustix::io::Errno::CHILD),
            "bounded cleanup already performed the owned wait"
        );
        assert!(!stdout.exists() && !stderr.exists());
    }

    #[test]
    fn externally_reaped_observer_retains_private_captures_and_fails_closed() {
        let mut command = Command::new("/bin/sh");
        command.args(["-c", "exit 0"]);
        let mut owner = ObserverOwner::spawn(command).expect("owned exiting observer");
        let stdout = owner.stdout.file().path().to_owned();
        let stderr = owner.stderr.file().path().to_owned();
        let pid = pid(&owner.child.as_ref().expect("owned observer").child);
        let deadline = Instant::now() + Duration::from_secs(10);
        loop {
            if rustix::process::waitpid(Some(pid), rustix::process::WaitOptions::NOHANG)
                .expect("independent observer reap")
                .is_some()
            {
                break;
            }
            assert!(
                Instant::now() < deadline,
                "independent observer reap deadline"
            );
            std::thread::sleep(Duration::from_millis(1));
        }
        assert_eq!(
            owner
                .wait_until(deadline)
                .expect_err("lost wait authority")
                .raw_os_error(),
            Some(rustix::io::Errno::CHILD.raw_os_error())
        );
        assert!(std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| drop(owner))).is_err());
        assert!(
            stdout.exists() && stderr.exists(),
            "unconfirmed captures retained"
        );
        // Only the independent wait above authorizes test-resource removal.
        std::fs::remove_file(stdout).expect("independently settled stdout cleanup");
        std::fs::remove_file(stderr).expect("independently settled stderr cleanup");
    }

    #[cfg(target_os = "linux")]
    fn observe_sigchld_disposition(phase: &str) {
        // Procfs exposes dispositions without installing a handler; it does not
        // expose sigaction flags such as SA_NOCLDWAIT.
        let status = std::fs::read_to_string("/proc/self/status")
            .expect("read-only observer signal disposition");
        let mask = |name: &str| {
            u64::from_str_radix(
                status
                    .lines()
                    .find_map(|line| line.strip_prefix(name))
                    .expect("procfs signal mask")
                    .trim(),
                16,
            )
            .expect("hexadecimal procfs signal mask")
        };
        let ignored = mask("SigIgn:");
        let caught = mask("SigCgt:");
        let chld = 1_u64 << (rustix::process::Signal::CHILD.as_raw() - 1);
        eprintln!(
            "ownership observer {phase}: SigIgn={ignored:016x} SigCgt={caught:016x} SIGCHLD_ignored={} SIGCHLD_caught={} sigaction_flags=unavailable",
            ignored & chld != 0,
            caught & chld != 0,
        );
    }

    #[cfg(target_os = "linux")]
    fn bounded_observer_capture(file: &NamedTempFile) -> String {
        const LIMIT: usize = 8192;
        let mut bytes = Vec::new();
        file.reopen()
            .expect("private observer capture")
            .take((LIMIT + 1) as u64)
            .read_to_end(&mut bytes)
            .expect("bounded private observer capture read");
        let truncated = bytes.len() > LIMIT;
        bytes.truncate(LIMIT);
        format!(
            "{}\n[capture truncated={truncated}; byte_limit={LIMIT}]",
            String::from_utf8_lossy(&bytes)
        )
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn actual_process_ownership_loss_retains_reap_authority_and_never_signals_again() {
        const OBSERVER: &str = "JAUNDER_PROCESS_OWNERSHIP_OBSERVER";
        if std::env::var_os(OBSERVER).is_none() {
            // Bash preserves ignored SIGCHLD across exec; BusyBox sh's CHLD trap does not.
            let mut command = Command::new("bash");
            command
                .args(["-c", "trap '' CHLD; exec \"$1\" --exact image_sanitizer::supervision::tests::actual_process_ownership_loss_retains_reap_authority_and_never_signals_again --nocapture", "owned-observer"])
                .arg(std::env::current_exe().expect("owner libtest executable"))
                .env(OBSERVER, "1");
            let mut owner = ObserverOwner::spawn(command).expect("isolated SIGCHLD observer");
            let status = owner.wait_until(Instant::now() + Duration::from_secs(10));
            // The isolated exact-filtered test uses only synthetic input. Read
            // both captures even on a wait error, without dumping ambient env.
            let stdout = bounded_observer_capture(owner.stdout.file());
            let stderr = bounded_observer_capture(owner.stderr.file());
            assert!(
                status.as_ref().is_ok_and(std::process::ExitStatus::success),
                "ownership observer: status={status:?}\nstdout:\n{stdout}\nstderr:\n{stderr}"
            );
            assert!(
                stdout.contains("PROCESS_OWNERSHIP_LOSS_OBSERVED"),
                "specific process-path receipt required\nstdout:\n{stdout}\nstderr:\n{stderr}"
            );
            return;
        }
        observe_sigchld_disposition("inner-entry");
        // This synchronous libtest is an observer, not a worker substitute.
        // Observe whether ignored SIGCHLD survives the isolated exec path.
        let input = NamedTempFile::new().expect("owned input");
        std::fs::write(input.path(), b"ordinary attachment").expect("input bytes");
        let (_cancel, mut canceled) = oneshot::channel();
        let reaped = Arc::new(AtomicBool::new(true));
        let executable = std::path::PathBuf::from(
            std::env::var_os("JAUNDER_IMAGE_WORKER_BIN").expect("actual worker fixture"),
        );
        observe_sigchld_disposition("pre-process-spawn");
        let outcome = process(
            &executable,
            input.path(),
            false,
            1024,
            Instant::now() + Duration::from_secs(5),
            &mut canceled,
            &reaped,
        );
        let io = match &outcome {
            Err(ImageSanitizerError::Processing(error) | ImageSanitizerError::Reap(error)) => {
                Some((error.kind(), error.raw_os_error()))
            }
            _ => None,
        };
        let confirmed_reaped = reaped.load(Ordering::Acquire);
        eprintln!(
            "ownership observer process-return: outcome={outcome:?} io_kind_errno={io:?} reaped={confirmed_reaped}"
        );
        assert!(
            matches!(outcome, Err(ImageSanitizerError::Processing(ref error))
            if error.raw_os_error() == Some(rustix::io::Errno::CHILD.raw_os_error())),
            "expected Processing(ECHILD): outcome={outcome:?} io_kind_errno={io:?} reaped={confirmed_reaped}"
        );
        assert!(
            !reaped.load(Ordering::Acquire),
            "no owned reap confirmation after kernel ownership loss"
        );
        assert!(input.path().exists(), "private resources remain owned");
        std::mem::forget(input);
        println!("PROCESS_OWNERSHIP_LOSS_OBSERVED");
    }

    #[test]
    fn expired_deadline_rejects_before_spawn_and_removes_private_spool() {
        let directory = tempfile::tempdir().expect("private test root");
        let input = NamedTempFile::new_in(directory.path()).expect("private spool");
        let path = input.path().to_owned();
        let (_cancel, canceled) = oneshot::channel();
        let reaped = Arc::new(AtomicBool::new(true));
        let error = run(
            &directory.path().join("missing-worker"),
            input,
            false,
            1024,
            Instant::now(),
            canceled,
            &reaped,
        )
        .expect_err("expired admission cannot spawn");
        assert!(matches!(error, ImageSanitizerError::TimedOut));
        assert!(reaped.load(Ordering::Acquire), "no child was created");
        assert!(!path.exists(), "no-child failure removes its private spool");
    }

    #[test]
    fn implicit_io_conversion_preserves_concrete_source() {
        let error = ImageSanitizerError::from(std::io::Error::new(
            std::io::ErrorKind::PermissionDenied,
            "controlled nonblocking setup failure",
        ));
        assert!(matches!(error, ImageSanitizerError::Processing(source)
            if source.kind() == std::io::ErrorKind::PermissionDenied
            && source.to_string() == "controlled nonblocking setup failure"));
    }

    #[test]
    fn child_settlement_prioritizes_reap_and_preserves_unexpected_kill_source() {
        assert!(settled_child(Ok(()), Ok(())).is_ok());
        assert!(
            settled_child(
                Err(std::io::Error::new(
                    std::io::ErrorKind::InvalidInput,
                    "already exited"
                )),
                Ok(()),
            )
            .is_ok()
        );
        assert!(matches!(settled_child(
            Err(std::io::Error::new(std::io::ErrorKind::PermissionDenied, "controlled kill failure")),
            Ok(()),
        ), Err(ImageSanitizerError::Processing(source))
            if source.kind() == std::io::ErrorKind::PermissionDenied
            && source.to_string() == "controlled kill failure"));
        assert!(matches!(settled_child(
            Err(std::io::Error::new(std::io::ErrorKind::PermissionDenied, "controlled kill failure")),
            Err(std::io::Error::new(std::io::ErrorKind::Interrupted, "controlled reap failure")),
        ), Err(ImageSanitizerError::Reap(source))
            if source.kind() == std::io::ErrorKind::Interrupted
            && source.to_string() == "controlled reap failure"));
    }

    #[test]
    fn unwind_reporting_handles_both_failures_without_panicking() {
        for kind in [
            None,
            Some(std::io::ErrorKind::InvalidInput),
            Some(std::io::ErrorKind::PermissionDenied),
        ] {
            assert!(
                std::panic::catch_unwind(|| {
                    let kill = kind.map_or(Ok(()), |kind| {
                        Err(std::io::Error::new(kind, "controlled kill failure"))
                    });
                    report_unwind_results(
                        kill,
                        Err(std::io::Error::new(
                            std::io::ErrorKind::Interrupted,
                            "controlled reap failure",
                        )),
                    );
                })
                .is_ok(),
                "unwind reporting cannot interrupt cleanup"
            );
        }
        assert!(std::panic::catch_unwind(|| report_unwind_results(Ok(()), Ok(()))).is_ok());
    }

    struct FailedRead(std::io::ErrorKind);
    impl Read for FailedRead {
        fn read(&mut self, _buffer: &mut [u8]) -> std::io::Result<usize> {
            Err(std::io::Error::new(self.0, "controlled reply read failure"))
        }
    }

    #[test]
    fn reply_read_retries_interruptions_but_polls_would_block() {
        // Interrupted reads immediately recheck cancellation/deadline; an
        // empty nonblocking pipe must still poll child status and then yield.
        for (kind, expected) in [
            (std::io::ErrorKind::Interrupted, ReadProgress::Retry),
            (std::io::ErrorKind::WouldBlock, ReadProgress::Poll),
        ] {
            let mut bytes = b"existing reply prefix".to_vec();
            let mut eof = false;
            assert_eq!(
                read_reply_chunk(&mut FailedRead(kind), &mut bytes, &mut eof)
                    .expect("nonfatal read"),
                expected
            );
            assert_eq!(bytes, b"existing reply prefix");
            assert!(!eof, "transient errors cannot certify EOF");
        }
        let mut bytes = Vec::new();
        let mut eof = false;
        let error = read_reply_chunk(
            &mut FailedRead(std::io::ErrorKind::PermissionDenied),
            &mut bytes,
            &mut eof,
        )
        .expect_err("unexpected I/O source");
        assert!(
            matches!(error, ImageSanitizerError::Processing(source) if source.kind() == std::io::ErrorKind::PermissionDenied && source.to_string() == "controlled reply read failure")
        );
        assert!(bytes.is_empty());
        assert!(!eof);
    }

    #[test]
    fn reply_read_accepts_exact_bound_and_rejects_first_excess_byte() {
        // Size validation applies to accumulated bytes, not each individual
        // read. EOF is required separately and cannot rescue oversized output.
        for size in [
            worker_protocol::MAX_REPLY_BYTES,
            worker_protocol::MAX_REPLY_BYTES + 1,
        ] {
            let mut reader = std::io::Cursor::new(vec![b'x'; size]);
            let mut bytes = Vec::new();
            let mut eof = false;
            while bytes.len() < worker_protocol::MAX_REPLY_BYTES {
                assert_eq!(
                    read_reply_chunk(&mut reader, &mut bytes, &mut eof).expect("bounded chunk"),
                    ReadProgress::Poll
                );
            }
            assert_eq!(bytes.len(), worker_protocol::MAX_REPLY_BYTES);
            assert!(!eof, "a full chunk alone does not establish EOF");
            let result = read_reply_chunk(&mut reader, &mut bytes, &mut eof);
            if size == worker_protocol::MAX_REPLY_BYTES {
                assert_eq!(result.expect("exact bound"), ReadProgress::Poll);
                assert!(eof);
            } else {
                assert!(matches!(
                    result,
                    Err(ImageSanitizerError::Protocol(
                        worker_protocol::ProtocolError::TooLarge
                    ))
                ));
                assert!(!eof);
                assert_eq!(bytes.len(), worker_protocol::MAX_REPLY_BYTES + 1);
            }
        }
    }

    #[test]
    fn spawn_failure_with_failed_unlink_preserves_both_concrete_sources() {
        // No child exists after a real executable-open failure. Failed private
        // cleanup must still preserve that primary I/O error and its own source.
        let directory = tempfile::tempdir().expect("owned private root");
        let input = NamedTempFile::new_in(directory.path()).expect("private spool");
        let path = input.path().to_owned();
        std::fs::remove_file(&path).expect("replace private pathname");
        std::fs::create_dir(&path).expect("non-file unlink failure");
        let (_cancel, canceled) = oneshot::channel();
        let reaped = Arc::new(AtomicBool::new(true));
        let error = run(
            &directory.path().join("missing-worker"),
            input,
            false,
            1024,
            Instant::now() + super::super::PROCESSING_TIMEOUT,
            canceled,
            &reaped,
        )
        .expect_err("missing executable and obstructed cleanup");
        assert!(
            matches!(error, ImageSanitizerError::Cleanup { primary, source } if matches!(primary.as_ref(), ImageSanitizerError::Processing(error) if error.kind() == std::io::ErrorKind::NotFound) && source.kind() == std::io::ErrorKind::IsADirectory)
        );
        assert!(
            reaped.load(Ordering::Acquire),
            "no spawned child can retain capacity"
        );
        assert!(path.is_dir(), "failed unlink cannot claim removal");
    }

    #[test]
    fn confirmed_spool_drop_does_not_hide_failed_unlink_state() {
        // Test-owned directory obstruction forces a real cleanup failure. Drop
        // exercises private reporting without claiming telemetry verification.
        let directory = tempfile::tempdir().expect("owned private root");
        let input = NamedTempFile::new_in(directory.path()).expect("private spool");
        let path = input.path().to_owned();
        std::fs::remove_file(&path).expect("replace private pathname");
        std::fs::create_dir(&path).expect("non-file unlink failure");
        drop(Spool {
            file: Some(input),
            reaped: Arc::new(AtomicBool::new(true)),
        });
        assert!(path.is_dir(), "failed unlink remains observable");
    }

    #[test]
    fn owner_unwinding_reaps_a_real_child_before_spool_and_capacity_release() {
        let input = NamedTempFile::new().expect("private spool");
        input
            .as_file()
            .set_len(64 * 1024 * 1024)
            .expect("bounded input");
        let path = input.path().to_owned();
        let child = worker(&input);
        let pid = pid(&child);
        rustix::process::kill_process(pid, rustix::process::Signal::STOP)
            .expect("pause native child");
        let reaped = Arc::new(AtomicBool::new(false));
        let capacity = Arc::new(Semaphore::new(2));
        let permit = PermitOwner::new(
            capacity.clone().try_acquire_owned().expect("permit"),
            Arc::new(AtomicBool::new(false)),
            reaped.clone(),
        );
        let panic = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            let _spool = Spool {
                file: Some(input),
                reaped: reaped.clone(),
            };
            let _child = ChildOwner {
                child,
                reaped: reaped.clone(),
                ownership_lost: false,
            };
            panic!("intentional supervisor unwind");
        }));
        assert!(panic.is_err());
        assert!(reaped.load(Ordering::Acquire));
        assert!(!path.exists());
        assert_eq!(
            rustix::process::test_kill_process(pid),
            Err(rustix::io::Errno::SRCH)
        );
        drop(permit);
        assert_eq!(capacity.available_permits(), 2);
    }

    #[test]
    fn an_external_reap_is_not_guessed_as_confirmed_and_resources_remain_retained() {
        let directory = tempfile::tempdir().expect("owned private root");
        let input = NamedTempFile::new_in(directory.path()).expect("private spool");
        // A tiny passthrough reply cannot fill stdout while the independent
        // reaper waits. All real guards own their resources before observation.
        std::fs::write(input.path(), b"ordinary attachment").expect("input bytes");
        let path = input.path().to_owned();
        let reaped = Arc::new(AtomicBool::new(true));
        let spool = Spool {
            file: Some(input),
            reaped: reaped.clone(),
        };
        let capacity = Arc::new(Semaphore::new(2));
        let permit = PermitOwner::new(
            capacity.clone().try_acquire_owned().expect("permit"),
            Arc::new(AtomicBool::new(false)),
            reaped.clone(),
        );
        let native_child = worker(spool.file());
        reaped.store(false, Ordering::Release);
        let mut child = ChildOwner {
            child: native_child,
            reaped: reaped.clone(),
            ownership_lost: false,
        };
        let pid = pid(&child.child);
        let stdout = child.child.stdout.take().expect("real worker reply pipe");
        let reap_deadline = Instant::now() + Duration::from_secs(10);
        let independently_reaped = loop {
            if let Some(record) =
                rustix::process::waitpid(Some(pid), rustix::process::WaitOptions::NOHANG)
                    .expect("controlled external reap")
            {
                break record;
            }
            assert!(
                Instant::now() < reap_deadline,
                "independent worker reap deadline"
            );
            std::thread::sleep(Duration::from_millis(1));
        };
        let (cancel, mut canceled) = oneshot::channel();
        let outcome = supervise(
            &mut child,
            stdout,
            Instant::now() + Duration::from_secs(5),
            &mut canceled,
        );
        assert!(
            matches!(outcome, Err(ImageSanitizerError::Processing(ref error))
                if error.raw_os_error() == Some(rustix::io::Errno::CHILD.raw_os_error())),
            "expected shared supervision Processing(ECHILD): {outcome:?}"
        );
        assert!(
            child.ownership_lost,
            "shared loop records lost wait authority"
        );
        assert!(
            !reaped.load(Ordering::Acquire),
            "no owned reap confirmation"
        );
        assert_eq!(
            independently_reaped.0, pid,
            "exact worker independently reaped"
        );
        drop(cancel);
        drop(child);
        drop(spool);
        drop(permit);
        assert!(!reaped.load(Ordering::Acquire));
        assert!(path.exists(), "unconfirmed owner cannot release its spool");
        assert_eq!(capacity.available_permits(), 1);
        // Only the independent exact-PID reap record authorizes test cleanup;
        // it never grants the production guards their own reap confirmation.
        std::fs::remove_file(path).expect("cleanup after independently confirmed external reap");
    }
}
