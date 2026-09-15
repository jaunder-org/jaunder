//! GitHub's JSON → typed values. The last layer that sees JSON at all.
//!
//! Deliberately logic-free: it reports what GitHub said, including conclusions it
//! has no opinion about. Deciding whether a failed merge-group run is *this* PR's
//! ejection — or a stale one from before the last push — belongs to `decide`, which
//! can be tested without any of this.

use std::time::{Duration, SystemTime, UNIX_EPOCH};

use serde_json::Value;

use super::gh::{self, ApiError};
use super::shared_failure::{
    CandidateJob, CandidateRun, EligibleHead, SharedFailureEvidence, SharedFailureSource,
    SubjectFailure, select_runs,
};
use super::{Outcome, PrNumber, Subject};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PrState {
    Open,
    Merged,
    Closed,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mergeable {
    Mergeable,
    Conflicting,
    Unknown,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MergeStateStatus {
    Behind,
    Blocked,
    Clean,
    Dirty,
    Draft,
    HasHooks,
    Unknown,
    Unstable,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CheckState {
    Pending,
    Success,
    Failure,
}

/// The provider identity of a status-rollup entry.
///
/// Only GitHub Actions check runs can be joined to an Actions workflow graph. A
/// status context deliberately retains no invented run identity: ruleset membership
/// is its only requirement evidence.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CheckProvider {
    GitHubActions {
        check_run_id: u64,
        workflow_run_id: u64,
    },
    StatusContext,
    OtherCheckRun,
}

/// One entry from `statusCheckRollup`, flattened across the `CheckRun` /
/// `StatusContext` union so nothing above this file has to know the union exists.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CheckEntry {
    pub name: String,
    pub provider: CheckProvider,
    pub state: CheckState,
    pub details_url: Option<String>,
    pub started_at: Option<String>,
    pub completed_at: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct QueueState {
    pub in_queue: bool,
    pub position: Option<u64>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PrSnapshot {
    pub state: PrState,
    pub merged_at: Option<String>,
    pub merge_commit: Option<String>,
    pub mergeable: Mergeable,
    pub merge_state_status: MergeStateStatus,
    pub auto_merge_armed: bool,
    pub queue: QueueState,
    pub head_sha: String,
    pub head_ref: String,
    pub base_sha: String,
    /// Git refreshes this on rebase and amend, which is what lets a re-pushed head
    /// reliably post-date a stale merge-group run.
    pub head_committed_at: String,
    pub checks: Vec<CheckEntry>,
}

/// The gate's shape, read per run rather than hardcoded — the required contexts
/// changed three times in a single cycle, and the merge queue can be rolled back.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RequiredChecks {
    pub contexts: Vec<String>,
    pub strict: bool,
    pub queue_present: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RunRef {
    pub url: String,
    pub created_at: String,
    pub conclusion: String,
}

/// Required-context observations for one exact commit.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CommitChecks {
    pub sha: String,
    pub checks: Vec<CheckEntry>,
}

/// One document for the whole state machine (#729 spec F4).
pub const PR_QUERY: &str = r#"query($owner:String!,$name:String!,$number:Int!,$after:String){
  repository(owner:$owner,name:$name){
    pullRequest(number:$number){
      number
      state
      mergedAt
      mergeCommit { oid }
      mergeable
      mergeStateStatus
      isInMergeQueue
      mergeQueueEntry { position }
      autoMergeRequest { enabledAt }
      headRefName
      baseRefOid
      commits(last:1){ nodes { commit { oid committedDate } } }
      statusCheckRollup {
        contexts(first:100, after:$after){
          nodes {
            __typename
            ... on CheckRun { databaseId name conclusion status detailsUrl startedAt completedAt checkSuite { app { slug } workflowRun { databaseId } } }
            ... on StatusContext { context state targetUrl createdAt }
          }
          pageInfo { hasNextPage endCursor }
        }
      }
    }
  }
}"#;

/// Read the status rollup for an arbitrary commit rather than a pull request.
///
/// The promoter needs the same typed check union on both the immutable PR head
/// and GitHub's ephemeral merge-group commit. Keeping the query and parser here
/// preserves `snapshot` as the last layer that sees GitHub JSON.
pub const COMMIT_CHECKS_QUERY: &str = r#"query($owner:String!,$name:String!,$oid:GitObjectID!){
  repository(owner:$owner,name:$name){
    object(oid:$oid){
      ... on Commit {
        oid
        statusCheckRollup {
          contexts(first:100){
            nodes {
              __typename
              ... on CheckRun { databaseId name conclusion status detailsUrl startedAt completedAt checkSuite { app { slug } workflowRun { databaseId } } }
              ... on StatusContext { context state targetUrl createdAt }
            }
          }
        }
      }
    }
  }
}"#;

fn str_at<'a>(v: &'a Value, path: &[&str]) -> Option<&'a str> {
    let mut cur = v;
    for key in path {
        cur = cur.get(key)?;
    }
    cur.as_str()
}

fn owned(v: &Value, path: &[&str]) -> Option<String> {
    str_at(v, path).map(str::to_string)
}

pub fn parse_snapshot(v: &Value) -> Result<PrSnapshot, ApiError> {
    // A null `pullRequest` is how GitHub reports "no such PR" inside a 200 response.
    // Defaulting it to an empty snapshot would read as a healthy PR with no checks,
    // so it fails loudly instead.
    let pr = v
        .get("data")
        .and_then(|d| d.get("repository"))
        .and_then(|r| r.get("pullRequest"))
        .filter(|p| !p.is_null())
        .ok_or_else(|| ApiError::Malformed("no pullRequest node in response".into()))?;

    let state = match str_at(pr, &["state"]).unwrap_or("") {
        "MERGED" => PrState::Merged,
        "CLOSED" => PrState::Closed,
        _ => PrState::Open,
    };
    let mergeable = match str_at(pr, &["mergeable"]).unwrap_or("") {
        "MERGEABLE" => Mergeable::Mergeable,
        "CONFLICTING" => Mergeable::Conflicting,
        _ => Mergeable::Unknown,
    };
    let merge_state_status = match str_at(pr, &["mergeStateStatus"]).unwrap_or("") {
        "BEHIND" => MergeStateStatus::Behind,
        "BLOCKED" => MergeStateStatus::Blocked,
        "CLEAN" => MergeStateStatus::Clean,
        "DIRTY" => MergeStateStatus::Dirty,
        "DRAFT" => MergeStateStatus::Draft,
        "HAS_HOOKS" => MergeStateStatus::HasHooks,
        "UNSTABLE" => MergeStateStatus::Unstable,
        _ => MergeStateStatus::Unknown,
    };

    let head = pr
        .get("commits")
        .and_then(|c| c.get("nodes"))
        .and_then(Value::as_array)
        .and_then(|n| n.first())
        .and_then(|n| n.get("commit"))
        .ok_or_else(|| ApiError::Malformed("no head commit in response".into()))?;

    let checks = pr
        .get("statusCheckRollup")
        .and_then(|r| r.get("contexts"))
        .and_then(|c| c.get("nodes"))
        .and_then(Value::as_array)
        .map(|nodes| parse_checks(nodes))
        .transpose()?
        .unwrap_or_default();

    Ok(PrSnapshot {
        state,
        merged_at: owned(pr, &["mergedAt"]),
        merge_commit: owned(pr, &["mergeCommit", "oid"]),
        mergeable,
        merge_state_status,
        // Armed iff GitHub actually recorded an auto-merge request. `gh pr merge`'s
        // own output is not evidence — it prints the same thing either way.
        auto_merge_armed: str_at(pr, &["autoMergeRequest", "enabledAt"]).is_some(),
        queue: QueueState {
            in_queue: pr
                .get("isInMergeQueue")
                .and_then(Value::as_bool)
                .unwrap_or(false),
            position: pr
                .get("mergeQueueEntry")
                .and_then(|e| e.get("position"))
                .and_then(Value::as_u64),
        },
        // Both are load-bearing and must never default. An empty `head_committed_at`
        // would make *every* failed merge-group run compare newer, reporting a stale
        // run from a previous push as a fresh ejection; an empty `head_sha` would
        // blind the divergence guard. A missing one is a broken response, not a PR
        // with no head.
        head_sha: owned(head, &["oid"])
            .ok_or_else(|| ApiError::Malformed("head commit has no oid".into()))?,
        head_ref: owned(pr, &["headRefName"]).unwrap_or_default(),
        base_sha: owned(pr, &["baseRefOid"])
            .ok_or_else(|| ApiError::Malformed("pull request has no base ref oid".into()))?,
        head_committed_at: owned(head, &["committedDate"])
            .ok_or_else(|| ApiError::Malformed("head commit has no committedDate".into()))?,
        checks,
    })
}

