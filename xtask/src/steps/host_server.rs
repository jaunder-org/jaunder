//! Immutable host-native artifact selection and server-session lifecycle.

use std::ffi::OsString;
use std::fs::{self, File, OpenOptions};
use std::path::{Path, PathBuf};
use std::thread::sleep;
use std::time::{Duration, Instant};

use anyhow::Context;
use processkit::Command;
use xshell::{Shell, cmd};

use crate::git;
use crate::result::{CommandResult, StepResult};
use crate::steps::process::Process;

const RUNTIME_FILE_TIMEOUT: Duration = Duration::from_secs(15);
const READINESS_POLL_INTERVAL: Duration = Duration::from_millis(100);
const HTTP_READINESS_ATTEMPTS: usize = 30;
const HTTP_READINESS_INTERVAL: Duration = Duration::from_millis(500);
const PROCESS_SHUTDOWN_TIMEOUT: Duration = Duration::from_secs(10);

/// Current host-native artifacts prepared as one compatible set.
#[derive(Debug)]
pub struct HostArtifacts {
    pub root: PathBuf,
    pub jaunder: PathBuf,
    pub test_support: PathBuf,
    // Retained by the caller until every consumer and its teardown have finished.
    pub(super) _root: tempfile::TempDir,
}

impl HostArtifacts {
    pub(super) fn prepare_interruptible(
        sh: &Shell,
        result: &mut CommandResult,
        release_csr: bool,
        signals: &mut super::host_signals::ServerSignals,
    ) -> Option<Self> {
        let started = Instant::now();
        if let Some(code) = signals.take_pending() {
            result.exit_override = Some(code);
            return None;
        }
        let selected = (|| -> anyhow::Result<Self> {
            anyhow::ensure!(
                sh.var_os("JAUNDER_CSR_BUNDLE_DIR").is_none(),
                "e2e-local selects embedded Nix artifacts; unset JAUNDER_CSR_BUNDLE_DIR before running e2e-local"
            );
            let root = PathBuf::from(git::toplevel(&sh.current_dir())?).canonicalize()?;
            sh.change_dir(&root);
            let attr = if release_csr {
                "sandboxArtifactsReleaseCsr"
            } else {
                "sandboxArtifacts"
            };
            crate::host_tooling::admit_untracked_inputs(&root, &format!(".#{attr}.sourceAdmits"))?;
            let gcroots = root.join(".xtask/gcroots");
            fs::create_dir_all(&gcroots)?;
            let retained = tempfile::Builder::new()
                .prefix("e2e-local-")
                .tempdir_in(gcroots)?;
            let link = retained.path().join("selected");
            let installable = format!(".#{attr}");
            // Reuse the foreground consumer's interruption/teardown path. A new
            // owned link means neither failure nor cancellation can select stale output.
            super::e2e_local::run_consumer(
                cmd!(
                    sh,
                    "nix build --no-update-lock-file --no-write-lock-file --out-link {link} {installable}"
                ),
                result,
                signals,
            )?;
            let jaunder = selected_executable(&link.join("bin/jaunder"))?;
            let test_support = selected_executable(&link.join("bin/test-support"))?;
            eprintln!(
                "Selected {} and {}; retained at {}",
                jaunder.display(),
                test_support.display(),
                link.display()
            );
            Ok(Self {
                root,
                jaunder,
                test_support,
                _root: retained,
            })
        })();
        match selected {
            Ok(artifacts) => {
                result.push(StepResult::ok("e2e-local-artifacts").with_duration(started.elapsed()));
                Some(artifacts)
            }
            Err(error) => {
                result.push(
                    StepResult::fail("e2e-local-artifacts")
                        .detail(format!("{error:#}"))
                        .with_duration(started.elapsed()),
                );
                None
            }
        }
    }
}

fn selected_executable(path: &Path) -> anyhow::Result<PathBuf> {
    use std::os::unix::fs::PermissionsExt;
    // Nix bin entries are immutable symlinks, unlike user workspace paths.
    let path = path
        .canonicalize()
        .with_context(|| format!("selecting {}", path.display()))?;
    let metadata = fs::metadata(&path)?;
    anyhow::ensure!(
        metadata.is_file() && metadata.permissions().mode() & 0o111 != 0,
        "not an executable: {}",
        path.display()
    );
    Ok(path)
}

