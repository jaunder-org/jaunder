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
        }
    }
}

/// Current host-native artifacts prepared as one compatible set.
#[derive(Debug)]
pub struct HostArtifacts {
    pub root: PathBuf,
    pub jaunder: PathBuf,
    pub test_support: Option<PathBuf>,
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
        let _build_lock = match host_artifact_build_lock() {
            Ok(lock) => lock,
            Err(error) => {
                result.push(StepResult::fail(config.server_label).detail(format!(
                    "cannot acquire host artifact build lock: {error:#}"
                )));
                return None;
            }
        };
        if config.include_csr {
            super::build_csr::run(sh, result, config.csr_profile == HostBuildProfile::Release);
            if !result.ok {
                return None;
            }
        }

        let root_start = std::time::Instant::now();
        let root = match git::toplevel(Path::new(".")) {
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
            let build_start = std::time::Instant::now();
            let mut command = cmd!(sh, "cargo build -p {pkg}");
            if config.server_profile == HostBuildProfile::Release {
                command = command.arg("--release");
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
        })
    }
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
    /// Spawn Jaunder, wait for its runtime file, and verify an HTTP response.
    pub fn start(config: ServerSessionConfig<'_>) -> Result<Self, HostServerStartError> {
        Self::start_interruptible(config, || false)
    }

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

    /// Gracefully stop the process tree, escalating after the bounded drain.
    pub fn stop(&mut self) -> anyhow::Result<()> {
        self.process.shutdown(PROCESS_SHUTDOWN_TIMEOUT).map(|_| ())
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

fn host_artifact_build_lock() -> anyhow::Result<File> {
    let path = Path::new(".xtask/host-artifacts.lock");
    fs::create_dir_all(path.parent().expect("artifact lock has a parent"))?;
    let file = OpenOptions::new()
        .create(true)
        .truncate(false)
        .read(true)
        .write(true)
        .open(path)?;
    file.lock()
        .with_context(|| format!("locking {}", path.display()))?;
    Ok(file)
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
    fn e2e_artifact_configuration_preserves_debug_server_builds() {
        assert_eq!(
            HostArtifactConfig::e2e_local(false),
            HostArtifactConfig {
                csr_profile: HostBuildProfile::Debug,
                server_profile: HostBuildProfile::Debug,
                include_csr: true,
                include_test_support: true,
                server_label: "e2e-local-build-server",
                test_support_label: "e2e-local-build-support",
            }
        );
        assert_eq!(
            HostArtifactConfig::e2e_local(true).csr_profile,
            HostBuildProfile::Release
        );
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
