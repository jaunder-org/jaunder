//! `pr land`: arm the merge, verify the arm actually took, then watch it home.
//!
//! Running this command *is* the merge approval — which is why arming lives here and
//! not in `watch`. `watch` cannot merge anything no matter how it is invoked.

use super::decide::{self, Progress, Step};
use super::gh::{self, ApiError};
use super::snapshot::{PrSnapshot, PrSource, RequiredChecks, RunRef};
use super::watch::{self, Clock, WatchConfig};
use super::{Event, EventKind, Outcome, PrReport, Subject};

/// Arming is a mutation, so it is a separate capability from observing: no
/// `PrSource` can merge anything, whatever a future caller passes it.
pub trait PrArmer {
    fn arm_auto_merge(&self, subject: &Subject) -> Result<(), ApiError>;
}

pub struct GhArmer;

impl PrArmer for GhArmer {
    fn arm_auto_merge(&self, subject: &Subject) -> Result<(), ApiError> {
        // `run_gh_raw`, not `run_gh`: this prints a human sentence rather than JSON,
        // so parsing stdout would classify every *successful* arm as malformed. Its
        // output is not evidence either way — the next snapshot is.
        let number = subject.number.to_string();
        let repo = format!("{}/{}", subject.owner, subject.repo);
        gh::run_gh_raw(&["pr", "merge", &number, "--repo", &repo, "--auto", "--merge"])
    }
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
/// `land` must not arm a PR it cannot prove was *not* ejected, so a probe failure
/// propagates rather than degrading to "no ejection found" — the difference between
/// refusing to re-enqueue and silently re-enqueueing a failing PR.
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
    }
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
    cfg: WatchConfig,
    sink: &mut dyn FnMut(&Event),
) -> PrReport {
    let mut events: Vec<Event> = Vec::new();

    let req = match source.required_checks(subject) {
        Ok(r) => r,
        Err(e) => {
            return report(
                subject,
                String::new(),
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
                String::new(),
                Outcome::WatcherError,
                Some(format!("could not read the PR: {}", e.detail())),
                None,
                events,
            );
        }
    };
    // Including the ejection probe. Without it an already-ejected PR — open, green,
    // unqueued — classifies as merely not-yet-armed, and `land` would re-enqueue it:
    // the one action this tool explicitly refuses, because requeuing a genuinely
    // failing PR loops forever.
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
    if let Step::Terminal {
        outcome,
        detail,
        pointer,
    } = decide::classify(&snap, &req, ejection.as_ref(), &Progress::default())
    {
        push(
            &mut events,
            sink,
            clock.now_rfc3339(),
            EventKind::Terminal,
            outcome.as_str().into(),
        );
        return report(subject, snap.head_sha, outcome, detail, pointer, events);
    }

    // Arm, then verify against GitHub's own state. `gh pr merge` reports success
    // identically whether it armed or did nothing, so its word is worth nothing here.
    // Armed **or** already queued: a green PR merged into a live queue is enqueued
    // directly, leaving `autoMergeRequest` null — checking only that field would
    // report a working direct enqueue as a failed arm.
    let mut head_sha = snap.head_sha;
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
        if let Err(e) = armer.arm_auto_merge(subject) {
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
        head_sha = after.head_sha.clone();
        if after.auto_merge_armed || after.queue.in_queue {
            let mut watch_cfg = cfg;
            watch_cfg.stop_at_ready = false;
            let mut result = watch::watch(source, clock, subject, watch_cfg, sink);
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
        if let Step::Terminal {
            outcome,
            detail,
            pointer,
        } = decide::classify(&after, &req, after_ejection.as_ref(), &Progress::default())
        {
            push(
                &mut events,
                sink,
                clock.now_rfc3339(),
                EventKind::Terminal,
                outcome.as_str().into(),
            );
            return report(subject, head_sha, outcome, detail, pointer, events);
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
    use crate::pr::snapshot::{Mergeable, RequiredChecks};
    use crate::pr::test_support::*;

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
    }

    impl CountingArmer {
        fn new() -> Self {
            Self {
                calls: std::cell::Cell::new(0),
            }
        }
    }

    impl PrArmer for CountingArmer {
        fn arm_auto_merge(&self, _: &Subject) -> Result<(), ApiError> {
            self.calls.set(self.calls.get() + 1);
            Ok(())
        }
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
        // Re-enqueueing after an ejection is the one action the tool refuses: a
        // genuinely failing PR would loop forever. Without the prologue probe this PR
        // (open, green, unqueued) reads as merely unarmed and gets re-enqueued.
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
