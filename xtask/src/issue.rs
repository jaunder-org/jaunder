use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};
use std::process::Command;

use anyhow::{Context, Result, anyhow, bail};
use clap::{Subcommand, ValueEnum};
use serde::Serialize;
use serde_json::Value;

use crate::pr::gh;
use crate::result::{CommandResult, StepResult};

const OWNER: &str = "jaunder-org";
const REPO: &str = "jaunder";
const BACKLOG_PROJECT_NUMBER: u64 = 1;
const BACKLOG_PROJECT_ID: &str = "PVT_kwDOECw7os4BblPP";
const PRIORITY_FIELD_ID: &str = "PVTSSF_lADOECw7os4BblPPzhWUx50";

#[derive(Subcommand)]
pub enum IssueCommand {
    /// Gather open milestone issues, claim/blocker state, and local resume artifacts. Read-only.
    #[command(
        after_help = "EXAMPLES:\n  cargo xtask issue candidates --milestone 3\n  cargo xtask --json issue candidates --milestone \"Developer tooling & DX\""
    )]
    Candidates {
        /// Open milestone title or number. Resolution must be exact and unambiguous.
        #[arg(long)]
        milestone: String,
    },
    /// Create a fully triaged issue from explicit decisions. Mutates GitHub.
    #[command(
        after_help = "EXAMPLES:\n  cargo xtask --json issue create --title \"feat(xtask): add helper\" --type Task --milestone \"Developer tooling & DX\" --priority P2 --label tooling --label dx --body-file /tmp/issue.md"
    )]
    Create {
        /// Conventional Commit issue title.
        #[arg(long)]
        title: String,
        /// GitHub issue type, validated against the repository.
        #[arg(long = "type")]
        issue_type: String,
        /// Open milestone title or number. Resolution must be exact and unambiguous.
        #[arg(long)]
        milestone: String,
        /// Jaunder Backlog Priority field value.
        #[arg(long, value_enum)]
        priority: Priority,
        /// Topic label. Repeat for multiple labels.
        #[arg(long = "label", required = true)]
        labels: Vec<String>,
        /// Markdown body file.
        #[arg(long)]
        body_file: PathBuf,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, ValueEnum)]
pub enum Priority {
    P1,
    P2,
    P3,
    P4,
}

impl Priority {
    fn as_str(self) -> &'static str {
        match self {
            Self::P1 => "P1",
            Self::P2 => "P2",
            Self::P3 => "P3",
            Self::P4 => "P4",
        }
    }

    fn option_id(self) -> &'static str {
        match self {
            Self::P1 => "2d6fa1d1",
            Self::P2 => "be01f9da",
            Self::P3 => "0bba09bc",
            Self::P4 => "4b1f556c",
        }
    }
}

#[derive(Debug, Serialize)]
#[serde(untagged)]
pub enum IssueReport {
    Candidates(CandidateReport),
    Created(CreateReport),
}