/// Server inputs owned for the complete lifetime of one host session.
pub struct ServerSessionConfig<'a> {
    pub shell: &'a Shell,
    pub jaunder: PathBuf,
    pub storage: PathBuf,
    pub runtime_file: PathBuf,
    pub database_url: String,
    pub stderr: File,
    pub extra_env: Vec<(OsString, OsString)>,
}

impl ServerSessionConfig<'_> {
    fn runtime_file(&self) -> &Path {
        &self.runtime_file
    }
}

/// A failure to make a spawned server ready, optionally retaining it for
/// caller-owned finalization of its stderr capture.
pub struct HostServerStartError {
    phase: ServerStartPhase,
    error: anyhow::Error,
    session: Option<Box<HostServerSession>>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ServerStartPhase {
    Spawn,
    RuntimeFile,
    Http,
    Interrupted,
}

impl HostServerStartError {
    pub fn phase(&self) -> ServerStartPhase {
        self.phase
    }

    pub fn error(&self) -> &anyhow::Error {
        &self.error
    }

    pub fn into_session(self) -> Option<HostServerSession> {
        self.session.map(|session| *session)
    }
}

/// A ready server with runtime-discovered URL and bounded process ownership.
pub struct HostServerSession {
    process: Process,
    pub base_url: String,
}

impl HostServerSession {
    /// Spawn Jaunder and wait for readiness until the caller requests interruption.
    pub fn start_interruptible<F>(
        config: ServerSessionConfig<'_>,
        mut interrupted: F,
    ) -> Result<Self, HostServerStartError>
    where
        F: FnMut() -> bool,
    {
        let runtime_file = config.runtime_file().to_path_buf();
        let ServerSessionConfig {
            shell,
            jaunder,
            storage,
            database_url,
            stderr,
            runtime_file: _,
            extra_env,
        } = config;
        let mut command = Command::new(jaunder).arg("serve");
        for (key, value) in &extra_env {
            command = command.env(key, value);
        }
        let command = command
            .env("JAUNDER_BIND", "127.0.0.1:0")
            .env("JAUNDER_STORAGE_PATH", &storage)
            .env("JAUNDER_DB", &database_url)
            .env("JAUNDER_RUNTIME_FILE", &runtime_file);
        let process = match start_process(command, stderr) {
            Ok(process) => process,
            Err(error) => {
                return Err(HostServerStartError {
                    phase: ServerStartPhase::Spawn,
                    error,
                    session: None,
                });
            }
        };
        let mut session = Self {
            process,
            base_url: String::new(),
        };
        let expected_identity = match session.process.identity() {
            Some(identity) => identity,
            None => {
                return Err(HostServerStartError {
                    phase: ServerStartPhase::RuntimeFile,
                    error: anyhow::anyhow!("could not determine spawned server process identity"),
                    session: Some(Box::new(session)),
                });
            }
        };
        let deadline = Instant::now() + RUNTIME_FILE_TIMEOUT;
        loop {
            if interrupted() {
                return Err(interrupted_start(session));
            }
            let remaining = deadline.saturating_duration_since(Instant::now());
            if remaining.is_zero() {
                return Err(HostServerStartError {
                    phase: ServerStartPhase::RuntimeFile,
                    error: anyhow::anyhow!("server did not create runtime.json within 15s"),
                    session: Some(Box::new(session)),
                });
            }
            if let Err(error) = session
                .process
                .wait_for_path(&runtime_file, remaining.min(READINESS_POLL_INTERVAL))
            {
                if session.process.is_stopped() {
                    return Err(HostServerStartError {
                        phase: ServerStartPhase::RuntimeFile,
                        error,
                        session: Some(Box::new(session)),
                    });
                }
                continue;
            }
            break;
        }
        for _ in 0..HTTP_READINESS_ATTEMPTS {
            if interrupted() {
                return Err(interrupted_start(session));
            }
            if session.process.is_stopped() {
                return Err(HostServerStartError {
                    phase: ServerStartPhase::Http,
                    error: anyhow::anyhow!("server exited before becoming HTTP-ready"),
                    session: Some(Box::new(session)),
                });
            }
            if let Ok(contents) = std::fs::read_to_string(&runtime_file)
                && let Some(base_url) = base_url_from_runtime(&contents, expected_identity)
                && cmd!(shell, "curl -sf -o /dev/null {base_url}/")
                    .quiet()
                    .run()
                    .is_ok()
            {
                session.base_url = base_url;
                return Ok(session);
            }
            let sleep_deadline = Instant::now() + HTTP_READINESS_INTERVAL;
            while Instant::now() < sleep_deadline {
                if interrupted() {
                    return Err(interrupted_start(session));
                }
                sleep(
                    sleep_deadline
                        .saturating_duration_since(Instant::now())
                        .min(READINESS_POLL_INTERVAL),
                );
            }
        }
        Err(HostServerStartError {
            phase: ServerStartPhase::Http,
            error: anyhow::anyhow!("server not reachable via runtime.json within 15s"),
            session: Some(Box::new(session)),
        })
    }

