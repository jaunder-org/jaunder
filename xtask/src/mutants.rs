//! Validate completed cargo-mutants evidence without running the producer.
//!
//! Decode the producer's verdict-bearing fields, reconcile phases, identities,
//! counts and category lists, then return one report. Producer metadata outside
//! that verdict is intentionally ignored; invocation policy stays in common.sh.

use std::collections::BTreeSet;
use std::fmt::Write;
use std::path::Path;

use anyhow::{Context, Result, ensure};
use serde::Deserialize;

#[derive(Deserialize)]
struct LabOutcome {
    outcomes: Vec<Outcome>,
    total_mutants: usize,
    missed: usize,
    caught: usize,
    unviable: usize,
    timeout: usize,
    success: usize,
    start_time: String,
    end_time: String,
}

#[derive(Deserialize)]
enum Scenario {
    Baseline,
    Mutant(Mutant),
}

#[derive(Deserialize)]
struct Mutant {
    package: String,
    file: String,
    name: String,
}

#[derive(Deserialize, PartialEq, Eq)]
enum Summary {
    Success,
    Failure,
    MissedMutant,
    CaughtMutant,
    Unviable,
    Timeout,
}

#[derive(Deserialize)]
struct Outcome {
    scenario: Scenario,
    summary: Summary,
    phase_results: Vec<PhaseResult>,
}

#[derive(Deserialize, PartialEq, Eq)]
enum Phase {
    Check,
    Build,
    Test,
}

#[derive(Deserialize, PartialEq, Eq)]
enum ProcessStatus {
    Success,
    Failure(i32),
    Timeout,
    Signalled(i32),
    Other,
}

impl ProcessStatus {
    fn failed(&self) -> bool {
        matches!(self, Self::Failure(code) if *code != 0)
    }
}

#[derive(Deserialize)]
struct PhaseResult {
    phase: Phase,
    duration: f64,
    process_status: ProcessStatus,
    argv: Vec<String>,
}

impl Outcome {
    fn phases_agree(&self) -> bool {
        if !self
            .phase_results
            .iter()
            .all(|p| p.duration.is_finite() && p.duration >= 0.0 && !p.argv.is_empty())
        {
            return false;
        }
        match self.phase_results.as_slice() {
            [build] if self.summary == Summary::Unviable => {
                build.phase == Phase::Build && build.process_status.failed()
            }
            [build, test] => {
                build.phase == Phase::Build
                    && build.process_status == ProcessStatus::Success
                    && test.phase == Phase::Test
                    && match self.summary {
                        Summary::CaughtMutant => test.process_status.failed(),
                        Summary::Success | Summary::MissedMutant => {
                            test.process_status == ProcessStatus::Success
                        }
                        _ => false,
                    }
            }
            _ => false,
        }
    }
}

/// Return authoritative text only after every required artifact agrees. The
/// shell owns producer execution; this seam never invokes or configures it.
pub(crate) fn verify_evidence(
    package: &str,
    file: &str,
    output: &Path,
    producer_status: i32,
) -> Result<String> {
    ensure!(
        matches!(producer_status, 0 | 2),
        "producer exited {producer_status}"
    );
    let artifacts = output.join("mutants.out");
    let lab: LabOutcome = serde_json::from_str(
        &std::fs::read_to_string(artifacts.join("outcomes.json"))
            .context("reading outcomes.json")?,
    )
    .context("decoding outcomes.json")?;
    let start: jiff::Timestamp = lab.start_time.parse().context("invalid start_time")?;
    let end: jiff::Timestamp = lab.end_time.parse().context("invalid end_time")?;
    ensure!(end >= start, "completion precedes start");

    let baselines: Vec<_> = lab
        .outcomes
        .iter()
        .filter(|o| matches!(o.scenario, Scenario::Baseline))
        .collect();
    ensure!(baselines.len() == 1, "expected one baseline");
    ensure!(
        baselines[0].summary == Summary::Success && baselines[0].phases_agree(),
        "baseline did not complete successful tests"
    );
    let mut identities = BTreeSet::new();
    let mut missed = Vec::new();
    let mut caught = Vec::new();
    let mut unviable = Vec::new();
    for outcome in &lab.outcomes {
        let Scenario::Mutant(mutant) = &outcome.scenario else {
            continue;
        };
        ensure!(
            mutant.package == package && mutant.file == file && !mutant.name.trim().is_empty(),
            "inapplicable mutant identity"
        );
        ensure!(identities.insert(&mutant.name), "duplicate mutant identity");
        ensure!(
            outcome.phases_agree(),
            "incomplete or inconsistent mutant phases"
        );
        match outcome.summary {
            Summary::MissedMutant => missed.push(mutant.name.as_str()),
            Summary::CaughtMutant => caught.push(mutant.name.as_str()),
            Summary::Unviable => unviable.push(mutant.name.as_str()),
            _ => anyhow::bail!("unsupported mutant outcome or timeout hole"),
        }
    }
    ensure!(
        lab.total_mutants == identities.len()
            && lab.missed == missed.len()
            && lab.caught == caught.len()
            && lab.unviable == unviable.len()
            && lab.timeout == 0
            && lab.success == 0,
        "inconsistent outcome records/counts"
    );
    ensure!(
        !identities.is_empty(),
        "no applicable mutants; target not verified"
    );
    ensure!(
        (producer_status == 2) == !missed.is_empty(),
        "producer exit contradicts outcomes"
    );
    for (category, names) in [
        ("missed", &missed),
        ("caught", &caught),
        ("unviable", &unviable),
        ("timeout", &Vec::new()),
    ] {
        let text = std::fs::read_to_string(artifacts.join(format!("{category}.txt")))
            .with_context(|| format!("reading {category}.txt"))?;
        let mut actual: Vec<_> = text.lines().collect();
        let mut expected = names.clone();
        actual.sort_unstable();
        expected.sort_unstable();
        ensure!(actual == expected, "{category}.txt contradicts outcomes");
    }
    let mut report = format!(
        "missed: {}\ncaught: {}\nunviable: {}\ntimeout: 0",
        missed.len(),
        caught.len(),
        unviable.len()
    );
    if !missed.is_empty() {
        write!(report, "\n\nSTILL ALIVE:\n{}", missed.join("\n"))?;
    }
    Ok(report)
}

