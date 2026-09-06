//! Persistent and disposable host-native UX sandbox orchestration.
//!
//! Lock order is fixed: acquire the per-name server lease before taking a
//! workspace lock for a server-generation transition.  Command mode never takes
//! the server lease.  This prevents a reset/server cycle from deadlocking with a
//! command that has pinned a ready generation.

use std::fs::{self, File, OpenOptions};
use std::io::Read;
use std::io::Write;
use std::os::unix::process::ExitStatusExt;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::thread::sleep;
use std::time::Duration;
use std::time::Instant;

use anyhow::{Context as _, bail};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use tokio::runtime::Builder;
use xshell::Shell;

use crate::cli::SandboxProfile;
use crate::result::{CommandResult, StepResult};
use crate::steps::host_server::{
    HostArtifactConfig, HostArtifacts, HostServerSession, ServerSessionConfig, ServerStartPhase,
};

const SANDBOX_ROOT: &str = ".xtask/sandboxes";
const PROFILE_METADATA: &str = ".sandbox.json";
const METADATA_VERSION: u8 = 1;
const ALLOWED_COMMANDS: &[&str] = &[
    "site-config",
    "user-create",
    "app-password-create",
    "user-invite",
    "smtp-test",
    "backup",
    "websub",
];

#[derive(Serialize, Deserialize)]
struct ProfileMetadata {
    version: u8,
    profile: SandboxProfile,
}

#[derive(Serialize, Deserialize)]
struct ServerMetadata {
    version: u8,
    state: ServerState,
    executable_digest: String,
}

#[derive(Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
enum ServerState {
    Ready,
    Stopping,
}

pub fn run(
    sh: &Shell,
    result: &mut CommandResult,
    name: Option<String>,
    profile: Option<SandboxProfile>,
    reset: bool,
    command: Vec<String>,
) {
    let start = Instant::now();
    let Some(mode) = validate(name.as_deref(), profile, reset, &command)
        .map_err(|error| record(result, "sandbox-validate", error))
        .ok()
    else {
        return;
    };
    if let Mode::Server(Some(name)) = &mode
        && let Err(error) = preflight_named_server(name, profile, reset)
    {
        record(result, "sandbox-validate", error);
        return;
    }
    match mode {
        Mode::Command(name) => run_command(sh, result, &name, command),
        Mode::Server(name) => run_server(sh, result, name, profile, reset),
    }
    result.push(StepResult::ok("sandbox").with_duration(start.elapsed()));
}

enum Mode {
    Server(Option<String>),
    Command(String),
}

enum ServerExit {
    Signal(i32),
    Unexpected(i32),
}

impl ServerExit {
    fn code(&self) -> i32 {
        match self {
            Self::Signal(code) | Self::Unexpected(code) => *code,
        }
    }
}

struct ServerSignals {
    runtime: tokio::runtime::Runtime,
    interrupt: tokio::signal::unix::Signal,
    terminate: tokio::signal::unix::Signal,
}

struct NamedServer {
    artifacts: HostArtifacts,
    digest: String,
    name: String,
    selected_profile: Option<SandboxProfile>,
    reset: bool,
    server_lease: File,
}

fn validate(
    name: Option<&str>,
    profile: Option<SandboxProfile>,
    reset: bool,
    command: &[String],
) -> anyhow::Result<Mode> {
    if let Some(name) = name {
        validate_name(name)?;
    }
    if !command.is_empty() {
        let name = name.context("command mode requires a named workspace")?;
        if profile.is_some() || reset {
            bail!("command mode is incompatible with --profile and --reset");
        }
        validate_operational_command(command)?;
        return Ok(Mode::Command(name.to_owned()));
    }
    if reset && name.is_none() {
        bail!("--reset requires NAME");
    }
    Ok(Mode::Server(name.map(str::to_owned)))
}

fn validate_name(name: &str) -> anyhow::Result<()> {
    let mut bytes = name.bytes();
    let Some(first) = bytes.next() else {
        bail!("sandbox NAME must not be empty");
    };
    if !first.is_ascii_lowercase() && !first.is_ascii_digit() {
        bail!("sandbox NAME must begin with a lowercase letter or digit");
    }
    if !bytes.all(|byte| {
        byte.is_ascii_lowercase() || byte.is_ascii_digit() || matches!(byte, b'-' | b'_')
    }) {
        bail!("sandbox NAME may contain only lowercase ASCII letters, digits, '-' and '_'");
    }
    Ok(())
}

