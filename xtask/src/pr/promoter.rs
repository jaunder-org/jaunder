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
use super::land::{self, GhArmer, PrArmer};
use super::snapshot::{
    self, AppIdentity, CheckState, CommentAuthor, CommitChecks, MergeStateStatus, Mergeable,
    PrComment, PrSnapshot, PrState, RequiredChecks,
};
use super::{PrNumber, Subject};
use crate::{StepResult, adr, git};

pub const BRANCH: &str = "automation/adr-promoter";
pub const TITLE: &str = "docs(adr): promote pending ADR drafts";
pub const MARKER: &str = "<!-- jaunder-adr-promoter -->";
pub const BASE_BRANCH: &str = "main";
const MERGE_GROUP_LIMIT: usize = 100;

pub const PROMOTER_BOT_LOGIN: &str = "jaunder-adr-promoter[bot]";
pub const PROMOTER_VERSION: u8 = 1;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GeneratedProvenance {
    pub version: u8,
    pub base: String,
    pub replaces: Option<(PrNumber, String)>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum IntentKind {
    Retirement,
    PublicationAbort,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PromoterIntent {
    pub kind: IntentKind,
    pub number: PrNumber,
    pub head: String,
    pub parent: String,
    pub main: String,
}

impl PromoterIntent {
    fn body(&self) -> String {
        let kind = match self.kind {
            IntentKind::Retirement => "retirement",
            IntentKind::PublicationAbort => "publication-abort",
        };
        format!(
            "{MARKER}\n<!-- jaunder-adr-promoter-intent:v1 kind={kind} pr={} head={} parent={} main={} -->",
            self.number, self.head, self.parent, self.main
        )
    }
}

fn parse_intent(body: &str) -> Option<PromoterIntent> {
    let prefix = "<!-- jaunder-adr-promoter-intent:v1 ";
    let fields = body
        .strip_prefix(MARKER)?
        .trim()
        .strip_prefix(prefix)?
        .strip_suffix(" -->")?;
    let mut kind = None;
    let mut number = None;
    let mut head = None;
    let mut parent = None;
    let mut main = None;
    for field in fields.split_whitespace() {
        let (name, value) = field.split_once('=')?;
        match name {
            "kind" => {
                kind = match value {
                    "retirement" => Some(IntentKind::Retirement),
                    "publication-abort" => Some(IntentKind::PublicationAbort),
                    _ => return None,
                }
            }
            "pr" => number = value.parse::<u64>().ok().map(PrNumber),
            "head" => head = Some(value.to_string()),
            "parent" => parent = Some(value.to_string()),
            "main" => main = Some(value.to_string()),
            _ => return None,
        }
    }
    let intent = PromoterIntent {
        kind: kind?,
        number: number?,
        head: head?,
        parent: parent?,
        main: main?,
    };
    (intent.body() == body).then_some(intent)
}

fn provenance_message(provenance: &GeneratedProvenance) -> String {
    let mut message = format!(
        "{TITLE}\n\nJaunder-Promoter-Version: {}\nJaunder-Promoter-Base: {}",
        provenance.version, provenance.base
    );
    if let Some((number, head)) = &provenance.replaces {
        message.push_str(&format!("\nJaunder-Promoter-Replaces: {number}@{head}"));
    }
    message
}

fn parse_generated_provenance(message: &str) -> Option<GeneratedProvenance> {
    // `%B` includes Git's terminal newline; no other normalization is accepted.
    let message = message.strip_suffix('\n').unwrap_or(message);
    let (title, trailers) = message.split_once("\n\n")?;
    if title != TITLE {
        return None;
    }
    let mut lines = trailers.lines();
    let version_text = lines.next()?.strip_prefix("Jaunder-Promoter-Version: ")?;
    let version = version_text.parse::<u8>().ok()?;
    if version.to_string() != version_text {
        return None;
    }
    let base = lines
        .next()?
        .strip_prefix("Jaunder-Promoter-Base: ")?
        .to_string();
    if base.is_empty() {
        return None;
    }
    let replaces = match lines.next() {
        None => None,
        Some(line) => {
            let (number_text, head) = line
                .strip_prefix("Jaunder-Promoter-Replaces: ")?
                .split_once('@')?;
            let number = number_text.parse::<u64>().ok().map(PrNumber)?;
            if number.to_string() != number_text || head.is_empty() || lines.next().is_some() {
                return None;
            }
            Some((number, head.to_string()))
        }
    };
    let provenance = GeneratedProvenance {
        version,
        base,
        replaces,
    };
    (provenance_message(&provenance) == message).then_some(provenance)
}

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
    pub author_login: String,
    pub head_owner: String,
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
                write!(
                    f,
                    "replaced stale promoter PR {stale} with successor PR {successor}"
                )
            }
            Self::IgnoredEvent => f.write_str("event does not target the ADR promoter"),
            Self::Rearmed(number) => write!(f, "re-armed promoter PR {number}"),
            Self::NotRearmed(reason) => write!(f, "promoter PR not re-armed: {reason}"),
        }
    }
}

/// Local mutation and publication operations.
pub trait PromoterGit {
    fn prepare_fresh_main(&self) -> Result<()>;
    fn promote(&self) -> Result<()>;
    fn fetch_exact(&self, sha: &str) -> Result<()>;
    fn main_sha(&self) -> Result<Option<String>>;
    fn sole_parent(&self, sha: &str) -> Result<Option<String>>;
    fn is_strict_ancestor(&self, ancestor: &str, descendant: &str) -> Result<bool>;
    fn merge_conflicts(&self, main: &str, head: &str) -> Result<bool>;
    fn delete_branch_with_lease(&self, expected: &str) -> Result<()>;
    fn discard_unpublished_candidate(&self, base: &str) -> Result<()>;
    fn verify_generated_tree(&self, head: &str, provenance: &GeneratedProvenance) -> Result<bool>;
    fn generated_provenance(&self, head: &str) -> Result<Option<GeneratedProvenance>>;
    fn has_staged_diff(&self) -> Result<bool>;
    fn format_staged_markdown(&self) -> Result<()>;
    fn commit(&self) -> Result<()>;
    fn head_sha(&self) -> Result<String>;
    fn commit_with_provenance(&self, provenance: &GeneratedProvenance) -> Result<()>;
    fn push(&self) -> Result<()>;
}

/// GitHub reads used to establish durable promoter identity and dequeue evidence.
pub trait PromoterPrRead {
    fn repository(&self) -> Result<(String, String)>;
    fn open_pull_requests(&self) -> Result<Vec<PromoterPullRequest>>;
    fn all_pull_requests(&self) -> Result<Vec<PromoterPullRequest>>;
    fn snapshot(&self, number: PrNumber) -> Result<Option<PrSnapshot>>;
    fn app_identity(&self) -> Result<Option<AppIdentity>>;
    fn comments(&self, number: PrNumber) -> Result<Vec<PrComment>>;
    fn pull_request(&self, number: PrNumber) -> Result<Option<PromoterPullRequest>>;
    fn remote_branch_head(&self) -> Result<Option<String>>;
    fn required_checks(&self, base: &str) -> Result<RequiredChecks>;
    fn merge_group_shas(&self, event: &PullRequestEvent) -> Result<Vec<String>>;
    fn commit_checks(&self, sha: &str) -> Result<CommitChecks>;
}

/// The only GitHub writes the promoter can perform.
pub trait PromoterPrWrite {
    fn create_pull_request(&self) -> Result<()>;
    fn arm_auto_merge(&self, number: PrNumber) -> Result<()>;
    fn append_intent(&self, number: PrNumber, intent: &PromoterIntent) -> Result<()>;
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

    fn commit_with_provenance(&self, provenance: &GeneratedProvenance) -> Result<()> {
        if provenance.version != PROMOTER_VERSION {
            bail!(
                "unsupported promoter generator version {}",
                provenance.version
            );
        }
        let message = provenance_message(provenance);
        let args = [
            "-c",
            "user.name=jaunder-adr-promoter[bot]",
            "-c",
            "user.email=jaunder-adr-promoter[bot]@users.noreply.github.com",
            "commit",
            "--no-verify",
            "-m",
            &message,
        ];
        git::run(&self.repo, &args)
    }

    fn head_sha(&self) -> Result<String> {
        git::head_sha(&self.repo)?.ok_or_else(|| anyhow!("promoter commit has no HEAD"))
    }

    fn push(&self) -> Result<()> {
        let destination = format!("HEAD:refs/heads/{BRANCH}");
        git::run(&self.repo, &["push", "origin", &destination])
    }

    fn generated_provenance(&self, head: &str) -> Result<Option<GeneratedProvenance>> {
        let message = git::output(&self.repo, &["show", "-s", "--format=%B", head])?;
        Ok(parse_generated_provenance(&message))
    }

    fn verify_generated_tree(&self, head: &str, provenance: &GeneratedProvenance) -> Result<bool> {
        if provenance.version != PROMOTER_VERSION
            || self.sole_parent(head)?.as_deref() != Some(provenance.base.as_str())
        {
            return Ok(false);
        }
        let candidate_tree = git::output(&self.repo, &["rev-parse", &format!("{head}^{{tree}}")])?;
        let worktree = tempfile::tempdir().context("creating promoter verification worktree")?;
        let path = worktree.path().to_path_buf();
        git::run(
            &self.repo,
            &[
                "worktree",
                "add",
                "--detach",
                path.to_str().unwrap_or_default(),
                &provenance.base,
            ],
        )?;
        let rebuilt = (|| {
            adr::run_promote(&path)?;
            if !git::output(&path, &["diff", "--cached", "--name-only"])?.is_empty() {
                Self::format_staged_markdown_at(&path)?;
            }
            git::output(&path, &["write-tree"])
        })();
        let removed = git::run(
            &self.repo,
            &[
                "worktree",
                "remove",
                "--force",
                path.to_str().unwrap_or_default(),
            ],
        );
        let rebuilt = rebuilt?;
        removed?;
        Ok(rebuilt == candidate_tree)
    }

    fn fetch_exact(&self, sha: &str) -> Result<()> {
        git::run(&self.repo, &["fetch", "origin", sha])
    }

    fn main_sha(&self) -> Result<Option<String>> {
        git::run(&self.repo, &["fetch", "origin", BASE_BRANCH])?;
        git::output(&self.repo, &["rev-parse", &format!("origin/{BASE_BRANCH}")]).map(Some)
    }

    fn sole_parent(&self, sha: &str) -> Result<Option<String>> {
        let parents = git::output(&self.repo, &["show", "-s", "--format=%P", sha])?;
        let mut parents = parents.split_whitespace();
        let parent = parents.next().map(str::to_owned);
        if parents.next().is_some() {
            return Ok(None);
        }
        Ok(parent)
    }

    fn is_strict_ancestor(&self, ancestor: &str, descendant: &str) -> Result<bool> {
        let status = git::at(&self.repo)
            .args(["merge-base", "--is-ancestor", ancestor, descendant])
            .status()
            .context("checking promoter ancestry")?;
        match status.code() {
            Some(0) if ancestor != descendant => Ok(true),
            Some(1) => Ok(false),
            _ => bail!("git merge-base failed ({status})"),
        }
    }

    fn merge_conflicts(&self, main: &str, head: &str) -> Result<bool> {
        git::merge_tree_conflicts(&self.repo, main, head)
    }

    fn delete_branch_with_lease(&self, expected: &str) -> Result<()> {
        git::delete_remote_with_lease(&self.repo, "origin", BRANCH, expected)
    }