#[cfg(test)]
mod tests {
    use super::*;

    use serde_json::{Value, json};

    fn completed(summary: &str) -> Value {
        let build = json!({"phase":"Build", "duration":0.1, "process_status":"Success", "argv":["cargo", "build"]});
        let test = json!({"phase":"Test", "duration":0.1, "process_status":"Success", "argv":["cargo", "nextest"]});
        let mut mutant_test = test.clone();
        if summary == "CaughtMutant" {
            mutant_test["process_status"] = json!({"Failure":100});
        }
        json!({
            "start_time":"2026-10-15T00:00:00.123456789Z",
            "end_time":"2026-10-15T00:00:00.123456790Z",
            "outcomes":[
                {"scenario":"Baseline", "summary":"Success", "phase_results":[build.clone(),test]},
                {"scenario":{"Mutant":{"package":"common", "file":"common/src/render.rs", "name":"common/src/render.rs:10:5: replace render with String::new()"}},
                 "summary":summary, "phase_results":[build,mutant_test]}
            ],
            "total_mutants":1, "missed":usize::from(summary == "MissedMutant"),
            "caught":usize::from(summary == "CaughtMutant"), "unviable":0,"timeout":0,"success":0
        })
    }

    fn check(value: &Value, status: i32) -> Result<String> {
        let root = tempfile::tempdir().unwrap();
        let artifacts = root.path().join("mutants.out");
        std::fs::create_dir(&artifacts).unwrap();
        std::fs::write(
            artifacts.join("outcomes.json"),
            serde_json::to_vec(value).unwrap(),
        )
        .unwrap();
        for (category, summary) in [
            ("missed", "MissedMutant"),
            ("caught", "CaughtMutant"),
            ("unviable", "Unviable"),
            ("timeout", "Timeout"),
        ] {
            let names: String = value["outcomes"]
                .as_array()
                .unwrap()
                .iter()
                .filter(|o| o["summary"] == summary)
                .map(|o| format!("{}\n", o["scenario"]["Mutant"]["name"].as_str().unwrap()))
                .collect();
            std::fs::write(artifacts.join(format!("{category}.txt")), names).unwrap();
        }
        verify_evidence("common", "common/src/render.rs", root.path(), status)
    }

    #[test]
    fn completed_results_preserve_counts_and_findings() {
        assert_eq!(
            check(&completed("CaughtMutant"), 0).unwrap(),
            "missed: 0\ncaught: 1\nunviable: 0\ntimeout: 0"
        );
        assert_eq!(
            check(&completed("MissedMutant"), 2).unwrap(),
            "missed: 1\ncaught: 0\nunviable: 0\ntimeout: 0\n\nSTILL ALIVE:\ncommon/src/render.rs:10:5: replace render with String::new()"
        );
    }

    #[test]
    fn timestamp_order_retains_nanosecond_precision() {
        let mut value = completed("CaughtMutant");
        assert!(check(&value, 0).is_ok());
        value["end_time"] = json!("2026-10-15T00:00:00.123456788Z");
        assert!(
            check(&value, 0)
                .unwrap_err()
                .to_string()
                .contains("completion precedes start")
        );
        for invalid in ["not a timestamp", "2026-02-30T00:00:00Z"] {
            value["end_time"] = json!(invalid);
            assert!(check(&value, 0).is_err());
        }
    }

    #[test]
    fn invalid_evidence_never_returns_a_report() {
        let original = completed("CaughtMutant");
        for (pointer, replacement) in [
            ("/end_time", Value::Null),
            ("/caught", json!(0)),
            ("/outcomes/0/summary", json!("Failure")),
            ("/outcomes/1/scenario/Mutant/package", json!("storage")),
            (
                "/outcomes/1/phase_results/1/process_status",
                json!("Timeout"),
            ),
        ] {
            let mut value = original.clone();
            *value.pointer_mut(pointer).unwrap() = replacement;
            assert!(check(&value, 0).is_err());
        }
        assert!(check(&original, 2).is_err());
        let mut empty = original.clone();
        empty["outcomes"].as_array_mut().unwrap().truncate(1);
        empty["total_mutants"] = json!(0);
        empty["caught"] = json!(0);
        assert!(
            check(&empty, 0)
                .unwrap_err()
                .to_string()
                .contains("no applicable mutants")
        );
    }

    #[test]
    fn tool_failure_is_non_verification_without_artifacts() {
        let root = tempfile::tempdir().unwrap();
        let error = verify_evidence("common", "common/src/render.rs", root.path(), 1).unwrap_err();
        assert!(error.to_string().contains("producer exited 1"));
    }
}
