//! The poll loop: snapshots in, one event log and one terminal report out.
//!
//! Generic over both the source and the clock, so the whole loop — strike budget,
//! rate-limit waiting, change detection, heartbeats, the 90-minute timeout — is
//! exercised offline and instantly.
//!
//! Note the return type: [`watch`] yields a `PrReport`, never a `Result`. Every
//! terminal state, *including* the tooling itself failing, is a report — so the one
//! outcome that most needs to be legible cannot become an error that never gets
//! written down.

use std::collections::BTreeSet;

use super::decide::{self, ClassifiedFailure, OptionalFailure, Phase, Progress, Step};
use super::gh::ApiError;
use super::snapshot::{
    CheckProvider, CheckState, PrSnapshot, PrSource, PrState, RequiredChecks, RunRef,
};
use super::workflow::Requirement;
use super::{Event, EventKind, Outcome, PrReport, Subject, SubjectFailure};

pub trait Clock {
    fn now_unix(&self) -> u64;
    fn now_rfc3339(&self) -> String;
    fn sleep_secs(&self, secs: u64);
}

pub struct SystemClock;

impl Clock for SystemClock {
    fn now_unix(&self) -> u64 {
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0)
    }
    fn now_rfc3339(&self) -> String {
        format_unix_utc(self.now_unix())
    }
    fn sleep_secs(&self, secs: u64) {
        std::thread::sleep(std::time::Duration::from_secs(secs));
    }
}

/// Unix seconds → RFC 3339 UTC, so the event log carries real timestamps without
/// pulling a date crate into a tool that needs nothing else from one.
fn format_unix_utc(secs: u64) -> String {
    let days = (secs / 86_400) as i64;
    let rem = secs % 86_400;
    // Civil-from-days (Howard Hinnant's algorithm), shifted to a March-based year so
    // the leap day falls at the end and needs no special case.
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1_460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = doy - (153 * mp + 2) / 5 + 1;
    let month = if mp < 10 { mp + 3 } else { mp - 9 };
    let year = yoe + era * 400 + i64::from(month <= 2);
    format!(
        "{year:04}-{month:02}-{day:02}T{:02}:{:02}:{:02}Z",
        rem / 3_600,
        (rem % 3_600) / 60,
        rem % 60
    )
}

#[derive(Debug, Clone, Copy)]
pub struct WatchConfig {
    pub interval_secs: u64,
    pub timeout_mins: u64,
    pub once: bool,
    pub stop_at_ready: bool,
    pub heartbeat_secs: u64,
    pub max_strikes: u32,
}

impl Default for WatchConfig {
    fn default() -> Self {
        Self {
            interval_secs: 30,
            timeout_mins: 90,
            once: false,
            stop_at_ready: true,
            heartbeat_secs: 600,
            max_strikes: 5,
        }
    }
}

/// How a required context reads in the event log.
///
/// Not `{:?}` on the `Option<CheckState>`: a human watching stderr should see
/// `e2e gate: not yet reported`, not `e2e gate: None` — and "has not appeared yet" is
/// exactly the state the late-appearing aggregate check makes worth naming clearly.
fn check_state_label(snap: &PrSnapshot, name: &str) -> String {
    match decide::resolve_context(&snap.checks, name).map(|e| e.state) {
        None => "not yet reported".into(),
        Some(CheckState::Pending) => "pending".into(),
        Some(CheckState::Success) => "success".into(),
        Some(CheckState::Failure) => "failure".into(),
    }
}

/// The previous poll's emitted view, component by component.
///
/// This **is** the change-detection fingerprint, kept decomposed rather than hashed
/// into one string: emission is per changed component, so comparing component-wise is
/// what the event log actually needs. Note what it holds — phase, the required
/// contexts' states, queue membership and position, and the standing warning — and
/// what it deliberately omits: elapsed time, poll count, `updatedAt`, and any check
/// the ruleset does not require. Anything that ticks on its own would turn this from
/// a change-emitter into a per-poll emitter.
struct Rendered {
    phase: Phase,
    /// `(required context name, its resolved state)`, in **ruleset order**.
    ///
    /// Compared positionally against the previous poll's vector, which is sound only
    /// because the ruleset is fetched once and cached for the life of the watch. If
    /// that ever becomes a per-poll read, pair by name instead — otherwise a reordered
    /// or resized ruleset would silently mis-attribute states to contexts.
    checks: Vec<(String, String)>,
    queue: String,
    warn: Option<String>,
    optional_failures: Vec<OptionalFailure>,
}

impl Rendered {
    fn of(
        snap: &PrSnapshot,
        req: &RequiredChecks,
        phase: Phase,
        warn: Option<String>,
        optional_failures: Vec<OptionalFailure>,
    ) -> Self {
        Self {
            phase,
            checks: req
                .contexts
                .iter()
                .map(|name| (name.clone(), check_state_label(snap, name)))
                .collect(),
            queue: format!("{}:{:?}", snap.queue.in_queue, snap.queue.position),
            warn,
            optional_failures,
        }
    }
}

/// Everything the loop carries between polls, kept in one place so the emit logic can
/// borrow it without fighting the event sink.
struct Polled {
    required: RequiredChecks,
    snapshot: PrSnapshot,
    classification: SnapshotClassification,
}

pub(super) enum SnapshotClassification {
    Complete {
        step: Step,
        optional_failures: Vec<OptionalFailure>,
    },
    Incomplete(super::evidence::IncompleteEvidence),
}

enum FailureResolution {
    Complete(Vec<ClassifiedFailure>),
    Incomplete(super::evidence::IncompleteEvidence),
}

struct Emitter<'a> {
    events: Vec<Event>,
    sink: &'a mut dyn FnMut(&Event),
    last_event_at: u64,
}

impl Emitter<'_> {
    fn emit(&mut self, at: String, now: u64, kind: EventKind, detail: String) {
        let event = Event { at, kind, detail };
        (self.sink)(&event);
        self.events.push(event);
        self.last_event_at = now;
    }
}

/// Classify one snapshot without turning uncertain Actions ancestry into an optional
/// failure. Kept at the watch boundary because evidence acquisition is fallible while
/// `decide` remains pure.
pub(super) fn classify_snapshot<S: PrSource>(
    source: &S,
    subject: &Subject,
    snap: &PrSnapshot,
    req: &RequiredChecks,
    ejection: Option<&RunRef>,
    progress: &Progress,
) -> Result<SnapshotClassification, ApiError> {
    match classified_failures(source, subject, snap, req)? {
        FailureResolution::Complete(failures) => {
            let optional_failures = decide::optional_failures(&failures);
            Ok(SnapshotClassification::Complete {
                step: decide::classify_with_failures(snap, req, ejection, progress, &failures),
                optional_failures,
            })
        }
        FailureResolution::Incomplete(evidence) => Ok(SnapshotClassification::Incomplete(evidence)),
    }
}

