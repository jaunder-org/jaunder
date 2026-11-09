//! Application readiness, status and publication-error ordering with real children.
use super::*;
use std::{fs, process::Stdio};

#[test]
fn native_tree() {
    let Some(root) = std::env::var_os("UX_SESSION_TREE") else {
        return;
    };
    let root = std::path::PathBuf::from(root);
    let descendant = std::env::var_os("UX_SESSION_DESCENDANT").is_some();
    tokio::runtime::Runtime::new().unwrap().block_on(async {
        let mut term = signal(SignalKind::terminate()).unwrap();
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
        if !descendant && let Some(path) = std::env::var_os("UX_SESSION_RUNTIME_ERROR") {
            tokio::time::timeout(Duration::from_secs(5), async {
                while !root.join("descendant").exists() {
                    sleep(Duration::from_millis(10)).await;
                }
            })
            .await
            .unwrap();
            fs::create_dir(path).unwrap();
        }
        if descendant && std::env::var_os("UX_SESSION_OBSERVE").is_some() {
            let resource = fs::OpenOptions::new()
                .create(true)
                .truncate(false)
                .write(true)
                .open(root.join("descendant-resource"))
                .unwrap();
            resource.lock().unwrap();
            let mut release = signal(SignalKind::user_defined1()).unwrap();
            fs::write(root.join("resource-held"), "").unwrap();
            loop {
                tokio::select! {
                    _ = term.recv() => { fs::write(root.join("drain-observed"), "").unwrap(); },
                    _ = release.recv() => return,
                }
            }
        }
        std::future::pending::<()>().await;
    });
}

async fn assert_unconfirmed_retains_exclusion(
    root: &Path,
    stopped: &processkit::Result<processkit::ShutdownReport>,
    waited: &io::Result<ExitStatus>,
) {
    let guard_path = root.join("unconfirmed.lock");
    let guard = fs::File::create(&guard_path).unwrap();
    guard.lock().unwrap();
    let probe = fs::File::open(&guard_path).unwrap();
    {
        // A real reaped-child/nonempty-group outcome cannot release application
        // exclusion. Only this test cancels the decision to finish its fixture.
        let custody = async move {
            let _guard = guard;
            confirm_terminal(stopped, waited).await;
        };
        tokio::pin!(custody);
        assert!(
            tokio::time::timeout(Duration::from_millis(20), &mut custody)
                .await
                .is_err()
        );
        assert!(matches!(
            probe.try_lock(),
            Err(fs::TryLockError::WouldBlock)
        ));
    }
    probe.try_lock().unwrap();
}

#[tokio::test]
async fn bounded_observation_confirms_descendant_exit_under_guard_and_consumes_signals() {
    if std::env::var_os("UX_OBSERVATION_TEST").is_none() {
        let output = std::process::Command::new(std::env::current_exe().unwrap())
            .args(["--exact", "session::tests::bounded_observation_confirms_descendant_exit_under_guard_and_consumes_signals", "--nocapture"])
            .env("UX_OBSERVATION_TEST", "1").output().unwrap();
        assert!(
            output.status.success(),
            "{}\n{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
        return;
    }
    let root = tempfile::tempdir().unwrap();
    let guard_path = root.path().join("workspace.lock");
    let guard = fs::File::create(&guard_path).unwrap();
    guard.lock().unwrap();
    let group = ProcessGroup::new().unwrap();
    let mut command = Command::new(std::env::current_exe().unwrap());
    command
        .args(["--exact", "session::tests::native_tree", "--nocapture"])
        .env("UX_SESSION_TREE", root.path())
        .env("UX_SESSION_OBSERVE", "1")
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    let mut child = group.spawn(command).unwrap();
    tokio::time::timeout(Duration::from_secs(5), async {
        while !root.path().join("resource-held").exists() {
            sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .unwrap();
    // Keep a real descendant until explicitly released: exercise our observation
    // deterministically, without depending on how quickly SIGKILL is scheduled.
    child.start_kill().unwrap();
    let waited = child.wait().await;
    assert!(waited.is_ok());
    let snapshot = group.stop(Duration::ZERO, false).await;
    assert!(
        snapshot
            .as_ref()
            .unwrap()
            .members_after()
            .is_some_and(|n| n > 0)
    );
    assert_unconfirmed_retains_exclusion(root.path(), &snapshot, &waited).await;
    let descendant: u32 = fs::read_to_string(root.path().join("descendant"))
        .unwrap()
        .parse()
        .unwrap();
    let mut signals = Signals::new().unwrap();
    let control = async {
        let send = |kind: &str, pid: u32| {
            assert!(
                std::process::Command::new("kill")
                    .args([kind, &pid.to_string()])
                    .status()
                    .unwrap()
                    .success()
            );
        };
        send("-INT", std::process::id());
        sleep(Duration::from_millis(20)).await;
        send("-TERM", std::process::id());
        sleep(Duration::from_millis(20)).await;
        send("-INT", std::process::id());
        sleep(Duration::from_millis(20)).await;
        let resource = fs::File::open(root.path().join("descendant-resource")).unwrap();
        assert!(
            matches!(resource.try_lock(), Err(fs::TryLockError::WouldBlock)),
            "signals must not force the observation's descendant"
        );
        let probe = fs::File::open(&guard_path).unwrap();
        assert!(
            matches!(probe.try_lock(), Err(fs::TryLockError::WouldBlock)),
            "workspace guard released before confirmation"
        );
        send("-USR1", descendant);
    };
    let (report, ()) = tokio::join!(observe_forced_drain(&group, &mut signals), control);
    assert_eq!(report.unwrap().members_after(), Some(0));
    assert_eq!(
        signals.status(),
        Some(130),
        "first post-child-exit signal survives"
    );
    assert!(
        tokio::time::timeout(Duration::from_millis(1), signals.next())
            .await
            .is_err(),
        "post-force observation left signals unconsumed"
    );
    let probe = fs::File::open(&guard_path).unwrap();
    assert!(matches!(
        probe.try_lock(),
        Err(fs::TryLockError::WouldBlock)
    ));
    drop(guard);
    probe.try_lock().unwrap();
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
    let error = tokio::time::timeout(
        Duration::from_secs(5),
        run_owned(
            command,
            None,
            &mut signals,
            Some(&mut server),
            std::process::Stdio::inherit(),
        ),
    )
    .await
    .expect("application cleanup did not finish")
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
    let error = tokio::time::timeout(
        Duration::from_secs(5),
        run_owned(
            command,
            Some((&runtime, Profile::Empty)),
            &mut signals,
            Some(&mut server),
            std::process::Stdio::inherit(),
        ),
    )
    .await
    .expect("application cleanup did not finish")
    .unwrap_err();
    assert!(error.to_string().contains("reading runtime-info"));
    assert!(error.downcast_ref::<io::Error>().is_some());
    // The helper owns/reaps its direct child. Descendant lifecycle/resource
    // ordering is exercised through the public helper, not an instant PID sample.
    let pid = fs::read_to_string(root.path().join("parent"))
        .unwrap()
        .trim()
        .parse()
        .unwrap();
    assert!(processkit::process_info(pid).unwrap().is_none());
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