fn validate_operational_command(command: &[String]) -> anyhow::Result<()> {
    let top = command
        .first()
        .context("command mode requires a Jaunder command")?;
    if !ALLOWED_COMMANDS.contains(&top.as_str()) {
        bail!("`{top}` is not permitted in sandbox command mode");
    }
    if command.iter().any(|argument| {
        matches!(argument.as_str(), "--storage-path" | "--db")
            || argument.starts_with("--storage-path=")
            || argument.starts_with("--db=")
    }) {
        bail!("sandbox command mode manages --storage-path and --db itself");
    }
    Ok(())
}

fn run_command(sh: &Shell, result: &mut CommandResult, name: &str, command: Vec<String>) {
    let workspace = named_workspace(name);
    let Some(artifacts) = HostArtifacts::prepare(sh, result, HostArtifactConfig::sandbox_command())
    else {
        return;
    };
    let digest = match executable_digest(&artifacts.jaunder) {
        Ok(digest) => digest,
        Err(error) => {
            record(result, "sandbox-command", error);
            return;
        }
    };
    let locks = LockPaths::for_name(name);
    let guards = match command_admission(name, &workspace, &locks, &digest) {
        Ok(guards) => guards,
        Err(error) => {
            record(result, "sandbox-command", error);
            return;
        }
    };
    let status = Command::new(&artifacts.jaunder)
        .args(&command)
        .env("JAUNDER_STORAGE_PATH", &workspace)
        .env("JAUNDER_DB", database_url(&workspace))
        .status();
    drop(guards);
    match status {
        Ok(status) => {
            let code = status
                .code()
                .unwrap_or_else(|| 128 + status.signal().unwrap_or(1));
            result.exit_override = Some(code);
            result.push(if status.success() {
                StepResult::ok("sandbox-command")
            } else {
                StepResult::fail("sandbox-command")
                    .detail(format!("child exited with status {code}"))
            });
        }
        Err(error) => record(result, "sandbox-command", error.into()),
    }
}

#[derive(Debug)]
struct CommandLocks {
    _workspace: File,
    _runtime: Option<File>,
}

fn command_admission(
    name: &str,
    workspace: &Path,
    locks: &LockPaths,
    digest: &str,
) -> anyhow::Result<CommandLocks> {
    let server_probe = lock_file(&locks.server)?;
    match server_probe.try_lock() {
        Ok(()) => {
            let exclusive = lock_exclusive_blocking(&locks.workspace)?;
            recover_workspace(name)?;
            if !workspace.is_dir() {
                bail!("sandbox `{name}` does not exist");
            }
            read_profile_metadata(workspace)?;
            let _ = fs::remove_file(&locks.metadata);
            let runtime = try_runtime_lock(&workspace.join("runtime.lock"))?;
            drop(exclusive);
            let shared = lock_shared(&locks.workspace)?;
            drop(server_probe);
            Ok(CommandLocks {
                _workspace: shared,
                _runtime: Some(runtime),
            })
        }
        Err(std::fs::TryLockError::WouldBlock) => {
            let shared = lock_shared(&locks.workspace)?;
            if !workspace.is_dir() {
                bail!("sandbox `{name}` does not exist");
            }
            match read_server_metadata(&locks.metadata)? {
                Some(metadata)
                    if metadata.state == ServerState::Ready
                        && metadata.executable_digest == digest =>
                {
                    Ok(CommandLocks {
                        _workspace: shared,
                        _runtime: None,
                    })
                }
                Some(ServerMetadata {
                    state: ServerState::Ready,
                    ..
                }) => bail!(
                    "sandbox server uses a different executable; restart the sandbox before running commands"
                ),
                Some(_) | None => bail!("sandbox server is starting or stopping; retry shortly"),
            }
        }
        Err(std::fs::TryLockError::Error(error)) => {
            Err(error).context("probing the sandbox server lease")
        }
    }
}

