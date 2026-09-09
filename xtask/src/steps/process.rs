//! Synchronous ownership of a processkit-supervised process.
//!
//! The running process precedes the runtime so containment cleanup always runs
//! while the executor that drives processkit remains available.

use std::net::SocketAddr;
use std::path::Path;
use std::time::Duration;

use anyhow::Context;
use processkit::{Command, Outcome, RunningProcess};
use tokio::runtime::{Builder, Runtime};

/// A synchronous facade over one processkit process and the runtime that drives it.
pub(super) struct Process {
    running: Option<RunningProcess>,
    runtime: Runtime,
    stopped: bool,
    identity: Option<(u32, Option<u64>)>,
}

impl Process {
    pub(super) fn start(command: Command) -> anyhow::Result<Self> {
        let runtime = Builder::new_multi_thread()
            .worker_threads(1)
            .enable_all()
            .build()
            .context("creating processkit runtime")?;
        let running = runtime
            .block_on(command.start())
            .map_err(anyhow::Error::from)?;
        let (identity, stopped) = match running.pid() {
            Some(pid) => match processkit::process_info(pid) {
                Ok(Some(info)) => (Some((pid, info.start_time())), false),
                Ok(None) => (None, true),
                Err(_) => (None, false),
            },
            None => (None, true),
        };
        Ok(Self {
            running: Some(running),
            runtime,
            stopped,
            identity,
        })
    }

    pub(super) fn wait(mut self) -> anyhow::Result<Outcome> {
        let running = self.take_running()?;
        let result = self
            .runtime
            .block_on(running.wait())
            .map_err(anyhow::Error::from);
        self.stopped = result.is_ok();
        result
    }

    pub(super) fn wait_for_port(
        &mut self,
        endpoint: SocketAddr,
        within: Duration,
    ) -> anyhow::Result<()> {
        let running = self
            .running
            .as_mut()
            .ok_or_else(|| anyhow::anyhow!("process was already stopped"))?;
        self.runtime
            .block_on(running.wait_for_port(endpoint, within))
            .map_err(anyhow::Error::from)
    }

    pub(super) fn wait_for_path(&mut self, path: &Path, within: Duration) -> anyhow::Result<()> {
        let running = self
            .running
            .as_mut()
            .ok_or_else(|| anyhow::anyhow!("process was already stopped"))?;
        self.runtime
            .block_on(running.wait_for_path(path, within))
            .map_err(anyhow::Error::from)
    }

    pub(super) fn shutdown(&mut self, grace: Duration) -> anyhow::Result<Outcome> {
        let running = self.take_running()?;
        let result = self
            .runtime
            .block_on(running.shutdown(grace))
            .map_err(anyhow::Error::from);
        self.stopped = result.is_ok();
        result
    }

    pub(super) fn shutdown_interruptible<F>(
        &mut self,
        grace: Duration,
        escalation: F,
    ) -> anyhow::Result<Option<Outcome>>
    where
        F: Future<Output = ()>,
    {
        let running = self.take_running()?;
        let result = self.runtime.block_on(async move {
            let shutdown = running.shutdown(grace);
            tokio::pin!(shutdown);
            tokio::pin!(escalation);
            tokio::select! {
                outcome = &mut shutdown => outcome.map(Some).map_err(anyhow::Error::from),
                () = &mut escalation => Ok(None),
            }
        });
        self.stopped = result.is_ok();
        result
    }

    pub(super) fn is_stopped(&self) -> bool {
        self.stopped
            || self
                .identity
                .is_some_and(|(pid, start_time)| process_identity_is_stopped(pid, start_time))
            || self
                .running
                .as_ref()
                .is_some_and(|running| running.pid().is_none())
    }

    pub(super) fn identity(&self) -> Option<(u32, Option<u64>)> {
        self.identity
    }

    fn take_running(&mut self) -> anyhow::Result<RunningProcess> {
        self.running
            .take()
            .ok_or_else(|| anyhow::anyhow!("process was already stopped"))
    }
}

/// `ESRCH` means the process vanished while processkit was resolving its
/// identity, so it is an observed stopped state rather than a probe failure.
fn process_identity_is_stopped(pid: u32, start_time: Option<u64>) -> bool {
    match processkit::process_is_alive(pid, start_time) {
        Ok(false) => true,
        Ok(true) => {
            #[cfg(target_os = "linux")]
            {
                let stat_path = format!("/proc/{pid}/stat");
                std::fs::read_to_string(stat_path).is_ok_and(|stat| {
                    stat.rsplit_once(") ")
                        .is_some_and(|(_, fields)| fields.starts_with("Z "))
                })
            }
            #[cfg(not(target_os = "linux"))]
            false
        }
        Err(error) => match error.reason() {
            processkit::ErrorReason::Io(source) => process_probe_reports_stopped(source),
            _ => false,
        },
    }
}

#[cfg(unix)]
fn process_probe_reports_stopped(error: &std::io::Error) -> bool {
    // POSIX reserves errno 3 for ESRCH.
    const ESRCH: i32 = 3;

    error.raw_os_error() == Some(ESRCH)
}

#[cfg(not(unix))]
fn process_probe_reports_stopped(_: &std::io::Error) -> bool {
    false
}

impl Drop for Process {
    fn drop(&mut self) {
        // Processkit's Drop tears down the whole child tree. Do it explicitly
        // before Runtime's Drop shuts down the worker that supports that cleanup.
        std::mem::drop(self.running.take());
    }
}

