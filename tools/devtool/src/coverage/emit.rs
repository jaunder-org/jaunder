use std::fs;
use std::io::ErrorKind;
use std::path::{Path, PathBuf};
use std::process::{Command, ExitStatus};
use std::thread;
use std::time::Instant;

use anyhow::{Context, Result, bail};
use coverage::status::{
    self, CoverageStatus, Population, ProcessOutcome, RequiredStage, StageResult, StatusCategory,
    TestCensus,
};
use coverage::workers::{
    AggregateEvidence, ExperimentStrategy, WORKER_EVIDENCE_VERSION, WorkerEvidence,
    WorkerPartition, aggregate_terminal,
};
use serde_json::Value;

use crate::pg;

const JUNIT_PATH: &str = "/tmp/jaunder-coverage-junit.xml";
const CSR_BUNDLE_FILENAME_REGEX: &str = r"(^|.*/)tools/csr_bundle/";
const CSR_BUNDLE_PACKAGE_PATH: &str = "tools/csr_bundle";
const LLVM_COV_ENVIRONMENT_SCRIPT: &str = r#"environment="$(cargo llvm-cov show-env --export-prefix)" || exit; eval "$environment" || exit; exec "$@""#;

fn is_csr_bundle_package_path(path: &str) -> bool {
    match path.strip_prefix(CSR_BUNDLE_PACKAGE_PATH) {
        Some(remainder) => remainder.is_empty() || remainder.starts_with('/'),
        None => false,
    }
}

#[derive(Debug)]
struct CommandSpec {
    stage: RequiredStage,
    program: &'static str,
    arguments: Vec<&'static str>,
}

#[derive(Debug)]
struct CapturedCommand {
    status: ExitStatus,
    stdout: String,
    output: String,
}

fn required_stage_commands() -> Vec<CommandSpec> {
    vec![
        CommandSpec {
            stage: RequiredStage::WorkspaceResolution,
            program: "cargo",
            arguments: vec![
                "metadata",
                "--manifest-path",
                "Cargo.toml",
                "--format-version",
                "1",
                "--no-deps",
            ],
        },
        CommandSpec {
            stage: RequiredStage::TestCensus,
            program: "sh",
            arguments: vec![
                "-c",
                LLVM_COV_ENVIRONMENT_SCRIPT,
                "--",
                "cargo",
                "nextest",
                "list",
                "--workspace",
                "--message-format",
                "json",
            ],
        },
        CommandSpec {
            stage: RequiredStage::InstrumentedTestRun,
            program: "sh",
            arguments: vec![
                "-c",
                LLVM_COV_ENVIRONMENT_SCRIPT,
                "--",
                "cargo",
                "nextest",
                "run",
                "--workspace",
                "--profile",
                "coverage",
                "--no-fail-fast",
            ],
        },
    ]
}

fn coverage_report_arguments(format: &'static str) -> [&'static str; 9] {
    [
        "-c",
        LLVM_COV_ENVIRONMENT_SCRIPT,
        "--",
        "cargo",
        "llvm-cov",
        "report",
        format,
        "--ignore-filename-regex",
        CSR_BUNDLE_FILENAME_REGEX,
    ]
}

fn new_status() -> CoverageStatus {
    CoverageStatus {
        version: status::COVERAGE_STATUS_VERSION,
        category: StatusCategory::Infra,
        stages: RequiredStage::ALL
            .into_iter()
            .map(|stage| StageResult {
                stage,
                outcome: ProcessOutcome::NotRun,
                duration_ms: None,
            })
            .collect(),
        population: Population {
            expected: 0,
            executed: 0,
            ignored: 0,
        },
        failed_tests: vec![],
        missing_tests: vec![],
        infra_detail: Some("coverage producer did not complete".into()),
    }
}

fn set_stage(status: &mut CoverageStatus, stage: RequiredStage, outcome: ProcessOutcome) {
    status
        .stages
        .iter_mut()
        .find(|result| result.stage == stage)
        .expect("required stage")
        .outcome = outcome;
}

fn record_duration(status: &mut CoverageStatus, stage: RequiredStage, started: Instant) {
    status
        .stages
        .iter_mut()
        .find(|result| result.stage == stage)
        .expect("required stage")
        .duration_ms = Some(started.elapsed().as_millis());
}

#[cfg(test)]
fn status_for_required_stage_failure(
    stage: RequiredStage,
    outcome: ProcessOutcome,
) -> CoverageStatus {
    let mut status = new_status();
    set_stage(&mut status, stage, outcome);
    status.infra_detail = Some(format!("{} failed", stage.as_str()));
    status
}

fn record_infra(status: &mut CoverageStatus, stage: RequiredStage, detail: &str) {
    status.category = StatusCategory::Infra;
    status.infra_detail = Some(format!("{}: {detail}", stage.as_str()));
}

fn record_evidence_error(status: &mut CoverageStatus, stage: RequiredStage, detail: &str) {
    set_stage(
        status,
        stage,
        ProcessOutcome::EvidenceError {
            evidence_error: detail.into(),
        },
    );
    record_infra(status, stage, detail);
}

fn command_outcome(status: ExitStatus) -> ProcessOutcome {
    match status.code() {
        Some(0) => ProcessOutcome::success(),
        Some(exit_code) => ProcessOutcome::ExitCode { exit_code },
        None => ProcessOutcome::Signal,
    }
}

fn final_category(
    instrumented_outcome: &ProcessOutcome,
    failed_tests: &[String],
) -> StatusCategory {
    if instrumented_outcome.is_success() {
        StatusCategory::TestsOk
    } else if matches!(
        instrumented_outcome,
        ProcessOutcome::ExitCode { exit_code } if *exit_code != 0
    ) && !failed_tests.is_empty()
    {
        StatusCategory::TestFailure
    } else {
        StatusCategory::Infra
    }
}
fn complete_status(status: &mut CoverageStatus, outcome: &ProcessOutcome, infra_detail: &str) {
    match final_category(outcome, &status.failed_tests) {
        category @ (StatusCategory::TestsOk | StatusCategory::TestFailure) => {
            status.category = category;
            status.infra_detail = None;
        }
        StatusCategory::Infra => {
            record_infra(status, RequiredStage::InstrumentedTestRun, infra_detail);
        }
    }
}

fn run_capture(command: &mut Command) -> Result<CapturedCommand> {
    let output = command
        .output()
        .with_context(|| format!("spawning {command:?}"))?;
    let stdout = String::from_utf8_lossy(&output.stdout).into_owned();
    let mut combined = stdout.clone();
    combined.push_str(&String::from_utf8_lossy(&output.stderr));
    Ok(CapturedCommand {
        status: output.status,
        stdout,
        output: combined,
    })
}

fn write_status(out: &Path, status: &CoverageStatus) -> Result<()> {
    status.validate().context("validating coverage status")?;
    fs::write(out.join("status.json"), status.to_json())
        .with_context(|| format!("writing {}/status.json", out.display()))
}

fn write_diagnostic(diag: &Path, name: &str, output: &str) {
    let _ = fs::write(diag.join(name), output);
}

fn command_failure(
    status: &mut CoverageStatus,
    stage: RequiredStage,
    command: Result<CapturedCommand>,
    diag: &Path,
    log_name: &str,
) -> Option<CapturedCommand> {
    match command {
        Ok(command) => {
            write_diagnostic(diag, log_name, &command.output);
            let outcome = command_outcome(command.status);
            set_stage(status, stage, outcome.clone());
            if outcome.is_success() {
                Some(command)
            } else {
                record_infra(status, stage, "command exited unsuccessfully");
                None
            }
        }
        Err(_) => {
            set_stage(
                status,
                stage,
                ProcessOutcome::SpawnError {
                    spawn_error: "could not spawn command".into(),
                },
            );
            record_infra(status, stage, "could not spawn command");
            None
        }
    }
}

fn load_junit_census() -> Result<TestCensus> {
    status::parse_junit_census(&fs::read_to_string(JUNIT_PATH).context("reading JUnit report")?)
}

fn report_command(
    stage: RequiredStage,
    command: &mut Command,
    status: &mut CoverageStatus,
    diag: &Path,
    name: &str,
) -> Option<String> {
    command_failure(status, stage, run_capture(command), diag, name).map(|command| command.stdout)
}