#[derive(Debug, Serialize)]
pub struct CandidateReport {
    pub milestone: MilestoneSummary,
    pub candidates: Vec<CandidateIssue>,
    pub skipped: Vec<SkippedIssue>,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct MilestoneSummary {
    pub number: u64,
    pub title: String,
}

#[derive(Debug, Serialize)]
pub struct CandidateIssue {
    pub number: u64,
    pub title: String,
    pub url: String,
    pub labels: Vec<String>,
    pub status: String,
    pub blocked_by: Vec<u64>,
    pub local: LocalState,
}

#[derive(Debug, Serialize)]
pub struct SkippedIssue {
    pub number: u64,
    pub title: String,
    pub reason: String,
    pub detail: String,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct LocalState {
    pub branch: Option<String>,
    pub spec: Option<String>,
    pub plan: Option<String>,
    pub plan_progress: Option<PlanProgress>,
    pub open_pr: Option<OpenPr>,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct PlanProgress {
    pub done: usize,
    pub total: usize,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct OpenPr {
    pub number: u64,
    pub url: String,
    pub state: String,
}

#[derive(Debug, Serialize)]
pub struct CreateReport {
    pub issue: CreatedIssue,
    pub project: ProjectApplication,
    pub body: BodyReadback,
}

#[derive(Debug, Serialize)]
pub struct CreatedIssue {
    pub number: u64,
    pub url: String,
    pub title: String,
    #[serde(rename = "type")]
    pub issue_type: String,
    pub labels: Vec<String>,
    pub milestone: String,
}

#[derive(Debug, Serialize)]
pub struct ProjectApplication {
    pub number: u64,
    pub item_id: String,
    pub priority: String,
}

#[derive(Debug, Serialize)]
pub struct BodyReadback {
    pub readback_matches: bool,
    pub warning: Option<String>,
}

#[derive(Debug, Clone)]
struct Milestone {
    id: String,
    number: u64,
    title: String,
}

#[derive(Debug, Clone)]
struct NamedNode {
    id: String,
    name: String,
}

#[derive(Debug, Clone)]
struct Metadata {
    repository_id: String,
    milestones: Vec<Milestone>,
    labels: Vec<NamedNode>,
    issue_types: Vec<NamedNode>,
}

#[derive(Debug, Clone)]
struct IssueListing {
    number: u64,
    title: String,
    url: String,
    labels: Vec<String>,
}

#[derive(Debug, Clone)]
struct ProjectStatus {
    item_id: Option<String>,
    status: Option<String>,
}

trait Github {
    fn json(&self, args: &[String]) -> Result<Value>;
    fn json_stdin(&self, args: &[String], stdin: &str) -> Result<Value>;
}

struct RealGithub;

impl Github for RealGithub {
    fn json(&self, args: &[String]) -> Result<Value> {
        let refs = args.iter().map(String::as_str).collect::<Vec<_>>();
        gh::run_gh(&refs).map_err(|err| anyhow!("{err:?}"))
    }

    fn json_stdin(&self, args: &[String], stdin: &str) -> Result<Value> {
        let refs = args.iter().map(String::as_str).collect::<Vec<_>>();
        gh::run_gh_stdin(&refs, stdin).map_err(|err| anyhow!("{err:?}"))
    }
}

pub fn execute(sub: IssueCommand) -> Result<CommandResult> {
    let start = std::time::Instant::now();
    let gh = RealGithub;
    let mut result = match sub {
        IssueCommand::Candidates { milestone } => {
            let report = candidates(&gh, &milestone)?;
            into_result("issue-candidates", IssueReport::Candidates(report))
        }
        IssueCommand::Create {
            title,
            issue_type,
            milestone,
            priority,
            labels,
            body_file,
        } => {
            let report = create(
                &gh,
                CreateInput {
                    title,
                    issue_type,
                    milestone,
                    priority,
                    labels,
                    body_file,
                },
            )?;
            into_result("issue-create", IssueReport::Created(report))
        }
    };
    crate::lifecycle::finalize(&mut result, start);
    Ok(result)
}

fn into_result(command: &str, report: IssueReport) -> CommandResult {
    let mut result = CommandResult::new(command);
    match &report {
        IssueReport::Candidates(report) => {
            result.push(StepResult::ok("issue-candidates").detail(format!(
                "{} candidate(s), {} skipped for milestone {}",
                report.candidates.len(),
                report.skipped.len(),
                report.milestone.title
            )))
        }
        IssueReport::Created(report) => {
            result.push(StepResult::ok("issue-create").detail(format!(
                "created #{} and set Jaunder Backlog Priority {}",
                report.issue.number, report.project.priority
            )))
        }
    }
    result.issue = Some(report);
    result
}

struct CreateInput {
    title: String,
    issue_type: String,
    milestone: String,
    priority: Priority,
    labels: Vec<String>,
    body_file: PathBuf,
}

fn candidates(gh: &impl Github, milestone_arg: &str) -> Result<CandidateReport> {
    let metadata = fetch_metadata(gh)?;
    let milestone = resolve_milestone(&metadata.milestones, milestone_arg)?;
    let issues = list_milestone_issues(gh, milestone.number)?;
    let mut candidates = Vec::new();
    let mut skipped = Vec::new();
    for issue in issues {
        let status = fetch_project_status(gh, issue.number)?;
        let blockers = fetch_open_blockers(gh, issue.number)?;
        if let Some(status) = status.status.as_deref()
            && status == "In Progress"
        {
            skipped.push(SkippedIssue {
                number: issue.number,
                title: issue.title,
                reason: "claimed".into(),
                detail: "Jaunder Backlog Status is In Progress".into(),
            });
            continue;
        }
        if status.status.is_none() {
            skipped.push(SkippedIssue {
                number: issue.number,
                title: issue.title,
                reason: "missing-project-status".into(),
                detail: status.item_id.map_or_else(
                    || format!("issue is not in Jaunder Backlog project #{BACKLOG_PROJECT_NUMBER}"),
                    |item_id| format!("Jaunder Backlog project item {item_id} has no Status value"),
                ),
            });
            continue;
        }
        if !blockers.is_empty() {
            skipped.push(SkippedIssue {
                number: issue.number,
                title: issue.title,
                reason: "blocked".into(),
                detail: format!("blocked by open issue(s): {}", join_numbers(&blockers)),
            });
            continue;
        }
        let branch = find_issue_branch(issue.number)?;
        let open_pr = match branch.as_deref() {
            Some(branch) => fetch_open_pr(gh, branch)?,
            None => None,
        };
        candidates.push(CandidateIssue {
            number: issue.number,
            title: issue.title,
            url: issue.url,
            labels: issue.labels,
            status: status.status.unwrap_or_default(),
            blocked_by: blockers,
            local: LocalState {
                branch,
                spec: find_issue_doc("docs/superpowers/specs", issue.number)?,
                plan: find_issue_doc("docs/superpowers/plans", issue.number)?,
                plan_progress: plan_progress(issue.number)?,
                open_pr,
            },
        });
    }
    candidates.sort_by_key(|issue| issue.number);
    skipped.sort_by_key(|issue| issue.number);
    Ok(CandidateReport {
        milestone: MilestoneSummary {
            number: milestone.number,
            title: milestone.title,
        },
        candidates,
        skipped,
    })
}

fn create(gh: &impl Github, input: CreateInput) -> Result<CreateReport> {
    validate_title(&input.title)?;
    let body = std::fs::read_to_string(&input.body_file)
        .with_context(|| format!("reading body file {}", input.body_file.display()))?;
    if body.trim().is_empty() {
        bail!("body file is empty: {}", input.body_file.display());
    }
    let metadata = fetch_metadata(gh)?;
    let milestone = resolve_milestone(&metadata.milestones, &input.milestone)?;
    let issue_type = resolve_named(&metadata.issue_types, &input.issue_type, "issue type")?;
    let labels = resolve_labels(&metadata.labels, &input.labels)?;
    let created = create_issue(
        gh,
        &metadata.repository_id,
        &input.title,
        &body,
        &milestone,
        &issue_type,
        &labels,
    )?;
    let item_id = add_to_backlog(gh, &created.id)?;
    set_priority(gh, &item_id, input.priority)?;
    let readback = fetch_issue_body(gh, created.number)?;
    let body_report = compare_body(&body, &readback);
    Ok(CreateReport {
        issue: CreatedIssue {
            number: created.number,
            url: created.url,
            title: created.title,
            issue_type: issue_type.name,
            labels: input.labels,
            milestone: milestone.title,
        },
        project: ProjectApplication {
            number: BACKLOG_PROJECT_NUMBER,
            item_id,
            priority: input.priority.as_str().into(),
        },
        body: body_report,
    })
}

#[derive(Debug)]
struct CreatedIssueRaw {
    id: String,
    number: u64,
    url: String,
    title: String,
}

fn fetch_metadata(gh: &impl Github) -> Result<Metadata> {
    let query = r#"query{
  repository(owner:"jaunder-org",name:"jaunder"){
    id
    milestones(first:100, states:OPEN){nodes{id number title}}
    labels(first:100){nodes{id name}}
    issueTypes(first:100){nodes{id name}}
  }
}"#;
    let v = gh.json(&graphql_args(query, &[]))?;
    let repo = v
        .pointer("/data/repository")
        .ok_or_else(|| anyhow!("metadata response missing repository"))?;
    Ok(Metadata {
        repository_id: str_field(repo, "id")?,
        milestones: repo
            .pointer("/milestones/nodes")
            .and_then(Value::as_array)
            .ok_or_else(|| anyhow!("metadata response missing milestones"))?
            .iter()
            .map(parse_milestone)
            .collect::<Result<Vec<_>>>()?,
        labels: parse_named_nodes(repo, "/labels/nodes")?,
        issue_types: parse_named_nodes(repo, "/issueTypes/nodes")?,
    })
}

fn list_milestone_issues(gh: &impl Github, milestone: u64) -> Result<Vec<IssueListing>> {
    let path = format!("repos/{OWNER}/{REPO}/issues?state=open&milestone={milestone}&per_page=100");
    let v = gh.json(&["api".into(), path])?;
    let array = v
        .as_array()
        .ok_or_else(|| anyhow!("milestone issues response is not an array"))?;
    let mut issues = Vec::new();
    for item in array {
        if item.get("pull_request").is_some() {
            continue;
        }
        let labels = item
            .get("labels")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .filter_map(|label| {
                label
                    .get("name")
                    .and_then(Value::as_str)
                    .map(str::to_string)
            })
            .collect();
        issues.push(IssueListing {
            number: u64_field(item, "number")?,
            title: str_field(item, "title")?,
            url: str_field(item, "html_url")?,
            labels,
        });
    }
    Ok(issues)
}

fn fetch_project_status(gh: &impl Github, number: u64) -> Result<ProjectStatus> {
    let query = r#"query($number:Int!){repository(owner:"jaunder-org",name:"jaunder"){
  issue(number:$number){projectItems(first:10){nodes{
    id project{number}
    fieldValueByName(name:"Status"){
      ... on ProjectV2ItemFieldSingleSelectValue{name optionId}
    }
  }}}
}}"#;
    let v = gh.json(&graphql_args(query, &[field_int("number", number)]))?;
    let nodes = v
        .pointer("/data/repository/issue/projectItems/nodes")
        .and_then(Value::as_array)
        .ok_or_else(|| anyhow!("project item response missing nodes for issue #{number}"))?;
    for node in nodes {
        if node.pointer("/project/number").and_then(Value::as_u64) == Some(BACKLOG_PROJECT_NUMBER) {
            return Ok(ProjectStatus {
                item_id: Some(str_field(node, "id")?),
                status: node
                    .pointer("/fieldValueByName/name")
                    .and_then(Value::as_str)
                    .map(str::to_string),
            });
        }
    }
    Ok(ProjectStatus {
        item_id: None,
        status: None,
    })
}

fn fetch_open_blockers(gh: &impl Github, number: u64) -> Result<Vec<u64>> {
    let path = format!("repos/{OWNER}/{REPO}/issues/{number}/dependencies/blocked_by");
    let v = gh.json(&["api".into(), path])?;
    let Some(array) = v.as_array() else {
        bail!("blocked_by response for issue #{number} is not an array");
    };
    let mut blockers = Vec::new();
    for item in array {
        if item.get("state").and_then(Value::as_str) == Some("open") {
            blockers.push(u64_field(item, "number")?);
        }
    }
    blockers.sort_unstable();
    Ok(blockers)
}

fn fetch_open_pr(gh: &impl Github, branch: &str) -> Result<Option<OpenPr>> {
    let path = format!("repos/{OWNER}/{REPO}/pulls?head={OWNER}:{branch}&state=open&per_page=1");
    let v = gh.json(&["api".into(), path])?;
    let Some(array) = v.as_array() else {
        bail!("pull request response for branch {branch} is not an array");
    };
    let Some(pr) = array.first() else {
        return Ok(None);
    };
    Ok(Some(OpenPr {
        number: u64_field(pr, "number")?,
        url: str_field(pr, "html_url")?,
        state: str_field(pr, "state")?,
    }))
}

fn create_issue(
    gh: &impl Github,
    repository_id: &str,
    title: &str,
    body: &str,
    milestone: &Milestone,
    issue_type: &NamedNode,
    labels: &[NamedNode],
) -> Result<CreatedIssueRaw> {
    let label_ids = labels
        .iter()
        .map(|label| label.id.clone())
        .collect::<Vec<_>>();
    let query = r#"mutation($repositoryId:ID!,$title:String!,$body:String!,$milestoneId:ID!,$issueTypeId:ID!,$labelIds:[ID!]){
  createIssue(input:{repositoryId:$repositoryId,title:$title,body:$body,milestoneId:$milestoneId,issueTypeId:$issueTypeId,labelIds:$labelIds}){
    issue{id number title url}
  }
}"#;
    let input = serde_json::json!({
        "query": query,
        "variables": {
            "repositoryId": repository_id,
            "title": title,
            "body": body,
            "milestoneId": milestone.id,
            "issueTypeId": issue_type.id,
            "labelIds": label_ids,
        }
    });
    let v = gh.json_stdin(
        &["api".into(), "graphql".into(), "--input".into(), "-".into()],
        &serde_json::to_string(&input)?,
    )?;
    let issue = v
        .pointer("/data/createIssue/issue")
        .ok_or_else(|| anyhow!("createIssue response missing issue"))?;
    Ok(CreatedIssueRaw {
        id: str_field(issue, "id")?,
        number: u64_field(issue, "number")?,
        title: str_field(issue, "title")?,
        url: str_field(issue, "url")?,
    })
}

