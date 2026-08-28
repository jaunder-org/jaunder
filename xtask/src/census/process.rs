//! Owned-process lifecycle support for census collectors.
//!
//! This module is the only cleanup seam for long-lived external census tools.
//! Callers start stderr draining immediately after spawning so diagnostic output
//! cannot backpressure a child. The drain retains only a bounded prefix while it
//! continues consuming the pipe; callers may attach that retained context to a
//! primary error. Cleanup always attempts to terminate an owned child and reap
//! it, including after `try_wait` itself fails. Ancillary cleanup failures are
//! warnings so they never replace the collector's primary evidence failure.

use std::io::Read;
use std::process::{Child, ChildStderr};
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;

const STDERR_LIMIT: usize = 8 * 1024;

/// A concurrent, bounded stderr drain whose retained diagnostics are optional context.
pub(crate) struct StderrDrain {
    retained: Arc<Mutex<Vec<u8>>>,
    task: Option<JoinHandle<Result<(), std::io::Error>>>,
}

impl StderrDrain {
    /// Start consuming a child's stderr immediately, retaining at most `STDERR_LIMIT` bytes.
    pub(crate) fn start(stderr: ChildStderr) -> Self {
        let retained = Arc::new(Mutex::new(Vec::with_capacity(STDERR_LIMIT)));
        let target = Arc::clone(&retained);
        let task = std::thread::spawn(move || {
            let mut stderr = stderr;
            let mut buffer = [0; 4096];
            loop {
                let count = stderr.read(&mut buffer)?;
                if count == 0 {
                    return Ok(());
                }
                let mut retained = target
                    .lock()
                    .unwrap_or_else(|poisoned| poisoned.into_inner());
                let available = STDERR_LIMIT.saturating_sub(retained.len());
                retained.extend_from_slice(&buffer[..count.min(available)]);
            }
        });
        Self {
            retained,
            task: Some(task),
        }
    }

    /// Snapshot the currently retained diagnostics without stopping the drain.
    pub(crate) fn diagnostics(&self) -> String {
        let retained = self
            .retained
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        String::from_utf8_lossy(&retained).trim().to_owned()
    }

    /// Return retained diagnostics after joining the drain, warning if draining itself failed.
    pub(crate) fn finish(&mut self, process: &str) -> String {
        if let Some(task) = self.task.take() {
            match task.join() {
                Ok(Ok(())) => {}
                Ok(Err(error)) => {
                    eprintln!("warning: draining census {process} stderr failed: {error}")
                }
                Err(_) => eprintln!("warning: census {process} stderr drain panicked"),
            }
        }
        self.diagnostics()
    }
}

/// Terminate and reap an owned child without replacing the caller's primary error.
pub(crate) fn terminate_and_reap(child: &mut Child, process: &str) {
    match child.try_wait() {
        Ok(Some(_)) => {}
        Ok(None) => {
            if let Err(error) = child.kill() {
                eprintln!("warning: stopping census {process} failed: {error}");
            }
        }
        Err(error) => {
            eprintln!("warning: checking census {process} status failed: {error}");
            if let Err(error) = child.kill() {
                eprintln!(
                    "warning: stopping census {process} after status failure failed: {error}"
                );
            }
        }
    }
    if let Err(error) = child.wait() {
        eprintln!("warning: reaping census {process} failed: {error}");
    }
}
