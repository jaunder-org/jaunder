//! CLI-surface tests for `devtool coverage validate-status`.

use std::fs;
use std::process::Command;

use coverage::status::{
    COVERAGE_STATUS_VERSION, CoverageStatus, Population, ProcessOutcome, RequiredStage,
    StageResult, StatusCategory,
};

const EXE: &str = env!("CARGO_BIN_EXE_devtool");

fn tmp() -> tempfile::TempDir {
    tempfile::Builder::new()
        .prefix("devtool-coverage-cli-")
        .tempdir()
        .unwrap()
}

fn tests_ok_status() -> CoverageStatus {
    CoverageStatus {
        version: COVERAGE_STATUS_VERSION,
        category: StatusCategory::TestsOk,
        stages: RequiredStage::ALL
            .into_iter()
            .map(|stage| StageResult {
                stage,
                outcome: ProcessOutcome::Success,
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

#[test]
fn coverage_validate_status_accepts_only_complete_tests_ok_evidence() {
    let t = tmp();
    let complete = t.path().join("complete.json");
    fs::write(&complete, tests_ok_status().to_json()).unwrap();
    assert!(
        Command::new(EXE)
            .args(["coverage", "validate-status", "--status"])
            .arg(&complete)
            .status()
            .unwrap()
            .success()
    );

    let mut unknown_version = tests_ok_status();
    unknown_version.version += 1;
    let mut contradiction = tests_ok_status();
    contradiction.infra_detail = Some("contradiction".into());
    let mut test_failure = tests_ok_status();
    test_failure.category = StatusCategory::TestFailure;
    test_failure.stages[3].outcome = ProcessOutcome::ExitCode { exit_code: 1 };
    test_failure.failed_tests = vec!["server::broken".into()];

    for (name, fixture) in [
        ("malformed", "{".to_owned()),
        ("unknown-version", unknown_version.to_json()),
        ("contradiction", contradiction.to_json()),
        ("test-failure", test_failure.to_json()),
    ] {
        let status = t.path().join(format!("{name}.json"));
        fs::write(&status, fixture).unwrap();
        let output = Command::new(EXE)
            .args(["coverage", "validate-status", "--status"])
            .arg(&status)
            .output()
            .unwrap();
        assert!(
            !output.status.success(),
            "{name} evidence must fail closed: {}",
            String::from_utf8_lossy(&output.stderr)
        );
    }
}
