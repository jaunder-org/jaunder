//! Read-only GitHub Actions evidence for PR check classification.
//!
//! A rollup's display name alone cannot establish a workflow dependency. This module
//! joins the stable GitHub Actions check-run/job identity to a workflow run, its exact
//! attempt, and immutable source at the run's head commit. Missing correlation is an
//! observation error, never evidence that a check is optional.

use std::collections::{BTreeMap, BTreeSet};

use base64::Engine;
use serde_json::Value;

use super::Subject;
use super::gh::ApiError;
use super::snapshot::{CheckEntry, CheckProvider, PrSnapshot};
use super::workflow::{Requirement, RuntimeJob, WorkflowGraph};

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
struct WorkflowKey {
    path: String,
    sha: String,
}

/// Immutable workflow source and graph evidence retained for one watch lifetime.
///
/// Dynamic run/job observations are deliberately absent: a re-run must always read
/// its current attempt, while an unchanged `(workflow path, SHA)` is safe to reuse.
#[derive(Debug, Clone)]
struct CachedWorkflow {
    source: String,
    local_sources: BTreeMap<String, String>,
    graph: WorkflowGraph,
}

/// Per-watch cache for immutable workflow evidence.
#[derive(Debug, Default)]
pub struct EvidenceCache {
    sources: BTreeMap<WorkflowKey, String>,
    workflows: BTreeMap<WorkflowKey, CachedWorkflow>,
}

/// One current-attempt Actions run, bound to the head it was observed for.
#[derive(Debug, Clone)]
pub struct WorkflowEvidence {
    pub head_sha: String,
    pub run_id: u64,
    pub attempt: u64,
    pub workflow_path: String,
    pub workflow_sha: String,
    pub jobs: Vec<RuntimeJob>,
    pub workflow_source: String,
    pub local_sources: BTreeMap<String, String>,
    pub graph: WorkflowGraph,
}

#[derive(Debug, Clone)]
pub struct ActionsEvidence {
    pub head_sha: String,
    pub runs: Vec<WorkflowEvidence>,
}

impl ActionsEvidence {
    /// Resolves a check-run identity to exactly one run attempt and runtime job.
    pub fn job_for_check(
        &self,
        check_run_id: u64,
    ) -> Result<(&WorkflowEvidence, &RuntimeJob), ApiError> {
        let matches = self
            .runs
            .iter()
            .flat_map(|run| run.jobs.iter().map(move |job| (run, job)))
            .filter(|(_, job)| job.check_run_id == Some(check_run_id))
            .collect::<Vec<_>>();
        match matches.as_slice() {
            [(run, job)] => Ok((run, job)),
            [] => Err(ApiError::Malformed(format!(
                "GitHub Actions check run {check_run_id} is absent from current-attempt jobs"
            ))),
            _ => Err(ApiError::Malformed(format!(
                "GitHub Actions check run {check_run_id} appears in multiple run attempts"
            ))),
        }
    }

    /// Classifies one Actions check against only the required contexts that the
    /// same workflow run can prove it produced. A ruleset context from another
    /// workflow has no edge in this graph and must not make this job look required.
    pub fn classify_check(
        &self,
        check_run_id: u64,
        required_contexts: &[String],
    ) -> Result<Requirement, ApiError> {
        let (run, job) = self.job_for_check(check_run_id)?;
        self.classify_job(run, job, required_contexts)
    }

    /// Resolves one same-run/display-name rollup group to its sole current-attempt
    /// job before classifying it. The rollup retains superseded rerun check IDs,
    /// whereas the REST jobs endpoint deliberately exposes only the current attempt.
    pub fn classify_current_attempt_group(
        &self,
        workflow_run_id: u64,
        name: &str,
        check_run_ids: &BTreeSet<u64>,
        required_contexts: &[String],
    ) -> Result<Requirement, ApiError> {
        let matches = self
            .runs
            .iter()
            .filter(|run| run.run_id == workflow_run_id)
            .flat_map(|run| {
                run.jobs
                    .iter()
                    .filter(move |job| {
                        job.name == name
                            && job
                                .check_run_id
                                .is_some_and(|check_run_id| check_run_ids.contains(&check_run_id))
                    })
                    .map(move |job| (run, job))
            })
            .collect::<Vec<_>>();
        match matches.as_slice() {
            [(run, job)] => self.classify_job(run, job, required_contexts),
            [] => Err(ApiError::Malformed(format!(
                "no current-attempt Actions job matches workflow run {workflow_run_id} check {name}"
            ))),
            _ => Err(ApiError::Malformed(format!(
                "multiple current-attempt Actions jobs match workflow run {workflow_run_id} check {name}"
            ))),
        }
    }

