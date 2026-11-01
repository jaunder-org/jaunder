//! Command-boundary proofs: capture is complete before explicit exit and reporting.
use std::{fs, path::Path, process::Command};

fn checkout(root: &Path, admitted: bool) {
    assert!(
        xtask::git::at(root)
            .args(["init", "--quiet"])
            .status()
            .unwrap()
            .success()
    );
    if admitted {
        fs::write(root.join("flake.nix"), "{}").unwrap();
        fs::create_dir(root.join("xtask")).unwrap();
        fs::write(root.join("xtask/Cargo.toml"), "").unwrap();
    }
}

fn invoke(root: &Path) -> std::process::Output {
    Command::new(env!("CARGO_BIN_EXE_xtask"))
        .current_dir(root)
        // Temp roots may live inside the real checkout in CI. Keep discovery
        // fixture-local, including when the test is launched by a Git hook.
        .env("GIT_CEILING_DIRECTORIES", root.parent().unwrap())
        .env_remove("GIT_DIR")
        .env_remove("GIT_INDEX_FILE")
        .env_remove("GIT_WORK_TREE")
        .env_remove("GIT_OBJECT_DIRECTORY")
        .env_remove("GIT_COMMON_DIR")
        .env_remove("GIT_NAMESPACE")
        .env(
            "JAUNDER_CSR_BUNDLE_DIR",
            root.join("private-missing-bundle"),
        )
        .args([
            "--json",
            "test-local",
            "--",
            "-p",
            "storage",
            "private-filter",
        ])
        .output()
        .unwrap()
}

fn trace(output: &std::process::Output) -> Vec<serde_json::Value> {
    let stderr = String::from_utf8_lossy(&output.stderr);
    let path = stderr
        .lines()
        .find_map(|line| line.strip_prefix("xtask-trace: status=complete path="))
        .expect("complete trace before exit");
    assert!(Path::new(path).is_absolute());
    let bytes = fs::read(path).unwrap();
    let text = String::from_utf8_lossy(&bytes);
    assert!(!text.contains("private-missing-bundle"));
    assert!(!text.contains("private-filter"));
    serde_json::from_slice(&bytes).unwrap()
}

#[test]
fn invocation_trace_unresolved_checkout_does_not_write_elsewhere() {
    let temp = tempfile::tempdir().unwrap();
    let output = invoke(temp.path());
    assert_eq!(output.status.code(), Some(2));
    assert!(
        String::from_utf8_lossy(&output.stderr)
            .contains("xtask-trace: status=unavailable reason=checkout-unavailable")
    );
    assert!(!temp.path().join(".xtask").exists());
}

#[test]
fn invocation_trace_early_error_closes_preparation_and_preserves_exit_two() {
    let temp = tempfile::tempdir().unwrap();
    checkout(temp.path(), false);
    let output = invoke(temp.path());
    assert_eq!(output.status.code(), Some(2));
    let events = trace(&output);
    assert!(
        events
            .iter()
            .any(|event| event["name"] == "xtask.checkout" && event["ph"] == "E")
    );
    let terminal = events
        .iter()
        .find(|event| event["name"] == "xtask.command.outcome")
        .unwrap();
    assert_eq!(terminal["args"]["exit_code"], "2");
    assert!(!temp.path().join(".xtask/last-result.json").exists());
}

#[test]
fn invocation_trace_failed_phase_keeps_json_sidecar_and_trace_timing_identical() {
    let temp = tempfile::tempdir().unwrap();
    checkout(temp.path(), true);
    let output = invoke(temp.path());
    assert_eq!(output.status.code(), Some(1));
    let events = trace(&output);
    let result: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    let sidecar: serde_json::Value =
        serde_json::from_slice(&fs::read(temp.path().join(".xtask/last-result.json")).unwrap())
            .unwrap();
    assert_eq!(result, sidecar);
    assert_eq!(result["ok"], false);
    assert!(
        events
            .iter()
            .any(|event| event["name"] == "xtask.artifact-lock-wait" && event["ph"] == "B")
    );
    for (name, result_ms) in [
        ("xtask.dispatch", result["duration_ms"].as_u64().unwrap()),
        (
            "test-local-csr-prerequisite",
            result["steps"][0]["duration_ms"].as_u64().unwrap(),
        ),
    ] {
        let begin = events
            .iter()
            .find(|event| event["name"] == name && event["ph"] == "B")
            .unwrap()["ts"]
            .as_f64()
            .unwrap();
        let end = events
            .iter()
            .find(|event| event["name"] == name && event["ph"] == "E")
            .unwrap()["ts"]
            .as_f64()
            .unwrap();
        assert_eq!(result_ms, ((end - begin) / 1000.0).floor() as u64);
    }
}

#[test]
fn invocation_trace_persistence_failure_does_not_replace_command_verdict_or_report() {
    let temp = tempfile::tempdir().unwrap();
    checkout(temp.path(), true);
    fs::create_dir(temp.path().join(".xtask")).unwrap();
    fs::write(temp.path().join(".xtask/traces"), "blocked").unwrap();
    let output = invoke(temp.path());
    assert_eq!(output.status.code(), Some(1));
    assert!(
        String::from_utf8_lossy(&output.stderr)
            .contains("xtask-trace: status=unavailable reason=directory-create")
    );
    let result: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(result["ok"], false);
    assert!(temp.path().join(".xtask/last-result.json").is_file());
}