/// Emit the shared merged-profile reports, retaining a valid red status on every
/// expected report or artifact failure.
fn emit_reports(
    out: &Path,
    diag: &Path,
    status: &mut CoverageStatus,
    coverage_root: &str,
    analysis_root: &str,
) -> Result<()> {
    let text_started = Instant::now();
    let Some(text) = report_command(
        RequiredStage::TextReport,
        Command::new("sh").args(coverage_report_arguments("--text")),
        status,
        diag,
        "text-report.log",
    ) else {
        record_duration(status, RequiredStage::TextReport, text_started);
        return write_status(out, status);
    };
    if fs::write(
        out.join("coverage-report.txt"),
        coverage::pathnorm::normalize_report_text(&text, coverage_root),
    )
    .is_err()
    {
        record_evidence_error(
            status,
            RequiredStage::TextReport,
            "could not write text report",
        );
        record_duration(status, RequiredStage::TextReport, text_started);
        return write_status(out, status);
    }
    record_duration(status, RequiredStage::TextReport, text_started);

    let lcov_started = Instant::now();
    let lcov = out.join("coverage-report.lcov");
    let Some(lcov_path) = lcov.to_str() else {
        record_evidence_error(
            status,
            RequiredStage::LcovReport,
            "invalid LCOV report path",
        );
        record_duration(status, RequiredStage::LcovReport, lcov_started);
        return write_status(out, status);
    };
    if report_command(
        RequiredStage::LcovReport,
        Command::new("sh")
            .args(coverage_report_arguments("--lcov"))
            .args(["--output-path", lcov_path]),
        status,
        diag,
        "lcov-report.log",
    )
    .is_none()
    {
        record_duration(status, RequiredStage::LcovReport, lcov_started);
        return write_status(out, status);
    }
    if coverage_root != analysis_root {
        let remapped = fs::read_to_string(&lcov)
            .map(|raw| remap_lcov_source_root(&raw, coverage_root, analysis_root));
        if remapped
            .and_then(|contents| fs::write(&lcov, contents))
            .is_err()
        {
            record_evidence_error(
                status,
                RequiredStage::LcovReport,
                "could not remap LCOV source paths",
            );
            record_duration(status, RequiredStage::LcovReport, lcov_started);
            return write_status(out, status);
        }
    }
    record_duration(status, RequiredStage::LcovReport, lcov_started);

    let crap_started = Instant::now();
    let raw_crap = out.join("crap-report.raw.json");
    let Some(raw_crap_path) = raw_crap.to_str() else {
        record_evidence_error(
            status,
            RequiredStage::CrapReport,
            "invalid CRAP report path",
        );
        record_duration(status, RequiredStage::CrapReport, crap_started);
        return write_status(out, status);
    };
    if report_command(
        RequiredStage::CrapReport,
        Command::new("cargo").args([
            "crap",
            "--workspace",
            "--lcov",
            lcov_path,
            "--exclude",
            "**/tests/**",
            "--format",
            "json",
            "--output",
            raw_crap_path,
        ]),
        status,
        diag,
        "crap-report.log",
    )
    .is_none()
    {
        record_duration(status, RequiredStage::CrapReport, crap_started);
        return write_status(out, status);
    }
    let crap = match fs::read_to_string(&raw_crap)
        .ok()
        .and_then(|raw| normalize_crap_paths(&raw, analysis_root).ok())
    {
        Some(crap) => crap,
        None => {
            record_evidence_error(status, RequiredStage::CrapReport, "invalid CRAP report");
            record_duration(status, RequiredStage::CrapReport, crap_started);
            return write_status(out, status);
        }
    };
    if fs::write(out.join("crap-report.json"), crap).is_err() {
        record_evidence_error(
            status,
            RequiredStage::CrapReport,
            "could not write CRAP report",
        );
        record_duration(status, RequiredStage::CrapReport, crap_started);
        return write_status(out, status);
    }
    record_duration(status, RequiredStage::CrapReport, crap_started);
    Ok(())
}

/// Run the instrumented suite and emit reports + status + diagnostics into `out`.
///
/// Required stages always leave a validated, red `status.json` when they can be
/// started. Raw process output is diagnostic-only; it never supplies success.
pub fn run(out: &str) -> Result<()> {
    let out = Path::new(out);
    let diag = out.join("diagnostics");
    fs::create_dir_all(&diag).with_context(|| format!("creating {}", diag.display()))?;
    let abs_root = std::env::current_dir()?.to_string_lossy().into_owned();
    let mut status = new_status();
    let commands = required_stage_commands();

    let metadata = &commands[0];
    let metadata_started = Instant::now();
    let mut command = Command::new(metadata.program);
    command.args(&metadata.arguments);
    let metadata_result = command_failure(
        &mut status,
        metadata.stage,
        run_capture(&mut command),
        &diag,
        "metadata.log",
    );
    record_duration(&mut status, metadata.stage, metadata_started);
    let Some(_) = metadata_result else {
        write_status(out, &status)?;
        return Ok(());
    };

    let cleanup_started = Instant::now();
    let cleanup = run_capture(Command::new("cargo").args(["llvm-cov", "clean", "--profraw-only"]));
    let cleanup_result = command_failure(
        &mut status,
        RequiredStage::ProfileCleanup,
        cleanup,
        &diag,
        "profile-cleanup.log",
    );
    let Some(_) = cleanup_result else {
        record_duration(&mut status, RequiredStage::ProfileCleanup, cleanup_started);
        write_status(out, &status)?;
        return Ok(());
    };
    if let Err(error) = fs::remove_file(JUNIT_PATH)
        && error.kind() != ErrorKind::NotFound
    {
        record_evidence_error(
            &mut status,
            RequiredStage::ProfileCleanup,
            "could not clear JUnit report",
        );
        record_duration(&mut status, RequiredStage::ProfileCleanup, cleanup_started);
        write_status(out, &status)?;
        return Ok(());
    }
    record_duration(&mut status, RequiredStage::ProfileCleanup, cleanup_started);

    let census = &commands[1];
    let census_started = Instant::now();
    let mut command = Command::new(census.program);
    command.args(&census.arguments);
    let census_result = command_failure(
        &mut status,
        census.stage,
        run_capture(&mut command),
        &diag,
        "nextest-list.json",
    );
    let Some(census_output) = census_result else {
        record_duration(&mut status, census.stage, census_started);
        write_status(out, &status)?;
        return Ok(());
    };
    let expected = match status::parse_nextest_census(&census_output.stdout) {
        Ok(expected) => expected,
        Err(_) => {
            record_evidence_error(
                &mut status,
                RequiredStage::TestCensus,
                "invalid nextest census",
            );
            record_duration(&mut status, RequiredStage::TestCensus, census_started);
            write_status(out, &status)?;
            return Ok(());
        }
    };
    status.population.expected = expected.expected.len();
    record_duration(&mut status, RequiredStage::TestCensus, census_started);

    let run = &commands[2];
    let test_run_started = Instant::now();
    let test_run = pg::with_ephemeral(|env| {
        let mut command = Command::new(run.program);
        command.args(&run.arguments);
        env.configure_command(&mut command);
        run_capture(&mut command)
    });
    let instrumented_outcome = match test_run {
        Ok(test_run) => {
            let outcome = command_outcome(test_run.status);
            write_diagnostic(&diag, "nextest.log", &test_run.output);
            set_stage(&mut status, run.stage, outcome.clone());
            record_duration(&mut status, run.stage, test_run_started);
            outcome
        }
        Err(_) => {
            set_stage(
                &mut status,
                run.stage,
                ProcessOutcome::SpawnError {
                    spawn_error: "could not spawn command".into(),
                },
            );
            record_infra(
                &mut status,
                RequiredStage::InstrumentedTestRun,
                "could not spawn command",
            );
            record_duration(&mut status, run.stage, test_run_started);
            write_status(out, &status)?;
            return Ok(());
        }
    };
    let test_run = instrumented_outcome.is_success();
    let _ = fs::copy(JUNIT_PATH, diag.join("nextest.junit.xml"));

    let reconciliation_started = Instant::now();
    let actual = match load_junit_census() {
        Ok(actual) => actual,
        Err(_) => {
            record_evidence_error(
                &mut status,
                RequiredStage::PopulationReconciliation,
                "invalid JUnit census",
            );
            record_duration(
                &mut status,
                RequiredStage::PopulationReconciliation,
                reconciliation_started,
            );
            write_status(out, &status)?;
            return Ok(());
        }
    };
    status.population.executed = actual.executed.len();
    status.population.ignored = actual.ignored.len();
    status.failed_tests = actual.failed.clone();
    if let Err(error) = status::reconcile_test_census(&expected, &actual) {
        status.missing_tests = expected
            .expected
            .iter()
            .filter(|test| !actual.executed.contains(*test) && !actual.ignored.contains(*test))
            .cloned()
            .collect();
        let _ = error;
        record_evidence_error(
            &mut status,
            RequiredStage::PopulationReconciliation,
            "test population did not reconcile",
        );
        record_duration(
            &mut status,
            RequiredStage::PopulationReconciliation,
            reconciliation_started,
        );
        write_status(out, &status)?;
        return Ok(());
    }
    set_stage(
        &mut status,
        RequiredStage::PopulationReconciliation,
        ProcessOutcome::success(),
    );
    record_duration(
        &mut status,
        RequiredStage::PopulationReconciliation,
        reconciliation_started,
    );

    if !test_run && status.failed_tests.is_empty() {
        record_infra(
            &mut status,
            RequiredStage::InstrumentedTestRun,
            "unclassified test command failure",
        );
        write_status(out, &status)?;
        return Ok(());
    }
    if test_run && !status.failed_tests.is_empty() {
        record_evidence_error(
            &mut status,
            RequiredStage::InstrumentedTestRun,
            "JUnit reported failures after successful command",
        );
        write_status(out, &status)?;
        return Ok(());
    }

    emit_reports(out, &diag, &mut status, &abs_root, &abs_root)?;
    if status.stages.iter().any(|stage| {
        matches!(
            stage.stage,
            RequiredStage::TextReport | RequiredStage::LcovReport | RequiredStage::CrapReport
        ) && !stage.outcome.is_success()
    }) {
        return Ok(());
    }

    // Disk diagnostics are intentionally best-effort and cannot change status.
    if let Ok(disk) = run_capture(Command::new("df").arg("-h")) {
        write_diagnostic(&diag, "disk-usage.txt", &disk.output);
    }

    complete_status(
        &mut status,
        &instrumented_outcome,
        "test command did not exit normally",
    );
    write_status(out, &status)
}
/// The CPU allocation policy recorded with an experimental observation.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ConcurrencyPolicy {
    Independent,
    Fixed,
}

