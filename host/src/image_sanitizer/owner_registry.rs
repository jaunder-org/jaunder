//! Bounded, runtime-independent supervisor ownership. A final adapter drop joins
//! every admitted owner, so returning from main cannot race child cleanup.
//!
//! A slot is reusable only after its owner releases capacity, and its predecessor
//! is joined before another thread occupies that slot. Owners carry readiness
//! cells, never this registry, and joining never holds its mutex.

use std::any::Any;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, mpsc};
use std::thread::JoinHandle;

use thiserror::Error;
use tokio::sync::OwnedSemaphorePermit;

use super::{ACTIVE_JOBS, report_private_failure};

#[derive(Debug, Error)]
pub enum OwnerFailure {
    #[error("image supervisor thread creation failed")]
    Spawn(#[source] std::io::Error),
    #[error("image supervisor start channel failed")]
    StartChannel(#[source] mpsc::SendError<()>),
    #[error("image supervisor start channel closed")]
    Receive(#[source] mpsc::RecvError),
    #[error("image supervisor panicked")]
    Panic(#[source] PanicFailure),
    #[error("image supervisor registry was poisoned")]
    Poisoned(#[source] std::sync::PoisonError<()>),
    #[error("image supervisor capacity ownership is inconsistent")]
    Capacity,
}

/// Preserve the panic payload's type, never its potentially private message.
#[derive(Debug, Error)]
#[error("image supervisor panic payload ({kind:?})")]
pub struct PanicFailure {
    pub kind: PanicKind,
}
#[derive(Debug)]
pub enum PanicKind {
    String,
    StaticString,
    Other,
}
impl From<&(dyn Any + Send)> for PanicFailure {
    fn from(payload: &(dyn Any + Send)) -> Self {
        Self {
            kind: if payload.is::<String>() {
                PanicKind::String
            } else if payload.is::<&'static str>() {
                PanicKind::StaticString
            } else {
                PanicKind::Other
            },
        }
    }
}

struct Entry {
    handle: JoinHandle<()>,
    done: Arc<AtomicBool>,
}
enum Slot {
    Vacant,
    Reserved,
    Running(Entry),
}
type Slots = [Slot; ACTIVE_JOBS];

pub(super) struct Owners {
    slots: Mutex<Slots>,
}
impl Owners {
    pub(super) fn new() -> Self {
        Self {
            slots: Mutex::new(std::array::from_fn(|_| Slot::Vacant)),
        }
    }
    fn lock(&self) -> MutexGuard<'_, Slots> {
        match self.slots.lock() {
            Ok(slots) => slots,
            Err(poisoned) => {
                // Cleanup must still own every handle. Project only the poison
                // classification, not the borrowed registry contents.
                report_private_failure(
                    &OwnerFailure::Poisoned(std::sync::PoisonError::new(())),
                    "image owner registry recovery",
                );
                poisoned.into_inner()
            }
        }
    }
    fn reserve(&self) -> Result<Reservation<'_>, OwnerFailure> {
        let mut slots = self.lock();
        let index = slots
            .iter()
            .position(|slot| match slot {
                Slot::Vacant => true,
                Slot::Reserved => false,
                Slot::Running(entry) => entry.done.load(Ordering::Acquire),
            })
            .ok_or(OwnerFailure::Capacity)?;
        let old = std::mem::replace(&mut slots[index], Slot::Reserved);
        drop(slots);
        if let Slot::Running(entry) = old {
            join(entry.handle);
        }
        Ok(Reservation {
            owners: self,
            index,
            committed: false,
        })
    }
    pub(super) fn spawn(
        &self,
        work: impl FnOnce(Arc<AtomicBool>) + Send + 'static,
    ) -> Result<(), OwnerFailure> {
        let mut reservation = self.reserve()?;
        let done = Arc::new(AtomicBool::new(false));
        let worker_done = done.clone();
        // Publish the handle before the owner can release its permit. This gate
        // is synchronous and never needs the Tokio executor to make progress.
        let (start, started) = mpsc::channel();
        let handle = std::thread::Builder::new()
            .name("image-supervisor".into())
            .spawn(move || {
                let _finished = Finished(worker_done.clone());
                match started.recv() {
                    Ok(()) => work(worker_done),
                    Err(error) => report_private_failure(
                        &OwnerFailure::Receive(error),
                        "image owner admission failed",
                    ),
                }
            })
            .map_err(OwnerFailure::Spawn)?;
        reservation.owners.lock()[reservation.index] = Slot::Running(Entry { handle, done });
        reservation.committed = true;
        start.send(()).map_err(OwnerFailure::StartChannel)
    }
}
impl Drop for Owners {
    fn drop(&mut self) {
        let slots = match self.slots.get_mut() {
            Ok(slots) => slots,
            Err(poisoned) => {
                report_private_failure(
                    &OwnerFailure::Poisoned(std::sync::PoisonError::new(())),
                    "image owner final-drop recovery",
                );
                poisoned.into_inner()
            }
        };
        // Exclusive access is not a mutex guard: owners never hold the registry,
        // and all borrowed admission reservations have already completed.
        for slot in slots {
            if let Slot::Running(entry) = std::mem::replace(slot, Slot::Vacant) {
                join(entry.handle);
            }
        }
    }
}
fn join(handle: JoinHandle<()>) {
    if let Err(payload) = handle.join() {
        report_private_failure(
            &OwnerFailure::Panic(payload.as_ref().into()),
            "image owner join failed",
        );
    }
}
struct Reservation<'a> {
    owners: &'a Owners,
    index: usize,
    committed: bool,
}
impl Drop for Reservation<'_> {
    fn drop(&mut self) {
        if !self.committed {
            self.owners.lock()[self.index] = Slot::Vacant;
        }
    }
}
struct Finished(Arc<AtomicBool>);
impl Drop for Finished {
    fn drop(&mut self) {
        self.0.store(true, Ordering::Release);
    }
}

