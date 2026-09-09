//! Identity-verified local graceful-shutdown command.

use std::{
    io,
    path::Path,
    time::{Duration, Instant},
};

use anyhow::{Context, Result, anyhow};
use rustix::{
    event::{self, PollFd, PollFlags, Timespec},
    process::{self, Pid, PidfdFlags, Signal},
};

use crate::{
    cli::StorageArgs,
    runtime_file::{self, RuntimeProcessIdentity, RuntimeRecord},
};

/// Requests graceful shutdown of the instance currently owning `storage`.
///
/// The command reads the canonical runtime identity without performing startup
/// recovery, binds the signal to a validated pidfd, and only succeeds once that
/// exact process has exited and relinquished its identity.
///
/// # Errors
///
/// Returns a categorized refusal for invalid runtime identity, an unavailable
/// target, timeout, or an identity that remains owned after process exit.
pub(super) fn cmd_shut_down(storage: &StorageArgs, timeout: Duration) -> Result<()> {
    if timeout.is_zero() {
        return Err(anyhow!("timeout must be positive"));
    }
    cmd_shut_down_with(&storage.storage_path, timeout, &LinuxProcessOperations)
}

enum OpenOutcome<Handle> {
    Captured(Handle),
    ProcessExited,
}

trait ProcessOperations {
    type Handle;

    fn open(&self, pid: u32) -> io::Result<OpenOutcome<Self::Handle>>;
    fn start_time(&self, pid: u32) -> io::Result<Option<u64>>;
    fn signal_term(&self, handle: &Self::Handle) -> io::Result<()>;
    fn wait_for_exit(&self, handle: &Self::Handle, timeout: Duration) -> io::Result<bool>;
}

struct LinuxProcessOperations;

impl ProcessOperations for LinuxProcessOperations {
    type Handle = rustix::fd::OwnedFd;

    fn open(&self, pid: u32) -> io::Result<OpenOutcome<Self::Handle>> {
        let pid = i32::try_from(pid)
            .ok()
            .and_then(Pid::from_raw)
            .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "invalid runtime pid"))?;
        match process::pidfd_open(pid, PidfdFlags::empty()) {
            Ok(handle) => Ok(OpenOutcome::Captured(handle)),
            // pidfd error identities depend on the running kernel/process table.
            Err(error) if error == rustix::io::Errno::SRCH => Ok(OpenOutcome::ProcessExited), // cov:ignore
            Err(error) => Err(error.into()), // cov:ignore
        }
    }

    fn start_time(&self, pid: u32) -> io::Result<Option<u64>> {
        runtime_file::read_proc_start_time(pid)
    }

    fn signal_term(&self, handle: &Self::Handle) -> io::Result<()> {
        match process::pidfd_send_signal(handle, Signal::TERM) {
            Ok(()) | Err(rustix::io::Errno::SRCH) => Ok(()),
            Err(error) => Err(error.into()), // cov:ignore -- OS pidfd permission/failure path
        }
    }

    fn wait_for_exit(&self, handle: &Self::Handle, timeout: Duration) -> io::Result<bool> {
        let deadline = Instant::now()
            .checked_add(timeout)
            .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "timeout is too large"))?;
        loop {
            let remaining = deadline.saturating_duration_since(Instant::now());
            if remaining.is_zero() {
                return Ok(false);
            }
            let seconds = i64::try_from(remaining.as_secs())
                .map_err(|_| io::Error::new(io::ErrorKind::InvalidInput, "timeout is too large"))?;
            let timeout = Timespec {
                tv_sec: seconds,
                tv_nsec: remaining.subsec_nanos().into(),
            };
            let mut fds = [PollFd::new(handle, PollFlags::IN)];
            match event::poll(&mut fds, Some(&timeout)) {
                Ok(0) => return Ok(false), // cov:ignore -- kernel poll timeout path
                Ok(_) => return Ok(true),
                Err(error) if error == rustix::io::Errno::INTR => {} // cov:ignore -- signal interruption path
                Err(error) => return Err(error.into()), // cov:ignore -- OS poll failure path
            }
        }
    }
}

