//! Exercise the real mutation consumer; only cargo-mutants is substituted.

use serde_json::{Value, json};
use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::Path;
use std::process::{Command, Output};

fn verify(fixture: Option<&Path>, status: i32) -> Output {
    verify_with_binary(fixture, status, Path::new(env!("CARGO_BIN_EXE_xtask")))
}

fn verify_with_binary(fixture: Option<&Path>, status: i32, binary: &Path) -> Output {
    let temp = tempfile::tempdir().unwrap();
    let cargo = temp.path().join("cargo");
    fs::write(
        &cargo,
        r#"#!/usr/bin/env bash
set -eu
if [ "$1" = xtask ]; then
  shift
  exec "$XTASK_BIN" "$@"
fi
[ "$1" = mutants ] || exit 64
while [ "$#" -gt 0 ]; do
  if [ "$1" = --output ]; then
    shift
    out="$1"
  fi
  shift
done
if [ -n "$FIXTURE" ]; then
  cp -R "$FIXTURE" "$out/mutants.out"
fi
exit "$PRODUCER_STATUS"
"#,
    )
    .unwrap();
    fs::set_permissions(&cargo, fs::Permissions::from_mode(0o755)).unwrap();
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).parent().unwrap();
    let mut command = Command::new("bash");
    command
        .arg(root.join(".mutants-loop/verify.sh"))
        .args(["common", "common/src/render.rs"])
        .current_dir(root)
        .env(
            "PATH",
            format!(
                "{}:{}",
                temp.path().display(),
                std::env::var("PATH").unwrap()
            ),
        )
        .env("TMPDIR_MUTANTS", temp.path().join("scratch"))
        .env("FIXTURE", fixture.unwrap_or(Path::new("")))
        .env("PRODUCER_STATUS", status.to_string())
        .env("XTASK_BIN", binary);
    // Normal xtask startup touches Git hooks; never inherit a hook's index or
    // repository redirection into that startup.
    for key in [
        "GIT_DIR",
        "GIT_WORK_TREE",
        "GIT_INDEX_FILE",
        "GIT_OBJECT_DIRECTORY",
        "GIT_COMMON_DIR",
        "GIT_NAMESPACE",
    ] {
        command.env_remove(key);
    }
    command.output().unwrap()
}

fn assert_non_verification(output: Output) {
    let stdout = String::from_utf8(output.stdout).unwrap();
    let stderr = String::from_utf8(output.stderr).unwrap();
    assert!(
        !output.status.success(),
        "stdout: {stdout}\nstderr: {stderr}"
    );
    assert!(stderr.contains("verification failed"), "{stderr}");
    for count in ["missed:", "caught:", "unviable:", "timeout:", "STILL ALIVE"] {
        assert!(!stdout.contains(count), "{stdout}");
    }
}

#[test]
fn invalid_argument_without_artifacts_is_not_verification() {
    assert_non_verification(verify(None, 1));
}

#[test]
fn real_consumer_delegates_valid_and_invalid_evidence_to_native_command() {
    let fixture = fixture(&evidence("MissedMutant"));
    let valid = verify(Some(fixture.path()), 2);
    assert!(
        valid.status.success(),
        "{}",
        String::from_utf8_lossy(&valid.stderr)
    );
    assert!(
        String::from_utf8_lossy(&valid.stderr)
            .contains("xtask-done: command=mutants-verify-evidence ok=true")
    );
    let invalid = verify(None, 1);
    assert!(
        String::from_utf8_lossy(&invalid.stderr)
            .contains("xtask-done: command=mutants-verify-evidence ok=false")
    );
    assert_non_verification(invalid);
}

// Shapes follow cargo-mutants v27.1.0 outcome.rs, mutant.rs and process.rs.
fn phase(name: &str, status: Value) -> Value {
    json!({"phase": name, "duration": 0.1, "process_status": status,
        "argv": ["cargo", if name == "Build" { "build" } else { "nextest" }]})
}

fn evidence(summary: &str) -> Value {
    let mutant = json!({
        "name": "common/src/render.rs:10:5: replace render -> String with String::new()",
        "package": "common", "file": "common/src/render.rs", "function": null,
        "span": {"start": {"line": 10, "column": 5}, "end": {"line": 10, "column": 20}},
        "replacement": "String::new()", "genre": "FnValue"
    });
    let phases = match summary {
        "Unviable" => vec![phase("Build", json!({"Failure": 101}))],
        "CaughtMutant" => vec![
            phase("Build", json!("Success")),
            phase("Test", json!({"Failure": 100})),
        ],
        "Timeout" => vec![
            phase("Build", json!("Success")),
            phase("Test", json!("Timeout")),
        ],
        _ => vec![
            phase("Build", json!("Success")),
            phase("Test", json!("Success")),
        ],
    };
    json!({
        "outcomes": [
            {"scenario": "Baseline", "summary": "Success", "log_path": "log/baseline.log",
             "diff_path": null, "phase_results": [phase("Build", json!("Success")), phase("Test", json!("Success"))]},
            {"scenario": {"Mutant": mutant}, "summary": summary, "log_path": "log/mutant.log",
             "diff_path": "diff/mutant.diff", "phase_results": phases}
        ],
        "total_mutants": 1, "missed": usize::from(summary == "MissedMutant"),
        "caught": usize::from(summary == "CaughtMutant"), "unviable": usize::from(summary == "Unviable"),
        "timeout": usize::from(summary == "Timeout"), "success": 0,
        "start_time": "2026-10-14T00:00:00Z", "end_time": "2026-10-14T00:00:01Z",
        "cargo_mutants_version": "27.1.0"
    })
}

