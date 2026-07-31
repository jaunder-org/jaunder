//! PR observation: `cargo xtask pr watch` / `pr land` (#729).
//!
//! Layered boundary → pure → loop (ADR draft `xtask-github-pr-observation`): only
//! `gh` runs a subprocess, `snapshot` turns its JSON into typed values, `decide` is a
//! pure state machine over those values, and `watch`/`land` drive the loop. Above
//! `snapshot` nothing sees JSON, a string status, or an exit code.

pub mod decide;
pub mod gh;
pub mod land;
pub mod snapshot;
#[cfg(test)]
pub(crate) mod test_support;
pub mod watch;

use anyhow::{anyhow, Result};
use serde::{Serialize, Serializer};

use crate::git;
use crate::result::{CommandResult, StepResult};
use gh::ApiError;
use snapshot::PrSource;

/// A pull request number. A newtype because it is threaded through every layer and
/// is transposable with the other bare integers around it (queue position, run id).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PrNumber(pub u64);

impl std::fmt::Display for PrNumber {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}

/// What is being watched: the repo (derived from the git remote, never hardcoded)
/// and the PR within it. Established once, before any watching, so that "which PR?"
/// can never fail halfway through a watch.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Subject {
    pub owner: String,
    pub repo: String,
    pub number: PrNumber,
}

/// The terminal verdicts, plus `Pending` which only `--once` can produce.
///
/// The whole point of the command is that these never collapse into each other:
/// `TimedOut` says GitHub never finished, `WatcherError` says *we* could not tell.
/// An agent branches differently on each, so conflating them would recreate the
/// "three meanings, one signal" defect this replaces.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Outcome {
    Merged,
    ChecksFailed,
    Ejected,
    Conflicted,
    ClosedUnmerged,
    Stale,
    TimedOut,
    WatcherError,
    Pending,
}

impl Outcome {
    pub fn is_merged(self) -> bool {
        matches!(self, Outcome::Merged)
    }

    /// The wire spelling. `Serialize` delegates here so the JSON an agent branches on
    /// and the step detail a human reads can never drift apart.
    pub fn as_str(self) -> &'static str {
        match self {
            Outcome::Merged => "merged",
            Outcome::ChecksFailed => "checks-failed",
            Outcome::Ejected => "ejected",
            Outcome::Conflicted => "conflicted",
            Outcome::ClosedUnmerged => "closed-unmerged",
            Outcome::Stale => "stale",
            Outcome::TimedOut => "timed-out",
            Outcome::WatcherError => "watcher-error",
            Outcome::Pending => "pending",
        }
    }
}

impl Serialize for Outcome {
    fn serialize<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        s.serialize_str(self.as_str())
    }
}

impl std::fmt::Display for Outcome {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum EventKind {
    Phase,
    Check,
    Queue,
    Heartbeat,
    PollError,
    Warning,
    Terminal,
}

impl EventKind {
    pub fn as_str(self) -> &'static str {
        match self {
            EventKind::Phase => "phase",
            EventKind::Check => "check",
            EventKind::Queue => "queue",
            EventKind::Heartbeat => "heartbeat",
            EventKind::PollError => "poll-error",
            EventKind::Warning => "warning",
            EventKind::Terminal => "terminal",
        }
    }
}

/// One entry in the single event log. The same values are rendered live to stderr
/// and serialized into `PrReport::events`, so a human watching and an agent reading
/// `--json` afterwards see the identical timeline — including the absorbed failures
/// that made the old hand-rolled watchers look like they were making progress.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Event {
    pub at: String,
    pub kind: EventKind,
    pub detail: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct PrReport {
    pub outcome: Outcome,
    pub pr: u64,
    pub head_sha: String,
    /// Only set for `Pending` (`--once`), where there is no terminal state to name.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub phase: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub detail: Option<String>,
    /// The outcome-specific thing to go look at: merge commit, failing job log, or
    /// the merge-group run that ejected the PR.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub pointer: Option<String>,
    pub events: Vec<Event>,
}

/// What the local repository says about where the caller is standing.
///
/// Passed in rather than read inside, so the divergence guard and the exit-2 messages
/// are reachable from a test instead of only from a real checkout.
#[derive(Debug, Clone, Default)]
pub struct GitFacts {
    pub branch: Option<String>,
    pub head_sha: Option<String>,
}