/// Capacity is released only after readiness is visible. Unconfirmed reaping
/// retains the permit even during unwinding; its private spool is retained too.
pub(super) struct PermitOwner {
    permit: Option<OwnedSemaphorePermit>,
    done: Arc<AtomicBool>,
    reaped: Arc<AtomicBool>,
}
impl PermitOwner {
    pub(super) fn new(
        permit: OwnedSemaphorePermit,
        done: Arc<AtomicBool>,
        reaped: Arc<AtomicBool>,
    ) -> Self {
        Self {
            permit: Some(permit),
            done,
            reaped,
        }
    }
}
impl Drop for PermitOwner {
    fn drop(&mut self) {
        if self.reaped.load(Ordering::Acquire) {
            self.done.store(true, Ordering::Release);
        } else if let Some(permit) = self.permit.take() {
            std::mem::forget(permit);
        }
    }
}

#[cfg(test)]
mod tests {
    use std::sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
        mpsc,
    };
    use std::time::{Duration, Instant};
    use tokio::sync::Semaphore;

    use super::{OwnerFailure, Owners, PanicFailure, PermitOwner, Slot};

    fn wait_for(mut ready: impl FnMut() -> bool) {
        let deadline = Instant::now() + Duration::from_secs(5);
        while !ready() {
            assert!(Instant::now() < deadline, "owner transition must complete");
            std::thread::sleep(Duration::from_millis(1));
        }
    }

    #[test]
    fn last_clone_joins_owners_but_an_earlier_clone_does_not() {
        let owners = Arc::new(Owners::new());
        let last = owners.clone();
        let (started, running) = mpsc::channel();
        let (stop, stopped) = mpsc::channel();
        owners
            .spawn(move |_| {
                started.send(()).expect("started");
                stopped.recv().expect("independent owner stop");
            })
            .expect("owner");
        running.recv().expect("live owner");
        drop(owners);
        let (dropping, begun) = mpsc::channel();
        let (finished, joined) = mpsc::channel();
        let dropper = std::thread::spawn(move || {
            dropping.send(()).expect("final drop starts");
            drop(last);
            finished.send(()).expect("final drop completed");
        });
        begun.recv().expect("drop begins");
        assert!(matches!(joined.try_recv(), Err(mpsc::TryRecvError::Empty)));
        stop.send(())
            .expect("owner can progress without Tokio or registry");
        joined.recv().expect("drop waits for join");
        dropper.join().expect("dropper");
    }

    #[test]
    fn ready_predecessors_are_joined_before_reuse_and_rapid_admission_is_bounded() {
        let owners = Arc::new(Owners::new());
        let capacity = Arc::new(Semaphore::new(2));
        let mut tails = Vec::new();
        let (ready, released) = mpsc::channel();
        for _ in 0..2 {
            let permit = capacity.clone().try_acquire_owned().expect("capacity");
            let (end, tail) = mpsc::channel();
            tails.push(end);
            let ready = ready.clone();
            owners
                .spawn(move |done| {
                    let permit = PermitOwner::new(permit, done, Arc::new(AtomicBool::new(true)));
                    drop(permit);
                    ready.send(()).expect("permit released");
                    tail.recv().expect("thread tail can finish independently");
                })
                .expect("predecessor");
        }
        released.recv().expect("first ready predecessor");
        released.recv().expect("second ready predecessor");
        assert_eq!(capacity.available_permits(), 2);
        let (started, running) = mpsc::channel();
        let mut admissions = Vec::new();
        for _ in 0..2 {
            let owners = owners.clone();
            let permit = capacity
                .clone()
                .try_acquire_owned()
                .expect("released permit");
            let started = started.clone();
            admissions.push(std::thread::spawn(move || {
                owners
                    .spawn(move |done| {
                        let _permit =
                            PermitOwner::new(permit, done, Arc::new(AtomicBool::new(true)));
                        started.send(()).expect("successor started");
                    })
                    .expect("successor");
            }));
        }
        wait_for(|| {
            owners
                .lock()
                .iter()
                .all(|slot| matches!(slot, Slot::Reserved))
        });
        assert!(
            matches!(running.try_recv(), Err(mpsc::TryRecvError::Empty)),
            "capacity alone must not allow a third retained owner thread"
        );
        for end in tails {
            end.send(()).expect("finish predecessor");
        }
        for admission in admissions {
            admission.join().expect("admission finished");
        }
        running.recv().expect("first successor");
        running.recv().expect("second successor");
        for _ in 0..64 {
            wait_for(|| capacity.available_permits() > 0);
            let permit = capacity
                .clone()
                .try_acquire_owned()
                .expect("rapid capacity");
            owners
                .spawn(move |done| {
                    let _permit = PermitOwner::new(permit, done, Arc::new(AtomicBool::new(true)));
                })
                .expect("rapid admission");
            assert!(
                owners
                    .lock()
                    .iter()
                    .filter(|slot| !matches!(slot, Slot::Vacant))
                    .count()
                    <= 2
            );
        }
        drop(owners);
        assert_eq!(capacity.available_permits(), 2);
    }

    #[test]
    fn poison_and_panic_are_projected_without_messages_and_cleanup_still_joins() {
        use std::error::Error;
        let marker = "owner@example.test private metadata";
        let payload: Box<dyn std::any::Any + Send> = Box::new(marker.to_owned());
        let error = OwnerFailure::Panic(PanicFailure::from(payload.as_ref()));
        assert!(!format!("{error:?} {error}").contains(marker));
        assert!(
            error
                .source()
                .expect("typed panic source")
                .downcast_ref::<PanicFailure>()
                .is_some()
        );
        let error = OwnerFailure::Poisoned(std::sync::PoisonError::new(()));
        assert!(
            error
                .source()
                .expect("typed poison source")
                .downcast_ref::<std::sync::PoisonError<()>>()
                .is_some()
        );
        let owners = Owners::new();
        let _panic = std::panic::catch_unwind(|| {
            let _slots = owners.slots.lock().expect("registry lock");
            panic!("intentional registry poison");
        })
        .expect_err("poison the registry");
        owners
            .spawn(|_| panic!("intentional owner panic"))
            .expect("recover to retain the owner handle");
        drop(owners);
    }

    #[test]
    fn unconfirmed_reap_never_releases_capacity() {
        let capacity = Arc::new(Semaphore::new(2));
        let done = Arc::new(AtomicBool::new(false));
        let permit = capacity.clone().try_acquire_owned().expect("permit");
        drop(PermitOwner::new(
            permit,
            done.clone(),
            Arc::new(AtomicBool::new(false)),
        ));
        assert_eq!(capacity.available_permits(), 1);
        assert!(!done.load(Ordering::Acquire));
    }
}