fn fixture(value: &Value) -> tempfile::TempDir {
    let temp = tempfile::tempdir().unwrap();
    fs::write(
        temp.path().join("outcomes.json"),
        serde_json::to_vec(value).unwrap(),
    )
    .unwrap();
    for (category, summary) in [
        ("missed", "MissedMutant"),
        ("caught", "CaughtMutant"),
        ("unviable", "Unviable"),
        ("timeout", "Timeout"),
    ] {
        let mut text = String::new();
        for outcome in value["outcomes"].as_array().unwrap() {
            if outcome["summary"] == summary {
                text.push_str(outcome["scenario"]["Mutant"]["name"].as_str().unwrap());
                text.push('\n');
            }
        }
        fs::write(temp.path().join(format!("{category}.txt")), text).unwrap();
    }
    temp
}

#[test]
fn completed_evidence_is_required_before_counts() {
    assert_non_verification(verify(None, 0));
    let mut value = evidence("CaughtMutant");
    value["end_time"] = Value::Null;
    let fixture = fixture(&value);
    assert_non_verification(verify(Some(fixture.path()), 0));
}

#[test]
fn malformed_and_missing_artifacts_are_not_verification() {
    for contents in ["", "{", "null", "{}", "[]", "{}\n{}"] {
        let fixture = fixture(&evidence("CaughtMutant"));
        fs::write(fixture.path().join("outcomes.json"), contents).unwrap();
        assert_non_verification(verify(Some(fixture.path()), 0));
    }
    for name in [
        "outcomes.json",
        "missed.txt",
        "caught.txt",
        "unviable.txt",
        "timeout.txt",
    ] {
        let fixture = fixture(&evidence("CaughtMutant"));
        fs::remove_file(fixture.path().join(name)).unwrap();
        assert_non_verification(verify(Some(fixture.path()), 0));
    }
}

#[test]
fn incomplete_inconsistent_or_inapplicable_outcomes_are_not_verification() {
    let original = evidence("CaughtMutant");
    let cases = [
        ("/end_time", json!("not a timestamp")),
        ("/end_time", json!("2026-10-13T00:00:00Z")),
        ("/total_mutants", json!(2)),
        ("/caught", json!(0)),
        ("/missed", json!(1)),
        ("/timeout", json!(1)),
        ("/success", json!(1)),
        ("/caught", json!("1")),
        ("/caught", json!(-1)),
        ("/outcomes/0/summary", json!("Failure")),
        ("/outcomes/0/phase_results", json!([])),
        (
            "/outcomes/0/phase_results/1/process_status",
            json!({"Failure": 100}),
        ),
        (
            "/outcomes/1/phase_results",
            json!([phase("Build", json!("Success"))]),
        ),
        (
            "/outcomes/1/phase_results/1/process_status",
            json!("Success"),
        ),
        (
            "/outcomes/1/phase_results/1/process_status",
            json!({"Failure": 0}),
        ),
        (
            "/outcomes/1/phase_results/1/process_status",
            json!({"Signalled": 9}),
        ),
        ("/outcomes/1/scenario/Mutant/package", json!("storage")),
        (
            "/outcomes/1/scenario/Mutant/file",
            json!("common/src/other.rs"),
        ),
        ("/outcomes/1/scenario/Mutant/name", Value::Null),
        ("/outcomes/1/summary", json!("Success")),
    ];
    for (pointer, replacement) in cases {
        let mut value = original.clone();
        *value.pointer_mut(pointer).unwrap() = replacement;
        let fixture = fixture(&original);
        fs::write(
            fixture.path().join("outcomes.json"),
            serde_json::to_vec(&value).unwrap(),
        )
        .unwrap();
        assert_non_verification(verify(Some(fixture.path()), 0));
    }
    for index in [0, 1] {
        let mut value = original.clone();
        value["outcomes"].as_array_mut().unwrap().remove(index);
        let fixture = fixture(&value);
        assert_non_verification(verify(Some(fixture.path()), 0));
    }
    let mut value = original.clone();
    value["outcomes"]
        .as_array_mut()
        .unwrap()
        .push(original["outcomes"][1].clone());
    value["total_mutants"] = json!(2);
    value["caught"] = json!(2);
    let fixture = fixture(&value);
    assert_non_verification(verify(Some(fixture.path()), 0));
}