#[derive(Clone, Debug)]
struct ExperimentWorker {
    partition: WorkerPartition,
    profile: PathBuf,
    junit: PathBuf,
    extract: PathBuf,
    filter_config: Option<PathBuf>,
    filter_expression: Option<String>,
    arguments: Vec<String>,
    threads: Option<String>,
}

fn experiment_workers(
    strategy: ExperimentStrategy,
    policy: ConcurrencyPolicy,
    backend_identities: Option<&[(String, String)]>,
    archive: &Path,
) -> Result<Vec<ExperimentWorker>> {
    let strategy_name = match strategy {
        ExperimentStrategy::Slice => "slice",
        ExperimentStrategy::Hash => "hash",
        ExperimentStrategy::Backend => "backend",
        ExperimentStrategy::Baseline => bail!("baseline has no partition workers"),
    };
    let workspace_root = std::env::current_dir()?;
    let profile_root = workspace_root.join("target");
    let extract_root = workspace_root.join("target/coverage-experiment-extract");
    let filter_root = workspace_root.join("target/coverage-experiment-filter");
    let fixed_threads = match policy {
        ConcurrencyPolicy::Independent => None,
        ConcurrencyPolicy::Fixed => Some(fixed_worker_threads()?.to_string()),
    };
    let backend_filters = match strategy {
        ExperimentStrategy::Backend => Some(backend_filters(
            backend_identities.context("missing backend identities")?,
        )?),
        _ => None,
    };
    Ok((1..=2)
        .map(|index| {
            let mut arguments = vec![
                "nextest".into(),
                "run".into(),
                "--profile".into(),
                format!("coverage-worker-{index}"),
                "--no-fail-fast".into(),
                "--archive-file".into(),
                archive.display().to_string(),
                "--workspace-remap".into(),
                workspace_root.display().to_string(),
                "--extract-to".into(),
                extract_root.join(index.to_string()).display().to_string(),
            ];
            let threads: Option<String> = fixed_threads.clone();
            match strategy {
                ExperimentStrategy::Slice | ExperimentStrategy::Hash => {
                    arguments.extend(["--partition".into(), format!("{strategy_name}:{index}/2")]);
                }
                ExperimentStrategy::Backend => {
                    let config = filter_root.join(format!("coverage-worker-{index}.toml"));
                    arguments.extend([
                        "--tool-config-file".into(),
                        format!("jaunder:{}", config.display()),
                    ]);
                }
                ExperimentStrategy::Baseline => unreachable!("baseline has no workers"),
            }
            if let Some(threads) = &threads {
                arguments.extend(["--test-threads".into(), threads.clone()]);
            }
            ExperimentWorker {
                partition: WorkerPartition {
                    strategy,
                    index,
                    total: 2,
                },
                profile: profile_root
                    .join(format!("coverage-experiment-worker-{index}-%m-%p.profraw")),
                junit: PathBuf::from(format!("/tmp/jaunder-coverage-worker-{index}-junit.xml")),
                extract: extract_root.join(index.to_string()),
                filter_config: (strategy == ExperimentStrategy::Backend)
                    .then(|| filter_root.join(format!("coverage-worker-{index}.toml"))),
                filter_expression: backend_filters
                    .as_ref()
                    .map(|filters| filters[usize::from(index - 1)].clone()),
                arguments,
                threads,
            }
        })
        .collect())
}

/// Classify authoritative binary/test pairs into two complete measurement-only assignments.
fn backend_filters(identities: &[(String, String)]) -> Result<[String; 2]> {
    let mut assignments = [Vec::new(), Vec::new()];
    for (binary, name) in identities {
        if !filter_atom_is_safe(binary) || !filter_atom_is_safe(name) {
            bail!("backend experiment cannot safely represent identity {binary}::{name}");
        }
        let identity = format!("{binary}::{name}");
        let worker = if name.ends_with("_sqlite") {
            0
        } else if name.ends_with("_postgres") {
            1
        } else {
            usize::from(stable_assignment(&identity))
        };
        assignments[worker].push((binary.as_str(), name.as_str()));
    }
    if assignments.iter().any(Vec::is_empty) {
        bail!("backend experiment has an empty worker assignment");
    }
    Ok([
        nextest_test_filter(&assignments[0]),
        nextest_test_filter(&assignments[1]),
    ])
}

fn fixed_worker_threads() -> Result<usize> {
    let budget = std::thread::available_parallelism()
        .context("detecting available parallelism")?
        .get();
    fixed_threads_for_budget(budget)
}

fn fixed_threads_for_budget(budget: usize) -> Result<usize> {
    if budget < 2 {
        bail!("fixed two-worker concurrency requires at least two CPUs");
    }
    Ok(budget / 2)
}

fn stable_assignment(identity: &str) -> u8 {
    identity
        .bytes()
        .fold(0_u8, |sum, byte| sum.wrapping_add(byte))
        % 2
}

fn filter_atom_is_safe(atom: &str) -> bool {
    !atom.is_empty()
        && atom.bytes().all(|byte| {
            byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b':' | b'-' | b'.' | b'/')
        })
}

fn nextest_test_filter(tests: &[(&str, &str)]) -> String {
    tests
        .iter()
        .map(|(binary, name)| {
            let binary = binary.replace('/', r"\/");
            format!("(binary_id(={binary}) & test(={name}))")
        })
        .collect::<Vec<_>>()
        .join(" | ")
}

fn parse_backend_identities(input: &str) -> Result<Vec<(String, String)>> {
    let value: Value = serde_json::from_str(input)?;
    let suites = value["rust-suites"]
        .as_object()
        .context("missing rust-suites")?;
    let mut identities = Vec::new();
    for suite in suites.values() {
        let binary = suite["binary-id"].as_str().context("missing binary-id")?;
        let tests = suite["testcases"]
            .as_object()
            .context("missing testcases")?;
        identities.extend(
            tests
                .keys()
                .map(|name| (binary.to_owned(), name.to_owned())),
        );
    }
    if identities.is_empty() {
        bail!("empty backend identity census");
    }
    Ok(identities)
}