/// Flatten one rollup node. `CheckRun` carries `name`/`conclusion`/`status`;
/// `StatusContext` carries `context`/`state`. Both become a `CheckEntry`.
fn parse_checks(nodes: &[Value]) -> Result<Vec<CheckEntry>, ApiError> {
    Ok(nodes
        .iter()
        .map(parse_check)
        .collect::<Result<Vec<_>, _>>()?
        .into_iter()
        .flatten()
        .collect())
}

fn parse_check(node: &Value) -> Result<Option<CheckEntry>, ApiError> {
    if let Some(name) = str_at(node, &["name"]) {
        let completed = str_at(node, &["status"]) == Some("COMPLETED");
        let state = if !completed {
            CheckState::Pending
        } else {
            match str_at(node, &["conclusion"]).unwrap_or("") {
                // NEUTRAL and SKIPPED do not block a merge, so they count as passing.
                "SUCCESS" | "NEUTRAL" | "SKIPPED" => CheckState::Success,
                _ => CheckState::Failure,
            }
        };
        let provider = match str_at(node, &["checkSuite", "app", "slug"]) {
            Some("github-actions") => CheckProvider::GitHubActions {
                check_run_id: database_id(node, "/databaseId", "check")?,
                workflow_run_id: database_id(
                    node,
                    "/checkSuite/workflowRun/databaseId",
                    "workflow run",
                )?,
            },
            _ => CheckProvider::OtherCheckRun,
        };
        return Ok(Some(CheckEntry {
            name: name.to_string(),
            provider,
            state,
            details_url: owned(node, &["detailsUrl"]),
            started_at: owned(node, &["startedAt"]),
            completed_at: owned(node, &["completedAt"]),
        }));
    }
    let Some(context) = str_at(node, &["context"]) else {
        return Ok(None);
    };
    let state = match str_at(node, &["state"]).unwrap_or("") {
        "SUCCESS" => CheckState::Success,
        "FAILURE" | "ERROR" => CheckState::Failure,
        _ => CheckState::Pending,
    };
    Ok(Some(CheckEntry {
        name: context.to_string(),
        provider: CheckProvider::StatusContext,
        state,
        details_url: owned(node, &["targetUrl"]),
        started_at: owned(node, &["createdAt"]),
        completed_at: match state {
            CheckState::Pending => None,
            _ => owned(node, &["createdAt"]),
        },
    }))
}

/// Parse GitHub GraphQL's numeric `databaseId` without losing correlation evidence.
///
/// Missing, negative, fractional, or out-of-range IDs are observation errors rather
/// than guesses about workflow identity.
fn database_id(value: &Value, pointer: &str, subject: &str) -> Result<u64, ApiError> {
    value
        .pointer(pointer)
        .and_then(Value::as_u64)
        .ok_or_else(|| {
            ApiError::Malformed(format!(
                "GitHub Actions {subject} has no unsigned integer {pointer}"
            ))
        })
}

pub fn parse_commit_checks(v: &Value) -> Result<CommitChecks, ApiError> {
    let commit = v
        .get("data")
        .and_then(|d| d.get("repository"))
        .and_then(|r| r.get("object"))
        .filter(|object| !object.is_null())
        .ok_or_else(|| ApiError::Malformed("no commit object in response".into()))?;
    let sha = owned(commit, &["oid"])
        .ok_or_else(|| ApiError::Malformed("commit object has no oid".into()))?;
    let checks = commit
        .get("statusCheckRollup")
        .and_then(|rollup| rollup.get("contexts"))
        .and_then(|contexts| contexts.get("nodes"))
        .and_then(Value::as_array)
        .map(|nodes| parse_checks(nodes))
        .transpose()?
        .unwrap_or_default();
    Ok(CommitChecks { sha, checks })
}

