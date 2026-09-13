//! Host orchestration for fresh, versioned performance evidence.
//!
//! Nix owns execution; this module owns selector normalization, trusted
//! provenance, strict contract assembly, and atomic host publication.
use anyhow::{Context, Result, bail, ensure};
use performance::{
    Backend, Baseline, Browser, BrowserDiagnostics, CountOverrides, DatasetProfile, Fragment,
    FragmentEnvelope, GitHubProvenance, NamedDerivationIdentity, Provenance, RESULT_FILENAME,
    RunEvidence, RunSelection, assemble_run, compare_summary, is_canonical_selection, validate_run,
};
use serde::Deserialize;
use serde_json::Value;
use std::{
    collections::BTreeSet,
    env, fs,
    io::{Read, Write},
    path::{Path, PathBuf},
    process::Command,
    time::{Instant, SystemTime, UNIX_EPOCH},
};

use crate::{
    cli::{
        PerformanceBackend, PerformanceBrowser, PerformanceCommand, PerformanceProfile,
        PerformanceRunArgs,
    },
    git, lifecycle,
    result::{CommandResult, StepResult},
};
const TRUSTED_REPOSITORY: &str = "jaunder-org/jaunder";
const TRUSTED_WORKFLOW: &str = "Performance";
const TRUSTED_WORKFLOW_PATH: &str = ".github/workflows/performance.yml";
const TRUSTED_JOB: &str = "performance";
const ARTIFACT_NAME: &str = "performance-result";

const ROOT: &str = ".xtask/performance";
const BASELINE: &str = "tools/performance/baseline-v1.json";

pub fn run(command: PerformanceCommand) -> Result<CommandResult> {
    let start = Instant::now();
    let name = match &command {
        PerformanceCommand::Small(_)
        | PerformanceCommand::Medium(_)
        | PerformanceCommand::Large(_) => "perf",
        PerformanceCommand::Compare { .. } => "perf-compare",
        PerformanceCommand::ImportBaseline { .. } => "perf-import-baseline",
    };
    let outcome = match command {
        PerformanceCommand::Small(args) => run_fresh(PerformanceProfile::Small, args),
        PerformanceCommand::Medium(args) => run_fresh(PerformanceProfile::Medium, args),
        PerformanceCommand::Large(args) => run_fresh(PerformanceProfile::Large, args),
        PerformanceCommand::Compare { result } => compare(&result),
        PerformanceCommand::ImportBaseline { run_id } => import_baseline(run_id),
    };
    Ok(finish_command(name, start, outcome))
}

fn finish_command(name: &str, start: Instant, outcome: Result<CommandResult>) -> CommandResult {
    let mut result = outcome.unwrap_or_else(|error| {
        let mut failed = CommandResult::new(name);
        failed.push(StepResult::fail(name).detail(format!("{error:#}")));
        failed
    });
    lifecycle::finalize(&mut result, start);
    result
}

