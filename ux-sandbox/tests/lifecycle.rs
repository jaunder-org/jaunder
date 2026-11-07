//! Public helper fixtures use real native children, signals, file locks and HTTP.
#[cfg(test)]
mod tests {
    use std::{
        fs,
        io::{BufRead, BufReader},
        net::TcpListener,
        os::unix::fs::PermissionsExt,
        path::Path,
        process::{Command, Stdio},
        time::{Duration, Instant},
    };

    fn executable(path: &Path, body: &str) {
        fs::write(path, body).unwrap();
        fs::set_permissions(path, fs::Permissions::from_mode(0o755)).unwrap();
    }

    fn fixture_root() -> tempfile::TempDir {
        let root = tempfile::tempdir().unwrap();
        fs::create_dir_all(root.path().join("target/debug")).unwrap();
        fs::create_dir(root.path().join("bin")).unwrap();
        for name in ["cargo", "just"] {
            executable(
                &root.path().join("bin").join(name),
                &format!(
                    "#!/bin/sh\nprintf '%s\\n' '{name}' \"$*\" >> build-calls\nexec \"$UX_FIXTURE_EXE\" --exact tests::build_fixture --nocapture\n"
                ),
            );
        }
        executable(
            &root.path().join("target/debug/jaunder"),
            "#!/bin/sh\nexport UX_FIXTURE_MODE=\"$1\"\nexec \"$UX_FIXTURE_EXE\" --exact tests::app_fixture --nocapture\n",
        );
        executable(
            &root.path().join("target/debug/test-support"),
            "#!/bin/sh\nexit 0\n",
        );
        root
    }

    #[test]
    fn build_fixture() {
        let Ok(root) = std::env::var("UX_FIXTURE_ROOT") else {
            return;
        };
        let lock = fs::OpenOptions::new()
            .read(true)
            .write(true)
            .open(Path::new(&root).join(".xtask/host-artifacts.lock"))
            .unwrap();
        assert!(matches!(lock.try_lock(), Err(fs::TryLockError::WouldBlock)));
        fs::create_dir_all(Path::new(&root).join("target/site/.vite")).unwrap();
        fs::write(
            Path::new(&root).join("target/site/.vite/manifest.json"),
            "{}",
        )
        .unwrap();
    }

    #[test]
    fn app_fixture() {
        let Ok(mode) = std::env::var("UX_FIXTURE_MODE") else {
            return;
        };
        let storage = std::env::var("JAUNDER_STORAGE_PATH").unwrap();
        let root = std::env::var("UX_FIXTURE_ROOT").unwrap();
        assert_eq!(
            std::env::var("JAUNDER_DB").unwrap(),
            format!("sqlite:{storage}/jaunder.db")
        );
        assert_eq!(std::env::var("JAUNDER_ENV").unwrap(), "dev");
        assert!(std::env::var_os("JAUNDER_TRUSTED_PROXIES").is_none());
        let lock = fs::OpenOptions::new()
            .read(true)
            .write(true)
            .open(Path::new(&root).join(".xtask/host-artifacts.lock"))
            .unwrap();
        lock.try_lock()
            .expect("artifact preparation releases before app execution");
        lock.unlock().unwrap();
        if mode == "init" {
            // Match the real init contract: it exclusively creates new storage.
            fs::create_dir(&storage).unwrap();
            fs::write(Path::new(&storage).join("initialized"), "").unwrap();
            return;
        }
        assert_eq!(mode, "serve");
        assert!(Path::new(&storage).join("initialized").is_file());
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        let pid = std::process::id();
        fs::write(Path::new(&root).join("app-pid"), pid.to_string()).unwrap();
        fs::write(Path::new(&root).join("storage"), &storage).unwrap();
        let identity = processkit::process_info(pid)
            .unwrap()
            .unwrap()
            .start_time()
            .unwrap();
        let valid = format!(
            r#"{{"pid":{pid},"start_time":{identity},"ip":"127.0.0.1","port":{}}}"#,
            addr.port()
        );
        if std::env::var_os("UX_RUNTIME_IO_ERROR").is_some() {
            fs::create_dir(std::env::var("JAUNDER_RUNTIME_FILE").unwrap()).unwrap();
        } else if std::env::var_os("UX_INVALID_RUNTIME").is_some() {
            fs::write(std::env::var("JAUNDER_RUNTIME_FILE").unwrap(), "{}").unwrap();
            fs::write(Path::new(&root).join("valid-runtime"), valid).unwrap();
        } else {
            fs::write(std::env::var("JAUNDER_RUNTIME_FILE").unwrap(), valid).unwrap();
        }
        if std::env::var_os("UX_DESCENDANT").is_some() {
            let mut descendant = Command::new(std::env::current_exe().unwrap())
                .args(["--exact", "tests::descendant_fixture", "--nocapture"])
                .stdin(Stdio::null())
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .spawn()
                .unwrap();
            // The fixture parent reaps normally; tests deliberately exit it before its child.
            std::thread::spawn(move || descendant.wait().unwrap());
            let deadline = Instant::now() + Duration::from_secs(5);
            while !Path::new(&root).join("descendant-pid").exists() {
                assert!(Instant::now() < deadline);
                std::thread::sleep(Duration::from_millis(10));
            }
        }
        let exit_root = root.clone();
        std::thread::spawn(move || {
            loop {
                if Path::new(&exit_root).join("exit-parent").exists() {
                    std::process::exit(23)
                }
                std::thread::sleep(Duration::from_millis(10));
            }
        });
        serve_fixture(listener, &root);
    }