    /// Gracefully stop unless the caller observes a second shutdown signal.
    pub fn stop_interruptible<F>(&mut self, escalation: F) -> anyhow::Result<()>
    where
        F: Future<Output = ()>,
    {
        self.process
            .shutdown_interruptible(PROCESS_SHUTDOWN_TIMEOUT, escalation)
            .map(|_| ())
    }

    pub fn is_stopped(&self) -> bool {
        self.process.is_stopped()
    }
}

fn interrupted_start(session: HostServerSession) -> HostServerStartError {
    HostServerStartError {
        phase: ServerStartPhase::Interrupted,
        error: anyhow::anyhow!("server startup interrupted"),
        session: Some(Box::new(session)),
    }
}

fn start_process(command: Command, stderr: File) -> anyhow::Result<Process> {
    let stderr = tokio::fs::File::from_std(stderr);
    Process::start(
        command
            .stderr_raw_tee(stderr)
            .on_stderr_line(|line| eprintln!("{line}")),
    )
}

/// Checkout-local artifact ownership, released when the owner finishes rather
/// than when an unrelated fork finally closes its inherited descriptor.
#[derive(Debug)]
pub(crate) struct HostArtifactBuildLock {
    file: File,
}

impl Drop for HostArtifactBuildLock {
    fn drop(&mut self) {
        if let Err(error) = self.file.unlock() {
            use std::io::Write;

            // Drop must also work during unwinding. Report the failure without
            // panicking; closing the file remains the fallback release path.
            let _ = writeln!(
                std::io::stderr(),
                "cannot unlock host artifact build lock: {error}"
            );
        }
    }
}

pub(crate) fn host_artifact_build_lock() -> anyhow::Result<HostArtifactBuildLock> {
    host_artifact_build_lock_at(Path::new("."))
}

fn host_artifact_build_lock_at(root: &Path) -> anyhow::Result<HostArtifactBuildLock> {
    let path = root.join(".xtask/host-artifacts.lock");
    fs::create_dir_all(path.parent().expect("artifact lock has a parent"))?;
    let file = OpenOptions::new()
        .create(true)
        .truncate(false)
        .read(true)
        .write(true)
        .open(&path)?;
    file.lock()
        .with_context(|| format!("locking {}", path.display()))?;
    Ok(HostArtifactBuildLock { file })
}

/// Parse Jaunder's runtime-info JSON when it belongs to the expected child.
fn base_url_from_runtime(json: &str, expected_identity: (u32, Option<u64>)) -> Option<String> {
    let value: serde_json::Value = serde_json::from_str(json).ok()?;
    let pid = u32::try_from(value.get("pid")?.as_u64()?).ok()?;
    let start_time = match value.get("start_time")? {
        serde_json::Value::Null => None,
        value => Some(value.as_u64()?),
    };
    if (pid, start_time) != expected_identity {
        return None;
    }
    let ip = value.get("ip")?.as_str()?;
    let port = value.get("port")?.as_u64()?;
    Some(format!("http://{ip}:{port}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[cfg(unix)]
    fn preparation_fixture() -> Option<(tempfile::TempDir, Shell)> {
        use std::os::unix::fs::PermissionsExt;
        // xshell cannot mask an inherited variable; isolate the automatic-path
        // fixtures without mutating the multithreaded test process environment.
        if std::env::var_os("JAUNDER_CSR_BUNDLE_DIR").is_some() {
            let current = std::thread::current();
            let output = std::process::Command::new(std::env::current_exe().unwrap())
                .args(["--exact", current.name().unwrap(), "--nocapture"])
                .env_remove("JAUNDER_CSR_BUNDLE_DIR")
                .output()
                .unwrap();
            assert!(
                output.status.success(),
                "{}\n{}",
                String::from_utf8_lossy(&output.stdout),
                String::from_utf8_lossy(&output.stderr),
            );
            return None;
        }
        let temp = tempfile::tempdir().unwrap();
        assert!(
            crate::git::at(temp.path())
                .args(["init", "--quiet"])
                .status()
                .unwrap()
                .success()
        );
        fs::write(temp.path().join(".git/info/exclude"), "*\n").unwrap();
        for (name, body) in [
            ("cargo", "#!/bin/sh\ntouch \"$COMPILED\"\nexit 0\n"),
            (
                "just",
                "#!/bin/sh\nprintf '%s|%s' \"$PWD\" \"$*\" > \"$JUST_CALL\"\nexit \"${JUST_EXIT:-0}\"\n",
            ),
            (
                "nix",
                concat!(
                    "#!/bin/sh\nprintf '%s\\0' \"$@\" > \"$NIX_CALL\"\n",
                    "[ \"$1 $2 $3 $4\" = 'build --no-update-lock-file --no-write-lock-file --out-link' ] || exit 98\n",
                    "printf '%s' \"$5\" > \"$SELECTED_ROOT\"\nln -s \"$BUNDLE\" \"$5\"\nexit \"${NIX_EXIT:-37}\"\n",
                ),
            ),
        ] {
            let path = temp.path().join(name);
            fs::write(&path, body).unwrap();
            fs::set_permissions(path, fs::Permissions::from_mode(0o755)).unwrap();
        }
        let shell = Shell::new().unwrap();
        shell.change_dir(temp.path());
        shell.set_var(
            "PATH",
            format!(
                "{}:{}",
                temp.path().display(),
                std::env::var("PATH").unwrap()
            ),
        );
        shell.set_var("COMPILED", temp.path().join("compiled"));
        shell.set_var("JUST_CALL", temp.path().join("just-call"));
        shell.set_var("NIX_CALL", temp.path().join("nix-call"));
        shell.set_var("SELECTED_ROOT", temp.path().join("selected-root"));
        let bin = temp.path().join("bundle/bin");
        fs::create_dir_all(&bin).unwrap();
        for name in ["jaunder", "test-support"] {
            fs::write(bin.join(name), "#!/bin/sh\nexit 0\n").unwrap();
            fs::set_permissions(bin.join(name), fs::Permissions::from_mode(0o755)).unwrap();
        }
        shell.set_var("BUNDLE", temp.path().join("bundle"));
        Some((temp, shell))
    }

    #[test]
    #[cfg(unix)]
    fn caller_csr_override_is_refused_before_any_artifact_request() {
        let Some((temp, shell)) = preparation_fixture() else {
            return;
        };
        for value in ["", "caller-owned-bundle"] {
            shell.set_var("JAUNDER_CSR_BUNDLE_DIR", value);
            let mut result = CommandResult::new("e2e-local");
            let mut signals = super::super::host_signals::ServerSignals::install().unwrap();
            assert!(
                HostArtifacts::prepare_interruptible(&shell, &mut result, false, &mut signals,)
                    .is_none()
            );
            let message = serde_json::to_string(&result).unwrap();
            assert!(
                message.contains("unset JAUNDER_CSR_BUNDLE_DIR"),
                "{message}"
            );
            assert!(!temp.path().join("nix-call").exists());
            assert!(!temp.path().join("compiled").exists());
            assert!(!temp.path().join("just-call").exists());
        }
    }

    #[test]
    #[cfg(unix)]
    fn failed_release_selection_never_builds_mutable_artifacts() {
        let Some((temp, shell)) = preparation_fixture() else {
            return;
        };
        let mut result = CommandResult::new("e2e-local");
        let mut signals = super::super::host_signals::ServerSignals::install().unwrap();
        assert!(
            HostArtifacts::prepare_interruptible(&shell, &mut result, true, &mut signals,)
                .is_none()
        );
        assert!(!result.ok);
        assert!(!temp.path().join("compiled").exists());
        assert!(!temp.path().join("just-call").exists());
        let link = fs::read_to_string(temp.path().join("selected-root")).unwrap();
        assert!(!Path::new(&link).parent().unwrap().exists());
    }

    #[test]
    #[cfg(unix)]
    fn both_csr_choices_select_checkout_artifacts_and_retain_only_the_owned_root() {
        for (release, attr) in [
            (false, ".#sandboxArtifacts"),
            (true, ".#sandboxArtifactsReleaseCsr"),
        ] {
            let Some((temp, shell)) = preparation_fixture() else {
                return;
            };
            let nested = temp.path().join("nested");
            fs::create_dir(&nested).unwrap();
            shell.change_dir(nested);
            shell.set_var("NIX_EXIT", "0");
            let mut result = CommandResult::new("e2e-local");
            let mut signals = super::super::host_signals::ServerSignals::install().unwrap();
            let prepared =
                HostArtifacts::prepare_interruptible(&shell, &mut result, release, &mut signals)
                    .unwrap();
            assert_eq!(prepared.root, temp.path().canonicalize().unwrap());
            assert_eq!(
                prepared.jaunder,
                temp.path()
                    .join("bundle/bin/jaunder")
                    .canonicalize()
                    .unwrap()
            );
            assert_eq!(
                prepared.test_support,
                temp.path()
                    .join("bundle/bin/test-support")
                    .canonicalize()
                    .unwrap()
            );
            let argv = fs::read(temp.path().join("nix-call")).unwrap();
            assert_eq!(
                argv.split(|b| *b == 0).rfind(|p| !p.is_empty()).unwrap(),
                attr.as_bytes()
            );
            let retained = prepared._root.path().to_path_buf();
            assert!(retained.join("selected/bin/test-support").is_file());
            assert!(!temp.path().join(".xtask/host-artifacts.lock").exists());
            assert!(!temp.path().join("compiled").exists());
            assert!(!temp.path().join("just-call").exists());
            assert!(result.ok, "{:?}", result.steps);
            drop(prepared);
            assert!(!retained.exists());
        }
    }

    #[cfg(unix)]
    fn assert_child_cannot_lock(lock_path: &Path) {
        const CHILD_TEST: &str = "steps::host_server::tests::child_lock_probe";
        let output = std::process::Command::new(std::env::current_exe().unwrap())
            .args(["--exact", CHILD_TEST, "--nocapture"])
            .env("JAUNDER_TEST_HOST_ARTIFACT_LOCK_PROBE", lock_path)
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "child-process lock probe failed:\n{}\n{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
    }

    #[test]
    #[cfg(unix)]
    fn child_lock_probe() {
        let Some(lock_path) = std::env::var_os("JAUNDER_TEST_HOST_ARTIFACT_LOCK_PROBE") else {
            return;
        };
        let file = OpenOptions::new()
            .read(true)
            .write(true)
            .open(lock_path)
            .unwrap();
        match file.try_lock() {
            Err(std::fs::TryLockError::WouldBlock) => {}
            Err(error) => panic!("child lock probe could not check exclusion: {error}"),
            Ok(()) => panic!("child lock probe unexpectedly acquired the held artifact lock"),
        }
    }

    // test-local still depends on the shared guard; this is not e2e ownership.
    #[test]
    #[cfg(unix)]
    fn shared_artifact_guard_excludes_peers_and_releases_on_unwind() {
        let temp = tempfile::tempdir().unwrap();
        let lock = host_artifact_build_lock_at(temp.path()).unwrap();
        let path = temp.path().join(".xtask/host-artifacts.lock");
        assert_child_cannot_lock(&path);
        let unwind = std::panic::catch_unwind(move || {
            let _lock = lock;
            panic!("controlled owner unwind");
        });
        assert!(unwind.is_err());
        let peer = OpenOptions::new()
            .read(true)
            .write(true)
            .open(path)
            .unwrap();
        peer.try_lock().unwrap();
    }

    #[test]
    #[cfg(unix)]
    fn failed_debug_realization_never_selects_an_older_or_newly_published_root() {
        let Some((temp, shell)) = preparation_fixture() else {
            return;
        };
        super::super::csr_prerequisite::tests::write_bundle(
            &temp.path().join(".xtask/csr-debug-bundle"),
        );
        let mut result = CommandResult::new("e2e-local");
        let mut signals = super::super::host_signals::ServerSignals::install().unwrap();
        assert!(
            HostArtifacts::prepare_interruptible(&shell, &mut result, false, &mut signals,)
                .is_none()
        );
        assert!(!result.ok);
        assert!(!temp.path().join("compiled").exists());
        let link = fs::read_to_string(temp.path().join("selected-root")).unwrap();
        assert!(!Path::new(&link).parent().unwrap().exists());
        assert!(temp.path().join(".xtask/csr-debug-bundle").is_dir());
    }

    #[test]
    fn runtime_info_requires_expected_process_identity_ip_and_port() {
        let runtime = r#"{"ip":"127.0.0.1","pid":42,"port":4312,"start_time":99}"#;
        assert_eq!(
            base_url_from_runtime(runtime, (42, Some(99))),
            Some("http://127.0.0.1:4312".to_owned())
        );
        assert_eq!(base_url_from_runtime(runtime, (41, Some(99))), None);
        assert_eq!(base_url_from_runtime(runtime, (42, Some(98))), None);
        assert_eq!(
            base_url_from_runtime(
                r#"{"ip":"127.0.0.1","pid":42,"port":4312,"start_time":null}"#,
                (42, None),
            ),
            Some("http://127.0.0.1:4312".to_owned())
        );
        assert_eq!(
            base_url_from_runtime(r#"{"ip":"127.0.0.1","pid":42,"port":4312}"#, (42, None)),
            None
        );
        assert_eq!(
            base_url_from_runtime(r#"{"pid":42,"port":4312,"start_time":99}"#, (42, Some(99)),),
            None
        );
        assert_eq!(base_url_from_runtime("not json", (42, Some(99))), None);
    }

    #[test]
    #[cfg(unix)]
    fn interrupted_start_retains_the_spawned_session() {
        use std::os::unix::fs::PermissionsExt;

        let shell = Shell::new().expect("shell construction");
        let storage = tempfile::tempdir().expect("temporary storage");
        let server = storage.path().join("server");
        fs::write(&server, "#!/bin/sh\nsleep 60\n").expect("write sleeping server");
        fs::set_permissions(&server, fs::Permissions::from_mode(0o755))
            .expect("make sleeping server executable");
        let config = ServerSessionConfig {
            shell: &shell,
            jaunder: server,
            storage: storage.path().to_path_buf(),
            runtime_file: storage.path().join("runtime.json"),
            database_url: "sqlite:session.db".to_owned(),
            stderr: tempfile::NamedTempFile::new()
                .expect("temporary stderr")
                .into_file(),
            extra_env: Vec::new(),
        };

        let failure = HostServerSession::start_interruptible(config, || true)
            .err()
            .expect("interrupted startup must fail");

        assert_eq!(failure.phase(), ServerStartPhase::Interrupted);
        let mut session = failure.into_session().expect("retain spawned session");
        assert!(!session.is_stopped());
        session
            .stop_interruptible(async {})
            .expect("clean up sleeping server");
    }

    #[test]
    fn session_configuration_places_runtime_info_in_its_storage() {
        let shell = Shell::new().expect("shell construction");
        let storage = tempfile::tempdir().expect("temporary storage");
        let stderr = tempfile::NamedTempFile::new()
            .expect("temporary stderr")
            .into_file();
        let config = ServerSessionConfig {
            shell: &shell,
            runtime_file: storage.path().join("runtime.json"),
            jaunder: PathBuf::from("target/debug/jaunder"),
            storage: storage.path().to_path_buf(),
            database_url: "sqlite:session.db".to_owned(),
            stderr,
            extra_env: Vec::new(),
        };

        assert_eq!(config.runtime_file(), storage.path().join("runtime.json"));
    }
}