fn run_fresh(profile: PerformanceProfile, args: PerformanceRunArgs) -> Result<CommandResult> {
    let PerformanceRunArgs {
        backend: backends,
        browser: browsers,
        posts,
        authors,
        revisions,
        storage_only,
        browser_only,
    } = args;
    let start = Instant::now();
    let mut result = CommandResult::new("perf");
    let selection = normalize(
        backends,
        browsers,
        CountOverrides {
            posts,
            authors,
            revisions,
        },
        storage_only,
        browser_only,
    )?;
    performance::plan(to_dataset_profile(profile), selection.count_overrides)
        .context("invalid performance count overrides")?;
    let nonce = freshness_nonce()?;
    let profile = profile_name(profile);
    let root = PathBuf::from(ROOT).join(&nonce);
    let fragments_root = root.join("fragments");
    fs::create_dir_all(&fragments_root).context("creating performance result root")?;

    let mut fragments = Vec::new();
    let mut identities = Vec::new();
    for backend in &selection.backends {
        if selection.storage {
            let (producer, identity) = build_fragment(
                "storage",
                *backend,
                None,
                profile,
                &nonce,
                &selection.count_overrides,
            )?;
            retain_producer_artifacts(&producer, &identity, &root)?;
            validate_producer_status(&producer)?;
            let path = producer_fragment_path(&producer, *backend, None)?;
            retain_fragment(&path, &root)?;
            fragments.push(read_fragment(&path)?);
            identities.push(identity);
        }
        if selection.browser {
            for browser in &selection.browsers {
                let (producer, identity) = build_fragment(
                    "browser",
                    *backend,
                    Some(*browser),
                    profile,
                    &nonce,
                    &selection.count_overrides,
                )?;
                retain_producer_artifacts(&producer, &identity, &root)?;
                validate_producer_status(&producer)?;
                let path = producer_fragment_path(&producer, *backend, Some(*browser))?;
                retain_fragment(&path, &root)?;
                let fragment = read_fragment(&path)?;
                let Fragment::Browser(browser_fragment) = &fragment.fragment else {
                    bail!("browser producer emitted a non-browser fragment");
                };
                validate_diagnostic_artifacts(&producer, &browser_fragment.diagnostics)?;
                fragments.push(fragment);
                identities.push(identity);
            }
        }
    }
    identities.sort_by(|a, b| a.name.cmp(&b.name));
    let canonical = is_canonical_selection(&selection);
    let provenance = trusted_provenance()?;
    let run = assemble_run(
        selection.clone(),
        provenance,
        RunEvidence {
            measured_at_unix_ms: measurement_timestamp()?,
            freshness_nonce: nonce.clone(),
            producer_derivation_identities: identities,
        },
        &fragments,
    )
    .context("assembling strict performance result")?;
    let artifact = root.join(RESULT_FILENAME);
    write_json_atomic(&artifact, &run)?;
    // Stable export for CI artifact upload/import; the nonce-owned copy remains
    // the authoritative local run path.
    write_json_atomic(&Path::new(ROOT).join(RESULT_FILENAME), &run)?;
    let detail = format!(
        "{} workload(s) at {}; {}",
        run.workloads.len(),
        artifact.display(),
        if canonical {
            "canonical selection"
        } else {
            "NONCANONICAL selector override; cannot become baseline"
        },
    );
    result.push(
        StepResult::ok("perf")
            .detail(detail)
            .with_duration(start.elapsed()),
    );
    Ok(result)
}

fn compare(path: &Path) -> Result<CommandResult> {
    let start = Instant::now();
    let mut result = CommandResult::new("perf-compare");
    let candidate: performance::RunEnvelope = read_json(path)?;
    let baseline: Baseline = read_json(Path::new(BASELINE))?;
    ensure!(
        baseline.schema_version == performance::RESULT_SCHEMA_VERSION,
        "baseline schema is unsupported"
    );
    validate_run(&candidate).context("candidate run violates the shared contract")?;
    validate_run(&baseline.run).context("baseline violates the shared contract")?;
    ensure!(
        compatible_dataset_plans(&candidate.manifest.plan, &baseline.run.manifest.plan),
        "candidate dataset plan differs from baseline"
    );
    ensure!(
        compatibility_fingerprint(&candidate)? == compatibility_fingerprint(&baseline.run)?,
        "candidate compatibility keys differ from baseline"
    );
    let mut notices = Vec::with_capacity(candidate.workloads.len());
    for workload in &candidate.workloads {
        let baseline_workload = baseline
            .run
            .workloads
            .iter()
            .find(|value| value.key == workload.key)
            .context("candidate workload is absent from baseline")?;
        let comparison = compare_summary(&baseline_workload.summary, &workload.summary);
        notices.push(format!(
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
        ));
    }
    result.push(
        StepResult::ok("perf-compare")
            .detail(format!("advisory only; {}", notices.join("; ")))
            .with_duration(start.elapsed()),
    );
    Ok(result)
}

fn import_baseline(run_id: u64) -> Result<CommandResult> {
    let mut result = CommandResult::new("perf-import-baseline");
    // The workflow has no write credential. This explicit operator action is
    // the only baseline mutation and authenticates every remote identity first.
    let run_path = format!("/repos/{TRUSTED_REPOSITORY}/actions/runs/{run_id}");
    let metadata = github_json(&run_path)?;
    let artifacts_path = format!("/repos/{TRUSTED_REPOSITORY}/actions/runs/{run_id}/artifacts");
    let jobs_path = format!("/repos/{TRUSTED_REPOSITORY}/actions/runs/{run_id}/jobs");
    let jobs = github_json(&jobs_path)?;
    let artifacts = github_json(&artifacts_path)?;
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
    let candidate = temp.path().join(RESULT_FILENAME);
    ensure!(candidate.is_file(), "run artifact lacks {RESULT_FILENAME}");
    let run: performance::RunEnvelope = read_json(&candidate)?;
    verify_imported_run(run_id, &metadata, &run)?;

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
    result.push(
        StepResult::ok("perf-import-baseline")
            .detail(format!("imported canonical Actions run {run_id}")),
    );
    Ok(result)
}