fn add_to_backlog(gh: &impl Github, issue_id: &str) -> Result<String> {
    let query = r#"mutation($projectId:ID!,$contentId:ID!){
  addProjectV2ItemById(input:{projectId:$projectId,contentId:$contentId}){item{id}}
}"#;
    let v = gh.json(&graphql_args(
        query,
        &[
            field("projectId", BACKLOG_PROJECT_ID),
            field("contentId", issue_id),
        ],
    ))?;
    v.pointer("/data/addProjectV2ItemById/item/id")
        .and_then(Value::as_str)
        .map(str::to_string)
        .ok_or_else(|| anyhow!("addProjectV2ItemById response missing item id"))
}

fn set_priority(gh: &impl Github, item_id: &str, priority: Priority) -> Result<()> {
    let query = r#"mutation($projectId:ID!,$itemId:ID!,$fieldId:ID!,$optionId:String!){
  updateProjectV2ItemFieldValue(input:{projectId:$projectId,itemId:$itemId,fieldId:$fieldId,value:{singleSelectOptionId:$optionId}}){projectV2Item{id}}
}"#;
    gh.json(&graphql_args(
        query,
        &[
            field("projectId", BACKLOG_PROJECT_ID),
            field("itemId", item_id),
            field("fieldId", PRIORITY_FIELD_ID),
            field("optionId", priority.option_id()),
        ],
    ))?;
    Ok(())
}

