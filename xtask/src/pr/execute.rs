use anyhow::{Result, anyhow};

use super::decide::Progress;
use super::gh::ApiError;
use super::invocation::{GitFacts, Invocation};
use super::snapshot::{PrSource, SharedFailureEvidenceSource};
use super::{Event, Outcome, PrNumber, PrReport, land, snapshot, watch};
use crate::result::{CommandResult, StepResult};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PrOperation {
    Watch,
    Land,
}

impl PrOperation {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Watch => "pr-watch",
            Self::Land => "pr-land",
        }
    }

    pub fn is_landing(self) -> bool {
        matches!(self, Self::Land)
    }

    fn succeeds_with(self, outcome: Outcome) -> bool {
        matches!(
            (self, outcome),
            (Self::Watch, Outcome::Merged | Outcome::ReadyToLand) | (Self::Land, Outcome::Merged)
        )
    }
}

/// Drive one `pr watch` / `pr land` invocation against the real GitHub.
///
/// A four-line shim over [`execute_with`]: everything with a decision in it lives
/// there, where a fake source, armer, and clock can reach it.
pub fn execute(number: Option<u64>, cfg: watch::WatchConfig, landing: bool) -> Result<PrReport> {
    // The event log streams to stderr as it happens, so `--json` keeps stdout to a
    // single parseable document and a human still sees progress live. The same events
    // are serialized into the report, so nothing here is stderr-only.
    let mut sink = |e: &Event| eprintln!("  {} [{}] {}", e.at, e.kind.as_str(), e.detail);
    dispatch_with_git_facts(
        || GitFacts::read(std::path::Path::new("."), landing),
        |git| {
            let source = snapshot::GhSource::default();
            execute_with(
                &source,
                &land::GhArmer,
                &watch::SystemClock,
                Invocation {
                    git,
                    number,
                    cfg,
                    landing,
                },
                &mut sink,
            )
        },
    )
}

fn dispatch_with_git_facts<T>(
    read: impl FnOnce() -> Result<GitFacts>,
    dispatch: impl FnOnce(&GitFacts) -> Result<T>,
) -> Result<T> {
    let git = read()?;
    dispatch(&git)
}

/// Establish the subject, guard it, and hand off to `watch` or `land`.
///
/// Returns `Err` only when the *subject* could not be established, or when landing is
/// refused — there is nothing to report on, so those become exit 2. Everything else,
/// including the tooling failing outright, comes back as a `PrReport`.
pub fn execute_with<
    S: PrSource + SharedFailureEvidenceSource,
    A: land::PrArmer,
    C: watch::Clock,