    fn classify_job(
        &self,
        run: &WorkflowEvidence,
        job: &RuntimeJob,
        required_contexts: &[String],
    ) -> Result<Requirement, ApiError> {
        let graph_targets = run
            .graph
            .required_targets(required_contexts)
            .map_err(|error| ApiError::Malformed(format!("workflow graph: {error}")))?;
        for target in &graph_targets {
            let matches = run
                .jobs
                .iter()
                .filter(|candidate| candidate.name == *target)
                .count();
            match matches {
                1 => {}
                0 => {
                    return Err(ApiError::Malformed(format!(
                        "required workflow target {target} is absent from current-attempt jobs"
                    )));
                }
                _ => {
                    return Err(ApiError::Malformed(format!(
                        "required workflow target {target} is ambiguous in current-attempt jobs"
                    )));
                }
            }
        }
        run.graph
            .classify(job, &graph_targets)
            .map_err(|error| ApiError::Malformed(format!("workflow graph: {error}")))
    }
}

/// Retrieves all Actions evidence for a snapshot's exact head.
///
/// `run` is injected so tests prove request construction and pagination without a
/// network. It is restricted to the JSON-producing, read-only `gh api` boundary.
pub fn collect_with(
    subject: &Subject,
    snapshot: &PrSnapshot,
    workflow_run_ids: &BTreeSet<u64>,
    run: impl FnMut(&[&str]) -> Result<Value, ApiError>,
) -> Result<ActionsEvidence, ApiError> {
    EvidenceCache::default().collect_with(subject, snapshot, workflow_run_ids, run)
}

impl EvidenceCache {
    /// Collects fresh dynamic run/job evidence while reusing only immutable sources
    /// and parsed graphs keyed by their exact workflow path and commit SHA.
    pub fn collect_with(
        &mut self,
        subject: &Subject,
        snapshot: &PrSnapshot,
        workflow_run_ids: &BTreeSet<u64>,
        mut run: impl FnMut(&[&str]) -> Result<Value, ApiError>,
    ) -> Result<ActionsEvidence, ApiError> {
        let runs = paged_runs(subject, &snapshot.head_sha, &mut run)?;
        let mut selected = BTreeMap::new();
        for run_value in runs {
            let run_id = required_u64(&run_value, "id")?;
            if workflow_run_ids.contains(&run_id) && selected.insert(run_id, run_value).is_some() {
                return Err(ApiError::Malformed(format!(
                    "requested Actions workflow run {run_id} appears more than once"
                )));
            }
        }
        for run_id in workflow_run_ids {
            if !selected.contains_key(run_id) {
                return Err(ApiError::Malformed(format!(
                    "requested Actions workflow run {run_id} is absent from current-head runs"
                )));
            }
        }
        let mut evidence = Vec::new();
        for (_, run_value) in selected {
            let run_id = required_u64(&run_value, "id")?;
            let workflow_sha = required_string(&run_value, "head_sha")?;
            if workflow_sha != snapshot.head_sha {
                return Err(ApiError::Malformed(format!(
                    "workflow run {run_id} belongs to {workflow_sha}, not current head {}",
                    snapshot.head_sha
                )));
            }
            let attempt = required_u64(&run_value, "run_attempt")?;
            let workflow_path = required_string(&run_value, "path")?;
            let jobs = paged_jobs(subject, run_id, attempt, &mut run)?;
            let cached = self.workflow(subject, &workflow_path, &workflow_sha, &mut run)?;
            evidence.push(WorkflowEvidence {
                head_sha: snapshot.head_sha.clone(),
                run_id,
                attempt,
                workflow_path,
                workflow_sha,
                jobs,
                workflow_source: cached.source,
                local_sources: cached.local_sources,
                graph: cached.graph,
            });
        }
        Ok(ActionsEvidence {
            head_sha: snapshot.head_sha.clone(),
            runs: evidence,
        })
    }