fn fetch_issue_body(gh: &impl Github, number: u64) -> Result<String> {
    let query = r#"query($number:Int!){repository(owner:"jaunder-org",name:"jaunder"){issue(number:$number){body}}}"#;
    let v = gh.json(&graphql_args(query, &[field_int("number", number)]))?;
    v.pointer("/data/repository/issue/body")
        .and_then(Value::as_str)
        .map(str::to_string)
        .ok_or_else(|| anyhow!("issue #{number} readback response missing body"))
}

fn graphql_args(query: &str, fields: &[String]) -> Vec<String> {
    let mut args = vec![
        "api".into(),
        "graphql".into(),
        "-f".into(),
        format!("query={query}"),
    ];
    for field in fields {
        let (flag, value) = field
            .split_once(' ')
            .map_or(("-f", field.as_str()), |(flag, value)| (flag, value));
        args.push(flag.into());
        args.push(value.into());
    }
    args
}

fn field(name: &str, value: &str) -> String {
    format!("-f {name}={value}")
}

fn field_int(name: &str, value: u64) -> String {
    format!("-F {name}={value}")
}

fn parse_milestone(v: &Value) -> Result<Milestone> {
    Ok(Milestone {
        id: str_field(v, "id")?,
        number: u64_field(v, "number")?,
        title: str_field(v, "title")?,
    })
}

