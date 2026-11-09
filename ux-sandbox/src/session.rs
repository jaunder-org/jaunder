//! Application policy around `ProcessKit` group stop and direct-child reaping.
use crate::cli::Profile;
use anyhow::{Context, bail};
use processkit::ProcessGroup;
use serde::Deserialize;
use std::{
    io,
    net::{IpAddr, SocketAddr},
    path::Path,
    process::ExitStatus,
    time::Duration,
};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::TcpStream,
    process::Command,
    signal::unix::{SignalKind, signal},
    time::sleep,
};

const GRACE: Duration = Duration::from_secs(10);

pub struct Signals {
    interrupt: tokio::signal::unix::Signal,
    terminate: tokio::signal::unix::Signal,
    initiating: Option<i32>,
}
impl Signals {
    pub fn new() -> anyhow::Result<Self> {
        Ok(Self {
            interrupt: signal(SignalKind::interrupt())?,
            terminate: signal(SignalKind::terminate())?,
            initiating: None,
        })
    }
    pub fn status(&self) -> Option<i32> {
        self.initiating
    }
    pub async fn next(&mut self) -> i32 {
        let code = tokio::select! {
            _ = self.interrupt.recv() => 130,
            _ = self.terminate.recv() => 143,
        };
        self.initiating.get_or_insert(code);
        code
    }
}

pub async fn run(
    command: Command,
    runtime: Option<(&Path, Profile)>,
    signals: &mut Signals,
) -> anyhow::Result<i32> {
    run_owned(
        command,
        runtime,
        signals,
        None,
        std::process::Stdio::inherit(),
    )
    .await
}

/// Keep machine-readable selection output off the operational command's stdout.
/// A file avoids pipe backpressure while the existing owner waits and drains.
pub async fn output(command: Command, signals: &mut Signals) -> anyhow::Result<(i32, Vec<u8>)> {
    let output = tempfile::NamedTempFile::new()?;
    let stdout = std::process::Stdio::from(output.reopen()?);
    let code = run_owned(command, None, signals, None, stdout).await?;
    Ok((code, std::fs::read(output.path())?))
}

pub async fn named_server(
    command: Command,
    runtime: &Path,
    profile: Profile,
    signals: &mut Signals,
    server: &mut crate::workspace::Server,
) -> anyhow::Result<i32> {
    run_owned(
        command,
        Some((runtime, profile)),
        signals,
        Some(server),
        std::process::Stdio::inherit(),
    )
    .await
}

async fn run_owned(
    mut command: Command,
    runtime: Option<(&Path, Profile)>,
    signals: &mut Signals,
    mut server: Option<&mut crate::workspace::Server>,
    stdout: std::process::Stdio,
) -> anyhow::Result<i32> {
    if let Some(code) = signals.initiating {
        return Ok(code);
    }
    command
        .stdin(std::process::Stdio::inherit())
        .stdout(stdout)
        .stderr(std::process::Stdio::inherit());
    let group = ProcessGroup::new()?;
    let mut child = group.spawn(command)?;
    let mut observed = async {
        let pid = child.id().context("spawned child has no pid")?;
        let readiness = async {
            match runtime {
                Some((path, _)) => ready(path, pid).await,
                None => std::future::pending().await,
            }
        };
        tokio::pin!(readiness);
        let mut reported = false;
        loop {
            tokio::select! {
                biased;
                code = signals.next() => return Ok(code),
                status = child.wait() => return Ok(exit_code(status.context("waiting for owned child")?)),
                result = &mut readiness, if !reported => {
                    let address = result?;
                    if let Some(server) = server.as_deref_mut() { server.ready()?; }
                    println!("Sandbox URL: http://{address}");
                    if runtime.is_some_and(|(_, profile)| profile != Profile::Empty) {
                        println!("Sandbox credentials: user / jaunder-dev; operator / jaunder-dev");
                    }
                    reported = true;
                }
            }
        }
    }.await;
    // Close admission before drain; a secondary publication failure must not
    // replace the application error that already initiated cleanup.
    if let Some(server) = server
        && let Err(error) = server.stopping()
    {
        if observed.is_ok() {
            observed = Err(error);
        } else {
            eprintln!("sandbox stopping publication also failed: {error:#}");
        }
    }
    let grace = if observed.is_ok() {
        GRACE
    } else {
        Duration::ZERO
    };
    if let Err(error) = &observed {
        eprintln!("sandbox operation failed; finalizing owned processes: {error:#}");
    }
    let stopped = async {
        tokio::select! {
            biased;
            () = async {
                loop {
                    // A first signal arriving after natural child exit joins
                    // the drain. Only a subsequent INT requests early force.
                    let initiated = signals.status().is_some();
                    if signals.next().await == 130 && initiated { break; }
                }
            } => group.stop(Duration::ZERO, true).await,
            report = group.stop(grace, true) => report,
        }
    };
    // ProcessKit owns group signaling/grace/escalation. Wait concurrently:
    // an unreaped direct child can otherwise keep the group looking alive.
    let (mut stopped, waited) = tokio::join!(stopped, child.wait());
    // A forced stop can snapshot membership before concurrent reaping finishes.
    // Give the library one bounded drain observation after force/reap, not a
    // second instantaneous snapshot or a restarted graceful allowance.
    if waited.is_ok()
        && stopped
            .as_ref()
            .is_ok_and(|report| report.members_after() != Some(0))
    {
        stopped = observe_forced_drain(&group, signals).await;
    }
    confirm_terminal(&stopped, &waited).await;
    observed.map(|code| signals.initiating.unwrap_or(code))
}