    fn discard_unpublished_candidate(&self, base: &str) -> Result<()> {
        git::run(&self.repo, &["reset", "--hard", base])
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
        let head_owner = value
            .get("headRepositoryOwner")
            .and_then(|owner| owner.get("login"))
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string();
        let author_login = value
            .get("author")
            .and_then(|author| author.get("login"))
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string();
        Ok(PromoterPullRequest {
            number: PrNumber(number),
            author_login,
            head_owner,
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
            // Queue state is not available from `gh pr ... --json`; exact-head
            // verification enriches a single PR from the shared GraphQL snapshot.
            in_merge_queue: false,
        })
    }

    fn pr_fields() -> &'static str {
        "number,state,author,headRepositoryOwner,headRefName,headRefOid,baseRefName,body,autoMergeRequest"
    }

    fn pull_requests_with(
        &self,
        state: &str,
        run: impl FnOnce(&[&str]) -> std::result::Result<Value, gh::ApiError>,
    ) -> Result<Vec<PromoterPullRequest>> {
        let slug = self.slug();
        // `gh pr list --head` accepts only a branch name. Repository-owner
        // identity remains enforced from the parsed `headRepositoryOwner`.
        let value = run(&[
            "pr",
            "list",
            "--repo",
            &slug,
            "--state",
            state,
            "--head",
            BRANCH,
            "--base",
            BASE_BRANCH,
            "--json",
            Self::pr_fields(),
        ])
        .map_err(github_error)?;
        value
            .as_array()
            .ok_or_else(|| anyhow!("promoter PR list is not an array"))?
            .iter()
            .map(|pr| self.parse_pull_request(pr))
            .collect()
    }

    fn open_pull_requests_with(
        &self,
        run: impl FnOnce(&[&str]) -> std::result::Result<Value, gh::ApiError>,
    ) -> Result<Vec<PromoterPullRequest>> {
        self.pull_requests_with("open", run)
    }

    fn pull_request_and_snapshot_with(
        &self,
        number: PrNumber,
        run_pr: impl FnOnce(&[&str]) -> std::result::Result<Value, gh::ApiError>,
        run_snapshot: impl FnOnce(&[&str]) -> std::result::Result<Value, gh::ApiError>,
    ) -> Result<Option<(PromoterPullRequest, PrSnapshot)>> {
        let slug = self.slug();
        let number_text = number.to_string();
        let value = match run_pr(&[
            "pr",
            "view",
            &number_text,
            "--repo",
            &slug,
            "--json",
            Self::pr_fields(),
        ]) {
            Ok(value) => value,
            Err(gh::ApiError::NotFound) => return Ok(None),
            Err(error) => return Err(github_error(error)),
        };
        let mut pr = self.parse_pull_request(&value)?;

        let query = format!("query={}", snapshot::PR_QUERY);
        let owner = format!("owner={}", self.owner);
        let name = format!("name={}", self.repo);
        let number_arg = format!("number={number}");
        let snapshot = snapshot::parse_snapshot(
            &run_snapshot(&[
                "api",
                "graphql",
                "-f",
                &query,
                "-f",
                &owner,
                "-f",
                &name,
                "-F",
                &number_arg,
            ])
            .map_err(github_error)?,
        )
        .map_err(github_error)?;
        if snapshot.head_sha != pr.head_sha {
            bail!("promoter PR GraphQL snapshot names a different head");
        }
        pr.auto_merge_armed = snapshot.auto_merge_armed;
        pr.in_merge_queue = snapshot.queue.in_queue;
        Ok(Some((pr, snapshot)))
    }

    fn pull_request_with(
        &self,
        number: PrNumber,
        run_pr: impl FnOnce(&[&str]) -> std::result::Result<Value, gh::ApiError>,
        run_snapshot: impl FnOnce(&[&str]) -> std::result::Result<Value, gh::ApiError>,
    ) -> Result<Option<PromoterPullRequest>> {
        self.pull_request_and_snapshot_with(number, run_pr, run_snapshot)
            .map(|result| result.map(|(pr, _)| pr))
    }
}

impl PromoterPrRead for GhPromoterPr {
    fn repository(&self) -> Result<(String, String)> {
        Ok((self.owner.clone(), self.repo.clone()))
    }

    fn open_pull_requests(&self) -> Result<Vec<PromoterPullRequest>> {
        self.open_pull_requests_with(gh::run_gh)
    }

    fn all_pull_requests(&self) -> Result<Vec<PromoterPullRequest>> {
        self.pull_requests_with("all", gh::run_gh)
    }

    fn pull_request(&self, number: PrNumber) -> Result<Option<PromoterPullRequest>> {
        self.pull_request_with(number, gh::run_gh, gh::run_gh)
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
        snapshot::parse_required_checks(&value).map_err(github_error)
    }