fn cmd_shut_down_with(
    storage_path: &Path,
    timeout: Duration,
    operations: &impl ProcessOperations,
) -> Result<()> {
    let runtime_path = runtime_file::canonical_runtime_path(storage_path);
    let runtime = read_runtime_record(&runtime_path)?
        .ok_or_else(|| anyhow!("missing runtime identity: {}", runtime_path.display()))?;
    let identity = runtime.process;
    if runtime.port == 0 {
        return Err(anyhow!(
            "instance still starting: runtime identity has port zero"
        ));
    }

    // Validate before and after pidfd acquisition: a reuse before acquisition
    // is refused, while a reuse after acquisition cannot redirect the pidfd.
    verify_process_identity(operations, identity)?;
    let handle = match operations
        .open(identity.pid)
        .context("cannot acquire process handle")?
    {
        OpenOutcome::Captured(handle) => handle,
        OpenOutcome::ProcessExited => {
            return Err(anyhow!("dead process: runtime pid no longer exists"));
        }
    };
    verify_process_identity(operations, identity)?;

    operations
        .signal_term(&handle)
        .context("cannot deliver SIGTERM through process handle")?;
    if !operations
        .wait_for_exit(&handle, timeout)
        .context("cannot wait for process handle")?
    {
        return Err(anyhow!("timeout waiting for graceful shutdown"));
    }

    match read_runtime_record(&runtime_path)?.map(|record| record.process) {
        None => Ok(()),
        Some(current) if current != identity => Ok(()),
        Some(_) => Err(anyhow!("runtime identity still owned after process exit")),
    }
}

fn verify_process_identity(
    operations: &impl ProcessOperations,
    identity: RuntimeProcessIdentity,
) -> Result<()> {
    match operations
        .start_time(identity.pid)
        .context("cannot read process start-time")?
    {
        None => Err(anyhow!("dead process: runtime pid no longer exists")),
        Some(start_time) if start_time != identity.start_time => {
            Err(anyhow!("process start-time mismatch"))
        }
        Some(_) => Ok(()),
    }
}

fn read_runtime_record(path: &Path) -> Result<Option<RuntimeRecord>> {
    let contents = match std::fs::read(path) {
        Ok(contents) => contents,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(error).context("malformed runtime identity: cannot read"),
    };
    runtime_file::decode_runtime_record(&contents)
        .context("malformed runtime identity")
        .map(Some)
}

#[cfg(test)]
mod tests {
    use std::{
        cell::RefCell,
        collections::VecDeque,
        fs, io,
        path::{Path, PathBuf},
        process::{Child, Command},
        time::Duration,
    };

    use super::*;

    const CAPTURED: RuntimeProcessIdentity = RuntimeProcessIdentity {
        pid: 41,
        start_time: 101,
    };

    struct FakeHandle(u32);

    struct FakeOperations {
        start_times: RefCell<VecDeque<io::Result<Option<u64>>>>,
        wait_result: bool,
        exit_on_open: bool,
        signal_target_exited: bool,
        signals: RefCell<Vec<u32>>,
        opens: RefCell<u32>,
        on_open: Option<Box<dyn Fn()>>,
        on_signal: Option<Box<dyn Fn()>>,
        on_wait: Option<Box<dyn Fn()>>,
    }

    impl FakeOperations {
        fn matching() -> Self {
            Self {
                start_times: RefCell::new(VecDeque::from([
                    Ok(Some(CAPTURED.start_time)),
                    Ok(Some(CAPTURED.start_time)),
                ])),
                wait_result: true,
                exit_on_open: false,
                signal_target_exited: false,
                signals: RefCell::new(Vec::new()),
                opens: RefCell::new(0),
                on_open: None,
                on_signal: None,
                on_wait: None,
            }
        }
    }

    impl ProcessOperations for FakeOperations {
        type Handle = FakeHandle;

        fn open(&self, _pid: u32) -> io::Result<OpenOutcome<Self::Handle>> {
            *self.opens.borrow_mut() += 1;
            if let Some(on_open) = &self.on_open {
                on_open();
            }
            if self.exit_on_open {
                Ok(OpenOutcome::ProcessExited)
            } else {
                Ok(OpenOutcome::Captured(FakeHandle(7)))
            }
        }

        fn start_time(&self, _pid: u32) -> io::Result<Option<u64>> {
            self.start_times
                .borrow_mut()
                .pop_front()
                .expect("configured start-time result")
        }

        fn signal_term(&self, handle: &Self::Handle) -> io::Result<()> {
            if let Some(on_signal) = &self.on_signal {
                on_signal();
            }
            if !self.signal_target_exited {
                self.signals.borrow_mut().push(handle.0);
            }
            Ok(())
        }

        fn wait_for_exit(&self, _handle: &Self::Handle, _timeout: Duration) -> io::Result<bool> {
            if let Some(on_wait) = &self.on_wait {
                on_wait();
            }
            Ok(self.wait_result)
        }
    }

    struct RuntimeReleasingLinuxOperations {
        runtime_path: PathBuf,
    }

    impl ProcessOperations for RuntimeReleasingLinuxOperations {
        type Handle = rustix::fd::OwnedFd;