fn run_server(
    sh: &Shell,
    result: &mut CommandResult,
    name: Option<String>,
    selected_profile: Option<SandboxProfile>,
    reset: bool,
) {
    let mut signals = match ServerSignals::install() {
        Ok(signals) => signals,
        Err(error) => {
            record(result, "sandbox-signals", error);
            return;
        }
    };
    let server_lease = match name.as_deref() {
        Some(name) => {
            let locks = LockPaths::for_name(name);
            match lock_exclusive(&locks.server) {
                Ok(lock) => Some(lock),
                Err(_) => {
                    record(
                        result,
                        "sandbox-server",
                        anyhow::anyhow!("sandbox `{name}` already has a managed server"),
                    );
                    return;
                }
            }
        }
        None => None,
    };
    let artifacts = HostArtifacts::prepare(sh, result, HostArtifactConfig::sandbox_server());
    let Some(artifacts) = artifacts else {
        if let Some(code) = signals.take_pending() {
            result.exit_override = Some(code);
        }
        return;
    };
    if let Some(code) = signals.take_pending() {
        result.exit_override = Some(code);
        return;
    }
    match name {
        Some(name) => {
            let digest = match executable_digest(&artifacts.jaunder) {
                Ok(digest) => digest,
                Err(error) => {
                    record(result, "sandbox-server", error);
                    return;
                }
            };
            run_named_server(
                sh,
                result,
                NamedServer {
                    artifacts,
                    digest,
                    name,
                    selected_profile,
                    reset,
                    server_lease: server_lease.expect("named sandbox acquired a server lease"),
                },
                &mut signals,
            );
        }
        None => run_disposable_server(
            sh,
            result,
            artifacts,
            selected_profile.unwrap_or(SandboxProfile::Empty),
            &mut signals,
        ),
    }
}

fn run_disposable_server(
    sh: &Shell,
    result: &mut CommandResult,
    artifacts: HostArtifacts,
    profile: SandboxProfile,
    signals: &mut ServerSignals,
) {
    let temp = match tempfile::tempdir().context("creating disposable sandbox") {
        Ok(temp) => temp,
        Err(error) => {
            record(result, "sandbox-workspace", error);
            return;
        }
    };
    let workspace = temp.path().join("workspace");
    if let Err(error) = prepare_workspace(&artifacts, &workspace, profile) {
        record_startup_failure(result, signals, "sandbox-workspace", error);
        return;
    }
    if let Some(code) = signals.take_pending() {
        result.exit_override = Some(code);
        return;
    }
    run_session(sh, result, &artifacts, &workspace, profile, signals);
}

fn run_named_server(
    sh: &Shell,
    result: &mut CommandResult,
    config: NamedServer,
    signals: &mut ServerSignals,
) {
    let NamedServer {
        artifacts,
        digest,
        name,
        selected_profile,
        reset,
        server_lease,
    } = config;
    let locks = LockPaths::for_name(&name);
    let exclusive = match lock_exclusive_blocking(&locks.workspace) {
        Ok(lock) => lock,
        Err(error) => {
            record(result, "sandbox-workspace", error);
            return;
        }
    };
    if let Err(error) = recover_workspace(&name) {
        record(result, "sandbox-recovery", error);
        return;
    }
    let workspace = named_workspace(&name);
    let profile = match workspace_profile(&workspace, selected_profile, reset) {
        Ok(profile) => profile,
        Err(error) => {
            record(result, "sandbox-validate", error);
            return;
        }
    };
    if let Err(error) = ensure_named_workspace(&artifacts, &name, profile, reset) {
        record_startup_failure(result, signals, "sandbox-workspace", error);
        return;
    }
    if let Some(code) = signals.take_pending() {
        result.exit_override = Some(code);
        return;
    }
    let mut session = match start_session(sh, &artifacts, &workspace) {
        Ok(session) => session,
        Err(error) => {
            record_startup_failure(result, signals, "sandbox-server", error);
            return;
        }
    };
    if let Err(error) = write_server_metadata(&locks.metadata, ServerState::Ready, &digest) {
        let _ = session.force_stop();
        record(result, "sandbox-server", error);
        return;
    }
    drop(exclusive);
    let shared = match lock_shared(&locks.workspace) {
        Ok(lock) => lock,
        Err(error) => {
            let _ = session.force_stop();
            record(result, "sandbox-server", error);
            return;
        }
    };
    print_session(&session, profile);
    let exit = signals.wait_for_server(&mut session);
    let code = exit.code();
    let shutdown = match write_stopping_metadata(&locks.metadata, &digest) {
        Ok(()) => signals.shutdown_session(&mut session, exit),
        Err(error) => {
            record(result, "sandbox-shutdown", error);
            session.force_stop()
        }
    };
    drop(shared);
    let exclusive = match lock_exclusive_blocking(&locks.workspace) {
        Ok(lock) => lock,
        Err(error) => {
            record(result, "sandbox-shutdown", error);
            return;
        }
    };
    let _ = fs::remove_file(&locks.metadata);
    drop(exclusive);
    drop(server_lease);
    if let Err(error) = shutdown {
        record(result, "sandbox-shutdown", error);
    } else if !is_signal_status(code) {
        record(
            result,
            "sandbox-server",
            anyhow::anyhow!("sandbox server exited unexpectedly ({code})"),
        );
    }
    result.exit_override = Some(code);
}