>(
    source: &S,
    armer: &A,
    clock: &C,
    inv: Invocation<'_>,
    sink: &mut dyn FnMut(&Event),
) -> Result<PrReport> {
    let Invocation {
        git: git_facts,
        number,
        cfg,
        landing,
    } = inv;
    let subject = match source.resolve(number.map(PrNumber)) {
        Ok(s) => s,
        Err(e) => {
            return match snapshot::resolution_failure(&e) {
                // Name what was actually searched for. `resolve` collapses four
                // distinct causes into `NotFound` — no remote, an unparseable remote,
                // detached HEAD, no open PR for the branch — and "no open PR found" is
                // unhelpful for the first three.
                snapshot::ResolutionFailure::Bail(msg) => Err(anyhow!(
                    "{msg}{}",
                    match &git_facts.branch {
                        Some(branch) => format!(" (searched for branch `{branch}`)"),
                        None => " (no branch checked out)".to_string(),
                    }
                )),
                snapshot::ResolutionFailure::Report(outcome) => Ok(PrReport {
                    outcome,
                    pr: number.unwrap_or_default(),
                    head_sha: String::new(),
                    phase: None,
                    detail: Some(e.detail()),
                    pointer: None,
                    events: Vec::new(),
                    shared_failure: None,
                    subject_failure: None,
                }),
            };
        }
    };

    // Establish the subject for real. `resolve` can hand back a well-formed
    // `Subject` for a PR that does not exist (an explicit number is taken at its
    // word), and GitHub reports that as a typed GraphQL `NOT_FOUND` on the first
    // read. Catching it *here* keeps it a failure to establish the subject — exit 2,
    // nothing to report on — rather than letting it masquerade as the tooling being
    // broken. Any other error falls through: `watch`/`land` retry and report it.
    // A transient blip on the very first read should not decide anything, so give it
    // a few tries before drawing a conclusion from it.
    //
    // Rate limits are retried too, not just `is_transient` errors: `watch`'s loop
    // rides them out, so `land` bailing on the same condition would make the two
    // commands disagree about what a 403 means.
    let mut established = source.snapshot(&subject);
    for _ in 1..3 {
        let wait = match &established {
            Err(ApiError::RateLimited { reset_unix }) => reset_unix
                .map(|r| r.saturating_sub(clock.now_unix()).min(60))
                .unwrap_or(5),
            Err(e) if e.is_transient() => 2,
            _ => break,
        };
        clock.sleep_secs(wait.max(1));
        established = source.snapshot(&subject);
    }
    if let Err(e) = &established
        && matches!(
            snapshot::resolution_failure(e),
            snapshot::ResolutionFailure::Bail(_)
        )
    {
        return Err(anyhow!(
            "no such pull request: #{} in {}/{}",
            subject.number,
            subject.owner,
            subject.repo
        ));
    }

    if landing {
        // Refuse to land something other than what the caller is looking at, reusing
        // the snapshot that established the subject. The guard must **fail closed**:
        // if the PR head cannot be read, the check is unevaluable, and proceeding
        // would arm a merge on exactly the condition the guard exists to catch.
        let snap = match &established {
            Ok(snap) => snap,
            Err(e) => {
                return Ok(PrReport {
                    outcome: Outcome::WatcherError,
                    pr: subject.number.0,
                    head_sha: String::new(),
                    phase: None,
                    detail: Some(format!(
                        "could not read the PR head, so the divergence guard could not \
                         run and nothing was armed: {}",
                        e.detail()
                    )),
                    pointer: None,
                    events: Vec::new(),
                    shared_failure: None,
                    subject_failure: None,
                });
            }
        };
        if let land::GuardVerdict::Diverged { local, remote } = land::divergence_guard(
            git_facts.branch.as_deref(),
            git_facts.head_sha.as_deref(),
            &snap.head_ref,
            &snap.head_sha,
        ) {
            return Err(anyhow!(land::divergence_message(&local, &remote)));
        }
        return Ok(land::land(source, armer, clock, &subject, cfg, sink));
    }
    let progress = established
        .as_ref()
        .map_or_else(|_| Progress::default(), Progress::from_snapshot);
    let mut report = watch::watch_with_progress(source, clock, &subject, cfg, progress, sink);
    if report.outcome == Outcome::ChecksFailed && report.subject_failure.is_some() {
        enrich_shared_failure(
            source,
            &subject,
            &mut report,
            super::gh::Deadline::ten_seconds(),
        );
    }
    Ok(report)
}
fn enrich_shared_failure<S: SharedFailureEvidenceSource>(
    source: &S,
    subject: &super::Subject,
    report: &mut PrReport,
    deadline: super::gh::Deadline,
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
    let annotation = super::shared_failure::shared_failure_with_check(
        &evidence.subject_log,
        evidence.selected_runs,
        evidence.jobs,
        &mut check,
    )
    .map_err(|_| super::gh::ApiError::Transport("shared-failure enrichment timed out".into()));
    if let Ok(annotation) = annotation {
        report.shared_failure = annotation;
    }
}

