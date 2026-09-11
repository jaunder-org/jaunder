use std::fs;
use std::io::ErrorKind;
use std::path::Path;
use std::process::{Command, ExitStatus};
use std::time::Instant;

use anyhow::{Context, Result};
use coverage::status::{
    self, CoverageStatus, Population, ProcessOutcome, RequiredStage, StageResult, StatusCategory,
    TestCensus,
};
use serde_json::Value;

use crate::pg;

const JUNIT_PATH: &str = "/tmp/jaunder-coverage-junit.xml";
const CSR_BUNDLE_FILENAME_REGEX: &str = r"(^|.*/)tools/csr_bundle/";
const CSR_BUNDLE_PACKAGE_PATH: &str = "tools/csr_bundle";

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
            program: "cargo",
            arguments: vec!["nextest", "list", "--workspace", "--message-format", "json"],
        },
        CommandSpec {
            stage: RequiredStage::InstrumentedTestRun,
            program: "cargo",
            arguments: vec![
                "llvm-cov",
                "--no-report",
                "nextest",
                "--workspace",
                "--profile",
                "coverage",
                "--no-fail-fast",
            ],
        },
    ]
}

fn coverage_report_arguments(format: &'static str) -> [&'static str; 5] {
    [
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

    let text_report_started = Instant::now();
    let text_report_result = report_command(
        RequiredStage::TextReport,
        Command::new("cargo").args(coverage_report_arguments("--text")),
        &mut status,
        &diag,
        "text-report.log",
    );
    let Some(report) = text_report_result else {
        record_duration(&mut status, RequiredStage::TextReport, text_report_started);
        write_status(out, &status)?;
        return Ok(());
    };
    let report = coverage::pathnorm::normalize_report_text(&report, &abs_root);
    if fs::write(out.join("coverage-report.txt"), report).is_err() {
        record_evidence_error(
            &mut status,
            RequiredStage::TextReport,
            "could not write text report",
        );
        record_duration(&mut status, RequiredStage::TextReport, text_report_started);
        write_status(out, &status)?;
        return Ok(());
    }
    record_duration(&mut status, RequiredStage::TextReport, text_report_started);

    let lcov_report_started = Instant::now();
    let lcov = out.join("coverage-report.lcov");
    let lcov_path = match lcov.to_str() {
        Some(path) => path,
        None => {
            record_evidence_error(
                &mut status,
                RequiredStage::LcovReport,
                "invalid LCOV report path",
            );
            record_duration(&mut status, RequiredStage::LcovReport, lcov_report_started);
            write_status(out, &status)?;
            return Ok(());
        }
    };
    let lcov_result = report_command(
        RequiredStage::LcovReport,
        Command::new("cargo")
            .args(coverage_report_arguments("--lcov"))
            .args(["--output-path", lcov_path]),
        &mut status,
        &diag,
        "lcov-report.log",
    );
    let Some(_) = lcov_result else {
        record_duration(&mut status, RequiredStage::LcovReport, lcov_report_started);
        write_status(out, &status)?;
        return Ok(());
    };
    record_duration(&mut status, RequiredStage::LcovReport, lcov_report_started);

    let crap_report_started = Instant::now();
    let raw_crap = out.join("crap-report.raw.json");
    let raw_crap_path = match raw_crap.to_str() {
        Some(path) => path,
        None => {
            record_evidence_error(
                &mut status,
                RequiredStage::CrapReport,
                "invalid CRAP report path",
            );
            record_duration(&mut status, RequiredStage::CrapReport, crap_report_started);
            write_status(out, &status)?;
            return Ok(());
        }
    };
    let crap_report_result = report_command(
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
        &mut status,
        &diag,
        "crap-report.log",
    );
    let Some(_) = crap_report_result else {
        record_duration(&mut status, RequiredStage::CrapReport, crap_report_started);
        write_status(out, &status)?;
        return Ok(());
    };
    let crap = match fs::read_to_string(&raw_crap) {
        Ok(raw) => match normalize_crap_paths(&raw, &abs_root) {
            Ok(crap) => crap,
            Err(_) => {
                record_evidence_error(
                    &mut status,
                    RequiredStage::CrapReport,
                    "invalid CRAP report",
                );
                record_duration(&mut status, RequiredStage::CrapReport, crap_report_started);
                write_status(out, &status)?;
                return Ok(());
            }
        },
        Err(_) => {
            record_evidence_error(
                &mut status,
                RequiredStage::CrapReport,
                "could not read CRAP report",
            );
            record_duration(&mut status, RequiredStage::CrapReport, crap_report_started);
            write_status(out, &status)?;
            return Ok(());
        }
    };
    if fs::write(out.join("crap-report.json"), crap).is_err() {
        record_evidence_error(
            &mut status,
            RequiredStage::CrapReport,
            "could not write CRAP report",
        );
        record_duration(&mut status, RequiredStage::CrapReport, crap_report_started);
        write_status(out, &status)?;
        return Ok(());
    }
    record_duration(&mut status, RequiredStage::CrapReport, crap_report_started);

    // Disk diagnostics are intentionally best-effort and cannot change status.
    if let Ok(disk) = run_capture(Command::new("df").arg("-h")) {
        write_diagnostic(&diag, "disk-usage.txt", &disk.output);
    }

    match final_category(&instrumented_outcome, &status.failed_tests) {
        category @ (StatusCategory::TestsOk | StatusCategory::TestFailure) => {
            status.category = category;
            status.infra_detail = None;
        }
        StatusCategory::Infra => record_infra(
            &mut status,
            RequiredStage::InstrumentedTestRun,
            "test command did not exit normally",
        ),
    }
    write_status(out, &status)
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
                "llvm-cov",
                "report",
                "--lcov",
                "--ignore-filename-regex",
                CSR_BUNDLE_FILENAME_REGEX,
            ]
        );
        assert_eq!(text[4], lcov[4]);
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
        for command in commands.iter().filter(|command| {
            matches!(
                command.stage,
                RequiredStage::TestCensus | RequiredStage::InstrumentedTestRun
            )
        }) {
            assert!(command.arguments.contains(&"--workspace"));
            assert!(!command.arguments.iter().any(|argument| matches!(
                *argument,
                "-p" | "--package" | "--test" | "--partition" | "-E" | "--expr-filter"
            )));
        }
        assert!(commands.iter().any(|command| {
            command.stage == RequiredStage::TestCensus
                && command
                    .arguments
                    .windows(2)
                    .any(|args| args == ["--message-format", "json"])
        }));
        assert!(commands.iter().any(|command| {
            command.stage == RequiredStage::InstrumentedTestRun
                && command
                    .arguments
                    .windows(3)
                    .any(|args| args == ["--profile", "coverage", "--no-fail-fast"])
        }));
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
    fn signaled_instrumented_run_with_junit_failures_is_infrastructure() {
        assert_eq!(
            final_category(&ProcessOutcome::Signal, &["server::interrupted".into()]),
            StatusCategory::Infra
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
}