fn aggregate_worker_outcome(workers: &[WorkerEvidence]) -> ProcessOutcome {
    workers
        .iter()
        .map(|worker| &worker.outcome)
        .find(|outcome| {
            matches!(
                outcome,
                ProcessOutcome::Signal
                    | ProcessOutcome::SpawnError { .. }
                    | ProcessOutcome::EvidenceError { .. }
            )
        })
        .or_else(|| {
            workers
                .iter()
                .map(|worker| &worker.outcome)
                .find(|outcome| outcome.is_failure())
        })
        .cloned()
        .unwrap_or_else(ProcessOutcome::success)
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum WorkerResultProblem {
    UnclassifiedFailure,
    FailureReportedAfterSuccess,
}

fn worker_result_problem(
    workers: &[WorkerEvidence],
    failed_tests: &[String],
) -> Option<WorkerResultProblem> {
    if workers
        .iter()
        .any(|worker| !worker.outcome.is_success() && worker.census.failed.is_empty())
    {
        Some(WorkerResultProblem::UnclassifiedFailure)
    } else if workers.iter().all(|worker| worker.outcome.is_success()) && !failed_tests.is_empty() {
        Some(WorkerResultProblem::FailureReportedAfterSuccess)
    } else {
        None
    }
}

fn worker_command(worker: &ExperimentWorker) -> Command {
    let script = r#"environment="$(cargo llvm-cov show-env --export-prefix)" || exit; eval "$environment" || exit; LLVM_PROFILE_FILE="$1"; export LLVM_PROFILE_FILE; shift; exec "$@""#;
    let mut command = Command::new("sh");
    command.arg("-c").arg(script).arg("--").arg(&worker.profile);
    command.args(["cargo"]);
    command.args(&worker.arguments);
    command
}

fn extract_experiment_archive(archive: &Path, workspace_root: &Path) -> Result<()> {
    let mut command = Command::new("tar");
    command
        .args(["--zstd", "-xf"])
        .arg(archive)
        .arg("-C")
        .arg(workspace_root);
    let captured = run_capture(&mut command).context("extracting coverage experiment archive")?;
    if !captured.status.success() {
        bail!(
            "extracting coverage experiment archive: {}",
            captured.output
        );
    }
    Ok(())
}

fn clear_experiment_profiles(profile_root: &Path) -> Result<()> {
    if !profile_root.is_dir() {
        return Ok(());
    }
    for entry in fs::read_dir(profile_root)? {
        let entry = entry?;
        if entry
            .file_name()
            .to_string_lossy()
            .starts_with("coverage-experiment-worker-")
        {
            fs::remove_file(entry.path())?;
        }
    }
    Ok(())
}

fn remove_experiment_inputs(workers: &[ExperimentWorker]) -> Result<()> {
    for worker in workers {
        if let Err(error) = fs::remove_dir_all(&worker.extract)
            && error.kind() != ErrorKind::NotFound
        {
            return Err(error).context("clearing experimental archive extraction");
        }
        if let Some(config) = &worker.filter_config
            && let Err(error) = fs::remove_file(config)
            && error.kind() != ErrorKind::NotFound
        {
            return Err(error).context("clearing experimental filter config");
        }
    }
    Ok(())
}

fn create_experiment_inputs(workers: &[ExperimentWorker]) -> Result<()> {
    for worker in workers {
        fs::create_dir_all(&worker.extract)
            .context("creating experimental archive extraction directory")?;
        if let (Some(config), Some(expression)) = (&worker.filter_config, &worker.filter_expression)
        {
            if let Some(parent) = config.parent() {
                fs::create_dir_all(parent)
                    .context("creating experimental filter config directory")?;
            }
            fs::write(
                config,
                format!(
                    "[profile.coverage-worker-{}]\ndefault-filter = '{}'\n",
                    worker.partition.index, expression
                ),
            )
            .context("writing experimental filter config")?;
        }
    }
    Ok(())
}

fn remove_experiment_paths(workers: &[ExperimentWorker]) -> Result<()> {
    remove_experiment_inputs(workers)?;
    for worker in workers {
        if let Err(error) = fs::remove_file(&worker.junit)
            && error.kind() != ErrorKind::NotFound
        {
            return Err(error).context("clearing experimental JUnit report");
        }
        let parent = worker
            .profile
            .parent()
            .context("experimental profile parent")?;
        clear_experiment_profiles(parent)?;
    }
    Ok(())
}
fn discovered_profiles(pattern: &Path) -> Vec<String> {
    let Some(parent) = pattern.parent() else {
        return Vec::new();
    };
    let Some(prefix) = pattern
        .file_name()
        .and_then(|name| name.to_str())
        .and_then(|name| name.split('%').next())
    else {
        return Vec::new();
    };
    let mut profiles = fs::read_dir(parent)
        .ok()
        .into_iter()
        .flatten()
        .flatten()
        .filter_map(|entry| {
            let path = entry.path();
            (path.is_file()
                && path
                    .extension()
                    .is_some_and(|extension| extension == "profraw")
                && path
                    .file_name()
                    .is_some_and(|name| name.to_string_lossy().starts_with(prefix)))
            .then(|| path.display().to_string())
        })
        .collect::<Vec<_>>();
    profiles.sort_unstable();
    profiles
}

fn run_worker(
    mut command: Command,
    worker: ExperimentWorker,
    diagnostics: PathBuf,
) -> WorkerEvidence {
    let started = Instant::now();
    let captured = run_capture(&mut command);
    let (outcome, output) = match captured {
        Ok(captured) => (command_outcome(captured.status), captured.output),
        Err(_) => (
            ProcessOutcome::SpawnError {
                spawn_error: "could not spawn command".into(),
            },
            "could not spawn command".into(),
        ),
    };
    let log = diagnostics.join(format!("worker-{}.log", worker.partition.index));
    write_diagnostic(
        &diagnostics,
        &format!("worker-{}.log", worker.partition.index),
        &output,
    );
    let junit_copy = diagnostics.join(format!("worker-{}.junit.xml", worker.partition.index));
    let census = match fs::read_to_string(&worker.junit)
        .and_then(|input| status::parse_junit_census(&input).map_err(std::io::Error::other))
    {
        Ok(census) => {
            let _ = fs::copy(&worker.junit, &junit_copy);
            census
        }
        Err(_) => TestCensus::default(),
    };
    WorkerEvidence {
        version: WORKER_EVIDENCE_VERSION,
        partition: worker.partition,
        outcome,
        census,
        profile_artifacts: discovered_profiles(&worker.profile),
        duration_ms: started.elapsed().as_millis(),
        diagnostics: vec![log.display().to_string()],
    }
}

fn write_worker_evidence(diag: &Path, workers: &[WorkerEvidence]) -> Result<()> {
    for worker in workers {
        fs::write(
            diag.join(format!("worker-{}.json", worker.partition.index)),
            worker.to_json(),
        )
        .context("writing worker evidence")?;
    }
    Ok(())
}

fn write_aggregate_evidence(diag: &Path, aggregate: &AggregateEvidence) -> Result<()> {
    fs::write(
        diag.join("aggregate.json"),
        format!("{}\n", serde_json::to_string_pretty(aggregate)?),
    )
    .context("writing aggregate worker evidence")
}

fn failed_worker(worker: ExperimentWorker, diagnostics: PathBuf, detail: &str) -> WorkerEvidence {
    let log = diagnostics.join(format!("worker-{}.log", worker.partition.index));
    write_diagnostic(
        &diagnostics,
        &format!("worker-{}.log", worker.partition.index),
        detail,
    );
    WorkerEvidence {
        version: WORKER_EVIDENCE_VERSION,
        partition: worker.partition,
        outcome: ProcessOutcome::SpawnError {
            spawn_error: detail.into(),
        },
        census: TestCensus::default(),
        profile_artifacts: discovered_profiles(&worker.profile),
        duration_ms: 0,
        diagnostics: vec![log.display().to_string()],
    }
}

fn join_worker_handles(
    handles: Vec<(ExperimentWorker, thread::JoinHandle<WorkerEvidence>)>,
    diagnostics: &Path,
) -> Vec<WorkerEvidence> {
    handles
        .into_iter()
        .map(|(worker, handle)| {
            handle.join().unwrap_or_else(|_| {
                failed_worker(worker, diagnostics.to_path_buf(), "worker thread panicked")
            })
        })
        .collect()
}

/// Run one explicitly selected, non-production coverage experiment.
pub fn run_experiment(
    out: &str,
    strategy: ExperimentStrategy,
    policy: ConcurrencyPolicy,
    archive: &Path,
) -> Result<()> {
    if strategy == ExperimentStrategy::Baseline {
        return run(out);
    }
    let archive = fs::canonicalize(archive)
        .with_context(|| format!("canonicalizing archive {}", archive.display()))?;
    if !archive.is_file() {
        bail!("coverage experiment archive is not a file");
    }
    fs::create_dir_all(out).with_context(|| format!("creating {out}"))?;
    let out = fs::canonicalize(out).with_context(|| format!("canonicalizing {out}"))?;
    let diag = out.join("diagnostics");
    fs::create_dir_all(&diag).with_context(|| format!("creating {}", diag.display()))?;
    let mut coverage_status = new_status();
    write_diagnostic(
        &diag,
        "experiment-policy.txt",
        &format!("strategy={strategy:?}\nconcurrency={policy:?}\n"),
    );

    let commands = required_stage_commands();
    let workspace = commands
        .iter()
        .find(|command| command.stage == RequiredStage::WorkspaceResolution)
        .expect("coverage workspace command");
    let workspace_started = Instant::now();
    let mut workspace_command = Command::new(workspace.program);
    workspace_command.args(&workspace.arguments);
    if command_failure(
        &mut coverage_status,
        RequiredStage::WorkspaceResolution,
        run_capture(&mut workspace_command),
        &diag,
        "metadata.log",
    )
    .is_none()
    {
        record_duration(
            &mut coverage_status,
            RequiredStage::WorkspaceResolution,
            workspace_started,
        );
        write_status(&out, &coverage_status)?;
        return Ok(());
    }
    record_duration(
        &mut coverage_status,
        RequiredStage::WorkspaceResolution,
        workspace_started,
    );

    let census_command = commands
        .iter()
        .find(|command| command.stage == RequiredStage::TestCensus)
        .expect("coverage census command");
    let census_started = Instant::now();
    let mut command = Command::new(census_command.program);
    command.args(&census_command.arguments);
    let Some(census) = command_failure(
        &mut coverage_status,
        RequiredStage::TestCensus,
        run_capture(&mut command),
        &diag,
        "nextest-list.json",
    ) else {
        record_duration(
            &mut coverage_status,
            RequiredStage::TestCensus,
            census_started,
        );
        write_status(&out, &coverage_status)?;
        return Ok(());
    };
    let expected = match status::parse_nextest_census(&census.stdout) {
        Ok(expected) => expected,
        Err(_) => {
            record_evidence_error(
                &mut coverage_status,
                RequiredStage::TestCensus,
                "invalid nextest census",
            );
            record_duration(
                &mut coverage_status,
                RequiredStage::TestCensus,
                census_started,
            );
            write_status(&out, &coverage_status)?;
            return Ok(());
        }
    };
    coverage_status.population.expected = expected.expected.len();
    set_stage(
        &mut coverage_status,
        RequiredStage::TestCensus,
        ProcessOutcome::success(),
    );
    record_duration(
        &mut coverage_status,
        RequiredStage::TestCensus,
        census_started,
    );

    let backend_identities = match strategy {
        ExperimentStrategy::Backend => match parse_backend_identities(&census.stdout) {
            Ok(identities) => Some(identities),
            Err(_) => {
                record_evidence_error(
                    &mut coverage_status,
                    RequiredStage::TestCensus,
                    "invalid backend identity census",
                );
                write_status(&out, &coverage_status)?;
                return Ok(());
            }
        },
        _ => None,
    };
    let workers =
        match experiment_workers(strategy, policy, backend_identities.as_deref(), &archive) {
            Ok(workers) => workers,
            Err(_) => {
                record_evidence_error(
                    &mut coverage_status,
                    RequiredStage::TestCensus,
                    "ambiguous backend experiment assignment",
                );
                write_status(&out, &coverage_status)?;
                return Ok(());
            }
        };
    write_diagnostic(
        &diag,
        "experiment-workers.txt",
        &workers
            .iter()
            .map(|worker| {
                format!(
                    "worker={} junit={} profile={} test_threads={}\n",
                    worker.partition.index,
                    worker.junit.display(),
                    worker.profile.display(),
                    worker.threads.as_deref().unwrap_or("nextest-default"),
                )
            })
            .collect::<String>(),
    );
    let cleanup_started = Instant::now();
    if command_failure(
        &mut coverage_status,
        RequiredStage::ProfileCleanup,
        run_capture(Command::new("cargo").args(["llvm-cov", "clean", "--profraw-only"])),
        &diag,
        "profile-cleanup.log",
    )
    .is_none()
        || remove_experiment_paths(&workers).is_err()
        || create_experiment_inputs(&workers).is_err()
    {
        if coverage_status
            .stages
            .iter()
            .any(|stage| stage.stage == RequiredStage::ProfileCleanup && stage.outcome.is_success())
        {
            record_evidence_error(
                &mut coverage_status,
                RequiredStage::ProfileCleanup,
                "could not clear experiment-owned paths",
            );
        }
        record_duration(
            &mut coverage_status,
            RequiredStage::ProfileCleanup,
            cleanup_started,
        );
        write_status(&out, &coverage_status)?;
        return Ok(());
    }
    set_stage(
        &mut coverage_status,
        RequiredStage::ProfileCleanup,
        ProcessOutcome::success(),
    );
    record_duration(
        &mut coverage_status,
        RequiredStage::ProfileCleanup,
        cleanup_started,
    );

    let run_started = Instant::now();
    let records = pg::with_ephemeral(|env| {
        let mut handles = Vec::with_capacity(workers.len());
        let mut records = Vec::new();
        for worker in workers.iter().cloned() {
            let mut command = worker_command(&worker);
            env.configure_command(&mut command);
            let diagnostics = diag.clone();
            let record_worker = worker.clone();
            match thread::Builder::new()
                .name(format!("coverage-worker-{}", worker.partition.index))
                .spawn(move || run_worker(command, record_worker, diagnostics))
            {
                Ok(handle) => handles.push((worker, handle)),
                Err(_) => records.push(failed_worker(
                    worker,
                    diag.clone(),
                    "could not spawn worker thread",
                )),
            }
        }
        records.extend(join_worker_handles(handles, &diag));
        Ok(records)
    });
    let records = match records {
        Ok(records) => records,
        Err(_) => workers
            .iter()
            .cloned()
            .map(|worker| {
                failed_worker(worker, diag.clone(), "could not start ephemeral PostgreSQL")
            })
            .collect(),
    };
    let worker_outcome = aggregate_worker_outcome(&records);
    set_stage(
        &mut coverage_status,
        RequiredStage::InstrumentedTestRun,
        worker_outcome,
    );
    if remove_experiment_inputs(&workers).is_err() {
        record_infra(
            &mut coverage_status,
            RequiredStage::InstrumentedTestRun,
            "could not clear experimental worker inputs",
        );
        let _ = write_worker_evidence(&diag, &records);
        record_duration(
            &mut coverage_status,
            RequiredStage::InstrumentedTestRun,
            run_started,
        );
        write_status(&out, &coverage_status)?;
        return Ok(());
    }
    record_duration(
        &mut coverage_status,
        RequiredStage::InstrumentedTestRun,
        run_started,
    );
    if write_worker_evidence(&diag, &records).is_err() {
        record_evidence_error(
            &mut coverage_status,
            RequiredStage::PopulationReconciliation,
            "could not write worker evidence",
        );
        write_status(&out, &coverage_status)?;
        return Ok(());
    }
    let reconciliation_started = Instant::now();
    let aggregate = aggregate_terminal(strategy, &expected, &records);
    coverage_status.population.executed = aggregate.population.executed.len();
    coverage_status.population.ignored = aggregate.population.ignored.len();
    coverage_status.failed_tests = aggregate.population.failed.clone();
    if write_aggregate_evidence(&diag, &aggregate).is_err() {
        record_evidence_error(
            &mut coverage_status,
            RequiredStage::PopulationReconciliation,
            "could not write aggregate worker evidence",
        );
        record_duration(
            &mut coverage_status,
            RequiredStage::PopulationReconciliation,
            reconciliation_started,
        );
        write_status(&out, &coverage_status)?;
        return Ok(());
    }
    if let coverage::workers::Reconciliation::Error { detail } = &aggregate.reconciliation {
        write_diagnostic(&diag, "aggregate-error.txt", detail);
        record_evidence_error(
            &mut coverage_status,
            RequiredStage::PopulationReconciliation,
            "worker evidence did not reconcile",
        );
        record_duration(
            &mut coverage_status,
            RequiredStage::PopulationReconciliation,
            reconciliation_started,
        );
        write_status(&out, &coverage_status)?;
        return Ok(());
    }
    set_stage(
        &mut coverage_status,
        RequiredStage::PopulationReconciliation,
        ProcessOutcome::success(),
    );
    record_duration(
        &mut coverage_status,
        RequiredStage::PopulationReconciliation,
        reconciliation_started,
    );
    match worker_result_problem(&aggregate.workers, &coverage_status.failed_tests) {
        Some(WorkerResultProblem::UnclassifiedFailure) => {
            record_infra(
                &mut coverage_status,
                RequiredStage::InstrumentedTestRun,
                "worker command failed without classified test failure",
            );
            write_status(&out, &coverage_status)?;
            return Ok(());
        }
        Some(WorkerResultProblem::FailureReportedAfterSuccess) => {
            record_evidence_error(
                &mut coverage_status,
                RequiredStage::InstrumentedTestRun,
                "JUnit reported failures after successful worker commands",
            );
            write_status(&out, &coverage_status)?;
            return Ok(());
        }
        None => {}
    }
    let instrumented_outcome = coverage_status
        .stages
        .iter()
        .find(|stage| stage.stage == RequiredStage::InstrumentedTestRun)
        .expect("instrumented test stage")
        .outcome
        .clone();
    let root = std::env::current_dir()?.to_string_lossy().into_owned();
    emit_reports(&out, &diag, &mut coverage_status, &root, &root)?;
    if coverage_status.stages.iter().any(|stage| {
        matches!(
            stage.stage,
            RequiredStage::TextReport | RequiredStage::LcovReport | RequiredStage::CrapReport
        ) && !stage.outcome.is_success()
    }) {
        return Ok(());
    }
    complete_status(
        &mut coverage_status,
        &instrumented_outcome,
        "test command did not exit normally",
    );
    write_status(&out, &coverage_status)
}
/// Execute exactly one selected worker from a read-only instrumented archive.
///
/// This is the CI measurement seam: unlike [`run_experiment`], it neither starts
/// a sibling worker nor emits a report. Its bundle is self-contained so an
/// aggregate on another runner can retain the terminal evidence, raw profiles,
/// and archive extraction required for one merged report.
pub fn run_experiment_worker(
    out: &str,
    strategy: ExperimentStrategy,
    index: u8,
    policy: ConcurrencyPolicy,
    census: &Path,
    archive: &Path,
) -> Result<()> {
    if strategy == ExperimentStrategy::Baseline {
        bail!("baseline is not a two-worker experiment");
    }
    let authoritative_census =
        fs::read_to_string(census).with_context(|| format!("reading {}", census.display()))?;
    let expected = status::parse_nextest_census(&authoritative_census)
        .context("parsing authoritative nextest census")?;
    let backend_identities = (strategy == ExperimentStrategy::Backend)
        .then(|| parse_backend_identities(&authoritative_census))
        .transpose()?;
    if expected.expected.is_empty() {
        bail!("authoritative nextest census is empty");
    }
    let archive = fs::canonicalize(archive)
        .with_context(|| format!("canonicalizing archive {}", archive.display()))?;
    if !archive.is_file() {
        bail!("coverage experiment archive is not a file");
    }
    let worker = experiment_workers(strategy, policy, backend_identities.as_deref(), &archive)?
        .into_iter()
        .find(|worker| worker.partition.index == index)
        .with_context(|| format!("worker index {index} is not one of 1 or 2"))?;
    fs::create_dir_all(out).with_context(|| format!("creating {out}"))?;
    let out = fs::canonicalize(out).with_context(|| format!("canonicalizing {out}"))?;
    let diagnostics = out.join("diagnostics");
    fs::create_dir_all(&diagnostics)?;
    remove_experiment_paths(std::slice::from_ref(&worker))?;
    extract_experiment_archive(&archive, &std::env::current_dir()?)?;
    create_experiment_inputs(std::slice::from_ref(&worker))?;
    let mut command = worker_command(&worker);
    let evidence = match pg::with_ephemeral(|env| {
        env.configure_command(&mut command);
        Ok(run_worker(command, worker.clone(), diagnostics.clone()))
    }) {
        Ok(evidence) => evidence,
        Err(_) => failed_worker(
            worker.clone(),
            diagnostics.clone(),
            "could not start ephemeral PostgreSQL",
        ),
    };
    let profiles = out.join("profiles");
    fs::create_dir_all(&profiles)?;
    let mut evidence = evidence;
    let mut retained = Vec::new();
    for profile in &evidence.profile_artifacts {
        let source = Path::new(profile);
        let name = source
            .file_name()
            .context("worker profile has no filename")?;
        let destination = profiles.join(name);
        fs::copy(source, &destination).with_context(|| format!("copying {}", source.display()))?;
        retained.push(format!("profiles/{}", name.to_string_lossy()));
    }
    evidence.profile_artifacts = retained;
    if worker.junit.is_file() {
        fs::copy(&worker.junit, out.join("junit.xml"))?;
    }
    fs::write(out.join("census.json"), authoritative_census)?;
    fs::write(out.join("worker.json"), evidence.to_json())?;
    remove_experiment_inputs(std::slice::from_ref(&worker))?;
    Ok(())
}

/// Aggregate two immutable worker bundles into the one per-ref coverage verdict.
pub fn aggregate_experiment_bundles(
    out: &str,
    census: &Path,
    archive: &Path,
    bundles: &[PathBuf],
) -> Result<()> {
    if bundles.len() != 2 {
        bail!("coverage aggregation requires exactly two worker bundles");
    }
    let archive = fs::canonicalize(archive)
        .with_context(|| format!("canonicalizing archive {}", archive.display()))?;
    if !archive.is_file() {
        bail!("coverage experiment archive is not a file");
    }
    let expected = status::parse_nextest_census(
        &fs::read_to_string(census).with_context(|| format!("reading {}", census.display()))?,
    )
    .context("parsing authoritative nextest census")?;
    fs::create_dir_all(out)?;
    let out = fs::canonicalize(out)?;
    let diagnostics = out.join("diagnostics");
    fs::create_dir_all(&diagnostics)?;
    let mut status = new_status();
    let started = Instant::now();
    set_stage(
        &mut status,
        RequiredStage::WorkspaceResolution,
        ProcessOutcome::success(),
    );
    record_duration(&mut status, RequiredStage::WorkspaceResolution, started);
    set_stage(
        &mut status,
        RequiredStage::TestCensus,
        ProcessOutcome::success(),
    );
    status.population.expected = expected.expected.len();
    let staging_started = Instant::now();
    let workspace_root = std::env::current_dir()?;
    let profile_root = workspace_root.join("target");
    clear_experiment_profiles(&profile_root)?;
    extract_experiment_archive(&archive, &workspace_root)?;
    fs::remove_file(&archive)
        .with_context(|| format!("removing consumed archive {}", archive.display()))?;
    let mut workers = Vec::with_capacity(2);
    for bundle in bundles {
        let worker = WorkerEvidence::from_json(
            &fs::read_to_string(bundle.join("worker.json"))
                .with_context(|| format!("reading worker bundle {}", bundle.display()))?,
        )
        .with_context(|| format!("invalid worker bundle {}", bundle.display()))?;
        for profile in &worker.profile_artifacts {
            let source = bundle.join(profile);
            let name = source
                .file_name()
                .context("worker profile has no filename")?;
            fs::hard_link(&source, profile_root.join(name))
                .with_context(|| format!("linking {}", source.display()))?;
        }
        workers.push(worker);
    }
    set_stage(
        &mut status,
        RequiredStage::ProfileCleanup,
        ProcessOutcome::success(),
    );
    record_duration(&mut status, RequiredStage::ProfileCleanup, staging_started);
    let aggregate = aggregate_terminal(
        workers.first().map_or(ExperimentStrategy::Slice, |worker| {
            worker.partition.strategy
        }),
        &expected,
        &workers,
    );
    write_aggregate_evidence(&diagnostics, &aggregate)?;
    status.population.executed = aggregate.population.executed.len();
    status.population.ignored = aggregate.population.ignored.len();
    status.failed_tests = aggregate.population.failed.clone();
    if let coverage::workers::Reconciliation::Error { detail } = &aggregate.reconciliation {
        write_diagnostic(&diagnostics, "aggregate-error.txt", detail);
        record_evidence_error(
            &mut status,
            RequiredStage::PopulationReconciliation,
            "worker evidence did not reconcile",
        );
        write_status(&out, &status)?;
        return Ok(());
    }
    set_stage(
        &mut status,
        RequiredStage::PopulationReconciliation,
        ProcessOutcome::success(),
    );
    let outcome = aggregate_worker_outcome(&workers);
    set_stage(
        &mut status,
        RequiredStage::InstrumentedTestRun,
        outcome.clone(),
    );
    if worker_result_problem(&workers, &status.failed_tests).is_some() {
        record_infra(
            &mut status,
            RequiredStage::InstrumentedTestRun,
            "worker command failed without complete classified evidence",
        );
        write_status(&out, &status)?;
        return Ok(());
    }
    let compiled_source_root = Path::new("/build/source");
    let report_root = if compiled_source_root.join("Cargo.toml").is_file() {
        compiled_source_root
    } else {
        &workspace_root
    };
    emit_reports(
        &out,
        &diagnostics,
        &mut status,
        &report_root.to_string_lossy(),
        &workspace_root.to_string_lossy(),
    )?;
    if status.stages.iter().any(|stage| {
        matches!(
            stage.stage,
            RequiredStage::TextReport | RequiredStage::LcovReport | RequiredStage::CrapReport
        ) && !stage.outcome.is_success()
    }) {
        return Ok(());
    }
    complete_status(
        &mut status,
        &outcome,
        "worker commands did not exit normally",
    );
    write_diagnostic(&diagnostics, "status-candidate.json", &status.to_json());
    write_status(&out, &status)
}

fn remap_lcov_source_root(raw: &str, from: &str, to: &str) -> String {
    let prefix = format!("SF:{from}/");
    let replacement = format!("SF:{to}/");
    raw.lines()
        .map(|line| {
            line.strip_prefix(&prefix)
                .map_or_else(|| line.to_owned(), |path| format!("{replacement}{path}"))
        })
        .collect::<Vec<_>>()
        .join("\n")
        + if raw.ends_with('\n') { "\n" } else { "" }
}

/// Strip the absolute sandbox prefix and external package entries from CRAP output.
fn normalize_crap_paths(raw: &str, abs_root: &str) -> Result<String> {
    let prefix = format!("{abs_root}/");
    let mut value: Value = serde_json::from_str(raw)?;
    let entries = value
        .get_mut("entries")
        .and_then(Value::as_array_mut)
        .context("missing CRAP entries")?;
    let mut normalized_entries = Vec::with_capacity(entries.len());
    for mut entry in std::mem::take(entries) {
        let file = entry
            .get("file")
            .and_then(Value::as_str)
            .context("missing CRAP entry file")?;
        let normalized_file = file.strip_prefix(&prefix).unwrap_or(file).to_owned();
        let is_external_package = is_csr_bundle_package_path(&normalized_file);
        entry["file"] = Value::String(normalized_file);
        if !is_external_package {
            normalized_entries.push(entry);
        }
    }
    *entries = normalized_entries;
    Ok(format!("{}\n", serde_json::to_string_pretty(&value)?))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn report_commands_exclude_only_csr_bundle_with_the_same_regex() {
        let text = coverage_report_arguments("--text");
        let lcov = coverage_report_arguments("--lcov");

        assert_eq!(
            text,
            [
                "-c",
                LLVM_COV_ENVIRONMENT_SCRIPT,
                "--",
                "cargo",
                "llvm-cov",
                "report",
                "--text",
                "--ignore-filename-regex",
                CSR_BUNDLE_FILENAME_REGEX,
            ]
        );
        assert_eq!(
            lcov,
            [
                "-c",
                LLVM_COV_ENVIRONMENT_SCRIPT,
                "--",
                "cargo",
                "llvm-cov",
                "report",
                "--lcov",
                "--ignore-filename-regex",
                CSR_BUNDLE_FILENAME_REGEX,
            ]
        );
        assert_eq!(text[8], lcov[8]);
    }
    #[test]
    fn required_commands_use_root_workspace_coverage_profile_without_filters() {
        let commands = required_stage_commands();
        let metadata = commands
            .iter()
            .find(|command| command.stage == RequiredStage::WorkspaceResolution)
            .expect("metadata command");
        assert_eq!(metadata.program, "cargo");
        assert_eq!(
            metadata.arguments,
            [
                "metadata",
                "--manifest-path",
                "Cargo.toml",
                "--format-version",
                "1",
                "--no-deps"
            ]
        );

        let census = commands
            .iter()
            .find(|command| command.stage == RequiredStage::TestCensus)
            .expect("census command");
        assert_eq!(census.program, "sh");
        assert_eq!(
            census.arguments,
            [
                "-c",
                LLVM_COV_ENVIRONMENT_SCRIPT,
                "--",
                "cargo",
                "nextest",
                "list",
                "--workspace",
                "--message-format",
                "json",
            ]
        );

        let run = commands
            .iter()
            .find(|command| command.stage == RequiredStage::InstrumentedTestRun)
            .expect("instrumented test command");
        assert_eq!(run.program, "sh");
        assert_eq!(
            run.arguments,
            [
                "-c",
                LLVM_COV_ENVIRONMENT_SCRIPT,
                "--",
                "cargo",
                "nextest",
                "run",
                "--workspace",
                "--profile",
                "coverage",
                "--no-fail-fast",
            ]
        );

        for arguments in [&census.arguments[3..], &run.arguments[3..]] {
            assert!(!arguments.iter().any(|argument| matches!(
                *argument,
                "-p" | "--package" | "--test" | "--partition" | "-E" | "--expr-filter"
            )));
        }
    }

    #[test]
    fn failed_census_is_authoritative_and_leaves_instrumented_run_not_run() {
        let status = status_for_required_stage_failure(
            RequiredStage::TestCensus,
            ProcessOutcome::ExitCode { exit_code: 1 },
        );

        assert_eq!(status.category, StatusCategory::Infra);
        assert_eq!(
            status
                .stages
                .iter()
                .find(|result| result.stage == RequiredStage::TestCensus)
                .expect("census stage")
                .outcome,
            ProcessOutcome::ExitCode { exit_code: 1 }
        );
        assert_eq!(
            status
                .stages
                .iter()
                .find(|result| result.stage == RequiredStage::InstrumentedTestRun)
                .expect("instrumented test stage")
                .outcome,
            ProcessOutcome::NotRun
        );
        assert!(status.validate().is_ok());
    }

    #[test]
    fn metadata_exit_101_without_a_fail_line_is_never_tests_ok() {
        let status = status_for_required_stage_failure(
            RequiredStage::WorkspaceResolution,
            ProcessOutcome::ExitCode { exit_code: 101 },
        );
        assert_eq!(status.category, StatusCategory::Infra);
        assert_ne!(status.category, StatusCategory::TestsOk);
        assert!(status.validate().is_ok());
    }

    #[test]
    fn checked_nonzero_and_spawn_outcomes_are_red_at_every_required_stage() {
        for stage in RequiredStage::ALL {
            for outcome in [
                ProcessOutcome::ExitCode { exit_code: 101 },
                ProcessOutcome::SpawnError {
                    spawn_error: "not-found".into(),
                },
            ] {
                let status = status_for_required_stage_failure(stage, outcome);
                assert_eq!(status.category, StatusCategory::Infra, "{}", stage.as_str());
                assert!(status.validate().is_ok(), "{}", stage.as_str());
            }
        }
    }

    #[test]
    fn classified_test_failure_clears_provisional_infrastructure_detail() {
        let mut status = new_status();
        status.failed_tests.push("server::failed".into());

        complete_status(
            &mut status,
            &ProcessOutcome::ExitCode { exit_code: 100 },
            "worker commands did not exit normally",
        );

        assert_eq!(status.category, StatusCategory::TestFailure);
        assert_eq!(status.infra_detail, None);
    }

    #[test]
    fn signaled_instrumented_run_with_junit_failures_is_infrastructure() {
        assert_eq!(
            final_category(&ProcessOutcome::Signal, &["server::interrupted".into()]),
            StatusCategory::Infra
        );
    }

    #[test]
    fn remaps_only_lcov_source_paths_under_the_compiled_workspace() {
        let raw = "TN:\nSF:/build/source/server/src/lib.rs\nDA:1,1\nSF:/nix/store/pkg/src/lib.rs\nend_of_record\n";

        assert_eq!(
            remap_lcov_source_root(raw, "/build/source", "/checkout"),
            "TN:\nSF:/checkout/server/src/lib.rs\nDA:1,1\nSF:/nix/store/pkg/src/lib.rs\nend_of_record\n"
        );
    }

    #[test]
    fn normalizes_and_excludes_csr_bundle_crap_entries() {
        let raw = r#"{
            "entries": [
                {"file": "/build/source/tools/csr_bundle", "crap": 1.0},
                {"file": "/build/source/tools/csr_bundle/src/lib.rs", "crap": 2.0},
                {"file": "/build/source/tools/csr_bundle_extra/src/lib.rs", "crap": 3.0},
                {"file": "/build/source/server/src/a.rs", "crap": 4.0}
            ]
        }"#;

        let got = normalize_crap_paths(raw, "/build/source").expect("valid report");
        let value: Value = serde_json::from_str(&got).expect("normalized JSON");
        let files = value["entries"]
            .as_array()
            .expect("CRAP entries")
            .iter()
            .map(|entry| entry["file"].as_str().expect("CRAP entry file"))
            .collect::<Vec<_>>();

        assert_eq!(
            files,
            ["tools/csr_bundle_extra/src/lib.rs", "server/src/a.rs"]
        );
        assert!(normalize_crap_paths("{}", "/build/source").is_err());
    }

    #[test]
    fn partition_experiment_builds_exact_nextest_commands_and_isolated_paths() {
        let archive = Path::new("/tmp/instrumented-tests.tar.zst");
        let workers = experiment_workers(
            ExperimentStrategy::Slice,
            ConcurrencyPolicy::Fixed,
            None,
            archive,
        )
        .expect("slice workers");

        let workspace_root = std::env::current_dir().expect("workspace root");
        for (index, worker) in workers.iter().enumerate() {
            assert_eq!(
                worker.arguments[..13],
                [
                    "nextest",
                    "run",
                    "--profile",
                    &format!("coverage-worker-{}", index + 1),
                    "--no-fail-fast",
                    "--archive-file",
                    &archive.display().to_string(),
                    "--workspace-remap",
                    &workspace_root.display().to_string(),
                    "--extract-to",
                    &worker.extract.display().to_string(),
                    "--partition",
                    &format!("slice:{}/2", index + 1),
                ]
            );
            assert_eq!(
                worker.arguments[13..],
                [
                    "--test-threads",
                    &fixed_worker_threads().unwrap().to_string()
                ]
            );
        }
        assert_ne!(workers[0].junit, workers[1].junit);
        assert_ne!(workers[0].profile, workers[1].profile);
        assert_ne!(workers[0].extract, workers[1].extract);
        assert!(workers.iter().all(|worker| worker.junit.is_absolute()));
        assert!(workers.iter().all(|worker| worker.profile.is_absolute()));
        assert!(workers.iter().all(|worker| worker.extract.is_absolute()));
    }

    #[test]
    fn independent_workers_preserve_nextest_thread_default() {
        let workers = experiment_workers(
            ExperimentStrategy::Hash,
            ConcurrencyPolicy::Independent,
            None,
            Path::new("/tmp/instrumented-tests.tar.zst"),
        )
        .expect("hash workers");

        assert!(workers.iter().all(|worker| worker.threads.is_none()));
        assert!(
            workers
                .iter()
                .all(|worker| !worker.arguments.contains(&"--test-threads".into()))
        );
        assert_eq!(
            workers[0].arguments[11..13],
            ["--partition".to_owned(), "hash:1/2".to_owned()]
        );
        assert_eq!(
            workers[1].arguments[11..13],
            ["--partition".to_owned(), "hash:2/2".to_owned()]
        );
    }

    #[test]
    fn backend_comparator_rejects_unsafe_identity() {
        let error = backend_filters(&[
            ("bin".into(), "case_sqlite".into()),
            ("bin".into(), "case postgres".into()),
        ])
        .expect_err("unsafe identity cannot become an expression filter");
        assert!(
            error
                .to_string()
                .contains("cannot safely represent identity")
        );
    }

    #[test]
    fn backend_filters_escape_binary_paths_and_assign_every_identity() {
        let filters = backend_filters(&[
            ("jaunder::bin/jaunder".into(), "case_sqlite".into()),
            (
                "test-support::bin/test-support".into(),
                "case_postgres".into(),
            ),
            ("jaunder::bin/jaunder".into(), "ordinary::case".into()),
        ])
        .expect("complete assignments");
        let combined = format!("{} | {}", filters[0], filters[1]);
        assert!(combined.contains("binary_id(=jaunder::bin\\/jaunder)"));
        assert!(combined.contains("(binary_id(="));
        assert_eq!(combined.matches("case_sqlite").count(), 1);
        assert_eq!(combined.matches("case_postgres").count(), 1);
        assert_eq!(combined.matches("ordinary::case").count(), 1);
    }

    #[test]
    fn fixed_budget_never_oversubscribes_two_workers() {
        assert!(fixed_threads_for_budget(1).is_err());
        assert_eq!(fixed_threads_for_budget(2).unwrap(), 1);
        assert_eq!(fixed_threads_for_budget(3).unwrap(), 1);
        assert_eq!(fixed_threads_for_budget(8).unwrap(), 4);
    }

    fn worker_evidence(index: u8, outcome: ProcessOutcome) -> WorkerEvidence {
        WorkerEvidence {
            version: WORKER_EVIDENCE_VERSION,
            partition: WorkerPartition {
                strategy: ExperimentStrategy::Slice,
                index,
                total: 2,
            },
            outcome,
            census: TestCensus::default(),
            profile_artifacts: vec![format!("worker-{index}.profraw")],
            duration_ms: 10,
            diagnostics: vec![format!("worker-{index}.log")],
        }
    }

    #[test]
    fn aggregate_outcome_prioritizes_abnormal_worker_termination() {
        let workers = [
            worker_evidence(1, ProcessOutcome::ExitCode { exit_code: 100 }),
            worker_evidence(2, ProcessOutcome::Signal),
        ];

        assert_eq!(aggregate_worker_outcome(&workers), ProcessOutcome::Signal);
        assert_eq!(
            worker_result_problem(&workers, &["bin::failed".into()]),
            Some(WorkerResultProblem::UnclassifiedFailure)
        );

        let successful = [
            worker_evidence(1, ProcessOutcome::Success),
            worker_evidence(2, ProcessOutcome::Success),
        ];
        assert_eq!(
            worker_result_problem(&successful, &["bin::failed".into()]),
            Some(WorkerResultProblem::FailureReportedAfterSuccess)
        );
    }

    #[test]
    fn discovered_profiles_returns_only_sorted_matching_regular_files() {
        let dir = tempfile::tempdir().unwrap();
        let pattern = dir.path().join("worker-1-%m-%p.profraw");
        let first = dir.path().join("worker-1-b.profraw");
        let second = dir.path().join("worker-1-a.profraw");
        fs::write(&first, "").unwrap();
        fs::write(&second, "").unwrap();
        fs::write(dir.path().join("worker-2-a.profraw"), "").unwrap();
        fs::write(dir.path().join("worker-1-a.txt"), "").unwrap();
        fs::create_dir(dir.path().join("worker-1-dir.profraw")).unwrap();

        assert_eq!(
            discovered_profiles(&pattern),
            vec![second.display().to_string(), first.display().to_string(),]
        );
    }

    #[test]
    fn joining_workers_retains_a_returned_failure_and_a_thread_panic() {
        let dir = tempfile::tempdir().unwrap();
        let mut definitions = experiment_workers(
            ExperimentStrategy::Slice,
            ConcurrencyPolicy::Independent,
            None,
            Path::new("/tmp/instrumented-tests.tar.zst"),
        )
        .unwrap();
        let first = definitions.remove(0);
        let second = definitions.remove(0);
        let returned = worker_evidence(1, ProcessOutcome::ExitCode { exit_code: 100 });
        let returned_handle = thread::spawn(move || returned);
        let panicked_handle = thread::spawn(|| -> WorkerEvidence {
            panic!("controlled worker panic");
        });

        let records = join_worker_handles(
            vec![(first, returned_handle), (second, panicked_handle)],
            dir.path(),
        );

        assert_eq!(records.len(), 2);
        assert_eq!(
            records[0].outcome,
            ProcessOutcome::ExitCode { exit_code: 100 }
        );
        assert!(matches!(
            records[1].outcome,
            ProcessOutcome::SpawnError { ref spawn_error }
                if spawn_error == "worker thread panicked"
        ));
        assert_ne!(records[0].diagnostics, records[1].diagnostics);
    }

    #[test]
    fn worker_and_aggregate_evidence_are_persisted_for_consumers() {
        let dir = tempfile::tempdir().unwrap();
        let workers = vec![
            worker_evidence(1, ProcessOutcome::Success),
            worker_evidence(2, ProcessOutcome::Success),
        ];
        write_worker_evidence(dir.path(), &workers).unwrap();
        let aggregate = AggregateEvidence {
            strategy: ExperimentStrategy::Slice,
            population: TestCensus::default(),
            workers,
            reconciliation: coverage::workers::Reconciliation::Reconciled,
        };
        write_aggregate_evidence(dir.path(), &aggregate).unwrap();

        let first = fs::read_to_string(dir.path().join("worker-1.json")).unwrap();
        assert_eq!(
            WorkerEvidence::from_json(&first).unwrap(),
            aggregate.workers[0]
        );
        let aggregate_json = fs::read_to_string(dir.path().join("aggregate.json")).unwrap();
        assert_eq!(
            serde_json::from_str::<AggregateEvidence>(&aggregate_json).unwrap(),
            aggregate
        );
    }
    #[test]
    fn separate_runner_aggregate_rejects_missing_worker_bundle_before_consuming_inputs() {
        let error = aggregate_experiment_bundles(
            "unused-output",
            Path::new("missing-census.json"),
            Path::new("missing-archive.tar.zst"),
            &[PathBuf::from("only-worker")],
        )
        .unwrap_err();
        assert!(error.to_string().contains("exactly two worker bundles"));
    }
}