        fn open(&self, pid: u32) -> io::Result<OpenOutcome<Self::Handle>> {
            LinuxProcessOperations.open(pid)
        }

        fn start_time(&self, pid: u32) -> io::Result<Option<u64>> {
            LinuxProcessOperations.start_time(pid)
        }

        fn signal_term(&self, handle: &Self::Handle) -> io::Result<()> {
            LinuxProcessOperations.signal_term(handle)
        }

        fn wait_for_exit(&self, handle: &Self::Handle, timeout: Duration) -> io::Result<bool> {
            let exited = LinuxProcessOperations.wait_for_exit(handle, timeout)?;
            if exited {
                fs::remove_file(&self.runtime_path)?;
            } // cov:ignore
            Ok(exited)
        }
    }
    struct ChildGuard(Child);

    impl Drop for ChildGuard {
        fn drop(&mut self) {
            let _ = self.0.kill();
            let _ = self.0.wait();
        }
    }

    fn runtime_path(temp: &tempfile::TempDir) -> PathBuf {
        temp.path().join("runtime.json")
    }

    fn write_identity(path: &Path, identity: RuntimeProcessIdentity, port: u16) {
        fs::write(
            path,
            format!(
                r#"{{"ip":"127.0.0.1","port":{port},"pid":{},"start_time":{}}}"#,
                identity.pid, identity.start_time
            ),
        )
        .expect("runtime identity");
    }

    #[test]
    fn public_shutdown_command_rejects_zero_timeout_before_reading_runtime_identity() {
        let temp = tempfile::TempDir::new().expect("temp dir");
        let storage = StorageArgs {
            storage_path: temp.path().to_path_buf(),
            db: "sqlite:./unused.db".parse().expect("SQLite URL"),
        };

        let error = cmd_shut_down(&storage, Duration::ZERO)
            .expect_err("the public command must reject a zero timeout");

        assert_eq!(error.to_string(), "timeout must be positive");
    }

    #[test]
    fn unreadable_runtime_identity_keeps_the_read_failure_category() {
        let temp = tempfile::TempDir::new().expect("temp dir");
        let path = runtime_path(&temp);
        fs::create_dir(&path).expect("runtime path directory");

        let Err(error) = read_runtime_record(&path) else {
            unreachable!("a directory is not a runtime record");
        };

        assert!(
            error
                .to_string()
                .contains("malformed runtime identity: cannot read"),
            "runtime read failure must retain its command category: {error:#}"
        );
    }

    #[test]
    fn pidfd_wait_with_no_remaining_time_reports_timeout_without_signaling() {
        let OpenOutcome::Captured(handle) = LinuxProcessOperations
            .open(std::process::id())
            .expect("capture this process through pidfd")
        else {
            unreachable!("this process remains live while its pidfd is acquired");
        };

        assert!(
            !LinuxProcessOperations
                .wait_for_exit(&handle, Duration::ZERO)
                .expect("zero remaining time is a normal timeout")
        );
    }
    #[test]
    fn real_child_is_signaled_through_pidfd_and_completion_waits_for_runtime_release() {
        let temp = tempfile::TempDir::new().expect("temp dir");
        let path = runtime_path(&temp);
        let mut child = ChildGuard(
            Command::new("sleep")
                .arg("60")
                .spawn()
                .expect("dedicated target child"),
        );
        let identity = RuntimeProcessIdentity {
            pid: child.0.id(),
            start_time: runtime_file::read_proc_start_time(child.0.id())
                .expect("read child start time")
                .expect("live child start time"),
        };
        write_identity(&path, identity, 3000);
        let operations = RuntimeReleasingLinuxOperations {
            runtime_path: path.clone(),
        };

        cmd_shut_down_with(temp.path(), Duration::from_secs(5), &operations)
            .expect("pidfd SIGTERM and runtime relinquishment complete");

        assert!(child.0.try_wait().expect("child status").is_some());
        assert!(!path.exists());
    }

    #[test]
    fn missing_runtime_identity_refuses_without_mutating_bytes() {
        let temp = tempfile::TempDir::new().expect("temp dir");
        let operations = FakeOperations::matching();

        let error = cmd_shut_down_with(temp.path(), Duration::from_secs(1), &operations)
            .expect_err("missing identity must refuse");

        assert!(error.to_string().contains("missing runtime identity"));
        assert!(operations.signals.borrow().is_empty());
        assert!(!runtime_path(&temp).exists());
    }

