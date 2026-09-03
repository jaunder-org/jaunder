//! Serialized ADR promotion around the local `adr promote` mutation.
//!
//! This controller owns the durable promoter identity and the fail-closed dequeue
//! policy. Git and GitHub are injected separately: tests can prove publication
//! ordering and retry authorization without a checkout, token, or network.

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};
use std::process::Command;

use anyhow::{Context, Result, anyhow, bail};
use serde::Deserialize;
use serde_json::Value;

use super::decide;
use super::gh;
use super::land::{GhArmer, PrArmer, arm_is_verified};
use super::snapshot::{
    COMMIT_CHECKS_QUERY, CheckState, CommitChecks, MergeStateStatus, Mergeable, PR_QUERY,
    PrSnapshot, PrState, RequiredChecks, parse_commit_checks, parse_required_checks,
    parse_snapshot,
};
use super::{PrNumber, Subject};
use crate::{StepResult, adr, git};

pub const BRANCH: &str = "automation/adr-promoter";
pub const TITLE: &str = "docs(adr): promote pending ADR drafts";
pub const MARKER: &str = "<!-- jaunder-adr-promoter -->";
pub const BOT_LOGIN: &str = "jaunder-adr-promoter[bot]";
pub const BASE_BRANCH: &str = "main";
const MERGE_GROUP_LIMIT: usize = 100;
const PROMOTION_COMMIT_ARGS: [&str; 8] = [
    "-c",
    "user.name=jaunder-adr-promoter[bot]",
    "-c",
    "user.email=jaunder-adr-promoter[bot]@users.noreply.github.com",
    "commit",
    "--no-verify",
    "-m",
    TITLE,
];

