//! Lifecycle guard for cron-backed server workers.

use std::future::Future;
use std::sync::{Arc, Mutex, PoisonError};

use anyhow::{Context, Result};
use tokio::sync::Notify;
use tokio_cron_scheduler::JobScheduler;

#[derive(Clone, Default)]
pub(crate) struct WorkTracker {
    state: Arc<WorkState>,
}

#[derive(Default)]
struct WorkState {
    inner: Mutex<WorkStateInner>,
    changed: Notify,
}

#[derive(Default)]
struct WorkStateInner {
    stopping: bool,
    active: usize,
}

impl WorkTracker {
    pub(crate) async fn run<F>(self, work: F)
    where
        F: Future<Output = ()>,
    {
        let Some(_active) = self.admit() else {
            return;
        };
        work.await;
    }

    pub(crate) fn stop(&self) {
        let mut inner = self
            .state
            .inner
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        inner.stopping = true;
        drop(inner);
        self.state.changed.notify_waiters();
    }

    pub(crate) async fn stopped(&self) {
        loop {
            let changed = self.state.changed.notified();
            tokio::pin!(changed);
            changed.as_mut().enable();
            if self
                .state
                .inner
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .stopping
            {
                return;
            }
            changed.await;
        }
    }

    async fn wait(&self) {
        loop {
            let changed = self.state.changed.notified();
            tokio::pin!(changed);
            changed.as_mut().enable();
            if self
                .state
                .inner
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .active
                == 0
            {
                return;
            }
            changed.await;
        }
    }

    fn admit(&self) -> Option<ActiveWork> {
        let mut inner = self
            .state
            .inner
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        if inner.stopping {
            return None;
        }
        inner.active += 1;
        Some(ActiveWork {
            state: Arc::clone(&self.state),
        })
    }
}

struct ActiveWork {
    state: Arc<WorkState>,
}

impl Drop for ActiveWork {
    fn drop(&mut self) {
        let mut inner = self
            .state
            .inner
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        inner.active -= 1;
        let idle = inner.active == 0;
        drop(inner);
        if idle {
            self.state.changed.notify_waiters();
        }
    }
}

/// Owns one scheduler and drains every admitted job during shutdown.
pub(crate) struct ScheduledWorkerGuard {
    scheduler: JobScheduler,
    tracker: WorkTracker,
}

impl ScheduledWorkerGuard {
    /// Starts `scheduler`, stopping and draining it before returning any startup
    /// failure.
    ///
    /// # Errors
    ///
    /// Returns an error when the underlying scheduler cannot start or cannot be
    /// stopped after a partial start.
    pub(crate) async fn start(mut scheduler: JobScheduler, tracker: WorkTracker) -> Result<Self> {
        if let Err(start_error) = scheduler.start().await {
            tracker.stop();
            let shutdown_result = scheduler.shutdown().await;
            tracker.wait().await;
            if let Err(shutdown_error) = shutdown_result {
                return Err(shutdown_error).context(format!(
                    "cannot stop scheduled worker after startup failed: {start_error}"
                ));
            }
            return Err(start_error).context("cannot start scheduled worker");
        }
        Ok(Self { scheduler, tracker })
    }

    pub(crate) fn stop(&self) {
        self.tracker.stop();
    }

    pub(crate) async fn shutdown(&mut self) -> Result<()> {
        self.tracker.stop();
        let result = self
            .scheduler
            .shutdown()
            .await
            .context("cannot stop scheduled worker");
        self.tracker.wait().await;
        result
    }
}

impl Drop for ScheduledWorkerGuard {
    fn drop(&mut self) {
        // Drop cannot await active work, but it must prevent detached scheduler
        // callbacks from admitting more storage work on an error path.
        self.tracker.stop();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn stop_refuses_new_work_and_waits_for_active_work() {
        let tracker = WorkTracker::default();
        let started = Arc::new(Notify::new());
        let release = Arc::new(Notify::new());
        let active_tracker = tracker.clone();
        let active_started = Arc::clone(&started);
        let active_release = Arc::clone(&release);
        let active = tokio::spawn(async move {
            active_tracker
                .run(async move {
                    active_started.notify_one();
                    active_release.notified().await;
                })
                .await;
        });
        started.notified().await;

        tracker.stop();
        let rejected = Arc::new(std::sync::atomic::AtomicBool::new(false));
        let rejected_work = Arc::clone(&rejected);
        tracker
            .clone()
            .run(async move {
                rejected_work.store(true, std::sync::atomic::Ordering::SeqCst);
            })
            .await;
        assert!(!rejected.load(std::sync::atomic::Ordering::SeqCst));

        let waiting_tracker = tracker.clone();
        let mut waiting = tokio::spawn(async move { waiting_tracker.wait().await });
        assert!(
            tokio::time::timeout(std::time::Duration::from_millis(10), &mut waiting)
                .await
                .is_err()
        );
        release.notify_one();
        active.await.unwrap();
        waiting.await.unwrap();
    }

    #[tokio::test]
    async fn stopped_waits_for_stop_signal() {
        let tracker = WorkTracker::default();
        let waiting_tracker = tracker.clone();
        let mut waiting = tokio::spawn(async move { waiting_tracker.stopped().await });
        assert!(
            tokio::time::timeout(std::time::Duration::from_millis(10), &mut waiting)
                .await
                .is_err()
        );
        tracker.stop();
        waiting.await.unwrap();
    }
}
