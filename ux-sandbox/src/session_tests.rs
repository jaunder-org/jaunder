//! Local one-shot operation failures retain real COTS children, groups and exclusion.
use super::*;
use std::{fs, process::Stdio};

#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) enum Operation {
    Wait,
    Signal,
    Members,
    Kill,
    Confirmation,
    Stop,
}

impl OwnedProcess {
    pub(super) fn fail_once(&mut self, operation: Operation) -> anyhow::Result<()> {
        if self.failure == Some(operation) {
            self.failure = None;
            return Err(io::Error::new(
                io::ErrorKind::BrokenPipe,
                format!("one-shot {operation:?}"),
            )
            .into());
        }
        Ok(())
    }
}

#[test]
fn native_tree() {
    let Some(root) = std::env::var_os("UX_SESSION_TREE") else {
        return;
    };
    let root = std::path::PathBuf::from(root);
    let descendant = std::env::var_os("UX_SESSION_DESCENDANT").is_some();
    tokio::runtime::Runtime::new().unwrap().block_on(async {
        let _term = signal(SignalKind::terminate()).unwrap();
        let _int = signal(SignalKind::interrupt()).unwrap();
        if !descendant {
            let mut child = std::process::Command::new(std::env::current_exe().unwrap())
                .args(["--exact", "session::tests::native_tree", "--nocapture"])
                .env("UX_SESSION_DESCENDANT", "1")
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .spawn()
                .unwrap();
            std::thread::spawn(move || child.wait().unwrap());
        }
        fs::write(
            root.join(if descendant { "descendant" } else { "parent" }),
            std::process::id().to_string(),
        )
        .unwrap();
        std::future::pending::<()>().await;
    });
}

#[test]
fn lock_probe() {
    let Some(path) = std::env::var_os("UX_SESSION_LOCK") else {
        return;
    };
    let lock = fs::OpenOptions::new()
        .read(true)
        .write(true)
        .open(path)
        .unwrap();
    assert!(matches!(lock.try_lock(), Err(fs::TryLockError::WouldBlock)));
}

fn assert_lock_held(path: &Path) {
    assert!(
        std::process::Command::new(std::env::current_exe().unwrap())
            .args(["--exact", "session::tests::lock_probe"])
            .env("UX_SESSION_LOCK", path)
            .status()
            .unwrap()
            .success()
    );
}

async fn tree(root: &Path, operation: Operation) -> OwnedProcess {
    let group = ProcessGroup::new().unwrap();
    let mut command = Command::new(std::env::current_exe().unwrap());
    command
        .args(["--exact", "session::tests::native_tree", "--nocapture"])
        .env("UX_SESSION_TREE", root)
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    let child = group.spawn(command).unwrap();
    tokio::time::timeout(Duration::from_secs(5), async {
        while !root.join("parent").exists() || !root.join("descendant").exists() {
            sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .unwrap();
    OwnedProcess {
        group,
        child,
        reaped: false,
        failure: Some(operation),
    }
}

#[tokio::test]
async fn operation_errors_finalize_real_tree_before_return_and_lock_release() {
    for operation in [
        Operation::Wait,
        Operation::Signal,
        Operation::Members,
        Operation::Kill,
        Operation::Confirmation,
        Operation::Stop,
    ] {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("host-artifacts.lock");
        let lock = fs::OpenOptions::new()
            .create(true)
            .truncate(false)
            .read(true)
            .write(true)
            .open(&path)
            .unwrap();
        lock.lock().unwrap();
        let mut process = tree(root.path(), operation).await;
        let mut signals = Signals::new().unwrap();
        let result = match operation {
            Operation::Wait => {
                let observed = process.observe(None, &mut signals).await;
                process.finish(observed, &mut signals).await
            }
            Operation::Members => {
                process.child.start_kill().unwrap();
                process.child.wait().await.unwrap();
                process.reaped = true;
                process.finish(Ok(23), &mut signals).await
            }
            Operation::Stop => {
                process
                    .finish(
                        Err(
                            io::Error::new(io::ErrorKind::BrokenPipe, "initiating read failure")
                                .into(),
                        ),
                        &mut signals,
                    )
                    .await
            }
            _ => process.finish(Ok(23), &mut signals).await,
        };
        let error = result.unwrap_err();
        if operation == Operation::Confirmation {
            assert!(
                error
                    .to_string()
                    .contains("terminal confirmation timed out")
            );
        } else {
            assert_eq!(
                error.downcast_ref::<io::Error>().unwrap().kind(),
                io::ErrorKind::BrokenPipe
            );
        }
        assert_lock_held(&path);
        let confirmed = process.reaped && process.group.members().unwrap().is_empty();
        if !confirmed {
            // A red regression must still clean up its real native fixture tree.
            process.failure = None;
            process.force_and_confirm().await;
        }
        assert!(
            confirmed,
            "{operation:?} returned before owned terminal confirmation"
        );
        for role in ["parent", "descendant"] {
            let pid = fs::read_to_string(root.path().join(role))
                .unwrap()
                .parse()
                .unwrap();
            assert!(
                processkit::process_info(pid).unwrap().is_none(),
                "{operation:?}: {role} remains"
            );
        }
        lock.unlock().unwrap();
    }
}

#[tokio::test]
async fn unexpected_runtime_read_error_keeps_typed_source_and_path() {
    let root = tempfile::tempdir().unwrap();
    let error = ready(root.path(), std::process::id()).await.unwrap_err();
    assert!(error.downcast_ref::<io::Error>().is_some());
    assert!(format!("{error:#}").contains(&root.path().display().to_string()));
    assert!(!format!("{error:#}").contains("exceeded 30 seconds"));
}

#[test]
fn platform_identity_matches_server_runtime_schema() {
    let pid = std::process::id();
    let identity = start_time(pid).unwrap();
    assert_eq!(
        Some(identity),
        processkit::process_info(pid).unwrap().unwrap().start_time()
    );
    #[cfg(target_os = "linux")]
    {
        let stat = fs::read_to_string(format!("/proc/{pid}/stat")).unwrap();
        let ticks: u64 = stat
            .rsplit_once(')')
            .unwrap()
            .1
            .split_whitespace()
            .nth(19)
            .unwrap()
            .parse()
            .unwrap();
        assert_eq!(
            identity, ticks,
            "Linux runtime JSON remains clock ticks, not wall time"
        );
    }
    let info: RuntimeInfo = serde_json::from_value(
        serde_json::json!({"ip":"127.0.0.1", "port":1234, "pid":pid, "start_time":identity}),
    )
    .unwrap();
    assert_eq!(info.start_time, identity);
}