    fn workflow(
        &mut self,
        subject: &Subject,
        path: &str,
        sha: &str,
        run: &mut impl FnMut(&[&str]) -> Result<Value, ApiError>,
    ) -> Result<CachedWorkflow, ApiError> {
        let key = WorkflowKey {
            path: path.into(),
            sha: sha.into(),
        };
        if let Some(cached) = self.workflows.get(&key) {
            return Ok(cached.clone());
        }
        let source = self.source(subject, &key, run)?;
        let mut local_sources = BTreeMap::new();
        self.local_sources(
            subject,
            &source,
            sha,
            &mut local_sources,
            &mut BTreeSet::new(),
            run,
        )?;
        let graph = WorkflowGraph::parse_with_local_sources(&source, &local_sources)
            .map_err(|error| ApiError::Malformed(format!("workflow graph: {error}")))?;
        let cached = CachedWorkflow {
            source,
            local_sources,
            graph,
        };
        self.workflows.insert(key, cached.clone());
        Ok(cached)
    }

    fn source(
        &mut self,
        subject: &Subject,
        key: &WorkflowKey,
        run: &mut impl FnMut(&[&str]) -> Result<Value, ApiError>,
    ) -> Result<String, ApiError> {
        if let Some(source) = self.sources.get(key) {
            return Ok(source.clone());
        }
        let path = contents_path(subject, &key.path, &key.sha);
        let source = decode_contents(&run(&["api", &path])?)?;
        self.sources.insert(key.clone(), source.clone());
        Ok(source)
    }

    fn local_sources(
        &mut self,
        subject: &Subject,
        source: &str,
        sha: &str,
        sources: &mut BTreeMap<String, String>,
        visiting: &mut BTreeSet<WorkflowKey>,
        run: &mut impl FnMut(&[&str]) -> Result<Value, ApiError>,
    ) -> Result<(), ApiError> {
        for path in WorkflowGraph::local_reusable_workflow_paths(source)
            .map_err(|error| ApiError::Malformed(format!("workflow graph: {error}")))?
        {
            let key = WorkflowKey {
                path: path.clone(),
                sha: sha.into(),
            };
            if !visiting.insert(key.clone()) {
                return Err(ApiError::Malformed(format!(
                    "local reusable workflow cycle at {path}"
                )));
            }
            let nested = self.source(subject, &key, run)?;
            sources.insert(path, nested.clone());
            self.local_sources(subject, &nested, sha, sources, visiting, run)?;
            visiting.remove(&key);
        }
        Ok(())
    }
}

/// Reads the evidence needed by an Actions check. Non-Actions checks intentionally
/// have no synthetic graph identity; callers use exact ruleset membership instead.
pub fn evidence_for_check_with(
    subject: &Subject,
    snapshot: &PrSnapshot,
    check: &CheckEntry,
    run: impl FnMut(&[&str]) -> Result<Value, ApiError>,
) -> Result<Option<ActionsEvidence>, ApiError> {
    match check.provider {
        CheckProvider::GitHubActions {
            workflow_run_id, ..
        } => collect_with(subject, snapshot, &BTreeSet::from([workflow_run_id]), run).map(Some),
        CheckProvider::StatusContext | CheckProvider::OtherCheckRun => Ok(None),
    }
}

