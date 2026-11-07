//! Read-only command-level setup for the observer.
//!
//! The existing xtask entrypoint temporarily calls the same poller after it has
//! established the subject; the dedicated binary uses [`observe`] for that setup
//! plus the shared poller. No armer or cleanup capability is accepted here.

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
    sink: &mut dyn FnMut(&Event),
) -> Result<PrReport>
where
    S: PrSource + SharedFailureEvidenceSource,
    C: Clock,
{
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

    Ok(observe_established(
        source,
        clock,
        &subject,
        established,
        config,
        sink,
    ))
}

/// Read the initial subject with the same bounded retry policy for both frontdoors.
/// Keep this setup here so the compatibility caller cannot grow a second poller.
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

/// Enter the poller from an already-resolved subject and its initial snapshot.
/// Shared with the compatibility xtask command so there is exactly one watch loop.
pub fn observe_established<S, C>(
    source: &S,
    clock: &C,
    subject: &Subject,
    established: Result<super::snapshot::PrSnapshot, ApiError>,
    config: WatchConfig,
    sink: &mut dyn FnMut(&Event),
) -> PrReport
where
    S: PrSource + SharedFailureEvidenceSource,
    C: Clock,
{
    let progress = established
        .as_ref()
        .map_or_else(|_| Progress::default(), Progress::from_snapshot);
    let mut report = watch::watch_with_progress(source, clock, subject, config, progress, sink);
    if report.outcome == Outcome::ChecksFailed && report.subject_failure.is_some() {
        enrich_shared_failure(source, subject, &mut report, Deadline::ten_seconds());
    }
    report
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
            &mut |event| events.push(event.clone()),
        )
        .unwrap();

        assert_eq!(report.outcome, Outcome::ReadyToLand);
        assert_eq!(report.pr, 731);
        assert!(!events.is_empty());
        assert_eq!(*clock.now.borrow(), 0, "one-shot observation never waits");
    }
}
