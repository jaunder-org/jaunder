//! Controlled host-only coverage benchmark orchestration.
//!
//! This module intentionally observes the existing `devtool coverage emit`
//! producer rather than reimplementing coverage, partitioning, or verdict rules.

use std::collections::BTreeSet;
use std::env;
use std::fs;
use std::io::{BufRead, BufReader, Read};
use std::path::{Path, PathBuf};
use std::process::Command;

use anyhow::{Context, Result, bail};
use coverage::status::{CoverageStatus, StageResult};
use coverage::workers::{
    AggregateEvidence, ExperimentStrategy, WorkerConcurrencyPolicy, WorkerEvidence,
};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use super::run as evaluate_coverage;
use crate::git;

const MANIFEST_VERSION: &str = "coverage-local-benchmark-v6";
const ROOT: &str = ".xtask/coverage/benchmark-local";
const TIME_FORMAT: &str = "elapsed_seconds=%e\nuser_seconds=%U\nsystem_seconds=%S\ncpu_percent=%P\nmajor_page_faults=%F\nminor_page_faults=%R\nmax_rss_kib=%M\nexit_status=%x";

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct ScheduleEntry {
    pub ordinal: usize,
    pub strategy: ExperimentStrategy,
    pub concurrency: Option<WorkerConcurrencyPolicy>,
}

