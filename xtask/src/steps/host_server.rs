//! Shared host-native Jaunder build and server-session lifecycle.

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

/// The optimization level of a host artifact.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum HostBuildProfile {
    Debug,
    Release,
}

/// The artifact variants a host caller needs for one session.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct HostArtifactConfig {
    pub csr_profile: HostBuildProfile,
    pub server_profile: HostBuildProfile,
    pub include_csr: bool,
    pub include_test_support: bool,
    pub server_label: &'static str,
    pub test_support_label: &'static str,
    nix_debug_bundle: bool,
}

impl HostArtifactConfig {
    pub const fn e2e_local(release_csr: bool) -> Self {
        Self {
            csr_profile: if release_csr {
                HostBuildProfile::Release
            } else {
                HostBuildProfile::Debug
            },
            // Preserve e2e-local's historical debug server build.
            server_profile: HostBuildProfile::Debug,
            include_csr: true,
            include_test_support: true,
            server_label: "e2e-local-build-server",
            test_support_label: "e2e-local-build-support",
            nix_debug_bundle: !release_csr,
        }
    }

    pub const fn sandbox_server() -> Self {
        Self {
            csr_profile: HostBuildProfile::Debug,
            server_profile: HostBuildProfile::Debug,
            include_csr: true,
            include_test_support: true,
            server_label: "sandbox-build-server",
            test_support_label: "sandbox-build-support",
            nix_debug_bundle: false,
        }
    }

    pub const fn sandbox_command() -> Self {
        Self {
            csr_profile: HostBuildProfile::Debug,
            server_profile: HostBuildProfile::Debug,
            include_csr: false,
            include_test_support: false,
            server_label: "sandbox-build-jaunder",
            test_support_label: "sandbox-build-support",
            nix_debug_bundle: false,
        }
    }
}

/// Current host-native artifacts prepared as one compatible set.
#[derive(Debug)]
pub struct HostArtifacts {
    pub root: PathBuf,
    pub jaunder: PathBuf,
    pub test_support: Option<PathBuf>,
    // Retained by the caller until every consumer and its teardown have finished.
    pub(super) _build_lock: Option<HostArtifactBuildLock>,
}

impl HostArtifacts {
    /// Build the selected CSR bundle and the binaries used by host sessions.
    ///
    /// Step names deliberately remain those established by `e2e-local`.
    pub fn prepare(
        sh: &Shell,
        result: &mut CommandResult,
        config: HostArtifactConfig,
    ) -> Option<Self> {
        // Interactive sandboxes allow operational commands while a managed
        // server is live; preserve their preparation-only lock scope.
        let mut artifacts = Self::prepare_interruptible(sh, result, config, || None)?;
        artifacts._build_lock.take();
        Some(artifacts)
    }