impl GitFacts {
    fn read(dir: &std::path::Path) -> Self {
        Self {
            branch: git::current_branch(dir).ok().flatten(),
            head_sha: git::head_sha(dir).ok().flatten(),
        }
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
    execute_with(
        &snapshot::GhSource,
        &land::GhArmer,
        &watch::SystemClock,
        Invocation {
            git: &GitFacts::read(std::path::Path::new(".")),
            number,
            cfg,
            landing,
        },
        &mut sink,
    )
}

/// One invocation's inputs, as a value rather than a parameter list.
pub struct Invocation<'a> {
    pub git: &'a GitFacts,
    pub number: Option<u64>,
    pub cfg: watch::WatchConfig,
    /// `true` for `pr land` — the only mode that may mutate anything.
    pub landing: bool,
}

/// Establish the subject, guard it, and hand off to `watch` or `land`.
///
/// Returns `Err` only when the *subject* could not be established, or when landing is
/// refused — there is nothing to report on, so those become exit 2. Everything else,
/// including the tooling failing outright, comes back as a `PrReport`.
pub fn execute_with<S: PrSource, A: land::PrArmer, C: watch::Clock>(
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
    if let Err(e) = &established {
        if matches!(
            snapshot::resolution_failure(e),
            snapshot::ResolutionFailure::Bail(_)
        ) {
            return Err(anyhow!(
                "no such pull request: #{} in {}/{}",
                subject.number,
                subject.owner,
                subject.repo
            ));
        }
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
                })
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
    Ok(watch::watch(source, clock, &subject, cfg, sink))
}

/// Wrap a report in the command envelope.
///
/// The single pushed step is not incidental: `CommandResult::push` recomputes `ok`
/// from the step vector, so pushing exactly one step whose `ok` mirrors the outcome
/// is what keeps `ok`, `exit_code()`, and `pr.outcome` from disagreeing — and is why
/// neither `push` nor `exit_code` needed to change. Push a second step here and the
/// sidecar starts reporting `ok: true` for a failed watch.
pub fn into_result(command: &str, report: PrReport) -> CommandResult {
    let mut result = CommandResult::new(command);
    let step = if report.outcome.is_merged() {
        StepResult::ok(command)
    } else {
        StepResult::fail(command)
    };
    result.push(step.detail(report.outcome.as_str()));
    result.pr = Some(report);
    result
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::pr::gh::ApiError as E;
    use crate::pr::land::PrArmer;
    use crate::pr::test_support::*;

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
    fn watching_never_arms_anything() {
        // The structural guarantee behind the observe/act split: no `watch` path can
        // reach the armer, whatever it is handed.
        let src = FakeSource::new(vec![Ok(merged_snapshot())], queue_rules());
        let armer = SpyArmer::new();
        let git = GitFacts::default();
        execute_with(&src, &armer, &clock(), invocation(&git, false), &mut |_| {}).unwrap();
        assert_eq!(armer.calls.get(), 0);
    }

    fn report(outcome: Outcome) -> PrReport {
        PrReport {
            outcome,
            pr: 731,
            head_sha: "abc123".into(),
            phase: None,
            detail: None,
            pointer: None,
            events: vec![Event {
                at: "2026-07-30T14:02:11Z".into(),
                kind: EventKind::Phase,
                detail: "awaiting-checks".into(),
            }],
        }
    }

    #[test]
    fn merged_result_is_ok_and_exits_zero() {
        let r = into_result("pr-watch", report(Outcome::Merged));
        assert!(r.ok);
        assert_eq!(r.exit_code(), 0);
    }

    #[test]
    fn non_merged_result_is_not_ok_and_exits_one() {
        for outcome in [
            Outcome::ChecksFailed,
            Outcome::Ejected,
            Outcome::Conflicted,
            Outcome::ClosedUnmerged,
            Outcome::Stale,
            Outcome::TimedOut,
            Outcome::WatcherError,
            Outcome::Pending,
        ] {
            let r = into_result("pr-watch", report(outcome));
            assert!(!r.ok, "{outcome:?} must not be ok");
            assert_eq!(r.exit_code(), 1, "{outcome:?} must exit 1");
        }
    }

    #[test]
    fn exactly_one_step_is_pushed() {
        // Load-bearing: `push()` recomputes `ok` from the step vector, so a second
        // step would decouple `ok` from the outcome.
        let r = into_result("pr-watch", report(Outcome::ChecksFailed));
        assert_eq!(r.steps.len(), 1);
        assert_eq!(r.steps[0].name, "pr-watch");
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
    }

    #[test]
    fn report_rides_the_envelope_json() {
        let r = into_result("pr-watch", report(Outcome::Ejected));
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