fn run_session(
    sh: &Shell,
    result: &mut CommandResult,
    artifacts: &HostArtifacts,
    workspace: &Path,
    profile: SandboxProfile,
    signals: &mut ServerSignals,
) {
    let mut session = match start_session(sh, artifacts, workspace) {
        Ok(session) => session,
        Err(error) => {
            record_startup_failure(result, signals, "sandbox-server", error);
            return;
        }
    };
    print_session(&session, profile);
    let exit = signals.wait_for_server(&mut session);
    let code = exit.code();
    if let Err(error) = signals.shutdown_session(&mut session, exit) {
        record(result, "sandbox-shutdown", error);
    } else if !is_signal_status(code) {
        record(
            result,
            "sandbox-server",
            anyhow::anyhow!("sandbox server exited unexpectedly ({code})"),
        );
    }
    result.exit_override = Some(code);
}

fn start_session(
    sh: &Shell,
    artifacts: &HostArtifacts,
    workspace: &Path,
) -> anyhow::Result<HostServerSession> {
    let stderr = File::create(workspace.join("sandbox-server.stderr"))
        .context("creating sandbox stderr capture")?;
    HostServerSession::start(ServerSessionConfig {
        shell: sh,
        jaunder: artifacts.jaunder.clone(),
        storage: workspace.to_path_buf(),
        runtime_file: workspace.join("runtime.json"),
        database_url: database_url(workspace),
        stderr,
        extra_env: Vec::new(),
    })
    .map_err(|error| {
        let phase = match error.phase() {
            ServerStartPhase::Spawn => "spawn",
            ServerStartPhase::RuntimeFile => "runtime file",
            ServerStartPhase::Http => "HTTP readiness",
        };
        let detail = error.error().to_string();
        let _ = error.into_session().map(|mut session| session.force_stop());
        anyhow::anyhow!("sandbox server {phase} failed: {detail}")
    })
}

impl ServerSignals {
    fn install() -> anyhow::Result<Self> {
        let runtime = Builder::new_current_thread()
            .enable_time()
            .enable_io()
            .build()
            .context("creating sandbox signal runtime")?;
        let (interrupt, terminate) = runtime.block_on(async {
            Ok::<_, std::io::Error>((
                tokio::signal::unix::signal(tokio::signal::unix::SignalKind::interrupt())?,
                tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())?,
            ))
        })?;
        Ok(Self {
            runtime,
            interrupt,
            terminate,
        })
    }

    fn take_pending(&mut self) -> Option<i32> {
        self.runtime.block_on(async {
            tokio::time::timeout(Duration::from_millis(1), async {
                tokio::select! {
                    _ = self.interrupt.recv() => 130,
                    _ = self.terminate.recv() => 143,
                }
            })
            .await
            .ok()
        })
    }

    fn wait_for_server(&mut self, session: &mut HostServerSession) -> ServerExit {
        self.runtime.block_on(async {
            loop {
                if session.is_stopped() {
                    return ServerExit::Unexpected(1);
                }
                tokio::select! {
                    _ = tokio::time::sleep(Duration::from_millis(100)) => {},
                    _ = self.interrupt.recv() => return ServerExit::Signal(130),
                    _ = self.terminate.recv() => return ServerExit::Signal(143),
                }
            }
        })
    }

    fn shutdown_session(
        &mut self,
        session: &mut HostServerSession,
        exit: ServerExit,
    ) -> anyhow::Result<()> {
        match exit {
            ServerExit::Signal(_) => session.stop_interruptible(async {
                tokio::select! {
                    _ = self.interrupt.recv() => {},
                    _ = self.terminate.recv() => {},
                }
            }),
            ServerExit::Unexpected(_) => Ok(()),
        }
    }
}