fn github_error(error: gh::ApiError) -> anyhow::Error {
    anyhow!(error.detail())
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PromoterEvent {
    Generate,
    PullRequest(PullRequestEvent),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PullRequestEvent {
    pub action: String,
    pub number: PrNumber,
    pub head_ref: String,
    pub head_sha: String,
    pub base_ref: String,
}

#[derive(Deserialize)]
struct RawEvent {
    action: Option<String>,
    pull_request: Option<RawPullRequest>,
}

#[derive(Deserialize)]
struct RawPullRequest {
    number: u64,
    head: RawRef,
    base: RawRef,
}

#[derive(Deserialize)]
struct RawRef {
    #[serde(rename = "ref")]
    name: String,
    sha: String,
}

impl PromoterEvent {
    pub fn from_reader(reader: impl std::io::Read) -> Result<Self> {
        let raw: RawEvent = serde_json::from_reader(reader).context("parsing GitHub event")?;
        let Some(pr) = raw.pull_request else {
            return Ok(Self::Generate);
        };
        Ok(Self::PullRequest(PullRequestEvent {
            action: raw.action.unwrap_or_default(),
            number: PrNumber(pr.number),
            head_ref: pr.head.name,
            head_sha: pr.head.sha,
            base_ref: pr.base.name,
        }))
    }

    fn from_env() -> Result<Self> {
        let path = std::env::var_os("GITHUB_EVENT_PATH")
            .map(PathBuf::from)
            .ok_or_else(|| anyhow!("GITHUB_EVENT_PATH is not set"))?;
        let file = std::fs::File::open(&path)
            .with_context(|| format!("opening GitHub event {}", path.display()))?;
        Self::from_reader(file)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PromoterPullRequest {
    pub number: PrNumber,
    pub head_owner: String,
    pub author: String,
    pub head_ref: String,
    pub head_sha: String,
    pub base_ref: String,
    pub body: String,
    pub is_open: bool,
    pub auto_merge_armed: bool,
    pub in_merge_queue: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PromoterOutcome {
    NoChanges,
    Existing(PrNumber),
    Created(PrNumber),
    Replaced {
        stale: PrNumber,
        successor: PrNumber,
    },
    IgnoredEvent,
    Rearmed(PrNumber),
    NotRearmed(&'static str),
}

impl std::fmt::Display for PromoterOutcome {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::NoChanges => f.write_str("no ADR promotion diff"),
            Self::Existing(number) => write!(f, "promoter PR {number} already owns the branch"),
            Self::Created(number) => write!(f, "created and armed promoter PR {number}"),
            Self::Replaced { stale, successor } => {
                write!(f, "replaced stale promoter PR {stale} with {successor}")
            }
            Self::IgnoredEvent => f.write_str("event does not target the ADR promoter"),
            Self::Rearmed(number) => write!(f, "re-armed promoter PR {number}"),
            Self::NotRearmed(reason) => write!(f, "promoter PR not re-armed: {reason}"),
        }
    }
}

/// Local mutation and publication operations. The controller deliberately keeps
/// `push` separate from `commit`, making the no-publication-before-success rule
/// observable rather than an ordering convention hidden inside a shell script.
pub trait PromoterGit {
    fn prepare_fresh_main(&self) -> Result<()>;
    fn promote(&self) -> Result<()>;
    fn has_staged_diff(&self) -> Result<bool>;
    fn format_staged_markdown(&self) -> Result<()>;
    fn commit(&self) -> Result<()>;
    fn head_sha(&self) -> Result<String>;
    fn push(&self) -> Result<()>;
    fn fetch_exact(&self, reference: &str) -> Result<()>;
    fn refreshed_main_sha(&self) -> Result<String>;
    fn sole_parent(&self, commit: &str) -> Result<Option<String>>;
    fn is_strict_ancestor(&self, ancestor: &str, descendant: &str) -> Result<bool>;
    fn merge_conflicts(&self, main: &str, head: &str) -> Result<bool>;
    fn delete_branch(&self, expected: &str) -> Result<()>;
}

pub trait PromoterPrRead {
    fn repository(&self) -> Result<(String, String)>;
    fn promoter_pull_request(&self) -> Result<Option<PromoterPullRequest>>;
    fn pull_request(&self, number: PrNumber) -> Result<Option<PromoterPullRequest>>;
    fn snapshot(&self, number: PrNumber) -> Result<Option<PrSnapshot>>;
    fn remote_branch_head(&self) -> Result<Option<String>>;
    fn required_checks(&self, base: &str) -> Result<RequiredChecks>;
    fn merge_group_shas(&self, event: &PullRequestEvent) -> Result<Vec<String>>;
    fn commit_checks(&self, sha: &str) -> Result<CommitChecks>;
}

pub trait PromoterPrWrite {
    fn create_pull_request(&self) -> Result<()>;
    fn arm_auto_merge(&self, number: PrNumber) -> Result<()>;
    fn close_pull_request(&self, number: PrNumber) -> Result<()>;
}

pub struct RealPromoterGit {
    repo: PathBuf,
}

impl RealPromoterGit {
    fn current() -> Self {
        Self {
            repo: PathBuf::from("."),
        }
    }

    fn commit_with(&self, run: impl FnOnce(&std::path::Path, &[&str]) -> Result<()>) -> Result<()> {
        // Per-invocation config supplies both Git identities without persistent
        // workflow-local configuration. The generated tracked rename is exactly
        // the delete/rename state that precommit's auto-staging reconciliation
        // rejects; the promoter PR's required checks gate the commit before main.
        run(&self.repo, &PROMOTION_COMMIT_ARGS)
    }

    fn format_staged_markdown_at(repo: &Path) -> Result<()> {
        let changed = git::output(
            repo,
            &[
                "diff",
                "--cached",
                "--name-only",
                "--diff-filter=ACMR",
                "--",
                "*.md",
            ],
        )?;
        let paths = changed
            .lines()
            .filter(|path| !path.is_empty())
            .collect::<Vec<_>>();
        if paths.is_empty() {
            return Ok(());
        }

        let status = Command::new("prettier")
            .arg("--write")
            .args(&paths)
            .current_dir(repo)
            .status()
            .context("running Prettier for promoted Markdown")?;
        if !status.success() {
            bail!("Prettier failed for promoted Markdown ({status})");
        }

        let mut add = vec!["add", "--"];
        add.extend(paths);
        git::run(repo, &add)
    }
}

impl PromoterGit for RealPromoterGit {
    fn prepare_fresh_main(&self) -> Result<()> {
        let dirty = git::working_tree_status(&self.repo)?;
        if !dirty.trim().is_empty() {
            bail!("ADR promoter requires a clean working tree");
        }
        git::run(&self.repo, &["fetch", "origin", BASE_BRANCH])?;
        let fresh_main = format!("origin/{BASE_BRANCH}");
        git::run(&self.repo, &["switch", "--detach", &fresh_main])
    }

    fn promote(&self) -> Result<()> {
        adr::run_promote(&self.repo).map(|_| ())
    }

    fn has_staged_diff(&self) -> Result<bool> {
        Ok(!git::output(&self.repo, &["diff", "--cached", "--name-only"])?.is_empty())
    }

    fn format_staged_markdown(&self) -> Result<()> {
        Self::format_staged_markdown_at(&self.repo)
    }

    fn commit(&self) -> Result<()> {
        self.commit_with(git::run)
    }

    fn head_sha(&self) -> Result<String> {
        git::head_sha(&self.repo)?.ok_or_else(|| anyhow!("promoter commit has no HEAD"))
    }

    fn fetch_exact(&self, reference: &str) -> Result<()> {
        git::run(&self.repo, &["fetch", "origin", reference])
    }

    fn refreshed_main_sha(&self) -> Result<String> {
        git::run(&self.repo, &["fetch", "origin", BASE_BRANCH])?;
        git::output(&self.repo, &["rev-parse", &format!("origin/{BASE_BRANCH}")])
    }

    fn sole_parent(&self, commit: &str) -> Result<Option<String>> {
        let parents = git::lines(&self.repo, &["show", "-s", "--format=%P", commit])?;
        let parents = parents
            .first()
            .map(String::as_str)
            .unwrap_or_default()
            .split_whitespace()
            .collect::<Vec<_>>();
        Ok((parents.len() == 1).then(|| parents[0].to_string()))
    }

    fn is_strict_ancestor(&self, ancestor: &str, descendant: &str) -> Result<bool> {
        if ancestor == descendant {
            return Ok(false);
        }
        let status = git::at(&self.repo)
            .args(["merge-base", "--is-ancestor", ancestor, descendant])
            .status()
            .context("checking commit ancestry")?;
        match status.code() {
            Some(0) => Ok(true),
            Some(1) => Ok(false),
            _ => bail!("git merge-base failed ({status})"),
        }
    }

    fn merge_conflicts(&self, main: &str, head: &str) -> Result<bool> {
        git::merge_tree_conflicts(&self.repo, main, head)
    }

    fn delete_branch(&self, expected: &str) -> Result<()> {
        git::delete_remote_with_lease(&self.repo, "origin", BRANCH, expected)
    }

    fn push(&self) -> Result<()> {
        let destination = format!("HEAD:refs/heads/{BRANCH}");
        git::run(&self.repo, &["push", "origin", &destination])
    }
}

pub struct GhPromoterPr {
    owner: String,
    repo: String,
}

impl GhPromoterPr {
    fn from_env() -> Result<Self> {
        let slug = std::env::var("GITHUB_REPOSITORY").context("GITHUB_REPOSITORY is not set")?;
        let (owner, repo) = slug
            .split_once('/')
            .filter(|(owner, repo)| !owner.is_empty() && !repo.is_empty())
            .ok_or_else(|| anyhow!("GITHUB_REPOSITORY must be owner/repository"))?;
        Ok(Self {
            owner: owner.to_string(),
            repo: repo.to_string(),
        })
    }

    fn slug(&self) -> String {
        format!("{}/{}", self.owner, self.repo)
    }

    fn parse_pull_request(&self, value: &Value) -> Result<PromoterPullRequest> {
        let number = value
            .get("number")
            .and_then(Value::as_u64)
            .ok_or_else(|| anyhow!("promoter PR response has no number"))?;
        let text = |key: &str| {
            value
                .get(key)
                .and_then(Value::as_str)
                .map(str::to_string)
                .ok_or_else(|| anyhow!("promoter PR response has no {key}"))
        };
        Ok(PromoterPullRequest {
            number: PrNumber(number),
            head_owner: value
                .get("headRepositoryOwner")
                .and_then(|owner| owner.get("login"))
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_string(),
            author: value
                .get("author")
                .and_then(|author| author.get("login"))
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_string(),
            head_ref: text("headRefName")?,
            head_sha: text("headRefOid")?,
            base_ref: text("baseRefName")?,
            body: value
                .get("body")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_string(),
            is_open: value.get("state").and_then(Value::as_str) == Some("OPEN"),
            auto_merge_armed: value
                .get("autoMergeRequest")
                .is_some_and(|request| !request.is_null()),
            in_merge_queue: false,
        })
    }

    fn pr_fields() -> &'static str {
        "number,state,author,headRepositoryOwner,headRefName,headRefOid,baseRefName,body,autoMergeRequest"
    }

    fn promoter_pull_request_with(
        &self,
        run: impl FnOnce(&[&str]) -> std::result::Result<Value, gh::ApiError>,
    ) -> Result<Option<PromoterPullRequest>> {
        let slug = self.slug();
        let value = run(&[
            "pr",
            "list",
            "--repo",
            &slug,
            "--state",
            "open",
            "--head",
            BRANCH,
            "--base",
            BASE_BRANCH,
            "--json",
            Self::pr_fields(),
        ])
        .map_err(github_error)?;
        let pulls = value
            .as_array()
            .ok_or_else(|| anyhow!("promoter PR list is not an array"))?
            .iter()
            .map(|pr| self.parse_pull_request(pr))
            .collect::<Result<Vec<_>>>()?
            .into_iter()
            .filter(|pr| is_promoter_occupant(pr, &self.owner))
            .collect::<Vec<_>>();
        match pulls.as_slice() {
            [] => Ok(None),
            [pr] => Ok(Some(pr.clone())),
            _ => bail!("multiple open pull requests occupy the ADR promoter branch"),
        }
    }

    fn pull_request_rest_with(
        &self,
        number: PrNumber,
        run: impl FnOnce(&[&str]) -> std::result::Result<Value, gh::ApiError>,
    ) -> Result<Option<PromoterPullRequest>> {
        let slug = self.slug();
        let number = number.to_string();
        match run(&[
            "pr",
            "view",
            &number,
            "--repo",
            &slug,
            "--json",
            Self::pr_fields(),
        ]) {
            Ok(value) => self.parse_pull_request(&value).map(Some),
            Err(gh::ApiError::NotFound) => Ok(None),
            Err(error) => Err(github_error(error)),
        }
    }

    fn snapshot_with(
        &self,
        number: PrNumber,
        run: impl FnOnce(&[&str]) -> std::result::Result<Value, gh::ApiError>,
    ) -> Result<Option<PrSnapshot>> {
        let query = format!("query={PR_QUERY}");
        let owner = format!("owner={}", self.owner);
        let name = format!("name={}", self.repo);
        let number = format!("number={number}");
        match run(&[
            "api", "graphql", "-f", &query, "-f", &owner, "-f", &name, "-F", &number,
        ]) {
            Ok(value) => parse_snapshot(&value).map(Some).map_err(github_error),
            Err(gh::ApiError::NotFound) => Ok(None),
            Err(error) => Err(github_error(error)),
        }
    }
}

impl PromoterPrRead for GhPromoterPr {
    fn repository(&self) -> Result<(String, String)> {
        Ok((self.owner.clone(), self.repo.clone()))
    }

    fn promoter_pull_request(&self) -> Result<Option<PromoterPullRequest>> {
        self.promoter_pull_request_with(gh::run_gh)
    }

    fn pull_request(&self, number: PrNumber) -> Result<Option<PromoterPullRequest>> {
        self.pull_request_rest_with(number, gh::run_gh)
    }

    fn snapshot(&self, number: PrNumber) -> Result<Option<PrSnapshot>> {
        self.snapshot_with(number, gh::run_gh)
    }
    fn remote_branch_head(&self) -> Result<Option<String>> {
        let path = format!("/repos/{}/{}/git/ref/heads/{BRANCH}", self.owner, self.repo);
        match gh::run_gh(&["api", &path]) {
            Ok(value) => Ok(value
                .get("object")
                .and_then(|object| object.get("sha"))
                .and_then(Value::as_str)
                .map(str::to_string)),
            Err(gh::ApiError::NotFound) => Ok(None),
            Err(error) => Err(github_error(error)),
        }
    }

    fn required_checks(&self, base: &str) -> Result<RequiredChecks> {
        let path = format!("/repos/{}/{}/rules/branches/{base}", self.owner, self.repo);
        let value = gh::run_gh(&["api", &path]).map_err(github_error)?;
        parse_required_checks(&value).map_err(github_error)
    }

    fn merge_group_shas(&self, event: &PullRequestEvent) -> Result<Vec<String>> {
        let path = format!(
            "/repos/{}/{}/actions/runs?event=merge_group&per_page={MERGE_GROUP_LIMIT}",
            self.owner, self.repo
        );
        let runs = gh::run_gh(&["api", &path]).map_err(github_error)?;
        let mut correlated = BTreeSet::new();
        // Queue branch suffixes identify the base tip, not the PR head. Confirm
        // correlation from the synthetic merge commit's parents instead of
        // treating branch-name recency as authorization to retry.
        for candidate in parse_merge_group_candidates(&runs, event) {
            let path = format!("/repos/{}/{}/commits/{candidate}", self.owner, self.repo);
            let commit = gh::run_gh(&["api", &path]).map_err(github_error)?;
            if commit_has_parent(&commit, &candidate, &event.head_sha) {
                correlated.insert(candidate);
            }
        }
        Ok(correlated.into_iter().collect())
    }

    fn commit_checks(&self, sha: &str) -> Result<CommitChecks> {
        let query = format!("query={COMMIT_CHECKS_QUERY}");
        let owner = format!("owner={}", self.owner);
        let name = format!("name={}", self.repo);
        let oid = format!("oid={sha}");
        let value = gh::run_gh(&[
            "api", "graphql", "-f", &query, "-f", &owner, "-f", &name, "-f", &oid,
        ])
        .map_err(github_error)?;
        parse_commit_checks(&value).map_err(github_error)
    }
}

impl PromoterPrWrite for GhPromoterPr {
    fn create_pull_request(&self) -> Result<()> {
        let slug = self.slug();
        gh::run_gh_raw(&[
            "pr",
            "create",
            "--repo",
            &slug,
            "--head",
            BRANCH,
            "--base",
            BASE_BRANCH,
            "--title",
            TITLE,
            "--body",
            MARKER,
        ])
        .map_err(github_error)?;
        Ok(())
    }

    fn arm_auto_merge(&self, number: PrNumber) -> Result<()> {
        let subject = Subject {
            owner: self.owner.clone(),
            repo: self.repo.clone(),
            number,
        };
        GhArmer.arm_auto_merge(&subject).map_err(github_error)?;
        Ok(())
    }

    fn close_pull_request(&self, number: PrNumber) -> Result<()> {
        let slug = self.slug();
        let number = number.to_string();
        gh::run_gh_raw(&["pr", "close", &number, "--repo", &slug]).map_err(github_error)?;
        Ok(())
    }
}

fn parse_merge_group_candidates(value: &Value, event: &PullRequestEvent) -> Vec<String> {
    let prefix = format!("gh-readonly-queue/{}/pr-{}-", event.base_ref, event.number);
    value
        .get("workflow_runs")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter(|run| run.get("event").and_then(Value::as_str) == Some("merge_group"))
        .filter(|run| {
            run.get("head_branch")
                .and_then(Value::as_str)
                .is_some_and(|branch| branch.starts_with(&prefix))
        })
        .filter_map(|run| run.get("head_sha").and_then(Value::as_str))
        .map(str::to_string)
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect()
}

fn commit_has_parent(value: &Value, candidate: &str, parent: &str) -> bool {
    value.get("sha").and_then(Value::as_str) == Some(candidate)
        && value
            .get("parents")
            .and_then(Value::as_array)
            .is_some_and(|parents| {
                parents
                    .iter()
                    .any(|entry| entry.get("sha").and_then(Value::as_str) == Some(parent))
            })
}

fn is_promoter_occupant(pr: &PromoterPullRequest, owner: &str) -> bool {
    pr.is_open && pr.head_owner == owner && pr.head_ref == BRANCH && pr.base_ref == BASE_BRANCH
}

fn is_promoter_identity(pr: &PromoterPullRequest, owner: &str) -> bool {
    is_promoter_occupant(pr, owner) && pr.author == BOT_LOGIN && pr.body.contains(MARKER)
}

fn singleton<R: PromoterPrRead>(read: &R) -> Result<Option<PromoterPullRequest>> {
    let (owner, _) = read.repository()?;
    let Some(pr) = read.promoter_pull_request()? else {
        return Ok(None);
    };
    if !is_promoter_identity(&pr, &owner) {
        bail!("open pull request occupies the ADR promoter branch with a different identity");
    }
    Ok(Some(pr))
}

fn contexts_are_green(required: &RequiredChecks, checks: &CommitChecks) -> bool {
    !required.contexts.is_empty()
        && required.contexts.iter().all(|name| {
            decide::resolve_context(&checks.checks, name)
                .is_some_and(|check| check.state == CheckState::Success)
        })
}

pub fn run_with<G, R, W>(
    event: PromoterEvent,
    git: &G,
    read: &R,
    write: &W,
) -> Result<PromoterOutcome>
where
    G: PromoterGit,
    R: PromoterPrRead,
    W: PromoterPrWrite,
{
    match event {
        PromoterEvent::Generate => generate(git, read, write),
        PromoterEvent::PullRequest(event) => recover_dequeue(&event, read, write),
    }
}

fn failed_context(required: &RequiredChecks, snapshot: &PrSnapshot) -> bool {
    required.contexts.iter().any(|name| {
        decide::resolve_context(&snapshot.checks, name)
            .is_some_and(|check| check.state == CheckState::Failure)
    })
}

fn close_verified<R: PromoterPrRead, W: PromoterPrWrite>(
    read: &R,
    write: &W,
    expected: &PromoterPullRequest,
) -> Result<()> {
    let (owner, _) = read.repository()?;
    let current = read
        .pull_request(expected.number)?
        .ok_or_else(|| anyhow!("promoter PR disappeared before close"))?;
    if !is_promoter_identity(&current, &owner)
        || current.number != expected.number
        || current.head_sha != expected.head_sha
        || current.head_ref != expected.head_ref
        || current.base_ref != expected.base_ref
        || read.remote_branch_head()?.is_some()
    {
        bail!("promoter PR changed before close");
    }
    let close = write.close_pull_request(expected.number);
    let closed = read
        .pull_request(expected.number)?
        .ok_or_else(|| anyhow!("promoter PR disappeared after close"))?;
    if !closed.is_open
        && closed.number == expected.number
        && closed.head_owner == expected.head_owner
        && closed.author == expected.author
        && closed.head_sha == expected.head_sha
        && closed.head_ref == expected.head_ref
        && closed.base_ref == expected.base_ref
        && closed.body.contains(MARKER)
    {
        return Ok(());
    }
    match close {
        Ok(()) => bail!("promoter PR close was not verified"),
        Err(error) => Err(error).context("closing promoter PR was not verified"),
    }
}

fn delete_verified<G: PromoterGit, R: PromoterPrRead>(
    git: &G,
    read: &R,
    expected: &str,
) -> Result<()> {
    let deleted = git.delete_branch(expected);
    match read.remote_branch_head()? {
        None => Ok(()),
        Some(actual) if actual == expected => {
            deleted.and_then(|_| bail!("promoter branch remained after leased deletion"))
        }
        Some(_) => bail!("promoter branch changed during leased deletion"),
    }
}

fn arm_verified<R: PromoterPrRead, W: PromoterPrWrite>(
    read: &R,
    write: &W,
    number: PrNumber,
    expected_head: &str,
) -> Result<()> {
    let arm = write.arm_auto_merge(number);
    let (owner, _) = read.repository()?;
    let pr = read
        .pull_request(number)?
        .ok_or_else(|| anyhow!("promoter PR disappeared after auto-merge arm"))?;
    let snapshot = read
        .snapshot(number)?
        .ok_or_else(|| anyhow!("promoter PR snapshot disappeared after auto-merge arm"))?;
    if is_promoter_identity(&pr, &owner)
        && pr.number == number
        && pr.head_sha == expected_head
        && read.remote_branch_head()?.as_deref() == Some(expected_head)
        && snapshot.state == PrState::Open
        && snapshot.head_ref == BRANCH
        && snapshot.head_sha == expected_head
        && arm_is_verified(snapshot.auto_merge_armed, snapshot.queue.in_queue)
    {
        return Ok(());
    }
    match arm {
        Ok(()) => bail!(
            "GitHub did not verify auto-merge or queue membership on the unchanged promoter head"
        ),
        Err(error) => Err(error).context("auto-merge arm was not verified"),
    }
}

fn existing_or_retire<G: PromoterGit, R: PromoterPrRead, W: PromoterPrWrite>(
    git: &G,
    read: &R,
    write: &W,
) -> Result<Option<PrNumber>> {
    let branch = read.remote_branch_head()?;
    let Some(pr) = singleton(read)? else {
        if let Some(head) = branch {
            delete_verified(git, read, &head)?;
        }
        return Ok(None);
    };
    let Some(head) = branch else {
        close_verified(read, write, &pr)?;
        return Ok(Some(pr.number));
    };
    if head != pr.head_sha {
        bail!("open promoter PR head differs from stable branch");
    }
    let snapshot = read
        .snapshot(pr.number)?
        .ok_or_else(|| anyhow!("promoter PR disappeared during classification"))?;
    if snapshot.state != PrState::Open || snapshot.head_ref != BRANCH || snapshot.head_sha != head {
        bail!("promoter snapshot differs from the stable branch");
    }
    if snapshot.mergeable == Mergeable::Conflicting
        && snapshot.merge_state_status == MergeStateStatus::Dirty
    {
        git.fetch_exact(&head)?;
        let main = git.refreshed_main_sha()?;
        if snapshot.base_sha == main
            && let Some(parent) = git.sole_parent(&head)?
            && git.is_strict_ancestor(&parent, &main)?
            && git.merge_conflicts(&main, &head)?
        {
            delete_verified(git, read, &head)?;
            close_verified(read, write, &pr)?;
            return Ok(Some(pr.number));
        }
        // GitHub reports this head as conflicted, but local evidence did not
        // authorize retirement. Preserve the immutable attempt rather than
        // trying to arm or mutate it.
        return Ok(Some(pr.number));
    }
    if snapshot.auto_merge_armed || snapshot.queue.in_queue {
        return Ok(Some(pr.number));
    }
    let required = read.required_checks(BASE_BRANCH)?;
    if !required.queue_present || required.contexts.is_empty() {
        bail!("ADR promoter requires a merge queue with required contexts");
    }
    if failed_context(&required, &snapshot) {
        return Ok(Some(pr.number));
    }
    arm_verified(read, write, pr.number, &head)?;
    Ok(Some(pr.number))
}

fn generate<G: PromoterGit, R: PromoterPrRead, W: PromoterPrWrite>(
    git: &G,
    read: &R,
    write: &W,
) -> Result<PromoterOutcome> {
    let retired = existing_or_retire(git, read, write)?;
    if let Some(number) = retired
        && singleton(read)?.is_some()
    {
        return Ok(PromoterOutcome::Existing(number));
    }
    git.prepare_fresh_main()?;
    git.promote()?;
    if !git.has_staged_diff()? {
        return Ok(PromoterOutcome::NoChanges);
    }
    let queue = read.required_checks(BASE_BRANCH)?;
    if !queue.queue_present || queue.contexts.is_empty() {
        bail!("ADR promoter requires a merge queue with required contexts");
    }
    git.format_staged_markdown()?;
    git.commit()?;
    let head = git.head_sha()?;
    let push = git.push();
    if read.remote_branch_head()?.as_deref() != Some(head.as_str()) {
        return match push {
            Ok(()) => Err(anyhow!(
                "remote promoter head does not equal the generated commit"
            )),
            Err(error) => Err(error).context("promoter push was not verified"),
        };
    }
    let create_error = write.create_pull_request().err();
    let created = match singleton(read)? {
        Some(pr) if pr.head_sha == head => pr,
        Some(_) => bail!("promoter PR head does not equal the generated commit"),
        None => {
            return Err(
                create_error.unwrap_or_else(|| anyhow!("created promoter PR was not found"))
            );
        }
    };
    arm_verified(read, write, created.number, &head)?;
    Ok(match retired {
        Some(stale) => PromoterOutcome::Replaced {
            stale,
            successor: created.number,
        },
        None => PromoterOutcome::Created(created.number),
    })
}

fn recover_dequeue<R: PromoterPrRead, W: PromoterPrWrite>(
    event: &PullRequestEvent,
    read: &R,
    write: &W,
) -> Result<PromoterOutcome> {
    if event.action != "dequeued" || event.head_ref != BRANCH || event.base_ref != BASE_BRANCH {
        return Ok(PromoterOutcome::IgnoredEvent);
    }
    let (owner, _) = read.repository()?;
    let Some(pr) = read.pull_request(event.number)? else {
        return Ok(PromoterOutcome::NotRearmed("pull request is absent"));
    };
    if !is_promoter_identity(&pr, &owner)
        || pr.number != event.number
        || pr.head_ref != event.head_ref
        || pr.head_sha != event.head_sha
        || pr.base_ref != event.base_ref
    {
        return Ok(PromoterOutcome::NotRearmed(
            "event identity does not match the open promoter",
        ));
    }
    if read.remote_branch_head()?.as_deref() != Some(event.head_sha.as_str()) {
        return Ok(PromoterOutcome::NotRearmed(
            "remote promoter branch no longer matches the event head",
        ));
    }

    let required = read.required_checks(&event.base_ref)?;
    if !required.queue_present {
        return Ok(PromoterOutcome::NotRearmed("merge queue is absent"));
    }
    if required.contexts.is_empty() {
        return Ok(PromoterOutcome::NotRearmed("required contexts are absent"));
    }
    let merge_groups = read.merge_group_shas(event)?;
    let [merge_group_sha] = merge_groups.as_slice() else {
        return Ok(PromoterOutcome::NotRearmed(
            "merge-group correlation is absent or ambiguous",
        ));
    };
    let head_checks = read.commit_checks(&event.head_sha)?;
    let merge_checks = read.commit_checks(merge_group_sha)?;
    if head_checks.sha != event.head_sha || merge_checks.sha != merge_group_sha.as_str() {
        return Ok(PromoterOutcome::NotRearmed(
            "context evidence names a different commit",
        ));
    }
    if !contexts_are_green(&required, &head_checks) || !contexts_are_green(&required, &merge_checks)
    {
        return Ok(PromoterOutcome::NotRearmed(
            "required contexts are missing, incomplete, or failed",
        ));
    }

    arm_verified(read, write, event.number, &event.head_sha)?;
    Ok(PromoterOutcome::Rearmed(event.number))
}

pub fn execute() -> StepResult {
    let result = PromoterEvent::from_env().and_then(|event| {
        let git = RealPromoterGit::current();
        let github = GhPromoterPr::from_env()?;
        run_with(event, &git, &github, &github)
    });
    match result {
        Ok(outcome) => StepResult::ok("adr-promoter").detail(outcome.to_string()),
        Err(error) => StepResult::fail("adr-promoter").detail(format!("{error:#}")),
    }
}

#[cfg(test)]
mod tests {
    use std::cell::{Cell, RefCell};
    use std::collections::BTreeMap;
    use std::rc::Rc;

    use serde_json::json;

    use super::*;
    use crate::pr::snapshot::{CheckEntry, MergeStateStatus, Mergeable, PrSnapshot, QueueState};

    struct FakeGit {
        calls: RefCell<Vec<&'static str>>,
        staged_diff: bool,
        fail_on: Option<&'static str>,
        head_sha: String,
        parent: Option<String>,
        ancestor: bool,
        conflicts: bool,
        delete_updates: bool,
        push_head: String,
        remote_head: Option<Rc<RefCell<Option<String>>>>,
        trace: Rc<RefCell<Vec<&'static str>>>,
        fetched: RefCell<Vec<String>>,
        parent_commits: RefCell<Vec<String>>,
        ancestry: RefCell<Vec<(String, String)>>,
        merges: RefCell<Vec<(String, String)>>,
        deleted: RefCell<Vec<String>>,
        push_updates: bool,
    }

    impl FakeGit {
        fn new(staged_diff: bool) -> Self {
            Self {
                calls: RefCell::new(Vec::new()),
                staged_diff,
                fail_on: None,
                head_sha: "promoted-head".into(),
                parent: Some("old-main".into()),
                ancestor: true,
                conflicts: true,
                delete_updates: true,
                push_head: "promoted-head".into(),
                remote_head: None,
                trace: Rc::new(RefCell::new(Vec::new())),
                fetched: RefCell::new(Vec::new()),
                parent_commits: RefCell::new(Vec::new()),
                ancestry: RefCell::new(Vec::new()),
                merges: RefCell::new(Vec::new()),
                deleted: RefCell::new(Vec::new()),
                push_updates: true,
            }
        }

        fn failing(operation: &'static str) -> Self {
            Self {
                fail_on: Some(operation),
                ..Self::new(true)
            }
        }

        fn connected(staged_diff: bool, github: &FakeGithub) -> Self {
            Self {
                remote_head: Some(Rc::clone(&github.remote_head)),
                trace: Rc::clone(&github.trace),
                ..Self::new(staged_diff)
            }
        }

        fn call(&self, operation: &'static str) -> Result<()> {
            self.calls.borrow_mut().push(operation);
            self.trace.borrow_mut().push(operation);
            if self.fail_on == Some(operation) {
                bail!("{operation} failed");
            }
            Ok(())
        }
    }

    impl PromoterGit for FakeGit {
        fn prepare_fresh_main(&self) -> Result<()> {
            self.call("prepare")
        }

        fn promote(&self) -> Result<()> {
            self.call("promote")
        }

        fn has_staged_diff(&self) -> Result<bool> {
            self.call("diff")?;
            Ok(self.staged_diff)
        }

        fn format_staged_markdown(&self) -> Result<()> {
            self.call("format")
        }

        fn commit(&self) -> Result<()> {
            self.call("commit")
        }

        fn head_sha(&self) -> Result<String> {
            self.call("head")?;
            Ok(self.head_sha.clone())
        }

        fn push(&self) -> Result<()> {
            let result = self.call("push");
            if self.push_updates
                && let Some(remote) = &self.remote_head
            {
                remote.borrow_mut().replace(self.push_head.clone());
            }
            result
        }

        fn fetch_exact(&self, reference: &str) -> Result<()> {
            self.fetched.borrow_mut().push(reference.into());
            self.call("fetch")
        }

        fn refreshed_main_sha(&self) -> Result<String> {
            self.call("main")?;
            Ok("main-head".into())
        }

        fn sole_parent(&self, commit: &str) -> Result<Option<String>> {
            self.parent_commits.borrow_mut().push(commit.into());
            self.call("parent")?;
            Ok(self.parent.clone())
        }

        fn is_strict_ancestor(&self, ancestor: &str, descendant: &str) -> Result<bool> {
            self.ancestry
                .borrow_mut()
                .push((ancestor.into(), descendant.into()));
            self.call("ancestor")?;
            Ok(self.ancestor)
        }

        fn merge_conflicts(&self, main: &str, head: &str) -> Result<bool> {
            self.merges.borrow_mut().push((main.into(), head.into()));
            self.call("conflicts")?;
            Ok(self.conflicts)
        }

        fn delete_branch(&self, _expected: &str) -> Result<()> {
            self.deleted.borrow_mut().push(_expected.into());
            let result = self.call("delete");
            if self.delete_updates
                && let Some(remote) = &self.remote_head
            {
                remote.borrow_mut().take();
            }
            result
        }
    }

    struct FakeGithub {
        owner: String,
        pulls: RefCell<Vec<PromoterPullRequest>>,
        remote_head: Rc<RefCell<Option<String>>>,
        required: RequiredChecks,
        required_reads: Cell<usize>,
        merge_groups: Vec<String>,
        checks: BTreeMap<String, CommitChecks>,
        writes: RefCell<Vec<&'static str>>,
        trace: Rc<RefCell<Vec<&'static str>>>,
        arm_to_queue: bool,
        head_after_arm: Option<String>,
        snapshot_mergeable: Mergeable,
        snapshot_status: MergeStateStatus,
        snapshot_base: String,
        snapshot_checks: Vec<CheckEntry>,
        close_updates: bool,
        close_fails: bool,
        arm_fails: bool,
    }

    impl FakeGithub {
        fn empty() -> Self {
            Self {
                owner: "jaunder-org".into(),
                pulls: RefCell::new(Vec::new()),
                remote_head: Rc::new(RefCell::new(None)),
                required: required(),
                required_reads: Cell::new(0),
                merge_groups: vec!["merge-group".into()],
                checks: BTreeMap::from([
                    ("event-head".into(), green_checks("event-head")),
                    ("merge-group".into(), green_checks("merge-group")),
                ]),
                writes: RefCell::new(Vec::new()),
                trace: Rc::new(RefCell::new(Vec::new())),
                arm_to_queue: false,
                head_after_arm: None,
                snapshot_mergeable: Mergeable::Mergeable,
                snapshot_status: MergeStateStatus::Clean,
                snapshot_base: "base-head".into(),
                snapshot_checks: green_checks("snapshot").checks,
                close_updates: true,
                close_fails: false,
                arm_fails: false,
            }
        }

        fn dequeue_ready() -> Self {
            let fake = Self::empty();
            fake.remote_head.borrow_mut().replace("event-head".into());
            fake.pulls.borrow_mut().push(promoter_pr("event-head"));
            fake
        }
    }

    impl PromoterPrRead for FakeGithub {
        fn repository(&self) -> Result<(String, String)> {
            Ok((self.owner.clone(), "jaunder".into()))
        }

        fn promoter_pull_request(&self) -> Result<Option<PromoterPullRequest>> {
            let pulls = self
                .pulls
                .borrow()
                .iter()
                .filter(|pr| is_promoter_occupant(pr, &self.owner))
                .cloned()
                .collect::<Vec<_>>();
            match pulls.as_slice() {
                [] => Ok(None),
                [pr] => Ok(Some(pr.clone())),
                _ => bail!("multiple open pull requests occupy the ADR promoter branch"),
            }
        }

        fn pull_request(&self, number: PrNumber) -> Result<Option<PromoterPullRequest>> {
            Ok(self
                .pulls
                .borrow()
                .iter()
                .find(|pr| pr.number == number)
                .cloned())
        }

        fn snapshot(&self, number: PrNumber) -> Result<Option<PrSnapshot>> {
            Ok(self.pull_request(number)?.map(|pr| PrSnapshot {
                state: crate::pr::snapshot::PrState::Open,
                merged_at: None,
                merge_commit: None,
                mergeable: self.snapshot_mergeable,
                merge_state_status: self.snapshot_status,
                auto_merge_armed: pr.auto_merge_armed,
                queue: QueueState {
                    in_queue: pr.in_merge_queue,
                    position: None,
                },
                head_sha: pr.head_sha,
                head_ref: pr.head_ref,
                base_sha: self.snapshot_base.clone(),
                head_committed_at: "2026-01-01T00:00:00Z".into(),
                checks: self.snapshot_checks.clone(),
            }))
        }

        fn remote_branch_head(&self) -> Result<Option<String>> {
            Ok(self.remote_head.borrow().clone())
        }

        fn required_checks(&self, _base: &str) -> Result<RequiredChecks> {
            self.required_reads.set(self.required_reads.get() + 1);
            Ok(self.required.clone())
        }

        fn merge_group_shas(&self, _event: &PullRequestEvent) -> Result<Vec<String>> {
            Ok(self.merge_groups.clone())
        }

        fn commit_checks(&self, sha: &str) -> Result<CommitChecks> {
            self.checks
                .get(sha)
                .cloned()
                .ok_or_else(|| anyhow!("no checks for {sha}"))
        }
    }

    impl PromoterPrWrite for FakeGithub {
        fn create_pull_request(&self) -> Result<()> {
            self.writes.borrow_mut().push("create");
            self.trace.borrow_mut().push("create");
            let number = self
                .pulls
                .borrow()
                .iter()
                .map(|pr| pr.number.0)
                .max()
                .map_or(742, |number| number + 1);
            self.pulls
                .borrow_mut()
                .push(promoter_pr_with_number(number, "promoted-head"));
            Ok(())
        }

        fn arm_auto_merge(&self, number: PrNumber) -> Result<()> {
            self.writes.borrow_mut().push("arm");
            self.trace.borrow_mut().push("arm");
            let mut pulls = self.pulls.borrow_mut();
            let pr = pulls
                .iter_mut()
                .find(|pr| pr.number == number)
                .ok_or_else(|| anyhow!("missing PR"))?;
            if self.arm_to_queue {
                pr.in_merge_queue = true;
            } else {
                pr.auto_merge_armed = true;
            }
            if let Some(head) = &self.head_after_arm {
                pr.head_sha.clone_from(head);
            }
            if self.arm_fails {
                bail!("arm failed");
            }
            Ok(())
        }

        fn close_pull_request(&self, number: PrNumber) -> Result<()> {
            self.writes.borrow_mut().push("close");
            self.trace.borrow_mut().push("close");
            if self.close_updates {
                let mut pulls = self.pulls.borrow_mut();
                let pr = pulls
                    .iter_mut()
                    .find(|pr| pr.number == number)
                    .ok_or_else(|| anyhow!("missing PR"))?;
                pr.is_open = false;
            }
            if self.close_fails {
                bail!("close failed");
            }
            Ok(())
        }
    }

    fn required() -> RequiredChecks {
        RequiredChecks {
            contexts: vec!["Validate (no e2e)".into(), "e2e gate".into()],
            strict: false,
            queue_present: true,
        }
    }

    fn check(name: &str, state: CheckState) -> CheckEntry {
        CheckEntry {
            name: name.into(),
            state,
            details_url: None,
            started_at: Some("2026-08-24T00:00:00Z".into()),
            completed_at: (state != CheckState::Pending).then(|| "2026-08-24T00:01:00Z".into()),
        }
    }

    fn green_checks(sha: &str) -> CommitChecks {
        CommitChecks {
            sha: sha.into(),
            checks: vec![
                check("Validate (no e2e)", CheckState::Success),
                check("e2e gate", CheckState::Success),
            ],
        }
    }

    fn promoter_pr(sha: &str) -> PromoterPullRequest {
        promoter_pr_with_number(742, sha)
    }

    fn promoter_pr_with_number(number: u64, sha: &str) -> PromoterPullRequest {
        PromoterPullRequest {
            number: PrNumber(number),
            head_owner: "jaunder-org".into(),
            author: BOT_LOGIN.into(),
            head_ref: BRANCH.into(),
            head_sha: sha.into(),
            base_ref: BASE_BRANCH.into(),
            body: format!("Automated ADR promotion.\n\n{MARKER}"),
            is_open: true,
            auto_merge_armed: false,
            in_merge_queue: false,
        }
    }

    fn conflicted_github() -> FakeGithub {
        let mut github = FakeGithub::dequeue_ready();
        github.snapshot_mergeable = Mergeable::Conflicting;
        github.snapshot_status = MergeStateStatus::Dirty;
        github.snapshot_base = "main-head".into();
        github
    }

    fn dequeue_event() -> PullRequestEvent {
        PullRequestEvent {
            action: "dequeued".into(),
            number: PrNumber(742),
            head_ref: BRANCH.into(),
            head_sha: "event-head".into(),
            base_ref: BASE_BRANCH.into(),
        }
    }

    fn assert_not_rearmed(github: &FakeGithub) {
        let outcome = run_with(
            PromoterEvent::PullRequest(dequeue_event()),
            &FakeGit::new(true),
            github,
            github,
        )
        .unwrap();
        assert!(
            matches!(outcome, PromoterOutcome::NotRearmed(_)),
            "{outcome:?}"
        );
        assert!(github.writes.borrow().is_empty());
    }

    #[test]
    fn promotion_commit_passes_deterministic_bot_identity_at_the_git_boundary() {
        let git = RealPromoterGit {
            repo: PathBuf::from("checkout"),
        };

        git.commit_with(|repo, args| {
            assert_eq!(repo, std::path::Path::new("checkout"));
            assert_eq!(
                args,
                [
                    "-c",
                    "user.name=jaunder-adr-promoter[bot]",
                    "-c",
                    "user.email=jaunder-adr-promoter[bot]@users.noreply.github.com",
                    "commit",
                    "--no-verify",
                    "-m",
                    TITLE,
                ]
            );
            Ok(())
        })
        .unwrap();
    }

    #[test]
    fn open_pr_lookup_passes_branch_only_and_parses_owner() {
        let github = GhPromoterPr {
            owner: "jaunder-org".into(),
            repo: "jaunder".into(),
        };

        let pull = github
            .promoter_pull_request_with(|args| {
                assert_eq!(
                    args,
                    [
                        "pr",
                        "list",
                        "--repo",
                        "jaunder-org/jaunder",
                        "--state",
                        "open",
                        "--head",
                        BRANCH,
                        "--base",
                        BASE_BRANCH,
                        "--json",
                        GhPromoterPr::pr_fields(),
                    ]
                );
                assert!(!GhPromoterPr::pr_fields().contains("isInMergeQueue"));
                Ok(json!([{
                    "number": 742,
                    "state": "OPEN",
                    "headRepositoryOwner": {"login": "jaunder-org"},
                    "author": {"login": BOT_LOGIN},
                    "headRefName": BRANCH,
                    "headRefOid": "queued-head",
                    "baseRefName": BASE_BRANCH,
                    "body": MARKER,
                    "autoMergeRequest": null
                }]))
            })
            .unwrap();

        assert_eq!(pull.unwrap().head_owner, "jaunder-org");
    }

    #[test]
    fn marked_promoter_branch_from_a_non_bot_author_is_rejected() {
        let github = FakeGithub::dequeue_ready();
        github.pulls.borrow_mut()[0].author = "collaborator".into();
        let error = run_with(
            PromoterEvent::Generate,
            &FakeGit::new(true),
            &github,
            &github,
        )
        .unwrap_err();
        assert!(error.to_string().contains("different identity"));

        assert!(github.writes.borrow().is_empty());
    }

    #[test]
    fn positive_conflict_replaces_after_exact_delete_then_close_even_with_failed_checks() {
        let mut github = conflicted_github();
        github.snapshot_checks[0].state = CheckState::Failure;
        let git = FakeGit::connected(true, &github);

        let outcome = run_with(PromoterEvent::Generate, &git, &github, &github).unwrap();

        assert_eq!(
            outcome,
            PromoterOutcome::Replaced {
                stale: PrNumber(742),
                successor: PrNumber(743),
            }
        );
        assert_eq!(
            *github.trace.borrow(),
            [
                "fetch",
                "main",
                "parent",
                "ancestor",
                "conflicts",
                "delete",
                "close",
                "prepare",
                "promote",
                "diff",
                "format",
                "commit",
                "head",
                "push",
                "create",
                "arm",
            ]
        );
        assert_eq!(*git.fetched.borrow(), ["event-head"]);
        assert_eq!(*git.parent_commits.borrow(), ["event-head"]);
        assert_eq!(
            *git.ancestry.borrow(),
            [("old-main".into(), "main-head".into())]
        );
        assert_eq!(
            *git.merges.borrow(),
            [("main-head".into(), "event-head".into())]
        );
        assert_eq!(*git.deleted.borrow(), ["event-head"]);
        assert!(!github.pulls.borrow()[0].is_open);
        assert!(github.pulls.borrow()[1].auto_merge_armed);
    }

    #[test]
    fn failed_checks_stay_visible_while_pending_checks_resume_arming() {
        let mut failed = FakeGithub::dequeue_ready();
        failed.snapshot_checks[0].state = CheckState::Failure;
        let git = FakeGit::new(true);
        assert_eq!(
            run_with(PromoterEvent::Generate, &git, &failed, &failed).unwrap(),
            PromoterOutcome::Existing(PrNumber(742))
        );
        assert!(git.calls.borrow().is_empty());
        assert!(failed.writes.borrow().is_empty());

        let mut pending = FakeGithub::dequeue_ready();
        pending.snapshot_checks[0].state = CheckState::Pending;
        let git = FakeGit::new(true);
        assert_eq!(
            run_with(PromoterEvent::Generate, &git, &pending, &pending).unwrap(),
            PromoterOutcome::Existing(PrNumber(742))
        );
        assert!(git.calls.borrow().is_empty());
        assert_eq!(*pending.writes.borrow(), ["arm"]);
        assert!(pending.pulls.borrow()[0].auto_merge_armed);
    }

    #[test]
    fn incomplete_conflict_proof_preserves_the_immutable_attempt() {
        for (base, ancestor, conflicts) in [
            ("other-main", true, true),
            ("main-head", false, true),
            ("main-head", true, false),
        ] {
            let mut github = conflicted_github();
            github.snapshot_base = base.into();
            let mut git = FakeGit::connected(true, &github);
            git.ancestor = ancestor;
            git.conflicts = conflicts;

            assert_eq!(
                run_with(PromoterEvent::Generate, &git, &github, &github).unwrap(),
                PromoterOutcome::Existing(PrNumber(742))
            );
            assert!(!git.calls.borrow().contains(&"delete"));
            assert!(github.writes.borrow().is_empty());
            assert!(github.pulls.borrow()[0].is_open);
        }
    }

    #[test]
    fn interrupted_retirement_closes_then_regenerates() {
        let github = conflicted_github();
        github.remote_head.borrow_mut().take();
        let git = FakeGit::connected(true, &github);

        assert_eq!(
            run_with(PromoterEvent::Generate, &git, &github, &github).unwrap(),
            PromoterOutcome::Replaced {
                stale: PrNumber(742),
                successor: PrNumber(743),
            }
        );
        assert_eq!(github.trace.borrow()[0], "close");
        assert!(!github.pulls.borrow()[0].is_open);
        assert!(github.pulls.borrow()[1].auto_merge_armed);
    }

    #[test]
    fn incomplete_publication_is_lease_deleted_then_regenerated() {
        let github = FakeGithub::empty();
        github.remote_head.borrow_mut().replace("orphan".into());
        let git = FakeGit::connected(true, &github);

        assert_eq!(
            run_with(PromoterEvent::Generate, &git, &github, &github).unwrap(),
            PromoterOutcome::Created(PrNumber(742))
        );
        assert_eq!(github.trace.borrow()[0..2], ["delete", "prepare"]);
        assert_eq!(
            github.remote_head.borrow().as_deref(),
            Some("promoted-head")
        );
    }

    #[test]
    fn changed_ref_and_failed_cleanup_postconditions_fail_without_successor_writes() {
        let changed = conflicted_github();
        changed
            .remote_head
            .borrow_mut()
            .replace("different-head".into());
        let git = FakeGit::connected(true, &changed);
        assert!(run_with(PromoterEvent::Generate, &git, &changed, &changed).is_err());
        assert!(git.calls.borrow().is_empty());
        assert!(changed.writes.borrow().is_empty());

        let orphan = FakeGithub::empty();
        orphan.remote_head.borrow_mut().replace("orphan".into());
        let mut git = FakeGit::connected(true, &orphan);
        git.delete_updates = false;
        assert!(run_with(PromoterEvent::Generate, &git, &orphan, &orphan).is_err());
        assert_eq!(*git.calls.borrow(), ["delete"]);
        assert!(orphan.writes.borrow().is_empty());

        let mut unclosed = conflicted_github();
        unclosed.remote_head.borrow_mut().take();
        unclosed.close_updates = false;
        let git = FakeGit::connected(true, &unclosed);
        assert!(run_with(PromoterEvent::Generate, &git, &unclosed, &unclosed).is_err());
        assert_eq!(*unclosed.writes.borrow(), ["close"]);
        assert!(git.calls.borrow().is_empty());
    }

    #[test]
    fn ambiguous_close_uses_postcondition_and_duplicate_run_converges() {
        let mut github = conflicted_github();
        github.remote_head.borrow_mut().take();
        github.close_fails = true;
        let git = FakeGit::connected(true, &github);

        assert!(matches!(
            run_with(PromoterEvent::Generate, &git, &github, &github).unwrap(),
            PromoterOutcome::Replaced { .. }
        ));
        let writes = github.writes.borrow().len();
        github.snapshot_mergeable = Mergeable::Mergeable;
        github.snapshot_status = MergeStateStatus::Clean;

        assert_eq!(
            run_with(PromoterEvent::Generate, &git, &github, &github).unwrap(),
            PromoterOutcome::Existing(PrNumber(743))
        );
        assert_eq!(github.writes.borrow().len(), writes);
    }

    #[test]
    fn ambiguous_push_and_arm_use_exact_postconditions() {
        let mut github = FakeGithub::empty();
        github.arm_fails = true;
        let mut git = FakeGit::connected(true, &github);
        git.fail_on = Some("push");

        assert_eq!(
            run_with(PromoterEvent::Generate, &git, &github, &github).unwrap(),
            PromoterOutcome::Created(PrNumber(742))
        );
        assert_eq!(*github.writes.borrow(), ["create", "arm"]);
        assert_eq!(
            github.remote_head.borrow().as_deref(),
            Some("promoted-head")
        );
        assert!(github.pulls.borrow()[0].auto_merge_armed);
    }

    #[test]
    fn direct_queue_snapshot_stays_existing_without_local_promotion() {
        let github = FakeGithub::dequeue_ready();
        github.pulls.borrow_mut()[0].in_merge_queue = true;
        let git = FakeGit::new(true);
        assert_eq!(
            run_with(PromoterEvent::Generate, &git, &github, &github).unwrap(),
            PromoterOutcome::Existing(PrNumber(742))
        );
        assert!(git.calls.borrow().is_empty());
        assert!(github.writes.borrow().is_empty());
    }

    #[test]
    fn existing_singleton_freezes_its_head_and_skips_commit_or_remote_writes() {
        let git = FakeGit::new(true);
        let github = FakeGithub::dequeue_ready();
        github.pulls.borrow_mut()[0].auto_merge_armed = true;

        let outcome = run_with(PromoterEvent::Generate, &git, &github, &github).unwrap();

        assert_eq!(outcome, PromoterOutcome::Existing(PrNumber(742)));
        assert!(git.calls.borrow().is_empty());
        assert!(github.writes.borrow().is_empty());
        assert_eq!(github.pulls.borrow()[0].head_sha, "event-head");
    }

    #[test]
    fn singleton_ignores_non_occupants_and_reuses_a_marked_occupant() {
        let github = FakeGithub::empty();
        let mut wrong_owner = promoter_pr("one");
        wrong_owner.head_owner = "fork".into();
        let mut wrong_head = promoter_pr("two");
        wrong_head.head_ref = "automation/other".into();
        let mut wrong_base = promoter_pr("three");
        wrong_base.base_ref = "release".into();
        github
            .pulls
            .borrow_mut()
            .extend([wrong_owner, wrong_head, wrong_base]);

        assert!(singleton(&github).unwrap().is_none());
        github.pulls.borrow_mut().push(promoter_pr("exact"));
        assert_eq!(singleton(&github).unwrap().unwrap().head_sha, "exact");
    }

    #[test]
    fn unmarked_occupant_fails_before_commit_or_remote_writes() {
        let git = FakeGit::new(true);
        let github = FakeGithub::empty();
        github
            .remote_head
            .borrow_mut()
            .replace("occupied-head".into());
        let mut occupant = promoter_pr("occupied-head");
        occupant.body = TITLE.into();
        github.pulls.borrow_mut().push(occupant);
        let remote_head = github.remote_head.borrow().clone();
        let pulls = github.pulls.borrow().clone();

        let error = run_with(PromoterEvent::Generate, &git, &github, &github).unwrap_err();

        assert!(error.to_string().contains("different identity"));
        assert!(git.calls.borrow().is_empty());
        assert!(github.writes.borrow().is_empty());
        assert_eq!(*github.remote_head.borrow(), remote_head);
        assert_eq!(*github.pulls.borrow(), pulls);
    }

    #[test]
    fn multiple_occupants_fail_even_when_only_one_has_the_marker() {
        let git = FakeGit::new(true);
        let github = FakeGithub::empty();
        github.pulls.borrow_mut().push(promoter_pr("marked"));
        let mut unmarked = promoter_pr("unmarked");
        unmarked.body.clear();
        github.pulls.borrow_mut().push(unmarked);
        let pulls = github.pulls.borrow().clone();

        let error = run_with(PromoterEvent::Generate, &git, &github, &github).unwrap_err();

        assert_eq!(
            error.to_string(),
            "multiple open pull requests occupy the ADR promoter branch"
        );
        assert!(git.calls.borrow().is_empty());
        assert!(github.writes.borrow().is_empty());
        assert_eq!(*github.pulls.borrow(), pulls);
    }

    #[test]
    fn generation_requires_queue_and_contexts_after_finding_a_diff_before_commit() {
        for required in [
            RequiredChecks {
                queue_present: false,
                ..required()
            },
            RequiredChecks {
                contexts: Vec::new(),
                ..required()
            },
        ] {
            let git = FakeGit::new(true);
            let mut github = FakeGithub::empty();
            github.required = required;

            assert!(run_with(PromoterEvent::Generate, &git, &github, &github).is_err());
            assert_eq!(*git.calls.borrow(), ["prepare", "promote", "diff"]);
            assert!(github.writes.borrow().is_empty());
            assert_eq!(github.required_reads.get(), 1);
        }
    }

    #[test]
    fn no_promotion_diff_succeeds_before_queue_policy_validation() {
        let git = FakeGit::new(false);
        let mut github = FakeGithub::empty();
        github.required.queue_present = false;
        github.required.contexts.clear();

        let outcome = run_with(PromoterEvent::Generate, &git, &github, &github).unwrap();

        assert_eq!(outcome, PromoterOutcome::NoChanges);
        assert_eq!(github.required_reads.get(), 0);
        assert_eq!(*git.calls.borrow(), ["prepare", "promote", "diff"]);
        assert!(github.writes.borrow().is_empty());
    }

    #[test]
    fn promotion_failure_isolated_before_commit_push_or_github_write() {
        let git = FakeGit::failing("promote");
        let github = FakeGithub::empty();

        assert!(run_with(PromoterEvent::Generate, &git, &github, &github).is_err());
        assert_eq!(*git.calls.borrow(), ["prepare", "promote"]);
        assert!(github.writes.borrow().is_empty());
    }

    #[test]
    fn commit_failure_isolated_before_push_or_github_write() {
        let git = FakeGit::failing("commit");
        let github = FakeGithub::empty();

        assert!(run_with(PromoterEvent::Generate, &git, &github, &github).is_err());
        assert_eq!(
            *git.calls.borrow(),
            ["prepare", "promote", "diff", "format", "commit"]
        );
        assert!(github.writes.borrow().is_empty());
    }

    #[test]
    fn remote_head_must_equal_generated_commit_before_pr_creation_or_arm() {
        let github = FakeGithub::empty();
        let mut git = FakeGit::connected(true, &github);
        git.push_head = "different".into();

        assert!(run_with(PromoterEvent::Generate, &git, &github, &github).is_err());
        assert_eq!(
            *git.calls.borrow(),
            [
                "prepare", "promote", "diff", "format", "commit", "head", "push"
            ]
        );
        assert!(github.writes.borrow().is_empty());
    }

    #[test]
    fn new_promotion_arms_and_verifies_auto_merge_on_the_exact_head() {
        let github = FakeGithub::empty();
        let git = FakeGit::connected(true, &github);

        let outcome = run_with(PromoterEvent::Generate, &git, &github, &github).unwrap();

        assert_eq!(outcome, PromoterOutcome::Created(PrNumber(742)));
        assert_eq!(*github.writes.borrow(), ["create", "arm"]);
        let pulls = github.pulls.borrow();
        assert_eq!(pulls[0].head_sha, "promoted-head");
        assert!(pulls[0].auto_merge_armed);
    }

    #[test]
    fn new_promotion_accepts_direct_queue_membership_on_the_exact_head() {
        let mut github = FakeGithub::empty();
        github.arm_to_queue = true;
        let git = FakeGit::connected(true, &github);

        let outcome = run_with(PromoterEvent::Generate, &git, &github, &github).unwrap();

        assert_eq!(outcome, PromoterOutcome::Created(PrNumber(742)));
        let pulls = github.pulls.borrow();
        assert_eq!(pulls[0].head_sha, "promoted-head");
        assert!(!pulls[0].auto_merge_armed);
        assert!(pulls[0].in_merge_queue);
    }

    #[test]
    fn direct_queue_membership_on_a_replaced_head_does_not_verify_creation() {
        let mut github = FakeGithub::empty();
        github.arm_to_queue = true;
        github.head_after_arm = Some("replacement-head".into());
        let git = FakeGit::connected(true, &github);

        assert!(run_with(PromoterEvent::Generate, &git, &github, &github).is_err());
    }

    #[test]
    fn github_event_payload_becomes_typed_exact_dequeue_identity() {
        let payload = json!({
            "action": "dequeued",
            "pull_request": {
                "number": 742,
                "head": {"ref": BRANCH, "sha": "abc123"},
                "base": {"ref": BASE_BRANCH, "sha": "base123"}
            }
        });

        let event = PromoterEvent::from_reader(payload.to_string().as_bytes()).unwrap();

        assert_eq!(
            event,
            PromoterEvent::PullRequest(PullRequestEvent {
                action: "dequeued".into(),
                number: PrNumber(742),
                head_ref: BRANCH.into(),
                head_sha: "abc123".into(),
                base_ref: BASE_BRANCH.into(),
            })
        );
    }

    #[test]
    fn non_pull_request_payload_is_typed_as_generation() {
        let payload = json!({"ref": "refs/heads/main", "after": "abc123"});
        assert_eq!(
            PromoterEvent::from_reader(payload.to_string().as_bytes()).unwrap(),
            PromoterEvent::Generate
        );
    }

    #[test]
    fn non_dequeued_or_wrong_head_base_event_is_filtered_without_reads_or_writes() {
        for event in [
            PullRequestEvent {
                action: "opened".into(),
                ..dequeue_event()
            },
            PullRequestEvent {
                head_ref: "automation/other".into(),
                ..dequeue_event()
            },
            PullRequestEvent {
                base_ref: "release".into(),
                ..dequeue_event()
            },
        ] {
            let github = FakeGithub::empty();
            let outcome = run_with(
                PromoterEvent::PullRequest(event),
                &FakeGit::new(true),
                &github,
                &github,
            )
            .unwrap();
            assert_eq!(outcome, PromoterOutcome::IgnoredEvent);
            assert!(github.writes.borrow().is_empty());
        }
    }

    #[test]
    fn merge_group_candidates_are_exact_to_base_and_pr_and_deduplicate_workflow_jobs() {
        let event = dequeue_event();
        let exact = format!("gh-readonly-queue/{BASE_BRANCH}/pr-742-base-tip");
        let payload = json!({
            "workflow_runs": [
                {"event": "merge_group", "head_branch": exact, "head_sha": "group-a"},
                {"event": "merge_group", "head_branch": exact, "head_sha": "group-a"},
                {"event": "merge_group", "head_branch": exact, "head_sha": "group-b"},
                {"event": "merge_group", "head_branch": "gh-readonly-queue/release/pr-742-base-tip", "head_sha": "ignored"},
                {"event": "merge_group", "head_branch": "gh-readonly-queue/main/pr-741-base-tip", "head_sha": "ignored"},
                {"event": "pull_request", "head_branch": exact, "head_sha": "ignored"}
            ]
        });

        assert_eq!(
            parse_merge_group_candidates(&payload, &event),
            ["group-a".to_string(), "group-b".to_string()]
        );
        assert!(commit_has_parent(
            &json!({"sha": "group-a", "parents": [{"sha": "event-head"}]}),
            "group-a",
            "event-head"
        ));
        assert!(!commit_has_parent(
            &json!({"sha": "group-a", "parents": [{"sha": "old-head"}]}),
            "group-a",
            "event-head"
        ));
    }

    #[test]
    fn one_bounded_green_dequeue_read_rearms_and_verifies_the_unchanged_head() {
        let github = FakeGithub::dequeue_ready();

        let outcome = run_with(
            PromoterEvent::PullRequest(dequeue_event()),
            &FakeGit::new(true),
            &github,
            &github,
        )
        .unwrap();

        assert_eq!(outcome, PromoterOutcome::Rearmed(PrNumber(742)));
        assert_eq!(*github.writes.borrow(), ["arm"]);
        assert_eq!(github.pulls.borrow()[0].head_sha, "event-head");
        assert!(github.pulls.borrow()[0].auto_merge_armed);
    }

    #[test]
    fn green_dequeue_recovery_accepts_direct_queue_membership_on_the_exact_head() {
        let mut github = FakeGithub::dequeue_ready();
        github.arm_to_queue = true;

        let outcome = run_with(
            PromoterEvent::PullRequest(dequeue_event()),
            &FakeGit::new(true),
            &github,
            &github,
        )
        .unwrap();

        assert_eq!(outcome, PromoterOutcome::Rearmed(PrNumber(742)));
        let pulls = github.pulls.borrow();
        assert_eq!(pulls[0].head_sha, "event-head");
        assert!(!pulls[0].auto_merge_armed);
        assert!(pulls[0].in_merge_queue);
    }

    #[test]
    fn direct_queue_membership_on_a_replaced_head_does_not_verify_recovery() {
        let mut github = FakeGithub::dequeue_ready();
        github.arm_to_queue = true;
        github.head_after_arm = Some("replacement-head".into());

        assert!(
            run_with(
                PromoterEvent::PullRequest(dequeue_event()),
                &FakeGit::new(true),
                &github,
                &github,
            )
            .is_err()
        );
    }

    #[test]
    fn absent_or_mismatched_promoter_never_rearms() {
        assert_not_rearmed(&FakeGithub::empty());

        let github = FakeGithub::dequeue_ready();
        github.pulls.borrow_mut()[0].head_sha = "new-head".into();
        assert_not_rearmed(&github);

        let github = FakeGithub::dequeue_ready();
        github.pulls.borrow_mut()[0].body.clear();
        assert_not_rearmed(&github);
    }

    #[test]
    fn absent_or_ambiguous_merge_group_never_rearms() {
        let mut absent = FakeGithub::dequeue_ready();
        absent.merge_groups.clear();
        assert_not_rearmed(&absent);

        let mut ambiguous = FakeGithub::dequeue_ready();
        ambiguous.merge_groups.push("other-group".into());
        assert_not_rearmed(&ambiguous);
    }

    #[test]
    fn absent_or_mismatched_context_evidence_never_rearms() {
        let mut absent_required = FakeGithub::dequeue_ready();
        absent_required.required.contexts.clear();
        assert_not_rearmed(&absent_required);

        let mut absent_queue = FakeGithub::dequeue_ready();
        absent_queue.required.queue_present = false;
        assert_not_rearmed(&absent_queue);

        let mut mismatched_sha = FakeGithub::dequeue_ready();
        mismatched_sha
            .checks
            .insert("event-head".into(), green_checks("other-head"));
        assert_not_rearmed(&mismatched_sha);

        let mut missing_context = FakeGithub::dequeue_ready();
        missing_context
            .checks
            .get_mut("merge-group")
            .unwrap()
            .checks
            .pop();
        assert_not_rearmed(&missing_context);
    }

    #[test]
    fn pending_or_failed_context_on_either_commit_never_rearms() {
        for (sha, state) in [
            ("event-head", CheckState::Pending),
            ("event-head", CheckState::Failure),
            ("merge-group", CheckState::Pending),
            ("merge-group", CheckState::Failure),
        ] {
            let mut github = FakeGithub::dequeue_ready();
            github.checks.get_mut(sha).unwrap().checks[0].state = state;
            assert_not_rearmed(&github);
        }
    }
}
