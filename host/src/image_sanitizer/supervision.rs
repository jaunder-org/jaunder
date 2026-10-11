//! Runtime-independent child and spool ownership, including supervisor unwinding.
//! No capacity or private file can be released without a confirmed child reap.

use std::io::Read;
use std::path::Path;
use std::process::{Child, Command, Stdio};
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
        match self.wait() {
            Ok(()) => {}
            Err(error) => report_private_failure(&error, "unwinding image child reap failed"),
        }
        if let Err(error) = kill
            && error.kind() != std::io::ErrorKind::InvalidInput
        {
            // Reaping must still be attempted after an unexpected kill failure.
            report_private_failure(&error, "unwinding image child kill failed");
        }
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
    let Some(mut stdout) = child.child.stdout.take() else {
        unreachable!("spawned child owns configured stdout");
    };
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
            let mut buffer = [0; 4096];
            match stdout.read(&mut buffer) {
                Ok(0) => eof = true,
                Ok(count) => {
                    bytes.extend_from_slice(&buffer[..count]);
                    if bytes.len() > worker_protocol::MAX_REPLY_BYTES {
                        return Err(ImageSanitizerError::Protocol(
                            worker_protocol::ProtocolError::TooLarge,
                        ));
                    }
                }
                Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {}
                Err(error) if error.kind() == std::io::ErrorKind::Interrupted => continue,
                Err(error) => return Err(ImageSanitizerError::Processing(error)),
            }
            if status.is_none() {
                status = child.try_wait().map_err(ImageSanitizerError::Processing)?;
            }
            if let Some(status) = status {
                if !status.success() {
                    return Err(ImageSanitizerError::WorkerExited(status));
                }
                if eof {
                    return worker_protocol::decode(&bytes).map_err(ImageSanitizerError::Protocol);
                }
            }
            std::thread::sleep(Duration::from_millis(2));
        }
    })();
    if child.ownership_lost {
        return work;
    }
    let kill = child.child.kill();
    child.wait().map_err(ImageSanitizerError::Reap)?;
    if let Err(error) = kill
        && error.kind() != std::io::ErrorKind::InvalidInput
    {
        return Err(ImageSanitizerError::Processing(error));
    }
    work
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
        let path = input.path().to_owned();
        let child = worker(&input);
        let pid = pid(&child);
        rustix::process::waitpid(Some(pid), rustix::process::WaitOptions::empty())
            .expect("controlled external reap")
            .expect("child reaped");
        let reaped = Arc::new(AtomicBool::new(false));
        let capacity = Arc::new(Semaphore::new(2));
        let permit = PermitOwner::new(
            capacity.clone().try_acquire_owned().expect("permit"),
            Arc::new(AtomicBool::new(false)),
            reaped.clone(),
        );
        let mut child = ChildOwner {
            child,
            reaped: reaped.clone(),
            ownership_lost: false,
        };
        let error = child.wait().expect_err("owner has no confirmed status");
        assert_eq!(
            error.raw_os_error(),
            Some(rustix::io::Errno::CHILD.raw_os_error())
        );
        drop(child);
        drop(Spool {
            file: Some(input),
            reaped: reaped.clone(),
        });
        drop(permit);
        assert!(!reaped.load(Ordering::Acquire));
        assert!(path.exists(), "unconfirmed owner cannot release its spool");
        assert_eq!(capacity.available_permits(), 1);
        assert_eq!(
            rustix::process::test_kill_process(pid),
            Err(rustix::io::Errno::SRCH)
        );
        // Only this test's external waitpid proved termination; the production
        // owner correctly retained resources instead of assuming that proof.
        std::fs::remove_file(path).expect("cleanup after independently confirmed external reap");
    }
}