fn record_startup_failure(
    result: &mut CommandResult,
    signals: &mut ServerSignals,
    step: &'static str,
    error: anyhow::Error,
) {
    if let Some(code) = signals.take_pending() {
        result.exit_override = Some(code);
    } else {
        record(result, step, error);
    }
}

fn preflight_named_server(
    name: &str,
    selected: Option<SandboxProfile>,
    reset: bool,
) -> anyhow::Result<()> {
    let workspace = named_workspace(name);
    if workspace.exists() && !reset && selected.is_some() {
        bail!("--profile is only valid when creating or resetting a named workspace");
    }
    Ok(())
}

fn workspace_profile(
    workspace: &Path,
    selected: Option<SandboxProfile>,
    reset: bool,
) -> anyhow::Result<SandboxProfile> {
    if !workspace.exists() {
        if reset {
            bail!("cannot reset absent sandbox workspace");
        }
        return Ok(selected.unwrap_or(SandboxProfile::Empty));
    }
    let metadata = read_profile_metadata(workspace)?;
    if reset {
        return Ok(selected.unwrap_or(metadata.profile));
    }
    if selected.is_some() {
        bail!("--profile is only valid when creating or resetting a named workspace");
    }
    Ok(metadata.profile)
}

fn ensure_named_workspace(
    artifacts: &HostArtifacts,
    name: &str,
    profile: SandboxProfile,
    reset: bool,
) -> anyhow::Result<()> {
    let workspace = named_workspace(name);
    if workspace.exists() && !reset {
        return Ok(());
    }
    if reset {
        let runtime = try_runtime_lock(&workspace.join("runtime.lock"))?;
        drop(runtime);
    }
    let parent = sandbox_root();
    fs::create_dir_all(&parent)?;
    let new = parent.join(format!(".{name}.reset-new"));
    let old = parent.join(format!(".{name}.reset-old"));
    let _ = fs::remove_dir_all(&new);
    if let Err(error) = prepare_workspace(artifacts, &new, profile) {
        let _ = fs::remove_dir_all(&new);
        return Err(error);
    }
    if workspace.exists() {
        fs::rename(&workspace, &old).context("retaining old sandbox workspace")?;
        if let Err(error) = fs::rename(&new, &workspace) {
            let _ = fs::rename(&old, &workspace);
            return Err(error).context("publishing replacement sandbox workspace");
        }
        fs::remove_dir_all(old)?;
    } else {
        fs::rename(new, workspace)?;
    }
    Ok(())
}

fn recover_workspace(name: &str) -> anyhow::Result<()> {
    let parent = sandbox_root();
    let workspace = named_workspace(name);
    let new = parent.join(format!(".{name}.reset-new"));
    let old = parent.join(format!(".{name}.reset-old"));
    match (workspace.exists(), new.exists(), old.exists()) {
        (false, _, true) => fs::rename(old, workspace)?,
        (false, true, false) => fs::remove_dir_all(new)?,
        (true, _, true) => fs::remove_dir_all(old)?,
        (true, true, false) => fs::remove_dir_all(new)?,
        _ => {}
    }
    Ok(())
}

fn prepare_workspace(
    artifacts: &HostArtifacts,
    workspace: &Path,
    profile: SandboxProfile,
) -> anyhow::Result<()> {
    let status = Command::new(&artifacts.jaunder)
        .arg("init")
        .env("JAUNDER_STORAGE_PATH", workspace)
        .env("JAUNDER_DB", database_url(workspace))
        .status()
        .context("running jaunder init")?;
    if !status.success() {
        bail!("jaunder init failed");
    }
    if profile != SandboxProfile::Empty {
        let support = artifacts
            .test_support
            .as_ref()
            .context("sandbox profile seeder was not built")?;
        let status = Command::new(support)
            .args(["seed-sandbox-profile", "--db"])
            .arg(database_url(workspace))
            .args(["--profile", profile.as_str()])
            .status()
            .context("running sandbox profile seed")?;
        if !status.success() {
            bail!("sandbox profile seed failed");
        }
    }
    write_profile_metadata(workspace, profile)
}

fn named_workspace(name: &str) -> PathBuf {
    sandbox_root().join(name)
}
fn sandbox_root() -> PathBuf {
    PathBuf::from(SANDBOX_ROOT)
}
fn database_url(workspace: &Path) -> String {
    format!("sqlite:{}", workspace.join("jaunder.db").display())
}