pub fn parse_required_checks(v: &Value) -> Result<RequiredChecks, ApiError> {
    let rules = v
        .as_array()
        .ok_or_else(|| ApiError::Malformed("branch rules response is not an array".into()))?;
    let status_rule = rules
        .iter()
        .find(|r| str_at(r, &["type"]) == Some("required_status_checks"));
    let contexts = status_rule
        .and_then(|r| r.get("parameters"))
        .and_then(|p| p.get("required_status_checks"))
        .and_then(Value::as_array)
        .map(|cs| {
            cs.iter()
                .filter_map(|c| owned(c, &["context"]))
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();
    Ok(RequiredChecks {
        contexts,
        strict: status_rule
            .and_then(|r| r.get("parameters"))
            .and_then(|p| p.get("strict_required_status_checks_policy"))
            .and_then(Value::as_bool)
            .unwrap_or(false),
        queue_present: rules
            .iter()
            .any(|r| str_at(r, &["type"]) == Some("merge_queue")),
    })
}

/// The most recent merge-group run for this PR, whatever its conclusion.
///
/// The branch is `gh-readonly-queue/main/pr-<N>-<BASE sha>` — the suffix is the base
/// commit, not the PR head, so recency cannot be read off the name and `?branch=`
/// (which needs an exact match) is unusable. Hence: prefix match, newest by
/// `created_at`, and the failure/recency judgment left to `decide`.
pub fn parse_ejection_run(v: &Value, pr: PrNumber) -> Option<RunRef> {
    let prefix = format!("gh-readonly-queue/main/pr-{pr}-");
    v.get("workflow_runs")?
        .as_array()?
        .iter()
        .filter(|r| str_at(r, &["head_branch"]).is_some_and(|b| b.starts_with(&prefix)))
        .max_by_key(|r| owned(r, &["created_at"]).unwrap_or_default())
        .map(|r| RunRef {
            url: owned(r, &["html_url"]).unwrap_or_default(),
            created_at: owned(r, &["created_at"]).unwrap_or_default(),
            conclusion: owned(r, &["conclusion"]).unwrap_or_default(),
        })
}

/// Everything the watcher needs to read. Domain-shaped on purpose: a fake supplies
/// `PrSnapshot`s directly, so the whole loop is testable without `gh` or a network.
pub trait PrSource {
    fn resolve(&self, requested: Option<PrNumber>) -> Result<Subject, ApiError>;
    fn snapshot(&self, subject: &Subject) -> Result<PrSnapshot, ApiError>;
    fn actions_evidence(
        &self,
        subject: &Subject,
        snapshot: &PrSnapshot,
        workflow_run_ids: &std::collections::BTreeSet<u64>,
    ) -> Result<super::evidence::ActionsEvidence, ApiError>;
    fn required_checks(&self, subject: &Subject) -> Result<RequiredChecks, ApiError>;
    fn ejection_run(&self, subject: &Subject) -> Result<Option<RunRef>, ApiError>;
}
/// Best-effort evidence for a terminal Actions failure, deliberately separate from
/// [`PrSource`] so observing it cannot affect the watch state machine.
pub trait SharedFailureEvidenceSource {
    fn shared_failure_evidence(
        &self,
        subject: &Subject,
        failure: &SubjectFailure,
        deadline: &gh::Deadline,
    ) -> Result<SharedFailureEvidence, ApiError>;
}

/// Owner and repo from a git remote URL, in either of the two forms git writes.
/// Deriving this beats hardcoding an org into a tool — invisible until someone forks.
pub fn parse_remote(url: &str) -> Option<(String, String)> {
    let url = url.trim().trim_end_matches('/');
    let path = match url.split_once("://") {
        Some((_, rest)) => rest.split_once('/')?.1,
        None => url.split_once(':')?.1,
    };
    let path = path.strip_suffix(".git").unwrap_or(path);
    let (owner, repo) = path.split_once('/')?;
    (!owner.is_empty() && !repo.is_empty() && !repo.contains('/'))
        .then(|| (owner.to_string(), repo.to_string()))
}

/// What a failure during *subject resolution* means for the caller.
///
/// The line: failures to **establish** the subject exit 2 with no report — there is
/// nothing to report *on*. Failures to **observe** an established subject are
/// `watcher-error` reports. `gh` being broken lands on the report side even during
/// resolution, because "the tooling is broken" is more actionable than "no such PR"
/// and is what actually happened.
#[derive(Debug, PartialEq, Eq)]
pub enum ResolutionFailure {
    Bail(String),
    Report(Outcome),
}

pub fn resolution_failure(err: &ApiError) -> ResolutionFailure {
    match err {
        ApiError::NotFound => ResolutionFailure::Bail(
            "no open PR found — pass a PR number, or run from the PR's branch in a \
             repo with a GitHub remote"
                .into(),
        ),
        _ => ResolutionFailure::Report(Outcome::WatcherError),
    }
}

/// The live source retains immutable workflow evidence for this one watch/land
/// invocation. Dynamic snapshots and Actions job populations are never cached.
pub struct GhSource {
    evidence_cache: std::cell::RefCell<super::evidence::EvidenceCache>,
}

impl Default for GhSource {
    fn default() -> Self {
        Self {
            evidence_cache: std::cell::RefCell::new(super::evidence::EvidenceCache::default()),
        }
    }
}

impl GhSource {
    fn resolve_with(
        requested: Option<PrNumber>,
        dir: &std::path::Path,
        remote_url: impl FnOnce() -> anyhow::Result<Option<String>>,
        current_branch: impl FnOnce() -> anyhow::Result<Option<String>>,
        find_pr: impl FnOnce(&str, &str, &str) -> Result<Value, ApiError>,
    ) -> Result<Subject, ApiError> {
        let url = required_git_fact("reading origin remote", dir, remote_url())?;
        let (owner, repo) = parse_remote(&url).ok_or(ApiError::NotFound)?;
        let number = match requested {
            Some(number) => number,
            None => {
                let branch = required_git_fact("reading current branch", dir, current_branch())?;
                let found = find_pr(&owner, &repo, &branch)?;
                let number = found
                    .as_array()
                    .and_then(|items| items.first())
                    .and_then(|item| item.get("number"))
                    .and_then(Value::as_u64)
                    .ok_or(ApiError::NotFound)?;
                PrNumber(number)
            }
        };
        Ok(Subject {
            owner,
            repo,
            number,
        })
    }
}

impl SharedFailureEvidenceSource for GhSource {
    fn shared_failure_evidence(
        &self,
        subject: &Subject,
        failure: &SubjectFailure,
        deadline: &gh::Deadline,
    ) -> Result<SharedFailureEvidence, ApiError> {
        deadline.check()?;
        let not_before = rfc3339_24_hours_ago()?;
        deadline.check()?;
        collect_shared_failure_evidence_with(
            subject,
            failure,
            &not_before,
            deadline,
            |args| gh::run_gh_deadline(args, deadline),
            |args| gh::run_gh_raw_deadline(args, deadline),
        )
    }
}

const MAX_LOG_EVIDENCE_BYTES: usize = 16 * 1024 * 1024;
/// Coordinate the fixed observation sequence while keeping transport injectable for
/// request/parser tests.  Every path is repository-scoped from `Subject`; callers
/// supply only the primary workflow's newest-first, 24-hour-bounded population.
fn collect_shared_failure_evidence_with(
    subject: &Subject,
    failure: &SubjectFailure,
    not_before: &str,
    deadline: &gh::Deadline,
    mut json: impl FnMut(&[&str]) -> Result<Value, ApiError>,
    mut raw: impl FnMut(&[&str]) -> Result<Vec<u8>, ApiError>,
) -> Result<SharedFailureEvidence, ApiError> {
    deadline.check()?;
    let slug = format!("{}/{}", subject.owner, subject.repo);
    let subject_jobs_path = format!(
        "/repos/{slug}/actions/runs/{}/jobs?filter=latest&per_page=100",
        failure.workflow_run_id
    );
    let subject_job = find_job_for_check(
        &json(&["api", &subject_jobs_path])?,
        failure.check_run_id,
        deadline,
    )?;
    deadline.check()?;
    let subject_log_path = format!("/repos/{slug}/actions/jobs/{}/logs", subject_job.id);
    let mut log_bytes = 0;
    let subject_log =
        decode_evidence_log(raw(&["api", &subject_log_path])?, &mut log_bytes, deadline)?;

    let mut eligible_heads = Vec::new();
    let mut page = 1;
    loop {
        deadline.check()?;
        let pulls_path = format!("/repos/{slug}/pulls?state=open&per_page=100&page={page}");
        let heads = parse_open_heads(&json(&["api", &pulls_path])?, deadline)?;
        let complete = heads.len() < 100;
        eligible_heads.extend(heads);
        if complete {
            break;
        }
        page += 1;
    }
    eligible_heads.retain(|head| {
        !matches!(
            &head.source,
            SharedFailureSource::PullRequest { number, .. } if *number == subject.number.0
        )
    });
    deadline.check()?;
    let main_path = format!("/repos/{slug}/commits/main");
    deadline.check()?;
    eligible_heads.push(EligibleHead {
        sha: required_string(&json(&["api", &main_path])?, "sha")?,
        source: SharedFailureSource::Main,
    });
    deadline.check()?;

    let runs_path =
        format!("/repos/{slug}/actions/workflows/ci.yml/runs?created=>={not_before}&per_page=50");
    let runs = parse_candidate_runs(&json(&["api", &runs_path])?, deadline)?;
    deadline.check()?;
    let selected_runs = select_runs(runs, &eligible_heads, failure.workflow_run_id, not_before);
    deadline.check()?;
    let mut jobs = Vec::with_capacity(selected_runs.len());
    for selected in &selected_runs {
        deadline.check()?;
        let jobs_path = format!(
            "/repos/{slug}/actions/runs/{}/jobs?filter=latest&per_page=100",
            selected.run.id
        );
        let failed = parse_failed_jobs(&json(&["api", &jobs_path])?, deadline)?;
        deadline.check()?;
        let mut with_logs = Vec::with_capacity(failed.len());
        for job in failed {
            deadline.check()?;
            let logs_path = format!("/repos/{slug}/actions/jobs/{}/logs", job.id);
            let log = decode_evidence_log(raw(&["api", &logs_path])?, &mut log_bytes, deadline)?;
            with_logs.push(CandidateJob { log, ..job });
            deadline.check()?;
        }
        jobs.push((selected.run.id, with_logs));
        deadline.check()?;
    }
    Ok(SharedFailureEvidence {
        subject_log,
        selected_runs,
        jobs,
    })
}

fn parse_open_heads(value: &Value, deadline: &gh::Deadline) -> Result<Vec<EligibleHead>, ApiError> {
    let pulls = value
        .as_array()
        .ok_or_else(|| ApiError::Malformed("open PR response is not an array".into()))?;
    let mut heads = Vec::with_capacity(pulls.len());
    for pull in pulls {
        deadline.check()?;
        heads.push(EligibleHead {
            sha: pull
                .pointer("/head/sha")
                .and_then(Value::as_str)
                .map(str::to_owned)
                .ok_or_else(|| ApiError::Malformed("open PR omitted head SHA".into()))?,
            source: SharedFailureSource::PullRequest {
                number: required_u64(pull, "number")?,
                url: required_string(pull, "html_url")?,
            },
        });
    }
    Ok(heads)
}

fn decode_evidence_log(
    bytes: Vec<u8>,
    total: &mut usize,
    deadline: &gh::Deadline,
) -> Result<String, ApiError> {
    deadline.check()?;
    *total = total
        .checked_add(bytes.len())
        .ok_or_else(|| ApiError::Malformed("Actions log evidence size overflow".into()))?;
    if *total > MAX_LOG_EVIDENCE_BYTES {
        return Err(ApiError::Malformed(format!(
            "Actions log evidence exceeds {MAX_LOG_EVIDENCE_BYTES} bytes"
        )));
    }
    let log = String::from_utf8(bytes)
        .map_err(|error| ApiError::Malformed(format!("Actions log is not UTF-8: {error}")))?;
    deadline.check()?;
    Ok(log)
}

fn parse_candidate_runs(
    value: &Value,
    deadline: &gh::Deadline,
) -> Result<Vec<CandidateRun>, ApiError> {
    let runs = required_array(value, "workflow_runs")?;
    let mut parsed = Vec::with_capacity(runs.len());
    for run in runs {
        deadline.check()?;
        let conclusion = match run.get("conclusion") {
            Some(Value::Null) => None,
            Some(Value::String(value)) => Some(value.clone()),
            _ => {
                return Err(ApiError::Malformed(
                    "workflow run omitted string-or-null conclusion".into(),
                ));
            }
        };
        parsed.push(CandidateRun {
            id: required_u64(run, "id")?,
            url: required_string(run, "html_url")?,
            head_sha: required_string(run, "head_sha")?,
            created_at: required_string(run, "created_at")?,
            conclusion,
        });
    }
    Ok(parsed)
}

fn find_job_for_check(
    value: &Value,
    check_run_id: u64,
    deadline: &gh::Deadline,
) -> Result<CandidateJob, ApiError> {
    let jobs = required_array(value, "jobs")?;
    for job in jobs {
        deadline.check()?;
        if job
            .get("check_run_url")
            .and_then(Value::as_str)
            .and_then(|url| url.rsplit('/').next())
            .and_then(|id| id.parse::<u64>().ok())
            == Some(check_run_id)
        {
            return parse_job(job);
        }
    }
    Err(ApiError::Malformed(format!(
        "check run {check_run_id} has no REST job"
    )))
}
fn parse_failed_jobs(
    value: &Value,
    deadline: &gh::Deadline,
) -> Result<Vec<CandidateJob>, ApiError> {
    let jobs = required_array(value, "jobs")?;
    let mut failed = Vec::new();
    for job in jobs {
        deadline.check()?;
        let conclusion = job
            .get("conclusion")
            .and_then(Value::as_str)
            .ok_or_else(|| {
                ApiError::Malformed("comparison job omitted string conclusion".into())
            })?;
        if conclusion == "failure" {
            failed.push(parse_job(job)?);
        }
    }
    Ok(failed)
}

fn parse_job(value: &Value) -> Result<CandidateJob, ApiError> {
    Ok(CandidateJob {
        id: required_u64(value, "id")?,
        name: required_string(value, "name")?,
        url: required_string(value, "html_url")?,
        log: String::new(),
    })
}

fn required_array<'a>(value: &'a Value, name: &str) -> Result<&'a Vec<Value>, ApiError> {
    value
        .get(name)
        .and_then(Value::as_array)
        .ok_or_else(|| ApiError::Malformed(format!("response omitted array {name}")))
}