fn github_json(path: &str) -> Result<Value> {
    crate::pr::gh::run_gh(&["api", path]).map_err(|error| anyhow::anyhow!(error.detail()))
}

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

fn verify_imported_run(
    run_id: u64,
    metadata: &Value,
    run: &performance::RunEnvelope,
) -> Result<()> {
    verify_import_identity(
        run_id,
        metadata,
        run.manifest.plan.profile,
        &run.selection,
        &run.provenance,
    )?;
    validate_run(run).context("imported run violates the shared contract")
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

fn compatible_dataset_plans(
    candidate: &performance::DatasetPlan,
    baseline: &performance::DatasetPlan,
) -> bool {
    candidate == baseline
}

fn compatibility_fingerprint(run: &performance::RunEnvelope) -> Result<Vec<String>> {
    let mut keys = run
        .workloads
        .iter()
        .map(|workload| serde_json::to_string(&workload.key))
        .collect::<std::result::Result<Vec<_>, _>>()?;
    keys.sort();
    Ok(keys)
}

fn build_fragment(
    kind: &str,
    backend: Backend,
    browser: Option<Browser>,
    profile: &str,
    nonce: &str,
    overrides: &CountOverrides,
) -> Result<(PathBuf, NamedDerivationIdentity)> {
    let backend = backend_name(backend);
    let browser_name = browser.map(browser_name);
    let nix_string = |value: &str| serde_json::to_string(value).expect("strings serialize");
    let browser_argument = browser_name.map_or_else(|| "null".to_owned(), nix_string);
    let count_argument =
        |value: Option<u64>| value.map_or_else(|| "null".to_owned(), |value| value.to_string());
    let expression = format!(
        "let flake = builtins.getFlake (toString ./.); in \
         flake.performanceLib.${{builtins.currentSystem}}.mkPerformanceProducer {{ \
         freshnessNonce = {}; profile = {}; backend = {}; producerKind = {}; browser = {}; \
         posts = {}; authors = {}; revisions = {}; }}",
        nix_string(nonce),
        nix_string(profile),
        nix_string(backend),
        nix_string(kind),
        browser_argument,
        count_argument(overrides.posts),
        count_argument(overrides.authors),
        count_argument(overrides.revisions),
    );
    let label = format!(
        "performance-{kind}-{backend}{}",
        browser_name.map_or(String::new(), |name| format!("-{name}"))
    );
    let output = Command::new("nix")
        .args([
            "build",
            "--impure",
            "--no-link",
            "--print-out-paths",
            "--expr",
            &expression,
        ])
        .output()
        .with_context(|| format!("building {label}"))?;
    ensure!(
        output.status.success(),
        "{label} failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let root = String::from_utf8(output.stdout).context("Nix emitted non-UTF8 output path")?;
    let root = PathBuf::from(root.trim());
    ensure!(root.is_dir(), "{label} did not emit a producer directory");
    Ok((
        root.clone(),
        NamedDerivationIdentity {
            name: label,
            identity: root.display().to_string(),
        },
    ))
}

fn normalize(
    backends: Vec<PerformanceBackend>,
    browsers: Vec<PerformanceBrowser>,
    overrides: CountOverrides,
    storage_only: bool,
    browser_only: bool,
) -> Result<RunSelection> {
    ensure!(
        !(storage_only && browser_only),
        "--storage-only conflicts with --browser-only"
    );
    ensure!(
        !storage_only || browsers.is_empty(),
        "--browser conflicts with --storage-only"
    );
    let storage = !browser_only;
    let browser = !storage_only;
    ensure!(storage || browser, "empty producer selection");
    let mut backend_set: BTreeSet<_> = backends.into_iter().map(to_backend).collect();
    if backend_set.is_empty() {
        backend_set.extend([Backend::Sqlite, Backend::Postgres]);
    }
    let mut browser_set: BTreeSet<_> = browsers.into_iter().map(to_browser).collect();
    if browser && browser_set.is_empty() {
        browser_set.insert(Browser::Chromium);
    }
    Ok(RunSelection {
        backends: backend_set.into_iter().collect(),
        browsers: browser_set.into_iter().collect(),
        count_overrides: overrides,
        storage,
        browser,
    })
}

fn trusted_provenance() -> Result<Provenance> {
    let repository = env::var("GITHUB_REPOSITORY").ok();
    if let Some(repository) = repository {
        let workflow = required_env("GITHUB_WORKFLOW")?;
        let job = required_env("GITHUB_JOB")?;
        let reference = required_env("GITHUB_REF")?;
        let head_sha = required_env("GITHUB_SHA")?;
        let run_id = required_env("GITHUB_RUN_ID")?
            .parse()
            .context("parsing GITHUB_RUN_ID")?;
        let attempt = required_env("GITHUB_RUN_ATTEMPT")?
            .parse()
            .context("parsing GITHUB_RUN_ATTEMPT")?;
        return Ok(Provenance::Github(GitHubProvenance {
            repository,
            workflow,
            job,
            reference,
            head_sha,
            run_id,
            attempt,
        }));
    }
    let git_commit =
        git::head_sha(Path::new("."))?.context("local performance run requires a Git commit")?;
    Ok(Provenance::Local(performance::LocalProvenance {
        git_commit,
    }))
}

fn required_env(name: &str) -> Result<String> {
    env::var(name).with_context(|| format!("CI performance provenance requires {name}"))
}
fn validate_diagnostic_artifacts(
    producer_root: &Path,
    diagnostics: &BrowserDiagnostics,
) -> Result<()> {
    let fragments_root = fs::canonicalize(producer_root.join("performance/fragments"))
        .context("resolving producer fragment directory")?;
    for reference in diagnostics
        .navigation_artifacts
        .iter()
        .chain(&diagnostics.trace_artifacts)
        .chain(std::iter::once(&diagnostics.otel_trace_artifact))
    {
        let path = Path::new(reference);
        ensure!(
            !reference.is_empty()
                && path
                    .components()
                    .all(|component| matches!(component, std::path::Component::Normal(_))),
            "browser diagnostic artifact must be a relative fragment path: {reference}"
        );
        let resolved = fs::canonicalize(fragments_root.join(path))
            .with_context(|| format!("resolving browser diagnostic artifact {reference}"))?;
        ensure!(
            resolved.is_file() && resolved.starts_with(&fragments_root),
            "browser diagnostic artifact is outside producer fragments: {reference}"
        );
    }
    Ok(())
}

fn read_fragment(path: &Path) -> Result<FragmentEnvelope> {
    read_json(path)
}
fn read_json<T: serde::de::DeserializeOwned>(path: &Path) -> Result<T> {
    serde_json::from_slice(&fs::read(path).with_context(|| format!("reading {}", path.display()))?)
        .with_context(|| format!("parsing {}", path.display()))
}
fn write_json_atomic(path: &Path, value: &impl serde::Serialize) -> Result<()> {
    let parent = path.parent().context("result path has no parent")?;
    fs::create_dir_all(parent)?;
    let temporary = path.with_extension("tmp");
    {
        let mut file = fs::File::create(&temporary)?;
        serde_json::to_writer_pretty(&mut file, value)?;
        file.write_all(b"\n")?;
        file.sync_all()?;
    }
    fs::rename(temporary, path)?;
    Ok(())
}
#[derive(Deserialize)]
struct ProducerStatus {
    schema_version: u64,
    ok: bool,
    detail: String,
}

fn producer_fragment_path(
    producer_root: &Path,
    backend: Backend,
    browser: Option<Browser>,
) -> Result<PathBuf> {
    let filename = match browser {
        Some(browser) => performance::browser_fragment_filename(backend, browser),
        None => performance::storage_fragment_filename(backend),
    };
    let path = producer_root.join("performance/fragments").join(filename);
    ensure!(
        path.is_file(),
        "producer omitted fragment {}",
        path.display()
    );
    Ok(path)
}

fn validate_producer_status(producer_root: &Path) -> Result<()> {
    let status: ProducerStatus =
        read_json(&producer_root.join("performance/producer-status-v1.json"))?;
    ensure!(
        status.schema_version == 1,
        "producer status has unsupported schema version"
    );
    ensure!(status.ok, "producer reported failure: {}", status.detail);
    Ok(())
}

fn retain_producer_artifacts(
    producer_root: &Path,
    identity: &NamedDerivationIdentity,
    run_root: &Path,
) -> Result<()> {
    let performance_root = producer_root.join("performance");
    ensure!(
        performance_root.is_dir(),
        "producer output lacks performance evidence directory"
    );
    copy_tree(
        &performance_root,
        &run_root.join("producers").join(&identity.name),
    )?;
    Ok(())
}

fn retain_fragment(fragment: &Path, run_root: &Path) -> Result<()> {
    let filename = fragment
        .file_name()
        .context("producer fragment has no name")?;
    fs::copy(fragment, run_root.join("fragments").join(filename))
        .with_context(|| format!("retaining producer fragment {}", fragment.display()))?;
    Ok(())
}

fn copy_tree(source: &Path, destination: &Path) -> Result<()> {
    fs::create_dir_all(destination)?;
    for entry in fs::read_dir(source)? {
        let entry = entry?;
        let target = destination.join(entry.file_name());
        if entry.file_type()?.is_dir() {
            copy_tree(&entry.path(), &target)?;
        } else {
            fs::copy(entry.path(), target)?;
        }
    }
    Ok(())
}

fn freshness_nonce() -> Result<String> {
    let mut bytes = [0_u8; 32];
    fs::File::open("/dev/urandom")?.read_exact(&mut bytes)?;
    Ok(bytes.iter().map(|value| format!("{value:02x}")).collect())
}
fn measurement_timestamp() -> Result<u64> {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .context("system clock predates the Unix epoch")?
        .as_millis()
        .try_into()
        .context("measurement timestamp exceeds u64")
}
fn to_backend(value: PerformanceBackend) -> Backend {
    match value {
        PerformanceBackend::Sqlite => Backend::Sqlite,
        PerformanceBackend::Postgres => Backend::Postgres,
    }
}
fn to_browser(value: PerformanceBrowser) -> Browser {
    match value {
        PerformanceBrowser::Chromium => Browser::Chromium,
        PerformanceBrowser::Firefox => Browser::Firefox,
    }
}
fn to_dataset_profile(value: PerformanceProfile) -> DatasetProfile {
    match value {
        PerformanceProfile::Small => DatasetProfile::Small,
        PerformanceProfile::Medium => DatasetProfile::Medium,
        PerformanceProfile::Large => DatasetProfile::Large,
    }
}

fn profile_name(value: PerformanceProfile) -> &'static str {
    match value {
        PerformanceProfile::Small => "small",
        PerformanceProfile::Medium => "medium",
        PerformanceProfile::Large => "large",
    }
}
fn backend_name(value: Backend) -> &'static str {
    match value {
        Backend::Sqlite => "sqlite",
        Backend::Postgres => "postgres",
    }
}
fn browser_name(value: Browser) -> &'static str {
    match value {
        Browser::Chromium => "chromium",
        Browser::Firefox => "firefox",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn selector_defaults_are_canonical_and_deduplicated() {
        let selection = normalize(
            vec![
                PerformanceBackend::Postgres,
                PerformanceBackend::Sqlite,
                PerformanceBackend::Postgres,
            ],
            vec![PerformanceBrowser::Chromium, PerformanceBrowser::Chromium],
            CountOverrides {
                posts: None,
                authors: None,
                revisions: None,
            },
            false,
            false,
        )
        .expect("selectors normalize");
        assert!(is_canonical_selection(&selection));
        assert_eq!(selection.backends, vec![Backend::Sqlite, Backend::Postgres]);
    }

    #[test]
    fn selector_exclusion_is_visible_and_noncanonical() {
        let selection = normalize(
            vec![],
            vec![],
            CountOverrides {
                posts: None,
                authors: None,
                revisions: None,
            },
            true,
            false,
        )
        .expect("storage-only selection");
        assert!(selection.storage);
        assert!(!selection.browser);
        assert!(selection.browsers.is_empty());
        assert!(!is_canonical_selection(&selection));
    }

    #[test]
    fn count_override_is_noncanonical_selector_identity() {
        let selection = normalize(
            vec![],
            vec![],
            CountOverrides {
                posts: Some(1),
                authors: None,
                revisions: None,
            },
            false,
            false,
        )
        .expect("selector with count override");

        assert!(!is_canonical_selection(&selection));
        assert_eq!(selection.count_overrides.posts, Some(1));
    }

    #[test]
    fn comparison_requires_identical_dataset_plans() {
        let canonical = performance::canonical_plan(DatasetProfile::Small);
        let overridden = performance::plan(
            DatasetProfile::Small,
            CountOverrides {
                posts: Some(200),
                authors: None,
                revisions: Some(600),
            },
        )
        .expect("valid overridden plan");

        assert!(!compatible_dataset_plans(&overridden, &canonical));
        assert!(compatible_dataset_plans(&canonical, &canonical));
    }

    #[test]
    fn browser_diagnostic_artifacts_are_confined_to_producer_fragments() {
        let producer = tempfile::tempdir().expect("producer directory");
        let fragments = producer.path().join("performance/fragments");
        fs::create_dir_all(&fragments).expect("fragment directory");
        fs::write(fragments.join("navigation.json"), "").expect("navigation artifact");
        fs::write(fragments.join("trace.zip"), "").expect("trace artifact");
        fs::write(fragments.join("otel-traces.jsonl"), "").expect("OTel trace artifact");
        let diagnostics = BrowserDiagnostics {
            navigation_artifacts: vec!["navigation.json".into()],
            trace_artifacts: vec!["trace.zip".into()],
            otel_trace_artifact: "otel-traces.jsonl".into(),
        };

        validate_diagnostic_artifacts(producer.path(), &diagnostics)
            .expect("contained diagnostic artifacts");

        let traversal = BrowserDiagnostics {
            navigation_artifacts: vec!["../navigation.json".into()],
            trace_artifacts: vec!["trace.zip".into()],
            otel_trace_artifact: "otel-traces.jsonl".into(),
        };
        assert!(validate_diagnostic_artifacts(producer.path(), &traversal).is_err());

        let missing = BrowserDiagnostics {
            navigation_artifacts: vec!["navigation.json".into()],
            trace_artifacts: vec!["missing.zip".into()],
            otel_trace_artifact: "otel-traces.jsonl".into(),
        };
        assert!(validate_diagnostic_artifacts(producer.path(), &missing).is_err());
    }

    #[test]
    fn advisory_regression_never_changes_the_comparison_verdict() {
        let baseline = performance::Summary {
            sample_count: 1,
            minimum_us: 1,
            maximum_us: 100,
            mean_us: 100,
            median_us: 100,
            p95_us: 100,
        };
        let candidate = performance::Summary {
            sample_count: 1,
            minimum_us: 1,
            maximum_us: 120,
            mean_us: 120,
            median_us: 120,
            p95_us: 120,
        };
        let comparison = compare_summary(&baseline, &candidate);
        assert_eq!(
            comparison.median,
            performance::Regression::AtLeastTwentyPercent
        );
        assert_eq!(
            comparison.p95,
            performance::Regression::AtLeastTwentyPercent
        );
    }

    fn approved_actions_metadata() -> (Value, Value, Value) {
        let run = serde_json::json!({
            "id": 1434,
            "repository": { "full_name": TRUSTED_REPOSITORY },
            "head_repository": { "full_name": TRUSTED_REPOSITORY },
            "path": TRUSTED_WORKFLOW_PATH,
            "name": TRUSTED_WORKFLOW,
            "event": "workflow_dispatch",
            "status": "completed",
            "conclusion": "success",
            "head_branch": "main",
            "head_sha": "abc123",
            "run_attempt": 2
        });
        let jobs = serde_json::json!({
            "jobs": [{
                "name": TRUSTED_JOB,
                "status": "completed",
                "conclusion": "success",
                "head_sha": "abc123",
                "run_attempt": 2
            }]
        });
        let artifacts = serde_json::json!({
            "artifacts": [{
                "id": 99,
                "name": ARTIFACT_NAME,
                "expired": false,
                "size_in_bytes": 100,
                "workflow_run": {
                    "id": 1434,
                    "head_branch": "main",
                    "head_sha": "abc123"
                }
            }]
        });
        (run, jobs, artifacts)
    }

    #[test]
    fn approved_successful_main_actions_metadata_is_accepted() {
        let (run, jobs, artifacts) = approved_actions_metadata();
        verify_actions_metadata(1434, &run, &jobs, &artifacts).expect("approved run");
    }

    #[test]
    fn pull_request_failed_job_and_expired_artifact_are_rejected() {
        let (mut run, jobs, artifacts) = approved_actions_metadata();
        run["event"] = Value::String("pull_request".into());
        assert!(verify_actions_metadata(1434, &run, &jobs, &artifacts).is_err());

        let (run, mut jobs, artifacts) = approved_actions_metadata();
        jobs["jobs"][0]["conclusion"] = Value::String("failure".into());
        assert!(verify_actions_metadata(1434, &run, &jobs, &artifacts).is_err());

        let (run, jobs, mut artifacts) = approved_actions_metadata();
        artifacts["artifacts"][0]["expired"] = Value::Bool(true);
        assert!(verify_actions_metadata(1434, &run, &jobs, &artifacts).is_err());
    }

    #[test]
    fn baseline_identity_rejects_local_and_mismatched_provenance() {
        let (metadata, _, _) = approved_actions_metadata();
        let selection = RunSelection {
            backends: vec![Backend::Sqlite, Backend::Postgres],
            browsers: vec![Browser::Chromium],
            count_overrides: CountOverrides {
                posts: None,
                authors: None,
                revisions: None,
            },
            storage: true,
            browser: true,
        };
        let local = Provenance::Local(performance::LocalProvenance {
            git_commit: "abc123".into(),
        });
        assert!(
            verify_import_identity(1434, &metadata, DatasetProfile::Medium, &selection, &local,)
                .is_err()
        );

        let mut github = GitHubProvenance {
            repository: TRUSTED_REPOSITORY.into(),
            workflow: TRUSTED_WORKFLOW.into(),
            job: TRUSTED_JOB.into(),
            reference: "refs/heads/main".into(),
            head_sha: "wrong".into(),
            run_id: 1434,
            attempt: 2,
        };
        assert!(
            verify_import_identity(
                1434,
                &metadata,
                DatasetProfile::Medium,
                &selection,
                &Provenance::Github(github.clone()),
            )
            .is_err()
        );
        github.head_sha = "abc123".into();
        verify_import_identity(
            1434,
            &metadata,
            DatasetProfile::Medium,
            &selection,
            &Provenance::Github(github),
        )
        .expect("matching canonical provenance");
    }

    #[test]
    fn freshness_values_are_distinct_and_timestamped() {
        assert_ne!(freshness_nonce().unwrap(), freshness_nonce().unwrap());
        assert!(measurement_timestamp().unwrap() > 0);
    }

    #[test]
    fn retained_failed_producer_status_is_rejected_before_fragment_assembly() {
        let producer = tempfile::tempdir().expect("producer directory");
        let performance = producer.path().join("performance");
        fs::create_dir_all(&performance).expect("performance directory");
        fs::write(
            performance.join("producer-status-v1.json"),
            r#"{"schema_version":1,"ok":false,"detail":"validation failed"}"#,
        )
        .expect("producer status");

        let retained = tempfile::tempdir().expect("retained directory");
        let identity = NamedDerivationIdentity {
            name: "fixture".into(),
            identity: producer.path().display().to_string(),
        };
        retain_producer_artifacts(producer.path(), &identity, retained.path())
            .expect("retains producer evidence");

        let error = validate_producer_status(producer.path()).expect_err("failed producer status");
        assert!(error.to_string().contains("validation failed"));
        assert!(
            retained
                .path()
                .join("producers/fixture/producer-status-v1.json")
                .is_file()
        );
    }

    #[test]
    fn successful_fragment_is_retained_in_the_run_fragment_set() {
        let producer = tempfile::tempdir().expect("producer directory");
        let fragment = producer.path().join("storage-sqlite-v1.json");
        fs::write(&fragment, "{}").expect("producer fragment");
        let run = tempfile::tempdir().expect("run directory");
        fs::create_dir(run.path().join("fragments")).expect("run fragments directory");

        retain_fragment(&fragment, run.path()).expect("retains fragment");

        assert_eq!(
            fs::read_to_string(run.path().join("fragments/storage-sqlite-v1.json"))
                .expect("retained fragment"),
            "{}"
        );
    }

    #[test]
    fn producer_failure_becomes_a_standard_failed_command_result() {
        let result = finish_command(
            "perf",
            Instant::now(),
            Err(anyhow::anyhow!("producer failed")),
        );
        assert!(!result.ok);
        assert_ne!(result.finished_at_unix, 0);
        assert_eq!(result.steps.len(), 1);
        assert!(
            result.steps[0]
                .detail
                .as_deref()
                .is_some_and(|detail| detail.contains("producer failed"))
        );
    }
}