/// Application guards may leave custody only after both terminal observations.
async fn confirm_terminal(
    stopped: &processkit::Result<processkit::ShutdownReport>,
    waited: &io::Result<ExitStatus>,
) {
    if !stopped
        .as_ref()
        .is_ok_and(|report| report.members_after() == Some(0))
        || waited.is_err()
    {
        eprintln!(
            "sandbox teardown unconfirmed; retaining group and workspace exclusion: stop={stopped:?}; wait={waited:?}"
        );
        // An unconfirmed outcome is not permission to release app
        // guards. Keep custody without rebuilding a polling/retry supervisor.
        std::future::pending::<()>().await;
    }
}

async fn observe_forced_drain(
    group: &ProcessGroup,
    signals: &mut Signals,
) -> processkit::Result<processkit::ShutdownReport> {
    let observation = group.stop(Duration::from_secs(5), false);
    tokio::pin!(observation);
    loop {
        tokio::select! {
            biased;
            _ = signals.next() => {},
            report = &mut observation => return report,
        }
    }
}

fn exit_code(status: ExitStatus) -> i32 {
    use std::os::unix::process::ExitStatusExt;
    status
        .code()
        .unwrap_or_else(|| 128 + status.signal().unwrap_or(1))
}

#[derive(Deserialize)]
struct RuntimeInfo {
    ip: IpAddr,
    port: u16,
    pid: u32,
    start_time: u64,
}

async fn ready(path: &Path, pid: u32) -> anyhow::Result<SocketAddr> {
    let identity = start_time(pid).context("reading spawned process identity")?;
    tokio::time::timeout(Duration::from_secs(30), async {
        loop {
            let bytes = match tokio::fs::read(path).await {
                Ok(bytes) => Some(bytes),
                Err(error) if error.kind() == io::ErrorKind::NotFound => None,
                Err(error) => {
                    return Err(error)
                        .with_context(|| format!("reading runtime-info {}", path.display()));
                }
            };
            if let Some(bytes) = bytes
                && let Ok(info) = serde_json::from_slice::<RuntimeInfo>(&bytes)
                && info.pid == pid
                && info.start_time == identity
                && info.ip.is_loopback()
                && info.port != 0
            {
                let address = SocketAddr::new(info.ip, info.port);
                if tokio::time::timeout(Duration::from_secs(2), http_ready(address))
                    .await
                    .is_ok_and(|r| r.is_ok())
                {
                    return Ok(address);
                }
            }
            sleep(Duration::from_millis(100)).await;
        }
    })
    .await
    .context("runtime identity/HTTP readiness exceeded 30 seconds")?
}

/// A live command validates legacy runtime-info against the actual writer before storage access.
pub async fn live_ready(path: &Path) -> anyhow::Result<()> {
    let info: RuntimeInfo = serde_json::from_slice(
        &tokio::fs::read(path)
            .await
            .with_context(|| format!("reading live runtime-info {}", path.display()))?,
    )?;
    anyhow::ensure!(
        info.ip.is_loopback() && info.port != 0 && start_time(info.pid)? == info.start_time,
        "live sandbox runtime identity/address mismatch"
    );
    tokio::time::timeout(
        Duration::from_secs(2),
        http_ready(SocketAddr::new(info.ip, info.port)),
    )
    .await
    .context("live sandbox HTTP readiness timed out")??;
    Ok(())
}

// processkit returns Linux clock ticks and macOS proc_pidinfo tokens, matching Jaunder's schema.
fn start_time(pid: u32) -> anyhow::Result<u64> {
    processkit::process_info(pid)
        .context("looking up owned process")?
        .context("owned process disappeared")?
        .start_time()
        .context("process lookup omitted start-time token")
}

async fn http_ready(address: SocketAddr) -> anyhow::Result<()> {
    let mut stream = TcpStream::connect(address).await?;
    stream
        .write_all(b"GET / HTTP/1.0\r\nHost: localhost\r\nConnection: close\r\n\r\n")
        .await?;
    let mut line = Vec::new();
    while line.len() < 1024 {
        let byte = stream.read_u8().await?;
        line.push(byte);
        if byte == b'\n' {
            break;
        }
    }
    if !line.starts_with(b"HTTP/1.1 200 ") && !line.starts_with(b"HTTP/1.0 200 ") {
        bail!("HTTP readiness did not return 200")
    }
    Ok(())
}

#[cfg(test)]
#[path = "session_tests.rs"]
mod tests;