fn parse_named_nodes(v: &Value, pointer: &str) -> Result<Vec<NamedNode>> {
    v.pointer(pointer)
        .and_then(Value::as_array)
        .ok_or_else(|| anyhow!("metadata response missing {pointer}"))?
        .iter()
        .map(|node| {
            Ok(NamedNode {
                id: str_field(node, "id")?,
                name: str_field(node, "name")?,
            })
        })
        .collect()
}

fn str_field(v: &Value, name: &str) -> Result<String> {
    v.get(name)
        .and_then(Value::as_str)
        .map(str::to_string)
        .ok_or_else(|| anyhow!("missing string field `{name}`"))
}

fn u64_field(v: &Value, name: &str) -> Result<u64> {
    v.get(name)
        .and_then(Value::as_u64)
        .ok_or_else(|| anyhow!("missing integer field `{name}`"))
}

fn resolve_milestone(milestones: &[Milestone], arg: &str) -> Result<Milestone> {
    let matches = milestones
        .iter()
        .filter(|milestone| {
            milestone.title == arg || arg.parse::<u64>().is_ok_and(|n| milestone.number == n)
        })
        .cloned()
        .collect::<Vec<_>>();
    match matches.as_slice() {
        [milestone] => Ok(milestone.clone()),
        [] => bail!("no open milestone matches `{arg}`"),
        many => bail!(
            "milestone `{arg}` is ambiguous: {}",
            many.iter()
                .map(|m| format!("{} ({})", m.title, m.number))
                .collect::<Vec<_>>()
                .join(", ")
        ),
    }
}