fn write_profile_metadata(workspace: &Path, profile: SandboxProfile) -> anyhow::Result<()> {
    write_atomic(
        &workspace.join(PROFILE_METADATA),
        &ProfileMetadata {
            version: METADATA_VERSION,
            profile,
        },
    )
}
fn read_profile_metadata(workspace: &Path) -> anyhow::Result<ProfileMetadata> {
    let metadata = fs::read_to_string(workspace.join(PROFILE_METADATA))
        .context("reading sandbox profile metadata")?;
    let metadata: ProfileMetadata =
        serde_json::from_str(&metadata).context("parsing sandbox profile metadata")?;
    if metadata.version != METADATA_VERSION {
        bail!(
            "unsupported sandbox profile metadata version {}",
            metadata.version
        );
    }
    Ok(metadata)
}
fn write_server_metadata(
    path: &Path,
    state: ServerState,
    executable_digest: &str,
) -> anyhow::Result<()> {
    write_atomic(
        path,
        &ServerMetadata {
            version: METADATA_VERSION,
            state,
            executable_digest: executable_digest.to_owned(),
        },
    )
}

fn write_stopping_metadata(path: &Path, executable_digest: &str) -> anyhow::Result<()> {
    let metadata = ServerMetadata {
        version: METADATA_VERSION,
        state: ServerState::Stopping,
        executable_digest: executable_digest.to_owned(),
    };
    write_atomic(path, &metadata).or_else(|atomic_error| {
        let bytes = serde_json::to_vec_pretty(&metadata)?;
        fs::write(path, bytes).with_context(|| {
            format!("publishing sandbox stopping state after atomic write failed: {atomic_error:#}")
        })
    })
}
fn read_server_metadata(path: &Path) -> anyhow::Result<Option<ServerMetadata>> {
    match fs::read_to_string(path) {
        Ok(contents) => {
            let metadata: ServerMetadata =
                serde_json::from_str(&contents).context("parsing sandbox server metadata")?;
            if metadata.version != METADATA_VERSION {
                bail!(
                    "unsupported sandbox server metadata version {}",
                    metadata.version
                );
            }
            Ok(Some(metadata))
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(error.into()),
    }
}
fn is_signal_status(status: i32) -> bool {
    matches!(status, 130 | 143)
}

fn write_atomic<T: Serialize>(path: &Path, value: &T) -> anyhow::Result<()> {
    let parent = path.parent().context("metadata path has no parent")?;
    fs::create_dir_all(parent)?;
    let temporary = path.with_extension("new");
    let bytes = serde_json::to_vec_pretty(value)?;
    let mut file = File::create(&temporary)?;
    file.write_all(&bytes)?;
    file.sync_all()?;
    fs::rename(temporary, path)?;
    Ok(())
}
fn executable_digest(path: &Path) -> anyhow::Result<String> {
    let mut file =
        File::open(path).with_context(|| format!("reading executable {}", path.display()))?;
    let mut digest = Sha256::new();
    let mut buffer = [0_u8; 16 * 1024];
    loop {
        let read = file.read(&mut buffer)?;
        if read == 0 {
            break;
        }
        digest.update(&buffer[..read]);
    }
    Ok(format!("{:x}", digest.finalize()))
}

struct LockPaths {
    workspace: PathBuf,
    server: PathBuf,
    metadata: PathBuf,
}
impl LockPaths {
    fn for_name(name: &str) -> Self {
        let root = sandbox_root().join(".locks");
        Self {
            workspace: root.join(format!("{name}.workspace.lock")),
            server: root.join(format!("{name}.server.lock")),
            metadata: root.join(format!("{name}.server.json")),
        }
    }
}
fn lock_file(path: &Path) -> anyhow::Result<File> {
    fs::create_dir_all(path.parent().context("lock path has no parent")?)?;
    Ok(OpenOptions::new()
        .create(true)
        .truncate(false)
        .read(true)
        .write(true)
        .open(path)?)
}
fn lock_exclusive(path: &Path) -> anyhow::Result<File> {
    let file = lock_file(path)?;
    file.try_lock()
        .with_context(|| format!("acquiring exclusive lock {}", path.display()))?;
    Ok(file)
}
fn lock_shared(path: &Path) -> anyhow::Result<File> {
    let file = lock_file(path)?;
    file.try_lock_shared()
        .with_context(|| format!("acquiring shared lock {}", path.display()))?;
    Ok(file)
}
fn try_runtime_lock(path: &Path) -> anyhow::Result<File> {
    let file = lock_file(path)?;
    file.try_lock()
        .map_err(|_| anyhow::anyhow!("sandbox runtime is held by an external server"))?;
    Ok(file)
}

fn print_session(session: &HostServerSession, profile: SandboxProfile) {
    println!("Sandbox URL: {}", session.base_url);
    if profile != SandboxProfile::Empty {
        println!("Sandbox credentials: user / jaunder-dev; operator / jaunder-dev");
    }
}
fn lock_exclusive_blocking(path: &Path) -> anyhow::Result<File> {
    let file = lock_file(path)?;
    loop {
        match file.try_lock() {
            Ok(()) => return Ok(file),
            Err(std::fs::TryLockError::WouldBlock) => {
                sleep(Duration::from_millis(25));
            }
            Err(std::fs::TryLockError::Error(error)) => {
                return Err(error)
                    .with_context(|| format!("acquiring exclusive lock {}", path.display()));
            }
        }
    }
}

fn record(result: &mut CommandResult, step: &'static str, error: anyhow::Error) {
    result.push(StepResult::fail(step).detail(format!("{error:#}")));
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn name_grammar_is_one_safe_component() {
        assert!(validate_name("demo_2").is_ok());
        for invalid in ["", ".", "..", "../demo", "Demo", "-demo", "demo/name"] {
            assert!(validate_name(invalid).is_err(), "{invalid}");
        }
    }

    #[test]
    fn command_mode_requires_named_operational_workspace() {
        let command = vec!["site-config".to_owned()];
        assert!(validate(None, None, false, &command).is_err());
        assert!(validate(Some("demo"), Some(SandboxProfile::Empty), false, &command).is_err());
        assert!(matches!(
            validate(Some("demo"), None, false, &command),
            Ok(Mode::Command(name)) if name == "demo"
        ));
    }

    #[test]
    fn storage_replacement_selectors_are_rejected_in_both_forms() {
        for argument in [
            "--db",
            "--db=sqlite:other",
            "--storage-path",
            "--storage-path=other",
        ] {
            assert!(
                validate_operational_command(&["site-config".to_owned(), argument.to_owned()])
                    .is_err(),
                "{argument}"
            );
        }
        assert!(validate_operational_command(&["serve".to_owned()]).is_err());
        assert!(validate_operational_command(&["site-config".to_owned()]).is_ok());
    }

    #[test]
    fn profile_metadata_is_explicit_and_versioned() {
        let encoded = serde_json::to_value(ProfileMetadata {
            version: METADATA_VERSION,
            profile: SandboxProfile::Demo,
        })
        .expect("serialize metadata");
        assert_eq!(encoded["version"], METADATA_VERSION);
        assert_eq!(encoded["profile"], "demo");
    }

    #[test]
    fn mismatched_managed_generation_is_rejected_before_command_execution() {
        let temp = tempfile::tempdir().expect("temporary control root");
        let workspace = temp.path().join("workspace");
        fs::create_dir(&workspace).expect("workspace");
        write_profile_metadata(&workspace, SandboxProfile::Empty).expect("profile metadata");
        let locks = LockPaths {
            workspace: temp.path().join("workspace.lock"),
            server: temp.path().join("server.lock"),
            metadata: temp.path().join("server.json"),
        };
        let server = lock_file(&locks.server).expect("server lease");
        server.try_lock().expect("hold server lease");
        write_server_metadata(&locks.metadata, ServerState::Ready, "old-digest")
            .expect("ready metadata");

        let error = command_admission("unused", &workspace, &locks, "new-digest")
            .expect_err("mismatched generation must fail");

        assert!(error.to_string().contains("different executable"));
    }

    #[test]
    fn external_runtime_holder_is_rejected() {
        let temp = tempfile::tempdir().expect("temporary runtime root");
        let path = temp.path().join("runtime.lock");
        let held = lock_file(&path).expect("runtime lock");
        held.try_lock().expect("hold runtime lock");

        let error = try_runtime_lock(&path).expect_err("external runtime holder must fail");

        assert!(error.to_string().contains("external server"));
    }

    #[test]
    fn exact_exit_status_overrides_binary_result_status() {
        let mut result = CommandResult::new("sandbox");
        result.exit_override = Some(37);
        assert_eq!(result.exit_code(), 37);
    }
}