fn classified_failures<S: PrSource>(
    source: &S,
    subject: &Subject,
    snap: &PrSnapshot,
    req: &RequiredChecks,
) -> Result<FailureResolution, ApiError> {
    if snap.state != PrState::Open || snap.mergeable == super::snapshot::Mergeable::Conflicting {
        return Ok(FailureResolution::Complete(Vec::new()));
    }
    // Load Actions evidence before rerun settledness can hide a failed constituent.
    // A same-run, same-named pending job is a re-run only after the immutable graph
    // proves that its runtime identity has exactly one node; otherwise it could be a
    // distinct sibling that happens to share a display name.
    let action_failures = snap
        .checks
        .iter()
        .filter(|check| {
            check.state == CheckState::Failure
                && !req.contexts.iter().any(|context| context == &check.name)
                && matches!(check.provider, CheckProvider::GitHubActions { .. })
        })
        .collect::<Vec<_>>();
    let workflow_run_ids = action_failures
        .iter()
        .filter_map(|check| match check.provider {
            CheckProvider::GitHubActions {
                workflow_run_id, ..
            } => Some(workflow_run_id),
            _ => None,
        })
        .collect::<std::collections::BTreeSet<_>>();
    let evidence = (!workflow_run_ids.is_empty())
        .then(|| source.actions_evidence(subject, snap, &workflow_run_ids))
        .transpose()?;
    if evidence
        .as_ref()
        .is_some_and(|evidence| evidence.head_sha != snap.head_sha)
    {
        return Err(ApiError::Malformed(
            "Actions evidence belongs to a different PR head".into(),
        ));
    }
    if let Some(evidence) = evidence.as_ref()
        && let Some(incomplete) = validate_action_failure_collisions(snap, req, evidence)?
    {
        return Ok(FailureResolution::Incomplete(incomplete));
    }
    let failures = decide::resolved_failures(snap)
        .into_iter()
        .filter(|check| !req.contexts.iter().any(|context| context == &check.name))
        .collect::<Vec<_>>();
    if failures.is_empty() {
        return Ok(FailureResolution::Complete(Vec::new()));
    }
    let mut classified = Vec::with_capacity(failures.len());
    for check in failures {
        let subject_failure = match check.provider {
            CheckProvider::GitHubActions {
                check_run_id,
                workflow_run_id,
            } => {
                let evidence = evidence.as_ref().ok_or_else(|| {
                    ApiError::Malformed("GitHub Actions evidence unexpectedly absent".into())
                })?;
                let requirement = match evidence.classify_check(check_run_id, &req.contexts) {
                    Ok(requirement) => requirement,
                    Err(super::evidence::ClassificationError::Incomplete(incomplete)) => {
                        return Ok(FailureResolution::Incomplete(incomplete));
                    }
                    Err(super::evidence::ClassificationError::Observation(error)) => {
                        return Err(error);
                    }
                };
                matches!(requirement, Requirement::Direct | Requirement::Transitive).then(|| {
                    SubjectFailure {
                        workflow_run_id,
                        check_run_id,
                        name: check.name.clone(),
                    }
                })
            }
            CheckProvider::StatusContext | CheckProvider::OtherCheckRun => None,
        };
        classified.push(if subject_failure.is_some() {
            ClassifiedFailure::Required {
                name: check.name.clone(),
                pointer: check.details_url.clone(),
                subject_failure,
            }
        } else {
            ClassifiedFailure::Optional {
                id: failure_identity(&snap.head_sha, check),
                name: check.name.clone(),
                pointer: check.details_url.clone(),
            }
        });
    }
    Ok(FailureResolution::Complete(classified))
}

/// Validate every same-run/display-name collision containing a failed Actions check
/// before settledness. Only a uniquely correlated graph node can make it a rerun.
fn validate_action_failure_collisions(
    snap: &PrSnapshot,
    req: &RequiredChecks,
    evidence: &super::evidence::ActionsEvidence,
) -> Result<Option<super::evidence::IncompleteEvidence>, ApiError> {
    let mut groups =
        std::collections::BTreeMap::<(u64, String), std::collections::BTreeSet<u64>>::new();
    for failed in snap.checks.iter().filter(|check| {
        check.state == CheckState::Failure
            && !req.contexts.iter().any(|context| context == &check.name)
    }) {
        let CheckProvider::GitHubActions {
            workflow_run_id, ..
        } = failed.provider
        else {
            continue;
        };
        let check_run_ids = snap
            .checks
            .iter()
            .filter_map(|check| match check.provider {
                CheckProvider::GitHubActions {
                    check_run_id,
                    workflow_run_id: candidate_run_id,
                } if candidate_run_id == workflow_run_id && check.name == failed.name => {
                    Some(check_run_id)
                }
                _ => None,
            })
            .collect();
        groups.insert((workflow_run_id, failed.name.clone()), check_run_ids);
    }
    for ((workflow_run_id, name), check_run_ids) in groups {
        match evidence.classify_current_attempt_group(
            workflow_run_id,
            &name,
            &check_run_ids,
            &req.contexts,
        ) {
            Ok(_) => {}
            Err(super::evidence::ClassificationError::Incomplete(incomplete)) => {
                return Ok(Some(incomplete));
            }
            Err(super::evidence::ClassificationError::Observation(error)) => {
                return Err(error);
            }
        }
    }
    Ok(None)
}

fn failure_identity(head_sha: &str, check: &super::snapshot::CheckEntry) -> String {
    let provider = match check.provider {
        CheckProvider::GitHubActions {
            check_run_id,
            workflow_run_id,
        } => format!("actions:{workflow_run_id}:{check_run_id}"),
        CheckProvider::StatusContext => "status-context".into(),
        CheckProvider::OtherCheckRun => "other-check-run".into(),
    };
    format!(
        "{head_sha}:{provider}:{}:{}:{}",
        check.name,
        check.started_at.as_deref().unwrap_or(""),
        check.completed_at.as_deref().unwrap_or("")
    )
}

/// Poll until the next actionable outcome, timeout, or watcher failure.
///
/// By default, a ready approval handoff ends observation even though the PR remains
/// open. Passive mode continues through ready until a terminal PR outcome.
pub fn watch<S: PrSource, C: Clock>(
    source: &S,
    clock: &C,
    subject: &Subject,
    cfg: WatchConfig,
    sink: &mut dyn FnMut(&Event),
) -> PrReport {
    watch_with_progress(source, clock, subject, cfg, Progress::default(), sink)
}

pub(super) fn watch_with_progress<S: PrSource, C: Clock>(
    source: &S,
    clock: &C,
    subject: &Subject,
    cfg: WatchConfig,
    progress: Progress,
    sink: &mut dyn FnMut(&Event),
) -> PrReport {
    watch_with_progress_and_optional_failures(
        source,
        clock,
        subject,
        cfg,
        progress,
        BTreeSet::new(),
        sink,
    )
}

