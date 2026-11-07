//! Direct async ownership: every observed exit/error is finalized before returning.
use anyhow::{Context, bail};
use processkit::{ProcessGroup, Signal};
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
    process::{Child, Command},
    signal::unix::{SignalKind, signal},
    time::{Instant, sleep},
};

const GRACE: Duration = Duration::from_secs(10);

pub struct Signals {
    interrupt: tokio::signal::unix::Signal,
    terminate: tokio::signal::unix::Signal,
    initiating: Option<i32>,
    received: usize,
}
impl Signals {
    pub fn new() -> anyhow::Result<Self> {
        Ok(Self {
            interrupt: signal(SignalKind::interrupt())?,
            terminate: signal(SignalKind::terminate())?,
            initiating: None,
            received: 0,
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
        self.received = self.received.saturating_add(1);
        code
    }
}

/// Custody stays in this stack frame even when an observation or teardown operation fails.
struct OwnedProcess {
    group: ProcessGroup,
    child: Child,
    reaped: bool,
    #[cfg(test)]
    failure: Option<tests::Operation>,
}

pub async fn run(
    command: Command,
    runtime: Option<&Path>,
    signals: &mut Signals,
) -> anyhow::Result<i32> {
    run_owned(command, runtime, signals, None).await
}

pub async fn named_server(
    command: Command,
    runtime: &Path,
    signals: &mut Signals,
    server: &mut crate::workspace::Server,
) -> anyhow::Result<i32> {
    run_owned(command, Some(runtime), signals, Some(server)).await
}

async fn run_owned(
    mut command: Command,
    runtime: Option<&Path>,
    signals: &mut Signals,
    mut server: Option<&mut crate::workspace::Server>,
) -> anyhow::Result<i32> {
    if let Some(code) = signals.initiating {
        return Ok(code);
    }
    command
        .stdin(std::process::Stdio::inherit())
        .stdout(std::process::Stdio::inherit())
        .stderr(std::process::Stdio::inherit());
    let group = ProcessGroup::new()?;
    let child = group.spawn(command)?;
    let mut process = OwnedProcess {
        group,
        child,
        reaped: false,
        #[cfg(test)]
        failure: None,
    };
    let mut observed = process
        .observe(runtime, signals, server.as_deref_mut())
        .await;
    // Admission closes before the owned child begins drain, even on unexpected exit.
    if let Some(server) = server
        && let Err(error) = server.stopping()
    {
        if observed.is_ok() {
            observed = Err(error);
        } else {
            eprintln!("sandbox stopping publication also failed: {error:#}");
        }
    }
    process.finish(observed, signals).await
}

impl OwnedProcess {
    async fn wait(&mut self) -> anyhow::Result<ExitStatus> {
        #[cfg(test)]
        self.fail_once(tests::Operation::Wait)?;
        let status = self.child.wait().await.context("waiting for owned child")?;
        self.reaped = true;
        Ok(status)
    }

    async fn observe(
        &mut self,
        runtime: Option<&Path>,
        signals: &mut Signals,
        mut server: Option<&mut crate::workspace::Server>,
    ) -> anyhow::Result<i32> {
        let pid = self.child.id().context("spawned child has no pid")?;
        let readiness = async {
            match runtime {
                Some(path) => ready(path, pid).await,
                None => std::future::pending().await,
            }
        };
        tokio::pin!(readiness);
        let mut reported = false;
        loop {
            tokio::select! {
                biased;
                code = signals.next() => return Ok(code),
                status = self.wait() => return Ok(exit_code(status?)),
                result = &mut readiness, if !reported => {
                    let address = result?;
                    if let Some(server) = server.as_deref_mut() { server.ready()?; }
                    println!("Sandbox URL: http://{address}");
                    reported = true;
                }
            }
        }
    }

    async fn finish(
        &mut self,
        observed: anyhow::Result<i32>,
        signals: &mut Signals,
    ) -> anyhow::Result<i32> {
        // Errors are data until custody is discharged, never an early-return shortcut.
        let mut result = observed;
        if let Err(error) = &result {
            eprintln!("sandbox operation failed; finalizing owned processes: {error:#}");
            self.force_and_confirm().await;
            return result;
        }
        if let Err(error) = self.drain(signals).await {
            eprintln!("sandbox drain failed; finalizing owned processes: {error:#}");
            result = Err(error);
            self.force_and_confirm().await;
        }
        result.map(|code| signals.initiating.unwrap_or(code))
    }

    async fn drain(&mut self, signals: &mut Signals) -> anyhow::Result<()> {
        #[cfg(test)]
        self.fail_once(tests::Operation::Signal)?;
        self.group
            .signal(Signal::Term)
            .context("signaling owned group")?;
        let deadline = sleep(GRACE);
        tokio::pin!(deadline);
        let mut forced = false;
        let mut confirmation_deadline = Instant::now() + GRACE + Duration::from_secs(5);
        loop {
            #[cfg(test)]
            if self.failure == Some(tests::Operation::Confirmation) {
                self.failure = None;
                confirmation_deadline = Instant::now();
            }
            if self.reaped {
                #[cfg(test)]
                self.fail_once(tests::Operation::Members)?;
                if self
                    .group
                    .members()
                    .context("reading owned group membership")?
                    .is_empty()
                {
                    return Ok(());
                }
            }
            if Instant::now() >= confirmation_deadline {
                bail!("owned group terminal confirmation timed out")
            }
            tokio::select! {
                biased;
                code = signals.next() => {
                    if code == 130 && signals.received > 1 && !forced {
                        self.kill()?;
                        forced = true;
                        confirmation_deadline = Instant::now() + Duration::from_secs(5);
                    }
                }
                () = &mut deadline, if !forced => {
                    self.kill()?;
                    forced = true;
                    confirmation_deadline = Instant::now() + Duration::from_secs(5);
                }
                status = self.wait(), if !self.reaped => { status?; }
                () = sleep(Duration::from_millis(25)) => {}
            }
        }
    }

    fn kill(&mut self) -> anyhow::Result<()> {
        #[cfg(test)]
        self.fail_once(tests::Operation::Kill)?;
        self.group.kill_all().context("forcing owned group")
    }

    async fn force_and_confirm(&mut self) {
        // stop's report is observational: neither a failed read nor a surviving member is zero.
        // Persistent OS refusal cannot be solved here. Fail closed with custody/locks retained,
        // report the unresolved operation, and retry without spinning; never turn a timeout into release.
        let mut reported = false;
        loop {
            let stopped = self.stop().await;
            let waited = if self.reaped {
                Ok(())
            } else {
                match tokio::time::timeout(Duration::from_millis(500), self.wait()).await {
                    Ok(result) => result.map(|_| ()),
                    Err(error) => Err(error).context("owned child reap remains pending"),
                }
            };
            match (stopped, waited) {
                (Ok(report), Ok(())) if report.members_after() == Some(0) => return,
                (stop, wait) => {
                    if !reported {
                        eprintln!(
                            "sandbox teardown unresolved; retaining owned processes and exclusion: stop={stop:?}; wait={wait:?}"
                        );
                        reported = true;
                    }
                }
            }
            sleep(Duration::from_millis(100)).await;
        }
    }

    async fn stop(&mut self) -> anyhow::Result<processkit::ShutdownReport> {
        #[cfg(test)]
        if self.failure == Some(tests::Operation::Stop) {
            self.failure = None;
            return Err(io::Error::other("one-shot stop failure").into());
        }
        self.group
            .stop(Duration::ZERO, true)
            .await
            .context("confirming forced owned group stop")
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