fn required_string(value: &Value, name: &str) -> Result<String, ApiError> {
    value
        .get(name)
        .and_then(Value::as_str)
        .map(str::to_owned)
        .ok_or_else(|| ApiError::Malformed(format!("response omitted string {name}")))
}

fn required_u64(value: &Value, name: &str) -> Result<u64, ApiError> {
    value
        .get(name)
        .and_then(Value::as_u64)
        .ok_or_else(|| ApiError::Malformed(format!("response omitted integer {name}")))
}

fn rfc3339_24_hours_ago() -> Result<String, ApiError> {
    let seconds = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|error| ApiError::Transport(format!("reading wall clock: {error}")))?
        .saturating_sub(Duration::from_secs(24 * 60 * 60))
        .as_secs();
    Ok(format_unix_rfc3339(seconds))
}

fn format_unix_rfc3339(seconds: u64) -> String {
    // Howard Hinnant's civil-from-days conversion, kept local to avoid a second
    // time dependency at this narrow REST-query boundary.
    let days = (seconds / 86_400) as i64;
    let seconds_of_day = seconds % 86_400;
    let z = days + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1_460 + doe / 36_524 - doe / 146_096) / 365;
    let year = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = doy - (153 * mp + 2) / 5 + 1;
    let month = mp + if mp < 10 { 3 } else { -9 };
    let year = year + i64::from(month <= 2);
    format!(
        "{year:04}-{month:02}-{day:02}T{:02}:{:02}:{:02}Z",
        seconds_of_day / 3_600,
        (seconds_of_day % 3_600) / 60,
        seconds_of_day % 60
    )
}