    pub(super) fn prepare_interruptible(
        sh: &Shell,
        result: &mut CommandResult,
        config: HostArtifactConfig,
        mut interrupted: impl FnMut() -> Option<i32>,
    ) -> Option<Self> {
        let build_lock = match host_artifact_build_lock_at(&sh.current_dir()) {
            Ok(lock) => lock,
            Err(error) => {
                result.push(StepResult::fail(config.server_label).detail(format!(
                    "cannot acquire host artifact build lock: {error:#}"
                )));
                return None;
            }
        };
        if let Some(code) = interrupted() {
            result.exit_override = Some(code);
            return None;
        }
        let bundle = if config.nix_debug_bundle {
            let started = Instant::now();
            match e2e_debug_bundle(sh) {
                Ok(root) => {
                    result.push(
                        StepResult::ok("e2e-local-csr-bundle")
                            .detail(format!("verified CSR bundle {}", root.display()))
                            .with_duration(started.elapsed()),
                    );
                    Some(root)
                }
                Err(error) => {
                    result.push(
                        StepResult::fail("e2e-local-csr-bundle")
                            .detail(format!("{error:#}"))
                            .with_duration(started.elapsed()),
                    );
                    return None;
                }
            }
        } else {
            if config.include_csr {
                super::build_csr::run(sh, result, config.csr_profile == HostBuildProfile::Release);
                if !result.ok {
                    return None;
                }
            }
            None
        };

        let root_start = std::time::Instant::now();
        let root = match git::toplevel(&sh.current_dir()) {
            Ok(root) => PathBuf::from(root),
            Err(_) => {
                result.push(
                    StepResult::fail("e2e-local")
                        .detail("cannot locate repo root".to_owned())
                        .with_duration(root_start.elapsed()),
                );
                return None;
            }
        };

        let mut binaries = vec![("jaunder", config.server_label)];
        if config.include_test_support {
            binaries.push(("test-support", config.test_support_label));
        }
        for (pkg, label) in binaries {
            if let Some(code) = interrupted() {
                result.exit_override = Some(code);
                return None;
            }
            let build_start = std::time::Instant::now();
            let mut command = cmd!(sh, "cargo build -p {pkg}");
            if config.server_profile == HostBuildProfile::Release {
                command = command.arg("--release");
            }
            if let Some(root) = &bundle {
                command = command.env("JAUNDER_CSR_BUNDLE_DIR", root);
            }
            if command.run().is_err() {
                result.push(
                    StepResult::fail(label)
                        .detail(format!("cargo build -p {pkg} failed"))
                        .with_duration(build_start.elapsed()),
                );
                return None;
            }
            result.push(StepResult::ok(label).with_duration(build_start.elapsed()));
        }

        let profile = match config.server_profile {
            HostBuildProfile::Debug => "debug",
            HostBuildProfile::Release => "release",
        };
        Some(Self {
            jaunder: root.join(format!("target/{profile}/jaunder")),
            test_support: config
                .include_test_support
                .then(|| root.join(format!("target/{profile}/test-support"))),
            root,
            _build_lock: Some(build_lock),
        })
    }
}

