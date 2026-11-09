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
        fs::create_dir_all(root.path().join("artifacts/bin")).unwrap();
        fs::create_dir(root.path().join("bin")).unwrap();
        for name in ["cargo", "just"] {
            executable(
                &root.path().join("bin").join(name),
                "#!/bin/sh\nprintf '%s\\n' unexpected >> build-calls\nexit 97\n",
            );
        }
        executable(&root.path().join("bin/git"), "#!/bin/sh\nexit 0\n");
        executable(
            &root.path().join("bin/nix"),
            concat!(
                "#!/bin/sh\n",
                "if [ \"$1\" = eval ]; then printf '%s' \"${UX_OMITTED_INPUTS:-[]}\"; exit 0; fi\n",
                "printf '%s\\0' \"$@\" > nix-argv\n",
                "[ \"$1 $2 $3 $4\" = 'build --no-update-lock-file --no-write-lock-file --out-link' ] || exit 98\n",
                "[ \"$6\" = '.#sandboxArtifacts' ] || exit 99\n",
                "export UX_SELECTED_LINK=\"$5\"\n",
                "printf '%s' \"$5\" > selected-root\n",
                "exec \"$UX_FIXTURE_EXE\" --exact tests::build_fixture --nocapture\n",
            ),
        );
        executable(
            &root.path().join("artifacts/bin/jaunder"),
            "#!/bin/sh\nexport UX_FIXTURE_MODE=\"$1\"\nprintf '%s\\0' \"$@\" > \"$UX_FIXTURE_ROOT/last-argv\"\nexec \"$UX_FIXTURE_EXE\" --exact tests::app_fixture --nocapture\n",
        );
        executable(
            &root.path().join("artifacts/bin/test-support"),
            "#!/bin/sh\nprintf '%s\\0' \"$@\" > \"$UX_FIXTURE_ROOT/seed-argv\"\nexec \"$UX_FIXTURE_EXE\" --exact tests::seed_fixture --nocapture\n",
        );
        root
    }

    #[test]
    fn build_fixture() {
        let Ok(root) = std::env::var("UX_FIXTURE_ROOT") else {
            return;
        };
        let link = std::env::var("UX_SELECTED_LINK").unwrap();
        std::os::unix::fs::symlink(Path::new(&root).join("artifacts"), &link).unwrap();
        fs::write(Path::new(&root).join("selection-started"), "").unwrap();
        if std::env::var_os("UX_SELECTION_HOLD").is_some() {
            tokio::runtime::Runtime::new().unwrap().block_on(async {
                let mut term =
                    tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())
                        .unwrap();
                fs::write(Path::new(&root).join("selection-listening"), "").unwrap();
                term.recv().await;
            });
        }
        if let Ok(code) = std::env::var("UX_SELECTION_EXIT") {
            std::process::exit(code.parse().unwrap());
        }
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
        let selected = fs::read_to_string(Path::new(&root).join("selected-root")).unwrap();
        assert!(
            Path::new(&selected).exists(),
            "root must survive app execution"
        );
        if mode == "init" {
            // Match the real init contract: it exclusively creates new storage.
            fs::create_dir(&storage).unwrap();
            fs::write(Path::new(&storage).join("initialized"), "").unwrap();
            if std::env::var_os("UX_PROFILE_FAILURE").is_some() {
                fs::create_dir(Path::new(&storage).join(".sandbox.json")).unwrap();
            }
            if let Ok(code) = std::env::var("UX_INIT_EXIT") {
                std::process::exit(code.parse().unwrap());
            }
            return;
        }
        if mode != "serve" {
            operation_fixture(Path::new(&root), Path::new(&storage));
            return;
        }
        assert!(Path::new(&storage).join("initialized").is_file());
        let runtime_guard = fs::OpenOptions::new()
            .create(true)
            .truncate(false)
            .read(true)
            .write(true)
            .open(Path::new(&storage).join("runtime.lock"))
            .unwrap();
        runtime_guard.try_lock().unwrap();
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

    fn operation_fixture(root: &Path, storage: &Path) {
        let name = storage.file_name().unwrap().to_str().unwrap();
        let ownership = root
            .join(".xtask/sandboxes/.locks")
            .join(format!("{name}.workspace.lock"));
        let workspace = fs::OpenOptions::new()
            .read(true)
            .write(true)
            .open(ownership)
            .unwrap();
        assert!(matches!(
            workspace.try_lock(),
            Err(fs::TryLockError::WouldBlock)
        ));
        let runtime = fs::OpenOptions::new()
            .read(true)
            .write(true)
            .open(storage.join("runtime.lock"))
            .unwrap();
        assert!(matches!(
            runtime.try_lock(),
            Err(fs::TryLockError::WouldBlock)
        ));
        fs::write(
            root.join(format!("operation-{name}")),
            storage.display().to_string(),
        )
        .unwrap();
        while fs::read_to_string(root.join("hold-command")).unwrap_or_default() == name
            && !root.join("release-command").exists()
        {
            std::thread::sleep(Duration::from_millis(10));
        }
        if let Ok(code) = std::env::var("UX_COMMAND_EXIT") {
            std::process::exit(code.parse().unwrap());
        }
    }

    #[test]
    fn seed_fixture() {
        let Ok(root) = std::env::var("UX_FIXTURE_ROOT") else {
            return;
        };
        let args: Vec<_> = fs::read(Path::new(&root).join("seed-argv"))
            .unwrap()
            .split(|b| *b == 0)
            .filter(|arg| !arg.is_empty())
            .map(|arg| String::from_utf8(arg.to_vec()).unwrap())
            .collect();
        let storage = std::env::var("JAUNDER_STORAGE_PATH").unwrap();
        assert_eq!(args[0], "seed-sandbox-profile");
        assert_eq!(args[1], "--db");
        assert_eq!(args[2], format!("sqlite:{storage}/jaunder.db"));
        assert_eq!(args[3], "--profile");
        assert_eq!(args[5], "--storage-path");
        assert_eq!(args[6], storage);
        assert_eq!(
            std::env::var("JAUNDER_TEST_EXECUTABLE").unwrap(),
            format!("{root}/artifacts/bin/jaunder")
        );
        assert!(Path::new(&storage).join("initialized").is_file());
        fs::write(Path::new(&storage).join("seeded"), &args[4]).unwrap();
        println!(r#"{{"posts":[],"media":[]}}"#);
        if std::env::var_os("UX_SEED_DESCENDANT").is_some() {
            let mut child = Command::new(std::env::current_exe().unwrap())
                .args(["--exact", "tests::descendant_fixture", "--nocapture"])
                .stdin(Stdio::null())
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .spawn()
                .unwrap();
            std::thread::spawn(move || child.wait().unwrap());
            wait_for(&Path::new(&root).join("descendant-pid"));
        }
        if let Ok(code) = std::env::var("UX_SEED_EXIT") {
            std::process::exit(code.parse().unwrap());
        }
    }

    #[test]
    fn profile_output_is_concise_on_create_resume_and_empty_sessions() {
        let root = fixture_root();
        for (args, hint) in [
            (vec!["standard", "--profile", "standard"], true),
            (vec!["standard"], true),
            (vec!["demo", "--profile", "demo"], true),
            (vec!["demo"], true),
            (vec!["empty"], false),
        ] {
            let mut child = named_child(root.path(), &args);
            let stdout = child.stdout.take().unwrap();
            let (tx, rx) = std::sync::mpsc::channel();
            let reader = std::thread::spawn(move || {
                let mut text = String::new();
                for line in BufReader::new(stdout).lines() {
                    let line = line.unwrap();
                    if line.starts_with("Sandbox URL:") {
                        tx.send(()).unwrap();
                    }
                    text.push_str(&line);
                    text.push('\n');
                }
                text
            });
            rx.recv_timeout(Duration::from_secs(10)).unwrap();
            send_signal(child.id(), "-TERM");
            assert_eq!(wait_child(&mut child).code(), Some(143));
            let text = reader.join().unwrap();
            assert_eq!(
                text.contains("Sandbox credentials: user / jaunder-dev; operator / jaunder-dev"),
                hint,
                "{text}"
            );
            assert!(
                !text.contains(r#"{"posts":[],"media":[]}"#),
                "seed manifest leaked: {text}"
            );
        }
    }

    fn wait_for(path: &Path) {
        let deadline = Instant::now() + Duration::from_secs(5);
        while !path.exists() {
            assert!(Instant::now() < deadline, "missing {}", path.display());
            std::thread::sleep(Duration::from_millis(10));
        }
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

    fn helper(root: &Path) -> Command {
        let mut command = Command::new(env!("CARGO_BIN_EXE_ux-sandbox"));
        command
            .current_dir(root)
            .env("UX_FIXTURE_ROOT", root)
            .env("UX_FIXTURE_EXE", std::env::current_exe().unwrap())
            .env(
                "PATH",
                format!(
                    "{}:{}",
                    root.join("bin").display(),
                    std::env::var("PATH").unwrap()
                ),
            );
        command
    }

    #[test]
    fn failed_selection_never_uses_existing_artifacts_or_initializes_storage() {
        let root = fixture_root();
        fs::create_dir(root.path().join(".xtask")).unwrap();
        let previous = root.path().join(".xtask/previous-artifacts");
        std::os::unix::fs::symlink(root.path().join("artifacts"), &previous).unwrap();
        let old = root.path().join(".xtask/sandboxes/.writing.reset-old");
        fs::create_dir_all(&old).unwrap();
        fs::write(old.join("sentinel"), b"unrecovered generation").unwrap();
        let output = helper(root.path())
            .arg("writing")
            .env("UX_SELECTION_EXIT", "37")
            .env("UX_INIT_EXIT", "93")
            .output()
            .unwrap();
        assert_eq!(output.status.code(), Some(37), "{output:?}");
        assert!(!root.path().join(".xtask/sandboxes/writing").exists());
        assert!(!root.path().join("last-argv").exists());
        assert!(!root.path().join("build-calls").exists());
        assert!(previous.exists(), "foreign retained root must be untouched");
        assert_eq!(
            fs::read(old.join("sentinel")).unwrap(),
            b"unrecovered generation"
        );
        assert_selection_retired(root.path());
    }

    #[test]
    fn real_git_nix_admission_refuses_omitted_source_as_data_before_selection() {
        let root = fixture_root();
        for program in ["git", "nix"] {
            fs::remove_file(root.path().join("bin").join(program)).unwrap();
        }
        fs::write(
            root.path().join("flake.nix"),
            r#"{
          outputs = { self }: { packages = builtins.listToAttrs (map (name: {
            inherit name;
            value.sandboxArtifacts.sourceAdmits = path: kind:
              builtins.match "server/.*" path != null;
          }) [ "x86_64-linux" "aarch64-linux" "aarch64-darwin" ]); };
        }"#,
        )
        .unwrap();
        let git = |args: &[&str]| {
            let mut command = Command::new("git");
            command.current_dir(root.path()).args(args);
            for name in [
                "GIT_DIR",
                "GIT_WORK_TREE",
                "GIT_INDEX_FILE",
                "GIT_OBJECT_DIRECTORY",
                "GIT_COMMON_DIR",
                "GIT_NAMESPACE",
            ] {
                command.env_remove(name);
            }
            let output = command.output().unwrap();
            assert!(output.status.success(), "{output:?}");
            output.stdout
        };
        git(&["init", "--quiet"]);
        git(&["add", "flake.nix"]);
        let relative = "server/src/space \"quote\" ${not-code}.rs";
        fs::create_dir_all(root.path().join("server/src")).unwrap();
        fs::write(root.path().join(relative), "not silently omitted").unwrap();
        // This evaluation needs neither the host daemon/store nor user Nix config.
        let home = root.path().join("home");
        fs::create_dir(&home).unwrap();
        let output = helper(root.path())
            .arg("writing")
            .env("HOME", &home)
            .env("NIX_CONF_DIR", &home)
            .env("NIX_USER_CONF_FILES", "/dev/null")
            .env("NIX_CONFIG", "experimental-features = nix-command flakes")
            .env(
                "NIX_REMOTE",
                format!("local?root={}", root.path().join("nix-store").display()),
            )
            .output()
            .unwrap();
        let tracked = git(&["ls-files", relative]);
        let selected = root.path().join("selected-root").exists();
        let started = root.path().join("last-argv").exists();
        let workspace = root.path().join(".xtask/sandboxes/writing").exists();
        // Nix makes its private source directory read-only. Restore only directory
        // owner-write for destruction; never change real store or file permissions.
        for directory in ["nix-store", "nix-store/nix", "nix-store/nix/store"] {
            assert!(
                fs::symlink_metadata(root.path().join(directory))
                    .unwrap()
                    .is_dir()
            );
        }
        for entry in fs::read_dir(root.path().join("nix-store/nix/store")).unwrap() {
            let entry = entry.unwrap();
            let metadata = fs::symlink_metadata(entry.path()).unwrap();
            if entry.file_name().to_string_lossy().ends_with("-source") {
                assert!(metadata.is_dir(), "private source must not be a symlink");
                fs::set_permissions(
                    entry.path(),
                    fs::Permissions::from_mode(metadata.permissions().mode() | 0o200),
                )
                .unwrap();
            }
        }
        let fixture = root.path().to_path_buf();
        root.close().unwrap();
        assert!(!fixture.exists(), "private Nix fixture was not retired");
        assert_eq!(output.status.code(), Some(1));
        let stderr = String::from_utf8_lossy(&output.stderr);
        assert!(stderr.contains(relative), "{stderr}");
        assert!(stderr.contains("git add -N"), "{stderr}");
        assert!(output.stdout.is_empty(), "admission JSON leaked to stdout");
        assert_eq!(tracked, b"");
        assert!(!selected);
        assert!(!started);
        assert!(!workspace);
    }

    fn assert_selection_retired(root: &Path) {
        let selected = fs::read_to_string(root.join("selected-root")).unwrap();
        assert!(
            !Path::new(&selected).parent().unwrap().exists(),
            "root retained after teardown"
        );
        assert!(!root.join(".xtask/host-artifacts.lock").exists());
    }

    #[test]
    fn failed_selection_status_returns_unchanged_in_all_three_modes_and_releases_locks() {
        for args in [vec![], vec!["writing"], vec!["writing", "--", "backup"]] {
            let root = fixture_root();
            let output = helper(root.path())
                .args(&args)
                .env("UX_SELECTION_EXIT", "37")
                .output()
                .unwrap();
            assert_eq!(output.status.code(), Some(37), "{args:?}: {output:?}");
            assert_selection_retired(root.path());
            let locks = root.path().join(".xtask/sandboxes/.locks");
            assert_control_locks_released(&locks);
            assert!(!root.path().join(".xtask/sandboxes/writing").exists());
            assert!(!root.path().join("app-pid").exists());
        }
    }

    fn assert_control_locks_released(locks: &Path) {
        if !locks.exists() {
            return;
        }
        for path in fs::read_dir(locks).unwrap() {
            let file = fs::OpenOptions::new()
                .read(true)
                .write(true)
                .open(path.unwrap().path())
                .unwrap();
            file.try_lock()
                .expect("failed preparation must release workspace and server locks");
        }
    }

    #[test]
    fn interrupted_selection_discards_even_a_published_link_before_storage_work() {
        let root = fixture_root();
        let mut child = helper(root.path())
            .arg("writing")
            .env("UX_SELECTION_HOLD", "1")
            .stdout(Stdio::null())
            .spawn()
            .unwrap();
        wait_for(&root.path().join("selection-listening"));
        let selected = fs::read_to_string(root.path().join("selected-root")).unwrap();
        assert!(Path::new(&selected).exists());
        send_signal(child.id(), "-INT");
        assert_eq!(wait_child(&mut child).code(), Some(130));
        assert_selection_retired(root.path());
        assert_control_locks_released(&root.path().join(".xtask/sandboxes/.locks"));
        assert!(!root.path().join("build-calls").exists());
        assert!(!root.path().join("last-argv").exists());
        assert!(!root.path().join(".xtask/sandboxes/writing").exists());
    }

    #[test]
    fn selected_non_executable_is_rejected_before_initialization() {
        let root = fixture_root();
        let path = root.path().join("artifacts/bin/jaunder");
        fs::set_permissions(&path, fs::Permissions::from_mode(0o644)).unwrap();
        let output = helper(root.path()).output().unwrap();
        assert_eq!(output.status.code(), Some(1));
        let stderr = String::from_utf8_lossy(&output.stderr);
        assert!(stderr.contains("not an executable"), "{stderr}");
        assert!(stderr.contains(&path.display().to_string()), "{stderr}");
        assert_selection_retired(root.path());
        assert!(!root.path().join("storage").exists());
    }

    #[test]
    fn disposable_init_failure_keeps_child_status_and_removes_partial_storage() {
        let root = fixture_root();
        let output = helper(root.path())
            .env("UX_INIT_EXIT", "41")
            .output()
            .unwrap();
        assert_eq!(output.status.code(), Some(41), "{output:?}");
        let stderr = String::from_utf8_lossy(&output.stderr);
        let storage = stderr
            .lines()
            .find_map(|line| line.strip_prefix("Disposable storage: "))
            .unwrap();
        assert!(!Path::new(storage).exists());
        assert!(!Path::new(storage).parent().unwrap().exists());
        assert_selection_retired(root.path());
        assert!(!root.path().join("app-pid").exists());
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
        let selected = fs::read_to_string(root.path().join("selected-root")).unwrap();
        assert!(Path::new(&selected).exists());
        assert!(!root.path().join("build-calls").exists());
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
            signal_during_drain(scenario, root.path(), child.id());
        }
        let status = wait_child(&mut child);
        reader.join().unwrap();
        assert_eq!(status.code(), Some(expected));
        assert_selection_retired(root.path());
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

    fn signal_during_drain(scenario: Scenario, root: &Path, pid: u32) {
        if scenario == Scenario::EarlyForce {
            // Force may legitimately kill the parent; establish its graceful acknowledgement first.
            wait_for(&root.join("parent-drained"));
        }
        if matches!(scenario, Scenario::EarlyForce | Scenario::ExitThenInterrupt) {
            send_signal(pid, "-INT");
        } else if scenario != Scenario::Unexpected {
            send_signal(pid, "-TERM");
        }
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

    #[test]
    fn named_empty_create_and_resume_preserve_legacy_metadata() {
        let root = fixture_root();
        for _ in 0..2 {
            let mut child = named_child(root.path(), &["writing"]);
            let stdout = child.stdout.take().unwrap();
            let (tx, rx) = std::sync::mpsc::channel();
            let reader = std::thread::spawn(move || {
                for line in BufReader::new(stdout).lines() {
                    let line = line.unwrap();
                    if line.starts_with("Sandbox URL:") {
                        tx.send(line).unwrap();
                    }
                }
            });
            let ready = rx.recv_timeout(Duration::from_secs(10));
            if ready.is_err() {
                let _ = child.kill();
                child.wait().unwrap();
            }
            assert!(ready.is_ok(), "named server never became ready: {ready:?}");
            let workspace = root.path().join(".xtask/sandboxes/writing");
            let profile: serde_json::Value =
                serde_json::from_slice(&fs::read(workspace.join(".sandbox.json")).unwrap())
                    .unwrap();
            assert_eq!(profile, serde_json::json!({"version":1,"profile":"empty"}));
            let ready_path = root
                .path()
                .join(".xtask/sandboxes/.locks/writing.server.json");
            let metadata: serde_json::Value =
                serde_json::from_slice(&fs::read(&ready_path).unwrap()).unwrap();
            assert_eq!(metadata["version"], 1);
            assert_eq!(metadata["state"], "ready");
            assert_eq!(metadata["executable_digest"].as_str().unwrap().len(), 64);
            send_signal(child.id(), "-TERM");
            assert_eq!(wait_child(&mut child).code(), Some(143));
            reader.join().unwrap();
            assert!(workspace.join("initialized").is_file());
            assert!(!ready_path.exists());
        }
    }

    #[test]
    fn live_commands_require_matching_generation_and_current_runtime_identity() {
        let root = fixture_root();
        let server = Running::start(root.path(), &["writing"]);
        let output = named_command(root.path(), &["writing", "--", "backup"])
            .env("UX_COMMAND_EXIT", "37")
            .output()
            .unwrap();
        assert_eq!(output.status.code(), Some(37), "{output:?}");
        fs::remove_file(root.path().join("operation-writing")).unwrap();
        let metadata = root
            .path()
            .join(".xtask/sandboxes/.locks/writing.server.json");
        let original_metadata = fs::read(&metadata).unwrap();
        let mut value: serde_json::Value = serde_json::from_slice(&original_metadata).unwrap();
        value["executable_digest"] = serde_json::json!("mismatched");
        fs::write(&metadata, serde_json::to_vec(&value).unwrap()).unwrap();
        let mismatch = named_command(root.path(), &["writing", "--", "backup"])
            .output()
            .unwrap();
        assert!(!mismatch.status.success());
        assert!(String::from_utf8_lossy(&mismatch.stderr).contains("different executable"));
        fs::write(&metadata, original_metadata).unwrap();
        let runtime = root.path().join(".xtask/sandboxes/writing/runtime.json");
        let original_runtime = fs::read(&runtime).unwrap();
        let mut value: serde_json::Value = serde_json::from_slice(&original_runtime).unwrap();
        value["start_time"] = serde_json::json!(0);
        fs::write(&runtime, serde_json::to_vec(&value).unwrap()).unwrap();
        let mismatch = named_command(root.path(), &["writing", "--", "backup"])
            .output()
            .unwrap();
        assert!(!mismatch.status.success());
        assert!(String::from_utf8_lossy(&mismatch.stderr).contains("identity/address mismatch"));
        assert!(!root.path().join("operation-writing").exists());
        fs::write(runtime, original_runtime).unwrap();
        server.stop();
    }

    #[test]
    fn external_runtime_and_starting_server_reject_before_storage_child() {
        let root = fixture_root();
        let workspace = legacy_workspace(root.path(), "writing");
        let runtime = fs::OpenOptions::new()
            .create(true)
            .truncate(false)
            .read(true)
            .write(true)
            .open(workspace.join("runtime.lock"))
            .unwrap();
        runtime.try_lock().unwrap();
        for args in [vec!["writing", "--", "backup"], vec!["writing", "--reset"]] {
            let output = named_command(root.path(), &args).output().unwrap();
            assert!(!output.status.success(), "{args:?}");
            assert!(String::from_utf8_lossy(&output.stderr).contains("external server"));
        }
        runtime.unlock().unwrap();
        let control = root.path().join(".xtask/sandboxes/.locks");
        let lease = fs::OpenOptions::new()
            .read(true)
            .write(true)
            .open(control.join("writing.server.lock"))
            .unwrap();
        lease.try_lock().unwrap();
        let ownership = fs::OpenOptions::new()
            .read(true)
            .write(true)
            .open(control.join("writing.workspace.lock"))
            .unwrap();
        ownership.try_lock().unwrap();
        let started = Instant::now();
        let output = named_command(root.path(), &["writing", "--", "backup"])
            .output()
            .unwrap();
        assert!(!output.status.success());
        assert!(started.elapsed() < Duration::from_secs(3));
        assert!(!root.path().join("last-argv").exists());
    }

    #[test]
    fn admitted_command_pins_generation_through_stopping_and_cleanup_barrier() {
        let root = fixture_root();
        let mut server = Running::start(root.path(), &["writing"]);
        let workspace = root.path().join(".xtask/sandboxes/writing");
        fs::write(workspace.join("generation"), "old").unwrap();
        fs::write(root.path().join("hold-command"), "writing").unwrap();
        let mut command = named_command(root.path(), &["writing", "--", "backup"])
            .env("UX_COMMAND_EXIT", "37")
            .stdout(Stdio::null())
            .stderr(Stdio::inherit())
            .spawn()
            .unwrap();
        wait_for(&root.path().join("operation-writing"));
        send_signal(server.child.id(), "-TERM");
        let metadata = root
            .path()
            .join(".xtask/sandboxes/.locks/writing.server.json");
        let deadline = Instant::now() + Duration::from_secs(5);
        while serde_json::from_slice::<serde_json::Value>(&fs::read(&metadata).unwrap()).unwrap()["state"]
            != "stopping"
        {
            assert!(Instant::now() < deadline);
            std::thread::sleep(Duration::from_millis(10));
        }
        assert!(server.child.try_wait().unwrap().is_none());
        assert_eq!(
            fs::read_to_string(workspace.join("generation")).unwrap(),
            "old"
        );
        let refused = named_command(root.path(), &["writing", "--reset"])
            .output()
            .unwrap();
        assert!(!refused.status.success());
        fs::remove_file(root.path().join("operation-writing")).unwrap();
        let refused = named_command(root.path(), &["writing", "--", "backup"])
            .output()
            .unwrap();
        assert!(!refused.status.success());
        assert!(!root.path().join("operation-writing").exists());
        let other = Running::start(root.path(), &["independent"]);
        assert!(
            named_command(root.path(), &["independent", "--", "backup"])
                .status()
                .unwrap()
                .success()
        );
        other.stop();
        fs::write(root.path().join("release-command"), "").unwrap();
        assert_eq!(wait_child(&mut command).code(), Some(37));
        server.stop();
        assert!(!metadata.exists());
        Running::start(root.path(), &["writing", "--reset"]).stop();
        assert!(!workspace.join("generation").exists());
    }

    #[test]
    fn command_selection_does_not_wait_for_existing_e2e_artifact_exclusion() {
        let root = fixture_root();
        legacy_workspace(root.path(), "writing");
        let lock = fs::OpenOptions::new()
            .create(true)
            .truncate(false)
            .read(true)
            .write(true)
            .open(root.path().join(".xtask/host-artifacts.lock"))
            .unwrap();
        lock.try_lock().unwrap();
        let mut child = named_command(root.path(), &["writing", "--", "backup"])
            .stdout(Stdio::null())
            .spawn()
            .unwrap();
        assert!(wait_child(&mut child).success());
        assert!(root.path().join("operation-writing").exists());
        assert!(!root.path().join("build-calls").exists());
        let probe = fs::File::open(root.path().join(".xtask/host-artifacts.lock")).unwrap();
        assert!(matches!(
            probe.try_lock(),
            Err(fs::TryLockError::WouldBlock)
        ));
        lock.unlock().unwrap();
    }

    struct Running {
        child: std::process::Child,
        reader: Option<std::thread::JoinHandle<()>>,
        stopped: bool,
    }
    impl Running {
        fn start(root: &Path, args: &[&str]) -> Self {
            let mut child = named_child(root, args);
            let stdout = child.stdout.take().unwrap();
            let (tx, rx) = std::sync::mpsc::channel();
            let reader = std::thread::spawn(move || {
                for line in BufReader::new(stdout).lines() {
                    let line = line.unwrap();
                    if line.starts_with("Sandbox URL:") {
                        let _ = tx.send(line);
                    }
                }
            });
            let running = Self {
                child,
                reader: Some(reader),
                stopped: false,
            };
            assert!(
                rx.recv_timeout(Duration::from_secs(10)).is_ok(),
                "named server never ready"
            );
            running
        }
        fn stop(mut self) {
            send_signal(self.child.id(), "-TERM");
            assert_eq!(wait_child(&mut self.child).code(), Some(143));
            self.stopped = true;
            self.reader.take().unwrap().join().unwrap();
        }
    }
    impl Drop for Running {
        fn drop(&mut self) {
            if !self.stopped && self.child.try_wait().unwrap().is_none() {
                send_signal(self.child.id(), "-TERM");
                wait_child(&mut self.child);
            }
            if let Some(reader) = self.reader.take() {
                reader.join().unwrap();
            }
        }
    }

    fn legacy_workspace(root: &Path, name: &str) -> std::path::PathBuf {
        let path = root.join(".xtask/sandboxes").join(name);
        fs::create_dir_all(&path).unwrap();
        fs::write(
            path.join(".sandbox.json"),
            r#"{"version":1,"profile":"empty"}"#,
        )
        .unwrap();
        fs::write(path.join("initialized"), "legacy").unwrap();
        path
    }

    #[test]
    fn stopped_commands_select_the_bundle_and_forward_exact_argv_status() {
        let root = fixture_root();
        legacy_workspace(root.path(), "writing");
        for mode in [
            "site-config",
            "user-create",
            "app-password-create",
            "user-invite",
            "smtp-test",
            "backup",
            "websub",
        ] {
            let arguments = [mode, "space here", "", "--", "$(touch injected); *"];
            let mut args = vec!["writing", "--"];
            args.extend(arguments);
            let output = named_command(root.path(), &args)
                .env("UX_COMMAND_EXIT", "37")
                .output()
                .unwrap();
            assert_eq!(output.status.code(), Some(37), "{mode}: {output:?}");
            let expected: Vec<u8> = arguments
                .iter()
                .flat_map(|s| s.bytes().chain([0]))
                .collect();
            assert_eq!(fs::read(root.path().join("last-argv")).unwrap(), expected);
        }
        assert!(!root.path().join("build-calls").exists());
        assert_selection_retired(root.path());
        assert!(!root.path().join("target/site").exists());
        assert!(!root.path().join("injected").exists());
    }

    #[test]
    fn lifecycle_and_storage_selectors_are_rejected_before_any_build_or_child() {
        let root = fixture_root();
        for args in [
            vec!["writing", "--", "serve"],
            vec!["writing", "--", "init"],
            vec!["writing", "--", "restore"],
            vec!["writing", "--", "create-pg-db"],
            vec!["writing", "--", "backup", "--db=x"],
            vec![
                "writing",
                "--",
                "site-config",
                "set",
                "--storage-path=/elsewhere",
            ],
            vec!["writing", "--", "backup", "--", "--db", "x"],
            vec!["writing", "--profile", "demo", "--", "backup"],
            vec!["writing", "--reset", "--", "backup"],
        ] {
            let result = named_command(root.path(), &args).output().unwrap();
            assert_eq!(result.status.code(), Some(2), "{args:?}: {result:?}");
        }
        assert!(!root.path().join("build-calls").exists());
        assert!(!root.path().join("last-argv").exists());
    }

    #[test]
    fn disposable_profiles_seed_then_remove_only_after_owned_session_finishes() {
        for profile in ["standard", "demo"] {
            let root = fixture_root();
            let session = Running::start(root.path(), &["--profile", profile]);
            let path =
                std::path::PathBuf::from(fs::read_to_string(root.path().join("storage")).unwrap());
            assert_eq!(fs::read_to_string(path.join("seeded")).unwrap(), profile);
            session.stop();
            assert!(!path.parent().unwrap().exists());
        }
    }

    #[test]
    fn profiles_create_resume_and_reset_without_reseeding_existing_state() {
        let root = fixture_root();
        Running::start(root.path(), &["writing", "--profile", "standard"]).stop();
        let path = root.path().join(".xtask/sandboxes/writing");
        assert_eq!(fs::read_to_string(path.join("seeded")).unwrap(), "standard");
        fs::write(path.join("seeded"), "user-modified").unwrap();
        Running::start(root.path(), &["writing"]).stop();
        assert_eq!(
            fs::read_to_string(path.join("seeded")).unwrap(),
            "user-modified"
        );
        let invalid = named_command(root.path(), &["writing", "--profile", "demo"])
            .output()
            .unwrap();
        assert!(!invalid.status.success());
        Running::start(root.path(), &["writing", "--reset"]).stop();
        assert_eq!(fs::read_to_string(path.join("seeded")).unwrap(), "standard");
        Running::start(root.path(), &["writing", "--reset", "--profile", "demo"]).stop();
        assert_eq!(fs::read_to_string(path.join("seeded")).unwrap(), "demo");
        let profile: serde_json::Value =
            serde_json::from_slice(&fs::read(path.join(".sandbox.json")).unwrap()).unwrap();
        assert_eq!(profile, serde_json::json!({"version":1,"profile":"demo"}));
    }

    #[test]
    fn init_and_profile_metadata_failures_leave_existing_generation_intact() {
        for (variable, code) in [("UX_INIT_EXIT", 43), ("UX_PROFILE_FAILURE", 1)] {
            let root = fixture_root();
            let old = legacy_workspace(root.path(), "writing");
            fs::write(old.join("must-survive"), "old").unwrap();
            let output = named_command(root.path(), &["writing", "--reset", "--profile", "demo"])
                .env(variable, code.to_string())
                .output()
                .unwrap();
            assert_eq!(output.status.code(), Some(code), "{output:?}");
            assert_eq!(fs::read_to_string(old.join("must-survive")).unwrap(), "old");
            assert!(
                !root
                    .path()
                    .join(".xtask/sandboxes/.writing.reset-new")
                    .exists()
            );
            assert!(
                !root
                    .path()
                    .join(".xtask/sandboxes/.writing.reset-old")
                    .exists()
            );
        }
    }

    #[test]
    fn failed_seed_contains_descendant_before_discarding_stage_and_preserves_old() {
        let root = fixture_root();
        let old = legacy_workspace(root.path(), "writing");
        fs::write(old.join("must-survive"), "old").unwrap();
        let started = Instant::now();
        let mut child = named_command(root.path(), &["writing", "--reset", "--profile", "demo"])
            .env("UX_SEED_EXIT", "41")
            .env("UX_SEED_DESCENDANT", "1")
            .stdout(Stdio::null())
            .stderr(Stdio::inherit())
            .spawn()
            .unwrap();
        wait_for(&root.path().join("descendant-term"));
        assert!(
            root.path()
                .join(".xtask/sandboxes/.writing.reset-new")
                .is_dir()
        );
        assert!(
            child.try_wait().unwrap().is_none(),
            "owner exited with resistant seeder descendant alive"
        );
        assert_eq!(wait_child(&mut child).code(), Some(41));
        assert!(started.elapsed() >= Duration::from_secs(10));
        assert_eq!(fs::read_to_string(old.join("must-survive")).unwrap(), "old");
        assert!(
            !root
                .path()
                .join(".xtask/sandboxes/.writing.reset-new")
                .exists()
        );
        let pid: u32 = fs::read_to_string(root.path().join("descendant-pid"))
            .unwrap()
            .parse()
            .unwrap();
        assert!(processkit::process_info(pid).unwrap().is_none());
        assert!(!old.join("seeded").exists());
    }

    fn named_command(root: &Path, args: &[&str]) -> Command {
        let mut command = Command::new(env!("CARGO_BIN_EXE_ux-sandbox"));
        command
            .args(args)
            .current_dir(root)
            .env("UX_FIXTURE_ROOT", root)
            .env("UX_FIXTURE_EXE", std::env::current_exe().unwrap())
            .env(
                "PATH",
                format!(
                    "{}:{}",
                    root.join("bin").display(),
                    std::env::var("PATH").unwrap()
                ),
            );
        command
    }

    fn named_child(root: &Path, args: &[&str]) -> std::process::Child {
        named_command(root, args)
            .stdout(Stdio::piped())
            .stderr(Stdio::inherit())
            .spawn()
            .unwrap()
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
