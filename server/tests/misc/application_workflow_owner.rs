//! Linux test-only ownership of an actual application child and its resources.

use std::{
    fs::{File, OpenOptions},
    io,
    os::unix::fs::OpenOptionsExt,
    path::{Path, PathBuf},
    process::{Child, Command, ExitStatus, Stdio},
    thread,
    time::{Duration, Instant},
};

use rustix::{
    fd::OwnedFd,
    process::{Pid, PidfdFlags, Signal},
};

// Reserve time for confirmed failure cleanup inside the existing 30-second
// nextest test budget. This does not change the product worker deadline.
const CHILD_TIMEOUT: Duration = Duration::from_secs(10);
const CLEANUP_TIMEOUT: Duration = Duration::from_secs(5);

pub(super) fn workflow_deadline() -> Instant {
    Instant::now() + Duration::from_secs(20)
}

pub(super) struct ApplicationOwner<R> {
    resources: Option<R>,
    child: Option<Child>,
    pidfd: Option<OwnedFd>,
    uncertain_wait: bool,
    deadline: Instant,
    executable: PathBuf,
    stdout: PathBuf,
    stderr: PathBuf,
}

impl<R> ApplicationOwner<R> {
    pub(super) fn new(resources: R, diagnostics: &Path, deadline: Instant) -> Self {
        let executable = std::env::var_os("JAUNDER_APPLICATION_BIN")
            .map(PathBuf::from)
            .expect("explicit production application fixture");
        assert!(executable.is_absolute(), "absolute application fixture");
        Self {
            resources: Some(resources),
            child: None,
            pidfd: None,
            uncertain_wait: false,
            deadline,
            executable,
            stdout: diagnostics.join("application.stdout"),
            stderr: diagnostics.join("application.stderr"),
        }
    }

    pub(super) fn deadline(&self) -> Instant {
        self.deadline
    }

    pub(super) fn command(&self, name: &str) -> Command {
        let mut command = Command::new(&self.executable);
        command.arg(name);
        command
    }

    pub(super) fn spawn(&mut self, mut command: Command) -> io::Result<()> {
        if self.child.is_some() {
            return Err(io::Error::other("application child already owned"));
        }
        command.stdout(Stdio::from(private_capture(&self.stdout)?));
        command.stderr(Stdio::from(private_capture(&self.stderr)?));
        self.child = Some(command.spawn()?);
        let raw = i32::try_from(self.pid()).map_err(io::Error::other)?;
        let pid = Pid::from_raw(raw).ok_or_else(|| io::Error::other("invalid owned child PID"))?;
        // The PID belongs to our unreaped std::Child, not a runtime-file lookup.
        self.pidfd = Some(rustix::process::pidfd_open(pid, PidfdFlags::empty())?);
        Ok(())
    }

    pub(super) fn pid(&self) -> u32 {
        self.child.as_ref().expect("spawned application child").id()
    }

    pub(super) fn observe_exit(&mut self) -> io::Result<Option<ExitStatus>> {
        let Some(child) = self.child.as_mut() else {
            return Err(io::Error::other("application child not owned"));
        };
        match child.try_wait() {
            Ok(Some(status)) => {
                self.child.take();
                self.pidfd.take();
                Ok(Some(status))
            }
            Ok(None) => Ok(None),
            Err(error) => {
                self.uncertain_wait = true;
                Err(error)
            }
        }
    }

    pub(super) fn wait(&mut self) -> io::Result<ExitStatus> {
        self.wait_until(self.deadline.min(Instant::now() + CHILD_TIMEOUT))
    }

    fn wait_until(&mut self, deadline: Instant) -> io::Result<ExitStatus> {
        loop {
            if let Some(status) = self.observe_exit()? {
                return Ok(status);
            }
            if Instant::now() >= deadline {
                return Err(io::Error::new(
                    io::ErrorKind::TimedOut,
                    "owned application exit not confirmed",
                ));
            }
            thread::sleep(Duration::from_millis(1));
        }
    }

    pub(super) fn run(&mut self, command: Command) -> io::Result<ExitStatus> {
        self.spawn(command)?;
        self.wait()
    }