fn e2e_debug_bundle(sh: &Shell) -> anyhow::Result<PathBuf> {
    let root = match sh.var_os("JAUNDER_CSR_BUNDLE_DIR") {
        Some(root) => sh.current_dir().join(root),
        None => {
            crate::host_tooling::admit_untracked_inputs(
                &sh.current_dir(),
                ".#csrDebugBundle.sourceAdmits",
            )?;
            let root = sh.current_dir().join(".xtask/csr-debug-bundle");
            sh.create_dir(root.parent().expect("bundle root has a parent"))?;
            crate::nix_build::build_rooted(sh, "csrDebugBundle", &root)?;
            root
        }
    };
    super::csr_prerequisite::validate(&root)
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

    /// Request immediate process-tree cleanup when a caller has already escalated.
    pub fn force_stop(&mut self) -> anyhow::Result<()> {
        self.process.shutdown(Duration::ZERO).map(|_| ())
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
            ("nix", "#!/bin/sh\nexit 37\n"),
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
        Some((temp, shell))
    }

    #[test]
    #[cfg(unix)]
    fn explicit_bundle_is_validated_before_compilation_without_automatic_fallback() {
        let Some((temp, shell)) = preparation_fixture() else {
            return;
        };
        let stale = temp.path().join(".xtask/csr-debug-bundle");
        super::super::csr_prerequisite::tests::write_bundle(&stale);
        for input in ["missing", "corrupt"] {
            fs::create_dir_all(temp.path().join("corrupt")).unwrap();
            fs::write(temp.path().join("corrupt/manifest.json"), "corrupt").unwrap();
            shell.set_var("JAUNDER_CSR_BUNDLE_DIR", input);
            let mut result = CommandResult::new("e2e-local");
            assert!(
                HostArtifacts::prepare_interruptible(
                    &shell,
                    &mut result,
                    HostArtifactConfig::e2e_local(false),
                    || None
                )
                .is_none()
            );
            assert!(!result.ok);
            assert!(!temp.path().join("compiled").exists());
        }
        shell.set_var("JAUNDER_CSR_BUNDLE_DIR", ".xtask/csr-debug-bundle/.");
        let mut result = CommandResult::new("e2e-local");
        let prepared = HostArtifacts::prepare_interruptible(
            &shell,
            &mut result,
            HostArtifactConfig::e2e_local(false),
            || None,
        )
        .expect("valid explicit input does not invoke the failing Nix producer");
        assert!(result.ok, "{:?}", result.steps);
        assert!(temp.path().join("compiled").exists());
        assert!(!temp.path().join("target/site").exists());
        drop(prepared);
    }

    #[test]
    #[cfg(unix)]
    fn failed_debug_realization_never_compiles_an_older_root() {
        let Some((temp, shell)) = preparation_fixture() else {
            return;
        };
        super::super::csr_prerequisite::tests::write_bundle(
            &temp.path().join(".xtask/csr-debug-bundle"),
        );
        let mut result = CommandResult::new("e2e-local");
        assert!(
            HostArtifacts::prepare_interruptible(
                &shell,
                &mut result,
                HostArtifactConfig::e2e_local(false),
                || None
            )
            .is_none()
        );
        assert!(!result.ok);
        assert!(!temp.path().join("compiled").exists());
    }

    #[test]
    #[cfg(unix)]
    fn failed_host_compilation_releases_preparation_ownership() {
        let Some((temp, shell)) = preparation_fixture() else {
            return;
        };
        let bundle = temp.path().join("bundle");
        super::super::csr_prerequisite::tests::write_bundle(&bundle);
        shell.set_var("JAUNDER_CSR_BUNDLE_DIR", bundle);
        fs::write(temp.path().join("cargo"), "#!/bin/sh\nexit 9\n").unwrap();
        let mut result = CommandResult::new("e2e-local");
        assert!(
            HostArtifacts::prepare_interruptible(
                &shell,
                &mut result,
                HostArtifactConfig::e2e_local(false),
                || None
            )
            .is_none()
        );
        assert!(!result.ok);
        let peer = OpenOptions::new()
            .read(true)
            .write(true)
            .open(temp.path().join(".xtask/host-artifacts.lock"))
            .unwrap();
        peer.try_lock()
            .expect("failed consumer compilation releases the guard");
    }

    #[test]
    #[cfg(unix)]
    fn prepared_artifacts_exclude_peers_until_drop_and_release_on_unwind() {
        let Some((temp, shell)) = preparation_fixture() else {
            return;
        };
        let bundle = temp.path().join("bundle");
        super::super::csr_prerequisite::tests::write_bundle(&bundle);
        shell.set_var("JAUNDER_CSR_BUNDLE_DIR", &bundle);
        let mut result = CommandResult::new("e2e-local");
        let prepared = HostArtifacts::prepare_interruptible(
            &shell,
            &mut result,
            HostArtifactConfig::e2e_local(false),
            || None,
        )
        .unwrap();
        let peer = OpenOptions::new()
            .read(true)
            .write(true)
            .open(temp.path().join(".xtask/host-artifacts.lock"))
            .unwrap();
        assert!(matches!(
            peer.try_lock(),
            Err(std::fs::TryLockError::WouldBlock)
        ));
        let unwind = std::panic::catch_unwind(move || {
            let _prepared = prepared;
            panic!("controlled consumer unwind");
        });
        assert!(unwind.is_err());
        peer.try_lock().expect("unwind releases prepared ownership");
    }

    #[test]
    #[cfg(unix)]
    fn pending_interruption_blocks_host_consumers() {
        let Some((temp, shell)) = preparation_fixture() else {
            return;
        };
        let bundle = temp.path().join("bundle");
        super::super::csr_prerequisite::tests::write_bundle(&bundle);
        shell.set_var("JAUNDER_CSR_BUNDLE_DIR", bundle);
        let mut result = CommandResult::new("e2e-local");
        let mut boundaries = 0;
        assert!(
            HostArtifacts::prepare_interruptible(
                &shell,
                &mut result,
                HostArtifactConfig::e2e_local(false),
                || {
                    boundaries += 1;
                    (boundaries == 2).then_some(143)
                }
            )
            .is_none()
        );
        assert_eq!(result.exit_override, Some(143));
        assert!(!temp.path().join("compiled").exists());
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
        session.force_stop().expect("clean up sleeping server");
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
