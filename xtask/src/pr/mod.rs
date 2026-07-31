//! PR observation: `cargo xtask pr watch` / `pr land` (#729).
//!
//! Layered boundary → pure → loop (ADR draft `xtask-github-pr-observation`): only
//! `gh` runs a subprocess, `snapshot` turns its JSON into typed values, `decide` is a
//! pure state machine over those values, and `watch`/`land` drive the loop. Above
//! `snapshot` nothing sees JSON, a string status, or an exit code.

use serde::{Serialize, Serializer};

use crate::result::{CommandResult, StepResult};

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