    #[test]
    fn malformed_runtime_identity_refuses_without_mutating_bytes() {
        let temp = tempfile::TempDir::new().expect("temp dir");
        let path = runtime_path(&temp);
        let bytes = b"not json";
        fs::write(&path, bytes).expect("malformed runtime identity");
        let operations = FakeOperations::matching();

        let error = cmd_shut_down_with(temp.path(), Duration::from_secs(1), &operations)
            .expect_err("malformed identity must refuse");

        assert!(error.to_string().contains("malformed runtime identity"));
        assert_eq!(fs::read(path).expect("runtime bytes"), bytes);
        assert!(operations.signals.borrow().is_empty());
    }

    #[test]
    fn port_zero_runtime_identity_refuses_without_mutating_bytes() {
        let temp = tempfile::TempDir::new().expect("temp dir");
        let path = runtime_path(&temp);
        write_identity(&path, CAPTURED, 0);
        let bytes = fs::read(&path).expect("runtime bytes");
        let operations = FakeOperations::matching();

        let error = cmd_shut_down_with(temp.path(), Duration::from_secs(1), &operations)
            .expect_err("starting instance must refuse");

        assert!(error.to_string().contains("instance still starting"));
        assert_eq!(fs::read(path).expect("runtime bytes"), bytes);
        assert!(operations.signals.borrow().is_empty());
    }

    #[test]
    fn reuse_before_handle_acquisition_refuses_without_opening_or_signaling() {
        let temp = tempfile::TempDir::new().expect("temp dir");
        write_identity(&runtime_path(&temp), CAPTURED, 3000);
        let operations = FakeOperations {
            start_times: RefCell::new(VecDeque::from([Ok(Some(CAPTURED.start_time + 1))])),
            ..FakeOperations::matching()
        };

        let error = cmd_shut_down_with(temp.path(), Duration::from_secs(1), &operations)
            .expect_err("reused pid must refuse");

        assert!(error.to_string().contains("process start-time mismatch"));
        assert_eq!(*operations.opens.borrow(), 0);
        assert!(operations.signals.borrow().is_empty());
    }

    #[test]
    fn exit_during_handle_acquisition_reports_dead_category() {
        let temp = tempfile::TempDir::new().expect("temp dir");
        let path = runtime_path(&temp);
        write_identity(&path, CAPTURED, 3000);
        let bytes = fs::read(&path).expect("runtime bytes");
        let operations = FakeOperations {
            exit_on_open: true,
            ..FakeOperations::matching()
        };

        let error = cmd_shut_down_with(temp.path(), Duration::from_secs(1), &operations)
            .expect_err("exit during handle acquisition must refuse");

        assert!(error.to_string().contains("dead process"));
        assert_eq!(*operations.opens.borrow(), 1);
        assert!(operations.signals.borrow().is_empty());
        assert_eq!(fs::read(path).expect("runtime bytes"), bytes);
    }

    #[test]
    fn reuse_during_handle_acquisition_refuses_before_signaling() {
        let temp = tempfile::TempDir::new().expect("temp dir");
        write_identity(&runtime_path(&temp), CAPTURED, 3000);
        let operations = FakeOperations {
            start_times: RefCell::new(VecDeque::from([
                Ok(Some(CAPTURED.start_time)),
                Ok(Some(CAPTURED.start_time + 1)),
            ])),
            ..FakeOperations::matching()
        };

        let error = cmd_shut_down_with(temp.path(), Duration::from_secs(1), &operations)
            .expect_err("post-open identity validation must reject reuse");

        assert!(error.to_string().contains("process start-time mismatch"));
        assert_eq!(*operations.opens.borrow(), 1);
        assert!(operations.signals.borrow().is_empty());
    }
    #[test]
    fn reuse_after_validation_signals_only_the_captured_handle() {
        let temp = tempfile::TempDir::new().expect("temp dir");
        let path = runtime_path(&temp);
        write_identity(&path, CAPTURED, 3000);
        let replacement = RuntimeProcessIdentity {
            pid: CAPTURED.pid,
            start_time: CAPTURED.start_time + 1,
        };
        let replacement_path = path.clone();
        let operations = FakeOperations {
            on_signal: Some(Box::new(move || {
                write_identity(&replacement_path, replacement, 3001);
            })),
            ..FakeOperations::matching()
        };

        cmd_shut_down_with(temp.path(), Duration::from_secs(1), &operations)
            .expect("signal remains bound to captured handle");

        assert_eq!(*operations.signals.borrow(), vec![7]);
        assert_eq!(
            read_runtime_record(&path)
                .expect("read replacement")
                .expect("replacement identity")
                .process,
            replacement
        );
    }