/// Wrap a report in the command envelope.
///
pub fn into_result(
    operation: PrOperation,
    report: PrReport,
    duration: std::time::Duration,
) -> CommandResult {
    let command = operation.as_str();
    let mut result = CommandResult::new(command);
    let step = if operation.succeeds_with(report.outcome) {
        StepResult::ok(command)
    } else {
        StepResult::fail(command)
    };
    result.push(step.detail(report.outcome.as_str()).with_duration(duration));
    result.pr = Some(report);
    result
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::pr::gh::ApiError as E;
    use crate::pr::land::PrArmer;
    use crate::pr::shared_failure::{
        CandidateJob, CandidateRun, SelectedRun, SharedFailureEvidence, SharedFailureSource,
        SubjectFailure,
    };
    use crate::pr::test_support::*;
    use crate::pr::{EventKind, Subject};

    /// An armer that records calls and never fails, so "did anything get armed?" is
    /// assertable at this layer too.
    struct SpyArmer {
        calls: std::cell::Cell<u32>,
    }

    impl SpyArmer {
        fn new() -> Self {
            Self {
                calls: std::cell::Cell::new(0),
            }
        }
    }

    impl PrArmer for SpyArmer {
        fn arm_auto_merge(&self, _: &Subject) -> std::result::Result<(), ApiError> {
            self.calls.set(self.calls.get() + 1);
            Ok(())
        }
    }

    #[test]
    fn pr_land_git_fact_failure_returns_before_dispatch_or_arming() {
        let dispatched = std::cell::Cell::new(false);
        let error = dispatch_with_git_facts::<()>(
            || {
                Err(anyhow::Error::new(std::io::Error::new(
                    std::io::ErrorKind::PermissionDenied,
                    "injected local Git failure",
                )))
            },
            |_| {
                dispatched.set(true);
                Ok(())
            },
        )
        .unwrap_err();
        assert!(!dispatched.get());
        assert_eq!(
            error
                .downcast_ref::<std::io::Error>()
                .map(std::io::Error::kind),
            Some(std::io::ErrorKind::PermissionDenied)
        );
    }

    fn invocation(git: &GitFacts, landing: bool) -> Invocation<'_> {
        Invocation {
            git,
            number: Some(731),
            cfg: cfg(),
            landing,
        }
    }

    #[test]
    fn a_nonexistent_pr_bails_to_exit_two_with_no_report() {
        // The subject could not be established, so there is nothing to report *on* —
        // and an `Err` here is what `main.rs` turns into exit 2.
        let src = FakeSource::new(vec![Err(E::NotFound)], queue_rules());
        let git = GitFacts::default();
        let err = execute_with(
            &src,
            &SpyArmer::new(),
            &clock(),
            invocation(&git, false),
            &mut |_| {},
        )
        .expect_err("a missing PR must not produce a report");
        assert!(err.to_string().contains("no such pull request"), "{err}");
    }

    #[test]
    fn broken_tooling_during_resolution_is_a_report_not_an_exit_two() {
        // "`gh` is broken" is more actionable than "no such PR", and it is what
        // actually happened — so it must survive as a readable outcome.
        let src = FakeSource::new(vec![], queue_rules()).with_resolve_error(E::GhMissing);
        let git = GitFacts::default();
        let report = execute_with(
            &src,
            &SpyArmer::new(),
            &clock(),
            invocation(&git, false),
            &mut |_| {},
        )
        .expect("a tooling failure is a report, never an Err");
        assert_eq!(report.outcome, Outcome::WatcherError);
    }

    #[test]
    fn a_failed_resolution_names_the_branch_it_searched_for() {
        let src = FakeSource::new(vec![], queue_rules()).with_resolve_error(E::NotFound);
        let git = GitFacts {
            branch: Some("feature".into()),
            head_sha: Some("abc".into()),
        };
        let err = execute_with(
            &src,
            &SpyArmer::new(),
            &clock(),
            Invocation {
                git: &git,
                number: None,
                cfg: cfg(),
                landing: false,
            },
            &mut |_| {},
        )
        .expect_err("no PR for the branch must bail");
        assert!(err.to_string().contains("feature"), "{err}");
    }

    #[test]
    fn landing_from_the_prs_branch_with_local_commits_refuses() {
        // Exit 2, no report, and — critically — nothing armed.
        let src = FakeSource::new(vec![Ok(open_pending())], queue_rules());
        let armer = SpyArmer::new();
        let git = GitFacts {
            // `open_pending()` has head_ref "feature" and head_sha "abc".
            branch: Some("feature".into()),
            head_sha: Some("local-only".into()),
        };
        let err = execute_with(&src, &armer, &clock(), invocation(&git, true), &mut |_| {})
            .expect_err("divergence must refuse, not report");
        assert!(err.to_string().contains("local-only"), "{err}");
        assert!(err.to_string().contains("abc"), "{err}");
        assert_eq!(armer.calls.get(), 0, "nothing may be armed after a refusal");
    }

    #[test]
    fn landing_with_an_unreadable_head_refuses_to_arm() {
        // The guard is unevaluable, so it must fail closed. Five errors exhausts the
        // pre-flight retries.
        let src = FakeSource::new(
            (0..5).map(|_| Err(E::Transport("down".into()))).collect(),
            queue_rules(),
        );
        let armer = SpyArmer::new();
        let git = GitFacts::default();
        let report = execute_with(&src, &armer, &clock(), invocation(&git, true), &mut |_| {})
            .expect("a tooling failure is a report");
        assert_eq!(report.outcome, Outcome::WatcherError);
        assert_eq!(armer.calls.get(), 0, "must not arm what it cannot read");
        assert!(report.detail.unwrap().contains("nothing was armed"));
    }

    #[test]
    fn a_transient_first_read_is_retried_rather_than_believed() {
        let src = FakeSource::new(
            vec![Err(E::Transport("blip".into())), Ok(merged_snapshot())],
            queue_rules(),
        );
        let git = GitFacts::default();
        let report = execute_with(
            &src,
            &SpyArmer::new(),
            &clock(),
            invocation(&git, false),
            &mut |_| {},
        )
        .expect("a blip must not bail");
        assert_eq!(report.outcome, Outcome::Merged);
    }

    #[test]
    fn establishment_snapshot_preserves_same_head_queue_history() {
        let src = FakeSource::new(vec![Ok(queued_at(2)), Ok(open(green()))], queue_rules());
        let git = GitFacts::default();
        let report = execute_with(
            &src,
            &SpyArmer::new(),
            &clock(),
            invocation(&git, false),
            &mut |_| {},
        )
        .unwrap();
        assert_eq!(report.outcome, Outcome::Dequeued);
    }

    #[test]
    fn watching_never_arms_anything() {
        // The structural guarantee behind the observe/act split: no `watch` path can
        // reach the armer, whatever it is handed.
        let src = FakeSource::new(vec![Ok(merged_snapshot())], queue_rules());
        let armer = SpyArmer::new();
        let git = GitFacts::default();
        execute_with(&src, &armer, &clock(), invocation(&git, false), &mut |_| {}).unwrap();
        assert_eq!(armer.calls.get(), 0);
    }

    fn failed_actions_snapshot() -> super::snapshot::PrSnapshot {
        open(vec![
            actions_check(
                "Validate (no e2e)",
                9,
                super::snapshot::CheckState::Failure,
                "2026-09-15T12:00:00Z",
            ),
            check("e2e gate", super::snapshot::CheckState::Pending, ""),
        ])
    }

    fn matching_evidence() -> SharedFailureEvidence {
        SharedFailureEvidence {
            subject_log: "error: shared".into(),
            selected_runs: vec![SelectedRun {
                run: CandidateRun {
                    id: 88,
                    url: "https://github.com/jaunder-org/jaunder/actions/runs/88".into(),
                    head_sha: "other-head".into(),
                    created_at: "2026-09-15T11:00:00Z".into(),
                    conclusion: Some("failure".into()),
                },
                source: SharedFailureSource::PullRequest {
                    number: 732,
                    url: "https://github.com/jaunder-org/jaunder/pull/732".into(),
                },
            }],
            jobs: vec![(
                88,
                vec![CandidateJob {
                    id: 90,
                    name: "Validate (no e2e)".into(),
                    url: "https://github.com/jaunder-org/jaunder/actions/jobs/90".into(),
                    log: "error: shared".into(),
                }],
            )],
        }
    }

    #[test]
    fn typed_actions_failure_enriches_the_report_without_changing_its_execution_result() {
        let annotated_source = FakeSource::new(vec![Ok(failed_actions_snapshot())], queue_rules())
            .with_shared_failure_evidence(matching_evidence());
        let bare_source = FakeSource::new(vec![Ok(failed_actions_snapshot())], queue_rules())
            .with_shared_failure_evidence_script(vec![]);
        let git = GitFacts::default();
        let mut annotated_events = Vec::new();
        let annotated = execute_with(
            &annotated_source,
            &SpyArmer::new(),
            &clock(),
            invocation(&git, false),
            &mut |event| annotated_events.push(event.clone()),
        )
        .unwrap();
        let mut bare_events = Vec::new();
        let bare = execute_with(
            &bare_source,
            &SpyArmer::new(),
            &clock(),
            invocation(&git, false),
            &mut |event| bare_events.push(event.clone()),
        )
        .unwrap();

        assert_eq!(annotated_source.shared_failure_evidence_requests(), 1);
        assert_eq!(
            annotated_source.shared_failure_subject_failures(),
            vec![SubjectFailure {
                workflow_run_id: 1,
                check_run_id: 9,
                name: "Validate (no e2e)".into(),
            }]
        );
        assert_eq!(bare_source.shared_failure_evidence_requests(), 1);
        assert_eq!(annotated.outcome, Outcome::ChecksFailed);
        assert_eq!(annotated_events, bare_events, "enrichment emits no events");
        let mut expected = bare.clone();
        expected.shared_failure = annotated.shared_failure.clone();
        assert_eq!(annotated, expected, "only the annotation may change");
        assert_eq!(
            serde_json::to_value(&annotated).unwrap()["shared_failure"],
            serde_json::json!({
                "version": 1,
                "algorithm": "github-actions-error-block-v1",
                "digest_sha256": "d44f789533a2260f997226e225848d0c7904b037e50e523f469c9b1dc4ce5547",
                "excerpt": "error: shared",
                "matches": [{
                    "source": {
                        "kind": "pull-request",
                        "number": 732,
                        "url": "https://github.com/jaunder-org/jaunder/pull/732",
                    },
                    "head_sha": "other-head",
                    "run": {
                        "id": 88,
                        "url": "https://github.com/jaunder-org/jaunder/actions/runs/88",
                    },
                    "job": {
                        "id": 90,
                        "name": "Validate (no e2e)",
                        "url": "https://github.com/jaunder-org/jaunder/actions/jobs/90",
                    },
                }],
            })
        );
        let annotated_result = into_result(
            PrOperation::Watch,
            annotated,
            std::time::Duration::from_millis(42),
        );
        let bare_result = into_result(
            PrOperation::Watch,
            bare,
            std::time::Duration::from_millis(42),
        );
        assert_eq!(
            serde_json::to_value(&annotated_result).unwrap()["steps"],
            serde_json::to_value(&bare_result).unwrap()["steps"]
        );
        assert_eq!(annotated_result.ok, bare_result.ok);
        assert_eq!(annotated_result.exit_code(), bare_result.exit_code());
    }

    #[test]
    fn evidence_failures_and_non_matches_leave_the_terminal_report_unannotated() {
        let no_anchor = SharedFailureEvidence {
            subject_log: "ordinary output".into(),
            ..matching_evidence()
        };
        let no_match = SharedFailureEvidence {
            jobs: vec![(
                88,
                vec![CandidateJob {
                    log: "error: different".into(),
                    ..matching_evidence().jobs[0].1[0].clone()
                }],
            )],
            ..matching_evidence()
        };
        let git = GitFacts::default();
        for (case, scripted) in [
            ("no anchor", Ok(no_anchor)),
            ("unique failure", Ok(no_match)),
            ("query", Err(E::GraphQlErrors("query failed".into()))),
            ("log", Err(E::Malformed("non-UTF-8 log".into()))),
            ("parse", Err(E::Malformed("response was malformed".into()))),
            ("timeout", Err(E::Transport("deadline elapsed".into()))),
        ] {
            let source = FakeSource::new(vec![Ok(failed_actions_snapshot())], queue_rules())
                .with_shared_failure_evidence_script(vec![scripted]);
            let report = execute_with(
                &source,
                &SpyArmer::new(),
                &clock(),
                invocation(&git, false),
                &mut |_| {},
            )
            .unwrap();
            assert_eq!(report.outcome, Outcome::ChecksFailed, "{case}");
            assert!(report.shared_failure.is_none(), "{case}");
            assert_eq!(report.events.len(), 1, "{case}");
            assert_eq!(report.events[0].kind, EventKind::Terminal, "{case}");
            assert_eq!(source.shared_failure_evidence_requests(), 1, "{case}");
        }
    }

    #[test]
    fn expiry_after_evidence_discards_the_annotation_during_policy_work() {
        let git = GitFacts::default();
        let bare_source = FakeSource::new(vec![Ok(failed_actions_snapshot())], queue_rules())
            .with_shared_failure_evidence_script(vec![]);
        let mut report = execute_with(
            &bare_source,
            &SpyArmer::new(),
            &clock(),
            invocation(&git, false),
            &mut |_| {},
        )
        .unwrap();
        let evidence_source = FakeSource::new(vec![], queue_rules())
            .with_shared_failure_evidence(matching_evidence())
            .with_shared_failure_evidence_delay(std::time::Duration::from_millis(20));
        enrich_shared_failure(
            &evidence_source,
            &crate::pr::test_support::subject(),
            &mut report,
            crate::pr::gh::Deadline::after(std::time::Duration::from_millis(1)),
        );
        assert!(report.shared_failure.is_none());
        assert_eq!(evidence_source.shared_failure_evidence_requests(), 1);
    }

    #[test]
    fn land_status_context_failures_and_other_watch_outcomes_never_request_evidence() {
        let land_source = FakeSource::new(vec![Ok(failed_actions_snapshot())], queue_rules())
            .with_shared_failure_evidence(matching_evidence());
        let land = execute_with(
            &land_source,
            &SpyArmer::new(),
            &clock(),
            invocation(&GitFacts::default(), true),
            &mut |_| {},
        )
        .unwrap();
        assert_eq!(land.outcome, Outcome::ChecksFailed);
        assert_eq!(land_source.shared_failure_evidence_requests(), 0);

        let status_source = FakeSource::new(
            vec![Ok(open(vec![
                check(
                    "Validate (no e2e)",
                    super::snapshot::CheckState::Failure,
                    "2026-09-15T12:00:00Z",
                ),
                check("e2e gate", super::snapshot::CheckState::Pending, ""),
            ]))],
            queue_rules(),
        )
        .with_shared_failure_evidence(matching_evidence());
        let status = execute_with(
            &status_source,
            &SpyArmer::new(),
            &clock(),
            invocation(&GitFacts::default(), false),
            &mut |_| {},
        )
        .unwrap();
        assert_eq!(status.outcome, Outcome::ChecksFailed);
        assert_eq!(status_source.shared_failure_evidence_requests(), 0);

        let merged_source = FakeSource::new(vec![Ok(merged_snapshot())], queue_rules())
            .with_shared_failure_evidence(matching_evidence());
        let merged = execute_with(
            &merged_source,
            &SpyArmer::new(),
            &clock(),
            invocation(&GitFacts::default(), false),
            &mut |_| {},
        )
        .unwrap();
        assert_eq!(merged.outcome, Outcome::Merged);

        let pending_source = FakeSource::new(vec![Ok(open_pending())], queue_rules())
            .with_shared_failure_evidence(matching_evidence());
        let mut pending_cfg = cfg();
        let pending_git = GitFacts::default();
        pending_cfg.once = true;
        let pending = execute_with(
            &pending_source,
            &SpyArmer::new(),
            &clock(),
            Invocation {
                git: &pending_git,
                number: Some(731),
                cfg: pending_cfg,
                landing: false,
            },
            &mut |_| {},
        )
        .unwrap();
        assert_eq!(pending.outcome, Outcome::Pending);
        assert_eq!(pending_source.shared_failure_evidence_requests(), 0);
        assert_eq!(merged_source.shared_failure_evidence_requests(), 0);
    }

    fn report(outcome: Outcome) -> PrReport {
        PrReport {
            outcome,
            pr: 731,
            head_sha: "abc123".into(),
            phase: None,
            detail: None,
            pointer: None,
            shared_failure: None,
            subject_failure: None,
            events: vec![Event {
                at: "2026-07-30T14:02:11Z".into(),
                kind: EventKind::Phase,
                detail: "awaiting-checks".into(),
            }],
        }
    }

    fn result(operation: PrOperation, outcome: Outcome) -> CommandResult {
        into_result(
            operation,
            report(outcome),
            std::time::Duration::from_millis(42),
        )
    }

    #[test]
    fn merged_result_is_ok_and_exits_zero() {
        let r = result(PrOperation::Watch, Outcome::Merged);
        assert!(r.ok);
        assert_eq!(r.exit_code(), 0);
    }

    #[test]
    fn ready_to_land_succeeds_only_for_watch() {
        let watched = result(PrOperation::Watch, Outcome::ReadyToLand);
        assert!(watched.ok);
        assert_eq!(watched.exit_code(), 0);
        assert_eq!(watched.steps.len(), 1);
        assert!(watched.steps[0].ok);

        let landed = result(PrOperation::Land, Outcome::ReadyToLand);
        assert!(!landed.ok);
        assert_eq!(landed.exit_code(), 1);
    }

    #[test]
    fn adverse_results_are_not_ok_and_exit_one() {
        for outcome in [
            Outcome::ChecksFailed,
            Outcome::Ejected,
            Outcome::Dequeued,
            Outcome::Blocked,
            Outcome::Conflicted,
            Outcome::ClosedUnmerged,
            Outcome::Stale,
            Outcome::TimedOut,
            Outcome::WatcherError,
            Outcome::Pending,
        ] {
            for operation in [PrOperation::Watch, PrOperation::Land] {
                let r = result(operation, outcome);
                assert!(!r.ok, "{operation:?} {outcome:?} must not be ok");
                assert_eq!(r.exit_code(), 1, "{operation:?} {outcome:?} must exit 1");
            }
        }
    }

    #[test]
    fn exactly_one_step_is_pushed() {
        // Load-bearing: `push()` recomputes `ok` from the step vector, so a second
        // step would decouple `ok` from the outcome.
        let r = result(PrOperation::Watch, Outcome::ChecksFailed);
        assert_eq!(r.steps.len(), 1);
        assert_eq!(r.steps[0].name, "pr-watch");
        assert_eq!(r.steps[0].duration_ms, 42);
    }

    #[test]
    fn outcomes_serialize_kebab_case() {
        assert_eq!(
            serde_json::to_value(Outcome::WatcherError).unwrap(),
            "watcher-error"
        );
        assert_eq!(
            serde_json::to_value(Outcome::ClosedUnmerged).unwrap(),
            "closed-unmerged"
        );
        for (outcome, spelling) in [
            (Outcome::ReadyToLand, "ready-to-land"),
            (Outcome::Dequeued, "dequeued"),
            (Outcome::Blocked, "blocked"),
        ] {
            assert_eq!(serde_json::to_value(outcome).unwrap(), spelling);
        }
    }

    #[test]
    fn report_rides_the_envelope_json() {
        let r = result(PrOperation::Watch, Outcome::Ejected);
        let v = serde_json::to_value(&r).unwrap();
        assert_eq!(v["pr"]["outcome"], "ejected");
        assert_eq!(v["pr"]["pr"], 731);
        assert_eq!(v["pr"]["events"][0]["kind"], "phase");
        assert_eq!(v["ok"], false);
    }

    #[test]
    fn absent_report_is_omitted_from_json() {
        let r = crate::result::CommandResult::new("check");
        let v = serde_json::to_value(&r).unwrap();
        assert!(
            v.get("pr").is_none(),
            "no `pr` key when the command has no report"
        );
    }
}
