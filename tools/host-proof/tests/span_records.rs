//! test-local joins a `dev` run's trace without changing its report or status.
use std::{fs, path::Path, process::Command};

const TRACE_ID: &str = "0af7651916cd43dd8448eb211c80319c";
const CALLER_SPAN: &str = "00f067aa0ba902b7";

fn checkout(root: &Path) {
    assert!(
        host_proof::git::at(root)
            .args(["init", "--quiet"])
            .status()
            .unwrap()
            .success()
    );
    fs::write(root.join("flake.nix"), "{}").unwrap();
    fs::create_dir(root.join("tools")).unwrap();
    fs::write(root.join("tools/Cargo.toml"), "").unwrap();
}

fn invoke(root: &Path, trace: &[(&str, &str)]) -> std::process::Output {
    let mut command = Command::new(env!("CARGO_BIN_EXE_test-local"));
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
        .env_remove("JAUNDER_TRACE_DIR")
        .env_remove("TRACEPARENT")
        .env_remove("TRACESTATE")
        .env(
            "JAUNDER_CSR_BUNDLE_DIR",
            root.join("private-missing-bundle"),
        )
        .envs(trace.iter().copied())
        .args(["--", "-p", "storage", "private-filter"]);
    command.output().unwrap()
}

/// The text of the run directory's one record file, named `<pid>-<random>.jsonl`.
fn record_file(directory: &Path) -> String {
    let files: Vec<_> = fs::read_dir(directory).unwrap().collect();
    assert_eq!(files.len(), 1, "one record file per process");
    let path = files.into_iter().next().unwrap().unwrap().path();
    let name = path.file_name().unwrap().to_string_lossy().into_owned();
    let (pid, rest) = name.split_once('-').unwrap();
    assert!(
        pid.parse::<u32>().is_ok() && rest.ends_with(".jsonl"),
        "{name}"
    );
    fs::read_to_string(path).unwrap()
}

#[test]
fn phases_nest_under_the_callers_step() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().join("checkout");
    fs::create_dir(&root).unwrap();
    checkout(&root);
    let run = temp.path().join("run.trace.d");
    fs::create_dir(&run).unwrap();
    let traceparent = format!("00-{TRACE_ID}-{CALLER_SPAN}-01");

    let output = invoke(
        &root,
        &[
            ("JAUNDER_TRACE_DIR", run.to_str().unwrap()),
            ("TRACEPARENT", &traceparent),
        ],
    );

    assert_eq!(output.status.code(), Some(1));
    let report: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(report["command"], "test-local");
    let text = record_file(&run);
    // Spans carry names and outcomes, never arguments or environment.
    assert!(!text.contains("private"));
    let records: Vec<serde_json::Value> = text
        .lines()
        .map(|line| serde_json::from_str(line).unwrap())
        .collect();
    assert!(records.iter().all(|record| record["trace_id"] == TRACE_ID));
    let span = |name: &str, kind: &str| {
        records
            .iter()
            .find(|record| record["name"] == name && record["kind"] == kind)
            .unwrap_or_else(|| panic!("no {kind} record for {name}: {records:?}"))
    };
    let command = span("xtask.command", "start");
    assert_eq!(command["parent_id"], CALLER_SPAN);
    assert!(command.get("attributes").is_none());
    for (child, parent) in [
        ("xtask.checkout", "xtask.command"),
        ("xtask.hooks", "xtask.command"),
        ("xtask.dispatch", "xtask.command"),
        ("xtask.artifact-lock-wait", "xtask.dispatch"),
        ("test-local-csr-prerequisite", "xtask.dispatch"),
    ] {
        assert_eq!(
            span(child, "start")["parent_id"],
            span(parent, "start")["span_id"],
            "{child} under {parent}"
        );
        assert_eq!(
            span(child, "end")["span_id"],
            span(child, "start")["span_id"]
        );
    }
    assert_eq!(
        span("xtask.command", "end")["attributes"],
        serde_json::json!({"process.exit.code": 1, "jaunder.status": "failed"})
    );
    assert_eq!(
        span("test-local-csr-prerequisite", "end")["attributes"],
        serde_json::json!({"jaunder.status": "failed"})
    );
}

#[test]
fn tracing_never_changes_the_report_or_status() {
    let temp = tempfile::tempdir().unwrap();
    let untraced = invoke(temp.path(), &[]);
    assert_eq!(untraced.status.code(), Some(2));
    assert!(!temp.path().join(".xtask").exists());

    let disabled = invoke(temp.path(), &[("JAUNDER_TRACE_DIR", "")]);
    assert_eq!(disabled.status, untraced.status);
    assert_eq!(disabled.stdout, untraced.stdout);
    assert_eq!(disabled.stderr, untraced.stderr);

    let blocked = temp.path().join("not-a-directory");
    fs::write(&blocked, "blocked").unwrap();
    let unwritable = invoke(
        temp.path(),
        &[("JAUNDER_TRACE_DIR", blocked.to_str().unwrap())],
    );
    assert_eq!(unwritable.status, untraced.status);
    assert_eq!(unwritable.stdout, untraced.stdout);
    let stderr = String::from_utf8_lossy(&unwritable.stderr);
    let (warnings, rest): (Vec<_>, Vec<_>) = stderr
        .lines()
        .partition(|line| line.starts_with("host-proof: tracing disabled: "));
    assert_eq!(warnings.len(), 1, "{stderr}");
    assert_eq!(
        rest,
        String::from_utf8_lossy(&untraced.stderr)
            .lines()
            .collect::<Vec<_>>()
    );
}
