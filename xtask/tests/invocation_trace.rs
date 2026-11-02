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
    invoke_with_runner_context(root, None)
}

fn invoke_with_runner_context(
    root: &Path,
    context: Option<&serde_json::Value>,
) -> std::process::Output {
    let mut command = Command::new(env!("CARGO_BIN_EXE_xtask"));
    command
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
        ]);
    if let Some(context) = context {
        command.env("JAUNDER_TRACED_RUN_CONTEXT", context.to_string());
    } else {
        command.env_remove("JAUNDER_TRACED_RUN_CONTEXT");
    }
    command.output().unwrap()
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
    assert!(text.contains("\"test_selector\":\"private-filter\""));
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
fn invocation_trace_with_runner_context_contributes_fragment_only() {
    let temp = tempfile::tempdir().unwrap();
    checkout(temp.path(), true);
    let trace_dir = temp.path().join(".xtask/run/fixture.trace.d");
    fs::create_dir_all(&trace_dir).unwrap();
    let now = rustix::time::clock_gettime(rustix::time::ClockId::Monotonic);
    let origin_ns = (now.tv_sec as u64) * 1_000_000_000 + now.tv_nsec as u64;
    let context = serde_json::json!({
        "run_id": "fixture",
        "trace_dir": trace_dir,
        "origin_ns": origin_ns,
        "parent_span_id": "runner-parent",
        "parent_pid": std::process::id(),
        "parent_started_ns": origin_ns,
        "parent_name": "xtask-execute",
        "parent_flow_id": 123456
    });

    let output = invoke_with_runner_context(temp.path(), Some(&context));

    assert_eq!(output.status.code(), Some(1));
    assert!(
        String::from_utf8_lossy(&output.stderr).contains("xtask-trace: status=contributed"),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let fragment_path = fs::read_dir(temp.path().join(".xtask/run/fixture.trace.d"))
        .unwrap()
        .next()
        .unwrap()
        .unwrap()
        .path();
    assert!(
        fragment_path
            .file_name()
            .unwrap()
            .to_string_lossy()
            .starts_with("fragment-")
    );
    let fragment = fs::read_to_string(fragment_path).unwrap();
    assert!(fragment.contains("xtask.command"));
    assert!(fragment.contains("xtask.command.outcome"));
    let events: Vec<serde_json::Value> = fragment
        .lines()
        .map(|line| serde_json::from_str(line).unwrap())
        .collect();
    let root = events
        .iter()
        .find(|event| event["name"] == "xtask.command" && event["ph"] == "B")
        .unwrap();
    assert_eq!(root["args"]["subcommand"], "test-local");
    assert_eq!(root["args"]["package"], "storage");
    assert_eq!(root["args"]["test_selector"], "private-filter");
    let flows: Vec<_> = events
        .iter()
        .filter(|event| event["ph"] == "s" || event["ph"] == "f")
        .collect();
    assert_eq!(flows.len(), 2);
    let marker_index = events
        .iter()
        .position(|event| event["name"] == "recorded child entry")
        .unwrap();
    assert_eq!(events[marker_index]["pid"], std::process::id());
    assert_eq!(events[marker_index]["tid"], std::process::id());
    assert_eq!(events[marker_index + 1]["id"], flows[0]["id"]);
    assert_eq!(events[marker_index + 1]["ph"], "s");
    assert_eq!(events[marker_index]["ts"], flows[0]["ts"]);
    assert_eq!(flows[0]["id"], flows[1]["id"]);
    assert_eq!(flows[0]["id"], "000000000001e240");
    assert_eq!(flows[0]["name"], "xtask-execute-to-command");
    assert_eq!(flows[0]["ph"], "s");
    assert_eq!(flows[0]["pid"], std::process::id());
    assert_eq!(flows[0]["args"]["parent_span_id"], "runner-parent");
    assert_eq!(flows[1]["ph"], "f");
    assert_eq!(flows[1]["pid"], root["pid"]);
    assert!(!temp.path().join(".xtask/traces").exists());
    assert!(temp.path().join(".xtask/last-result.json").is_file());
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
