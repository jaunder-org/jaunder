//! Shared signal handling for owned host consumers.

use std::time::Duration;

use anyhow::Context;
use tokio::runtime::Builder;

use super::process::Process;

pub(super) enum ServerExit {
    Signal(i32),
    Unexpected,
}

pub(super) struct ServerSignals {
    runtime: tokio::runtime::Runtime,
    interrupt: tokio::signal::unix::Signal,
    terminate: tokio::signal::unix::Signal,
    pending: Option<i32>,
}

impl ServerSignals {
    pub(super) fn install() -> anyhow::Result<Self> {
        let runtime = Builder::new_current_thread()
            .enable_time()
            .enable_io()
            .build()
            .context("creating host signal runtime")?;
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
            pending: None,
        })
    }

    pub(super) fn take_pending(&mut self) -> Option<i32> {
        self.pending.take().or_else(|| self.receive_pending())
    }

    fn receive_pending(&mut self) -> Option<i32> {
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

    pub(super) fn wait_until_stopped(&mut self, mut stopped: impl FnMut() -> bool) -> ServerExit {
        if let Some(code) = self.take_pending() {
            return ServerExit::Signal(code);
        }
        self.runtime.block_on(async {
            loop {
                if stopped() {
                    return ServerExit::Unexpected;
                }
                tokio::select! {
                    _ = tokio::time::sleep(Duration::from_millis(100)) => {},
                    _ = self.interrupt.recv() => return ServerExit::Signal(130),
                    _ = self.terminate.recv() => return ServerExit::Signal(143),
                }
            }
        })
    }

    pub(super) fn shutdown_process(&mut self, process: &mut Process) -> anyhow::Result<()> {
        process
            .shutdown_interruptible(Duration::from_secs(10), self.next_signal())
            .map(|_| ())
    }

    pub(super) async fn next_signal(&mut self) {
        self.next_signal_code().await;
    }

    pub(super) async fn next_signal_code(&mut self) -> i32 {
        tokio::select! {
            _ = self.interrupt.recv() => 130,
            _ = self.terminate.recv() => 143,
        }
    }
}
