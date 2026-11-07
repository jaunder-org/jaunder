//! Read-only command-level setup for the observer.
//!
//! The binary resolves its subject and enters the sole shared poller, including
//! read-only head-bound continuation. No armer or cleanup capability is accepted.

use anyhow::{Result, anyhow};

use super::decide::Progress;
use super::gh::{ApiError, Deadline};
use super::snapshot::{PrSource, SharedFailureEvidenceSource, resolution_failure};
use super::watch::{self, Clock, WatchConfig};
use super::{Event, Outcome, PrNumber, PrReport, Subject};

/// Resolve the requested PR, establish a first snapshot, then enter the shared
/// read-only watch state machine. Landing and checkout mutation are not parameters.
pub fn observe<S, C>(
    source: &S,
    clock: &C,
    number: Option<u64>,
    config: WatchConfig,
    mut continuation: watch::WatchContinuation<'_>,
    sink: &mut dyn FnMut(&Event),
) -> Result<PrReport>
where
    S: PrSource + SharedFailureEvidenceSource,
    C: Clock,
{
    let expected_head = continuation.expected_head;
    // Queue history is a caller-supplied positive observation, not authority to
    // act or evidence about another head. Validate before any GitHub read.
    if let Some(queued) = continuation.progress.queued_head_sha.as_deref()
        && expected_head != Some(queued)
    {
        return Err(anyhow!("observed queued head must equal expected head"));
    }
    let subject = match source.resolve(number.map(PrNumber)) {
        Ok(subject) => subject,
        Err(error) => {
            return match resolution_failure(&error) {
                super::snapshot::ResolutionFailure::Bail(message) => Err(anyhow!(
                    "{message}{}",
                    match number {
                        Some(number) => format!(" (requested PR #{number})"),
                        None => String::new(),
                    }
                )),
                super::snapshot::ResolutionFailure::Report(outcome) => Ok(empty_report(
                    number.unwrap_or_default(),
                    outcome,
                    error.detail(),
                )),
            };
        }
    };

    let established = establish_snapshot(source, clock, &subject);
    if let Err(error) = &established
        && matches!(
            resolution_failure(error),
            super::snapshot::ResolutionFailure::Bail(_)
        )
    {
        return Err(anyhow!(
            "no such pull request: #{} in {}/{}",
            subject.number,
            subject.owner,
            subject.repo
        ));
    }

    // Establishment is already an observation: do not discard a replacement
    // head just because a later read might happen to return the expected one.
    if let (Some(expected), Ok(snapshot)) = (expected_head, &established)
        && snapshot.head_sha != expected
    {
        let mut report = empty_report(
            subject.number.0,
            Outcome::Stale,
            super::decide::head_drift_detail(expected, &snapshot.head_sha),
        );
        report.head_sha = expected.into();
        return Ok(report);
    }

    // Reuse the same continuation and classifier as the watch loop. These inputs
    // constrain read-only evidence; neither grants an arming capability.
    if continuation.progress.queued_head_sha.is_none() {
        continuation.progress = established
            .as_ref()
            .map_or_else(|_| Progress::default(), Progress::from_snapshot);
    }
    let mut report = watch::watch_with_progress_and_optional_failures(
        source,
        clock,
        &subject,
        config,
        continuation,
        sink,
    );
    if report.outcome == Outcome::ChecksFailed && report.subject_failure.is_some() {
        enrich_shared_failure(source, &subject, &mut report, Deadline::ten_seconds());
    }
    Ok(report)
}

/// Establish initial queue history within the observer's bounded read policy.
pub fn establish_snapshot<S: PrSource, C: Clock>(
    source: &S,
    clock: &C,
    subject: &Subject,
) -> Result<super::snapshot::PrSnapshot, ApiError> {
    let mut established = source.snapshot(subject);
    for _ in 1..3 {
        let wait = match &established {
            Err(ApiError::RateLimited { reset_unix }) => reset_unix
                .map(|reset| reset.saturating_sub(clock.now_unix()).min(60))
                .unwrap_or(5),
            Err(error) if error.is_transient() => 2,
            _ => break,
        };
        clock.sleep_secs(wait.max(1));
        established = source.snapshot(subject);
    }
    established
}