fn paged_runs(
    subject: &Subject,
    head_sha: &str,
    run: &mut impl FnMut(&[&str]) -> Result<Value, ApiError>,
) -> Result<Vec<Value>, ApiError> {
    let mut page = 1;
    let mut runs = Vec::new();
    let mut expected = None;
    loop {
        let path = format!(
            "/repos/{}/{}/actions/runs?head_sha={head_sha}&per_page=100&page={page}",
            subject.owner, subject.repo
        );
        let value = run(&["api", &path])?;
        let total = required_u64(&value, "total_count")? as usize;
        if expected
            .replace(total)
            .is_some_and(|previous| previous != total)
        {
            return Err(ApiError::Malformed(
                "Actions run page total_count changed".into(),
            ));
        }
        let page_runs = required_array(&value, "workflow_runs")?;
        if page_runs.is_empty() && runs.len() < total {
            return Err(ApiError::Malformed(
                "Actions run pagination ended before total_count".into(),
            ));
        }
        runs.extend(page_runs.iter().cloned());
        if runs.len() == total {
            return Ok(runs);
        }
        if runs.len() > total {
            return Err(ApiError::Malformed(
                "Actions run pagination exceeded total_count".into(),
            ));
        }
        page += 1;
    }
}

fn paged_jobs(
    subject: &Subject,
    run_id: u64,
    attempt: u64,
    run: &mut impl FnMut(&[&str]) -> Result<Value, ApiError>,
) -> Result<Vec<RuntimeJob>, ApiError> {
    let mut page = 1;
    let mut jobs = Vec::new();
    let mut expected = None;
    loop {
        let path = format!(
            "/repos/{}/{}/actions/runs/{run_id}/attempts/{attempt}/jobs?filter=latest&per_page=100&page={page}",
            subject.owner, subject.repo
        );
        let value = run(&["api", &path])?;
        let total = required_u64(&value, "total_count")? as usize;
        if expected
            .replace(total)
            .is_some_and(|previous| previous != total)
        {
            return Err(ApiError::Malformed(
                "Actions job page total_count changed".into(),
            ));
        }
        let page_jobs = required_array(&value, "jobs")?;
        if page_jobs.is_empty() && jobs.len() < total {
            return Err(ApiError::Malformed(
                "Actions job pagination ended before total_count".into(),
            ));
        }
        jobs.extend(
            page_jobs
                .iter()
                .map(parse_runtime_job)
                .collect::<Result<Vec<_>, _>>()?,
        );
        if jobs.len() == total {
            return Ok(jobs);
        }
        if jobs.len() > total {
            return Err(ApiError::Malformed(
                "Actions job pagination exceeded total_count".into(),
            ));
        }
        page += 1;
    }
}

fn parse_runtime_job(value: &Value) -> Result<RuntimeJob, ApiError> {
    let check_run_url = required_string(value, "check_run_url")?;
    let check_run_id = check_run_url
        .rsplit_once("/check-runs/")
        .and_then(|(_, id)| (!id.is_empty() && !id.contains('/')).then_some(id))
        .and_then(|id| id.parse().ok())
        .ok_or_else(|| {
            ApiError::Malformed("Actions job check_run_url has invalid check-run ID".into())
        })?;
    Ok(RuntimeJob {
        name: required_string(value, "name")?,
        check_run_id: Some(check_run_id),
        job_key: None,
        matrix: BTreeMap::new(),
    })
}

fn contents_path(subject: &Subject, path: &str, sha: &str) -> String {
    format!(
        "/repos/{}/{}/contents/{}?ref={sha}",
        subject.owner, subject.repo, path
    )
}

fn decode_contents(value: &Value) -> Result<String, ApiError> {
    let encoding = required_string(value, "encoding")?;
    if encoding != "base64" {
        return Err(ApiError::Malformed(format!(
            "unsupported contents encoding {encoding}"
        )));
    }
    let content = required_string(value, "content")?.replace('\n', "");
    let bytes = base64::engine::general_purpose::STANDARD
        .decode(content)
        .map_err(|error| {
            ApiError::Malformed(format!("invalid base64 workflow content: {error}"))
        })?;
    String::from_utf8(bytes)
        .map_err(|error| ApiError::Malformed(format!("workflow content is not UTF-8: {error}")))
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
        .map(str::to_string)
        .ok_or_else(|| ApiError::Malformed(format!("response omitted string {name}")))
}

