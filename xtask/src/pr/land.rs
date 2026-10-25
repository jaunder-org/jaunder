//! `pr land`: arm the merge, verify the arm actually took, then watch it home.
//!
//! Running this command *is* the merge approval — which is why arming lives here and
//! not in `watch`. `watch` cannot merge anything no matter how it is invoked.

use std::collections::BTreeSet;

use super::decide::{self, OptionalFailure, Progress, Step};
use super::gh::{self, ApiError};
use super::snapshot::{PrSnapshot, PrSource, RequiredChecks, RunRef};
use super::watch::{self, Clock, WatchConfig};
use super::{Event, EventKind, Outcome, PrReport, Subject};

/// Arming is a mutation, so it is a separate capability from observing: no
/// `PrSource` can merge anything, whatever a future caller passes it.
pub trait PrArmer {
    fn arm_auto_merge(&self, subject: &Subject, expected_head: &str) -> Result<(), ApiError>;
}

/// GitHub represents a successful arm in either of two ways: a pending
/// auto-merge request or immediate membership in the live merge queue.
///
/// Keep this predicate shared with privileged automation so every caller
/// verifies the same GitHub factors after `gh pr merge --auto`.
pub(super) fn arm_is_verified(auto_merge_armed: bool, in_merge_queue: bool) -> bool {
    auto_merge_armed || in_merge_queue
}

pub struct GhArmer;

impl PrArmer for GhArmer {
    fn arm_auto_merge(&self, subject: &Subject, expected_head: &str) -> Result<(), ApiError> {
        // `run_gh_raw`, not `run_gh`: this prints a human sentence rather than JSON,
        // so parsing stdout would classify every *successful* arm as malformed. Its
        // output is not evidence either way — the next snapshot is.
        let number = subject.number.to_string();
        let repo = format!("{}/{}", subject.owner, subject.repo);
        gh::run_gh_raw(&merge_args(&number, &repo, expected_head))
    }
}

// Condition the mutation itself: a matching read cannot close the read/write race.
fn merge_args<'a>(number: &'a str, repo: &'a str, expected_head: &'a str) -> [&'a str; 9] {
    [
        "pr",
        "merge",
        number,
        "--repo",
        repo,
        "--auto",
        "--merge",
        "--match-head-commit",
        expected_head,
    ]
}

pub(super) fn head_drift_detail(expected: &str, observed: &str) -> String {
    format!(
        "refusing to land: expected PR head {expected}, observed {observed}; a fresh qualified landing invocation is required"
    )
}

#[derive(Debug, PartialEq, Eq)]
pub enum GuardVerdict {
    Proceed,
    Diverged { local: String, remote: String },
}

/// Refuse to land a PR whose head is not what the caller is looking at.
///
/// Pure — the caller supplies what git said. Only fires when invoked *from the PR's
/// own branch*, where "my local commits" and "what will merge" are easy to conflate;
/// run from anywhere else it is location-agnostic and says nothing.
pub fn divergence_guard(
    current_branch: Option<&str>,
    local_sha: Option<&str>,
    pr_head_ref: &str,
    pr_head_sha: &str,
) -> GuardVerdict {
    match (current_branch, local_sha) {
        (Some(branch), Some(local)) if branch == pr_head_ref && local != pr_head_sha => {
            GuardVerdict::Diverged {
                local: local.to_string(),
                remote: pr_head_sha.to_string(),
            }
        }
        _ => GuardVerdict::Proceed,
    }
}

pub fn divergence_message(local: &str, remote: &str) -> String {
    format!(
        "refusing to land: local HEAD is {local} but the PR head is {remote} — \
         what would merge is not what you are looking at. Push, or pass the PR \
         number from outside its branch."
    )
}

/// The merge-group probe, in the one state where ejection is possible.
///
/// Both ordinary landing and explicit recovery need trustworthy ejection evidence.
/// A probe failure must not degrade to "no ejection found" and authorize a mutation.
fn probe<S: PrSource>(
    source: &S,
    subject: &Subject,
    snap: &PrSnapshot,
    req: &RequiredChecks,
) -> Result<Option<RunRef>, ApiError> {
    if decide::needs_ejection_probe(snap, req) {
        source.ejection_run(subject)
    } else {
        Ok(None)
    }
}

fn push_optional_failures(
    events: &mut Vec<Event>,
    sink: &mut dyn FnMut(&Event),
    at: &str,
    failures: &[OptionalFailure],
    emitted: &mut BTreeSet<String>,
) {
    for failure in failures {
        if emitted.insert(failure.id.clone()) {
            push(
                events,
                sink,
                at.to_string(),
                EventKind::Warning,
                failure.detail.clone(),
            );
        }
    }
}

fn push(
    events: &mut Vec<Event>,
    sink: &mut dyn FnMut(&Event),
    at: String,
    kind: EventKind,
    detail: String,
) {
    let event = Event { at, kind, detail };
    sink(&event);
    events.push(event);
}

fn report(
    subject: &Subject,
    head_sha: String,
    outcome: Outcome,
    detail: Option<String>,
    pointer: Option<String>,
    events: Vec<Event>,
) -> PrReport {
    PrReport {
        outcome,
        pr: subject.number.0,
        head_sha,
        phase: None,
        detail,
        pointer,
        events,
        shared_failure: None,
        subject_failure: None,
    }
}

fn pending_classification_report(
    subject: &Subject,
    head_sha: String,
    incomplete: super::evidence::IncompleteEvidence,
    events: Vec<Event>,
) -> PrReport {
    report(
        subject,
        head_sha,
        Outcome::Pending,
        Some(incomplete.detail()),
        None,
        events,
    )
}

