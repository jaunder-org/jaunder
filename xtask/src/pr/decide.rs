//! The state machine. Pure: no IO, no clock, no `gh`.
//!
//! Every rule this command exists to get right lives here and nowhere else, which is
//! what makes them all testable from hand-built values.

use super::Outcome;
use super::snapshot::{
    CheckEntry, CheckState, MergeStateStatus, Mergeable, PrSnapshot, PrState, RequiredChecks,
    RunRef,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Phase {
    AwaitingChecks,
    AwaitingMergeability,
    ReadyToLand,
    Armed,
    Queued,
    Terminal,
}

impl Phase {
    pub fn as_str(self) -> &'static str {
        match self {
            Phase::AwaitingChecks => "awaiting-checks",
            Phase::AwaitingMergeability => "awaiting-mergeability",
            Phase::ReadyToLand => "ready-to-land",
            Phase::Armed => "armed",
            Phase::Queued => "queued",
            Phase::Terminal => "terminal",
        }
    }
}

/// The head observed in the queue. Queue disappearance is meaningful only while
/// observing that same head; a push starts a new history.
#[derive(Debug, Clone, Default)]
pub struct Progress {
    pub queued_head_sha: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Step {
    Continue {
        phase: Phase,
        warn: Option<String>,
    },
    Ready,
    Terminal {
        outcome: Outcome,
        detail: Option<String>,
        pointer: Option<String>,
    },
}

/// Resolve one required context name against the rollup.
///
/// Matches `CheckRun.name` or `StatusContext.context` exactly — no name is ever
/// hardcoded, so this join is the only thing connecting the ruleset to the verdict.
/// When a context appears more than once (a re-run), an **in-flight** entry wins over
/// a completed one: a re-run in progress means the context is not settled, and
/// reading the superseded conclusion would let the machine call a verdict early.
/// Among entries of the same settledness, the latest timestamp wins.
pub fn resolve_context<'a>(checks: &'a [CheckEntry], name: &str) -> Option<&'a CheckEntry> {
    let matching = || checks.iter().filter(|c| c.name == name);
    matching()
        .filter(|c| c.state == CheckState::Pending)
        .max_by(|a, b| a.started_at.cmp(&b.started_at))
        .or_else(|| matching().max_by(|a, b| a.completed_at.cmp(&b.completed_at)))
}

fn required_states<'a>(
    snap: &'a PrSnapshot,
    req: &'a RequiredChecks,
) -> impl Iterator<Item = (&'a str, Option<&'a CheckEntry>)> {
    req.contexts
        .iter()
        .map(|name| (name.as_str(), resolve_context(&snap.checks, name)))
}

/// Whether **every** required context has concluded successfully.
///
/// A context that has not appeared at all is neither success nor failure — which is
/// what keeps a late-appearing aggregate check (the `e2e gate` case) from letting an
/// incomplete set read as complete.
fn all_required_green(snap: &PrSnapshot, req: &RequiredChecks) -> bool {
    required_states(snap, req)
        .all(|(_, entry)| entry.is_some_and(|e| e.state == CheckState::Success))
}

/// Whether to spend a second API call looking for a merge-group run.
///
/// Ejection presupposes having been enqueued, which presupposes green checks — so
/// this stays quiet through the long pre-green phase and only fires in the state
/// that actually needs it.
pub fn needs_ejection_probe(snap: &PrSnapshot, req: &RequiredChecks) -> bool {
    snap.state == PrState::Open
        && !req.contexts.is_empty()
        && req.queue_present
        && !snap.queue.in_queue
        && all_required_green(snap, req)
}

/// Is this run the ejection of the PR's *current* head?
///
/// The run's branch name carries the **base** SHA, not the head, so recency cannot be
/// read off the name. Comparing against `head_committed_at` is what stops a failed
/// merge-group run from a previous push reporting as a fresh ejection.
/// Compared as strings, which is exact for the timestamps GitHub actually returns:
/// REST `created_at` and GraphQL `committedDate` are both Z-normalised, fixed-width
/// RFC 3339 with no fractional seconds, so lexical order is chronological order. If
/// either side ever gains fractional seconds or a numeric offset, parse instead —
/// this would go silently wrong rather than fail.
fn is_ejection(run: &RunRef, snap: &PrSnapshot) -> bool {
    run.conclusion == "failure" && run.created_at > snap.head_committed_at
}