pub(super) fn watch_with_progress_and_optional_failures<S: PrSource, C: Clock>(
    source: &S,
    clock: &C,
    subject: &Subject,
    cfg: WatchConfig,
    mut progress: Progress,
    mut emitted_optional_failure_ids: BTreeSet<String>,
    sink: &mut dyn FnMut(&Event),
) -> PrReport {
    let start = clock.now_unix();
    // Saturating: `--timeout` has no upper bound, and an absurd value should mean
    // "effectively forever", not a debug-build overflow panic.
    let deadline = start.saturating_add(cfg.timeout_mins.saturating_mul(60));
    let mut em = Emitter {
        events: Vec::new(),
        sink,
        last_event_at: start,
    };

    let mut required: Option<RequiredChecks> = None;
    let mut strikes = 0u32;
    let mut head_sha = String::new();
    let mut prev: Option<Rendered> = None;
    let mut last_incomplete_detail: Option<String> = None;
    let mut ever_read = false;

    loop {
        let now = clock.now_unix();
        if now >= deadline {
            let at = clock.now_rfc3339();
            // Which terminal state this is turns on whether we ever managed to read
            // the PR at all. Riding out a rate limit that never clears reaches the
            // deadline having learned nothing — reporting that as `timed-out`
            // ("GitHub never finished") would send an agent looking at the queue when
            // the truth is we could not see. That conflation is the whole defect.
            let (outcome, detail) = if ever_read {
                (
                    Outcome::TimedOut,
                    "the watch budget expired; GitHub never finished",
                )
            } else {
                (
                    Outcome::WatcherError,
                    "the watch budget expired without a single successful read",
                )
            };
            em.emit(at, now, EventKind::Terminal, outcome.as_str().into());
            return finish(
                subject,
                head_sha,
                Terminal {
                    outcome,
                    detail: Some(detail.into()),
                    pointer: None,
                    subject_failure: None,
                    phase: None,
                },
                em.events,
            );
        }

        // One fallible unit: the ruleset (fetched once), the snapshot, and — only in
        // the state where ejection is possible — the merge-group probe. A probe
        // failure is a poll failure, never a silent `None`, which would read as "not
        // ejected".
        let polled = (|| -> Result<Polled, ApiError> {
            let req = match &required {
                Some(r) => r.clone(),
                None => source.required_checks(subject)?,
            };
            let snap = source.snapshot(subject)?;
            let ejection = if decide::needs_ejection_probe(&snap, &req) {
                source.ejection_run(subject)?
            } else {
                None
            };
            let classification =
                classify_snapshot(source, subject, &snap, &req, ejection.as_ref(), &progress)?;
            Ok(Polled {
                required: req,
                snapshot: snap,
                classification,
            })
        })();

        let Polled {
            required: req,
            snapshot: snap,
            classification,
        } = match polled {
            Ok(v) => {
                strikes = 0;
                ever_read = true;
                v
            }
            Err(e) => {
                let now = clock.now_unix();
                let at = clock.now_rfc3339();
                // Absorbed failures are still events. A silently swallowed error is
                // indistinguishable from "nothing changed" — the exact bug that made
                // the hand-rolled watchers look healthy while they were blind.
                em.emit(at, now, EventKind::PollError, e.detail());

                // Rate limiting is not a strike *when we know when it clears*: GitHub
                // says so, and waiting is strictly better than spending five strikes
                // over two minutes on a condition known to last twelve.
                if let ApiError::RateLimited {
                    reset_unix: Some(reset),
                } = e
                {
                    if reset >= deadline {
                        // Waiting would consume the whole budget and still not get an
                        // answer. Say so now instead of discovering it in 90 minutes.
                        return finish(
                            subject,
                            head_sha,
                            Terminal {
                                outcome: Outcome::WatcherError,
                                detail: Some(format!(
                                    "rate limited past the watch budget: {}",
                                    e.detail()
                                )),
                                pointer: None,
                                subject_failure: None,
                                phase: None,
                            },
                            em.events,
                        );
                    }
                    if !cfg.once {
                        // A reset already in the past means the window has cleared;
                        // resume on the normal interval rather than spinning at one
                        // poll per second against a stale timestamp.
                        clock.sleep_secs(reset.saturating_sub(now).max(cfg.interval_secs));
                        continue;
                    }
                }

                // Everything else — including a rate limit whose reset we could not
                // learn (a secondary limit, or the `rate_limit` probe itself failing)
                // — goes through the strike budget. Treating an unattributed 403 as
                // terminal would end a 90-minute watch on one bad poll.
                strikes += 1;
                if !e.is_transient() && !matches!(e, ApiError::RateLimited { .. })
                    || strikes >= cfg.max_strikes
                    || cfg.once
                {
                    return finish(
                        subject,
                        head_sha,
                        Terminal {
                            outcome: Outcome::WatcherError,
                            detail: Some(format!(
                                "giving up after {strikes} failure(s): {}",
                                e.detail()
                            )),
                            pointer: None,
                            subject_failure: None,
                            phase: None,
                        },
                        em.events,
                    );
                }
                clock.sleep_secs(cfg.interval_secs);
                continue;
            }
        };

        head_sha = snap.head_sha.clone();
        required = Some(req.clone());

        // A push starts a new observation history. Without this reset, seeing head A
        // queued and then head B unqueued would falsely report B as dequeued.
        if progress
            .queued_head_sha
            .as_deref()
            .is_some_and(|sha| sha != snap.head_sha)
        {
            progress.queued_head_sha = None;
        }

        let (step, optional_failures) = match classification {
            SnapshotClassification::Complete {
                step,
                optional_failures,
            } => {
                last_incomplete_detail = None;
                (step, optional_failures)
            }
            SnapshotClassification::Incomplete(incomplete) => {
                let detail = incomplete.detail();
                let now = clock.now_unix();
                if last_incomplete_detail.as_deref() != Some(&detail) {
                    em.emit(clock.now_rfc3339(), now, EventKind::Phase, detail.clone());
                    last_incomplete_detail = Some(detail.clone());
                }
                prev = None;
                if cfg.once {
                    return finish(
                        subject,
                        head_sha,
                        Terminal {
                            outcome: Outcome::Pending,
                            detail: Some(detail),
                            pointer: None,
                            subject_failure: None,
                            phase: Some("awaiting-classification-evidence".into()),
                        },
                        em.events,
                    );
                }
                if clock.now_unix().saturating_sub(em.last_event_at) >= cfg.heartbeat_secs {
                    let now = clock.now_unix();
                    em.emit(
                        clock.now_rfc3339(),
                        now,
                        EventKind::Heartbeat,
                        "still awaiting classification evidence".into(),
                    );
                }
                clock.sleep_secs(cfg.interval_secs);
                continue;
            }
        };

        let phase = match &step {
            Step::Continue { phase, .. } => *phase,
            Step::Ready => Phase::ReadyToLand,
            Step::Terminal { .. } => Phase::Terminal,
        };

        // Emit per changed component, not per poll. The first poll emits the whole
        // current state so the log opens with where things stand.
        let warn = match &step {
            Step::Continue { warn, .. } => warn.clone(),
            Step::Ready | Step::Terminal { .. } => None,
        };
        let current = Rendered::of(&snap, &req, phase, warn, optional_failures);
        let now = clock.now_unix();
        let at = clock.now_rfc3339();

        // A returned outcome speaks alone; a passive ready state remains a phase.
        let stopping = matches!(step, Step::Terminal { .. })
            || matches!(step, Step::Ready) && cfg.stop_at_ready;
        match &prev {
            _ if stopping => {}
            None => {
                em.emit(at.clone(), now, EventKind::Phase, phase.as_str().into());
                for (name, state) in &current.checks {
                    em.emit(
                        at.clone(),
                        now,
                        EventKind::Check,
                        format!("{name}: {state}"),
                    );
                }
                if snap.queue.in_queue {
                    em.emit(at.clone(), now, EventKind::Queue, queue_detail(&snap));
                }
            }
            Some(before) => {
                if before.phase != phase {
                    em.emit(at.clone(), now, EventKind::Phase, phase.as_str().into());
                }
                for ((name, state), (_, prev_state)) in current.checks.iter().zip(&before.checks) {
                    if state != prev_state {
                        em.emit(
                            at.clone(),
                            now,
                            EventKind::Check,
                            format!("{name}: {state}"),
                        );
                    }
                }
                if current.queue != before.queue {
                    em.emit(at.clone(), now, EventKind::Queue, queue_detail(&snap));
                }
            }
        }
        if let Some(text) = current.warn.as_ref()
            && prev.as_ref().and_then(|p| p.warn.as_ref()) != Some(text)
        {
            em.emit(at.clone(), now, EventKind::Warning, text.clone());
        }
        for failure in &current.optional_failures {
            if emitted_optional_failure_ids.insert(failure.id.clone()) {
                em.emit(at.clone(), now, EventKind::Warning, failure.detail.clone());
            }
        }
        prev = Some(current);

        // Queue disappearance is meaningful only for the same observed head.
        if snap.queue.in_queue {
            progress.queued_head_sha = Some(snap.head_sha.clone());
        }

        match step {
            Step::Terminal {
                outcome,
                detail,
                pointer,
                subject_failure,
            } => {
                em.emit(at, now, EventKind::Terminal, outcome.as_str().into());
                return finish(
                    subject,
                    head_sha,
                    Terminal {
                        outcome,
                        detail,
                        pointer,
                        subject_failure,
                        phase: None,
                    },
                    em.events,
                );
            }
            Step::Ready if cfg.stop_at_ready => {
                em.emit(
                    at,
                    now,
                    EventKind::Terminal,
                    Outcome::ReadyToLand.as_str().into(),
                );
                return finish(
                    subject,
                    head_sha,
                    Terminal {
                        outcome: Outcome::ReadyToLand,
                        detail: Some(
                            "all required checks passed; obtain approval, then run `pr land`"
                                .into(),
                        ),
                        pointer: None,
                        subject_failure: None,
                        phase: None,
                    },
                    em.events,
                );
            }
            Step::Ready | Step::Continue { .. } => {}
        }

        if cfg.once {
            return finish(
                subject,
                head_sha,
                Terminal {
                    outcome: Outcome::Pending,
                    detail: None,
                    pointer: None,
                    subject_failure: None,
                    phase: Some(phase.as_str().into()),
                },
                em.events,
            );
        }

        // Silence must not read as either progress or death: if nothing has moved for
        // the heartbeat interval, say so.
        if clock.now_unix().saturating_sub(em.last_event_at) >= cfg.heartbeat_secs {
            let now = clock.now_unix();
            let at = clock.now_rfc3339();
            em.emit(
                at,
                now,
                EventKind::Heartbeat,
                format!(
                    "still {}, no change for {}m",
                    phase.as_str(),
                    cfg.heartbeat_secs / 60
                ),
            );
        }

        clock.sleep_secs(cfg.interval_secs);
    }
}