/// Approval-bound head and explicit recovery intent for this invocation only.
pub struct LandRequest<'a> {
    pub expected_head: &'a str,
    pub reenqueue: bool,
}

/// Arm the merge and drive it to a terminal outcome.
///
/// Like [`watch`](super::watch::watch), never returns `Err` — a failure to arm is a
/// report, not an error.
pub fn land<S: PrSource, A: PrArmer, C: Clock>(
    source: &S,
    armer: &A,
    clock: &C,
    subject: &Subject,
    request: LandRequest<'_>,
    cfg: WatchConfig,
    sink: &mut dyn FnMut(&Event),
) -> PrReport {
    let expected_head = request.expected_head;
    let mut events: Vec<Event> = Vec::new();
    let mut emitted_optional_failure_ids = BTreeSet::new();

    let req = match source.required_checks(subject) {
        Ok(r) => r,
        Err(e) => {
            return report(
                subject,
                expected_head.into(),
                Outcome::WatcherError,
                Some(format!("could not read the branch ruleset: {}", e.detail())),
                None,
                events,
            );
        }
    };

    // Look before arming. A PR that cannot merge — conflicted, closed, already red —
    // would otherwise sit in a misleading "armed, waiting" state forever.
    let snap = match source.snapshot(subject) {
        Ok(s) => s,
        Err(e) => {
            return report(
                subject,
                expected_head.into(),
                Outcome::WatcherError,
                Some(format!("could not read the PR: {}", e.detail())),
                None,
                events,
            );
        }
    };
    if snap.head_sha != expected_head {
        return report(
            subject,
            expected_head.into(),
            Outcome::Stale,
            Some(head_drift_detail(expected_head, &snap.head_sha)),
            None,
            events,
        );
    }
    if request.reenqueue && arm_is_verified(snap.auto_merge_armed, snap.queue.in_queue) {
        return report(
            subject,
            snap.head_sha,
            Outcome::Blocked,
            Some("refusing --reenqueue: the PR is already queued or auto-merge armed".into()),
            None,
            events,
        );
    }
    // Recovery requires an observed ejection, never an inference from an unarmed PR.
    // A failed probe cannot authorize either ordinary arming or explicit recovery.
    let ejection = match probe(source, subject, &snap, &req) {
        Ok(run) => run,
        Err(e) => {
            return report(
                subject,
                snap.head_sha,
                Outcome::WatcherError,
                Some(format!(
                    "could not check for a prior ejection, so refusing to arm: {}",
                    e.detail()
                )),
                None,
                events,
            );
        }
    };
    let (mut step, optional_failures) = match watch::classify_snapshot(
        source,
        subject,
        &snap,
        &req,
        ejection.as_ref(),
        &Progress::default(),
    ) {
        Ok(watch::SnapshotClassification::Complete {
            step,
            optional_failures,
        }) => (step, optional_failures),
        Ok(watch::SnapshotClassification::Incomplete(incomplete)) => {
            return pending_classification_report(subject, snap.head_sha, incomplete, events);
        }
        Err(e) => {
            return report(
                subject,
                snap.head_sha,
                Outcome::WatcherError,
                Some(format!(
                    "could not classify failed checks, so refusing to arm: {}",
                    e.detail()
                )),
                None,
                events,
            );
        }
    };
    push_optional_failures(
        &mut events,
        sink,
        &clock.now_rfc3339(),
        &optional_failures,
        &mut emitted_optional_failure_ids,
    );
    if request.reenqueue {
        if !matches!(
            step,
            Step::Terminal {
                outcome: Outcome::Ejected,
                ..
            }
        ) {
            // Explicit recovery must never fall back to the ordinary arming path.
            if !matches!(step, Step::Terminal { .. }) {
                return report(
                    subject,
                    snap.head_sha,
                    Outcome::Blocked,
                    Some("refusing --reenqueue: no prior ejection was established".into()),
                    None,
                    events,
                );
            }
        } else {
            // Ejection precedes readiness in the classifier. Acknowledging it must
            // not hide GitHub's blocked or indeterminate merge state.
            step = decide::classify(&snap, &req, None, &Progress::default());
            if matches!(step, Step::Ready) {
                let Some(old_run) = ejection.as_ref().filter(|run| !run.url.is_empty()) else {
                    return report(
                        subject,
                        snap.head_sha,
                        Outcome::WatcherError,
                        Some("refusing --reenqueue: the failed run has no stable URL".into()),
                        None,
                        events,
                    );
                };
                push(
                    &mut events,
                    sink,
                    clock.now_rfc3339(),
                    EventKind::Phase,
                    format!("reenqueueing once after failed run {}", old_run.url),
                );
                if let Err(e) = armer.arm_auto_merge(subject, expected_head) {
                    return report(
                        subject,
                        snap.head_sha,
                        Outcome::WatcherError,
                        Some(format!("could not arm auto-merge: {}", e.detail())),
                        None,
                        events,
                    );
                }
                // Visibility may lag the mutation. Observe within the normal budget;
                // uncertainty can never acquire another mutation opportunity.
                let mut watch_cfg = cfg;
                watch_cfg.stop_at_ready = false;
                let mut result = watch::watch_with_progress_and_optional_failures(
                    source,
                    clock,
                    subject,
                    watch_cfg,
                    watch::WatchContinuation {
                        progress: Progress::default(),
                        emitted_optional_failure_ids,
                        expected_head: Some(expected_head),
                        acknowledged_ejection: Some(&old_run.url),
                    },
                    sink,
                );
                events.extend(std::mem::take(&mut result.events));
                result.events = events;
                return result;
            }
            if matches!(step, Step::Continue { .. }) {
                return report(
                    subject,
                    snap.head_sha,
                    Outcome::Pending,
                    Some("refusing --reenqueue: readiness is not established".into()),
                    None,
                    events,
                );
            }
        }
    }
    if let Step::Terminal {
        outcome,
        detail,
        pointer,
        subject_failure,
    } = step
    {
        push(
            &mut events,
            sink,
            clock.now_rfc3339(),
            EventKind::Terminal,
            outcome.as_str().into(),
        );
        let mut report = report(subject, snap.head_sha, outcome, detail, pointer, events);
        report.subject_failure = subject_failure;
        return report;
    }

    // Arm, then verify against GitHub's own state. `gh pr merge` reports success
    // identically whether it armed or did nothing, so its word is worth nothing here.
    // Armed **or** already queued: a green PR merged into a live queue is enqueued
    // directly, leaving `autoMergeRequest` null — checking only that field would
    // report a working direct enqueue as a failed arm.
    let head_sha = expected_head.to_string();
    for attempt in 1..=2u32 {
        if attempt == 1 {
            push(
                &mut events,
                sink,
                clock.now_rfc3339(),
                EventKind::Phase,
                "arming auto-merge".into(),
            );
        } else {
            push(
                &mut events,
                sink,
                clock.now_rfc3339(),
                EventKind::Warning,
                "auto-merge did not take; re-arming once".into(),
            );
        }
        if let Err(e) = armer.arm_auto_merge(subject, expected_head) {
            return report(
                subject,
                head_sha,
                Outcome::WatcherError,
                Some(format!("could not arm auto-merge: {}", e.detail())),
                None,
                events,
            );
        }

        // Give GitHub a beat before believing the arm did not take. Reading back
        // immediately races its own write: `autoMergeRequest`/`isInMergeQueue` may not
        // have surfaced yet, and reporting `watcher-error` for a PR that is in fact
        // armed and about to merge is exactly the false signal this command exists to
        // eliminate. Through the injected clock, so tests stay instant.
        clock.sleep_secs(cfg.interval_secs);

        let after = match source.snapshot(subject) {
            Ok(s) => s,
            Err(e) => {
                return report(
                    subject,
                    head_sha,
                    Outcome::WatcherError,
                    Some(format!("could not verify the arm: {}", e.detail())),
                    None,
                    events,
                );
            }
        };
        if after.head_sha != expected_head {
            return report(
                subject,
                head_sha,
                Outcome::Stale,
                Some(head_drift_detail(expected_head, &after.head_sha)),
                None,
                events,
            );
        }
        if arm_is_verified(after.auto_merge_armed, after.queue.in_queue) {
            let mut watch_cfg = cfg;
            watch_cfg.stop_at_ready = false;
            let progress = Progress::from_snapshot(&after);
            let mut result = watch::watch_with_progress_and_optional_failures(
                source,
                clock,
                subject,
                watch_cfg,
                watch::WatchContinuation {
                    progress,
                    emitted_optional_failure_ids,
                    expected_head: Some(expected_head),
                    acknowledged_ejection: None,
                },
                sink,
            );
            // One log: the prologue happened first, so it reads first.
            events.extend(std::mem::take(&mut result.events));
            result.events = events;
            return result;
        }

        // Something may have gone terminal between arming and verifying. Same
        // fail-closed rule as the prologue: an unreadable probe must not degrade to
        // "no ejection found" and let the loop arm again.
        let after_ejection = match probe(source, subject, &after, &req) {
            Ok(run) => run,
            Err(e) => {
                return report(
                    subject,
                    head_sha,
                    Outcome::WatcherError,
                    Some(format!(
                        "could not re-check for an ejection after arming: {}",
                        e.detail()
                    )),
                    None,
                    events,
                );
            }
        };
        let (after_step, optional_failures) = match watch::classify_snapshot(
            source,
            subject,
            &after,
            &req,
            after_ejection.as_ref(),
            &Progress::default(),
        ) {
            Ok(watch::SnapshotClassification::Complete {
                step,
                optional_failures,
            }) => (step, optional_failures),
            Ok(watch::SnapshotClassification::Incomplete(incomplete)) => {
                return pending_classification_report(subject, head_sha, incomplete, events);
            }
            Err(e) => {
                return report(
                    subject,
                    head_sha,
                    Outcome::WatcherError,
                    Some(format!(
                        "could not classify failed checks after arming: {}",
                        e.detail()
                    )),
                    None,
                    events,
                );
            }
        };
        push_optional_failures(
            &mut events,
            sink,
            &clock.now_rfc3339(),
            &optional_failures,
            &mut emitted_optional_failure_ids,
        );
        if let Step::Terminal {
            outcome,
            detail,
            pointer,
            subject_failure,
        } = after_step
        {
            push(
                &mut events,
                sink,
                clock.now_rfc3339(),
                EventKind::Terminal,
                outcome.as_str().into(),
            );
            let mut report = report(subject, head_sha, outcome, detail, pointer, events);
            report.subject_failure = subject_failure;
            return report;
        }
    }

    // Nothing is blocking the PR, yet the arm will not stick: GitHub's reported state
    // and its behaviour disagree. That is a tooling failure, not a PR outcome.
    report(
        subject,
        head_sha,
        Outcome::WatcherError,
        Some(
            "auto-merge would not arm after two attempts and nothing in the PR's state \
             explains why"
                .into(),
        ),
        None,
        events,
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::pr::snapshot::{CheckState, Mergeable, RequiredChecks};
    use crate::pr::test_support::*;

    fn land<S: PrSource, A: PrArmer, C: Clock>(
        source: &S,
        armer: &A,
        clock: &C,
        subject: &Subject,
        cfg: WatchConfig,
        sink: &mut dyn FnMut(&Event),
    ) -> PrReport {
        super::land(
            source,
            armer,
            clock,
            subject,
            LandRequest {
                expected_head: "abc",
                reenqueue: false,
            },
            cfg,
            sink,
        )
    }

    #[test]
    fn reenqueue_old_failure_waits_for_merge_without_rearming() {
        let old = ejection("2026-07-30T14:30:00Z");
        let src = FakeSource::new(
            vec![Ok(open(green())), Ok(open(green())), Ok(merged_snapshot())],
            queue_rules(),
        )
        .with_ejection(Some(old));
        let armer = CountingArmer::new();
        let report = super::land(
            &src,
            &armer,
            &clock(),
            &subject(),
            LandRequest {
                expected_head: "abc",
                reenqueue: true,
            },
            cfg(),
            &mut |_| {},
        );
        assert_eq!(report.outcome, Outcome::Merged);
        assert_eq!(armer.calls.get(), 1);
    }

    fn reenqueue(source: &FakeSource, armer: &impl PrArmer) -> PrReport {
        super::land(
            source,
            armer,
            &clock(),
            &subject(),
            LandRequest {
                expected_head: "abc",
                reenqueue: true,
            },
            cfg(),
            &mut |_| {},
        )
    }

    #[test]
    fn reenqueue_distinguishes_old_failure_from_fresh_pending_pass_and_failure() {
        let old = ejection("2026-07-30T14:30:00Z");
        for (conclusion, expected) in [
            ("", Outcome::Merged),
            ("success", Outcome::Merged),
            ("failure", Outcome::Ejected),
        ] {
            let fresh = RunRef {
                url: "https://github.com/o/r/actions/runs/10".into(),
                conclusion: conclusion.into(),
                ..old.clone()
            };
            let src = FakeSource::new(
                vec![
                    Ok(open(green())),
                    Ok(open(green())),
                    Ok(open(green())),
                    Ok(open(green())),
                    Ok(merged_snapshot()),
                ],
                queue_rules(),
            )
            .with_ejection_script(vec![
                Ok(Some(old.clone())),
                Ok(Some(old.clone())),
                Ok(None),
                Ok(Some(fresh)),
            ]);
            let armer = CountingArmer::new();
            let report = reenqueue(&src, &armer);
            assert_eq!(report.outcome, expected, "{conclusion}");
            assert_eq!(armer.calls.get(), 1);
            assert_eq!(*armer.expected_heads.borrow(), ["abc"]);
            if expected == Outcome::Ejected {
                assert_eq!(
                    report.pointer.as_deref(),
                    Some("https://github.com/o/r/actions/runs/10")
                );
            }
            assert_eq!(old.conclusion, "failure");
        }
    }

    #[test]
    fn reenqueue_uncertainty_or_timeout_never_rearms() {
        let old = ejection("2026-07-30T14:30:00Z");
        for (next, expected) in [
            (Ok(Some(old.clone())), Outcome::TimedOut),
            (Ok(None), Outcome::TimedOut),
            (
                Err(ApiError::Transport("probe unavailable".into())),
                Outcome::WatcherError,
            ),
            (
                Err(ApiError::Malformed("invalid run".into())),
                Outcome::WatcherError,
            ),
        ] {
            let src = FakeSource::new(vec![Ok(open(green()))], queue_rules())
                .with_ejection_script(vec![Ok(Some(old.clone())), next]);
            let armer = CountingArmer::new();
            let report = reenqueue(&src, &armer);
            assert_eq!(report.outcome, expected);
            assert_eq!(armer.calls.get(), 1);
        }
        let src = FakeSource::new(
            vec![
                Ok(open(green())),
                Err(ApiError::Transport("readback unavailable".into())),
            ],
            queue_rules(),
        )
        .with_ejection(Some(old));
        let armer = CountingArmer::new();
        let report = reenqueue(&src, &armer);
        assert_eq!(report.outcome, Outcome::WatcherError);
        assert_eq!(armer.calls.get(), 1);
    }

    #[test]
    fn reenqueue_never_erases_failed_checks_or_retries_a_rejected_mutation() {
        let old = ejection("2026-07-30T14:30:00Z");
        let mut failed = open(green());
        failed.checks[0].state = CheckState::Failure;
        let src = FakeSource::new(vec![Ok(open(green())), Ok(failed)], queue_rules())
            .with_ejection(Some(old.clone()));
        let armer = CountingArmer::new();
        assert_eq!(reenqueue(&src, &armer).outcome, Outcome::ChecksFailed);
        assert_eq!(armer.calls.get(), 1);

        struct RejectHead(std::cell::Cell<u32>);
        impl PrArmer for RejectHead {
            fn arm_auto_merge(&self, _: &Subject, expected: &str) -> Result<(), ApiError> {
                assert_eq!(expected, "abc");
                self.0.set(self.0.get() + 1);
                Err(ApiError::Transport(
                    "head does not match replacement".into(),
                ))
            }
        }
        let src = FakeSource::new(vec![Ok(open(green()))], queue_rules()).with_ejection(Some(old));
        let armer = RejectHead(std::cell::Cell::new(0));
        let report = reenqueue(&src, &armer);
        assert_eq!(report.outcome, Outcome::WatcherError);
        assert!(report.detail.unwrap().contains("head does not match"));
        assert_eq!(armer.0.get(), 1);
    }

    #[test]
    fn reenqueue_requires_ejection_and_refuses_queued_or_armed() {
        for snap in [
            open(green()),
            open_pending(),
            queued_at(1),
            armed_snapshot(),
        ] {
            let src = FakeSource::new(vec![Ok(snap)], queue_rules());
            let armer = CountingArmer::new();
            assert_eq!(reenqueue(&src, &armer).outcome, Outcome::Blocked);
            assert_eq!(armer.calls.get(), 0);
        }
    }

    #[test]
    fn reenqueue_preserves_prearm_guards() {
        use crate::pr::snapshot::MergeStateStatus;
        let mut red = open(green());
        red.checks[0].state = CheckState::Failure;
        let mut conflicting = open(green());
        conflicting.mergeable = Mergeable::Conflicting;
        let mut drift = open(green());
        drift.head_sha = "replacement".into();
        let mut blocked = open(green());
        blocked.merge_state_status = MergeStateStatus::Blocked;
        let mut unknown = open(green());
        unknown.mergeable = Mergeable::Unknown;
        let mut behind = open(green());
        behind.merge_state_status = MergeStateStatus::Behind;
        for (snap, rules, expected) in [
            (red, queue_rules(), Outcome::ChecksFailed),
            (conflicting, queue_rules(), Outcome::Conflicted),
            (drift, queue_rules(), Outcome::Stale),
            (blocked, queue_rules(), Outcome::Blocked),
            (unknown, queue_rules(), Outcome::Pending),
            (behind, strict_rules(), Outcome::Stale),
            (
                open(green()),
                RequiredChecks {
                    contexts: vec![],
                    ..queue_rules()
                },
                Outcome::WatcherError,
            ),
        ] {
            let src = FakeSource::new(vec![Ok(snap)], rules)
                .with_ejection(Some(ejection("2026-07-30T14:30:00Z")));
            let armer = CountingArmer::new();
            assert_eq!(reenqueue(&src, &armer).outcome, expected);
            assert_eq!(armer.calls.get(), 0);
        }
        let src = FakeSource::new(vec![Ok(open(green()))], queue_rules())
            .with_ejection_error(ApiError::Transport("unknown prior run".into()));
        let armer = CountingArmer::new();
        assert_eq!(reenqueue(&src, &armer).outcome, Outcome::WatcherError);
        assert_eq!(armer.calls.get(), 0);
    }

    #[test]
    fn reenqueue_requires_stable_failed_run_identity() {
        let mut old = ejection("2026-07-30T14:30:00Z");
        old.url.clear();
        let src = FakeSource::new(vec![Ok(open(green()))], queue_rules()).with_ejection(Some(old));
        let armer = CountingArmer::new();
        let report = reenqueue(&src, &armer);
        assert_eq!(report.outcome, Outcome::WatcherError);
        assert_eq!(armer.calls.get(), 0);
    }

    #[test]
    fn reenqueue_preserves_postarm_head_and_queue_guards() {
        let mut replacement = merged_snapshot();
        replacement.head_sha = "replacement".into();
        for (snaps, expected) in [
            (vec![Ok(open(green())), Ok(replacement)], Outcome::Stale),
            (
                vec![Ok(open(green())), Ok(queued_at(1)), Ok(open(green()))],
                Outcome::Dequeued,
            ),
        ] {
            let src = FakeSource::new(snaps, queue_rules())
                .with_ejection(Some(ejection("2026-07-30T14:30:00Z")));
            let armer = CountingArmer::new();
            assert_eq!(reenqueue(&src, &armer).outcome, expected);
            assert_eq!(armer.calls.get(), 1);
        }
    }

    // ---- the divergence guard ----

    #[test]
    fn matching_branch_and_sha_proceeds() {
        assert_eq!(
            divergence_guard(Some("feature"), Some("abc"), "feature", "abc"),
            GuardVerdict::Proceed
        );
    }

    #[test]
    fn same_branch_different_sha_is_divergence() {
        match divergence_guard(Some("feature"), Some("local1"), "feature", "remote2") {
            GuardVerdict::Diverged { local, remote } => {
                assert_eq!(local, "local1");
                assert_eq!(remote, "remote2");
            }
            other => panic!("expected divergence, got {other:?}"),
        }
    }

    #[test]
    fn a_different_branch_is_location_agnostic() {
        assert_eq!(
            divergence_guard(Some("main"), Some("zzz"), "feature", "abc"),
            GuardVerdict::Proceed
        );
    }

    #[test]
    fn detached_head_or_no_git_proceeds() {
        assert_eq!(
            divergence_guard(None, None, "feature", "abc"),
            GuardVerdict::Proceed
        );
    }

    #[test]
    fn the_refusal_message_names_both_shas() {
        let m = divergence_message("local1", "remote2");
        assert!(m.contains("local1"), "must name the local sha: {m}");
        assert!(m.contains("remote2"), "must name the PR head sha: {m}");
    }

    // ---- the arming prologue ----

    struct CountingArmer {
        calls: std::cell::Cell<u32>,
        expected_heads: std::cell::RefCell<Vec<String>>,
    }

    impl CountingArmer {
        fn new() -> Self {
            Self {
                calls: std::cell::Cell::new(0),
                expected_heads: std::cell::RefCell::new(Vec::new()),
            }
        }
    }

    impl PrArmer for CountingArmer {
        fn arm_auto_merge(&self, _: &Subject, expected_head: &str) -> Result<(), ApiError> {
            self.expected_heads.borrow_mut().push(expected_head.into());
            assert_eq!(expected_head, "abc");
            self.calls.set(self.calls.get() + 1);
            Ok(())
        }
    }

    #[test]
    fn production_merge_request_conditions_the_mutation_on_the_approved_head() {
        assert_eq!(
            merge_args("731", "owner/repo", "approved-A"),
            [
                "pr",
                "merge",
                "731",
                "--repo",
                "owner/repo",
                "--auto",
                "--merge",
                "--match-head-commit",
                "approved-A",
            ]
        );
    }

    #[test]
    fn changed_arm_readback_never_verifies_or_retries_a_replacement() {
        for mut replacement in [
            open_pending(),
            armed_snapshot(),
            queued_at(1),
            merged_snapshot(),
        ] {
            replacement.head_sha = "replacement-B".into();
            let src = FakeSource::new(vec![Ok(open_pending()), Ok(replacement)], queue_rules());
            let armer = CountingArmer::new();
            let report = land(&src, &armer, &clock(), &subject(), cfg(), &mut |_| {});
            assert_eq!(report.outcome, Outcome::Stale);
            assert_eq!(report.head_sha, "abc");
            let detail = report.detail.unwrap();
            assert!(detail.contains("abc") && detail.contains("replacement-B"));
            assert_eq!(armer.calls.get(), 1);
        }
    }

    #[test]
    fn changed_head_after_verified_arm_cannot_land_a_replacement() {
        let mut replacement = merged_snapshot();
        replacement.head_sha = "replacement-B".into();
        let src = FakeSource::new(
            vec![Ok(open_pending()), Ok(armed_snapshot()), Ok(replacement)],
            queue_rules(),
        );
        let armer = CountingArmer::new();
        let report = land(&src, &armer, &clock(), &subject(), cfg(), &mut |_| {});
        assert_eq!(report.outcome, Outcome::Stale);
        assert_eq!(report.head_sha, "abc");
        assert!(report.detail.unwrap().contains("replacement-B"));
        assert_eq!(armer.calls.get(), 1);
    }

    #[test]
    fn unreadable_prearm_snapshot_never_authorizes_mutation() {
        let src = FakeSource::new(
            vec![Err(ApiError::Malformed("unreadable".into()))],
            queue_rules(),
        );
        let armer = CountingArmer::new();
        let report = land(&src, &armer, &clock(), &subject(), cfg(), &mut |_| {});
        assert_eq!(report.outcome, Outcome::WatcherError);
        assert_eq!(report.head_sha, "abc");
        assert_eq!(armer.calls.get(), 0);
    }

    #[test]
    fn unchanged_head_initial_arm_is_verified_and_merged() {
        let src = FakeSource::new(
            vec![
                Ok(open_pending()),
                Ok(armed_snapshot()),
                Ok(merged_snapshot()),
            ],
            queue_rules(),
        );
        let armer = CountingArmer::new();
        let report = land(&src, &armer, &clock(), &subject(), cfg(), &mut |_| {});
        assert_eq!(report.outcome, Outcome::Merged);
        assert_eq!(report.head_sha, "abc");
        assert_eq!(*armer.expected_heads.borrow(), ["abc"]);
    }

    #[test]
    fn unreadable_arm_readback_never_authorizes_a_retry() {
        let src = FakeSource::new(
            vec![
                Ok(open_pending()),
                Err(ApiError::Transport("readback unavailable".into())),
            ],
            queue_rules(),
        );
        let armer = CountingArmer::new();
        let report = land(&src, &armer, &clock(), &subject(), cfg(), &mut |_| {});
        assert_eq!(report.outcome, Outcome::WatcherError);
        assert_eq!(report.head_sha, "abc");
        assert!(report.detail.unwrap().contains("readback unavailable"));
        assert_eq!(armer.calls.get(), 1);
    }

    #[test]
    fn mutation_boundary_head_mismatch_fails_without_retry() {
        struct ConditionalArmer(std::cell::RefCell<Vec<String>>);
        impl PrArmer for ConditionalArmer {
            fn arm_auto_merge(&self, _: &Subject, expected_head: &str) -> Result<(), ApiError> {
                self.0.borrow_mut().push(expected_head.into());
                // GitHub's head changed after the matching pre-arm read.
                if expected_head != "replacement-B" {
                    return Err(ApiError::Transport(
                        "head does not match replacement-B".into(),
                    ));
                }
                Ok(())
            }
        }
        let src = FakeSource::new(vec![Ok(open_pending())], queue_rules());
        let armer = ConditionalArmer(std::cell::RefCell::new(Vec::new()));
        let report = land(&src, &armer, &clock(), &subject(), cfg(), &mut |_| {});
        assert_eq!(report.outcome, Outcome::WatcherError);
        assert_eq!(report.head_sha, "abc");
        assert_eq!(*armer.0.borrow(), ["abc"]);
        assert!(report.detail.unwrap().contains("head does not match"));
    }

    #[test]
    fn a_silent_no_op_arm_is_retried_once_then_succeeds() {
        // Snapshots: prologue, after arm #1 (NOT armed), after arm #2 (armed), merged.
        let src = FakeSource::new(
            vec![
                Ok(open_pending()),
                Ok(open_pending()),
                Ok(armed_snapshot()),
                Ok(merged_snapshot()),
            ],
            queue_rules(),
        );
        let armer = CountingArmer::new();
        let report = land(&src, &armer, &clock(), &subject(), cfg(), &mut |_| {});
        assert_eq!(
            armer.calls.get(),
            2,
            "a silent no-op must be re-armed exactly once"
        );
        assert_eq!(*armer.expected_heads.borrow(), ["abc", "abc"]);
        assert_eq!(report.head_sha, "abc");
        assert_eq!(report.outcome, Outcome::Merged);
    }

    #[test]
    fn a_direct_enqueue_is_not_a_failed_arm() {
        // Green PR + live queue: `autoMergeRequest` stays null while `isInMergeQueue`
        // goes true. The narrower predicate would call this a failed arm.
        let src = FakeSource::new(
            vec![Ok(open_pending()), Ok(queued_at(1)), Ok(merged_snapshot())],
            queue_rules(),
        );
        let armer = CountingArmer::new();
        let report = land(&src, &armer, &clock(), &subject(), cfg(), &mut |_| {});
        assert_eq!(
            armer.calls.get(),
            1,
            "a direct enqueue must not trigger a re-arm"
        );
        assert_eq!(report.outcome, Outcome::Merged);
    }

    #[test]
    fn queue_disappearance_confirmation_retains_landing_head_guard() {
        let mut replacement_merged = merged_snapshot();
        replacement_merged.head_sha = "replacement".into();
        let mut replacement_open = open(green());
        replacement_open.head_sha = "replacement".into();
        for (confirmation, expected) in [
            (merged_snapshot(), Outcome::Merged),
            (replacement_merged, Outcome::Stale),
            (replacement_open, Outcome::Stale),
        ] {
            let src = FakeSource::new(
                vec![
                    Ok(open(green())),
                    Ok(queued_at(1)),
                    Ok(open(green())),
                    Ok(confirmation),
                ],
                queue_rules(),
            );
            let armer = CountingArmer::new();
            let report = land(&src, &armer, &clock(), &subject(), cfg(), &mut |_| {});
            assert_eq!(report.outcome, expected);
            assert_eq!(report.head_sha, "abc");
            assert_eq!(*armer.expected_heads.borrow(), ["abc"]);
            assert_eq!(armer.calls.get(), 1, "confirmation never re-arms");
        }
    }

    #[test]
    fn verified_queue_entry_disappearance_is_dequeued() {
        let src = FakeSource::new(
            vec![Ok(open(green())), Ok(queued_at(1)), Ok(open(green()))],
            queue_rules(),
        );
        let armer = CountingArmer::new();
        let report = land(&src, &armer, &clock(), &subject(), cfg(), &mut |_| {});
        assert_eq!(armer.calls.get(), 1);
        assert_eq!(report.outcome, Outcome::Dequeued);
    }

    #[test]
    fn a_terminally_bad_pr_is_reported_without_arming() {
        let mut bad = open_pending();
        bad.mergeable = Mergeable::Conflicting;
        let src = FakeSource::new(vec![Ok(bad)], queue_rules());
        let armer = CountingArmer::new();
        let report = land(&src, &armer, &clock(), &subject(), cfg(), &mut |_| {});
        assert_eq!(report.outcome, Outcome::Conflicted);
        assert_eq!(armer.calls.get(), 0, "never arm a PR that cannot merge");
    }

    #[test]
    fn an_already_ejected_pr_is_reported_and_never_re_armed() {
        // Ordinary landing must refuse recovery: a genuinely failing PR could loop
        // forever if open, green and unqueued were sufficient to authorize rearming.
        let src = FakeSource::new(vec![Ok(open(green()))], queue_rules())
            .with_ejection(Some(ejection("2026-07-30T14:30:00Z")));
        let armer = CountingArmer::new();
        let report = land(&src, &armer, &clock(), &subject(), cfg(), &mut |_| {});
        assert_eq!(report.outcome, Outcome::Ejected);
        assert_eq!(armer.calls.get(), 0, "must not re-enqueue an ejected PR");
    }

    #[test]
    fn an_unreadable_ejection_probe_refuses_to_arm() {
        // "Cannot prove it was not ejected" must fail closed, not proceed.
        let src = FakeSource::new(vec![Ok(open(green()))], queue_rules())
            .with_ejection_error(ApiError::Transport("probe down".into()));
        let armer = CountingArmer::new();
        let report = land(&src, &armer, &clock(), &subject(), cfg(), &mut |_| {});
        assert_eq!(report.outcome, Outcome::WatcherError);
        assert_eq!(armer.calls.get(), 0);
        assert!(report.detail.unwrap().contains("refusing to arm"));
    }

    #[test]
    fn an_already_merged_pr_is_a_no_op() {
        let src = FakeSource::new(vec![Ok(merged_snapshot())], queue_rules());
        let armer = CountingArmer::new();
        assert_eq!(
            land(&src, &armer, &clock(), &subject(), cfg(), &mut |_| {}).outcome,
            Outcome::Merged
        );
        assert_eq!(armer.calls.get(), 0);
    }

    #[test]
    fn an_unexplained_failed_arm_is_watcher_error() {
        // Nothing blocking, yet neither arm sticks: reported state and behaviour
        // disagree, which is a tooling failure rather than a verdict about the PR.
        let src = FakeSource::new(vec![Ok(open_pending())], queue_rules());
        let armer = CountingArmer::new();
        let report = land(&src, &armer, &clock(), &subject(), cfg(), &mut |_| {});
        assert_eq!(report.outcome, Outcome::WatcherError);
        assert_eq!(armer.calls.get(), 2, "arm, then exactly one re-arm");
        assert!(report.detail.unwrap().contains("arm"));
    }
    #[test]
    fn empty_required_set_refuses_before_arming() {
        let empty = RequiredChecks {
            contexts: Vec::new(),
            strict: false,
            queue_present: true,
        };
        let src = FakeSource::new(vec![Ok(open(green()))], empty);
        let armer = CountingArmer::new();
        let report = land(&src, &armer, &clock(), &subject(), cfg(), &mut |_| {});
        assert_eq!(report.outcome, Outcome::WatcherError);
        assert_eq!(armer.calls.get(), 0);
    }

    #[test]
    fn incomplete_classification_is_pending_and_never_arms() {
        let rules = RequiredChecks {
            contexts: vec!["Aggregate verdict".into()],
            strict: false,
            queue_present: false,
        };
        let snap = open(vec![actions_check(
            "Arbitrarily renamed lane",
            9,
            CheckState::Failure,
            "2026-07-30T14:10:00Z",
        )]);
        let source = r#"
            jobs:
              lane:
                name: Arbitrarily renamed lane
              aggregate:
                name: Aggregate verdict
                needs: lane
        "#;
        let src = FakeSource::new(vec![Ok(snap)], rules).with_actions_evidence(
            active_actions_evidence("abc", source, vec![("Arbitrarily renamed lane", 9)]),
        );
        let armer = CountingArmer::new();
        let report = land(&src, &armer, &clock(), &subject(), cfg(), &mut |_| {});
        assert_eq!(report.outcome, Outcome::Pending);
        assert_eq!(armer.calls.get(), 0);
        assert!(
            report
                .detail
                .as_deref()
                .is_some_and(|detail| detail.contains("Aggregate verdict"))
        );
    }

    #[test]
    fn transitive_failure_refuses_to_arm_without_a_workflow_mutation() {
        let rules = RequiredChecks {
            contexts: vec!["Aggregate verdict".into()],
            strict: false,
            queue_present: false,
        };
        let snap = open(vec![
            actions_check(
                "Arbitrarily renamed lane",
                9,
                CheckState::Failure,
                "2026-07-30T14:10:00Z",
            ),
            check("Aggregate verdict", CheckState::Pending, ""),
        ]);
        let source = r#"
            jobs:
              lane:
                name: Arbitrarily renamed lane
              aggregate:
                name: Aggregate verdict
                needs: lane
        "#;
        let src = FakeSource::new(vec![Ok(snap)], rules).with_actions_evidence(actions_evidence(
            "abc",
            source,
            vec![("Arbitrarily renamed lane", 9), ("Aggregate verdict", 10)],
        ));
        let armer = CountingArmer::new();
        let report = land(&src, &armer, &clock(), &subject(), cfg(), &mut |_| {});
        assert_eq!(report.outcome, Outcome::ChecksFailed);
        assert_eq!(
            armer.calls.get(),
            0,
            "watch evidence must not arm or cancel anything"
        );
    }

    #[test]
    fn land_reports_optional_failures_before_and_after_arming_once() {
        let rules = RequiredChecks {
            contexts: vec!["Aggregate verdict".into()],
            strict: false,
            queue_present: false,
        };
        let optional = || {
            open(vec![
                actions_check(
                    "Optional diagnostics",
                    41,
                    CheckState::Failure,
                    "2026-07-30T14:10:00Z",
                ),
                check("Aggregate verdict", CheckState::Pending, ""),
            ])
        };
        let source = r#"
            jobs:
              aggregate:
                name: Aggregate verdict
              diagnostics:
                name: Optional diagnostics
        "#;
        let evidence = || {
            actions_evidence(
                "abc",
                source,
                vec![("Aggregate verdict", 40), ("Optional diagnostics", 41)],
            )
        };
        let src = FakeSource::new(
            vec![
                Ok(optional()),
                Ok(optional()),
                Ok(PrSnapshot {
                    auto_merge_armed: true,
                    ..open(vec![check("Aggregate verdict", CheckState::Pending, "")])
                }),
                Ok(merged_snapshot()),
            ],
            rules,
        )
        .with_actions_evidence_script(vec![Ok(evidence()), Ok(evidence())]);
        let armer = CountingArmer::new();
        let report = land(&src, &armer, &clock(), &subject(), cfg(), &mut |_| {});
        assert_eq!(report.outcome, Outcome::Merged);
        assert_eq!(armer.calls.get(), 2);
        assert_eq!(
            report
                .events
                .iter()
                .filter(|event| {
                    event.kind == EventKind::Warning
                        && event
                            .detail
                            .contains("optional check failed: Optional diagnostics")
                })
                .count(),
            1,
            "the same optional attempt is visible but never duplicated"
        );
    }

    #[test]
    fn post_arm_watch_stops_for_a_transitive_failure_without_rearming() {
        let rules = RequiredChecks {
            contexts: vec!["Aggregate verdict".into()],
            strict: false,
            queue_present: false,
        };
        let pending = open(vec![check("Aggregate verdict", CheckState::Pending, "")]);
        let failed_after_arm = PrSnapshot {
            auto_merge_armed: true,
            ..open(vec![
                actions_check(
                    "Renamed post-arm lane",
                    19,
                    CheckState::Failure,
                    "2026-07-30T14:10:00Z",
                ),
                check("Aggregate verdict", CheckState::Pending, ""),
            ])
        };
        let source = r#"
            jobs:
              lane:
                name: Renamed post-arm lane
              aggregate:
                name: Aggregate verdict
                needs: lane
        "#;
        let src = FakeSource::new(vec![Ok(pending), Ok(failed_after_arm)], rules)
            .with_actions_evidence(actions_evidence(
                "abc",
                source,
                vec![("Renamed post-arm lane", 19), ("Aggregate verdict", 20)],
            ));
        let armer = CountingArmer::new();
        let report = land(&src, &armer, &clock(), &subject(), cfg(), &mut |_| {});
        assert_eq!(report.outcome, Outcome::ChecksFailed);
        assert_eq!(
            armer.calls.get(),
            1,
            "the watcher must not retry or mutate CI"
        );
    }

    #[test]
    fn approved_land_continues_if_the_arm_temporarily_disappears() {
        let src = FakeSource::new(
            vec![
                Ok(open(green())),
                Ok(armed_snapshot()),
                Ok(open(green())),
                Ok(merged_snapshot()),
            ],
            queue_rules(),
        );
        let armer = CountingArmer::new();
        let report = land(&src, &armer, &clock(), &subject(), cfg(), &mut |_| {});
        assert_eq!(armer.calls.get(), 1);
        assert_eq!(report.outcome, Outcome::Merged);
    }
}