fn resolve_named(nodes: &[NamedNode], arg: &str, kind: &str) -> Result<NamedNode> {
    nodes
        .iter()
        .find(|node| node.name == arg)
        .cloned()
        .ok_or_else(|| anyhow!("unknown {kind} `{arg}`"))
}

fn resolve_labels(known: &[NamedNode], requested: &[String]) -> Result<Vec<NamedNode>> {
    let mut seen = BTreeSet::new();
    let mut resolved = Vec::new();
    let mut missing = Vec::new();
    for label in requested {
        if !seen.insert(label.clone()) {
            continue;
        }
        match known.iter().find(|node| node.name == *label) {
            Some(node) => resolved.push(node.clone()),
            None => missing.push(label.clone()),
        }
    }
    if !missing.is_empty() {
        bail!("unknown label(s): {}", missing.join(", "));
    }
    Ok(resolved)
}

fn validate_title(title: &str) -> Result<()> {
    let Some((kind, subject)) = title.split_once(": ") else {
        bail!("title must match Conventional Commit form `<type>(optional-scope): summary`");
    };
    if subject.is_empty() || title.chars().count() > 72 {
        bail!("title summary must be non-empty and the full title must be ≤72 characters");
    }
    let allowed = [
        "feat", "fix", "refactor", "perf", "docs", "test", "build", "ci", "chore",
    ];
    let type_part = kind.split_once('(').map_or(kind, |(t, scope)| {
        if !scope.ends_with(')') || scope.len() <= 1 {
            ""
        } else {
            t
        }
    });
    if !allowed.contains(&type_part) {
        bail!(
            "title type `{type_part}` is not allowed; expected one of {}",
            allowed.join(", ")
        );
    }
    Ok(())
}

fn compare_body(expected: &str, actual: &str) -> BodyReadback {
    if expected == actual {
        return BodyReadback {
            readback_matches: true,
            warning: None,
        };
    }
    let likely_angle_bracket_mangling =
        expected.contains('<') && strip_angle_placeholders(expected) == actual;
    let warning = if likely_angle_bracket_mangling {
        "issue body readback differs; GitHub likely stripped angle-bracket placeholders"
    } else {
        "issue body readback differs from submitted body"
    };
    BodyReadback {
        readback_matches: false,
        warning: Some(warning.into()),
    }
}

fn strip_angle_placeholders(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut in_angle = false;
    for c in s.chars() {
        match c {
            '<' => in_angle = true,
            '>' if in_angle => in_angle = false,
            _ if !in_angle => out.push(c),
            _ => {}
        }
    }
    out
}

fn find_issue_branch(number: u64) -> Result<Option<String>> {
    let pattern = format!("issue-{number}-*");
    let output = Command::new("git")
        .args(["branch", "--list", &pattern, "--format", "%(refname:short)"])
        .output()
        .context("listing local issue branches")?;
    if !output.status.success() {
        bail!(
            "git branch lookup failed: {}",
            String::from_utf8_lossy(&output.stderr)
        );
    }
    Ok(String::from_utf8_lossy(&output.stdout)
        .lines()
        .map(str::trim)
        .find(|line| !line.is_empty())
        .map(str::to_string))
}

fn find_issue_doc(dir: &str, number: u64) -> Result<Option<String>> {
    let root = Path::new(dir);
    if !root.exists() {
        return Ok(None);
    }
    let token = format!("issue-{number}");
    let mut matches = std::fs::read_dir(root)
        .with_context(|| format!("reading {dir}"))?
        .filter_map(Result::ok)
        .filter_map(|entry| {
            let path = entry.path();
            let name = path.file_name()?.to_str()?;
            if name.contains(&token) && name.ends_with(".md") {
                Some(path.to_string_lossy().replace('\\', "/"))
            } else {
                None
            }
        })
        .collect::<Vec<_>>();
    matches.sort();
    Ok(matches.into_iter().next())
}