fn queue_detail(snap: &PrSnapshot) -> String {
    match snap.queue.position {
        Some(p) => format!("position {p}"),
        None => "queued".into(),
    }
}

/// How a watch ended, before it is joined with the subject and the event log.
struct Terminal {
    outcome: Outcome,
    detail: Option<String>,
    pointer: Option<String>,
    subject_failure: Option<SubjectFailure>,
    /// Only ever `Some` for `Pending`, which `--once` alone can produce.
    phase: Option<String>,
}

fn finish(subject: &Subject, head_sha: String, end: Terminal, events: Vec<Event>) -> PrReport {
    PrReport {
        outcome: end.outcome,
        pr: subject.number.0,
        head_sha,
        phase: end.phase,
        detail: end.detail,
        pointer: end.pointer,
        shared_failure: None,
        subject_failure: end.subject_failure,
        events,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::pr::snapshot::{
        CheckProvider, CheckState, MergeStateStatus, Mergeable, RequiredChecks,
    };
    use crate::pr::test_support::*;

    #[test]
    fn unix_seconds_format_as_rfc3339_utc() {
        assert_eq!(format_unix_utc(0), "1970-01-01T00:00:00Z");
        assert_eq!(format_unix_utc(86_399), "1970-01-01T23:59:59Z");
        assert_eq!(format_unix_utc(86_400), "1970-01-02T00:00:00Z");
        assert_eq!(format_unix_utc(1_609_459_200), "2021-01-01T00:00:00Z");
    }

    // ---- sustained API failure is its own outcome ----

    #[test]
    fn five_consecutive_api_failures_yield_watcher_error() {
        let src = FakeSource::new(
            (0..5)
                .map(|_| Err(ApiError::Transport("boom".into())))
                .collect(),
            queue_rules(),
        );
        let mut seen: Vec<Event> = Vec::new();
        let report = watch(&src, &clock(), &subject(), cfg(), &mut |e| {
            seen.push(e.clone())
        });

        // Must fail if the loop returns silence, success, or a check verdict.
        assert_eq!(
            report.outcome,
            Outcome::WatcherError,
            "sustained API failure must be watcher-error, not {:?}",
            report.outcome
        );
        assert_ne!(report.outcome, Outcome::Merged);
        assert_ne!(report.outcome, Outcome::ChecksFailed);
        assert_ne!(report.outcome, Outcome::TimedOut);
        assert!(
            !report.events.is_empty(),
            "silence must never be the answer"
        );
        assert_eq!(
            report
                .events
                .iter()
                .filter(|e| e.kind == EventKind::PollError)
                .count(),
            5,
            "every absorbed failure is an event"
        );
    }

    #[test]
    fn a_transient_failure_before_success_does_not_end_the_watch() {
        let src = FakeSource::new(
            vec![
                Err(ApiError::Transport("blip".into())),
                Ok(merged_snapshot()),
            ],
            queue_rules(),
        );
        assert_eq!(
            watch(&src, &clock(), &subject(), cfg(), &mut |_| {}).outcome,
            Outcome::Merged
        );
    }

    // ---- rate limiting is not a strike ----

    #[test]
    fn rate_limit_inside_the_budget_waits_and_continues() {
        let c = clock();
        let src = FakeSource::new(
            vec![
                Err(ApiError::RateLimited {
                    reset_unix: Some(600),
                }),
                Ok(merged_snapshot()),
            ],
            queue_rules(),
        );
        let report = watch(&src, &c, &subject(), cfg(), &mut |_| {});
        assert_eq!(report.outcome, Outcome::Merged);
        assert!(c.now_unix() >= 600, "must have waited for the reset");
        assert!(
            report
                .events
                .iter()
                .any(|e| e.detail.contains("rate limited"))
        );
    }

    #[test]
    fn rate_limit_beyond_the_budget_is_watcher_error_immediately() {
        let c = clock();
        let src = FakeSource::new(
            vec![Err(ApiError::RateLimited {
                reset_unix: Some(99_999),
            })],
            queue_rules(),
        );
        let report = watch(&src, &c, &subject(), cfg(), &mut |_| {});
        assert_eq!(report.outcome, Outcome::WatcherError);
        assert!(c.now_unix() < 5_400, "must not burn the budget waiting");
    }

    // ---- emit on change only ----

    #[test]
    fn identical_consecutive_snapshots_emit_exactly_one_phase_event() {
        let src = FakeSource::new(
            vec![
                Ok(open_pending()),
                Ok(open_pending()),
                Ok(open_pending()),
                Ok(merged_snapshot()),
            ],
            queue_rules(),
        );
        let report = watch(&src, &clock(), &subject(), cfg(), &mut |_| {});
        let phase_events = report
            .events
            .iter()
            .filter(|e| e.kind == EventKind::Phase && e.detail == "awaiting-checks")
            .count();
        assert_eq!(phase_events, 1, "unchanged state must not re-emit per poll");
    }

    #[test]
    fn a_queue_position_change_emits_per_change() {
        // Poll 1 emits the full current state (including the queue at position 3);
        // poll 2 emits only the changed component (position 2).
        let src = FakeSource::new(
            vec![Ok(queued_at(3)), Ok(queued_at(2)), Ok(merged_snapshot())],
            queue_rules(),
        );
        let report = watch(&src, &clock(), &subject(), cfg(), &mut |_| {});
        assert_eq!(
            report
                .events
                .iter()
                .filter(|e| e.kind == EventKind::Queue)
                .count(),
            2
        );
    }

    #[test]
    fn a_non_required_check_changing_emits_nothing() {
        // The counterpart to the queue test above, through the real loop: change
        // detection must ignore checks the ruleset does not require, or every
        // unrelated lint job would produce an event.
        let mut noisy = queued_at(3);
        noisy.checks.push(check(
            "optional-lint",
            CheckState::Failure,
            "2026-07-30T14:05:00Z",
        ));
        let src = FakeSource::new(
            vec![Ok(queued_at(3)), Ok(noisy), Ok(merged_snapshot())],
            queue_rules(),
        );
        let report = watch(&src, &clock(), &subject(), cfg(), &mut |_| {});
        assert_eq!(
            report
                .events
                .iter()
                .filter(|e| e.kind == EventKind::Check)
                .count(),
            2,
            "only the two required contexts, emitted once on the first poll"
        );
    }

    #[test]
    fn checks_render_as_words_not_debug_output() {
        // A human reads this stream live; `Some(Pending)` / `None` is not a report.
        let src = FakeSource::new(
            vec![Ok(open_pending()), Ok(merged_snapshot())],
            queue_rules(),
        );
        let report = watch(&src, &clock(), &subject(), cfg(), &mut |_| {});
        let checks: Vec<&str> = report
            .events
            .iter()
            .filter(|e| e.kind == EventKind::Check)
            .map(|e| e.detail.as_str())
            .collect();
        assert!(
            checks.iter().any(|d| d.ends_with(": pending")),
            "{checks:?}"
        );
        assert!(
            !checks
                .iter()
                .any(|d| d.contains("Some(") || d.contains("None")),
            "{checks:?}"
        );
    }

    #[test]
    fn an_ejected_pr_is_reported_through_the_loop_not_just_the_rule() {
        // The probe wiring — `needs_ejection_probe` → `ejection_run` → `classify` —
        // has to be exercised end-to-end, or a dropped result would pass every test
        // in `decide`. The PR is green, open, and unqueued, with a failed merge-group
        // run newer than its head: reachable with no prior sight of the queue entry.
        let src = FakeSource::new(vec![Ok(open(green()))], queue_rules())
            .with_ejection(Some(ejection("2026-07-30T14:30:00Z")));
        let report = watch(&src, &clock(), &subject(), cfg(), &mut |_| {});
        assert_eq!(report.outcome, Outcome::Ejected);
        assert!(report.pointer.unwrap().contains("/actions/runs/"));
    }

    #[test]
    fn a_probe_failure_is_a_poll_error_never_a_silent_not_ejected() {
        // Swallowing the probe error would read as "no ejection found", which is the
        // silent-failure shape this command exists to eliminate.
        let src = FakeSource::new(vec![Ok(open(green()))], queue_rules())
            .with_ejection_error(ApiError::Transport("probe down".into()));
        let report = watch(&src, &clock(), &subject(), cfg(), &mut |_| {});
        assert_eq!(report.outcome, Outcome::WatcherError);
        assert!(
            report
                .events
                .iter()
                .any(|e| e.kind == EventKind::PollError && e.detail.contains("probe down"))
        );
    }

    #[test]
    fn an_unattributed_rate_limit_is_absorbed_not_terminal() {
        // A secondary rate limit carries no reset (the `rate_limit` probe can fail
        // too). One such 403 must not end a 90-minute watch.
        let src = FakeSource::new(
            vec![
                Err(ApiError::RateLimited { reset_unix: None }),
                Ok(merged_snapshot()),
            ],
            queue_rules(),
        );
        assert_eq!(
            watch(&src, &clock(), &subject(), cfg(), &mut |_| {}).outcome,
            Outcome::Merged
        );
    }

    #[test]
    fn ten_minutes_of_stasis_emits_one_heartbeat() {
        // Poll-then-sleep, so poll k lands at t = 30*(k-1); poll 21 is exactly t=600.
        let mut snaps: Vec<_> = (0..21).map(|_| Ok(open_pending())).collect();
        snaps.push(Ok(merged_snapshot()));
        let src = FakeSource::new(snaps, queue_rules());
        let report = watch(&src, &clock(), &subject(), cfg(), &mut |_| {});
        assert_eq!(
            report
                .events
                .iter()
                .filter(|e| e.kind == EventKind::Heartbeat)
                .count(),
            1
        );
    }

    // ---- budget & --once ----

    #[test]
    fn budget_expiry_is_timed_out_not_watcher_error() {
        // One scripted snapshot; the fake repeats it until the budget runs out.
        let src = FakeSource::new(vec![Ok(open_pending())], queue_rules());
        let report = watch(&src, &clock(), &subject(), cfg(), &mut |_| {});
        assert_eq!(report.outcome, Outcome::TimedOut);
        assert_ne!(
            report.outcome,
            Outcome::WatcherError,
            "the tooling worked fine"
        );
    }

    #[test]
    fn ready_handoff_returns_without_sleeping_and_once_agrees() {
        for once in [false, true] {
            let c = clock();
            let src = FakeSource::new(vec![Ok(open(green()))], queue_rules());
            let mut config = cfg();
            config.once = once;
            let report = watch(&src, &c, &subject(), config, &mut |_| {});
            assert_eq!(report.outcome, Outcome::ReadyToLand);
            assert_eq!(report.head_sha, "abc");
            assert!(report.detail.unwrap().contains("obtain approval"));
            assert_eq!(c.now_unix(), 0);
        }
    }

    #[test]
    fn passive_watch_crosses_ready_to_merged() {
        let src = FakeSource::new(
            vec![
                Ok(open(green())),
                Ok(armed_snapshot()),
                Ok(queued_at(2)),
                Ok(merged_snapshot()),
            ],
            queue_rules(),
        );
        let mut config = cfg();
        config.stop_at_ready = false;
        let report = watch(&src, &clock(), &subject(), config, &mut |_| {});
        assert_eq!(report.outcome, Outcome::Merged);
        assert!(
            report
                .events
                .iter()
                .any(|e| e.kind == EventKind::Phase && e.detail == Phase::ReadyToLand.as_str())
        );
    }

    #[test]
    fn same_head_dequeue_returns_after_exactly_one_poll_interval() {
        let c = clock();
        let src = FakeSource::new(vec![Ok(queued_at(2)), Ok(open(green()))], queue_rules());
        let report = watch(&src, &c, &subject(), cfg(), &mut |_| {});
        assert_eq!(report.outcome, Outcome::Dequeued);
        let detail = report.detail.unwrap();
        assert!(detail.contains("queue entry vanished"));
        assert!(detail.contains("no failed current-head merge-group run"));
        assert_eq!(c.now_unix(), cfg().interval_secs);
    }

    #[test]
    fn same_head_queue_disappearance_waits_while_checks_are_pending() {
        let src = FakeSource::new(
            vec![Ok(queued_at(2)), Ok(open_pending()), Ok(merged_snapshot())],
            queue_rules(),
        );
        let report = watch(&src, &clock(), &subject(), cfg(), &mut |_| {});
        assert_eq!(report.outcome, Outcome::Merged);
        assert!(
            report
                .events
                .iter()
                .any(|e| e.kind == EventKind::Phase && e.detail == "awaiting-checks")
        );
        assert_ne!(report.outcome, Outcome::Dequeued);
    }

    #[test]
    fn new_head_resets_queue_history() {
        let mut new_head = open(green());
        new_head.head_sha = "def".into();
        new_head.head_committed_at = "2026-07-30T16:00:00Z".into();
        let src = FakeSource::new(
            vec![Ok(queued_at(2)), Ok(new_head), Ok(merged_snapshot())],
            queue_rules(),
        );
        let mut config = cfg();
        config.stop_at_ready = false;
        let report = watch(&src, &clock(), &subject(), config, &mut |_| {});
        assert_eq!(report.outcome, Outcome::Merged);
        assert_ne!(report.outcome, Outcome::Dequeued);
    }

    #[test]
    fn superseded_head_failure_cannot_terminate_after_evidence_switches() {
        let first = open(vec![check("Aggregate verdict", CheckState::Pending, "")]);
        let mut second = open(vec![
            actions_check(
                "Renamed validation lane",
                11,
                CheckState::Failure,
                "2026-07-30T14:10:00Z",
            ),
            check("Aggregate verdict", CheckState::Pending, ""),
        ]);
        second.head_sha = "def".into();
        let src = FakeSource::new(
            vec![Ok(first), Ok(second), Ok(merged_snapshot())],
            aggregate_rules(),
        )
        .with_actions_evidence_script(vec![Ok(evidence_for("abc"))]);
        let report = watch(&src, &clock(), &subject(), cfg(), &mut |_| {});
        assert_eq!(report.outcome, Outcome::Merged);
        assert!(!report.events.iter().any(|event| {
            event.kind == EventKind::Terminal && event.detail == Outcome::ChecksFailed.as_str()
        }));
    }

    #[test]
    fn empty_required_set_fails_closed_in_every_watch_mode() {
        let empty = RequiredChecks {
            contexts: Vec::new(),
            strict: false,
            queue_present: true,
        };
        for (once, stop_at_ready) in [(false, true), (true, true), (false, false)] {
            let c = clock();
            let src = FakeSource::new(vec![Ok(open(green()))], empty.clone());
            let mut config = cfg();
            config.once = once;
            config.stop_at_ready = stop_at_ready;
            let report = watch(&src, &c, &subject(), config, &mut |_| {});
            assert_eq!(report.outcome, Outcome::WatcherError);
            assert_eq!(c.now_unix(), 0);
        }
    }

    #[test]
    fn every_landable_merge_status_obeys_each_watch_mode() {
        for status in [
            MergeStateStatus::Clean,
            MergeStateStatus::HasHooks,
            MergeStateStatus::Unstable,
            MergeStateStatus::Behind,
        ] {
            let mut snap = open(green());
            snap.merge_state_status = status;

            for once in [false, true] {
                let src = FakeSource::new(vec![Ok(snap.clone())], queue_rules());
                let mut config = cfg();
                config.once = once;
                assert_eq!(
                    watch(&src, &clock(), &subject(), config, &mut |_| {}).outcome,
                    Outcome::ReadyToLand,
                    "{status:?}, once={once}"
                );
            }

            let src = FakeSource::new(vec![Ok(snap), Ok(merged_snapshot())], queue_rules());
            let mut config = cfg();
            config.stop_at_ready = false;
            let report = watch(&src, &clock(), &subject(), config, &mut |_| {});
            assert_eq!(report.outcome, Outcome::Merged);
            assert!(
                report
                    .events
                    .iter()
                    .any(|e| e.kind == EventKind::Phase && e.detail == "ready-to-land")
            );
        }
    }

    #[test]
    fn every_blocked_merge_status_stops_every_watch_mode() {
        for status in [
            MergeStateStatus::Blocked,
            MergeStateStatus::Draft,
            MergeStateStatus::Dirty,
        ] {
            for (once, stop_at_ready) in [(false, true), (true, true), (false, false)] {
                let mut snap = open(green());
                snap.merge_state_status = status;
                let src = FakeSource::new(vec![Ok(snap)], queue_rules());
                let mut config = cfg();
                config.once = once;
                config.stop_at_ready = stop_at_ready;
                assert_eq!(
                    watch(&src, &clock(), &subject(), config, &mut |_| {}).outcome,
                    Outcome::Blocked,
                    "{status:?}, once={once}, stop_at_ready={stop_at_ready}"
                );
            }
        }
    }

    #[test]
    fn unknown_merge_dimensions_wait_in_every_watch_mode() {
        let mut unknown_mergeable = open(green());
        unknown_mergeable.mergeable = Mergeable::Unknown;
        let mut unknown_status = open(green());
        unknown_status.merge_state_status = MergeStateStatus::Unknown;

        for snap in [unknown_mergeable, unknown_status] {
            let src = FakeSource::new(vec![Ok(snap.clone())], queue_rules());
            let mut once = cfg();
            once.once = true;
            let report = watch(&src, &clock(), &subject(), once, &mut |_| {});
            assert_eq!(report.outcome, Outcome::Pending);
            assert_eq!(
                report.phase.as_deref(),
                Some(Phase::AwaitingMergeability.as_str())
            );

            for stop_at_ready in [true, false] {
                let src =
                    FakeSource::new(vec![Ok(snap.clone()), Ok(merged_snapshot())], queue_rules());
                let mut config = cfg();
                config.stop_at_ready = stop_at_ready;
                let report = watch(&src, &clock(), &subject(), config, &mut |_| {});
                assert_eq!(report.outcome, Outcome::Merged);
                assert!(report.events.iter().any(|e| {
                    e.kind == EventKind::Phase && e.detail == Phase::AwaitingMergeability.as_str()
                }));
            }
        }
    }

    #[test]
    fn once_mode_returns_pending_without_looping() {
        let c = clock();
        let src = FakeSource::new(vec![Ok(open_pending())], queue_rules());
        let mut config = cfg();
        config.once = true;
        let report = watch(&src, &c, &subject(), config, &mut |_| {});
        assert_eq!(report.outcome, Outcome::Pending);
        assert_eq!(report.phase.as_deref(), Some("awaiting-checks"));
        assert_eq!(c.now_unix(), 0, "--once must not sleep");
    }

    #[test]
    fn once_mode_reaches_a_terminal_outcome_when_one_exists() {
        let src = FakeSource::new(vec![Ok(merged_snapshot())], queue_rules());
        let mut config = cfg();
        config.once = true;
        assert_eq!(
            watch(&src, &clock(), &subject(), config, &mut |_| {}).outcome,
            Outcome::Merged
        );
    }

    fn aggregate_rules() -> RequiredChecks {
        RequiredChecks {
            contexts: vec!["Aggregate verdict".into()],
            strict: false,
            queue_present: false,
        }
    }

    fn workflow_with_optional() -> &'static str {
        r#"
            jobs:
              renamed-lane:
                name: Renamed validation lane
              matrix-like:
                name: Arbitrary browser arm
              aggregate:
                name: Aggregate verdict
                needs: [renamed-lane, matrix-like]
              optional:
                name: Unrelated diagnostic
        "#
    }

    fn evidence_for(head: &str) -> super::super::evidence::ActionsEvidence {
        actions_evidence(
            head,
            workflow_with_optional(),
            vec![
                ("Renamed validation lane", 11),
                ("Arbitrary browser arm", 12),
                ("Aggregate verdict", 13),
                ("Unrelated diagnostic", 14),
            ],
        )
    }

    #[test]
    fn transitive_failures_stop_every_watch_mode_before_the_aggregate() {
        for (name, id) in [
            ("Renamed validation lane", 11),
            ("Arbitrary browser arm", 12),
        ] {
            for (once, stop_at_ready) in [(false, true), (true, true), (false, false)] {
                let snap = open(vec![
                    actions_check(name, id, CheckState::Failure, "2026-07-30T14:10:00Z"),
                    check("Aggregate verdict", CheckState::Pending, ""),
                ]);
                let src = FakeSource::new(vec![Ok(snap)], aggregate_rules())
                    .with_actions_evidence(evidence_for("abc"));
                let mut config = cfg();
                config.once = once;
                config.stop_at_ready = stop_at_ready;
                let report = watch(&src, &clock(), &subject(), config, &mut |_| {});
                assert_eq!(report.outcome, Outcome::ChecksFailed);
                assert!(report.detail.unwrap().contains(name));
                assert_eq!(report.pointer.as_deref(), Some("https://x/1"));
                assert_eq!(
                    report.subject_failure,
                    Some(SubjectFailure {
                        workflow_run_id: 1,
                        check_run_id: id,
                        name: name.into(),
                    })
                );
            }
        }
    }

    #[test]
    fn unrelated_same_named_actions_pending_does_not_suppress_transitive_failure() {
        let failed = actions_check(
            "Renamed validation lane",
            11,
            CheckState::Failure,
            "2026-07-30T14:10:00Z",
        );
        let mut unrelated = actions_check("Renamed validation lane", 22, CheckState::Pending, "");
        unrelated.provider = CheckProvider::GitHubActions {
            check_run_id: 22,
            workflow_run_id: 2,
        };
        let snap = open(vec![
            failed,
            unrelated,
            check("Aggregate verdict", CheckState::Pending, ""),
        ]);
        let src = FakeSource::new(vec![Ok(snap)], aggregate_rules())
            .with_actions_evidence(evidence_for("abc"));
        assert_eq!(
            watch(&src, &clock(), &subject(), cfg(), &mut |_| {}).outcome,
            Outcome::ChecksFailed
        );
    }

    #[test]
    fn duplicate_same_run_actions_jobs_fail_closed_instead_of_looking_like_a_rerun() {
        let workflow = r#"
            jobs:
              first:
                name: Shared display name
              second:
                name: Shared display name
              aggregate:
                name: Aggregate verdict
                needs: first
        "#;
        let snap = open(vec![
            actions_check(
                "Shared display name",
                11,
                CheckState::Failure,
                "2026-07-30T14:10:00Z",
            ),
            actions_check("Shared display name", 12, CheckState::Pending, ""),
            check("Aggregate verdict", CheckState::Pending, ""),
        ]);
        let evidence = actions_evidence(
            "abc",
            workflow,
            vec![
                ("Shared display name", 11),
                ("Shared display name", 12),
                ("Aggregate verdict", 13),
            ],
        );
        let report = watch(
            &FakeSource::new(vec![Ok(snap)], aggregate_rules()).with_actions_evidence(evidence),
            &clock(),
            &subject(),
            cfg(),
            &mut |_| {},
        );
        assert_eq!(report.outcome, Outcome::WatcherError);
    }

    #[test]
    fn completed_same_run_duplicate_actions_jobs_fail_closed() {
        let workflow = r#"
            jobs:
              first:
                name: Shared display name
              second:
                name: Shared display name
              aggregate:
                name: Aggregate verdict
                needs: first
        "#;
        let snap = open(vec![
            actions_check(
                "Shared display name",
                11,
                CheckState::Failure,
                "2026-07-30T14:10:00Z",
            ),
            actions_check(
                "Shared display name",
                12,
                CheckState::Success,
                "2026-07-30T14:11:00Z",
            ),
            check("Aggregate verdict", CheckState::Pending, ""),
        ]);
        let evidence = actions_evidence(
            "abc",
            workflow,
            vec![
                ("Shared display name", 11),
                ("Shared display name", 12),
                ("Aggregate verdict", 13),
            ],
        );
        let report = watch(
            &FakeSource::new(vec![Ok(snap)], aggregate_rules()).with_actions_evidence(evidence),
            &clock(),
            &subject(),
            cfg(),
            &mut |_| {},
        );
        assert_eq!(report.outcome, Outcome::WatcherError);
    }

    #[test]
    fn optional_failure_is_emitted_once_and_does_not_block_readiness() {
        let pending = open(vec![
            actions_check(
                "Unrelated diagnostic",
                14,
                CheckState::Failure,
                "2026-07-30T14:10:00Z",
            ),
            check("Aggregate verdict", CheckState::Pending, ""),
        ]);
        let ready = open(vec![
            actions_check(
                "Unrelated diagnostic",
                14,
                CheckState::Failure,
                "2026-07-30T14:10:00Z",
            ),
            check(
                "Aggregate verdict",
                CheckState::Success,
                "2026-07-30T14:20:00Z",
            ),
        ]);
        let src = FakeSource::new(vec![Ok(pending), Ok(ready)], aggregate_rules())
            .with_actions_evidence_script(vec![Ok(evidence_for("abc")), Ok(evidence_for("abc"))]);
        let report = watch(&src, &clock(), &subject(), cfg(), &mut |_| {});
        assert_eq!(report.outcome, Outcome::ReadyToLand);
        let warnings = report
            .events
            .iter()
            .filter(|event| event.kind == EventKind::Warning)
            .collect::<Vec<_>>();
        assert_eq!(warnings.len(), 1);
        assert!(
            warnings[0]
                .detail
                .contains("optional check failed: Unrelated diagnostic")
        );
        assert!(warnings[0].detail.contains("https://x/1"));
    }

    #[test]
    fn non_actions_optional_failure_needs_no_invented_actions_evidence() {
        let snap = open(vec![
            check(
                "External advisory",
                CheckState::Failure,
                "2026-07-30T14:10:00Z",
            ),
            check("Aggregate verdict", CheckState::Pending, ""),
        ]);
        let src = FakeSource::new(vec![Ok(snap)], aggregate_rules());
        let mut once = cfg();
        once.once = true;
        let report = watch(&src, &clock(), &subject(), once, &mut |_| {});
        assert_eq!(report.outcome, Outcome::Pending);
        assert!(report.events.iter().any(|event| {
            event.kind == EventKind::Warning && event.detail.contains("External advisory")
        }));
    }

    #[test]
    fn a_then_b_optional_failures_emit_exactly_once_each() {
        let source = r#"
            jobs:
              aggregate:
                name: Aggregate verdict
              first:
                name: Optional A
              second:
                name: Optional B
        "#;
        let failure = |name, id, at| actions_check(name, id, CheckState::Failure, at);
        let first = open(vec![
            failure("Optional A", 31, "2026-07-30T14:10:00Z"),
            check("Aggregate verdict", CheckState::Pending, ""),
        ]);
        let second = open(vec![
            failure("Optional A", 31, "2026-07-30T14:10:00Z"),
            failure("Optional B", 32, "2026-07-30T14:20:00Z"),
            check("Aggregate verdict", CheckState::Pending, ""),
        ]);
        let evidence = || {
            actions_evidence(
                "abc",
                source,
                vec![
                    ("Aggregate verdict", 30),
                    ("Optional A", 31),
                    ("Optional B", 32),
                ],
            )
        };
        let src = FakeSource::new(
            vec![Ok(first), Ok(second), Ok(merged_snapshot())],
            aggregate_rules(),
        )
        .with_actions_evidence_script(vec![Ok(evidence()), Ok(evidence())]);
        let report = watch(&src, &clock(), &subject(), cfg(), &mut |_| {});
        let warnings = report
            .events
            .iter()
            .filter(|event| event.kind == EventKind::Warning)
            .map(|event| event.detail.as_str())
            .collect::<Vec<_>>();
        assert_eq!(warnings.len(), 2, "A then B emits two warnings total");
        assert_eq!(
            warnings
                .iter()
                .filter(|detail| detail.contains("optional check failed: Optional A"))
                .count(),
            1,
            "adding B must not re-emit A"
        );
        assert_eq!(
            warnings
                .iter()
                .filter(|detail| detail.contains("optional check failed: Optional B"))
                .count(),
            1,
            "B emits once"
        );
    }

    #[test]
    fn optional_failure_rerun_emits_a_new_attempt() {
        let source = r#"
            jobs:
              aggregate:
                name: Aggregate verdict
              optional:
                name: Optional A
        "#;
        let first = open(vec![
            actions_check(
                "Optional A",
                31,
                CheckState::Failure,
                "2026-07-30T14:10:00Z",
            ),
            check("Aggregate verdict", CheckState::Pending, ""),
        ]);
        let rerun = open(vec![
            actions_check(
                "Optional A",
                31,
                CheckState::Failure,
                "2026-07-30T14:10:00Z",
            ),
            actions_check(
                "Optional A",
                33,
                CheckState::Failure,
                "2026-07-30T14:30:00Z",
            ),
            check("Aggregate verdict", CheckState::Pending, ""),
        ]);
        let first_evidence = actions_evidence(
            "abc",
            source,
            vec![("Aggregate verdict", 30), ("Optional A", 31)],
        );
        // The rerun rollup retains 31, but the current-attempt jobs endpoint only
        // exposes its replacement check run 33.
        let rerun_evidence = actions_evidence(
            "abc",
            source,
            vec![("Aggregate verdict", 30), ("Optional A", 33)],
        );
        let src = FakeSource::new(
            vec![Ok(first), Ok(rerun), Ok(merged_snapshot())],
            aggregate_rules(),
        )
        .with_actions_evidence_script(vec![Ok(first_evidence), Ok(rerun_evidence)]);
        let report = watch(&src, &clock(), &subject(), cfg(), &mut |_| {});
        assert_eq!(
            report
                .events
                .iter()
                .filter(|event| {
                    event.kind == EventKind::Warning
                        && event.detail.contains("optional check failed: Optional A")
                })
                .count(),
            2
        );
    }

    #[test]
    fn pending_or_successful_rerun_suppresses_a_superseded_failure() {
        let mut pending_rerun =
            actions_check("Renamed validation lane", 12, CheckState::Pending, "");
        pending_rerun.started_at = Some("2026-07-30T14:30:00Z".into());
        let pending = open(vec![
            actions_check(
                "Renamed validation lane",
                11,
                CheckState::Failure,
                "2026-07-30T14:10:00Z",
            ),
            pending_rerun,
            check("Aggregate verdict", CheckState::Pending, ""),
        ]);
        let src = FakeSource::new(vec![Ok(pending)], aggregate_rules()).with_actions_evidence(
            actions_evidence(
                "abc",
                workflow_with_optional(),
                vec![("Renamed validation lane", 12), ("Aggregate verdict", 13)],
            ),
        );
        let mut once = cfg();
        once.once = true;
        let report = watch(&src, &clock(), &subject(), once, &mut |_| {});
        assert_eq!(report.outcome, Outcome::Pending);

        let mut successful_rerun = actions_check(
            "Renamed validation lane",
            12,
            CheckState::Success,
            "2026-07-30T14:40:00Z",
        );
        successful_rerun.completed_at = Some("2026-07-30T14:40:00Z".into());
        let successful = open(vec![
            actions_check(
                "Renamed validation lane",
                11,
                CheckState::Failure,
                "2026-07-30T14:10:00Z",
            ),
            successful_rerun,
            check(
                "Aggregate verdict",
                CheckState::Success,
                "2026-07-30T14:50:00Z",
            ),
        ]);
        let src = FakeSource::new(vec![Ok(successful)], aggregate_rules()).with_actions_evidence(
            actions_evidence(
                "abc",
                workflow_with_optional(),
                vec![("Renamed validation lane", 12), ("Aggregate verdict", 13)],
            ),
        );
        assert_eq!(
            watch(&src, &clock(), &subject(), cfg(), &mut |_| {}).outcome,
            Outcome::ReadyToLand
        );
    }

    #[test]
    fn incomplete_classification_waits_without_spending_strikes_or_repeating_status() {
        let workflow = r#"
            jobs:
              lane:
                name: Renamed validation lane
              aggregate:
                name: Aggregate verdict
                needs: lane
        "#;
        let failed = || {
            open(vec![actions_check(
                "Renamed validation lane",
                11,
                CheckState::Failure,
                "2026-07-30T14:10:00Z",
            )])
        };
        let materialized = open(vec![
            actions_check(
                "Renamed validation lane",
                11,
                CheckState::Failure,
                "2026-07-30T14:10:00Z",
            ),
            check("Aggregate verdict", CheckState::Pending, ""),
        ]);
        let mut snapshots = (0..6).map(|_| Ok(failed())).collect::<Vec<_>>();
        snapshots.push(Ok(materialized));
        let mut evidence = (0..6)
            .map(|_| {
                Ok(active_actions_evidence(
                    "abc",
                    workflow,
                    vec![("Renamed validation lane", 11)],
                ))
            })
            .collect::<Vec<_>>();
        evidence.push(Ok(actions_evidence(
            "abc",
            workflow,
            vec![("Renamed validation lane", 11), ("Aggregate verdict", 12)],
        )));
        let src =
            FakeSource::new(snapshots, aggregate_rules()).with_actions_evidence_script(evidence);
        let mut config = cfg();
        config.heartbeat_secs = 60;
        let report = watch(&src, &clock(), &subject(), config, &mut |_| {});
        assert_eq!(report.outcome, Outcome::ChecksFailed);
        assert!(
            report
                .events
                .iter()
                .all(|event| event.kind != EventKind::PollError)
        );
        assert_eq!(
            report
                .events
                .iter()
                .filter(|event| {
                    event.kind == EventKind::Phase
                        && event.detail.contains("Aggregate verdict")
                        && event.detail.contains(".github/workflows/test.yml")
                })
                .count(),
            1,
            "unchanged incomplete evidence emits one phase event"
        );
        assert_eq!(
            report
                .events
                .iter()
                .filter(|event| event.kind == EventKind::Heartbeat)
                .count(),
            2,
            "unchanged incomplete evidence retains bounded liveness reporting"
        );
    }

    #[test]
    fn one_shot_incomplete_classification_is_pending() {
        let workflow = r#"
            jobs:
              lane:
                name: Renamed validation lane
              aggregate:
                name: Aggregate verdict
                needs: lane
        "#;
        let failed = open(vec![actions_check(
            "Renamed validation lane",
            11,
            CheckState::Failure,
            "2026-07-30T14:10:00Z",
        )]);
        let src = FakeSource::new(vec![Ok(failed)], aggregate_rules()).with_actions_evidence(
            active_actions_evidence("abc", workflow, vec![("Renamed validation lane", 11)]),
        );
        let mut once = cfg();
        once.once = true;
        let report = watch(&src, &clock(), &subject(), once, &mut |_| {});
        assert_eq!(report.outcome, Outcome::Pending);
        assert!(
            report
                .detail
                .as_deref()
                .is_some_and(|detail| detail.contains("Aggregate verdict"))
        );
    }

    #[test]
    fn completed_workflow_missing_target_exhausts_the_strike_budget() {
        let workflow = r#"
            jobs:
              lane:
                name: Renamed validation lane
              aggregate:
                name: Aggregate verdict
                needs: lane
        "#;
        let failed = open(vec![actions_check(
            "Renamed validation lane",
            11,
            CheckState::Failure,
            "2026-07-30T14:10:00Z",
        )]);
        let src = FakeSource::new(vec![Ok(failed)], aggregate_rules()).with_actions_evidence(
            actions_evidence("abc", workflow, vec![("Renamed validation lane", 11)]),
        );
        let report = watch(&src, &clock(), &subject(), cfg(), &mut |_| {});
        assert_eq!(report.outcome, Outcome::WatcherError);
        assert_eq!(
            report
                .events
                .iter()
                .filter(|event| event.kind == EventKind::PollError)
                .count(),
            5
        );
        assert!(
            report
                .detail
                .as_deref()
                .is_some_and(|detail| detail.contains("Aggregate verdict"))
        );
    }

    #[test]
    fn classification_error_is_a_poll_error_that_can_recover() {
        let failed = open(vec![
            actions_check(
                "Renamed validation lane",
                11,
                CheckState::Failure,
                "2026-07-30T14:10:00Z",
            ),
            check("Aggregate verdict", CheckState::Pending, ""),
        ]);
        let src = FakeSource::new(vec![Ok(failed), Ok(merged_snapshot())], aggregate_rules())
            .with_actions_evidence_error(ApiError::Transport("evidence unavailable".into()));
        let report = watch(&src, &clock(), &subject(), cfg(), &mut |_| {});
        assert_eq!(report.outcome, Outcome::Merged);
        assert!(report.events.iter().any(|event| {
            event.kind == EventKind::PollError && event.detail.contains("evidence unavailable")
        }));
    }

    #[test]
    fn a_new_head_emits_the_same_shaped_non_actions_failure_again() {
        let old = open(vec![
            check(
                "External advisory",
                CheckState::Failure,
                "2026-07-30T14:10:00Z",
            ),
            check("Aggregate verdict", CheckState::Pending, ""),
        ]);
        let mut new = open(vec![
            check(
                "External advisory",
                CheckState::Failure,
                "2026-07-30T14:10:00Z",
            ),
            check(
                "Aggregate verdict",
                CheckState::Success,
                "2026-07-30T14:20:00Z",
            ),
        ]);
        new.head_sha = "def".into();
        new.head_committed_at = "2026-07-30T14:20:00Z".into();
        let src = FakeSource::new(
            vec![Ok(old), Ok(new), Ok(merged_snapshot())],
            aggregate_rules(),
        );
        let mut passive = cfg();
        passive.stop_at_ready = false;
        let report = watch(&src, &clock(), &subject(), passive, &mut |_| {});
        assert_eq!(report.outcome, Outcome::Merged);
        assert_eq!(
            report
                .events
                .iter()
                .filter(|event| {
                    event.kind == EventKind::Warning
                        && event
                            .detail
                            .contains("optional check failed: External advisory")
                })
                .count(),
            2,
            "a current-head identity must not suppress the same-shaped failure after a push"
        );
    }

    #[test]
    fn every_event_reaches_both_the_sink_and_the_report() {
        let src = FakeSource::new(
            vec![Ok(open_pending()), Ok(merged_snapshot())],
            queue_rules(),
        );
        let mut seen: Vec<Event> = Vec::new();
        let report = watch(&src, &clock(), &subject(), cfg(), &mut |e| {
            seen.push(e.clone())
        });
        assert_eq!(seen, report.events, "one log, two renderings");
    }
}