    pub(super) fn stderr(&self) -> String {
        String::from_utf8_lossy(&std::fs::read(&self.stderr).expect("application diagnostics"))
            .into_owned()
    }

    pub(super) fn stdout(&self) -> String {
        String::from_utf8_lossy(&std::fs::read(&self.stdout).expect("application receipt"))
            .into_owned()
    }

    pub(super) fn settle(&mut self) -> io::Result<()> {
        if self.child.is_none() {
            return Ok(());
        }
        if self.uncertain_wait {
            return Err(io::Error::other("application wait authority uncertain"));
        }
        if self.observe_exit()?.is_some() {
            return Ok(());
        }
        let fd = self
            .pidfd
            .as_ref()
            .ok_or_else(|| io::Error::other("owned application pidfd unavailable"))?;
        match rustix::process::pidfd_send_signal(fd, Signal::TERM) {
            Ok(()) | Err(rustix::io::Errno::SRCH) => {}
            Err(error) => return Err(error.into()),
        }
        self.wait_until(Instant::now() + CLEANUP_TIMEOUT)
            .map(|_| ())
    }
}

impl<R> Drop for ApplicationOwner<R> {
    fn drop(&mut self) {
        if let Err(error) = self.settle() {
            // An uncertain live child must not lose its database/temp ownership.
            // Keep the complete remaining owner; never escalate or guess a PID.
            if let Some(resources) = self.resources.take() {
                std::mem::forget(resources);
            }
            if let Some(child) = self.child.take() {
                std::mem::forget(child);
            }
            if let Some(pidfd) = self.pidfd.take() {
                std::mem::forget(pidfd);
            }
            eprintln!("owned application cleanup unconfirmed: {error}");
            assert!(thread::panicking(), "owned application cleanup unconfirmed");
        }
    }
}

fn private_capture(path: &Path) -> io::Result<File> {
    OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(true)
        .mode(0o600)
        .open(path)
}

// guard:no-backend — observes real child ownership loss; no database resources.
#[test]
fn actual_application_external_reap_retains_uncertain_owner_resources() {
    use std::sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    };
    struct ResourceWitness(Arc<AtomicBool>);
    impl Drop for ResourceWitness {
        fn drop(&mut self) {
            self.0.store(true, Ordering::SeqCst);
        }
    }
    let diagnostics = tempfile::tempdir().expect("private ownership-loss diagnostics");
    let released = Arc::new(AtomicBool::new(false));
    let mut owner = ApplicationOwner::new(
        ResourceWitness(released.clone()),
        diagnostics.path(),
        workflow_deadline(),
    );
    let mut command = owner.command("--invalid-owned-observer-argument");
    command.env_remove("JAUNDER_DB");
    owner.spawn(command).expect("real owned application child");
    let pid = Pid::from_raw(i32::try_from(owner.pid()).expect("owned PID")).expect("positive PID");
    // A controlled independent reaper removes std::Child's wait authority.
    // This is ownership-loss/retention evidence, not this owner's reap proof.
    loop {
        if rustix::process::waitpid(Some(pid), rustix::process::WaitOptions::NOHANG)
            .expect("controlled external reap")
            .is_some()
        {
            break;
        }
        assert!(
            Instant::now() < owner.deadline(),
            "controlled external reap deadline"
        );
        // On timeout/panic the intact owner performs bounded settlement; only
        // an actual external reap proceeds to the deliberate ECHILD assertion.
        thread::sleep(Duration::from_millis(1));
    }
    assert_eq!(
        owner
            .observe_exit()
            .expect_err("lost wait authority")
            .raw_os_error(),
        Some(rustix::io::Errno::CHILD.raw_os_error())
    );
    let dropped = std::panic::catch_unwind(move || drop(owner));
    let panic = dropped.expect_err("unconfirmed owner cleanup must fail closed");
    let message = panic
        .downcast_ref::<&str>()
        .expect("specific cleanup panic");
    assert_eq!(*message, "owned application cleanup unconfirmed");
    assert!(
        !released.load(Ordering::SeqCst),
        "uncertain ownership cannot release resources"
    );
}