/// All host facts needed to decide whether two observations are comparable.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct HostIdentity {
    pub kernel: String,
    pub cpu_model: String,
    pub logical_cpus: usize,
    pub physical_memory_kib: u64,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct ToolIdentity {
    pub gnu_time: String,
    pub devtool: String,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct ResourceUsage {
    pub elapsed_seconds: f64,
    pub user_seconds: f64,
    pub system_seconds: f64,
    pub cpu_percent: f64,
    pub major_page_faults: u64,
    pub minor_page_faults: u64,
    pub max_rss_kib: u64,
    pub exit_status: i32,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct CoverageArtifactDigests {
    pub coverage_report_txt: String,
    pub coverage_report_lcov: String,
    pub coverage_semantics_lcov: String,
    pub crap_report_json: String,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct VerdictInputs {
    pub gate_passed: bool,
    pub failures: usize,
    pub guard_violations: usize,
    pub crap_fails: usize,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct Preparation {
    pub archive_identity: String,
    pub archive_path: String,
    pub archive_deleted_after_success: bool,
    pub command_log: String,
    pub resource_record: String,
    pub resource_usage: Option<ResourceUsage>,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct WarmCacheState {
    pub declared: String,
    pub preparation: Preparation,
    pub warmup_output: String,
    pub resource_record: String,
    pub completed: bool,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct Observation {
    pub schedule: ScheduleEntry,
    pub output_dir: String,
    /// Test-binary compilation is timed separately from coverage execution.
    pub preparation: Preparation,
    pub command_log: String,
    pub resource_record: String,
    pub resource_usage: Option<ResourceUsage>,
    pub producer_status: Option<CoverageStatus>,
    pub producer_stages: Vec<StageResult>,
    pub workers: Vec<WorkerEvidence>,
    pub aggregate: Option<AggregateEvidence>,
    pub coverage_verdict: Option<VerdictInputs>,
    pub artifact_digests: Option<CoverageArtifactDigests>,
    pub accepted: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub rejection: Option<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct Manifest {
    pub version: String,
    pub revision: String,
    pub host: HostIdentity,
    pub unloaded_system_assertion: String,
    pub tools: ToolIdentity,
    pub warm_cache: WarmCacheState,
    /// Producer `test-census` remains its residual build-check and census stage;
    /// timed instrumented-test preparation is deliberately recorded separately.
    pub test_census_interpretation: String,
    /// Text-report stage duration includes LLVM profile merge and report generation.
    pub text_report_interpretation: String,
    pub schedule: Vec<ScheduleEntry>,
    pub observations: Vec<Observation>,
}

/// Run the complete local matrix. The assertion is an operator statement, not a
/// heuristic: host load cannot be inferred reliably after the measurement.
pub fn run(unloaded_system_assertion: &str) -> Result<Manifest> {
    require_unloaded_system_assertion(unloaded_system_assertion)?;
    let revision = revision()?;
    if !git::working_tree_status(Path::new("."))?.trim().is_empty() {
        bail!("benchmark-local requires a clean working tree to identify the measured HEAD exactly")
    }
    let root = Path::new(ROOT).join(&revision);
    let host = host_identity()?;
    let tools = ToolIdentity {
        gnu_time: resolve_executable("time")?,
        devtool: build_live_devtool()?,
    };
    let schedule = schedule();
    let mut manifest = if root.exists() {
        let raw = fs::read_to_string(manifest_path(&revision))
            .context("reading interrupted benchmark manifest")?;
        let manifest: Manifest =
            serde_json::from_str(&raw).context("parsing interrupted benchmark manifest")?;
        validate_resumable_manifest(
            &manifest,
            &revision,
            &tools,
            &host,
            unloaded_system_assertion,
            &schedule,
        )?;
        manifest
    } else {
        fs::create_dir_all(&root).context("creating local benchmark evidence root")?;
        let manifest = Manifest {
            version: MANIFEST_VERSION.to_owned(),
            revision,
            host,
            tools,
            unloaded_system_assertion: unloaded_system_assertion.to_owned(),
            warm_cache: WarmCacheState {
                declared: "host coverage target warmed once before accepted observations"
                    .to_owned(),
                preparation: pending_preparation("warmup"),
                warmup_output: "warmup".to_owned(),
                resource_record: "warmup/gnu-time.txt".to_owned(),
                completed: false,
            },
            test_census_interpretation:
                "residual build-check+census after separately timed instrumented preparation"
                    .to_owned(),
            text_report_interpretation:
                "LLVM profile merge and text report generation are included in text-report duration"
                    .to_owned(),
            schedule,
            observations: Vec::new(),
        };
        write_manifest(&root, &manifest)?;
        manifest
    };
    if validate_manifest(&manifest).is_ok() {
        return Ok(manifest);
    }
    if !manifest.warm_cache.completed {
        if !preparation_succeeded(&manifest.warm_cache.preparation) {
            manifest.warm_cache.preparation =
                prepare_instrumented_tests(&root.join("warmup"), &manifest.tools, false)?;
            write_manifest(&root, &manifest)?;
        }
        if !preparation_succeeded(&manifest.warm_cache.preparation) {
            bail!("warmup instrumented preparation failed or retained its archive")
        }
        warm_host_target(&root, &manifest.tools)?;
        manifest.warm_cache.completed = true;
        write_manifest(&root, &manifest)?;
    }
    write_manifest(&root, &manifest)?;

    for index in 0..manifest.schedule.len() {
        if manifest.observations.len() == index {
            manifest
                .observations
                .push(pending_observation(&manifest.schedule[index]));
            write_manifest(&root, &manifest)?;
        }
        if manifest.observations[index].accepted {
            continue;
        }
        if manifest.observations[index].rejection.as_deref() != Some("observation did not complete")
        {
            bail!(
                "benchmark observation {} is rejected and cannot be resumed",
                index + 1
            )
        }
        let output = root.join(&manifest.observations[index].output_dir);
        if !preparation_succeeded(&manifest.observations[index].preparation) {
            let retain_archive =
                manifest.observations[index].schedule.strategy != ExperimentStrategy::Baseline;
            match prepare_instrumented_tests(&output, &manifest.tools, retain_archive) {
                Ok(preparation) => manifest.observations[index].preparation = preparation,
                Err(error) => {
                    manifest.observations[index].rejection =
                        Some(format!("could not prepare instrumented tests: {error:#}"))
                }
            }
            write_manifest(&root, &manifest)?;
        }
        if preparation_succeeded(&manifest.observations[index].preparation) {
            observe(&root, &manifest.tools, &mut manifest.observations[index]);
            write_manifest(&root, &manifest)?;
        }
        if !manifest.observations[index].accepted {
            bail!("benchmark observation {} was rejected", index + 1)
        }
    }
    validate_manifest(&manifest)?;
    write_manifest(&root, &manifest)?;
    Ok(manifest)
}

pub fn manifest_path(revision: &str) -> PathBuf {
    Path::new(ROOT).join(revision).join("manifest-v6.json")
}

fn require_unloaded_system_assertion(value: &str) -> Result<()> {
    if value.trim().is_empty() {
        bail!("benchmark-local requires a nonempty --unloaded-system assertion")
    }
    Ok(())
}

fn resolve_executable(name: &str) -> Result<String> {
    for directory in env::split_paths(&env::var_os("PATH").context("PATH is unset")?) {
        let path = directory.join(name);
        if path.is_absolute() && path.is_file() {
            return Ok(path.display().to_string());
        }
    }
    bail!("required executable {name} is absent from PATH")
}

fn build_live_devtool() -> Result<String> {
    let result = Command::new("cargo")
        .args([
            "build",
            "--quiet",
            "--manifest-path",
            "tools/Cargo.toml",
            "-p",
            "devtool",
        ])
        .output()
        .context("building live devtool for benchmark")?;
    if !result.status.success() {
        bail!(
            "building live devtool failed: {}",
            String::from_utf8_lossy(&result.stderr)
        );
    }
    Ok(fs::canonicalize("tools/target/debug/devtool")
        .context("canonicalizing live devtool")?
        .display()
        .to_string())
}

/// Two baselines and two observations per treatment. Pair reversal counterbalances
/// warming without pretending that baseline has a two-worker concurrency policy.
fn schedule() -> Vec<ScheduleEntry> {
    let treatments = [
        (
            ExperimentStrategy::Slice,
            Some(WorkerConcurrencyPolicy::Independent),
        ),
        (
            ExperimentStrategy::Slice,
            Some(WorkerConcurrencyPolicy::Fixed),
        ),
        (
            ExperimentStrategy::Hash,
            Some(WorkerConcurrencyPolicy::Independent),
        ),
        (
            ExperimentStrategy::Hash,
            Some(WorkerConcurrencyPolicy::Fixed),
        ),
        (
            ExperimentStrategy::Backend,
            Some(WorkerConcurrencyPolicy::Independent),
        ),
        (
            ExperimentStrategy::Backend,
            Some(WorkerConcurrencyPolicy::Fixed),
        ),
    ];
    let mut arms = Vec::with_capacity(14);
    for pair in 0..2 {
        arms.push((ExperimentStrategy::Baseline, None));
        let order: Box<
            dyn Iterator<Item = &(ExperimentStrategy, Option<WorkerConcurrencyPolicy>)>,
        > = if pair == 0 {
            Box::new(treatments.iter())
        } else {
            Box::new(treatments.iter().rev())
        };
        arms.extend(order.copied());
    }
    arms.into_iter()
        .enumerate()
        .map(|(ordinal, (strategy, concurrency))| ScheduleEntry {
            ordinal: ordinal + 1,
            strategy,
            concurrency,
        })
        .collect()
}

fn pending_observation(entry: &ScheduleEntry) -> Observation {
    let name = observation_name(entry);
    let preparation = pending_preparation(&format!("observations/{name}"));
    Observation {
        schedule: entry.clone(),
        output_dir: format!("observations/{name}"),
        preparation,
        artifact_digests: None,
        command_log: format!("observations/{name}/command.log"),
        resource_record: format!("observations/{name}/gnu-time.txt"),
        resource_usage: None,
        producer_status: None,
        producer_stages: Vec::new(),
        workers: Vec::new(),
        aggregate: None,
        coverage_verdict: None,
        accepted: false,
        rejection: Some("observation did not complete".to_owned()),
    }
}

fn pending_preparation(parent: &str) -> Preparation {
    Preparation {
        archive_identity: "cargo-nextest-archive:workspace/profile=coverage".to_owned(),
        archive_path: format!("{parent}/instrumented-tests.tar.zst"),
        archive_deleted_after_success: false,
        command_log: format!("{parent}/preparation.log"),
        resource_record: format!("{parent}/preparation-gnu-time.txt"),
        resource_usage: None,
    }
}

fn observation_name(entry: &ScheduleEntry) -> String {
    format!(
        "{:02}-{}-{}",
        entry.ordinal,
        entry.strategy.as_str(),
        entry
            .concurrency
            .map_or("not-applicable", WorkerConcurrencyPolicy::as_str)
    )
}

fn warm_host_target(root: &Path, tools: &ToolIdentity) -> Result<()> {
    let output = root.join("warmup");
    fs::create_dir_all(&output)?;
    let resource = output.join("gnu-time.txt");
    let log = output.join("command.log");
    let result = run_emit(&output, None, None, None, &resource, &log, tools)?;
    if !result.status.success() {
        bail!("warmup coverage producer exited unsuccessfully")
    }
    let usage = parse_gnu_time(&fs::read_to_string(&resource).context("reading warmup GNU time")?)?;
    if usage.exit_status != 0 {
        bail!("warmup GNU time recorded nonzero exit status")
    }
    CoverageStatus::from_completed_json(
        &fs::read_to_string(output.join("status.json"))
            .context("reading warmup producer status")?,
    )
    .context("validating warmup producer status")?;
    Ok(())
}

fn prepare_instrumented_tests(
    output: &Path,
    tools: &ToolIdentity,
    retain_archive: bool,
) -> Result<Preparation> {
    fs::create_dir_all(output).context("creating instrumented preparation output")?;
    let archive = output.join("instrumented-tests.tar.zst");
    let resource = output.join("preparation-gnu-time.txt");
    let log = output.join("preparation.log");
    let script = r#"environment="$(cargo llvm-cov show-env --export-prefix)" || exit
eval "$environment" || exit
exec cargo nextest archive --workspace --profile coverage --archive-file "$1""#;
    let result = Command::new(&tools.gnu_time)
        .args([
            "--output",
            &resource.display().to_string(),
            "--format",
            TIME_FORMAT,
        ])
        .args(["sh", "-c", script, "--"])
        .arg(&archive)
        .output()
        .context("starting GNU-time instrumented preparation")?;
    fs::write(
        &log,
        format!(
            "stdout:\n{}\nstderr:\n{}",
            String::from_utf8_lossy(&result.stdout),
            String::from_utf8_lossy(&result.stderr)
        ),
    )
    .context("writing instrumented preparation log")?;
    let usage = fs::read_to_string(&resource)
        .context("reading instrumented preparation GNU time")
        .and_then(|raw| parse_gnu_time(&raw))?;
    let succeeded = result.status.success() && usage.exit_status == 0 && archive.is_file();
    let archive_deleted_after_success = succeeded && !retain_archive;
    if archive_deleted_after_success {
        fs::remove_file(&archive).context("removing retained instrumented-test archive")?;
    }
    Ok(Preparation {
        archive_identity: "cargo-nextest-archive:workspace/profile=coverage".to_owned(),
        archive_path: archive.display().to_string(),
        archive_deleted_after_success,
        command_log: log.display().to_string(),
        resource_record: resource.display().to_string(),
        resource_usage: Some(usage),
    })
}

fn observe(root: &Path, tools: &ToolIdentity, observation: &mut Observation) {
    let output = root.join(&observation.output_dir);
    if !preparation_succeeded(&observation.preparation) {
        observation.rejection =
            Some("instrumented preparation failed or retained its archive".to_owned());
        return;
    }
    let resource = root.join(&observation.resource_record);
    let log = root.join(&observation.command_log);
    let archive = (observation.schedule.strategy != ExperimentStrategy::Baseline)
        .then(|| PathBuf::from(&observation.preparation.archive_path));
    let emitted = run_emit(
        &output,
        (observation.schedule.strategy != ExperimentStrategy::Baseline)
            .then_some(observation.schedule.strategy),
        observation
            .schedule
            .concurrency
            .map(WorkerConcurrencyPolicy::as_str),
        archive.as_deref(),
        &resource,
        &log,
        tools,
    );
    if let Some(archive) = archive {
        match fs::remove_file(&archive) {
            Ok(()) => observation.preparation.archive_deleted_after_success = true,
            Err(error) => {
                observation.rejection = Some(format!(
                    "could not remove instrumented-test archive: {error}"
                ));
                return;
            }
        }
    }
    match emitted {
        Ok(result) => {
            observation.resource_usage = fs::read_to_string(&resource)
                .ok()
                .and_then(|raw| parse_gnu_time(&raw).ok());
            if !result.status.success() {
                observation.rejection = Some("coverage producer exited unsuccessfully".to_owned());
                return;
            }
        }
        Err(error) => {
            observation.rejection = Some(format!("could not execute coverage producer: {error:#}"));
            return;
        }
    }
    let status = match fs::read_to_string(output.join("status.json"))
        .context("reading producer status")
        .and_then(|raw| {
            CoverageStatus::from_completed_json(&raw).context("validating producer status")
        }) {
        Ok(status) => status,
        Err(error) => {
            observation.rejection =
                Some(format!("non-green or malformed producer status: {error:#}"));
            return;
        }
    };
    observation.producer_stages = status.stages.clone();
    observation.producer_status = Some(status);
    if observation.schedule.strategy != ExperimentStrategy::Baseline {
        match read_aggregate(&output) {
            Ok(aggregate) => {
                observation.workers = aggregate.workers.clone();
                observation.aggregate = Some(aggregate);
            }
            Err(error) => {
                observation.rejection = Some(format!("malformed worker evidence: {error:#}"));
                return;
            }
        }
    }
    let output_text = output.to_string_lossy();
    let (coverage_step, coverage_verdict) = evaluate_coverage(&output_text);
    if coverage_verdict.is_none() {
        observation.rejection =
            Some("coverage line or CRAP verdict is missing or malformed".to_owned());
        return;
    }
    observation.artifact_digests = match coverage_artifact_digests(&output) {
        Ok(digests) => Some(digests),
        Err(error) => {
            observation.rejection = Some(format!(
                "missing or unreadable coverage artifact: {error:#}"
            ));
            return;
        }
    };
    observation.coverage_verdict = coverage_verdict.map(|verdict| VerdictInputs {
        gate_passed: !coverage_step.is_blocking_failure(),
        failures: verdict.failures,
        guard_violations: verdict.guard_violations,
        crap_fails: verdict.crap_fails,
    });
    match validate_accepted_observation(observation) {
        Ok(()) => {
            observation.accepted = true;
            observation.rejection = None;
        }
        Err(error) => observation.rejection = Some(format!("rejected evidence: {error:#}")),
    }
}

fn preparation_succeeded(preparation: &Preparation) -> bool {
    preparation
        .resource_usage
        .as_ref()
        .is_some_and(|usage| usage.exit_status == 0)
        && (preparation.archive_deleted_after_success
            || Path::new(&preparation.archive_path).is_file())
}

fn run_emit(
    output: &Path,
    strategy: Option<ExperimentStrategy>,
    concurrency: Option<&str>,
    archive: Option<&Path>,
    resource: &Path,
    log: &Path,
    tools: &ToolIdentity,
) -> Result<std::process::Output> {
    let mut command = Command::new(&tools.gnu_time);
    command.args([
        "--output",
        &resource.display().to_string(),
        "--format",
        TIME_FORMAT,
    ]);
    command.args([&tools.devtool, "coverage", "emit", "--out"]);
    command.arg(output);
    if let Some(strategy) = strategy {
        command.args(["--experiment", strategy.as_str()]);
    }
    if let Some(concurrency) = concurrency {
        command.args(["--concurrency", concurrency]);
    }
    if let Some(archive) = archive {
        command.args(["--archive-file", &archive.display().to_string()]);
    }
    let result = command
        .output()
        .context("starting GNU-time coverage observation")?;
    fs::write(
        log,
        format!(
            "stdout:\n{}\nstderr:\n{}",
            String::from_utf8_lossy(&result.stdout),
            String::from_utf8_lossy(&result.stderr)
        ),
    )
    .context("writing coverage observation command log")?;
    Ok(result)
}

fn read_aggregate(output: &Path) -> Result<AggregateEvidence> {
    let raw = fs::read_to_string(output.join("diagnostics/aggregate.json"))
        .context("reading aggregate worker evidence")?;
    serde_json::from_str(&raw).context("parsing aggregate worker evidence")
}

fn coverage_artifact_digests(output: &Path) -> Result<CoverageArtifactDigests> {
    let lcov = output.join("coverage-report.lcov");
    Ok(CoverageArtifactDigests {
        coverage_report_txt: sha256(&output.join("coverage-report.txt"))?,
        coverage_report_lcov: sha256(&lcov)?,
        coverage_semantics_lcov: semantic_lcov_sha256(&lcov)?,
        crap_report_json: sha256(&output.join("crap-report.json"))?,
    })
}

fn semantic_lcov_sha256(path: &Path) -> Result<String> {
    let file = fs::File::open(path).with_context(|| format!("opening {}", path.display()))?;
    let mut reader = BufReader::new(file);
    let mut digest = Sha256::new();
    let mut line = String::new();
    while reader
        .read_line(&mut line)
        .with_context(|| format!("reading {}", path.display()))?
        != 0
    {
        digest.update(semantic_lcov_line(line.trim_end_matches('\n'))?.as_bytes());
        digest.update(b"\n");
        line.clear();
    }
    Ok(crate::digest::lowercase_hex(digest.finalize()))
}

fn semantic_lcov_line(line: &str) -> Result<String> {
    if let Some(rest) = line.strip_prefix("DA:") {
        let mut fields = rest.splitn(3, ',');
        let number = fields.next().context("LCOV DA record lacks a line")?;
        let count = fields
            .next()
            .context("LCOV DA record lacks a count")?
            .parse::<u64>()
            .context("LCOV DA count is not numeric")?;
        return Ok(match fields.next() {
            Some(checksum) => format!("DA:{number},{},{checksum}", u8::from(count > 0)),
            None => format!("DA:{number},{}", u8::from(count > 0)),
        });
    }
    if let Some(rest) = line.strip_prefix("FNDA:") {
        let (count, name) = rest
            .split_once(',')
            .context("LCOV FNDA record lacks a function name")?;
        let count = count
            .parse::<u64>()
            .context("LCOV FNDA count is not numeric")?;
        return Ok(format!("FNDA:{},{name}", u8::from(count > 0)));
    }
    if let Some(rest) = line.strip_prefix("BRDA:") {
        let (identity, taken) = rest
            .rsplit_once(',')
            .context("LCOV BRDA record lacks a taken count")?;
        let taken = if taken == "-" {
            "-".to_owned()
        } else {
            u8::from(
                taken
                    .parse::<u64>()
                    .context("LCOV BRDA count is not numeric")?
                    > 0,
            )
            .to_string()
        };
        return Ok(format!("BRDA:{identity},{taken}"));
    }
    Ok(line.to_owned())
}

fn sha256(path: &Path) -> Result<String> {
    let mut file = fs::File::open(path).with_context(|| format!("opening {}", path.display()))?;
    let mut digest = Sha256::new();
    let mut buffer = [0_u8; 8192];
    loop {
        let read = file
            .read(&mut buffer)
            .with_context(|| format!("reading {}", path.display()))?;
        if read == 0 {
            break;
        }
        digest.update(&buffer[..read]);
    }
    Ok(crate::digest::lowercase_hex(digest.finalize()))
}

fn validate_accepted_observation(observation: &Observation) -> Result<()> {
    match (observation.accepted, observation.rejection.is_some()) {
        (true, false) | (false, true) => {}
        _ => bail!("observation acceptance state is contradictory"),
    }
    if !preparation_succeeded(&observation.preparation) {
        bail!("instrumented preparation is missing a successful resource record or deletion")
    }

    let usage = observation
        .resource_usage
        .as_ref()
        .context("missing GNU time record")?;
    if usage.exit_status != 0 {
        bail!(
            "GNU time recorded producer exit status {}",
            usage.exit_status
        )
    }
    let status = observation
        .producer_status
        .as_ref()
        .context("missing producer status")?;
    if observation.producer_stages != status.stages {
        bail!("recorded producer stage timings do not match producer status")
    }
    status.validate().context("validating producer status")?;
    if status.category != coverage::status::StatusCategory::TestsOk {
        bail!("producer status is not tests-ok")
    }
    let _verdict = observation
        .coverage_verdict
        .as_ref()
        .context("missing line/CRAP verdict")?;
    if observation.artifact_digests.is_none() {
        bail!("missing coverage artifact digests")
    }
    match observation.schedule.strategy {
        ExperimentStrategy::Baseline => {
            if observation.schedule.concurrency.is_some()
                || !observation.workers.is_empty()
                || observation.aggregate.is_some()
            {
                bail!("baseline must record two-worker fields as not applicable")
            }
        }
        strategy => {
            observation
                .schedule
                .concurrency
                .context("two-worker treatment lacks a concurrency policy")?;
            let aggregate = observation
                .aggregate
                .as_ref()
                .context("missing aggregate worker evidence")?;
            if aggregate.strategy != strategy || aggregate.workers.len() != 2 {
                bail!("aggregate does not describe the selected two-worker treatment")
            }
            if observation.workers != aggregate.workers {
                bail!("worker assignments do not match aggregate evidence")
            }
            if !matches!(
                aggregate.reconciliation,
                coverage::workers::Reconciliation::Reconciled
            ) {
                bail!("worker census did not reconcile completely")
            }
        }
    }
    Ok(())
}

fn validate_resumable_manifest(
    manifest: &Manifest,
    revision: &str,
    tools: &ToolIdentity,
    host: &HostIdentity,
    assertion: &str,
    schedule: &[ScheduleEntry],
) -> Result<()> {
    if manifest.version != MANIFEST_VERSION
        || manifest.revision != revision
        || &manifest.tools != tools
        || &manifest.host != host
        || manifest.unloaded_system_assertion != assertion
        || manifest.schedule.as_slice() != schedule
        || manifest.observations.len() > schedule.len()
    {
        bail!("existing manifest does not identify this exact benchmark invocation")
    }
    for (index, observation) in manifest.observations.iter().enumerate() {
        if observation.schedule != schedule[index] {
            bail!("existing manifest observation order does not match the fixed schedule")
        }
        if observation.accepted {
            validate_accepted_observation(observation)?;
        }
    }
    Ok(())
}

fn validate_manifest(manifest: &Manifest) -> Result<()> {
    if manifest.version != MANIFEST_VERSION || manifest.unloaded_system_assertion.trim().is_empty()
    {
        bail!("unknown manifest version or missing unloaded-system assertion")
    }
    if manifest.test_census_interpretation
        != "residual build-check+census after separately timed instrumented preparation"
        || manifest.text_report_interpretation
            != "LLVM profile merge and text report generation are included in text-report duration"
    {
        bail!("manifest stage timing interpretation is incomplete")
    }
    if !manifest.warm_cache.completed
        || manifest.schedule != schedule()
        || manifest.observations.len() != manifest.schedule.len()
    {
        bail!("manifest is incomplete or has a non-canonical schedule")
    }
    let baseline = manifest
        .observations
        .iter()
        .find(|observation| observation.schedule.strategy == ExperimentStrategy::Baseline)
        .and_then(|observation| observation.artifact_digests.as_ref())
        .context("missing accepted baseline artifact digests")?;
    let baseline_verdict = manifest
        .observations
        .iter()
        .find(|observation| observation.schedule.strategy == ExperimentStrategy::Baseline)
        .and_then(|observation| observation.coverage_verdict.as_ref())
        .context("missing accepted baseline verdict")?;
    let mut seen = BTreeSet::new();
    for observation in &manifest.observations {
        if !seen.insert(observation.output_dir.as_str()) {
            bail!("observation output directories must be isolated")
        }
        if !observation.accepted {
            bail!("manifest contains a rejected observation")
        }
        validate_accepted_observation(observation)?;
        let digests = observation
            .artifact_digests
            .as_ref()
            .context("missing coverage artifact digests")?;
        if digests.coverage_semantics_lcov != baseline.coverage_semantics_lcov
            || digests.crap_report_json != baseline.crap_report_json
        {
            bail!("coverage hit-set or CRAP semantics differ from baseline")
        }
        if observation.coverage_verdict.as_ref() != Some(baseline_verdict) {
            bail!("coverage line/CRAP verdict differs from baseline")
        }
    }
    Ok(())
}

fn parse_gnu_time(raw: &str) -> Result<ResourceUsage> {
    let mut fields = std::collections::BTreeMap::new();
    for line in raw.lines() {
        let (key, value) = line.split_once('=').context("malformed GNU time record")?;
        if fields.insert(key, value).is_some() {
            bail!("duplicate GNU time field {key}")
        }
    }
    if fields.len() != 8 {
        bail!("incomplete GNU time record")
    }
    let field = |name| {
        fields
            .get(name)
            .copied()
            .with_context(|| format!("missing GNU time field {name}"))
    };
    let usage = ResourceUsage {
        elapsed_seconds: field("elapsed_seconds")?
            .parse()
            .context("parsing elapsed_seconds")?,
        user_seconds: field("user_seconds")?
            .parse()
            .context("parsing user_seconds")?,
        system_seconds: field("system_seconds")?
            .parse()
            .context("parsing system_seconds")?,
        cpu_percent: field("cpu_percent")?
            .trim_end_matches('%')
            .parse()
            .context("parsing cpu_percent")?,
        major_page_faults: field("major_page_faults")?
            .parse()
            .context("parsing major_page_faults")?,
        minor_page_faults: field("minor_page_faults")?
            .parse()
            .context("parsing minor_page_faults")?,
        max_rss_kib: field("max_rss_kib")?
            .parse()
            .context("parsing max_rss_kib")?,
        exit_status: field("exit_status")?
            .parse()
            .context("parsing exit_status")?,
    };
    if !usage.elapsed_seconds.is_finite()
        || !usage.user_seconds.is_finite()
        || !usage.system_seconds.is_finite()
        || !usage.cpu_percent.is_finite()
        || usage.elapsed_seconds < 0.0
        || usage.user_seconds < 0.0
        || usage.system_seconds < 0.0
        || usage.cpu_percent < 0.0
    {
        bail!("GNU time record has an invalid duration")
    }
    Ok(usage)
}

fn revision() -> Result<String> {
    let output = git::at(Path::new("."))
        .args(["rev-parse", "HEAD"])
        .output()
        .context("reading benchmark revision")?;
    if !output.status.success() {
        bail!("git rev-parse HEAD failed")
    }
    let revision = String::from_utf8(output.stdout).context("decoding benchmark revision")?;
    let revision = revision.trim();
    if revision.len() != 40 || !revision.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        bail!("git revision is not a full hexadecimal object id")
    }
    Ok(revision.to_owned())
}

fn host_identity() -> Result<HostIdentity> {
    let kernel = command_text("uname", &["-srmo"])?;
    let cpu_model = fs::read_to_string("/proc/cpuinfo")
        .context("reading /proc/cpuinfo")?
        .lines()
        .find_map(|line| {
            line.strip_prefix("model name\t: ")
                .or_else(|| line.strip_prefix("Hardware\t: "))
        })
        .context("discovering host CPU model")?
        .to_owned();
    let logical_cpus = std::thread::available_parallelism()
        .context("discovering host CPU count")?
        .get();
    let physical_memory_kib =
        mem_total_kib(&fs::read_to_string("/proc/meminfo").context("reading /proc/meminfo")?)?;
    Ok(HostIdentity {
        kernel,
        cpu_model,
        logical_cpus,
        physical_memory_kib,
    })
}

fn mem_total_kib(meminfo: &str) -> Result<u64> {
    let mut values = meminfo
        .lines()
        .filter_map(|line| line.strip_prefix("MemTotal:"));
    let value = values.next().context("missing MemTotal in /proc/meminfo")?;
    if values.next().is_some() {
        bail!("duplicate MemTotal in /proc/meminfo")
    }
    let fields: Vec<_> = value.split_ascii_whitespace().collect();
    let [amount, unit] = fields.as_slice() else {
        bail!("malformed MemTotal in /proc/meminfo")
    };
    if *unit != "kB" {
        bail!("MemTotal unit is not kB")
    }
    let amount: u64 = amount.parse().context("parsing MemTotal")?;
    if amount == 0 {
        bail!("MemTotal must be positive")
    }
    Ok(amount)
}

fn command_text(program: &str, arguments: &[&str]) -> Result<String> {
    let output = Command::new(program)
        .args(arguments)
        .output()
        .with_context(|| format!("running {program}"))?;
    if !output.status.success() {
        bail!("{program} exited unsuccessfully")
    }
    let text =
        String::from_utf8(output.stdout).with_context(|| format!("decoding {program} output"))?;
    let text = text.trim();
    if text.is_empty() {
        bail!("{program} returned empty output")
    }
    Ok(text.to_owned())
}

fn write_manifest(root: &Path, manifest: &Manifest) -> Result<()> {
    fs::write(
        root.join("manifest-v6.json"),
        format!("{}\n", serde_json::to_string_pretty(manifest)?),
    )
    .context("persisting benchmark manifest")
}
#[cfg(test)]
mod tests {
    use super::*;
    use coverage::status::{
        COVERAGE_STATUS_VERSION, Population, ProcessOutcome, RequiredStage, StatusCategory,
    };

    fn green_status() -> CoverageStatus {
        CoverageStatus {
            version: COVERAGE_STATUS_VERSION,
            category: StatusCategory::TestsOk,
            stages: RequiredStage::ALL
                .into_iter()
                .map(|stage| StageResult {
                    stage,
                    outcome: ProcessOutcome::Success,
                    duration_ms: Some(1),
                })
                .collect(),
            population: Population {
                expected: 1,
                executed: 1,
                ignored: 0,
            },
            failed_tests: Vec::new(),
            missing_tests: Vec::new(),
            infra_detail: None,
        }
    }

    fn accepted_baseline() -> Observation {
        let entry = schedule()[0].clone();
        let mut observation = pending_observation(&entry);
        observation.preparation.archive_deleted_after_success = true;
        observation.preparation.resource_usage = Some(ResourceUsage {
            elapsed_seconds: 1.0,
            user_seconds: 0.5,
            system_seconds: 0.2,
            cpu_percent: 70.0,
            major_page_faults: 0,
            minor_page_faults: 1,
            max_rss_kib: 1,
            exit_status: 0,
        });
        observation.resource_usage = Some(ResourceUsage {
            elapsed_seconds: 1.0,
            user_seconds: 0.5,
            system_seconds: 0.2,
            cpu_percent: 70.0,
            major_page_faults: 0,
            minor_page_faults: 1,
            max_rss_kib: 1,
            exit_status: 0,
        });
        observation.producer_status = Some(green_status());
        observation.producer_stages = observation.producer_status.as_ref().unwrap().stages.clone();
        observation.coverage_verdict = Some(VerdictInputs::default());
        observation.artifact_digests = Some(CoverageArtifactDigests {
            coverage_report_txt: "a".repeat(64),
            coverage_report_lcov: "b".repeat(64),
            coverage_semantics_lcov: "d".repeat(64),
            crap_report_json: "c".repeat(64),
        });
        observation
    }

    #[test]
    fn schedule_has_exact_counterbalanced_population() {
        let schedule = schedule();
        assert_eq!(schedule.len(), 14);
        assert_eq!(schedule[0].strategy, ExperimentStrategy::Baseline);
        assert_eq!(schedule[7].strategy, ExperimentStrategy::Baseline);
        for (left, right) in schedule[1..7].iter().zip(schedule[8..14].iter().rev()) {
            assert_eq!(left.strategy, right.strategy);
            assert_eq!(left.concurrency, right.concurrency);
        }
        assert_eq!(
            schedule
                .iter()
                .filter(|entry| entry.strategy == ExperimentStrategy::Baseline)
                .count(),
            2
        );
        for (strategy, concurrency) in [
            (
                ExperimentStrategy::Slice,
                Some(WorkerConcurrencyPolicy::Independent),
            ),
            (
                ExperimentStrategy::Slice,
                Some(WorkerConcurrencyPolicy::Fixed),
            ),
            (
                ExperimentStrategy::Hash,
                Some(WorkerConcurrencyPolicy::Independent),
            ),
            (
                ExperimentStrategy::Hash,
                Some(WorkerConcurrencyPolicy::Fixed),
            ),
            (
                ExperimentStrategy::Backend,
                Some(WorkerConcurrencyPolicy::Independent),
            ),
            (
                ExperimentStrategy::Backend,
                Some(WorkerConcurrencyPolicy::Fixed),
            ),
        ] {
            assert_eq!(
                schedule
                    .iter()
                    .filter(|entry| entry.strategy == strategy && entry.concurrency == concurrency)
                    .count(),
                2
            );
        }
    }

    #[test]
    fn unloaded_system_assertion_is_explicit() {
        assert!(require_unloaded_system_assertion(" \t ").is_err());
        assert!(require_unloaded_system_assertion("operator confirms host is unloaded").is_ok());
    }

    #[test]
    fn mem_total_requires_one_positive_kib_value() {
        assert_eq!(mem_total_kib("MemTotal:       32768 kB\n").unwrap(), 32768);
        assert!(mem_total_kib("MemTotal: 0 kB\n").is_err());
        assert!(mem_total_kib("MemTotal: 1 MB\n").is_err());
        assert!(mem_total_kib("MemTotal: 1 kB\nMemTotal: 2 kB\n").is_err());
    }

    #[test]
    fn gnu_time_record_is_complete_and_strict() {
        let usage = parse_gnu_time(
            "elapsed_seconds=1.25\nuser_seconds=0.75\nsystem_seconds=0.25\ncpu_percent=80%\nmajor_page_faults=3\nminor_page_faults=4\nmax_rss_kib=1024\nexit_status=0\n",
        )
        .unwrap();
        assert_eq!(usage.max_rss_kib, 1024);
        assert_eq!(usage.major_page_faults, 3);
        assert!(parse_gnu_time("elapsed_seconds=1\n").is_err());
        assert!(parse_gnu_time("elapsed_seconds=1\nelapsed_seconds=2\n").is_err());
    }

    #[test]
    fn accepted_observation_requires_green_complete_evidence() {
        let observation = accepted_baseline();
        assert!(validate_accepted_observation(&observation).is_ok());
        let mut non_green = observation.clone();
        non_green.coverage_verdict = Some(VerdictInputs {
            gate_passed: false,
            failures: 30,
            guard_violations: 0,
            crap_fails: 0,
        });
        let treatment = non_green.clone();
        assert!(validate_accepted_observation(&non_green).is_ok());
        assert_eq!(non_green.coverage_verdict, treatment.coverage_verdict);
        let mut bad_time = observation.clone();
        bad_time.resource_usage.as_mut().unwrap().exit_status = 1;
        assert!(validate_accepted_observation(&bad_time).is_err());
        let mut bad_baseline = observation.clone();
        let mut missing_digest = observation;
        missing_digest.artifact_digests = None;
        assert!(validate_accepted_observation(&missing_digest).is_err());
        bad_baseline.schedule.concurrency = Some(WorkerConcurrencyPolicy::Fixed);
        assert!(validate_accepted_observation(&bad_baseline).is_err());
    }
    #[test]
    fn lcov_semantics_ignore_positive_execution_count_variance() {
        assert_eq!(
            semantic_lcov_line("DA:42,20,checksum").unwrap(),
            semantic_lcov_line("DA:42,21,checksum").unwrap()
        );
        assert_eq!(
            semantic_lcov_line("FNDA:20,function").unwrap(),
            semantic_lcov_line("FNDA:21,function").unwrap()
        );
        assert_eq!(
            semantic_lcov_line("BRDA:42,0,1,20").unwrap(),
            semantic_lcov_line("BRDA:42,0,1,21").unwrap()
        );
        assert_ne!(
            semantic_lcov_line("DA:42,0").unwrap(),
            semantic_lcov_line("DA:42,1").unwrap()
        );
    }
}