fn required_git_fact(
    operation: &'static str,
    dir: &std::path::Path,
    fact: anyhow::Result<Option<String>>,
) -> Result<String, ApiError> {
    match fact {
        Ok(Some(value)) => Ok(value),
        Ok(None) => Err(ApiError::NotFound),
        Err(source) => Err(ApiError::Git(crate::pr::gh::GitError {
            operation,
            path: dir.to_path_buf(),
            source: std::sync::Arc::from(
                source.reallocate_into_boxed_dyn_error_without_backtrace(),
            ),
        })),
    }
}

fn graphql_snapshot_page(subject: &Subject, after: Option<&str>) -> Result<Value, ApiError> {
    let query = format!("query={PR_QUERY}");
    let owner = format!("owner={}", subject.owner);
    let name = format!("name={}", subject.repo);
    let number = format!("number={}", subject.number);
    let mut args = vec![
        "api", "graphql", "-f", &query, "-f", &owner, "-f", &name, "-F", &number,
    ];
    let after_arg;
    if let Some(after) = after {
        after_arg = format!("after={after}");
        args.extend(["-f", &after_arg]);
    }
    gh::run_gh(&args)
}

fn rollup_page_info(value: &Value) -> Result<(bool, Option<String>), ApiError> {
    let page_info = value
        .pointer("/data/repository/pullRequest/statusCheckRollup/contexts/pageInfo")
        .ok_or_else(|| ApiError::Malformed("status-check response omitted pageInfo".into()))?;
    let has_next_page = page_info
        .get("hasNextPage")
        .and_then(Value::as_bool)
        .ok_or_else(|| ApiError::Malformed("status-check pageInfo omitted hasNextPage".into()))?;
    Ok((has_next_page, owned(page_info, &["endCursor"])))
}

impl PrSource for GhSource {
    fn resolve(&self, requested: Option<PrNumber>) -> Result<Subject, ApiError> {
        let dir = std::path::Path::new(".");
        Self::resolve_with(
            requested,
            dir,
            || crate::git::remote_url(dir, "origin"),
            || crate::git::current_branch(dir),
            |owner, repo, branch| {
                let slug = format!("{owner}/{repo}");
                gh::run_gh(&[
                    "pr", "list", "--head", branch, "--state", "open", "--repo", &slug, "--json",
                    "number",
                ])
            },
        )
    }

    fn snapshot(&self, subject: &Subject) -> Result<PrSnapshot, ApiError> {
        let mut after = None;
        let mut snapshot: Option<PrSnapshot> = None;
        loop {
            let value = graphql_snapshot_page(subject, after.as_deref())?;
            let mut page = parse_snapshot(&value)?;
            let (has_next_page, end_cursor) = rollup_page_info(&value)?;
            if let Some(existing) = &mut snapshot {
                if existing.head_sha != page.head_sha {
                    return Err(ApiError::Malformed(
                        "PR head changed while paginating status checks".into(),
                    ));
                }
                existing.checks.append(&mut page.checks);
            } else {
                snapshot = Some(page);
            }
            if !has_next_page {
                return snapshot.ok_or_else(|| ApiError::Malformed("empty PR snapshot".into()));
            }
            after = Some(end_cursor.ok_or_else(|| {
                ApiError::Malformed("status-check page hasNextPage without endCursor".into())
            })?);
        }
    }

    fn actions_evidence(
        &self,
        subject: &Subject,
        snapshot: &PrSnapshot,
        workflow_run_ids: &std::collections::BTreeSet<u64>,
    ) -> Result<super::evidence::ActionsEvidence, ApiError> {
        self.evidence_cache.borrow_mut().collect_with(
            subject,
            snapshot,
            workflow_run_ids,
            gh::run_gh,
        )
    }

    fn required_checks(&self, subject: &Subject) -> Result<RequiredChecks, ApiError> {
        let path = format!(
            "/repos/{}/{}/rules/branches/main",
            subject.owner, subject.repo
        );
        parse_required_checks(&gh::run_gh(&["api", &path])?)
    }

    fn ejection_run(&self, subject: &Subject) -> Result<Option<RunRef>, ApiError> {
        let path = format!(
            "/repos/{}/{}/actions/runs?event=merge_group&per_page=100",
            subject.owner, subject.repo
        );
        Ok(parse_ejection_run(
            &gh::run_gh(&["api", &path])?,
            subject.number,
        ))
    }
}

#[cfg(test)]
mod tests {
    //! Fixtures: `rules-queue.json`, `runs-merge-group.json`, and `pr-merged.json`
    //! are captured live. `pr-queued.json`, `pr-open-green.json`, and
    //! `rules-strict.json` are SYNTHESIZED by editing a capture — they are evidence
    //! about our parsing, not about GitHub's response shape.
    use super::*;

    macro_rules! fixture {
        ($name:literal) => {{
            let mut value = serde_json::from_str::<serde_json::Value>(include_str!(concat!(
                "testdata/",
                $name
            )))
            .expect("fixture parses");
            if let Some(pr) = value.pointer_mut("/data/repository/pullRequest") {
                pr["baseRefOid"] = Value::String("base".into());
            }
            value
        }};
    }