    // This historical workflow-run lookup is the sole consumer of the App's
    // Actions-read permission; commit-parent correlation remains the
    // authorization check for dequeue recovery.
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
        let query = format!("query={}", snapshot::COMMIT_CHECKS_QUERY);
        let owner = format!("owner={}", self.owner);
        let name = format!("name={}", self.repo);
        let oid = format!("oid={sha}");
        let value = gh::run_gh(&[
            "api", "graphql", "-f", &query, "-f", &owner, "-f", &name, "-f", &oid,
        ])
        .map_err(github_error)?;
        snapshot::parse_commit_checks(&value).map_err(github_error)
    }
    fn snapshot(&self, number: PrNumber) -> Result<Option<PrSnapshot>> {
        self.pull_request_and_snapshot_with(number, gh::run_gh, gh::run_gh)
            .map(|result| result.map(|(_, snapshot)| snapshot))
    }

    fn app_identity(&self) -> Result<Option<AppIdentity>> {
        let Some(client_id) = std::env::var_os("ADR_PROMOTER_APP_CLIENT_ID") else {
            return Ok(None);
        };
        let client_id = client_id
            .into_string()
            .map_err(|_| anyhow!("ADR_PROMOTER_APP_CLIENT_ID is not UTF-8"))?;
        if client_id.is_empty() {
            bail!("ADR_PROMOTER_APP_CLIENT_ID is empty");
        }
        Ok(Some(AppIdentity {
            login: PROMOTER_BOT_LOGIN.to_string(),
            client_id,
        }))
    }

    fn comments(&self, number: PrNumber) -> Result<Vec<PrComment>> {
        let path = format!(
            "/repos/{}/{}/issues/{number}/comments",
            self.owner, self.repo
        );
        let value = gh::run_gh(&["api", "--paginate", "--slurp", &path]).map_err(github_error)?;
        parse_comment_pages(&value)
    }
}
fn parse_comment_pages(value: &Value) -> Result<Vec<PrComment>> {
    let pages = value
        .as_array()
        .ok_or_else(|| anyhow!("promoter comment pages response is not an array"))?;
    let comments = pages
        .iter()
        .map(Value::as_array)
        .collect::<Option<Vec<_>>>()
        .ok_or_else(|| anyhow!("promoter comment page is not an array"))?;
    comments
        .into_iter()
        .flatten()
        .map(|comment| {
            let body = comment
                .get("body")
                .and_then(Value::as_str)
                .ok_or_else(|| anyhow!("promoter comment has no body"))?
                .to_string();
            let login = comment
                .get("user")
                .and_then(|user| user.get("login"))
                .and_then(Value::as_str)
                .ok_or_else(|| anyhow!("promoter comment has no author"))?
                .to_string();
            let app_client_id = comment
                .get("performed_via_github_app")
                .and_then(|app| app.get("client_id"))
                .and_then(Value::as_str)
                .map(str::to_string);
            Ok(PrComment {
                body,
                author: CommentAuthor {
                    login,
                    app_client_id,
                },
            })
        })
        .collect()
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

    fn append_intent(&self, number: PrNumber, intent: &PromoterIntent) -> Result<()> {
        let path = format!(
            "/repos/{}/{}/issues/{number}/comments",
            self.owner, self.repo
        );
        gh::run_gh(&[
            "api",
            "--method",
            "POST",
            &path,
            "-f",
            &format!("body={}", intent.body()),
        ])
        .map_err(github_error)?;
        Ok(())
    }

    fn close_pull_request(&self, number: PrNumber) -> Result<()> {
        let path = format!("/repos/{}/{}/pulls/{number}", self.owner, self.repo);
        gh::run_gh(&["api", "--method", "PATCH", &path, "-f", "state=closed"])
            .map_err(github_error)?;
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
    is_promoter_occupant(pr, owner)
        && pr.author_login == PROMOTER_BOT_LOGIN
        && pr.body.contains(MARKER)
}

fn singleton<R: PromoterPrRead>(read: &R) -> Result<Option<PromoterPullRequest>> {
    let (owner, _) = read.repository()?;
    let occupants = read
        .open_pull_requests()?
        .into_iter()
        .filter(|pr| is_promoter_occupant(pr, &owner))
        .collect::<Vec<_>>();
    match occupants.as_slice() {
        [] => Ok(None),
        [pr] if is_promoter_identity(pr, &owner) => Ok(Some(pr.clone())),
        [pr] => bail!(
            "open pull request #{} occupies the ADR promoter branch without the exact promoter identity",
            pr.number
        ),
        _ => bail!("multiple open pull requests occupy the ADR promoter branch"),
    }
}

fn exact_intent(comment: &PrComment, identity: &AppIdentity, intent: &PromoterIntent) -> bool {
    comment.author.login == identity.login
        && comment.author.app_client_id.as_deref() == Some(identity.client_id.as_str())
        && parse_intent(&comment.body).as_ref() == Some(intent)
        && comment.body == intent.body()
}

fn contexts_are_green(required: &RequiredChecks, checks: &CommitChecks) -> bool {
    !required.contexts.is_empty()
        && required.contexts.iter().all(|name| {
            decide::resolve_context(&checks.checks, name)
                .is_some_and(|check| check.state == CheckState::Success)
        })
}

fn required_contexts_failed(required: &RequiredChecks, checks: &CommitChecks) -> bool {
    required.contexts.iter().any(|name| {
        decide::resolve_context(&checks.checks, name)
            .is_some_and(|check| check.state == CheckState::Failure)
    })
}

fn required_checks_allow_arm<R: PromoterPrRead>(
    read: &R,
    candidate: &PromoterPullRequest,
) -> Result<bool> {
    let required = read.required_checks(BASE_BRANCH)?;
    if !required.queue_present || required.contexts.is_empty() {
        bail!("ADR promoter requires a merge queue with required contexts");
    }
    let checks = read.commit_checks(&candidate.head_sha)?;
    if checks.sha != candidate.head_sha {
        bail!("promoter required-check evidence names a different head");
    }
    Ok(!required_contexts_failed(&required, &checks))
}

fn arm_candidate_after_checks<R: PromoterPrRead, W: PromoterPrWrite>(
    read: &R,
    write: &W,
    candidate: &PromoterPullRequest,
) -> Result<()> {
    write.arm_auto_merge(candidate.number)?;
    let verified = read
        .pull_request(candidate.number)?
        .ok_or_else(|| anyhow!("promoter PR disappeared after auto-merge arm"))?;
    let (owner, _) = read.repository()?;
    if verified.head_sha != candidate.head_sha
        || !is_promoter_identity(&verified, &owner)
        || !land::arm_is_verified(verified.auto_merge_armed, verified.in_merge_queue)
    {
        bail!("GitHub did not verify auto-merge on the unchanged promoter head");
    }
    Ok(())
}

fn arm_candidate<R: PromoterPrRead, W: PromoterPrWrite>(
    read: &R,
    write: &W,
    candidate: &PromoterPullRequest,
) -> Result<bool> {
    if !required_checks_allow_arm(read, candidate)? {
        return Ok(false);
    }
    arm_candidate_after_checks(read, write, candidate)?;
    Ok(true)
}

fn publication_outcome(
    replaces: Option<&(PrNumber, String)>,
    successor: PrNumber,
) -> PromoterOutcome {
    match replaces {
        Some((stale, _)) => PromoterOutcome::Replaced {
            stale: *stale,
            successor,
        },
        None => PromoterOutcome::Created(successor),
    }
}

fn durable_intent<R: PromoterPrRead>(
    read: &R,
    number: PrNumber,
    kind: IntentKind,
) -> Result<Option<PromoterIntent>> {
    let Some(identity) = read.app_identity()? else {
        return Ok(None);
    };
    let intents = read
        .comments(number)?
        .iter()
        .filter(|comment| {
            comment.author.login == identity.login
                && comment.author.app_client_id.as_deref() == Some(identity.client_id.as_str())
        })
        .filter_map(|comment| parse_intent(&comment.body))
        .filter(|intent| intent.number == number && intent.kind == kind)
        .collect::<Vec<_>>();
    match intents.as_slice() {
        [] => Ok(None),
        [intent] => Ok(Some(intent.clone())),
        [first, rest @ ..] if rest.iter().all(|intent| intent == first) => Ok(Some(first.clone())),
        _ => bail!("conflicting durable promoter intents authorize the same transition"),
    }
}

fn delete_and_close_exact<G: PromoterGit, R: PromoterPrRead, W: PromoterPrWrite>(
    git: &G,
    read: &R,
    write: &W,
    pr: &PromoterPullRequest,
    expected_head: &str,
) -> Result<()> {
    match read.remote_branch_head()? {
        Some(head) if head == expected_head => {
            git.delete_branch_with_lease(expected_head)?;
            if read.remote_branch_head()?.is_some() {
                bail!("leased promoter branch deletion was not observed");
            }
        }
        None => {}
        Some(_) => bail!("promoter branch changed before exact close"),
    }
    let current = read
        .pull_request(pr.number)?
        .ok_or_else(|| anyhow!("promoter PR disappeared before exact close"))?;
    let (owner, _) = read.repository()?;
    if current.head_sha != expected_head
        || (current.is_open && !is_promoter_identity(&current, &owner))
    {
        bail!("promoter identity changed before exact close");
    }
    if current.is_open {
        write.close_pull_request(pr.number)?;
    }
    let closed = read
        .pull_request(pr.number)?
        .ok_or_else(|| anyhow!("promoter PR disappeared after exact close"))?;
    if closed.is_open || closed.head_sha != expected_head {
        bail!("promoter close was not verified at the exact head");
    }
    Ok(())
}

fn retirement_proof_holds<G: PromoterGit>(git: &G, intent: &PromoterIntent) -> Result<bool> {
    git.fetch_exact(&intent.head)?;
    git.fetch_exact(&intent.parent)?;
    git.fetch_exact(&intent.main)?;
    Ok(
        git.sole_parent(&intent.head)?.as_deref() == Some(intent.parent.as_str())
            && git.is_strict_ancestor(&intent.parent, &intent.main)?
            && git.merge_conflicts(&intent.main, &intent.head)?,
    )
}

fn replay_retirement<G: PromoterGit, R: PromoterPrRead, W: PromoterPrWrite>(
    git: &G,
    read: &R,
    write: &W,
    pr: &PromoterPullRequest,
    intent: &PromoterIntent,
) -> Result<()> {
    if intent.kind != IntentKind::Retirement
        || intent.number != pr.number
        || intent.head != pr.head_sha
    {
        bail!("retirement intent does not name the current promoter attempt");
    }
    if !retirement_proof_holds(git, intent)? {
        bail!("durable retirement intent no longer reproduces its exact conflict proof");
    }
    delete_and_close_exact(git, read, write, pr, &intent.head)
}

fn replay_publication_abort<G: PromoterGit, R: PromoterPrRead, W: PromoterPrWrite>(
    git: &G,
    read: &R,
    write: &W,
    pr: &PromoterPullRequest,
    intent: &PromoterIntent,
) -> Result<GeneratedProvenance> {
    if intent.kind != IntentKind::PublicationAbort
        || intent.number != pr.number
        || intent.head != pr.head_sha
    {
        bail!("publication-abort intent does not name the current promoter candidate");
    }
    git.fetch_exact(&intent.head)?;
    git.fetch_exact(&intent.parent)?;
    git.fetch_exact(&intent.main)?;
    let provenance = git
        .generated_provenance(&intent.head)?
        .ok_or_else(|| anyhow!("publication-abort candidate lacks canonical provenance"))?;
    if provenance.version != PROMOTER_VERSION
        || provenance.base != intent.parent
        || !git.verify_generated_tree(&intent.head, &provenance)?
    {
        bail!("publication-abort candidate does not match deterministic generation");
    }
    if git.sole_parent(&intent.head)?.as_deref() != Some(intent.parent.as_str())
        || !git.is_strict_ancestor(&intent.parent, &intent.main)?
    {
        bail!("durable publication-abort intent no longer reproduces base staleness");
    }
    delete_and_close_exact(git, read, write, pr, &intent.head)?;
    Ok(provenance)
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

fn positive_conflict<G: PromoterGit, R: PromoterPrRead>(
    git: &G,
    read: &R,
    pr: &PromoterPullRequest,
) -> Result<Option<PromoterIntent>> {
    let Some(main) = git.main_sha()? else {
        return Ok(None);
    };
    let Some(snapshot) = read.snapshot(pr.number)? else {
        return Ok(None);
    };
    if snapshot.state != PrState::Open
        || snapshot.head_sha != pr.head_sha
        || snapshot.base_sha != main
        || snapshot.mergeable != Mergeable::Conflicting
        || snapshot.merge_state_status != MergeStateStatus::Dirty
    {
        return Ok(None);
    }
    git.fetch_exact(&pr.head_sha)?;
    let Some(parent) = git.sole_parent(&pr.head_sha)? else {
        return Ok(None);
    };
    if !git.is_strict_ancestor(&parent, &main)? || !git.merge_conflicts(&main, &pr.head_sha)? {
        return Ok(None);
    }
    Ok(Some(PromoterIntent {
        kind: IntentKind::Retirement,
        number: pr.number,
        head: pr.head_sha.clone(),
        parent,
        main,
    }))
}

/// Linearize retirement at the branch, not at GitHub's unleased close endpoint.
/// Every reread precedes its destructive successor, so a changed or ambiguous
/// object is reported rather than adopted.
fn retire<G: PromoterGit, R: PromoterPrRead, W: PromoterPrWrite>(
    git: &G,
    read: &R,
    write: &W,
    stale: &PromoterPullRequest,
    intent: &PromoterIntent,
) -> Result<()> {
    let identity = read
        .app_identity()?
        .ok_or_else(|| anyhow!("promoter App identity is unavailable"))?;
    write.append_intent(stale.number, intent)?;
    if !read
        .comments(stale.number)?
        .iter()
        .any(|comment| exact_intent(comment, &identity, intent))
    {
        bail!("retirement intent was not durably recorded by the promoter App");
    }
    let current = read
        .pull_request(stale.number)?
        .ok_or_else(|| anyhow!("stale promoter disappeared"))?;
    if !is_promoter_identity(&current, &read.repository()?.0) || current.head_sha != intent.head {
        bail!("promoter identity changed before leased retirement");
    }
    if positive_conflict(git, read, &current)?.as_ref() != Some(intent) {
        bail!("retirement evidence changed before branch mutation");
    }
    delete_and_close_exact(git, read, write, stale, &intent.head)
}

fn abort_unarmed<G: PromoterGit, R: PromoterPrRead, W: PromoterPrWrite>(
    git: &G,
    read: &R,
    write: &W,
    pr: &PromoterPullRequest,
    parent: String,
    main: String,
) -> Result<()> {
    let intent = PromoterIntent {
        kind: IntentKind::PublicationAbort,
        number: pr.number,
        head: pr.head_sha.clone(),
        parent,
        main,
    };
    let identity = read
        .app_identity()?
        .ok_or_else(|| anyhow!("promoter App identity is unavailable"))?;
    write.append_intent(pr.number, &intent)?;
    if !read
        .comments(pr.number)?
        .iter()
        .any(|comment| exact_intent(comment, &identity, &intent))
    {
        bail!("publication-abort intent was not durably recorded by the promoter App");
    }
    if git.main_sha()?.as_deref() != Some(intent.main.as_str())
        || git.sole_parent(&intent.head)?.as_deref() != Some(intent.parent.as_str())
        || !git.is_strict_ancestor(&intent.parent, &intent.main)?
    {
        bail!("publication-abort evidence changed before branch mutation");
    }
    delete_and_close_exact(git, read, write, pr, &intent.head)
}
fn validated_orphan_successor<G: PromoterGit>(
    git: &G,
    closed: &PromoterPullRequest,
    intent: &PromoterIntent,
    head: &str,
) -> Result<GeneratedProvenance> {
    if intent.kind != IntentKind::Retirement
        || intent.number != closed.number
        || intent.head != closed.head_sha
    {
        bail!("closed promoter does not carry a matching retirement intent");
    }
    git.fetch_exact(head)?;
    if !retirement_proof_holds(git, intent)? {
        bail!("orphan predecessor does not reproduce its retirement conflict proof");
    }
    let provenance = git
        .generated_provenance(head)?
        .ok_or_else(|| anyhow!("orphan promoter branch lacks canonical generated provenance"))?;
    if provenance.version != PROMOTER_VERSION {
        bail!(
            "unsupported orphan promoter generator version {}",
            provenance.version
        );
    }
    if provenance.replaces.as_ref() != Some(&(closed.number, closed.head_sha.clone()))
        || git.sole_parent(head)?.as_deref() != Some(provenance.base.as_str())
        || !git.verify_generated_tree(head, &provenance)?
    {
        bail!("orphan promoter branch does not reconstruct the exact retired promotion");
    }
    Ok(provenance)
}

fn publish_existing_candidate<G: PromoterGit, R: PromoterPrRead, W: PromoterPrWrite>(
    git: &G,
    read: &R,
    write: &W,
    head: &str,
    provenance: &GeneratedProvenance,
) -> Result<Option<PromoterOutcome>> {
    if read.remote_branch_head()?.as_deref() != Some(head) {
        bail!("orphan promoter branch changed before publication");
    }
    if git.main_sha()?.as_deref() != Some(provenance.base.as_str()) {
        git.delete_branch_with_lease(head)?;
        if read.remote_branch_head()?.is_some() {
            bail!("stale orphan promoter branch deletion was not observed");
        }
        return Ok(None);
    }
    write.create_pull_request()?;
    let created = singleton(read)?.ok_or_else(|| anyhow!("adopted promoter PR was not found"))?;
    if created.head_sha != head {
        bail!("adopted promoter PR head does not equal the orphan commit");
    }
    let main = git
        .main_sha()?
        .ok_or_else(|| anyhow!("current main is unavailable"))?;
    if main != provenance.base {
        abort_unarmed(git, read, write, &created, provenance.base.clone(), main)?;
        return Ok(None);
    }
    if !arm_candidate(read, write, &created)? {
        return Ok(Some(PromoterOutcome::Existing(created.number)));
    }
    Ok(Some(publication_outcome(
        provenance.replaces.as_ref(),
        created.number,
    )))
}

fn is_durable_promoter_attempt(pr: &PromoterPullRequest, owner: &str) -> bool {
    pr.author_login == PROMOTER_BOT_LOGIN
        && pr.head_owner == owner
        && pr.head_ref == BRANCH
        && pr.base_ref == BASE_BRANCH
        && pr.body.contains(MARKER)
}

fn latest_closed_attempt<R: PromoterPrRead>(read: &R) -> Result<Option<PromoterPullRequest>> {
    let (owner, _) = read.repository()?;
    let latest = read.all_pull_requests()?.into_iter().find(|pr| {
        pr.head_owner == owner && pr.head_ref == BRANCH && pr.base_ref == BASE_BRANCH && !pr.is_open
    });
    match latest {
        None => Ok(None),
        Some(pr) if is_durable_promoter_attempt(&pr, &owner) => Ok(Some(pr)),
        Some(pr) => bail!(
            "latest closed pull request #{} on the promoter branch lacks the exact promoter identity",
            pr.number
        ),
    }
}

fn generate<G: PromoterGit, R: PromoterPrRead, W: PromoterPrWrite>(
    git: &G,
    read: &R,
    write: &W,
) -> Result<PromoterOutcome> {
    let replaces = if let Some(existing) = singleton(read)? {
        if let Some(intent) = durable_intent(read, existing.number, IntentKind::Retirement)? {
            let replaces = Some((existing.number, existing.head_sha.clone()));
            replay_retirement(git, read, write, &existing, &intent)?;
            replaces
        } else if let Some(intent) =
            durable_intent(read, existing.number, IntentKind::PublicationAbort)?
        {
            replay_publication_abort(git, read, write, &existing, &intent)?.replaces
        } else if read.remote_branch_head()?.as_deref() != Some(existing.head_sha.as_str()) {
            bail!("open promoter PR and stable branch do not name the same exact head");
        } else if let Some(intent) = positive_conflict(git, read, &existing)? {
            let replaces = Some((existing.number, existing.head_sha.clone()));
            retire(git, read, write, &existing, &intent)?;
            replaces
        } else if !existing.auto_merge_armed
            && !existing.in_merge_queue
            && git.main_sha()?.is_some()
        {
            let provenance = git
                .generated_provenance(&existing.head_sha)?
                .ok_or_else(|| {
                    anyhow!("unarmed promoter candidate lacks canonical generated provenance")
                })?;
            if provenance.version != PROMOTER_VERSION
                || git.sole_parent(&existing.head_sha)?.as_deref() != Some(provenance.base.as_str())
                || !git.verify_generated_tree(&existing.head_sha, &provenance)?
            {
                bail!("unarmed promoter candidate does not match deterministic generation");
            }
            if !required_checks_allow_arm(read, &existing)? {
                return Ok(PromoterOutcome::Existing(existing.number));
            }
            let main = git
                .main_sha()?
                .ok_or_else(|| anyhow!("current main is unavailable"))?;
            if main != provenance.base {
                abort_unarmed(git, read, write, &existing, provenance.base, main)?;
                return generate(git, read, write);
            }
            arm_candidate_after_checks(read, write, &existing)?;
            return Ok(publication_outcome(
                provenance.replaces.as_ref(),
                existing.number,
            ));
        } else {
            return Ok(PromoterOutcome::Existing(existing.number));
        }
    } else if let Some(closed) = latest_closed_attempt(read)? {
        if let Some(intent) = durable_intent(read, closed.number, IntentKind::Retirement)? {
            let replaces = Some((closed.number, closed.head_sha.clone()));
            if let Some(head) = read.remote_branch_head()?
                && head != closed.head_sha
            {
                let provenance = validated_orphan_successor(git, &closed, &intent, &head)?;
                if let Some(outcome) =
                    publish_existing_candidate(git, read, write, &head, &provenance)?
                {
                    return Ok(outcome);
                }
                return generate(git, read, write);
            }
            replay_retirement(git, read, write, &closed, &intent)?;
            replaces
        } else if let Some(intent) =
            durable_intent(read, closed.number, IntentKind::PublicationAbort)?
        {
            replay_publication_abort(git, read, write, &closed, &intent)?.replaces
        } else if read
            .snapshot(closed.number)?
            .is_some_and(|snapshot| snapshot.state == PrState::Merged)
        {
            match read.remote_branch_head()? {
                None => None,
                Some(head) if head == closed.head_sha => {
                    git.delete_branch_with_lease(&closed.head_sha)?;
                    if read.remote_branch_head()?.is_some() {
                        bail!("merged promoter branch cleanup was not observed");
                    }
                    None
                }
                Some(_) => bail!("stable branch does not name the latest merged promoter head"),
            }
        } else {
            bail!("latest closed promoter attempt has no durable controller intent");
        }
    } else {
        if read.remote_branch_head()?.is_some() {
            bail!("stable promoter branch exists without a validated latest promoter attempt");
        }
        None
    };
    for attempt in 0..3 {
        git.prepare_fresh_main()?;
        let base = git.main_sha()?;
        git.promote()?;
        if !git.has_staged_diff()? {
            return Ok(PromoterOutcome::NoChanges);
        }
        if let Some(existing) = singleton(read)? {
            return Ok(PromoterOutcome::Existing(existing.number));
        }
        let queue = read.required_checks(BASE_BRANCH)?;
        if !queue.queue_present || queue.contexts.is_empty() {
            bail!("ADR promoter requires a merge queue with required contexts");
        }
        git.format_staged_markdown()?;
        let provenance = base.as_ref().map(|base| GeneratedProvenance {
            version: PROMOTER_VERSION,
            base: base.clone(),
            replaces: replaces.clone(),
        });
        if let Some(provenance) = &provenance {
            if git.main_sha()?.as_deref() != Some(provenance.base.as_str()) {
                git.discard_unpublished_candidate(&provenance.base)?;
                if attempt == 2 {
                    bail!(
                        "main advanced before promoter branch publication on every bounded regeneration attempt"
                    );
                }
                continue;
            }
            git.commit_with_provenance(provenance)?;
        } else {
            git.commit()?;
        }
        let armed_sha = git.head_sha()?;
        if let Some(provenance) = &provenance {
            let current_main = git.main_sha()?;
            let committed = git.generated_provenance(&armed_sha)?;
            if committed.as_ref() != Some(provenance)
                || git.sole_parent(&armed_sha)?.as_deref() != Some(provenance.base.as_str())
            {
                git.discard_unpublished_candidate(&provenance.base)?;
                bail!("generated promoter commit lacks its exact canonical base provenance");
            }
            if current_main.as_deref() != Some(provenance.base.as_str()) {
                git.discard_unpublished_candidate(&provenance.base)?;
                if attempt == 2 {
                    bail!(
                        "main advanced before promoter branch publication on every bounded regeneration attempt"
                    );
                }
                continue;
            }
        }
        let push_error = git.push().err();
        let current_main = git.main_sha()?;
        let remote_head = read.remote_branch_head()?;
        if remote_head.as_deref() != Some(armed_sha.as_str()) {
            if let Some(error) = push_error {
                return Err(error).context("publishing promoter branch");
            }
            if current_main.is_some() {
                bail!("remote promoter head does not equal the generated commit");
            }
        }
        if let Some(base) = current_main {
            let candidate_base = git.sole_parent(&armed_sha)?;
            if candidate_base.as_deref() != Some(base.as_str()) {
                git.delete_branch_with_lease(&armed_sha)?;
                if read.remote_branch_head()?.is_some() {
                    bail!("stale unpublished promoter branch deletion was not observed");
                }
                if attempt == 2 {
                    bail!(
                        "main advanced after promoter branch push on every bounded regeneration attempt"
                    );
                }
                continue;
            }
        }
        let create_error = write.create_pull_request().err();
        let created = match singleton(read)? {
            Some(pr) => pr,
            None => {
                if let Some(error) = create_error {
                    return Err(error).context("creating promoter pull request");
                }
                bail!("created promoter PR was not found");
            }
        };
        if created.head_sha != armed_sha {
            bail!("promoter PR head does not equal the generated commit");
        }
        if let (Some(parent), Some(main)) = (git.sole_parent(&armed_sha)?, git.main_sha()?)
            && parent != main
        {
            abort_unarmed(git, read, write, &created, parent, main)?;
            if attempt == 2 {
                bail!(
                    "main advanced before promoter publication linearization on every bounded regeneration attempt"
                );
            }
            continue;
        }
        if !arm_candidate(read, write, &created)? {
            return Ok(PromoterOutcome::Existing(created.number));
        }
        return Ok(publication_outcome(replaces.as_ref(), created.number));
    }
    bail!("bounded promoter regeneration exhausted")
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

    write.arm_auto_merge(event.number)?;
    let Some(verified) = read.pull_request(event.number)? else {
        bail!("promoter PR disappeared after dequeue recovery arm");
    };
    if verified.head_sha != event.head_sha
        || !land::arm_is_verified(verified.auto_merge_armed, verified.in_merge_queue)
    {
        bail!(
            "GitHub did not verify dequeue recovery arm or queue membership on the unchanged promoter head"
        );
    }
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
    use std::collections::{BTreeMap, VecDeque};
    use std::rc::Rc;

    use serde_json::json;

    use super::*;
    use crate::pr::snapshot::{CheckEntry, QueueState};

    struct FakeGit {
        calls: RefCell<Vec<&'static str>>,
        staged_diff: bool,
        fail_on: Option<&'static str>,
        head_sha: String,
        main: Option<String>,
        committed_provenance: RefCell<Option<GeneratedProvenance>>,
    }

    impl FakeGit {
        fn new(staged_diff: bool) -> Self {
            Self {
                calls: RefCell::new(Vec::new()),
                staged_diff,
                fail_on: None,
                head_sha: "promoted-head".into(),
                main: None,
                committed_provenance: RefCell::new(None),
            }
        }

        fn failing(operation: &'static str) -> Self {
            Self {
                fail_on: Some(operation),
                ..Self::new(true)
            }
        }

        fn call(&self, operation: &'static str) -> Result<()> {
            self.calls.borrow_mut().push(operation);
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

        fn main_sha(&self) -> Result<Option<String>> {
            Ok(self.main.clone())
        }

        fn fetch_exact(&self, _sha: &str) -> Result<()> {
            self.call("fetch")
        }

        fn sole_parent(&self, _sha: &str) -> Result<Option<String>> {
            Ok(self
                .committed_provenance
                .borrow()
                .as_ref()
                .map(|provenance| provenance.base.clone()))
        }

        fn is_strict_ancestor(&self, _ancestor: &str, _descendant: &str) -> Result<bool> {
            Ok(false)
        }

        fn merge_conflicts(&self, _main: &str, _head: &str) -> Result<bool> {
            Ok(false)
        }

        fn delete_branch_with_lease(&self, _expected: &str) -> Result<()> {
            self.call("delete")
        }

        fn discard_unpublished_candidate(&self, _base: &str) -> Result<()> {
            self.call("discard")
        }

        fn verify_generated_tree(
            &self,
            _head: &str,
            _provenance: &GeneratedProvenance,
        ) -> Result<bool> {
            Ok(false)
        }

        fn generated_provenance(&self, _head: &str) -> Result<Option<GeneratedProvenance>> {
            Ok(self.committed_provenance.borrow().clone())
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

        fn commit_with_provenance(&self, provenance: &GeneratedProvenance) -> Result<()> {
            self.commit()?;
            self.committed_provenance.replace(Some(provenance.clone()));
            Ok(())
        }

        fn push(&self) -> Result<()> {
            self.call("push")
        }
    }

    struct FakeGithub {
        owner: String,
        pulls: RefCell<Vec<PromoterPullRequest>>,
        remote_head: Option<String>,
        required: RequiredChecks,
        required_reads: Cell<usize>,
        merge_groups: Vec<String>,
        checks: BTreeMap<String, CommitChecks>,
        writes: RefCell<Vec<&'static str>>,
        arm_to_queue: bool,
        head_after_arm: Option<String>,
    }

    impl FakeGithub {
        fn empty() -> Self {
            Self {
                owner: "jaunder-org".into(),
                pulls: RefCell::new(Vec::new()),
                remote_head: None,
                required: required(),
                required_reads: Cell::new(0),
                merge_groups: vec!["merge-group".into()],
                checks: BTreeMap::from([
                    ("event-head".into(), green_checks("event-head")),
                    ("merge-group".into(), green_checks("merge-group")),
                ]),
                arm_to_queue: false,
                head_after_arm: None,
                writes: RefCell::new(Vec::new()),
            }
        }

        fn dequeue_ready() -> Self {
            let mut fake = Self::empty();
            fake.remote_head = Some("event-head".into());
            fake.pulls.borrow_mut().push(promoter_pr("event-head"));
            fake
        }
    }

    impl PromoterPrRead for FakeGithub {
        fn repository(&self) -> Result<(String, String)> {
            Ok((self.owner.clone(), "jaunder".into()))
        }

        fn open_pull_requests(&self) -> Result<Vec<PromoterPullRequest>> {
            Ok(self.pulls.borrow().clone())
        }

        fn all_pull_requests(&self) -> Result<Vec<PromoterPullRequest>> {
            Ok(self.pulls.borrow().clone())
        }

        fn snapshot(&self, _number: PrNumber) -> Result<Option<PrSnapshot>> {
            Ok(None)
        }

        fn app_identity(&self) -> Result<Option<AppIdentity>> {
            Ok(None)
        }

        fn comments(&self, _number: PrNumber) -> Result<Vec<PrComment>> {
            Ok(Vec::new())
        }

        fn pull_request(&self, number: PrNumber) -> Result<Option<PromoterPullRequest>> {
            Ok(self
                .pulls
                .borrow()
                .iter()
                .find(|pr| pr.number == number)
                .cloned())
        }

        fn remote_branch_head(&self) -> Result<Option<String>> {
            Ok(self.remote_head.clone())
        }

        fn required_checks(&self, _base: &str) -> Result<RequiredChecks> {
            self.required_reads.set(self.required_reads.get() + 1);
            Ok(self.required.clone())
        }

        fn merge_group_shas(&self, _event: &PullRequestEvent) -> Result<Vec<String>> {
            Ok(self.merge_groups.clone())
        }

        fn commit_checks(&self, sha: &str) -> Result<CommitChecks> {
            Ok(self
                .checks
                .get(sha)
                .cloned()
                .unwrap_or_else(|| CommitChecks {
                    sha: sha.to_string(),
                    checks: Vec::new(),
                }))
        }
    }

    impl PromoterPrWrite for FakeGithub {
        fn create_pull_request(&self) -> Result<()> {
            self.writes.borrow_mut().push("create");
            self.pulls.borrow_mut().push(promoter_pr("promoted-head"));
            Ok(())
        }

        fn arm_auto_merge(&self, number: PrNumber) -> Result<()> {
            self.writes.borrow_mut().push("arm");
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
            Ok(())
        }

        fn append_intent(&self, _number: PrNumber, _intent: &PromoterIntent) -> Result<()> {
            self.writes.borrow_mut().push("intent");
            Ok(())
        }

        fn close_pull_request(&self, number: PrNumber) -> Result<()> {
            self.writes.borrow_mut().push("close");
            let mut pulls = self.pulls.borrow_mut();
            let pr = pulls
                .iter_mut()
                .find(|pr| pr.number == number)
                .ok_or_else(|| anyhow!("missing PR"))?;
            pr.is_open = false;
            Ok(())
        }
    }

    #[derive(Default)]
    struct RecoveryState {
        remote: Option<String>,
        pulls: Vec<PromoterPullRequest>,
        comments: Vec<(PrNumber, Vec<PrComment>)>,
        writes: Vec<&'static str>,
        snapshots: Vec<(PrNumber, PrSnapshot)>,
        snapshot_after_intent: Option<PrSnapshot>,
        checks: BTreeMap<String, CommitChecks>,
    }

    struct RecoveryGit {
        push_error_after_write: bool,
        state: Rc<RefCell<RecoveryState>>,
        main_reads: RefCell<VecDeque<String>>,
        parents: RefCell<BTreeMap<String, String>>,
        provenance: BTreeMap<String, Option<GeneratedProvenance>>,
        tree_matches: bool,
        generated_heads: RefCell<VecDeque<String>>,
        published_head: RefCell<String>,
        calls: RefCell<Vec<&'static str>>,
        committed_provenance: RefCell<Option<GeneratedProvenance>>,
        fetched: RefCell<Vec<String>>,
    }

    impl RecoveryGit {
        fn new(state: Rc<RefCell<RecoveryState>>, main_reads: &[&str]) -> Self {
            Self {
                push_error_after_write: false,
                state,
                main_reads: RefCell::new(main_reads.iter().map(|sha| (*sha).into()).collect()),
                parents: RefCell::new(BTreeMap::from([
                    ("stale".into(), "base".into()),
                    ("orphan".into(), "current".into()),
                    ("generated".into(), "current".into()),
                ])),
                provenance: BTreeMap::new(),
                tree_matches: true,
                generated_heads: RefCell::new(["generated".into()].into()),
                published_head: RefCell::new("generated".into()),
                calls: RefCell::new(Vec::new()),
                committed_provenance: RefCell::new(None),
                fetched: RefCell::new(Vec::new()),
            }
        }

        fn call(&self, call: &'static str) {
            self.calls.borrow_mut().push(call);
        }
    }

    impl PromoterGit for RecoveryGit {
        fn prepare_fresh_main(&self) -> Result<()> {
            self.call("prepare");
            Ok(())
        }

        fn promote(&self) -> Result<()> {
            self.call("promote");
            Ok(())
        }

        fn main_sha(&self) -> Result<Option<String>> {
            let mut reads = self.main_reads.borrow_mut();
            Ok(reads
                .pop_front()
                .or_else(|| reads.back().cloned())
                .or_else(|| Some("current".into())))
        }

        fn fetch_exact(&self, sha: &str) -> Result<()> {
            self.fetched.borrow_mut().push(sha.to_string());
            Ok(())
        }

        fn sole_parent(&self, sha: &str) -> Result<Option<String>> {
            Ok(self.parents.borrow().get(sha).cloned())
        }

        fn is_strict_ancestor(&self, ancestor: &str, descendant: &str) -> Result<bool> {
            Ok(matches!(
                (ancestor, descendant),
                ("base", "current" | "newer" | "latest")
                    | ("current", "newer" | "latest")
                    | ("newer", "latest")
            ))
        }

        fn merge_conflicts(&self, main: &str, head: &str) -> Result<bool> {
            Ok(main == "current" && head == "stale")
        }

        fn delete_branch_with_lease(&self, expected: &str) -> Result<()> {
            self.call("delete");
            let mut state = self.state.borrow_mut();
            if state.remote.as_deref() != Some(expected) {
                bail!("lease refused");
            }
            state.remote = None;
            Ok(())
        }

        fn discard_unpublished_candidate(&self, _base: &str) -> Result<()> {
            self.call("discard");
            Ok(())
        }

        fn generated_provenance(&self, head: &str) -> Result<Option<GeneratedProvenance>> {
            if let Some(provenance) = self.provenance.get(head) {
                return Ok(provenance.clone());
            }
            Ok((self.published_head.borrow().as_str() == head)
                .then(|| self.committed_provenance.borrow().clone())
                .flatten())
        }

        fn verify_generated_tree(
            &self,
            _head: &str,
            _provenance: &GeneratedProvenance,
        ) -> Result<bool> {
            Ok(self.tree_matches)
        }

        fn has_staged_diff(&self) -> Result<bool> {
            self.call("diff");
            Ok(true)
        }

        fn format_staged_markdown(&self) -> Result<()> {
            self.call("format");
            Ok(())
        }

        fn commit(&self) -> Result<()> {
            self.call("commit");
            Ok(())
        }

        fn commit_with_provenance(&self, provenance: &GeneratedProvenance) -> Result<()> {
            self.commit()?;
            self.committed_provenance.replace(Some(provenance.clone()));
            Ok(())
        }

        fn head_sha(&self) -> Result<String> {
            let head = self
                .generated_heads
                .borrow_mut()
                .pop_front()
                .unwrap_or_else(|| self.published_head.borrow().clone());
            self.published_head.replace(head.clone());
            Ok(head)
        }

        fn push(&self) -> Result<()> {
            self.call("push");
            self.state.borrow_mut().remote = Some(self.published_head.borrow().clone());
            if self.push_error_after_write {
                bail!("push response was lost");
            }
            Ok(())
        }
    }

    struct RecoveryGithub {
        state: Rc<RefCell<RecoveryState>>,
    }

    impl RecoveryGithub {
        fn new(state: Rc<RefCell<RecoveryState>>) -> Self {
            Self { state }
        }
    }

    impl PromoterPrRead for RecoveryGithub {
        fn repository(&self) -> Result<(String, String)> {
            Ok(("jaunder-org".into(), "jaunder".into()))
        }

        fn open_pull_requests(&self) -> Result<Vec<PromoterPullRequest>> {
            Ok(self
                .state
                .borrow()
                .pulls
                .iter()
                .filter(|pr| pr.is_open)
                .cloned()
                .collect())
        }

        fn all_pull_requests(&self) -> Result<Vec<PromoterPullRequest>> {
            Ok(self.state.borrow().pulls.clone())
        }

        fn snapshot(&self, number: PrNumber) -> Result<Option<PrSnapshot>> {
            Ok(self
                .state
                .borrow()
                .snapshots
                .iter()
                .find(|(snapshot_number, _)| *snapshot_number == number)
                .map(|(_, snapshot)| snapshot.clone()))
        }

        fn app_identity(&self) -> Result<Option<AppIdentity>> {
            Ok(Some(AppIdentity {
                login: PROMOTER_BOT_LOGIN.into(),
                client_id: "app".into(),
            }))
        }

        fn comments(&self, number: PrNumber) -> Result<Vec<PrComment>> {
            Ok(self
                .state
                .borrow()
                .comments
                .iter()
                .find(|(comment_number, _)| *comment_number == number)
                .map(|(_, comments)| comments.clone())
                .unwrap_or_default())
        }
        fn pull_request(&self, number: PrNumber) -> Result<Option<PromoterPullRequest>> {
            Ok(self
                .state
                .borrow()
                .pulls
                .iter()
                .find(|pr| pr.number == number)
                .cloned())
        }

        fn remote_branch_head(&self) -> Result<Option<String>> {
            Ok(self.state.borrow().remote.clone())
        }

        fn required_checks(&self, _base: &str) -> Result<RequiredChecks> {
            Ok(required())
        }

        fn merge_group_shas(&self, _event: &PullRequestEvent) -> Result<Vec<String>> {
            Ok(Vec::new())
        }

        fn commit_checks(&self, sha: &str) -> Result<CommitChecks> {
            Ok(self
                .state
                .borrow()
                .checks
                .get(sha)
                .cloned()
                .unwrap_or_else(|| CommitChecks {
                    sha: sha.to_string(),
                    checks: Vec::new(),
                }))
        }
    }

    impl PromoterPrWrite for RecoveryGithub {
        fn create_pull_request(&self) -> Result<()> {
            let mut state = self.state.borrow_mut();
            state.writes.push("create");
            let head = state.remote.clone().expect("published branch");
            let number = PrNumber(743 + state.pulls.len() as u64);
            state.pulls.push(promoter_pr_with(number, &head));
            Ok(())
        }
        fn arm_auto_merge(&self, number: PrNumber) -> Result<()> {
            let mut state = self.state.borrow_mut();
            state.writes.push("arm");
            state
                .pulls
                .iter_mut()
                .find(|pr| pr.number == number)
                .expect("existing PR")
                .auto_merge_armed = true;
            Ok(())
        }

        fn append_intent(&self, number: PrNumber, intent: &PromoterIntent) -> Result<()> {
            let mut state = self.state.borrow_mut();
            state.writes.push("intent");
            let comment = PrComment {
                body: intent.body(),
                author: CommentAuthor {
                    login: PROMOTER_BOT_LOGIN.into(),
                    app_client_id: Some("app".into()),
                },
            };
            if let Some((_, comments)) = state
                .comments
                .iter_mut()
                .find(|(comment_number, _)| *comment_number == number)
            {
                comments.push(comment);
            } else {
                state.comments.push((number, vec![comment]));
            }
            if let Some(snapshot) = state.snapshot_after_intent.take() {
                if let Some((_, current)) = state
                    .snapshots
                    .iter_mut()
                    .find(|(snapshot_number, _)| *snapshot_number == number)
                {
                    *current = snapshot;
                } else {
                    state.snapshots.push((number, snapshot));
                }
            }
            Ok(())
        }

        fn close_pull_request(&self, number: PrNumber) -> Result<()> {
            let mut state = self.state.borrow_mut();
            state.writes.push("close");
            state
                .pulls
                .iter_mut()
                .find(|pr| pr.number == number)
                .expect("existing PR")
                .is_open = false;
            Ok(())
        }
    }

    fn promoter_pr_with(number: PrNumber, sha: &str) -> PromoterPullRequest {
        PromoterPullRequest {
            number,
            ..promoter_pr(sha)
        }
    }

    fn conflicting_snapshot(base: &str) -> PrSnapshot {
        PrSnapshot {
            state: PrState::Open,
            merged_at: None,
            merge_commit: None,
            mergeable: Mergeable::Conflicting,
            merge_state_status: MergeStateStatus::Dirty,
            auto_merge_armed: true,
            queue: QueueState {
                in_queue: false,
                position: None,
            },
            head_sha: "stale".into(),
            head_ref: BRANCH.into(),
            base_sha: base.into(),
            head_committed_at: "2026-09-01T00:00:00Z".into(),
            checks: Vec::new(),
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
        PromoterPullRequest {
            number: PrNumber(742),
            author_login: PROMOTER_BOT_LOGIN.into(),
            head_owner: "jaunder-org".into(),
            head_ref: BRANCH.into(),
            head_sha: sha.into(),
            base_ref: BASE_BRANCH.into(),
            body: format!("Automated ADR promotion.\n\n{MARKER}"),
            is_open: true,
            auto_merge_armed: false,
            in_merge_queue: false,
        }
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

        let pulls = github
            .open_pull_requests_with(|args| {
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
                    "headRefName": BRANCH,
                    "headRefOid": "queued-head",
                    "baseRefName": BASE_BRANCH,
                    "body": MARKER,
                    "autoMergeRequest": null
                }]))
            })
            .unwrap();

        assert_eq!(pulls[0].head_owner, "jaunder-org");
        assert!(!pulls[0].in_merge_queue);
    }

    #[test]
    fn single_pr_lookup_enriches_queue_state_from_graphql() {
        let github = GhPromoterPr {
            owner: "jaunder-org".into(),
            repo: "jaunder".into(),
        };

        let pull = github
            .pull_request_with(
                PrNumber(742),
                |args| {
                    assert_eq!(args[0..2], ["pr", "view"]);
                    assert!(!GhPromoterPr::pr_fields().contains("isInMergeQueue"));
                    Ok(json!({
                        "number": 742,
                        "state": "OPEN",
                        "headRepositoryOwner": {"login": "jaunder-org"},
                        "headRefName": BRANCH,
                        "headRefOid": "queued-head",
                        "baseRefName": BASE_BRANCH,
                        "body": MARKER,
                        "autoMergeRequest": null
                    }))
                },
                |args| {
                    assert_eq!(args[0..2], ["api", "graphql"]);
                    assert!(args.iter().any(|arg| arg.contains(snapshot::PR_QUERY)));
                    Ok(json!({
                        "data": {"repository": {"pullRequest": {
                            "state": "OPEN",
                            "mergedAt": null,
                            "baseRefOid": "base",
                            "mergeCommit": null,
                            "mergeable": "MERGEABLE",
                            "mergeStateStatus": "CLEAN",
                            "isInMergeQueue": true,
                            "mergeQueueEntry": {"position": 1},
                            "autoMergeRequest": null,
                            "headRefName": BRANCH,
                            "commits": {"nodes": [{"commit": {
                                "oid": "queued-head",
                                "committedDate": "2026-08-25T00:00:00Z"
                            }}]},
                            "statusCheckRollup": {"contexts": {"nodes": []}}
                        }}}
                    }))
                },
            )
            .unwrap()
            .unwrap();

        assert_eq!(pull.head_sha, "queued-head");
        assert!(pull.in_merge_queue);
        assert!(!pull.auto_merge_armed);
    }

    #[test]
    fn existing_singleton_freezes_its_head_and_skips_commit_or_remote_writes() {
        let git = FakeGit::new(true);
        let github = FakeGithub::dequeue_ready();

        let outcome = run_with(PromoterEvent::Generate, &git, &github, &github).unwrap();

        assert_eq!(outcome, PromoterOutcome::Existing(PrNumber(742)));
        assert!(git.calls.borrow().is_empty());
        assert!(github.writes.borrow().is_empty());
        assert_eq!(github.pulls.borrow()[0].head_sha, "event-head");
    }

    #[test]
    fn open_attempt_with_absent_stable_ref_fails_closed() {
        let git = FakeGit::new(true);
        let mut github = FakeGithub::dequeue_ready();
        github.remote_head = None;
        github.pulls.borrow_mut()[0].auto_merge_armed = true;

        assert!(
            run_with(PromoterEvent::Generate, &git, &github, &github)
                .unwrap_err()
                .to_string()
                .contains("do not name the same exact head")
        );
        assert!(git.calls.borrow().is_empty());
        assert!(github.writes.borrow().is_empty());
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
        let mut github = FakeGithub::empty();
        github.remote_head = Some("occupied-head".into());
        let mut occupant = promoter_pr("occupied-head");
        occupant.body = TITLE.into();
        github.pulls.borrow_mut().push(occupant);
        let remote_head = github.remote_head.clone();
        let pulls = github.pulls.borrow().clone();

        let error = run_with(PromoterEvent::Generate, &git, &github, &github).unwrap_err();

        assert!(
            error
                .to_string()
                .contains("without the exact promoter identity")
        );

        assert!(git.calls.borrow().is_empty());
        assert!(github.writes.borrow().is_empty());
        assert_eq!(github.remote_head, remote_head);
        assert_eq!(*github.pulls.borrow(), pulls);
    }
    #[test]
    fn generated_provenance_requires_canonical_versioned_trailers() {
        let canonical = format!(
            "{TITLE}\n\nJaunder-Promoter-Version: 1\nJaunder-Promoter-Base: base\nJaunder-Promoter-Replaces: 7@stale"
        );
        assert_eq!(
            parse_generated_provenance(&canonical),
            Some(GeneratedProvenance {
                version: 1,
                base: "base".into(),
                replaces: Some((PrNumber(7), "stale".into())),
            })
        );
        assert!(
            parse_generated_provenance(&format!(
                "{TITLE}\n\nJaunder-Promoter-Version: 2\nJaunder-Promoter-Base: base\nextra"
            ))
            .is_none()
        );
    }

    #[test]
    fn generated_provenance_rejects_reordered_or_noncanonical_trailers() {
        assert!(
            parse_generated_provenance(&format!(
                "{TITLE}\n\nJaunder-Promoter-Base: base\nJaunder-Promoter-Version: 1"
            ))
            .is_none()
        );
        assert!(
            parse_generated_provenance(&format!(
                "{TITLE}\n\nJaunder-Promoter-Version: 01\nJaunder-Promoter-Base: base"
            ))
            .is_none()
        );
    }

    #[test]
    fn intent_requires_canonical_field_order() {
        let intent = retirement_intent();
        assert_eq!(parse_intent(&intent.body()), Some(intent));
        assert!(
            parse_intent(
                "<!-- jaunder-adr-promoter -->\n<!-- jaunder-adr-promoter-intent:v1 pr=742 kind=retirement head=stale parent=base main=current -->"
            )
            .is_none()
        );
    }

    #[test]
    fn paginated_comments_are_flattened_without_losing_app_identity() {
        let pages = json!([
            [{
                "body": "first",
                "user": {"login": PROMOTER_BOT_LOGIN},
                "performed_via_github_app": {"client_id": "app"}
            }],
            [{
                "body": "second",
                "user": {"login": "human"},
                "performed_via_github_app": null
            }]
        ]);

        let comments = parse_comment_pages(&pages).unwrap();
        assert_eq!(comments.len(), 2);
        assert_eq!(comments[0].author.app_client_id.as_deref(), Some("app"));
        assert_eq!(comments[1].body, "second");
        assert!(parse_comment_pages(&json!([{"body": "not a page"}])).is_err());
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
    fn failed_push_without_exact_remote_postcondition_remains_visible() {
        let git = FakeGit::failing("push");
        let github = FakeGithub::empty();

        let error = run_with(PromoterEvent::Generate, &git, &github, &github).unwrap_err();
        assert!(error.to_string().contains("publishing promoter branch"));
        assert!(github.writes.borrow().is_empty());
    }

    #[test]
    fn remote_head_must_equal_generated_commit_before_pr_creation_or_arm() {
        let mut git = FakeGit::new(true);
        git.main = Some("base".into());
        let github = FakeGithub::empty();

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
        let git = FakeGit::new(true);
        let github = FakeGithub::empty();

        let outcome = run_with(PromoterEvent::Generate, &git, &github, &github).unwrap();

        assert_eq!(outcome, PromoterOutcome::Created(PrNumber(742)));
        assert_eq!(*github.writes.borrow(), ["create", "arm"]);
        let pulls = github.pulls.borrow();
        assert_eq!(pulls[0].head_sha, "promoted-head");
        assert!(pulls[0].auto_merge_armed);
    }

    #[test]
    fn new_promotion_accepts_direct_queue_membership_on_the_exact_head() {
        let git = FakeGit::new(true);
        let mut github = FakeGithub::empty();
        github.arm_to_queue = true;

        let outcome = run_with(PromoterEvent::Generate, &git, &github, &github).unwrap();

        assert_eq!(outcome, PromoterOutcome::Created(PrNumber(742)));
        let pulls = github.pulls.borrow();
        assert_eq!(pulls[0].head_sha, "promoted-head");
        assert!(!pulls[0].auto_merge_armed);
        assert!(pulls[0].in_merge_queue);
    }

    #[test]
    fn direct_queue_membership_on_a_replaced_head_does_not_verify_creation() {
        let git = FakeGit::new(true);
        let mut github = FakeGithub::empty();
        github.arm_to_queue = true;
        github.head_after_arm = Some("replacement-head".into());

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
    fn retirement_intent() -> PromoterIntent {
        PromoterIntent {
            kind: IntentKind::Retirement,
            number: PrNumber(742),
            head: "stale".into(),
            parent: "base".into(),
            main: "current".into(),
        }
    }

    fn authenticated_intent(intent: &PromoterIntent) -> PrComment {
        PrComment {
            body: intent.body(),
            author: CommentAuthor {
                login: PROMOTER_BOT_LOGIN.into(),
                app_client_id: Some("app".into()),
            },
        }
    }

    #[test]
    fn identical_durable_intents_are_idempotent_but_conflicting_intents_fail() {
        let intent = retirement_intent();
        let state = Rc::new(RefCell::new(RecoveryState {
            comments: vec![(
                intent.number,
                vec![authenticated_intent(&intent), authenticated_intent(&intent)],
            )],
            ..RecoveryState::default()
        }));
        let github = RecoveryGithub::new(state.clone());
        assert_eq!(
            durable_intent(&github, intent.number, IntentKind::Retirement).unwrap(),
            Some(intent.clone())
        );

        let conflicting = PromoterIntent {
            main: "newer".into(),
            ..intent.clone()
        };
        state
            .borrow_mut()
            .comments
            .first_mut()
            .unwrap()
            .1
            .push(authenticated_intent(&conflicting));
        assert!(
            durable_intent(&github, intent.number, IntentKind::Retirement)
                .unwrap_err()
                .to_string()
                .contains("conflicting durable promoter intents")
        );
    }

    #[test]
    fn conflict_snapshot_must_name_the_refreshed_main_object() {
        let state = Rc::new(RefCell::new(RecoveryState {
            snapshots: vec![(PrNumber(742), conflicting_snapshot("other"))],
            ..RecoveryState::default()
        }));
        let git = RecoveryGit::new(state.clone(), &["current"]);
        let github = RecoveryGithub::new(state);

        assert_eq!(
            positive_conflict(&git, &github, &promoter_pr("stale")).unwrap(),
            None
        );
        assert!(!git.calls.borrow().contains(&"delete"));
    }

    #[test]
    fn new_conflict_cannot_authorize_retirement_without_exact_stable_ref() {
        let state = Rc::new(RefCell::new(RecoveryState {
            pulls: vec![promoter_pr("stale")],
            snapshots: vec![(PrNumber(742), conflicting_snapshot("current"))],
            ..RecoveryState::default()
        }));
        let git = RecoveryGit::new(state.clone(), &["current"]);
        let github = RecoveryGithub::new(state.clone());

        assert!(
            run_with(PromoterEvent::Generate, &git, &github, &github)
                .unwrap_err()
                .to_string()
                .contains("do not name the same exact head")
        );
        assert!(state.borrow().writes.is_empty());
        assert!(!git.calls.borrow().contains(&"delete"));
    }

    #[test]
    fn conflict_evidence_is_revalidated_after_recording_retirement_intent() {
        let state = Rc::new(RefCell::new(RecoveryState {
            remote: Some("stale".into()),
            pulls: vec![promoter_pr("stale")],
            snapshots: vec![(PrNumber(742), conflicting_snapshot("current"))],
            snapshot_after_intent: Some(conflicting_snapshot("other")),
            ..RecoveryState::default()
        }));
        let git = RecoveryGit::new(state.clone(), &["current", "current"]);
        let github = RecoveryGithub::new(state.clone());

        assert!(
            run_with(PromoterEvent::Generate, &git, &github, &github)
                .unwrap_err()
                .to_string()
                .contains("retirement evidence changed before branch mutation")
        );
        assert_eq!(state.borrow().remote.as_deref(), Some("stale"));
        assert_eq!(state.borrow().writes, ["intent"]);
        assert!(!git.calls.borrow().contains(&"delete"));
    }

    #[test]
    fn stale_failed_required_check_keeps_unarmed_candidate_visible() {
        let mut git = RecoveryGit::new(
            Rc::new(RefCell::new(RecoveryState::default())),
            &["current", "current", "current"],
        );
        git.provenance.insert(
            "generated".into(),
            Some(GeneratedProvenance {
                version: PROMOTER_VERSION,
                base: "base".into(),
                replaces: None,
            }),
        );
        git.parents
            .borrow_mut()
            .insert("generated".into(), "base".into());
        let state = git.state.clone();
        {
            let mut state = state.borrow_mut();
            state.remote = Some("generated".into());
            state.pulls.push(promoter_pr("generated"));
            state.checks.insert(
                "generated".into(),
                CommitChecks {
                    sha: "generated".into(),
                    checks: vec![check("Validate (no e2e)", CheckState::Failure)],
                },
            );
        }
        let github = RecoveryGithub::new(state.clone());

        assert_eq!(
            run_with(PromoterEvent::Generate, &git, &github, &github).unwrap(),
            PromoterOutcome::Existing(PrNumber(742))
        );
        assert!(state.borrow().writes.is_empty());
        assert_eq!(state.borrow().remote.as_deref(), Some("generated"));
    }

    fn closed_retirement_state(remote: &str) -> Rc<RefCell<RecoveryState>> {
        let state = Rc::new(RefCell::new(RecoveryState {
            remote: Some(remote.into()),
            pulls: vec![PromoterPullRequest {
                is_open: false,
                ..promoter_pr("stale")
            }],
            ..RecoveryState::default()
        }));
        state.borrow_mut().comments.push((
            PrNumber(742),
            vec![authenticated_intent(&retirement_intent())],
        ));
        state
    }

    fn orphan_provenance(base: &str) -> GeneratedProvenance {
        GeneratedProvenance {
            version: PROMOTER_VERSION,
            base: base.into(),
            replaces: Some((PrNumber(742), "stale".into())),
        }
    }

    #[test]
    fn authenticated_current_orphan_successor_is_adopted_and_armed() {
        let state = closed_retirement_state("orphan");
        let mut git = RecoveryGit::new(state.clone(), &["current"]);
        git.provenance
            .insert("orphan".into(), Some(orphan_provenance("current")));
        let github = RecoveryGithub::new(state.clone());

        assert_eq!(
            run_with(PromoterEvent::Generate, &git, &github, &github).unwrap(),
            PromoterOutcome::Replaced {
                stale: PrNumber(742),
                successor: PrNumber(744),
            }
        );
        assert_eq!(state.borrow().writes, ["create", "arm"]);
        assert_eq!(state.borrow().remote.as_deref(), Some("orphan"));
        assert!(!git.calls.borrow().contains(&"delete"));
        assert!(git.fetched.borrow().iter().any(|sha| sha == "stale"));
    }

    #[test]
    fn stale_orphan_is_exactly_deleted_then_regenerated() {
        let state = closed_retirement_state("orphan");
        let mut git = RecoveryGit::new(state.clone(), &["current"]);
        git.parents
            .borrow_mut()
            .insert("orphan".into(), "base".into());
        git.provenance
            .insert("orphan".into(), Some(orphan_provenance("base")));
        let github = RecoveryGithub::new(state.clone());

        assert_eq!(
            run_with(PromoterEvent::Generate, &git, &github, &github).unwrap(),
            PromoterOutcome::Replaced {
                stale: PrNumber(742),
                successor: PrNumber(744),
            }
        );
        assert_eq!(
            git.calls
                .borrow()
                .iter()
                .filter(|&&call| call == "delete")
                .count(),
            1
        );
        assert_eq!(state.borrow().remote.as_deref(), Some("generated"));
    }

    #[test]
    fn orphan_rejects_unknown_generator_version() {
        let state = closed_retirement_state("orphan");
        let mut git = RecoveryGit::new(state.clone(), &["current"]);
        let mut provenance = orphan_provenance("current");
        provenance.version = 2;
        git.provenance.insert("orphan".into(), Some(provenance));
        let github = RecoveryGithub::new(state.clone());

        assert!(
            run_with(PromoterEvent::Generate, &git, &github, &github)
                .unwrap_err()
                .to_string()
                .contains("unsupported orphan promoter generator version")
        );
        assert_eq!(state.borrow().remote.as_deref(), Some("orphan"));
    }

    #[test]
    fn orphan_rejects_malformed_canonical_provenance() {
        let state = closed_retirement_state("orphan");
        let git = RecoveryGit::new(state.clone(), &["current"]);
        let github = RecoveryGithub::new(state.clone());

        assert!(
            run_with(PromoterEvent::Generate, &git, &github, &github)
                .unwrap_err()
                .to_string()
                .contains("lacks canonical generated provenance")
        );
        assert_eq!(state.borrow().remote.as_deref(), Some("orphan"));
    }

    #[test]
    fn orphan_rejects_reconstructed_tree_mismatch() {
        let state = closed_retirement_state("orphan");
        let mut git = RecoveryGit::new(state.clone(), &["current"]);
        git.provenance
            .insert("orphan".into(), Some(orphan_provenance("current")));
        git.tree_matches = false;
        let github = RecoveryGithub::new(state.clone());

        assert!(
            run_with(PromoterEvent::Generate, &git, &github, &github)
                .unwrap_err()
                .to_string()
                .contains("does not reconstruct the exact retired promotion")
        );
        assert_eq!(state.borrow().remote.as_deref(), Some("orphan"));
    }
    #[test]
    fn pre_push_main_advance_regenerates_within_the_fixed_bound() {
        let state = Rc::new(RefCell::new(RecoveryState::default()));
        let git = RecoveryGit::new(
            state.clone(),
            &["base", "newer", "current", "current", "current", "current"],
        );
        let github = RecoveryGithub::new(state.clone());

        assert_eq!(
            run_with(PromoterEvent::Generate, &git, &github, &github).unwrap(),
            PromoterOutcome::Created(PrNumber(743))
        );
        assert_eq!(
            git.calls
                .borrow()
                .iter()
                .filter(|&&call| call == "prepare")
                .count(),
            2
        );
        assert_eq!(
            git.calls
                .borrow()
                .iter()
                .filter(|&&call| call == "discard")
                .count(),
            1
        );
    }

    #[test]
    fn main_advance_during_commit_discards_candidate_before_push() {
        let state = Rc::new(RefCell::new(RecoveryState::default()));
        let git = RecoveryGit::new(
            state.clone(),
            &[
                "base", "base", "newer", "current", "current", "current", "current", "current",
            ],
        );
        git.parents
            .borrow_mut()
            .insert("generated".into(), "base".into());
        git.parents
            .borrow_mut()
            .insert("successor".into(), "current".into());
        git.generated_heads
            .replace(["generated".into(), "successor".into()].into());
        let github = RecoveryGithub::new(state);

        assert_eq!(
            run_with(PromoterEvent::Generate, &git, &github, &github).unwrap(),
            PromoterOutcome::Created(PrNumber(743))
        );
        assert_eq!(
            git.calls
                .borrow()
                .iter()
                .filter(|&&call| call == "discard")
                .count(),
            1
        );
        assert_eq!(
            git.calls
                .borrow()
                .iter()
                .filter(|&&call| call == "push")
                .count(),
            1
        );
    }

    #[test]
    fn pre_push_main_advance_exhaustion_is_visible_after_three_attempts() {
        let state = Rc::new(RefCell::new(RecoveryState::default()));
        let git = RecoveryGit::new(
            state.clone(),
            &["base", "newer", "current", "newer", "current", "newer"],
        );
        let github = RecoveryGithub::new(state);

        assert!(
            run_with(PromoterEvent::Generate, &git, &github, &github)
                .unwrap_err()
                .to_string()
                .contains(
                    "before promoter branch publication on every bounded regeneration attempt"
                )
        );
        assert_eq!(
            git.calls
                .borrow()
                .iter()
                .filter(|&&call| call == "prepare")
                .count(),
            3
        );
        assert_eq!(
            git.calls
                .borrow()
                .iter()
                .filter(|&&call| call == "discard")
                .count(),
            3
        );
    }
    #[test]
    fn retirement_replay_closes_after_leased_deletion_when_branch_is_present() {
        let state = Rc::new(RefCell::new(RecoveryState {
            remote: Some("stale".into()),
            pulls: vec![promoter_pr("stale")],
            ..RecoveryState::default()
        }));
        let git = RecoveryGit::new(state.clone(), &["current"]);
        let github = RecoveryGithub::new(state.clone());
        replay_retirement(
            &git,
            &github,
            &github,
            &promoter_pr("stale"),
            &retirement_intent(),
        )
        .unwrap();
        assert_eq!(state.borrow().writes, ["close"]);
        assert!(git.calls.borrow().contains(&"delete"));
    }

    #[test]
    fn retirement_replay_closes_when_prior_crash_already_deleted_branch() {
        let state = Rc::new(RefCell::new(RecoveryState {
            pulls: vec![promoter_pr("stale")],
            ..RecoveryState::default()
        }));
        let git = RecoveryGit::new(state.clone(), &["current"]);
        let github = RecoveryGithub::new(state.clone());
        replay_retirement(
            &git,
            &github,
            &github,
            &promoter_pr("stale"),
            &retirement_intent(),
        )
        .unwrap();
        assert_eq!(state.borrow().writes, ["close"]);
        assert!(!git.calls.borrow().contains(&"delete"));
    }
    fn publication_abort_intent() -> PromoterIntent {
        PromoterIntent {
            kind: IntentKind::PublicationAbort,
            number: PrNumber(742),
            head: "candidate".into(),
            parent: "base".into(),
            main: "current".into(),
        }
    }

    fn publication_abort_provenance() -> GeneratedProvenance {
        GeneratedProvenance {
            version: PROMOTER_VERSION,
            base: "base".into(),
            replaces: None,
        }
    }

    #[test]
    fn publication_abort_replay_closes_after_leased_deletion_when_branch_is_present() {
        let state = Rc::new(RefCell::new(RecoveryState {
            remote: Some("candidate".into()),
            pulls: vec![promoter_pr("candidate")],
            ..RecoveryState::default()
        }));
        let mut git = RecoveryGit::new(state.clone(), &["current"]);
        git.parents
            .borrow_mut()
            .insert("candidate".into(), "base".into());
        git.provenance
            .insert("candidate".into(), Some(publication_abort_provenance()));
        let github = RecoveryGithub::new(state.clone());
        replay_publication_abort(
            &git,
            &github,
            &github,
            &promoter_pr("candidate"),
            &publication_abort_intent(),
        )
        .unwrap();
        assert_eq!(state.borrow().writes, ["close"]);
        assert!(git.calls.borrow().contains(&"delete"));
    }

    #[test]
    fn publication_abort_replay_closes_when_prior_crash_already_deleted_branch() {
        let state = Rc::new(RefCell::new(RecoveryState {
            pulls: vec![promoter_pr("candidate")],
            ..RecoveryState::default()
        }));
        let mut git = RecoveryGit::new(state.clone(), &["current"]);
        git.parents
            .borrow_mut()
            .insert("candidate".into(), "base".into());
        git.provenance
            .insert("candidate".into(), Some(publication_abort_provenance()));
        let github = RecoveryGithub::new(state.clone());
        replay_publication_abort(
            &git,
            &github,
            &github,
            &promoter_pr("candidate"),
            &publication_abort_intent(),
        )
        .unwrap();
        assert_eq!(state.borrow().writes, ["close"]);
        assert!(!git.calls.borrow().contains(&"delete"));
    }

    #[test]
    fn publication_abort_replay_refuses_unverified_generated_candidate() {
        let state = Rc::new(RefCell::new(RecoveryState {
            remote: Some("candidate".into()),
            pulls: vec![promoter_pr("candidate")],
            ..RecoveryState::default()
        }));
        let git = RecoveryGit::new(state.clone(), &["current"]);
        git.parents
            .borrow_mut()
            .insert("candidate".into(), "base".into());
        let github = RecoveryGithub::new(state.clone());

        assert!(
            replay_publication_abort(
                &git,
                &github,
                &github,
                &promoter_pr("candidate"),
                &publication_abort_intent(),
            )
            .unwrap_err()
            .to_string()
            .contains("lacks canonical provenance")
        );
        assert_eq!(state.borrow().remote.as_deref(), Some("candidate"));
        assert!(state.borrow().writes.is_empty());
        assert!(!git.calls.borrow().contains(&"delete"));
    }

    #[test]
    fn publication_abort_replay_preserves_replacement_provenance_and_outcome() {
        let intent = PromoterIntent {
            kind: IntentKind::PublicationAbort,
            number: PrNumber(743),
            head: "candidate".into(),
            parent: "base".into(),
            main: "current".into(),
        };
        let state = Rc::new(RefCell::new(RecoveryState {
            pulls: vec![PromoterPullRequest {
                is_open: false,
                ..promoter_pr_with(intent.number, &intent.head)
            }],
            comments: vec![(intent.number, vec![authenticated_intent(&intent)])],
            ..RecoveryState::default()
        }));
        let mut git = RecoveryGit::new(
            state.clone(),
            &["current", "current", "current", "current", "current"],
        );
        git.parents
            .borrow_mut()
            .insert("candidate".into(), "base".into());
        git.provenance.insert(
            "candidate".into(),
            Some(GeneratedProvenance {
                version: PROMOTER_VERSION,
                base: "base".into(),
                replaces: Some((PrNumber(742), "stale".into())),
            }),
        );
        let github = RecoveryGithub::new(state);

        assert_eq!(
            run_with(PromoterEvent::Generate, &git, &github, &github).unwrap(),
            PromoterOutcome::Replaced {
                stale: PrNumber(742),
                successor: PrNumber(744),
            }
        );
        assert_eq!(
            git.committed_provenance
                .borrow()
                .as_ref()
                .and_then(|provenance| provenance.replaces.clone()),
            Some((PrNumber(742), "stale".into()))
        );
    }

    #[test]
    fn ambiguous_push_response_continues_from_exact_remote_postcondition() {
        let state = Rc::new(RefCell::new(RecoveryState::default()));
        let mut git = RecoveryGit::new(
            state.clone(),
            &["current", "current", "current", "current", "current"],
        );
        git.push_error_after_write = true;
        let github = RecoveryGithub::new(state.clone());

        assert_eq!(
            run_with(PromoterEvent::Generate, &git, &github, &github).unwrap(),
            PromoterOutcome::Created(PrNumber(743))
        );
        assert_eq!(state.borrow().remote.as_deref(), Some("generated"));
        assert_eq!(state.borrow().writes, ["create", "arm"]);
    }
    #[test]
    fn post_push_main_advance_regenerates_within_the_fixed_bound() {
        let state = Rc::new(RefCell::new(RecoveryState::default()));
        let git = RecoveryGit::new(
            state.clone(),
            &[
                "base", "base", "base", "current", "current", "current", "current", "current",
                "current",
            ],
        );
        git.parents
            .borrow_mut()
            .insert("generated".into(), "base".into());
        git.parents
            .borrow_mut()
            .insert("successor".into(), "current".into());
        git.generated_heads
            .replace(["generated".into(), "successor".into()].into());
        let github = RecoveryGithub::new(state);
        assert_eq!(
            run_with(PromoterEvent::Generate, &git, &github, &github).unwrap(),
            PromoterOutcome::Created(PrNumber(743))
        );
        assert_eq!(
            git.calls
                .borrow()
                .iter()
                .filter(|&&call| call == "delete")
                .count(),
            1
        );
    }

    #[test]
    fn post_push_main_advance_exhaustion_is_visible_after_three_attempts() {
        let state = Rc::new(RefCell::new(RecoveryState::default()));
        let git = RecoveryGit::new(
            state.clone(),
            &[
                "base", "base", "base", "current", "current", "current", "current", "newer",
                "newer", "newer", "newer", "latest",
            ],
        );
        for (head, parent) in [
            ("generated", "base"),
            ("second", "current"),
            ("third", "newer"),
        ] {
            git.parents.borrow_mut().insert(head.into(), parent.into());
        }
        git.generated_heads
            .replace(["generated".into(), "second".into(), "third".into()].into());
        let github = RecoveryGithub::new(state);
        assert!(
            run_with(PromoterEvent::Generate, &git, &github, &github)
                .unwrap_err()
                .to_string()
                .contains("after promoter branch push on every bounded regeneration attempt")
        );
    }
    #[test]
    fn post_create_main_advance_aborts_then_regenerates_within_the_fixed_bound() {
        let state = Rc::new(RefCell::new(RecoveryState::default()));
        let git = RecoveryGit::new(
            state.clone(),
            &[
                "base", "base", "base", "base", "current", "current", "current", "current",
                "current", "current", "current",
            ],
        );
        git.parents
            .borrow_mut()
            .insert("generated".into(), "base".into());
        git.parents
            .borrow_mut()
            .insert("successor".into(), "current".into());
        git.generated_heads
            .replace(["generated".into(), "successor".into()].into());
        let github = RecoveryGithub::new(state.clone());
        assert_eq!(
            run_with(PromoterEvent::Generate, &git, &github, &github).unwrap(),
            PromoterOutcome::Created(PrNumber(744))
        );
        assert_eq!(
            state.borrow().writes,
            ["create", "intent", "close", "create", "arm"]
        );
    }

    #[test]
    fn post_create_main_advance_exhaustion_is_visible_after_three_attempts() {
        let state = Rc::new(RefCell::new(RecoveryState::default()));
        let git = RecoveryGit::new(
            state.clone(),
            &[
                "base", "base", "base", "base", "current", "current", "current", "current",
                "current", "current", "newer", "newer", "newer", "newer", "newer", "newer",
                "latest", "latest",
            ],
        );
        git.parents
            .borrow_mut()
            .insert("generated".into(), "base".into());
        git.parents
            .borrow_mut()
            .insert("second".into(), "current".into());
        git.parents
            .borrow_mut()
            .insert("third".into(), "newer".into());
        git.generated_heads
            .replace(["generated".into(), "second".into(), "third".into()].into());
        let github = RecoveryGithub::new(state);
        let error = run_with(PromoterEvent::Generate, &git, &github, &github).unwrap_err();
        assert!(
            error.to_string().contains(
                "before promoter publication linearization on every bounded regeneration attempt"
            ),
            "{error:#}"
        );
    }
}