    #[test]
    fn dead_process_during_handle_acquisition_refuses_before_signaling() {
        let temp = tempfile::TempDir::new().expect("temp dir");
        write_identity(&runtime_path(&temp), CAPTURED, 3000);
        let operations = FakeOperations {
            start_times: RefCell::new(VecDeque::from([Ok(Some(CAPTURED.start_time)), Ok(None)])),
            ..FakeOperations::matching()
        };

        let error = cmd_shut_down_with(temp.path(), Duration::from_secs(1), &operations)
            .expect_err("dead process must refuse");

        assert!(error.to_string().contains("dead process"));
        assert!(operations.signals.borrow().is_empty());
    }

    #[test]
    fn unexpected_process_errors_preserve_their_category() {
        let temp = tempfile::TempDir::new().expect("temp dir");
        write_identity(&runtime_path(&temp), CAPTURED, 3000);
        let operations = FakeOperations {
            start_times: RefCell::new(VecDeque::from([Err(io::Error::new(
                io::ErrorKind::PermissionDenied,
                "permission denied",
            ))])),
            ..FakeOperations::matching()
        };

        let error = cmd_shut_down_with(temp.path(), Duration::from_secs(1), &operations)
            .expect_err("permission errors must propagate");

        assert!(error.to_string().contains("cannot read process start-time"));
        assert!(!error.to_string().contains("dead process"));
        assert!(operations.signals.borrow().is_empty());
    }

    #[test]
    fn timeout_sends_one_signal_and_preserves_runtime_identity() {
        let temp = tempfile::TempDir::new().expect("temp dir");
        let path = runtime_path(&temp);
        write_identity(&path, CAPTURED, 3000);
        let bytes = fs::read(&path).expect("runtime bytes");
        let operations = FakeOperations {
            wait_result: false,
            ..FakeOperations::matching()
        };

        let error = cmd_shut_down_with(temp.path(), Duration::from_secs(1), &operations)
            .expect_err("timeout must fail");

        assert!(error.to_string().contains("timeout"));
        assert_eq!(*operations.signals.borrow(), vec![7]);
        assert_eq!(fs::read(path).expect("runtime bytes"), bytes);
    }

    #[test]
    fn replacement_runtime_identity_completes_after_captured_process_exits() {
        let temp = tempfile::TempDir::new().expect("temp dir");
        let path = runtime_path(&temp);
        write_identity(&path, CAPTURED, 3000);
        let replacement = RuntimeProcessIdentity {
            pid: 42,
            start_time: 202,
        };
        let replacement_path = path.clone();
        let operations = FakeOperations {
            on_wait: Some(Box::new(move || {
                write_identity(&replacement_path, replacement, 3001);
            })),
            ..FakeOperations::matching()
        };

        cmd_shut_down_with(temp.path(), Duration::from_secs(1), &operations)
            .expect("replacement identity is completion");
        assert_eq!(*operations.signals.borrow(), vec![7]);
    }

    #[test]
    fn exit_before_signal_delivery_can_still_complete() {
        let temp = tempfile::TempDir::new().expect("temp dir");
        let path = runtime_path(&temp);
        write_identity(&path, CAPTURED, 3000);
        let removed_path = path.clone();
        let operations = FakeOperations {
            signal_target_exited: true,
            on_signal: Some(Box::new(move || {
                fs::remove_file(&removed_path).expect("remove identity");
            })),
            ..FakeOperations::matching()
        };

        cmd_shut_down_with(temp.path(), Duration::from_secs(1), &operations)
            .expect("an exited captured process can satisfy completion");
        assert!(operations.signals.borrow().is_empty());
    }

    #[test]
    fn absent_runtime_identity_completes_after_captured_process_exits() {
        let temp = tempfile::TempDir::new().expect("temp dir");
        let path = runtime_path(&temp);
        write_identity(&path, CAPTURED, 3000);
        let removed_path = path.clone();
        let operations = FakeOperations {
            on_open: Some(Box::new(move || {
                fs::remove_file(&removed_path).expect("remove identity");
            })),
            ..FakeOperations::matching()
        };

        cmd_shut_down_with(temp.path(), Duration::from_secs(1), &operations)
            .expect("absent identity is completion");
        assert_eq!(*operations.signals.borrow(), vec![7]);
    }

    #[test]
    fn unchanged_runtime_identity_after_exit_refuses() {
        let temp = tempfile::TempDir::new().expect("temp dir");
        write_identity(&runtime_path(&temp), CAPTURED, 3000);
        let operations = FakeOperations::matching();

        let error = cmd_shut_down_with(temp.path(), Duration::from_secs(1), &operations)
            .expect_err("unchanged identity must refuse");

        assert!(error.to_string().contains("runtime identity still owned"));
        assert_eq!(*operations.signals.borrow(), vec![7]);
    }
}
