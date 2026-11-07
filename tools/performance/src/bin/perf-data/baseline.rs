//! Host-only comparison and trusted Actions baseline admission; gh owns transport.
use super::{read_json, write_json_atomic};
use anyhow::{Context, Result, bail, ensure};
use performance::{
    Baseline, DatasetProfile, Provenance, RunEnvelope, RunSelection, compare_summary,
    is_canonical_selection, validate_run,
};
use serde_json::Value;
use std::{path::Path, process::Command};

const TRUSTED_REPOSITORY: &str = "jaunder-org/jaunder";
const TRUSTED_WORKFLOW: &str = "Performance";
const TRUSTED_WORKFLOW_PATH: &str = ".github/workflows/performance.yml";
const TRUSTED_JOB: &str = "performance";
const ARTIFACT_NAME: &str = "performance-result";
const BASELINE: &str = "tools/performance/baseline-v1.json";

pub(super) fn compare(path: &Path) -> Result<()> {
    let candidate: RunEnvelope = read_json(path)?;
    let baseline: Baseline = read_json(Path::new(BASELINE))?;
    ensure!(
        baseline.schema_version == performance::RESULT_SCHEMA_VERSION,
        "baseline schema is unsupported"
    );
    validate_run(&candidate).context("candidate run violates the shared contract")?;
    validate_run(&baseline.run).context("baseline violates the shared contract")?;
    ensure!(
        candidate.manifest.plan == baseline.run.manifest.plan,
        "candidate dataset plan differs from baseline"
    );
    ensure!(
        compatibility_fingerprint(&candidate)? == compatibility_fingerprint(&baseline.run)?,
        "candidate compatibility keys differ from baseline"
    );
    println!("advisory only");
    for workload in &candidate.workloads {
        let baseline_workload = baseline
            .run
            .workloads
            .iter()
            .find(|value| value.key == workload.key)
            .context("candidate workload is absent from baseline")?;
        let comparison = compare_summary(&baseline_workload.summary, &workload.summary);
        println!(
            "{:?}/{:?}/{:?}: baseline median={}us p95={}us spread={}..{}us; \
             candidate median={}us p95={}us spread={}..{}us; advisory median={:?} p95={:?}",
            workload.key.workload,
            workload.key.backend,
            workload.key.browser,
            baseline_workload.summary.median_us,
            baseline_workload.summary.p95_us,
            baseline_workload.summary.minimum_us,
            baseline_workload.summary.maximum_us,
            workload.summary.median_us,
            workload.summary.p95_us,
            workload.summary.minimum_us,
            workload.summary.maximum_us,
            comparison.median,
            comparison.p95,
        );
    }
    Ok(())
}

pub(super) fn import(run_id: u64) -> Result<()> {
    // Download success is not admission: authenticate the remote tuple first,
    // then validate the downloaded artifact against that same run and attempt.
    let run_path = format!("/repos/{TRUSTED_REPOSITORY}/actions/runs/{run_id}");
    let metadata = github_json(&run_path)?;
    let jobs = github_json(&format!("{run_path}/jobs"))?;
    let artifacts = github_json(&format!("{run_path}/artifacts"))?;
    verify_actions_metadata(run_id, &metadata, &jobs, &artifacts)?;
    let temp = tempfile::tempdir().context("creating artifact staging directory")?;
    let download = Command::new("gh")
        .args([
            "run",
            "download",
            &run_id.to_string(),
            "--repo",
            TRUSTED_REPOSITORY,
            "--name",
            ARTIFACT_NAME,
            "--dir",
        ])
        .arg(temp.path())
        .output()
        .context("starting GitHub Actions artifact download")?;
    ensure!(
        download.status.success(),
        "GitHub Actions artifact download failed: {}",
        String::from_utf8_lossy(&download.stderr)
    );
    let candidate = temp.path().join(performance::RESULT_FILENAME);
    ensure!(
        candidate.is_file(),
        "run artifact lacks {}",
        performance::RESULT_FILENAME
    );
    let run: RunEnvelope = read_json(&candidate)?;
    verify_import_identity(
        run_id,
        &metadata,
        run.manifest.plan.profile,
        &run.selection,
        &run.provenance,
    )?;
    validate_run(&run).context("imported run violates the shared contract")?;
    if Path::new(BASELINE).is_file() {
        let baseline: Baseline = read_json(Path::new(BASELINE))?;
        ensure!(
            baseline.schema_version == performance::RESULT_SCHEMA_VERSION,
            "existing baseline schema is unsupported"
        );
        validate_run(&baseline.run).context("existing baseline violates the shared contract")?;
        ensure!(
            compatibility_fingerprint(&run)? == compatibility_fingerprint(&baseline.run)?,
            "imported run compatibility keys differ from the committed baseline"
        );
    }
    write_json_atomic(
        Path::new(BASELINE),
        &Baseline {
            schema_version: performance::RESULT_SCHEMA_VERSION,
            run,
        },
    )?;
    println!("imported canonical Actions run {run_id}");
    Ok(())
}