fn empty_report(number: u64, outcome: Outcome, detail: String) -> PrReport {
    PrReport {
        outcome,
        pr: number,
        head_sha: String::new(),
        phase: None,
        detail: Some(detail),
        pointer: None,
        events: Vec::new(),
        shared_failure: None,
        subject_failure: None,
    }
}

/// Add best-effort read-only failure context without changing the terminal verdict.
pub fn enrich_shared_failure<S: SharedFailureEvidenceSource>(
    source: &S,
    subject: &Subject,
    report: &mut PrReport,
    deadline: Deadline,
) {
    if deadline.check().is_err() {
        return;
    }
    let Some(failure) = report.subject_failure.as_ref() else {
        return;
    };
    let Ok(evidence) = source.shared_failure_evidence(subject, failure, &deadline) else {
        return;
    };
    let mut check = || {
        deadline
            .check()
            .map_err(|_| super::shared_failure::PolicyError::Cancelled)
    };
    if let Ok(annotation) = super::shared_failure::shared_failure_with_check(
        &evidence.subject_log,
        evidence.selected_runs,
        evidence.jobs,
        &mut check,
    ) {
        report.shared_failure = annotation;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::pr::test_support::{FakeSource, cfg, clock, green, open, queue_rules};

    fn continuation<'a>(
        expected_head: Option<&'a str>,
        acknowledged_ejection: Option<&'a str>,
    ) -> watch::WatchContinuation<'a> {
        watch::WatchContinuation {
            progress: Progress::default(),
            emitted_optional_failure_ids: Default::default(),
            expected_head,
            acknowledged_ejection,
        }
    }

    #[test]
    fn positive_queued_action_readback_survives_unqueued_establishment() {
        // The native action readback saw this head queued, but the first new
        // observer snapshot is already unqueued. Keep that known transition.
        let source = FakeSource::new(vec![Ok(open(green()))], queue_rules());
        let mut continued = continuation(Some("abc"), None);
        continued.progress.queued_head_sha = Some("abc".into());
        let report = observe(
            &source,
            &clock(),
            Some(731),
            WatchConfig {
                stop_at_ready: false,
                ..cfg()
            },
            continued,
            &mut |_| {},
        )
        .unwrap();
        assert_eq!(report.outcome, Outcome::Dequeued);
        assert_eq!(report.head_sha, "abc");

        // With no supplied positive queue observation, default behavior is unchanged.
        let source = FakeSource::new(vec![Ok(open(green()))], queue_rules());
        let report = observe(
            &source,
            &clock(),
            Some(731),
            cfg(),
            continuation(Some("abc"), None),
            &mut |_| {},
        )
        .unwrap();
        assert_eq!(report.outcome, Outcome::ReadyToLand);
    }

    #[test]
    fn queue_continuation_rejects_missing_or_mismatched_expected_head() {
        for expected in [None, Some("replacement")] {
            let source = FakeSource::new(vec![Ok(open(green()))], queue_rules());
            let mut continued = continuation(expected, None);
            continued.progress.queued_head_sha = Some("abc".into());
            let error =
                observe(&source, &clock(), Some(731), cfg(), continued, &mut |_| {}).unwrap_err();
            assert!(
                error
                    .to_string()
                    .contains("queued head must equal expected head")
            );
        }
    }

    #[test]
    fn queue_continuation_preserves_head_merge_and_fresh_failure_guards() {
        use crate::pr::test_support::{ejection, merged_snapshot};
        let historical = ejection("2026-07-30T14:30:00Z");
        let mut fresh = historical.clone();
        fresh.url.push('9');
        for (mut snapshot, drift, expected) in [
            (merged_snapshot(), false, Outcome::Merged),
            (merged_snapshot(), true, Outcome::Stale),
            (open(green()), false, Outcome::Ejected),
        ] {
            if drift {
                snapshot.head_sha = "replacement".into();
            }
            let source = FakeSource::new(vec![Ok(snapshot)], queue_rules())
                .with_ejection(Some(fresh.clone()));
            let mut continued = continuation(Some("abc"), Some(&historical.url));
            continued.progress.queued_head_sha = Some("abc".into());
            let report =
                observe(&source, &clock(), Some(731), cfg(), continued, &mut |_| {}).unwrap();
            assert_eq!(report.outcome, expected);
            if expected == Outcome::Ejected {
                assert_eq!(report.pointer.as_deref(), Some(fresh.url.as_str()));
            }
        }
    }

    #[test]
    fn continuation_acknowledges_only_historical_run_and_preserves_fresh_failure() {
        use crate::pr::test_support::{ejection, merged_snapshot};
        let historical = ejection("2026-07-30T14:30:00Z");
        let mut fresh = historical.clone();
        fresh.url = "https://github.com/jaunder-org/jaunder/actions/runs/999".into();
        let source = FakeSource::new(
            vec![Ok(open(green())), Ok(open(green())), Ok(open(green()))],
            queue_rules(),
        )
        .with_ejection_script(vec![Ok(Some(historical.clone())), Ok(Some(fresh.clone()))]);
        let report = observe(
            &source,
            &clock(),
            Some(731),
            WatchConfig {
                stop_at_ready: false,
                ..cfg()
            },
            continuation(Some("abc"), Some(&historical.url)),
            &mut |_| {},
        )
        .unwrap();
        assert_eq!(report.outcome, Outcome::Ejected);
        assert_eq!(report.pointer.as_deref(), Some(fresh.url.as_str()));
        assert_eq!(
            historical.conclusion, "failure",
            "acknowledgment never rewrites verdict"
        );

        let source = FakeSource::new(
            vec![Ok(open(green())), Ok(open(green())), Ok(merged_snapshot())],
            queue_rules(),
        )
        .with_ejection(Some(historical.clone()));
        let report = observe(
            &source,
            &clock(),
            Some(731),
            WatchConfig {
                stop_at_ready: false,
                ..cfg()
            },
            continuation(Some("abc"), Some(&historical.url)),
            &mut |_| {},
        )
        .unwrap();
        assert_eq!(report.outcome, Outcome::Merged);
    }

    #[test]
    fn continuation_head_guard_outranks_even_merged_replacement_and_readiness() {
        use crate::pr::test_support::merged_snapshot;
        for mut replacement in [open(green()), merged_snapshot()] {
            replacement.head_sha = "replacement".into();
            let source = FakeSource::new(vec![Ok(replacement), Ok(open(green()))], queue_rules());
            let report = observe(
                &source,
                &clock(),
                Some(731),
                cfg(),
                continuation(Some("abc"), None),
                &mut |_| {},
            )
            .unwrap();
            assert_eq!(report.outcome, Outcome::Stale);
            assert_eq!(report.head_sha, "abc");
        }
    }

    #[test]
    fn acknowledged_preflight_still_requires_readiness_and_complete_required_checks() {
        use crate::pr::snapshot::{CheckState, MergeStateStatus, Mergeable};
        use crate::pr::test_support::ejection;
        let historical = ejection("2026-07-30T14:30:00Z");
        let mut blocked = open(green());
        blocked.merge_state_status = MergeStateStatus::Blocked;
        let mut unknown = open(green());
        unknown.mergeable = Mergeable::Unknown;
        let mut failed = open(green());
        failed.checks[0].state = CheckState::Failure;
        for (snapshot, expected) in [
            (open(green()), Outcome::ReadyToLand),
            (blocked, Outcome::Blocked),
            (unknown, Outcome::Pending),
            (failed, Outcome::ChecksFailed),
        ] {
            let source = FakeSource::new(vec![Ok(snapshot)], queue_rules())
                .with_ejection(Some(historical.clone()));
            let report = observe(
                &source,
                &clock(),
                Some(731),
                WatchConfig {
                    once: true,
                    ..cfg()
                },
                continuation(Some("abc"), Some(&historical.url)),
                &mut |_| {},
            )
            .unwrap();
            assert_eq!(report.outcome, expected);
        }
    }

    #[test]
    fn one_shot_observation_returns_the_actionable_readiness_without_an_armer() {
        let source = FakeSource::new(vec![Ok(open(green()))], queue_rules());
        let clock = clock();
        let mut events = Vec::new();
        let report = observe(
            &source,
            &clock,
            Some(731),
            WatchConfig {
                once: true,
                ..cfg()
            },
            continuation(None, None),
            &mut |event| events.push(event.clone()),
        )
        .unwrap();

        assert_eq!(report.outcome, Outcome::ReadyToLand);
        assert_eq!(report.pr, 731);
        assert!(!events.is_empty());
        assert_eq!(*clock.now.borrow(), 0, "one-shot observation never waits");
    }
}