#[cfg(all(test, unix))]
mod tests {
    use std::fs;
    use std::time::{Duration, Instant};

    use processkit::{Command, Outcome};

    use super::{Process, process_identity_is_stopped, process_probe_reports_stopped};

    #[test]
    fn wait_returns_the_child_outcome() {
        let outcome = Process::start(Command::new("sh").args(["-c", "exit 7"]))
            .expect("start shell")
            .wait()
            .expect("wait for shell");

        assert_eq!(outcome, Outcome::Exited(7));
    }

    #[test]
    fn start_reports_a_missing_program() {
        let error = Process::start(Command::new("jaunder-definitely-missing-process"))
            .err()
            .expect("missing program must fail to start");

        assert!(!error.to_string().is_empty());
    }

    #[test]
    #[cfg(target_os = "linux")]
    fn second_signal_path_forces_cleanup_without_waiting_for_grace() {
        let directory = tempfile::tempdir().expect("create temporary directory");
        let ready_path = directory.path().join("ready");
        let mut process = Process::start(
            Command::new("sh")
                .args(["-c", "trap '' TERM; : > \"$1\"; sleep 60", "--"])
                .arg(&ready_path),
        )
        .expect("start signal-resistant process");
        let pid = process
            .running
            .as_ref()
            .and_then(|running| running.pid())
            .expect("running process id");
        let start_time = processkit::process_info(pid)
            .expect("inspect process")
            .expect("process remains alive")
            .start_time();
        let ready_deadline = Instant::now() + Duration::from_secs(5);
        while !ready_path.exists() && Instant::now() < ready_deadline {
            std::thread::sleep(Duration::from_millis(20));
        }
        assert!(ready_path.exists(), "child installed TERM trap");

        let started = Instant::now();
        let outcome = process
            .shutdown_interruptible(Duration::from_secs(60), async {})
            .expect("force shutdown");

        assert_eq!(outcome, None);
        assert!(started.elapsed() < Duration::from_secs(5));
        let deadline = Instant::now() + Duration::from_secs(5);
        while !process_identity_is_stopped(pid, start_time) && Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(20));
        }
        assert!(process_identity_is_stopped(pid, start_time));
    }

    #[test]
    fn esrch_identity_probe_means_process_stopped() {
        // The PID can disappear after a liveness check but before its identity
        // is read. That is a terminal lifecycle state, not a probe failure.
        let error = std::io::Error::from_raw_os_error(3);

        assert!(process_probe_reports_stopped(&error));
    }

    #[test]
    #[cfg(target_os = "linux")]
    fn drop_cleans_up_the_tree_without_losing_captured_output() {
        let directory = tempfile::tempdir().expect("create temporary directory");
        let pids_path = directory.path().join("pids");
        let capture_path = directory.path().join("stderr");
        let capture = fs::File::create(&capture_path).expect("create stderr capture");
        let mut process = Process::start(
            Command::new("sh")
                .arg("-c")
                .arg(
                    "printf 'captured-before-drop' >&2; sleep 60 & child=$!; printf '%s %s' \"$$\" \"$child\" > \"$1\"; wait",
                )
                .arg("--")
                .arg(&pids_path)
                .stderr_raw_tee(tokio::fs::File::from_std(capture)),
        )
        .expect("start process tree");
        process
            .wait_for_path(&pids_path, Duration::from_secs(5))
            .expect("wait for process tree IDs");
        let pids = wait_for_pids(&pids_path);
        wait_for_capture(&capture_path);
        let identities = pids
            .into_iter()
            .map(|pid| {
                let info = processkit::process_info(pid)
                    .expect("inspect process")
                    .expect("process remains alive before owner drop");
                (pid, info.start_time())
            })
            .collect::<Vec<_>>();

        drop(process);

        let deadline = Instant::now() + Duration::from_secs(5);
        while Instant::now() < deadline {
            if identities
                .iter()
                .all(|(pid, start_time)| process_identity_is_stopped(*pid, *start_time))
            {
                assert_eq!(
                    fs::read(&capture_path).expect("read stderr capture"),
                    b"captured-before-drop"
                );
                return;
            }
            std::thread::sleep(Duration::from_millis(20));
        }
        panic!("process tree remained alive after its owner dropped");
    }

    fn wait_for_pids(path: &std::path::Path) -> Vec<u32> {
        let deadline = Instant::now() + Duration::from_secs(5);
        while Instant::now() < deadline {
            if let Ok(pids) = fs::read_to_string(path) {
                let pids = pids
                    .split_whitespace()
                    .map(str::parse)
                    .collect::<Result<Vec<_>, _>>()
                    .expect("parse process IDs");
                if pids.len() == 2 {
                    return pids;
                }
            }
            std::thread::sleep(Duration::from_millis(20));
        }
        panic!("process tree did not publish parent and descendant IDs");
    }

    fn wait_for_capture(path: &std::path::Path) {
        let deadline = Instant::now() + Duration::from_secs(5);
        while Instant::now() < deadline {
            if fs::read(path).is_ok_and(|bytes| bytes == b"captured-before-drop") {
                return;
            }
            std::thread::sleep(Duration::from_millis(20));
        }
        panic!("stderr tee did not capture the pre-drop bytes");
    }
}
