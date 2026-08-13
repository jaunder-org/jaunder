use serde::{Serialize, Serializer};

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
