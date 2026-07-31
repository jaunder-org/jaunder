//! GitHub's JSON → typed values. The last layer that sees JSON at all.
//!
//! Deliberately logic-free: it reports what GitHub said, including conclusions it
//! has no opinion about. Deciding whether a failed merge-group run is *this* PR's
//! ejection — or a stale one from before the last push — belongs to `decide`, which
//! can be tested without any of this.

use serde_json::Value;

use super::gh::{self, ApiError};
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

/// One entry from `statusCheckRollup`, flattened across the `CheckRun` /
/// `StatusContext` union so nothing above this file has to know the union exists.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CheckEntry {
    pub name: String,
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

/// One document for the whole state machine (#729 spec F4).
pub const PR_QUERY: &str = r#"query($owner:String!,$name:String!,$number:Int!){
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
      commits(last:1){ nodes { commit { oid committedDate } } }
      statusCheckRollup {
        contexts(first:100){
          nodes {
            __typename
            ... on CheckRun { name conclusion status detailsUrl startedAt completedAt }
            ... on StatusContext { context state targetUrl createdAt }
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
        .map(|nodes| nodes.iter().filter_map(parse_check).collect())
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
        head_sha: owned(head, &["oid"]).unwrap_or_default(),
        head_ref: owned(pr, &["headRefName"]).unwrap_or_default(),
        head_committed_at: owned(head, &["committedDate"]).unwrap_or_default(),
        checks,
    })
}

/// Flatten one rollup node. `CheckRun` carries `name`/`conclusion`/`status`;
/// `StatusContext` carries `context`/`state`. Both become a `CheckEntry`.
fn parse_check(node: &Value) -> Option<CheckEntry> {
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
        return Some(CheckEntry {
            name: name.to_string(),
            state,
            details_url: owned(node, &["detailsUrl"]),
            started_at: owned(node, &["startedAt"]),
            completed_at: owned(node, &["completedAt"]),
        });
    }
    let context = str_at(node, &["context"])?;
    let state = match str_at(node, &["state"]).unwrap_or("") {
        "SUCCESS" => CheckState::Success,
        "FAILURE" | "ERROR" => CheckState::Failure,
        _ => CheckState::Pending,
    };
    Some(CheckEntry {
        name: context.to_string(),
        state,
        details_url: owned(node, &["targetUrl"]),
        started_at: owned(node, &["createdAt"]),
        completed_at: match state {
            CheckState::Pending => None,
            _ => owned(node, &["createdAt"]),
        },
    })
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
    fn required_checks(&self, subject: &Subject) -> Result<RequiredChecks, ApiError>;
    fn ejection_run(&self, subject: &Subject) -> Result<Option<RunRef>, ApiError>;
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

pub struct GhSource;

impl PrSource for GhSource {
    fn resolve(&self, requested: Option<PrNumber>) -> Result<Subject, ApiError> {
        let dir = std::path::Path::new(".");
        let url = crate::git::remote_url(dir, "origin")
            .ok()
            .flatten()
            .ok_or(ApiError::NotFound)?;
        let (owner, repo) = parse_remote(&url).ok_or(ApiError::NotFound)?;
        let number = match requested {
            Some(n) => n,
            None => {
                let branch = crate::git::current_branch(dir)
                    .ok()
                    .flatten()
                    .ok_or(ApiError::NotFound)?;
                let slug = format!("{owner}/{repo}");
                let found = gh::run_gh(&[
                    "pr", "list", "--head", &branch, "--state", "open", "--repo", &slug, "--json",
                    "number",
                ])?;
                let n = found
                    .as_array()
                    .and_then(|a| a.first())
                    .and_then(|o| o.get("number"))
                    .and_then(Value::as_u64)
                    .ok_or(ApiError::NotFound)?;
                PrNumber(n)
            }
        };
        Ok(Subject {
            owner,
            repo,
            number,
        })
    }

    fn snapshot(&self, subject: &Subject) -> Result<PrSnapshot, ApiError> {
        let query = format!("query={PR_QUERY}");
        let owner = format!("owner={}", subject.owner);
        let name = format!("name={}", subject.repo);
        let number = format!("number={}", subject.number);
        let v = gh::run_gh(&[
            "api", "graphql", "-f", &query, "-f", &owner, "-f", &name, "-F", &number,
        ])?;
        parse_snapshot(&v)
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
        ($name:literal) => {
            serde_json::from_str::<serde_json::Value>(include_str!(concat!("testdata/", $name)))
                .expect("fixture parses")
        };
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
    fn checks_flatten_both_union_members() {
        let s = parse_snapshot(&fixture!("pr-open-green.json")).unwrap();
        assert!(s.checks.iter().any(|c| c.name == "Validate (no e2e)"));
        assert!(s.checks.iter().any(|c| c.name == "e2e gate"));
        assert!(s.checks.iter().all(|c| !c.name.is_empty()));
        assert!(s.checks.iter().all(|c| c.state == CheckState::Success));
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
}