fn plan_progress(number: u64) -> Result<Option<PlanProgress>> {
    let Some(path) = find_issue_doc("docs/superpowers/plans", number)? else {
        return Ok(None);
    };
    let text = std::fs::read_to_string(&path).with_context(|| format!("reading {path}"))?;
    Ok(Some(count_checkboxes(&text)))
}

fn count_checkboxes(text: &str) -> PlanProgress {
    let mut done = 0;
    let mut total = 0;
    for line in text.lines() {
        if line.contains("- [ ]") {
            total += 1;
        } else if line.contains("- [x]") || line.contains("- [X]") {
            done += 1;
            total += 1;
        }
    }
    PlanProgress { done, total }
}

fn join_numbers(numbers: &[u64]) -> String {
    numbers
        .iter()
        .map(u64::to_string)
        .collect::<Vec<_>>()
        .join(", ")
}

pub fn render_human(report: &IssueReport) -> String {
    match report {
        IssueReport::Candidates(report) => render_candidates(report),
        IssueReport::Created(report) => render_created(report),
    }
}

fn render_candidates(report: &CandidateReport) -> String {
    let mut out = format!(
        "Milestone {} ({})\nCandidates: {}\nSkipped: {}\n",
        report.milestone.title,
        report.milestone.number,
        report.candidates.len(),
        report.skipped.len()
    );
    for issue in &report.candidates {
        out.push_str(&format!(
            "  #{} {} [{}]\n",
            issue.number, issue.title, issue.status
        ));
    }
    let mut reasons = BTreeMap::<&str, usize>::new();
    for skipped in &report.skipped {
        *reasons.entry(&skipped.reason).or_default() += 1;
    }
    for (reason, count) in reasons {
        out.push_str(&format!("  skipped {reason}: {count}\n"));
    }
    out
}