pub fn classify(
    snap: &PrSnapshot,
    req: &RequiredChecks,
    ejection: Option<&RunRef>,
    progress: &Progress,
) -> Step {
    let terminal = |outcome, detail: Option<String>, pointer: Option<String>| Step::Terminal {
        outcome,
        detail,
        pointer,
    };

    // Existing adverse verdicts outrank the approval handoff.
    if snap.state == PrState::Merged {
        return terminal(
            Outcome::Merged,
            snap.merged_at.clone(),
            snap.merge_commit.clone(),
        );
    }
    if snap.state == PrState::Closed {
        return terminal(Outcome::ClosedUnmerged, None, None);
    }
    if snap.mergeable == Mergeable::Conflicting {
        return terminal(
            Outcome::Conflicted,
            Some("the branch conflicts with the base and needs a rebase".into()),
            None,
        );
    }
    if let Some((name, entry)) = required_states(snap, req).find_map(|(name, e)| {
        e.filter(|e| e.state == CheckState::Failure)
            .map(|e| (name, e))
    }) {
        return terminal(
            Outcome::ChecksFailed,
            Some(format!("required check failed: {name}")),
            entry.details_url.clone(),
        );
    }
    if req.strict && snap.merge_state_status == MergeStateStatus::Behind {
        return terminal(
            Outcome::Stale,
            Some("behind the base branch; the strict ruleset blocks the merge".into()),
            None,
        );
    }
    if req.contexts.is_empty() {
        return terminal(
            Outcome::WatcherError,
            Some("the required-check set is empty; readiness cannot be established".into()),
            None,
        );
    }
    if let Some(run) = ejection.filter(|r| is_ejection(r, snap)) {
        return terminal(
            Outcome::Ejected,
            Some("the front-of-queue merge_group run failed".into()),
            Some(run.url.clone()),
        );
    }

    if snap.queue.in_queue {
        return Step::Continue {
            phase: Phase::Queued,
            warn: None,
        };
    }
    if snap.auto_merge_armed {
        return Step::Continue {
            phase: Phase::Armed,
            warn: None,
        };
    }
    if !all_required_green(snap, req) {
        return Step::Continue {
            phase: Phase::AwaitingChecks,
            warn: None,
        };
    }
    if progress.queued_head_sha.as_deref() == Some(snap.head_sha.as_str()) {
        return terminal(
            Outcome::Dequeued,
            Some(
                "the queue entry vanished and no failed current-head merge-group run explains it"
                    .into(),
            ),
            None,
        );
    }
    if snap.mergeable == Mergeable::Unknown || snap.merge_state_status == MergeStateStatus::Unknown
    {
        return Step::Continue {
            phase: Phase::AwaitingMergeability,
            warn: None,
        };
    }
    match snap.merge_state_status {
        MergeStateStatus::Blocked | MergeStateStatus::Draft | MergeStateStatus::Dirty => terminal(
            Outcome::Blocked,
            Some(format!(
                "GitHub reports merge state {}",
                merge_state_label(snap.merge_state_status)
            )),
            None,
        ),
        MergeStateStatus::Clean
        | MergeStateStatus::HasHooks
        | MergeStateStatus::Unstable
        | MergeStateStatus::Behind => Step::Ready,
        MergeStateStatus::Unknown => unreachable!("handled above"),
    }
}