#[test]
fn zero_applicable_mutants_and_timeout_holes_are_not_verification() {
    let mut empty = evidence("CaughtMutant");
    empty["outcomes"].as_array_mut().unwrap().truncate(1);
    empty["total_mutants"] = json!(0);
    empty["caught"] = json!(0);
    let fixture = fixture(&empty);
    let output = verify(Some(fixture.path()), 0);
    assert!(String::from_utf8_lossy(&output.stderr).contains("no applicable mutants"));
    assert_non_verification(output);
    let fixture = self::fixture(&evidence("Timeout"));
    for status in [0, 2, 3] {
        assert_non_verification(verify(Some(fixture.path()), status));
    }
}

#[test]
fn failed_baseline_and_tool_errors_cannot_publish_counts_even_with_artifacts() {
    let fixture = fixture(&evidence("CaughtMutant"));
    for status in [1, 3, 4, 5, 6, 70, 130, 137] {
        assert_non_verification(verify(Some(fixture.path()), status));
    }
    assert_non_verification(verify(Some(fixture.path()), 2));
    let fixture = self::fixture(&evidence("MissedMutant"));
    assert_non_verification(verify(Some(fixture.path()), 0));
}

#[test]
fn native_launch_failure_propagates_without_counts() {
    let fixture = fixture(&evidence("CaughtMutant"));
    let output = verify_with_binary(
        Some(fixture.path()),
        0,
        &fixture.path().join("missing-xtask"),
    );
    assert_eq!(output.status.code(), Some(127));
    let stdout = String::from_utf8(output.stdout).unwrap();
    assert!(!stdout.contains("missed:"), "{stdout}");
    assert!(!stdout.contains("STILL ALIVE"), "{stdout}");
}

#[test]
fn native_timestamp_validation_preserves_fractional_order() {
    let mut value = evidence("CaughtMutant");
    value["start_time"] = json!("2026-10-15T00:00:00.123456789Z");
    value["end_time"] = json!("2026-10-15T00:00:00.123456788Z");
    let fixture = fixture(&value);
    assert_non_verification(verify(Some(fixture.path()), 0));
}

#[test]
fn category_lists_must_agree_with_outcomes() {
    for (name, contents) in [
        ("caught.txt", ""),
        ("missed.txt", "unexpected survivor\n"),
        ("caught.txt", "wrong mutant\n"),
        ("unviable.txt", "unexpected\n"),
        ("timeout.txt", "unexpected\n"),
    ] {
        let fixture = fixture(&evidence("CaughtMutant"));
        fs::write(fixture.path().join(name), contents).unwrap();
        assert_non_verification(verify(Some(fixture.path()), 0));
    }
}

#[test]
fn completed_mixed_population_retains_actual_counts() {
    let mut value = evidence("MissedMutant");
    let mut caught = evidence("CaughtMutant")["outcomes"][1].clone();
    caught["scenario"]["Mutant"]["name"] = json!("common/src/render.rs:20:5: replace == with !=");
    caught["scenario"]["Mutant"]["span"]["start"]["line"] = json!(20);
    caught["scenario"]["Mutant"]["span"]["end"]["line"] = json!(20);
    caught["scenario"]["Mutant"]["replacement"] = json!("!=");
    caught["scenario"]["Mutant"]["genre"] = json!("BinaryOperator");
    value["outcomes"].as_array_mut().unwrap().push(caught);
    value["total_mutants"] = json!(2);
    value["caught"] = json!(1);
    value["start_time"] = json!("2026-10-14T00:00:00.123456789Z");
    value["end_time"] = json!("2026-10-14T00:00:01.123456789Z");
    let fixture = fixture(&value);
    let output = verify(Some(fixture.path()), 2);
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8(output.stdout).unwrap();
    assert!(
        stdout.contains("missed: 1\ncaught: 1\nunviable: 0\ntimeout: 0"),
        "{stdout}"
    );
    assert!(
        stdout.contains("STILL ALIVE:\ncommon/src/render.rs:10:5:"),
        "{stdout}"
    );
    assert!(!stdout.contains("replace == with !="), "{stdout}");
}

#[test]
fn completed_applicable_findings_keep_counts_and_survivor_details() {
    for (summary, status, counts) in [
        (
            "MissedMutant",
            2,
            "missed: 1\ncaught: 0\nunviable: 0\ntimeout: 0",
        ),
        (
            "CaughtMutant",
            0,
            "missed: 0\ncaught: 1\nunviable: 0\ntimeout: 0",
        ),
        (
            "Unviable",
            0,
            "missed: 0\ncaught: 0\nunviable: 1\ntimeout: 0",
        ),
    ] {
        let fixture = fixture(&evidence(summary));
        let output = verify(Some(fixture.path()), status);
        let stdout = String::from_utf8(output.stdout).unwrap();
        assert!(
            output.status.success(),
            "{stdout}\n{}",
            String::from_utf8_lossy(&output.stderr)
        );
        assert!(stdout.contains(counts), "{stdout}");
        assert_eq!(stdout.contains("STILL ALIVE:"), status == 2);
        if status == 2 {
            assert!(stdout.contains(
                "common/src/render.rs:10:5: replace render -> String with String::new()"
            ));
        }
    }
}
