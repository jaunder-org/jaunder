//! Shared fixtures for the `pr` unit tests.
//!
//! A `#[cfg(test)] mod tests` is private to its own file, so builders needed by more
//! than one module cannot live in one — `decide`, `watch`, and `land` all construct
//! the same snapshots. This is the same idiom as the crate-level
//! [`crate::test_support`], scoped to `pr`.

use std::cell::RefCell;
use std::collections::VecDeque;

use super::gh::ApiError;
use super::snapshot::{
    CheckEntry, CheckState, MergeStateStatus, Mergeable, PrSnapshot, PrSource, PrState, QueueState,
    RequiredChecks, RunRef,
};
use super::watch::{Clock, WatchConfig};
use super::{PrNumber, Subject};

/// The live ruleset: two required contexts, non-strict, merge queue present.
pub fn queue_rules() -> RequiredChecks {
    RequiredChecks {
        contexts: vec!["Validate (no e2e)".into(), "e2e gate".into()],
        strict: false,
        queue_present: true,
    }
}

/// The documented ADR-0077 rollback: strict, no queue. `BEHIND` blocks again here.
pub fn strict_rules() -> RequiredChecks {
    RequiredChecks {
        contexts: vec!["Validate (no e2e)".into(), "e2e gate".into()],
        strict: true,
        queue_present: false,
    }
}

pub fn check(name: &str, state: CheckState, completed: &str) -> CheckEntry {
    CheckEntry {
        name: name.into(),
        state,
        details_url: Some("https://x/1".into()),
        started_at: Some("2026-07-30T14:00:00Z".into()),
        completed_at: (!completed.is_empty()).then(|| completed.to_string()),
    }
}

/// Both required contexts successful.
pub fn green() -> Vec<CheckEntry> {
    vec![
        check(
            "Validate (no e2e)",
            CheckState::Success,
            "2026-07-30T14:10:00Z",
        ),
        check("e2e gate", CheckState::Success, "2026-07-30T14:20:00Z"),
    ]
}

/// An open, mergeable, unarmed, unqueued PR. `head_committed_at` is fixed so the
/// ejection-recency tests have a stable anchor to compare run timestamps against.
pub fn open(checks: Vec<CheckEntry>) -> PrSnapshot {
    PrSnapshot {
        state: PrState::Open,
        merged_at: None,
        merge_commit: None,
        mergeable: Mergeable::Mergeable,
        merge_state_status: MergeStateStatus::Clean,
        auto_merge_armed: false,
        queue: QueueState {
            in_queue: false,
            position: None,
        },
        head_sha: "abc".into(),
        head_ref: "feature".into(),
        head_committed_at: "2026-07-30T13:00:00Z".into(),
        checks,
    }
}

pub fn open_pending() -> PrSnapshot {
    open(vec![
        check("Validate (no e2e)", CheckState::Pending, ""),
        check("e2e gate", CheckState::Pending, ""),
    ])
}

/// `mergeable` is `Unknown` because that is what GitHub actually returns for a merged
/// PR — which also proves the state machine reaches `Merged` before consulting it.
pub fn merged_snapshot() -> PrSnapshot {
    PrSnapshot {
        state: PrState::Merged,
        merged_at: Some("2026-07-30T15:00:00Z".into()),
        merge_commit: Some("deadbeef".into()),
        mergeable: Mergeable::Unknown,
        merge_state_status: MergeStateStatus::Unknown,
        ..open(green())
    }
}

/// Auto-merge armed but **not** yet queued.
pub fn armed_snapshot() -> PrSnapshot {
    PrSnapshot {
        auto_merge_armed: true,
        ..open(green())
    }
}

/// Queued but **not** auto-merge-armed — the direct-enqueue shape GitHub produces
/// when a green PR is merged into a live queue. Distinguishing this from a failed
/// arm is the whole reason the armed predicate is a disjunction.
pub fn queued_at(position: u64) -> PrSnapshot {
    PrSnapshot {
        queue: QueueState {
            in_queue: true,
            position: Some(position),
        },
        ..open(green())
    }
}

pub fn ejection(created_at: &str) -> RunRef {
    RunRef {
        url: "https://github.com/o/r/actions/runs/9".into(),
        created_at: created_at.into(),
        conclusion: "failure".into(),
    }
}

pub fn subject() -> Subject {
    Subject {
        owner: "o".into(),
        repo: "r".into(),
        number: PrNumber(731),
    }
}

/// A virtual clock: `sleep` advances time instead of blocking, so a test of the
/// 90-minute budget runs in microseconds.
pub struct FakeClock {
    pub now: RefCell<u64>,
}

impl Clock for FakeClock {
    fn now_unix(&self) -> u64 {
        *self.now.borrow()
    }
    fn now_rfc3339(&self) -> String {
        format!("T+{}", self.now_unix())
    }
    fn sleep_secs(&self, secs: u64) {
        *self.now.borrow_mut() += secs;
    }
}

pub fn clock() -> FakeClock {
    FakeClock {
        now: RefCell::new(0),
    }
}

pub fn cfg() -> WatchConfig {
    WatchConfig::default()
}

/// A scripted `PrSource`.
///
/// Once the script runs out it **repeats its last value forever**, so a budget-expiry
/// test can script a single snapshot and still poll all the way to the timeout.
pub struct FakeSource {
    snaps: RefCell<VecDeque<Result<PrSnapshot, ApiError>>>,
    last: RefCell<Option<Result<PrSnapshot, ApiError>>>,
    req: RequiredChecks,
    ejection: Result<Option<RunRef>, ApiError>,
}

impl FakeSource {
    pub fn new(snaps: Vec<Result<PrSnapshot, ApiError>>, req: RequiredChecks) -> Self {
        Self {
            snaps: RefCell::new(snaps.into()),
            last: RefCell::new(None),
            req,
            ejection: Ok(None),
        }
    }

    /// What the merge-group probe finds. Without this the probe path is only ever
    /// driven with `None`, so a dropped result would pass the whole suite.
    pub fn with_ejection(mut self, run: Option<RunRef>) -> Self {
        self.ejection = Ok(run);
        self
    }

    /// Make the probe itself fail, so "probe failure is a poll failure" is testable.
    pub fn with_ejection_error(mut self, err: ApiError) -> Self {
        self.ejection = Err(err);
        self
    }
}

impl PrSource for FakeSource {
    fn resolve(&self, _requested: Option<PrNumber>) -> Result<Subject, ApiError> {
        unreachable!("FakeSource is always handed a Subject directly")
    }

    fn snapshot(&self, _subject: &Subject) -> Result<PrSnapshot, ApiError> {
        if let Some(next) = self.snaps.borrow_mut().pop_front() {
            *self.last.borrow_mut() = Some(next.clone());
            return next;
        }
        self.last
            .borrow()
            .clone()
            .expect("FakeSource was scripted with at least one snapshot")
    }

    fn required_checks(&self, _subject: &Subject) -> Result<RequiredChecks, ApiError> {
        Ok(self.req.clone())
    }

    fn ejection_run(&self, _subject: &Subject) -> Result<Option<RunRef>, ApiError> {
        self.ejection.clone()
    }
}