fn render_created(report: &CreateReport) -> String {
    let warning = report
        .body
        .warning
        .as_deref()
        .map(|warning| format!("\nBody warning: {warning}"))
        .unwrap_or_default();
    format!(
        "Created #{} {}\nProject #{} item {} Priority {}{}\n",
        report.issue.number,
        report.issue.url,
        report.project.number,
        report.project.item_id,
        report.project.priority,
        warning
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::RefCell;

    use serde_json::json;

    struct FakeGithub {
        responses: RefCell<Vec<Value>>,
        calls: RefCell<Vec<Vec<String>>>,
    }

    impl FakeGithub {
        fn new(responses: Vec<Value>) -> Self {
            Self {
                responses: RefCell::new(responses),
                calls: RefCell::new(Vec::new()),
            }
        }

        fn calls(&self) -> Vec<Vec<String>> {
            self.calls.borrow().clone()
        }
    }

    impl Github for FakeGithub {
        fn json(&self, args: &[String]) -> Result<Value> {
            self.calls.borrow_mut().push(args.to_vec());
            if self.responses.borrow().is_empty() {
                bail!("unexpected gh call: {}", args.join(" "));
            }
            Ok(self.responses.borrow_mut().remove(0))
        }

        fn json_stdin(&self, args: &[String], stdin: &str) -> Result<Value> {
            let mut call = args.to_vec();
            call.push(format!("stdin={stdin}"));
            self.calls.borrow_mut().push(call);
            if self.responses.borrow().is_empty() {
                bail!("unexpected gh call: {}", args.join(" "));
            }
            Ok(self.responses.borrow_mut().remove(0))
        }
    }

    fn metadata_response() -> Value {
        json!({
            "data": {
                "repository": {
                    "id": "R_repo",
                    "milestones": {
                        "nodes": [
                            {"id": "M_dx", "number": 3, "title": "Developer tooling & DX"}
                        ]
                    },
                    "labels": {
                        "nodes": [
                            {"id": "L_tooling", "name": "tooling"},
                            {"id": "L_dx", "name": "dx"}
                        ]
                    },
                    "issueTypes": {
                        "nodes": [
                            {"id": "IT_task", "name": "Task"}
                        ]
                    }
                }
            }
        })
    }

    fn temp_body(text: &str) -> tempfile::NamedTempFile {
        let file = tempfile::NamedTempFile::new().unwrap();
        std::fs::write(file.path(), text).unwrap();
        file
    }

    #[test]
    fn conventional_title_validation_accepts_optional_scope() {
        validate_title("feat(xtask): add issue helpers").unwrap();
        validate_title("fix: repair issue helper").unwrap();
    }

    #[test]
    fn conventional_title_validation_rejects_unknown_type() {
        let err = validate_title("feature: add issue helpers").unwrap_err();
        assert!(err.to_string().contains("not allowed"));
    }

    #[test]
    fn milestone_resolution_rejects_absent_and_ambiguous() {
        let milestones = vec![
            Milestone {
                id: "a".into(),
                number: 1,
                title: "DX".into(),
            },
            Milestone {
                id: "b".into(),
                number: 2,
                title: "1".into(),
            },
        ];
        assert!(resolve_milestone(&milestones, "missing").is_err());
        assert!(resolve_milestone(&milestones, "1").is_err());
    }

    #[test]
    fn label_resolution_names_every_unknown_label() {
        let known = vec![NamedNode {
            id: "L1".into(),
            name: "tooling".into(),
        }];
        let err =
            resolve_labels(&known, &["tooling".into(), "dx".into(), "privacy".into()]).unwrap_err();
        let text = err.to_string();
        assert!(text.contains("dx"));
        assert!(text.contains("privacy"));
    }

    #[test]
    fn body_readback_detects_angle_placeholder_stripping() {
        let report = compare_body("Keep <placeholder> here", "Keep  here");
        assert!(!report.readback_matches);
        assert!(report.warning.unwrap_or_default().contains("angle-bracket"));
    }

    #[test]
    fn checkbox_progress_counts_done_and_total() {
        assert_eq!(
            count_checkboxes("- [x] A\n- [ ] B\n- [X] C"),
            PlanProgress { done: 2, total: 3 }
        );
    }

    #[test]
    fn create_uses_explicit_metadata_sets_priority_and_reports_body() {
        let gh = FakeGithub::new(vec![
            metadata_response(),
            json!({"data":{"createIssue":{"issue":{"id":"I_1","number":1200,"title":"feat(xtask): add issue helpers","url":"https://github.com/jaunder-org/jaunder/issues/1200"}}}}),
            json!({"data":{"addProjectV2ItemById":{"item":{"id":"PVTI_1"}}}}),
            json!({"data":{"updateProjectV2ItemFieldValue":{"projectV2Item":{"id":"PVTI_1"}}}}),
            json!({"data":{"repository":{"issue":{"body":"Body with <placeholder> and [brackets] plus {braces}"}}}}),
        ]);
        let body = temp_body("Body with <placeholder> and [brackets] plus {braces}");

        let report = create(
            &gh,
            CreateInput {
                title: "feat(xtask): add issue helpers".into(),
                issue_type: "Task".into(),
                milestone: "Developer tooling & DX".into(),
                priority: Priority::P2,
                labels: vec!["tooling".into(), "dx".into()],
                body_file: body.path().into(),
            },
        )
        .unwrap();

        assert_eq!(report.issue.number, 1200);
        assert_eq!(report.project.item_id, "PVTI_1");
        assert_eq!(report.project.priority, "P2");
        assert!(report.body.readback_matches);
        let calls = gh.calls();
        assert!(calls[1].iter().any(|arg| arg == "--input"));
        assert!(calls[1].iter().any(|arg| {
            arg.starts_with("stdin=")
                && arg.contains("Body with <placeholder> and [brackets] plus {braces}")
                && arg.contains("\"labelIds\":[\"L_tooling\",\"L_dx\"]")
        }));
        assert!(calls[3].iter().any(|arg| arg == "optionId=be01f9da"));
    }

    #[test]
    fn create_stops_before_mutation_when_label_is_unknown() {
        let gh = FakeGithub::new(vec![metadata_response()]);
        let body = temp_body("Body");

        let err = create(
            &gh,
            CreateInput {
                title: "feat(xtask): add issue helpers".into(),
                issue_type: "Task".into(),
                milestone: "Developer tooling & DX".into(),
                priority: Priority::P2,
                labels: vec!["unknown".into()],
                body_file: body.path().into(),
            },
        )
        .unwrap_err();

        assert!(err.to_string().contains("unknown label"));
        assert_eq!(gh.calls().len(), 1);
    }

    #[test]
    fn absent_backlog_project_item_is_missing_project_status() {
        let gh = FakeGithub::new(vec![
            json!({"data":{"repository":{"issue":{"projectItems":{"nodes":[]}}}}}),
        ]);

        let status = fetch_project_status(&gh, 1200).unwrap();

        assert_eq!(status.item_id, None);
        assert_eq!(status.status, None);
    }
}