    fn serve_fixture(listener: TcpListener, root: &str) {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        listener.set_nonblocking(true).unwrap();
        tokio::runtime::Runtime::new().unwrap().block_on(async {
            let listener = tokio::net::TcpListener::from_std(listener).unwrap();
            let mut term = tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate()).unwrap();
            loop {
                tokio::select! {
                    _ = term.recv() => {
                        fs::write(Path::new(root).join("parent-drained"), "").unwrap();
                        return;
                    }
                    accepted = listener.accept() => {
                        let (mut stream, _) = accepted.unwrap();
                        let mut request = [0; 1024];
                        assert!(stream.read(&mut request).await.unwrap() > 0);
                        stream.write_all(b"HTTP/1.0 200 OK\r\nContent-Length: 0\r\n\r\n").await.unwrap();
                    }
                }
            }
        });
    }

    #[test]
    fn descendant_fixture() {
        let Ok(root) = std::env::var("UX_FIXTURE_ROOT") else {
            return;
        };
        let runtime = tokio::runtime::Runtime::new().unwrap();
        runtime.block_on(async {
        let mut term =
            tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate()).unwrap();
        let mut int =
            tokio::signal::unix::signal(tokio::signal::unix::SignalKind::interrupt()).unwrap();
        fs::write(
            Path::new(&root).join("descendant-pid"),
            std::process::id().to_string(),
        )
        .unwrap();
        loop {
            tokio::select! {
                _ = term.recv() => fs::write(Path::new(&root).join("descendant-term"), "").unwrap(),
                _ = int.recv() => {},
            }
        }
    });
    }

    #[test]
    fn disposable_initializes_reaches_http_and_reaps_before_cleanup() {
        exercise(Scenario::Identity);
    }

    #[test]
    fn graceful_parent_exit_does_not_release_resistant_descendant() {
        exercise(Scenario::Resistant);
    }

    #[test]
    fn unexpected_parent_exit_contains_descendant_and_preserves_status() {
        exercise(Scenario::Unexpected);
    }

    #[test]
    fn further_interrupt_forces_early_but_preserves_initiating_term_status() {
        exercise(Scenario::EarlyForce);
    }

    #[test]
    fn runtime_io_failure_is_reported_after_owned_teardown() {
        let root = fixture_root();
        let started = Instant::now();
        let output = Command::new(env!("CARGO_BIN_EXE_ux-sandbox"))
            .current_dir(root.path())
            .env("UX_FIXTURE_ROOT", root.path())
            .env("UX_FIXTURE_EXE", std::env::current_exe().unwrap())
            .env("UX_RUNTIME_IO_ERROR", "1")
            .env(
                "PATH",
                format!(
                    "{}:{}",
                    root.path().join("bin").display(),
                    std::env::var("PATH").unwrap()
                ),
            )
            .output()
            .unwrap();
        assert_eq!(output.status.code(), Some(1));
        let stderr = String::from_utf8_lossy(&output.stderr);
        assert!(stderr.contains("reading runtime-info"), "{stderr}");
        assert!(stderr.contains("runtime.json"), "{stderr}");
        assert!(!stderr.contains("exceeded 30 seconds"));
        assert!(started.elapsed() < Duration::from_secs(10));
        let storage = fs::read_to_string(root.path().join("storage")).unwrap();
        let pid: u32 = fs::read_to_string(root.path().join("app-pid"))
            .unwrap()
            .parse()
            .unwrap();
        assert!(processkit::process_info(pid).unwrap().is_none());
        assert!(!Path::new(&storage).exists());
    }

    #[test]
    fn initiating_interrupt_is_preserved() {
        exercise(Scenario::Interrupt);
    }

    #[test]
    fn first_interrupt_during_unexpected_exit_drain_keeps_grace_and_status() {
        exercise(Scenario::ExitThenInterrupt);
    }

    #[derive(Clone, Copy, PartialEq)]
    enum Scenario {
        Identity,
        Resistant,
        Unexpected,
        EarlyForce,
        Interrupt,
        ExitThenInterrupt,
    }

    impl Scenario {
        fn exit_code(self) -> i32 {
            match self {
                Self::Unexpected => 23,
                Self::Interrupt | Self::ExitThenInterrupt => 130,
                _ => 143,
            }
        }
    }

    fn exercise(scenario: Scenario) {
        let descendant = matches!(
            scenario,
            Scenario::Resistant
                | Scenario::Unexpected
                | Scenario::EarlyForce
                | Scenario::ExitThenInterrupt
        );
        let unexpected = matches!(scenario, Scenario::Unexpected | Scenario::ExitThenInterrupt);
        let expected = scenario.exit_code();
        let root = fixture_root();
        let mut child = Command::new(env!("CARGO_BIN_EXE_ux-sandbox"))
            .current_dir(root.path())
            .env("UX_FIXTURE_ROOT", root.path())
            .env("UX_FIXTURE_EXE", std::env::current_exe().unwrap())
            .env(
                "PATH",
                format!(
                    "{}:{}",
                    root.path().join("bin").display(),
                    std::env::var("PATH").unwrap()
                ),
            )
            .env("JAUNDER_TRUSTED_PROXIES", "invalid-inherited-config")
            .envs(descendant.then_some(("UX_DESCENDANT", "1")))
            .envs((scenario == Scenario::Identity).then_some(("UX_INVALID_RUNTIME", "1")))
            .stdout(Stdio::piped())
            .stderr(Stdio::inherit())
            .spawn()
            .unwrap();
        let stdout = child.stdout.take().unwrap();
        let (tx, rx) = std::sync::mpsc::channel();
        let reader = std::thread::spawn(move || {
            for line in BufReader::new(stdout).lines() {
                let line = line.unwrap();
                if line.starts_with("Sandbox URL: http://127.0.0.1:") {
                    tx.send(line).unwrap();
                }
            }
        });
        if scenario == Scenario::Identity {
            repair_runtime(root.path(), &rx);
        }
        let ready = rx.recv_timeout(Duration::from_secs(15));
        if ready.is_err() {
            let _ = child.kill();
        }
        assert!(
            ready.is_ok(),
            "helper never reported HTTP readiness: {ready:?}"
        );
        let storage = fs::read_to_string(root.path().join("storage")).unwrap();
        assert!(Path::new(&storage).exists());
        assert_eq!(
            fs::read_to_string(root.path().join("build-calls")).unwrap(),
            "just\nbuild-csr debug\ncargo\nbuild --locked -p jaunder\ncargo\nbuild --locked -p test-support\n"
        );
        let started = Instant::now();
        if unexpected {
            fs::write(root.path().join("exit-parent"), "").unwrap();
        } else {
            send_signal(child.id(), if expected == 130 { "-INT" } else { "-TERM" });
        }
        if descendant {
            let deadline = Instant::now() + Duration::from_secs(5);
            while !root.path().join("descendant-term").exists() {
                assert!(
                    Instant::now() < deadline,
                    "descendant never received graceful stop"
                );
                std::thread::sleep(Duration::from_millis(10));
            }
            assert!(
                Path::new(&storage).exists(),
                "storage removed with descendant alive"
            );
            if matches!(scenario, Scenario::EarlyForce | Scenario::ExitThenInterrupt) {
                send_signal(child.id(), "-INT");
            } else if !unexpected {
                send_signal(child.id(), "-TERM");
            }
        }
        let status = wait_child(&mut child);
        reader.join().unwrap();
        assert_eq!(status.code(), Some(expected));
        if descendant {
            if scenario == Scenario::EarlyForce {
                assert!(started.elapsed() < Duration::from_secs(5));
            } else {
                assert!(started.elapsed() >= Duration::from_secs(10));
            }
            let pid = fs::read_to_string(root.path().join("descendant-pid")).unwrap();
            assert!(
                processkit::process_info(pid.parse().unwrap())
                    .unwrap()
                    .is_none(),
                "descendant not reaped"
            );
        }
        assert_cleaned(root.path(), &storage);
    }

    fn wait_child(child: &mut std::process::Child) -> std::process::ExitStatus {
        let deadline = Instant::now() + Duration::from_secs(16);
        loop {
            if let Some(status) = child.try_wait().unwrap() {
                return status;
            }
            if Instant::now() >= deadline {
                child.kill().unwrap();
                child.wait().unwrap();
                panic!("helper failed to drain")
            }
            std::thread::sleep(Duration::from_millis(20));
        }
    }

    fn assert_cleaned(root: &Path, storage: &str) {
        if !root.join("exit-parent").exists() {
            assert!(
                root.join("parent-drained").exists(),
                "fixture parent did not exit gracefully"
            );
        }
        let pid = fs::read_to_string(root.join("app-pid")).unwrap();
        assert!(
            processkit::process_info(pid.parse().unwrap())
                .unwrap()
                .is_none(),
            "app not reaped"
        );
        assert!(
            !Path::new(storage).exists(),
            "disposable storage not removed"
        );
    }

    fn repair_runtime(root: &Path, rx: &std::sync::mpsc::Receiver<String>) {
        let deadline = Instant::now() + Duration::from_secs(10);
        while !root.join("valid-runtime").exists() {
            assert!(Instant::now() < deadline);
            std::thread::sleep(Duration::from_millis(10));
        }
        let storage = fs::read_to_string(root.join("storage")).unwrap();
        let valid: serde_json::Value =
            serde_json::from_slice(&fs::read(root.join("valid-runtime")).unwrap()).unwrap();
        for (key, invalid) in [
            ("pid", serde_json::json!(1)),
            ("start_time", serde_json::json!(0)),
            ("ip", serde_json::json!("0.0.0.0")),
            ("port", serde_json::json!(0)),
        ] {
            let mut value = valid.clone();
            value[key] = invalid;
            fs::write(
                Path::new(&storage).join("runtime.json"),
                serde_json::to_vec(&value).unwrap(),
            )
            .unwrap();
            assert!(
                rx.recv_timeout(Duration::from_millis(250)).is_err(),
                "invalid runtime accepted: {key}"
            );
        }
        fs::write(
            Path::new(&storage).join("runtime.json"),
            serde_json::to_vec(&valid).unwrap(),
        )
        .unwrap();
    }

    fn send_signal(pid: u32, signal: &str) {
        assert!(
            Command::new("kill")
                .args([signal, &pid.to_string()])
                .status()
                .unwrap()
                .success()
        );
    }
}