fn merge_state_label(status: MergeStateStatus) -> &'static str {
    match status {
        MergeStateStatus::Behind => "BEHIND",
        MergeStateStatus::Blocked => "BLOCKED",
        MergeStateStatus::Clean => "CLEAN",
        MergeStateStatus::Dirty => "DIRTY",
        MergeStateStatus::Draft => "DRAFT",
        MergeStateStatus::HasHooks => "HAS_HOOKS",
        MergeStateStatus::Unknown => "UNKNOWN",
        MergeStateStatus::Unstable => "UNSTABLE",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::pr::snapshot::{CheckState, MergeStateStatus, Mergeable, PrState, RequiredChecks};
    use crate::pr::test_support::*;

    // ---- terminal outcomes ----

    #[test]
    fn merged_pr_is_terminal_merged_with_the_commit() {
        match classify(
            &merged_snapshot(),
            &queue_rules(),
            None,
            &Progress::default(),
        ) {
            Step::Terminal {
                outcome, pointer, ..
            } => {
                assert_eq!(outcome, Outcome::Merged);
                assert!(pointer.is_some(), "merged must carry the merge commit");
            }
            other => panic!("expected merged, got {other:?}"),
        }
    }

    #[test]
    fn failing_required_check_is_checks_failed_with_its_url() {
        let s = open(vec![
            check(
                "Validate (no e2e)",
                CheckState::Failure,
                "2026-07-30T14:10:00Z",
            ),
            check("e2e gate", CheckState::Pending, ""),
        ]);
        match classify(&s, &queue_rules(), None, &Progress::default()) {
            Step::Terminal {
                outcome,
                pointer,
                detail,
            } => {
                assert_eq!(outcome, Outcome::ChecksFailed);
                assert!(pointer.is_some(), "must point at the failing job log");
                assert!(detail.unwrap().contains("Validate (no e2e)"));
            }
            other => panic!("expected checks-failed, got {other:?}"),
        }
    }

    #[test]
    fn conflicting_pr_is_terminal_conflicted() {
        let mut s = open(green());
        s.mergeable = Mergeable::Conflicting;
        assert!(matches!(
            classify(&s, &queue_rules(), None, &Progress::default()),
            Step::Terminal {
                outcome: Outcome::Conflicted,
                ..
            }
        ));
    }

    #[test]
    fn closed_unmerged_pr_is_terminal() {
        let mut s = open(green());
        s.state = PrState::Closed;
        assert!(matches!(
            classify(&s, &queue_rules(), None, &Progress::default()),
            Step::Terminal {
                outcome: Outcome::ClosedUnmerged,
                ..
            }
        ));
    }

    #[test]
    fn conflict_outranks_a_failed_check() {
        // Ordering rule: report the condition a human must act on first.
        let mut s = open(vec![check(
            "Validate (no e2e)",
            CheckState::Failure,
            "2026-07-30T14:10:00Z",
        )]);
        s.mergeable = Mergeable::Conflicting;
        assert!(matches!(
            classify(&s, &queue_rules(), None, &Progress::default()),
            Step::Terminal {
                outcome: Outcome::Conflicted,
                ..
            }
        ));
    }

    // ---- the traps ----

    #[test]
    fn all_required_green_is_ready_for_approval() {
        assert!(matches!(
            classify(&open(green()), &queue_rules(), None, &Progress::default()),
            Step::Ready
        ));
    }

    #[test]
    fn absent_required_context_is_not_terminal() {
        // `e2e gate` appears late, so "no check pending" is briefly true before it
        // exists. A context that has not appeared satisfies nothing.
        let s = open(vec![check(
            "Validate (no e2e)",
            CheckState::Success,
            "2026-07-30T14:10:00Z",
        )]);
        assert!(matches!(
            classify(&s, &queue_rules(), None, &Progress::default()),
            Step::Continue { .. }
        ));
    }

    #[test]
    fn failing_non_required_check_does_not_fail_the_pr() {
        let mut checks = green();
        checks.push(check(
            "some-optional-lint",
            CheckState::Failure,
            "2026-07-30T14:05:00Z",
        ));
        assert!(matches!(
            classify(&open(checks), &queue_rules(), None, &Progress::default()),
            Step::Ready
        ));
    }

    #[test]
    fn duplicate_context_resolves_to_the_latest_completion() {
        // A red original followed by a green re-run must read green.
        let checks = vec![
            check(
                "Validate (no e2e)",
                CheckState::Failure,
                "2026-07-30T14:10:00Z",
            ),
            check(
                "Validate (no e2e)",
                CheckState::Success,
                "2026-07-30T14:40:00Z",
            ),
            check("e2e gate", CheckState::Success, "2026-07-30T14:20:00Z"),
        ];
        assert_eq!(
            resolve_context(&checks, "Validate (no e2e)").unwrap().state,
            CheckState::Success
        );
        assert!(matches!(
            classify(&open(checks), &queue_rules(), None, &Progress::default()),
            Step::Ready
        ));
    }

    #[test]
    fn behind_is_stale_only_when_the_ruleset_is_strict() {
        let mut s = open(green());
        s.merge_state_status = MergeStateStatus::Behind;
        assert!(
            matches!(
                classify(&s, &strict_rules(), None, &Progress::default()),
                Step::Terminal {
                    outcome: Outcome::Stale,
                    ..
                }
            ),
            "strict ruleset: BEHIND blocks the merge"
        );
        assert!(
            matches!(
                classify(&s, &queue_rules(), None, &Progress::default()),
                Step::Ready
            ),
            "live ruleset is non-strict: BEHIND is not blocking"
        );
    }

    // ---- ejection ----

    #[test]
    fn failed_merge_group_run_newer_than_head_is_ejected_without_history() {
        // Reachable with NO prior observation of the queue entry, so `--once` and a
        // late-starting watch reach the same verdict.
        match classify(
            &open(green()),
            &queue_rules(),
            Some(&ejection("2026-07-30T14:30:00Z")),
            &Progress::default(),
        ) {
            Step::Terminal {
                outcome, pointer, ..
            } => {
                assert_eq!(outcome, Outcome::Ejected);
                assert!(pointer.unwrap().contains("/actions/runs/"));
            }
            other => panic!("expected ejected, got {other:?}"),
        }
    }

    #[test]
    fn stale_merge_group_run_older_than_head_is_not_ejected() {
        // Guards against a false `ejected` on a freshly pushed head.
        let mut s = open(green());
        s.head_committed_at = "2026-07-30T15:00:00Z".into();
        assert!(matches!(
            classify(
                &s,
                &queue_rules(),
                Some(&ejection("2026-07-30T14:30:00Z")),
                &Progress::default()
            ),
            Step::Ready
        ));
    }

    #[test]
    fn successful_merge_group_run_is_not_an_ejection() {
        let mut run = ejection("2026-07-30T14:30:00Z");
        run.conclusion = "success".into();
        assert!(matches!(
            classify(
                &open(green()),
                &queue_rules(),
                Some(&run),
                &Progress::default()
            ),
            Step::Ready
        ));
    }

    #[test]
    fn vanished_same_head_queue_entry_is_dequeued() {
        let progress = Progress {
            queued_head_sha: Some("abc".into()),
        };
        match classify(&open(green()), &queue_rules(), None, &progress) {
            Step::Terminal {
                outcome, detail, ..
            } => {
                assert_eq!(outcome, Outcome::Dequeued);
                let detail = detail.unwrap();
                assert!(detail.contains("queue entry vanished"));
                assert!(detail.contains("no failed current-head merge-group run"));
            }
            other => panic!("expected dequeued, got {other:?}"),
        }
    }

    #[test]
    fn vanished_queue_entry_with_pending_checks_keeps_waiting() {
        let progress = Progress {
            queued_head_sha: Some("abc".into()),
        };
        assert!(matches!(
            classify(&open_pending(), &queue_rules(), None, &progress),
            Step::Continue {
                phase: Phase::AwaitingChecks,
                ..
            }
        ));
    }

    #[test]
    fn green_and_unqueued_is_ready_to_land() {
        assert!(matches!(
            classify(&open(green()), &queue_rules(), None, &Progress::default()),
            Step::Ready
        ));
    }

    // ---- phases & the probe trigger ----

    #[test]
    fn queued_pr_reports_the_queued_phase() {
        assert!(matches!(
            classify(&queued_at(2), &queue_rules(), None, &Progress::default()),
            Step::Continue {
                phase: Phase::Queued,
                ..
            }
        ));
    }

    #[test]
    fn armed_pr_reports_the_armed_phase() {
        assert!(matches!(
            classify(
                &armed_snapshot(),
                &queue_rules(),
                None,
                &Progress::default()
            ),
            Step::Continue {
                phase: Phase::Armed,
                ..
            }
        ));
    }

    #[test]
    fn ejection_probe_fires_only_when_green_open_and_unqueued() {
        // Cost control: never during the long pre-green phase.
        assert!(needs_ejection_probe(&open(green()), &queue_rules()));
        assert!(!needs_ejection_probe(&open_pending(), &queue_rules()));
        assert!(!needs_ejection_probe(&queued_at(1), &queue_rules()));
        assert!(!needs_ejection_probe(&merged_snapshot(), &queue_rules()));
    }

    #[test]
    fn empty_required_set_fails_closed_without_an_ejection_probe() {
        let empty = RequiredChecks {
            contexts: Vec::new(),
            strict: false,
            queue_present: true,
        };
        assert!(!needs_ejection_probe(&open(green()), &empty));
        assert!(matches!(
            classify(&open(green()), &empty, None, &Progress::default()),
            Step::Terminal {
                outcome: Outcome::WatcherError,
                ..
            }
        ));
    }

    #[test]
    fn green_merge_state_matrix_is_explicit() {
        for status in [
            MergeStateStatus::Clean,
            MergeStateStatus::HasHooks,
            MergeStateStatus::Unstable,
            MergeStateStatus::Behind,
        ] {
            let mut snap = open(green());
            snap.merge_state_status = status;
            assert!(
                matches!(
                    classify(&snap, &queue_rules(), None, &Progress::default()),
                    Step::Ready
                ),
                "{status:?} should be ready"
            );
        }

        for status in [
            MergeStateStatus::Blocked,
            MergeStateStatus::Draft,
            MergeStateStatus::Dirty,
        ] {
            let mut snap = open(green());
            snap.merge_state_status = status;
            match classify(&snap, &queue_rules(), None, &Progress::default()) {
                Step::Terminal {
                    outcome, detail, ..
                } => {
                    assert_eq!(outcome, Outcome::Blocked);
                    assert!(detail.unwrap().contains(merge_state_label(status)));
                }
                other => panic!("{status:?} should be blocked, got {other:?}"),
            }
        }
    }

    #[test]
    fn unknown_mergeability_waits_for_a_stable_verdict() {
        let mut unknown_mergeable = open(green());
        unknown_mergeable.mergeable = Mergeable::Unknown;
        let mut unknown_status = open(green());
        unknown_status.merge_state_status = MergeStateStatus::Unknown;
        for snap in [unknown_mergeable, unknown_status] {
            assert!(matches!(
                classify(&snap, &queue_rules(), None, &Progress::default()),
                Step::Continue {
                    phase: Phase::AwaitingMergeability,
                    ..
                }
            ));
        }
    }

    #[test]
    fn queue_history_from_an_old_head_does_not_dequeue_the_new_head() {
        let mut snap = open(green());
        snap.head_sha = "def".into();
        let progress = Progress {
            queued_head_sha: Some("abc".into()),
        };
        assert!(matches!(
            classify(&snap, &queue_rules(), None, &progress),
            Step::Ready
        ));
    }

    #[test]
    fn no_check_name_is_hardcoded() {
        // Rename both contexts and the machine must follow the ruleset.
        let req = RequiredChecks {
            contexts: vec!["Alpha".into()],
            strict: false,
            queue_present: true,
        };
        let s = open(vec![check(
            "Alpha",
            CheckState::Failure,
            "2026-07-30T14:10:00Z",
        )]);
        assert!(matches!(
            classify(&s, &req, None, &Progress::default()),
            Step::Terminal {
                outcome: Outcome::ChecksFailed,
                ..
            }
        ));
    }
}
