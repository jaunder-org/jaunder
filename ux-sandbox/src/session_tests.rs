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
        publish_runtime_error(&root, descendant).await;
        std::future::pending::<()>().await;
    });
}

async fn publish_runtime_error(root: &Path, descendant: bool) {
    if descendant {
        return;
    }
    if let Some(path) = std::env::var_os("UX_SESSION_RUNTIME_ERROR") {
        tokio::time::timeout(Duration::from_secs(5), async {
            while !root.join("descendant").exists() {
                sleep(Duration::from_millis(10)).await;
            }
        })
        .await
        .unwrap();
        fs::create_dir(path).unwrap();
    }
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
async fn wait_error_finalizes_real_tree_before_lock_release() {
    exercise_operation_failure(Operation::Wait).await;
}
#[tokio::test]
async fn signal_error_finalizes_real_tree_before_lock_release() {
    exercise_operation_failure(Operation::Signal).await;
}
#[tokio::test]
async fn membership_error_finalizes_real_tree_before_lock_release() {
    exercise_operation_failure(Operation::Members).await;
}
#[tokio::test]
async fn kill_error_finalizes_real_tree_before_lock_release() {
    exercise_operation_failure(Operation::Kill).await;
}
#[tokio::test]
async fn confirmation_error_finalizes_real_tree_before_lock_release() {
    exercise_operation_failure(Operation::Confirmation).await;
}
#[tokio::test]
async fn stop_error_finalizes_real_tree_before_lock_release() {
    exercise_operation_failure(Operation::Stop).await;
}

async fn observe_failure(
    process: &mut OwnedProcess,
    operation: Operation,
    signals: &mut Signals,
) -> anyhow::Result<i32> {
    match operation {
        Operation::Wait => process.observe(None, signals, None).await,
        Operation::Members => {
            process.child.start_kill().unwrap();
            process.child.wait().await.unwrap();
            process.reaped = true;
            Ok(23)
        }
        Operation::Stop => {
            Err(io::Error::new(io::ErrorKind::BrokenPipe, "initiating read failure").into())
        }
        _ => Ok(23),
    }
}

fn assert_operation_error(error: &anyhow::Error, operation: Operation) {
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
}

async fn assert_terminal(process: &mut OwnedProcess) {
    let reaped = process.reaped;
    let empty = process.group.members().unwrap().is_empty();
    if !reaped || !empty {
        // Even a red regression must clean up its native fixture tree.
        process.failure = None;
        process.force_and_confirm().await;
    }
    assert!(reaped, "returned before child reap");
    assert!(empty, "returned before owned terminal confirmation");
}

fn assert_tree_gone(root: &Path) {
    for role in ["parent", "descendant"] {
        let pid = fs::read_to_string(root.join(role))
            .unwrap()
            .parse()
            .unwrap();
        assert!(
            processkit::process_info(pid).unwrap().is_none(),
            "{role} remains"
        );
    }
}

async fn exercise_operation_failure(operation: Operation) {
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
    let observed = observe_failure(&mut process, operation, &mut signals).await;
    let result = process.finish(observed, &mut signals).await;
    assert_operation_error(&result.unwrap_err(), operation);
    assert_lock_held(&path);
    assert_terminal(&mut process).await;
    assert_tree_gone(root.path());
    lock.unlock().unwrap();
    let next = fs::OpenOptions::new()
        .read(true)
        .write(true)
        .open(&path)
        .unwrap();
    next.try_lock()
        .expect("caller releases its artifact lock only after terminal confirmation");
}

#[tokio::test]
async fn unexpected_runtime_read_error_keeps_typed_source_and_path() {
    let root = tempfile::tempdir().unwrap();
    let error = ready(root.path(), std::process::id()).await.unwrap_err();
    assert!(error.downcast_ref::<io::Error>().is_some());
    assert!(format!("{error:#}").contains(&root.path().display().to_string()));
    assert!(!format!("{error:#}").contains("exceeded 30 seconds"));
}

#[tokio::test]
async fn already_interrupted_session_does_not_spawn_a_child() {
    let mut signals = Signals::new().unwrap();
    signals.initiating = Some(143);
    assert_eq!(
        run(
            Command::new("/nonexistent/should-not-spawn"),
            None,
            &mut signals
        )
        .await
        .unwrap(),
        143
    );
}

fn broken_publication_server(root: &Path) -> crate::workspace::Server {
    use crate::workspace::{Lock, Named, Server};
    let paths = Named::open(root, &"writing".parse().unwrap()).unwrap();
    let lease = Lock::try_exclusive(&paths.lease).unwrap().unwrap();
    let exclusive = Lock::try_exclusive(&paths.ownership).unwrap().unwrap();
    let control = paths.metadata.parent().unwrap().to_owned();
    let server = Server::new(paths, lease, exclusive, "abc".into());
    fs::remove_dir_all(control).unwrap();
    server
}

#[tokio::test]
async fn stopping_publication_failure_replaces_success_only_after_child_reap() {
    let root = tempfile::tempdir().unwrap();
    let mut server = broken_publication_server(root.path());
    let mut command = Command::new("sh");
    command
        .args(["-c", "echo $$ > child-pid; exit 23"])
        .current_dir(root.path());
    let mut signals = Signals::new().unwrap();
    let error = run_owned(command, None, &mut signals, Some(&mut server))
        .await
        .unwrap_err();
    assert!(error.to_string().contains("sandbox root disappeared"));
    let pid = fs::read_to_string(root.path().join("child-pid"))
        .unwrap()
        .trim()
        .parse()
        .unwrap();
    assert!(processkit::process_info(pid).unwrap().is_none());
}

#[tokio::test]
async fn stopping_publication_failure_preserves_original_read_error_after_tree_reap() {
    let root = tempfile::tempdir().unwrap();
    let mut server = broken_publication_server(root.path());
    let runtime = root.path().join("runtime.json");
    let mut command = Command::new(std::env::current_exe().unwrap());
    command
        .args(["--exact", "session::tests::native_tree", "--nocapture"])
        .env("UX_SESSION_TREE", root.path())
        .env("UX_SESSION_RUNTIME_ERROR", &runtime);
    let mut signals = Signals::new().unwrap();
    let error = run_owned(command, Some(&runtime), &mut signals, Some(&mut server))
        .await
        .unwrap_err();
    assert!(error.to_string().contains("reading runtime-info"));
    assert!(error.downcast_ref::<io::Error>().is_some());
    assert_tree_gone(root.path());
}

async fn readiness_server(
    responses: &'static [&'static [u8]],
) -> (SocketAddr, tokio::task::JoinHandle<()>) {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let task = tokio::spawn(async move {
        for response in responses {
            let (mut stream, _) = listener.accept().await.unwrap();
            let mut request = [0; 1024];
            let count = stream.read(&mut request).await.unwrap();
            assert!(request[..count].starts_with(b"GET / HTTP/1.0\r\n"));
            stream.write_all(response).await.unwrap();
        }
    });
    (address, task)
}