fn github_json(path: &str) -> Result<Value> {
    let output = Command::new("gh")
        .args(["api", path])
        .output()
        .context("starting GitHub API request")?;
    ensure!(
        output.status.success(),
        "GitHub API request failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let body: Value =
        serde_json::from_slice(&output.stdout).context("parsing GitHub API response")?;
    ensure!(
        body["errors"].as_array().is_none_or(Vec::is_empty),
        "GitHub API response contains errors"
    );
    Ok(body)
}

// These predicates preserve the existing xtask performance import policy.
fn verify_actions_metadata(
    run_id: u64,
    metadata: &Value,
    jobs: &Value,
    artifacts: &Value,
) -> Result<()> {
    ensure!(
        metadata["id"].as_u64() == Some(run_id),
        "Actions run ID mismatch"
    );
    ensure!(
        metadata["repository"]["full_name"].as_str() == Some(TRUSTED_REPOSITORY)
            && metadata["head_repository"]["full_name"].as_str() == Some(TRUSTED_REPOSITORY),
        "Actions run belongs to another repository"
    );
    ensure!(
        metadata["path"].as_str() == Some(TRUSTED_WORKFLOW_PATH)
            && metadata["name"].as_str() == Some(TRUSTED_WORKFLOW),
        "Actions run did not use the approved performance workflow"
    );
    ensure!(
        matches!(
            metadata["event"].as_str(),
            Some("workflow_dispatch" | "schedule")
        ),
        "pull-request and unapproved workflow events cannot become a baseline"
    );
    ensure!(
        metadata["status"].as_str() == Some("completed")
            && metadata["conclusion"].as_str() == Some("success"),
        "baseline import requires a completed successful run"
    );
    ensure!(
        metadata["head_branch"].as_str() == Some("main"),
        "baseline import requires main"
    );
    let head_sha = metadata["head_sha"]
        .as_str()
        .filter(|value| !value.trim().is_empty())
        .context("Actions run has no head SHA")?;
    let attempt = metadata["run_attempt"]
        .as_u64()
        .filter(|value| *value != 0)
        .context("Actions run has no attempt")?;
    ensure!(
        jobs["jobs"]
            .as_array()
            .is_some_and(|jobs| jobs.iter().any(|job| {
                job["name"].as_str() == Some(TRUSTED_JOB)
                    && job["status"].as_str() == Some("completed")
                    && job["conclusion"].as_str() == Some("success")
                    && job["head_sha"].as_str() == Some(head_sha)
                    && job["run_attempt"].as_u64() == Some(attempt)
            })),
        "approved performance job is absent or unsuccessful"
    );
    ensure!(
        artifacts["artifacts"]
            .as_array()
            .is_some_and(|artifacts| artifacts.iter().any(|artifact| {
                artifact["name"].as_str() == Some(ARTIFACT_NAME)
                    && artifact["expired"].as_bool() == Some(false)
                    && artifact["id"].as_u64().is_some_and(|id| id != 0)
                    && artifact["size_in_bytes"]
                        .as_u64()
                        .is_some_and(|size| size != 0)
                    && artifact["workflow_run"]["id"].as_u64() == Some(run_id)
                    && artifact["workflow_run"]["head_branch"].as_str() == Some("main")
                    && artifact["workflow_run"]["head_sha"].as_str() == Some(head_sha)
            })),
        "successful canonical run lacks the complete performance-result artifact"
    );
    Ok(())
}

fn verify_import_identity(
    run_id: u64,
    metadata: &Value,
    profile: DatasetProfile,
    selection: &RunSelection,
    provenance: &Provenance,
) -> Result<()> {
    ensure!(
        profile == DatasetProfile::Medium,
        "canonical baseline requires the medium profile"
    );
    ensure!(
        is_canonical_selection(selection),
        "baseline import rejects noncanonical selection"
    );
    let Provenance::Github(provenance) = provenance else {
        bail!("baseline import rejects local provenance");
    };
    ensure!(
        provenance.repository == TRUSTED_REPOSITORY
            && provenance.workflow == TRUSTED_WORKFLOW
            && provenance.job == TRUSTED_JOB
            && provenance.reference == "refs/heads/main"
            && provenance.run_id == run_id
            && metadata["head_sha"].as_str() == Some(provenance.head_sha.as_str())
            && metadata["run_attempt"].as_u64() == Some(u64::from(provenance.attempt)),
        "artifact provenance does not match the approved Actions run"
    );
    Ok(())
}

fn compatibility_fingerprint(run: &RunEnvelope) -> Result<Vec<String>> {
    let mut keys = run
        .workloads
        .iter()
        .map(|workload| serde_json::to_string(&workload.key))
        .collect::<std::result::Result<Vec<_>, _>>()?;
    keys.sort();
    Ok(keys)
}