fn required_u64(value: &Value, name: &str) -> Result<u64, ApiError> {
    value
        .get(name)
        .and_then(Value::as_u64)
        .ok_or_else(|| ApiError::Malformed(format!("response omitted integer {name}")))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::pr::snapshot::{CheckState, MergeStateStatus, Mergeable, PrState, QueueState};
    use crate::pr::test_support::actions_evidence;
    use crate::pr::{PrNumber, Subject};

    fn subject() -> Subject {
        Subject {
            owner: "o".into(),
            repo: "r".into(),
            number: PrNumber(1),
        }
    }

    fn snapshot() -> PrSnapshot {
        PrSnapshot {
            state: PrState::Open,
            merged_at: None,
            merge_commit: None,
            mergeable: Mergeable::Mergeable,
            merge_state_status: MergeStateStatus::Clean,
            auto_merge_armed: false,
            queue: QueueState {
                in_queue: false,
                position: None,
            },
            head_sha: "head".into(),
            head_ref: "branch".into(),
            base_sha: "base".into(),
            head_committed_at: "2026-01-01T00:00:00Z".into(),
            checks: Vec::new(),
        }
    }

    #[test]
    fn pagination_collects_current_attempt_and_pinned_sources() {
        let mut requests = Vec::new();
        let mut replies = vec![
            serde_json::json!({"total_count": 2, "workflow_runs": [{"id": 1, "head_sha": "head", "run_attempt": 2, "path": ".github/workflows/main.yml"}]}),
            serde_json::json!({"total_count": 2, "workflow_runs": [{"id": 2, "head_sha": "head", "run_attempt": 1, "path": ".github/workflows/other.yml"}]}),
            serde_json::json!({"total_count": 2, "jobs": [{"id": 1011, "check_run_url": "https://api.github.com/repos/o/r/check-runs/11", "name": "caller / nested"}]}),
            serde_json::json!({"total_count": 2, "jobs": [{"id": 1012, "check_run_url": "https://api.github.com/repos/o/r/check-runs/12", "name": "caller / sibling"}]}),
            serde_json::json!({"encoding": "base64", "content": "am9iczoKICBjYWxsZXI6CiAgICB1c2VzOiAuL2xvY2FsLnltbAogIGFnZ3JlZ2F0ZToKICAgIG5hbWU6IEFnZ3JlZ2F0ZQogICAgbmVlZHM6IGNhbGxlcgo="}),
            serde_json::json!({"encoding": "base64", "content": "am9iczoKICBib2R5OgogICAgbmFtZTogYm9keQogICAgcnVucy1vbjogdWJ1bnR1LTI0LjA0Cg=="}),
            serde_json::json!({"total_count": 1, "jobs": [{"id": 1022, "check_run_url": "https://api.github.com/repos/o/r/check-runs/22", "name": "other"}]}),
            serde_json::json!({"encoding": "base64", "content": "am9iczoKICBvdGhlcjoKICAgIG5hbWU6IG90aGVyCiAgICBydW5zLW9uOiB1YnVudHUtMjQuMDQK"}),
        ].into_iter();
        let evidence = collect_with(&subject(), &snapshot(), &BTreeSet::from([1, 2]), |args| {
            requests.push(args.join(" "));
            replies
                .next()
                .ok_or_else(|| ApiError::Malformed("unexpected request".into()))
        })
        .unwrap();
        assert_eq!(evidence.runs.len(), 2);
        assert_eq!(evidence.runs[0].attempt, 2);
        assert_eq!(
            evidence.job_for_check(11).unwrap().1.name,
            "caller / nested"
        );
        assert_eq!(evidence.job_for_check(12).unwrap().0.attempt, 2);
        assert!(requests.iter().any(|request| request.contains("page=2")));
        assert!(
            requests
                .iter()
                .any(|request| request.contains("attempts/2/jobs"))
        );
        assert!(
            requests
                .iter()
                .all(|request| request.contains(" api ") || request.starts_with("api "))
        );
        assert!(
            requests
                .iter()
                .all(|request| !request.contains("POST") && !request.contains("DELETE"))
        );
    }

    #[test]
    fn cache_reuses_immutable_sources_but_isolates_workflow_identity() {
        let root = "jobs:\n  caller:\n    uses: \"./.github/workflows/reusable.yml\"\n  aggregate:\n    name: Aggregate\n    needs: caller\n";
        let nested = "jobs:\n  body:\n    name: body\n    runs-on: ubuntu-24.04\n";
        let root_content = base64::engine::general_purpose::STANDARD.encode(root);
        let nested_content = base64::engine::general_purpose::STANDARD.encode(nested);
        let mut requests = Vec::new();
        let mut responses = vec![
            // First collection: dynamic run/jobs plus immutable root/local sources.
            serde_json::json!({"total_count": 1, "workflow_runs": [{"id": 1, "head_sha": "head", "run_attempt": 1, "path": ".github/workflows/main.yml"}]}),
            serde_json::json!({"total_count": 1, "jobs": [{"id": 1011, "check_run_url": "https://api.github.com/repos/o/r/check-runs/11", "name": "caller / body"}]}),
            serde_json::json!({"encoding": "base64", "content": root_content}),
            serde_json::json!({"encoding": "base64", "content": nested_content}),
            // Same path/SHA: only dynamic observations are fetched.
            serde_json::json!({"total_count": 1, "workflow_runs": [{"id": 1, "head_sha": "head", "run_attempt": 2, "path": ".github/workflows/main.yml"}]}),
            serde_json::json!({"total_count": 1, "jobs": [{"id": 1012, "check_run_url": "https://api.github.com/repos/o/r/check-runs/12", "name": "caller / body"}]}),
            // A different SHA cannot reuse source or graph evidence.
            serde_json::json!({"total_count": 1, "workflow_runs": [{"id": 2, "head_sha": "next", "run_attempt": 1, "path": ".github/workflows/main.yml"}]}),
            serde_json::json!({"total_count": 1, "jobs": [{"id": 1021, "check_run_url": "https://api.github.com/repos/o/r/check-runs/21", "name": "caller / body"}]}),
            serde_json::json!({"encoding": "base64", "content": root_content}),
            serde_json::json!({"encoding": "base64", "content": nested_content}),
            // A different path at the same SHA needs a distinct root graph/source,
            // while its shared pinned local reusable source remains reusable.
            serde_json::json!({"total_count": 1, "workflow_runs": [{"id": 3, "head_sha": "next", "run_attempt": 1, "path": ".github/workflows/other.yml"}]}),
            serde_json::json!({"total_count": 1, "jobs": [{"id": 1031, "check_run_url": "https://api.github.com/repos/o/r/check-runs/31", "name": "caller / body"}]}),
            serde_json::json!({"encoding": "base64", "content": root_content}),
        ]
        .into_iter();
        let mut cache = EvidenceCache::default();
        let mut call = |snapshot: &PrSnapshot, run_id| {
            cache.collect_with(&subject(), snapshot, &BTreeSet::from([run_id]), |args| {
                requests.push(args.join(" "));
                responses
                    .next()
                    .ok_or_else(|| ApiError::Malformed("unexpected request".into()))
            })
        };
        let first = call(&snapshot(), 1).unwrap();
        let second = call(&snapshot(), 1).unwrap();
        let mut changed = snapshot();
        changed.head_sha = "next".into();
        let third = call(&changed, 2).unwrap();
        let fourth = call(&changed, 3).unwrap();
        assert_eq!(first.runs[0].run_id, 1);
        assert_eq!(second.runs[0].attempt, 2);
        assert_eq!(third.runs[0].head_sha, "next");
        assert_eq!(fourth.runs[0].workflow_path, ".github/workflows/other.yml");
        let source_requests = requests
            .iter()
            .filter(|request| request.contains("/contents/"))
            .collect::<Vec<_>>();
        assert_eq!(
            source_requests.len(),
            5,
            "cache isolates root path/SHA and reuses shared local source"
        );
        assert_eq!(
            source_requests
                .iter()
                .filter(|request| request.contains("reusable.yml"))
                .count(),
            2,
            "the local source is reused by the second root path at the same SHA"
        );
        assert!(
            source_requests
                .iter()
                .any(|request| request.contains("ref=head"))
        );
        assert!(
            source_requests
                .iter()
                .any(|request| request.contains("ref=next"))
        );
    }

    #[test]
    fn collection_ignores_unselected_unsupported_current_head_workflows() {
        let source = "jobs:\n  lane:\n    name: Renamed lane\n  aggregate:\n    name: Aggregate\n    needs: lane\n";
        let content = base64::engine::general_purpose::STANDARD.encode(source);
        let mut replies = vec![
            serde_json::json!({"total_count": 2, "workflow_runs": [
                {"id": 7, "head_sha": "head", "run_attempt": 1, "path": ".github/workflows/selected.yml"},
                {"id": 8, "head_sha": "head", "run_attempt": 1, "path": ".github/workflows/unsupported.yml"}
            ]}),
            serde_json::json!({"total_count": 2, "jobs": [
                {"id": 701, "check_run_url": "https://api.github.com/repos/o/r/check-runs/71", "name": "Renamed lane"},
                {"id": 702, "check_run_url": "https://api.github.com/repos/o/r/check-runs/72", "name": "Aggregate"}
            ]}),
            serde_json::json!({"encoding": "base64", "content": content}),
        ].into_iter();
        let evidence = collect_with(&subject(), &snapshot(), &BTreeSet::from([7]), |_| {
            replies
                .next()
                .ok_or_else(|| ApiError::Malformed("unexpected request".into()))
        })
        .unwrap();
        assert_eq!(evidence.runs.len(), 1);
        assert_eq!(
            evidence.classify_check(71, &["Aggregate".into()]),
            Ok(Requirement::Transitive)
        );
    }

    #[test]
    fn production_actions_responses_correlate_check_runs_without_rest_job_ids() {
        let source = r#"
            jobs:
              renamed-ancestor:
                name: Renamed ancestor
              matrix-lane:
                name: Matrix ${{ matrix.backend }} / ${{ matrix.browser }}
                strategy:
                  matrix:
                    backend: [sqlite, postgres]
                    browser: [chromium, firefox]
              aggregate:
                name: Required aggregate
                needs: [renamed-ancestor, matrix-lane]
        "#;
        let content = base64::engine::general_purpose::STANDARD.encode(source);
        let mut replies = vec![
            serde_json::json!({"total_count": 1, "workflow_runs": [{"id": 77, "head_sha": "head", "run_attempt": 3, "path": ".github/workflows/ci.yml"}]}),
            serde_json::json!({"total_count": 6, "jobs": [
                {"id": 9001, "check_run_url": "https://api.github.com/repos/o/r/check-runs/101", "name": "Renamed ancestor"},
                {"id": 9002, "check_run_url": "https://api.github.com/repos/o/r/check-runs/102", "name": "Matrix sqlite / chromium"},
                {"id": 9003, "check_run_url": "https://api.github.com/repos/o/r/check-runs/103", "name": "Matrix sqlite / firefox"},
                {"id": 9004, "check_run_url": "https://api.github.com/repos/o/r/check-runs/104", "name": "Matrix postgres / chromium"},
                {"id": 9005, "check_run_url": "https://api.github.com/repos/o/r/check-runs/105", "name": "Matrix postgres / firefox"},
                {"id": 9006, "check_run_url": "https://api.github.com/repos/o/r/check-runs/106", "name": "Required aggregate"}
            ]}),
            serde_json::json!({"encoding": "base64", "content": content}),
        ].into_iter();
        let evidence = collect_with(&subject(), &snapshot(), &BTreeSet::from([77]), |_| {
            replies
                .next()
                .ok_or_else(|| ApiError::Malformed("unexpected request".into()))
        })
        .unwrap();
        assert_eq!(evidence.runs[0].jobs[0].check_run_id, Some(101));
        assert_eq!(evidence.runs[0].jobs[0].job_key, None);
        assert!(evidence.runs[0].jobs[0].matrix.is_empty());
        for check_run_id in [101, 102, 103, 104, 105] {
            assert_eq!(
                evidence.classify_check(check_run_id, &["Required aggregate".into()]),
                Ok(Requirement::Transitive)
            );
        }
        assert_eq!(
            evidence.classify_check(106, &["Required aggregate".into()]),
            Ok(Requirement::Direct)
        );
    }

    #[test]
    fn current_attempt_group_uses_replacement_check_run_id() {
        let evidence = actions_evidence(
            "head",
            r#"
                jobs:
                  lane:
                    name: Renamed lane
                  aggregate:
                    name: Aggregate
                    needs: lane
            "#,
            vec![("Renamed lane", 12), ("Aggregate", 13)],
        );
        assert_eq!(
            evidence.classify_current_attempt_group(
                1,
                "Renamed lane",
                &BTreeSet::from([11, 12]),
                &["Aggregate".into()],
            ),
            Ok(Requirement::Transitive)
        );
    }

    #[test]
    fn missing_or_ambiguous_runtime_required_target_fails_closed() {
        let source = r#"
            jobs:
              lane:
                name: Arbitrary lane
              aggregate:
                name: Required aggregate
                needs: lane
        "#;
        let missing = actions_evidence("head", source, vec![("Arbitrary lane", 1)]);
        let error = missing
            .classify_check(1, &["Required aggregate".into()])
            .unwrap_err();
        assert!(
            error
                .detail()
                .contains("Required aggregate is absent from current-attempt jobs")
        );

        let ambiguous = actions_evidence(
            "head",
            source,
            vec![
                ("Arbitrary lane", 1),
                ("Required aggregate", 2),
                ("Required aggregate", 3),
            ],
        );
        let error = ambiguous
            .classify_check(1, &["Required aggregate".into()])
            .unwrap_err();
        assert!(
            error
                .detail()
                .contains("Required aggregate is ambiguous in current-attempt jobs")
        );
    }

    #[test]
    fn graph_without_a_required_target_is_optional() {
        let source = r#"
            jobs:
              diagnostic:
                name: Diagnostic
        "#;
        let evidence = actions_evidence("head", source, vec![("Diagnostic", 1)]);
        assert_eq!(
            evidence.classify_check(1, &["Required elsewhere".into()]),
            Ok(Requirement::Optional)
        );
    }

    #[test]
    fn job_check_run_url_is_required_and_distinct_from_job_id() {
        let job = parse_runtime_job(&serde_json::json!({
            "id": 700,
            "name": "lane",
            "check_run_url": "https://api.github.com/repos/o/r/check-runs/42"
        }))
        .unwrap();
        assert_eq!(job.check_run_id, Some(42));
        for value in [
            serde_json::json!({"id": 700, "name": "lane"}),
            serde_json::json!({"id": 700, "name": "lane", "check_run_url": "https://api.github.com/repos/o/r/jobs/42"}),
        ] {
            assert!(matches!(
                parse_runtime_job(&value),
                Err(ApiError::Malformed(_))
            ));
        }
    }

    #[test]
    fn malformed_pagination_and_superseded_runs_fail_closed() {
        let error = collect_with(&subject(), &snapshot(), &BTreeSet::from([1]), |_| {
            Ok(serde_json::json!({"total_count": 1, "workflow_runs": []}))
        })
        .unwrap_err();
        assert!(matches!(error, ApiError::Malformed(_)));
        let error = collect_with(&subject(), &snapshot(), &BTreeSet::from([1]), |_| Ok(serde_json::json!({"total_count": 1, "workflow_runs": [{"id": 1, "head_sha": "old", "run_attempt": 1, "path": "x"}]}))).unwrap_err();
        assert!(matches!(error, ApiError::Malformed(_)));
    }

    #[test]
    fn unavailable_workflow_source_is_an_observation_error() {
        let error = collect_with(&subject(), &snapshot(), &BTreeSet::from([1]), |_| {
            Err(ApiError::NotFound)
        })
        .unwrap_err();
        assert_eq!(error, ApiError::NotFound);
    }

    #[test]
    fn non_actions_check_never_requests_evidence() {
        let check = CheckEntry {
            name: "external".into(),
            provider: CheckProvider::StatusContext,
            state: CheckState::Failure,
            details_url: None,
            started_at: None,
            completed_at: None,
        };
        assert!(matches!(
            evidence_for_check_with(&subject(), &snapshot(), &check, |_| panic!(
                "must not query"
            )),
            Ok(None)
        ));
    }
}