    fn io_error_kind(error: &(dyn std::error::Error + 'static)) -> Option<std::io::ErrorKind> {
        let mut current = Some(error);
        while let Some(error) = current {
            if let Some(error) = error.downcast_ref::<std::io::Error>() {
                return Some(error.kind());
            }
            current = error.source();
        }
        None
    }

    #[test]
    fn required_checks_come_from_the_ruleset_not_a_hardcoded_list() {
        let rc = parse_required_checks(&fixture!("rules-queue.json")).unwrap();
        assert_eq!(rc.contexts, vec!["Validate (no e2e)", "e2e gate"]);
        assert!(!rc.strict, "live ruleset is non-strict");
        assert!(rc.queue_present, "live ruleset has a merge_queue rule");
    }

    #[test]
    fn strict_rollback_ruleset_parses_as_strict_without_a_queue() {
        let rc = parse_required_checks(&fixture!("rules-strict.json")).unwrap();
        assert!(rc.strict);
        assert!(!rc.queue_present);
    }

    #[test]
    fn merged_pr_snapshot_carries_commit_and_timestamp() {
        let s = parse_snapshot(&fixture!("pr-merged.json")).unwrap();
        assert_eq!(s.state, PrState::Merged);
        assert!(s.merge_commit.is_some());
        assert!(s.merged_at.is_some());
    }

    #[test]
    fn queued_pr_snapshot_carries_queue_position() {
        let s = parse_snapshot(&fixture!("pr-queued.json")).unwrap();
        assert_eq!(s.state, PrState::Open);
        assert!(s.queue.in_queue);
        assert_eq!(s.queue.position, Some(2));
    }

    #[test]
    fn query_requests_supported_numeric_actions_database_ids() {
        for query in [PR_QUERY, COMMIT_CHECKS_QUERY] {
            assert!(query.contains("databaseId"));
            assert!(!query.contains("fullDatabaseId"));
        }
    }

    #[test]
    fn actions_check_runs_retain_their_stable_provider_identity() {
        let value = serde_json::json!({
            "databaseId": 104431520054_u64,
            "name": "job",
            "status": "COMPLETED",
            "conclusion": "FAILURE",
            "detailsUrl": "https://github.com/o/r/actions/runs/34984065696/job/9",
            "checkSuite": { "app": { "slug": "github-actions" }, "workflowRun": { "databaseId": 34984065696_u64 } }
        });
        let check = parse_check(&value).expect("check run parses").unwrap();
        assert_eq!(
            check.provider,
            CheckProvider::GitHubActions {
                check_run_id: 104431520054,
                workflow_run_id: 34984065696,
            }
        );
    }

    #[test]
    fn malformed_actions_database_ids_fail_closed() {
        for (check_run_id, workflow_run_id) in [
            (serde_json::json!(null), serde_json::json!(42)),
            (serde_json::json!(-1), serde_json::json!(42)),
            (serde_json::json!(42.5), serde_json::json!(42)),
            (serde_json::json!("42"), serde_json::json!(42)),
            (serde_json::json!(42), serde_json::json!(null)),
            (serde_json::json!(42), serde_json::json!(-1)),
            (serde_json::json!(42), serde_json::json!(42.5)),
            (serde_json::json!(42), serde_json::json!("42")),
        ] {
            let value = serde_json::json!({
                "databaseId": check_run_id,
                "name": "job",
                "status": "COMPLETED",
                "conclusion": "FAILURE",
                "checkSuite": { "app": { "slug": "github-actions" }, "workflowRun": { "databaseId": workflow_run_id } }
            });
            assert!(matches!(parse_check(&value), Err(ApiError::Malformed(_))));
        }
    }

    #[test]
    fn maximum_u64_actions_database_ids_remain_exact() {
        let value = serde_json::json!({
            "databaseId": u64::MAX,
            "name": "job",
            "status": "COMPLETED",
            "conclusion": "SUCCESS",
            "checkSuite": {
                "app": { "slug": "github-actions" },
                "workflowRun": { "databaseId": u64::MAX }
            }
        });
        let check = parse_check(&value).expect("maximum IDs parse").unwrap();
        assert_eq!(
            check.provider,
            CheckProvider::GitHubActions {
                check_run_id: u64::MAX,
                workflow_run_id: u64::MAX,
            }
        );
    }

    #[test]
    fn captured_actions_database_id_fixture_preserves_provider_identity() {
        let snapshot = parse_snapshot(&fixture!("pr-actions-database-id.json")).unwrap();
        assert_eq!(
            snapshot.checks[0].provider,
            CheckProvider::GitHubActions {
                check_run_id: 104431520054,
                workflow_run_id: 34984065696,
            }
        );
    }

    #[test]
    fn commit_checks_preserve_actions_database_id_provider_identity() {
        let value = serde_json::json!({
            "data": { "repository": { "object": {
                "oid": "merge-group-sha",
                "statusCheckRollup": { "contexts": { "nodes": [{
                    "name": "captured Actions job",
                    "status": "COMPLETED",
                    "conclusion": "SUCCESS",
                    "databaseId": 104431520054_u64,
                    "checkSuite": {
                        "app": { "slug": "github-actions" },
                        "workflowRun": { "databaseId": 34984065696_u64 }
                    }
                }]}}
            }}}
        });
        let checks = parse_commit_checks(&value).unwrap();
        assert_eq!(
            checks.checks[0].provider,
            CheckProvider::GitHubActions {
                check_run_id: 104431520054,
                workflow_run_id: 34984065696,
            }
        );
    }

    #[test]
    fn checks_flatten_both_union_members() {
        let s = parse_snapshot(&fixture!("pr-open-green.json")).unwrap();
        assert!(s.checks.iter().any(|c| c.name == "Validate (no e2e)"));
        assert!(s.checks.iter().any(|c| c.name == "e2e gate"));
        assert!(s.checks.iter().all(|c| !c.name.is_empty()));
        assert!(s.checks.iter().all(|c| c.state == CheckState::Success));
    }

    #[test]
    fn arbitrary_commit_checks_reuse_the_typed_rollup_parser() {
        let value = serde_json::json!({
            "data": {
                "repository": {
                    "object": {
                        "oid": "merge-group-sha",
                        "statusCheckRollup": {
                            "contexts": {
                                "nodes": [
                                    {
                                        "__typename": "CheckRun",
                                        "name": "Validate (no e2e)",
                                        "status": "COMPLETED",
                                        "conclusion": "SUCCESS"
                                    },
                                    {
                                        "__typename": "StatusContext",
                                        "context": "e2e gate",
                                        "state": "SUCCESS"
                                    }
                                ]
                            }
                        }
                    }
                }
            }
        });

        let parsed = parse_commit_checks(&value).unwrap();

        assert_eq!(parsed.sha, "merge-group-sha");
        assert_eq!(parsed.checks.len(), 2);
        assert!(
            parsed
                .checks
                .iter()
                .all(|check| check.state == CheckState::Success)
        );
    }

    #[test]
    fn head_sha_ref_and_committed_at_are_populated() {
        // All three are load-bearing: the ejection discriminator needs the timestamp,
        // the divergence guard needs the ref and the sha.
        let s = parse_snapshot(&fixture!("pr-open-green.json")).unwrap();
        assert!(!s.head_committed_at.is_empty());
        assert!(!s.head_sha.is_empty());
        assert_eq!(s.head_ref, "worktree-issue-671-timeline-gate");
    }

    #[test]
    fn ejection_run_matches_on_branch_prefix_not_exact_name() {
        // The branch suffix is the BASE sha, not the head, so only a prefix test can
        // match — `?branch=` needs an exact name and is unusable here.
        assert!(parse_ejection_run(&fixture!("runs-merge-group.json"), PrNumber(727)).is_some());
    }

    #[test]
    fn ejection_run_ignores_other_prs() {
        assert!(parse_ejection_run(&fixture!("runs-merge-group.json"), PrNumber(999)).is_none());
    }

    #[test]
    fn ejection_run_picks_the_most_recent_by_created_at() {
        // PR 646 is the only subject with multiple merge-group runs, which is why the
        // fixture retains all three of them.
        let all = fixture!("runs-merge-group.json");
        let newest = all["workflow_runs"]
            .as_array()
            .unwrap()
            .iter()
            .filter(|r| {
                r["head_branch"]
                    .as_str()
                    .unwrap()
                    .starts_with("gh-readonly-queue/main/pr-646-")
            })
            .map(|r| r["created_at"].as_str().unwrap().to_string())
            .max()
            .expect("fixture must retain the pr-646 runs");
        let picked = parse_ejection_run(&all, PrNumber(646)).unwrap();
        assert_eq!(
            picked.created_at, newest,
            "must pick the newest, not the first"
        );
    }
    #[test]
    fn gh_source_resolve_distinguishes_absent_git_facts_from_failures() {
        let dir = std::path::Path::new("/repo");
        let absent_remote = GhSource::resolve_with(
            Some(PrNumber(7)),
            dir,
            || Ok(None),
            || unreachable!(),
            |_, _, _| unreachable!(),
        );
        assert!(matches!(absent_remote, Err(ApiError::NotFound)));

        let remote_failure = GhSource::resolve_with(
            Some(PrNumber(7)),
            dir,
            || {
                Err(anyhow::Error::new(std::io::Error::new(
                    std::io::ErrorKind::PermissionDenied,
                    "injected remote",
                )))
            },
            || unreachable!(),
            |_, _, _| unreachable!(),
        )
        .unwrap_err();
        match remote_failure {
            ApiError::Git(error) => {
                assert_eq!(error.operation, "reading origin remote");
                assert_eq!(error.path, dir);
                assert_eq!(
                    io_error_kind(error.source.as_ref()),
                    Some(std::io::ErrorKind::PermissionDenied)
                );
            }
            other => panic!("expected typed Git failure, got {other:?}"),
        }

        let absent_branch = GhSource::resolve_with(
            None,
            dir,
            || Ok(Some("https://github.com/acme/project".to_owned())),
            || Ok(None),
            |_, _, _| unreachable!(),
        );
        assert!(matches!(absent_branch, Err(ApiError::NotFound)));

        let branch_failure = GhSource::resolve_with(
            None,
            dir,
            || Ok(Some("https://github.com/acme/project".to_owned())),
            || {
                Err(anyhow::Error::new(std::io::Error::new(
                    std::io::ErrorKind::PermissionDenied,
                    "injected branch",
                )))
            },
            |_, _, _| unreachable!(),
        )
        .unwrap_err();
        match branch_failure {
            ApiError::Git(error) => {
                assert_eq!(error.operation, "reading current branch");
                assert_eq!(error.path, dir);
                assert_eq!(
                    io_error_kind(error.source.as_ref()),
                    Some(std::io::ErrorKind::PermissionDenied)
                );
            }
            other => panic!("expected typed Git failure, got {other:?}"),
        }
    }

    #[test]
    fn ejection_run_reports_conclusion_verbatim_without_judging_it() {
        // `decide` judges failure and recency; this layer only reports. PR 646's
        // newest run succeeded even though two older ones failed.
        let picked = parse_ejection_run(&fixture!("runs-merge-group.json"), PrNumber(646)).unwrap();
        assert_eq!(picked.conclusion, "success");
        assert!(picked.url.contains("/actions/runs/"));
    }

    #[test]
    fn remote_urls_parse_to_owner_and_repo() {
        for url in [
            "git@github.com:jaunder-org/jaunder.git",
            "https://github.com/jaunder-org/jaunder.git",
            "https://github.com/jaunder-org/jaunder",
        ] {
            assert_eq!(
                parse_remote(url),
                Some(("jaunder-org".into(), "jaunder".into())),
                "{url}"
            );
        }
        assert_eq!(parse_remote("not-a-remote"), None);
        assert_eq!(parse_remote(""), None);
    }

    #[test]
    fn no_hardcoded_repo_literal_in_the_module() {
        // Reintroducing the org as a string literal breaks this. The needle is built
        // at runtime rather than written out, or this file would match itself.
        let needle = format!("{0}jaunder-org/jaunder{0}", '"');
        assert!(
            !include_str!("snapshot.rs").contains(&needle),
            "repo identity must come from the git remote, not a literal"
        );
    }

    #[test]
    fn resolution_failures_split_exit_two_from_watcher_error() {
        // Failures to ESTABLISH the subject exit 2; tooling failures are reports.
        assert!(matches!(
            resolution_failure(&ApiError::NotFound),
            ResolutionFailure::Bail(_)
        ));
        for tooling in [
            ApiError::GhMissing,
            ApiError::Unauthenticated,
            ApiError::RateLimited { reset_unix: None },
            ApiError::Transport("x".into()),
            ApiError::Malformed("x".into()),
            ApiError::GraphQlErrors("x".into()),
        ] {
            assert_eq!(
                resolution_failure(&tooling),
                ResolutionFailure::Report(Outcome::WatcherError),
                "{tooling:?} is the tooling breaking, not a missing subject"
            );
        }
    }

    #[test]
    fn malformed_payload_is_an_api_error_not_a_panic() {
        let bad = serde_json::json!({ "data": { "repository": null } });
        assert!(matches!(parse_snapshot(&bad), Err(ApiError::Malformed(_))));
    }

    #[test]
    fn a_head_commit_without_a_timestamp_is_malformed_not_defaulted() {
        // Defaulting `head_committed_at` to "" would make every failed merge-group run
        // compare newer than the head — a false `ejected` reached through a door the
        // recency test does not cover.
        let mut v = fixture!("pr-open-green.json");
        v["data"]["repository"]["pullRequest"]["commits"]["nodes"][0]["commit"]["committedDate"] =
            serde_json::Value::Null;
        assert!(matches!(parse_snapshot(&v), Err(ApiError::Malformed(_))));

        let mut v = fixture!("pr-open-green.json");
        v["data"]["repository"]["pullRequest"]["commits"]["nodes"][0]["commit"]["oid"] =
            serde_json::Value::Null;
        assert!(matches!(parse_snapshot(&v), Err(ApiError::Malformed(_))));
    }
    #[test]
    fn shared_failure_collection_scopes_requests_and_joins_exact_check_run() {
        let requests = std::cell::RefCell::new(Vec::new());
        let responses = std::cell::RefCell::new(std::collections::VecDeque::from([
            Ok(serde_json::json!({"jobs": [{
                "id": 71, "name": "subject", "html_url": "job/71",
                "check_run_url": "https://api.github.com/repos/o/r/check-runs/42"
            }]})),
            Ok(serde_json::json!([{
                "number": 9, "html_url": "https://github.com/o/r/pull/9",
                "head": {"sha": "other"}
            }])),
            Ok(serde_json::json!({"sha": "main"})),
            Ok(serde_json::json!({"workflow_runs": [{
                "id": 88, "html_url": "run/88", "head_sha": "other",
                "created_at": "2026-09-15T00:00:00Z", "conclusion": "failure"
            }]})),
            Ok(serde_json::json!({"jobs": [
                {"id": 90, "name": "failed", "html_url": "job/90", "conclusion": "failure"},
                {"id": 91, "name": "green", "html_url": "job/91", "conclusion": "success"}
            ]})),
        ]));
        let raw_paths = std::cell::RefCell::new(Vec::new());
        let subject = crate::pr::test_support::subject();
        let slug = format!("{}/{}", subject.owner, subject.repo);
        let evidence = collect_shared_failure_evidence_with(
            &subject,
            &SubjectFailure {
                workflow_run_id: 7,
                check_run_id: 42,
                name: "subject".into(),
            },
            "2026-09-14T00:00:00Z",
            &gh::Deadline::ten_seconds(),
            |args| {
                requests.borrow_mut().push(args.join(" "));
                responses.borrow_mut().pop_front().expect("scripted JSON")
            },
            |args| {
                raw_paths.borrow_mut().push(args.join(" "));
                Ok(b"error: same failure".to_vec())
            },
        )
        .unwrap();

        assert_eq!(evidence.selected_runs.len(), 1);
        assert_eq!(
            evidence.jobs[0].1.len(),
            1,
            "successful jobs are never inspected"
        );
        assert_eq!(
            raw_paths.into_inner(),
            vec![
                format!("api /repos/{slug}/actions/jobs/71/logs"),
                format!("api /repos/{slug}/actions/jobs/90/logs"),
            ]
        );
        let requests = requests.into_inner();
        assert!(
            requests.iter().all(|request| request.contains(&slug)),
            "every request must derive its repository from Subject"
        );
        assert!(
            requests
                .iter()
                .any(|request| request.contains("pulls?state=open"))
        );
        assert!(
            requests
                .iter()
                .any(|request| request.contains("commits/main"))
        );
        assert!(requests.iter().any(|request| {
            request.contains(
                "actions/workflows/ci.yml/runs?created=>=2026-09-14T00:00:00Z&per_page=50",
            )
        }));
    }

    #[test]
    fn shared_failure_collection_rejects_wrong_job_correlation_and_raw_bytes() {
        let failure = SubjectFailure {
            workflow_run_id: 7,
            check_run_id: 42,
            name: "subject".into(),
        };
        let wrong_job = collect_shared_failure_evidence_with(
            &crate::pr::test_support::subject(),
            &failure,
            "2026-09-14T00:00:00Z",
            &gh::Deadline::ten_seconds(),
            |_| {
                Ok(serde_json::json!({"jobs": [{
                    "id": 71, "name": "subject", "html_url": "job/71",
                    "check_run_url": "https://api.github.com/repos/o/r/check-runs/99"
                }]}))
            },
            |_| unreachable!(),
        );
        assert!(matches!(wrong_job, Err(ApiError::Malformed(_))));

        let responses = std::cell::RefCell::new(std::collections::VecDeque::from([Ok(
            serde_json::json!({"jobs": [{
                "id": 71, "name": "subject", "html_url": "job/71",
                "check_run_url": "https://api.github.com/repos/o/r/check-runs/42"
            }]}),
        )]));
        let bad_log = collect_shared_failure_evidence_with(
            &crate::pr::test_support::subject(),
            &failure,
            "2026-09-14T00:00:00Z",
            &gh::Deadline::ten_seconds(),
            |_| responses.borrow_mut().pop_front().expect("subject jobs"),
            |_| Ok(vec![0xff]),
        );
        assert!(matches!(bad_log, Err(ApiError::Malformed(_))));
    }
    #[test]
    fn malformed_conclusions_degrade_instead_of_silently_excluding_a_match() {
        let deadline = gh::Deadline::ten_seconds();
        for conclusion in [serde_json::json!("failure"), serde_json::json!(42)] {
            let value = serde_json::json!({"workflow_runs": [{
                "id": 88, "html_url": "run/88", "head_sha": "other",
                "created_at": "2026-09-15T00:00:00Z", "conclusion": conclusion
            }]});
            if value["workflow_runs"][0]["conclusion"].is_string() {
                assert!(parse_candidate_runs(&value, &deadline).is_ok());
            } else {
                assert!(matches!(
                    parse_candidate_runs(&value, &deadline),
                    Err(ApiError::Malformed(_))
                ));
            }
        }
        assert!(matches!(
            parse_candidate_runs(
                &serde_json::json!({"workflow_runs": [{
                    "id": 88, "html_url": "run/88", "head_sha": "other",
                    "created_at": "2026-09-15T00:00:00Z"
                }]}),
                &deadline
            ),
            Err(ApiError::Malformed(_))
        ));
        assert!(
            parse_candidate_runs(
                &serde_json::json!({"workflow_runs": [{
                    "id": 88, "html_url": "run/88", "head_sha": "other",

                    "created_at": "2026-09-15T00:00:00Z", "conclusion": null
                }]}),
                &deadline
            )
            .unwrap()[0]
                .conclusion
                .is_none()
        );
        assert!(matches!(
            parse_failed_jobs(
                &serde_json::json!({"jobs": [{
                    "id": 90, "name": "match", "html_url": "job/90", "conclusion": null
                }]}),
                &deadline
            ),
            Err(ApiError::Malformed(_))
        ));
    }

    #[test]
    fn oversized_log_evidence_is_rejected_before_normalization() {
        let mut total = 0;
        let error = decode_evidence_log(
            vec![b'x'; MAX_LOG_EVIDENCE_BYTES + 1],
            &mut total,
            &gh::Deadline::ten_seconds(),
        )
        .unwrap_err();
        assert!(matches!(error, ApiError::Malformed(message) if message.contains("exceeds")));
    }

    #[test]
    fn shared_failure_collection_preserves_query_failures_as_typed_errors() {
        let error = collect_shared_failure_evidence_with(
            &crate::pr::test_support::subject(),
            &SubjectFailure {
                workflow_run_id: 7,
                check_run_id: 42,
                name: "subject".into(),
            },
            "2026-09-14T00:00:00Z",
            &gh::Deadline::ten_seconds(),
            |_| Err(ApiError::GraphQlErrors("bad query".into())),
            |_| unreachable!(),
        )
        .unwrap_err();
        assert_eq!(error, ApiError::GraphQlErrors("bad query".into()));
    }
}