#[tokio::test]
async fn readiness_retries_http_rejection_before_accepting_identity_matched_server() {
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("runtime.json");
    let (address, task) =
        readiness_server(&[b"HTTP/1.0 503 Starting\r\n\r\n", b"HTTP/1.0 200 OK\r\n\r\n"]).await;
    let pid = std::process::id();
    fs::write(&path, serde_json::to_vec(&serde_json::json!({"ip":"127.0.0.1", "port":address.port(), "pid":pid, "start_time":start_time(pid).unwrap()})).unwrap()).unwrap();
    let found = tokio::time::timeout(Duration::from_secs(3), ready(&path, pid))
        .await
        .unwrap()
        .unwrap();
    assert_eq!(found, address);
    task.await.unwrap();
}

#[tokio::test]
async fn live_readiness_rejects_malformed_runtime_and_non_200_http() {
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("runtime.json");
    fs::write(&path, b"{}").unwrap();
    let error = live_ready(&path).await.unwrap_err();
    assert!(error.downcast_ref::<serde_json::Error>().is_some());
    let (address, task) = readiness_server(&[b"HTTP/1.1 503 Starting\r\n\r\n"]).await;
    let pid = std::process::id();
    fs::write(&path, serde_json::to_vec(&serde_json::json!({"ip":"127.0.0.1", "port":address.port(), "pid":pid, "start_time":start_time(pid).unwrap()})).unwrap()).unwrap();
    assert_eq!(
        live_ready(&path).await.unwrap_err().to_string(),
        "HTTP readiness did not return 200"
    );
    task.await.unwrap();
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
