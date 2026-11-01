use std::io::{BufRead, BufReader};
use std::process::{Command, Stdio};

fn executable() -> &'static str {
    env!("CARGO_BIN_EXE_traced-run")
}

fn root_command() -> Command {
    let mut command = Command::new(executable());
    // The hook's cargo-xtask launcher may already carry a trace context.
    // Only nested invocations inside a fixture should inherit its root.
    command.env_remove("JAUNDER_TRACED_RUN_CONTEXT");
    command
}

fn assert_flow_pair(
    flows: &[&serde_json::Value],
    parent: &serde_json::Value,
    child: &serde_json::Value,
) {
    let finish = flows
        .iter()
        .find(|event| event["ph"] == "f" && event["pid"] == child["pid"])
        .unwrap();
    let start = flows
        .iter()
        .find(|event| event["ph"] == "s" && event["id"] == finish["id"])
        .unwrap();
    assert_eq!(start["id"], finish["id"]);
    assert_eq!(finish["bp"], "e");
    assert_eq!(start["pid"], parent["pid"]);
    assert_eq!(finish["pid"], child["pid"]);
    assert_eq!(start["ts"], parent["ts"]);
    assert!(finish["ts"].as_u64().unwrap() >= child["ts"].as_u64().unwrap());
    assert!(
        finish["ts"].as_u64().unwrap()
            <= child["ts"].as_u64().unwrap() + child["dur"].as_u64().unwrap()
    );
}

fn assert_complete_unique_flows(flows: &[&serde_json::Value], expected_edges: usize) {
    let mut by_id = std::collections::BTreeMap::<&str, Vec<&serde_json::Value>>::new();
    for event in flows {
        by_id
            .entry(event["id"].as_str().expect("flow event has an id"))
            .or_default()
            .push(event);
    }
    assert_eq!(by_id.len(), expected_edges, "each relation has a unique id");
    for (id, endpoints) in by_id {
        assert!(!id.is_empty());
        assert!(u64::from_str_radix(id, 16).is_ok(), "flow id is hex");
        assert_eq!(endpoints.len(), 2, "flow {id} has both endpoints");
        assert_eq!(
            endpoints.iter().filter(|event| event["ph"] == "s").count(),
            1
        );
        assert_eq!(
            endpoints.iter().filter(|event| event["ph"] == "f").count(),
            1
        );
    }
}

fn final_trace(root: &std::path::Path) -> serde_json::Value {
    let trace = std::fs::read_dir(root.join(".xtask/run"))
        .unwrap()
        .filter_map(Result::ok)
        .find(|entry| {
            entry
                .path()
                .file_name()
                .is_some_and(|name| name.to_string_lossy().ends_with(".trace.json"))
        })
        .unwrap();
    serde_json::from_slice(&std::fs::read(trace.path()).unwrap()).unwrap()
}

#[test]
fn nested_runners_share_one_root_and_record_parentage() {
    let fixture = tempfile::tempdir().unwrap();
    let output = root_command()
        .current_dir(fixture.path())
        .args([
            "--name",
            "outer",
            "--",
            executable(),
            "--name",
            "inner",
            "--",
            executable(),
            "--name",
            "grandchild",
            "--",
            "true",
        ])
        .output()
        .unwrap();
    assert!(output.status.success());
    let run_dir = fixture.path().join(".xtask/run");
    let traces: Vec<_> = std::fs::read_dir(run_dir)
        .unwrap()
        .filter_map(Result::ok)
        .filter(|entry| {
            entry
                .path()
                .file_name()
                .is_some_and(|name| name.to_string_lossy().ends_with(".trace.json"))
        })
        .collect();
    assert_eq!(traces.len(), 1, "one root publishes one merged trace");
    let document: serde_json::Value =
        serde_json::from_slice(&std::fs::read(traces[0].path()).unwrap()).unwrap();
    let events = document["traceEvents"].as_array().unwrap();
    let spans: Vec<_> = events.iter().filter(|event| event["ph"] == "X").collect();
    let flows: Vec<_> = events
        .iter()
        .filter(|event| event["ph"] == "s" || event["ph"] == "f")
        .collect();
    assert_eq!(spans.len(), 3);
    assert_eq!(flows.len(), 4);
    assert_complete_unique_flows(&flows, 2);
    let outer = spans.iter().find(|event| event["name"] == "outer").unwrap();
    let inner = spans.iter().find(|event| event["name"] == "inner").unwrap();
    let grandchild = spans
        .iter()
        .find(|event| event["name"] == "grandchild")
        .unwrap();
    assert_eq!(outer["args"]["parent_span_id"], serde_json::Value::Null);
    assert_eq!(outer["ts"], 0);
    assert_eq!(inner["args"]["parent_span_id"], outer["args"]["span_id"]);
    assert_eq!(inner["args"]["run_id"], outer["args"]["run_id"]);
    assert_flow_pair(&flows, outer, inner);
    assert_eq!(
        grandchild["args"]["parent_span_id"],
        inner["args"]["span_id"]
    );
    assert_flow_pair(&flows, inner, grandchild);
    let child_start = inner["ts"].as_u64().unwrap();
    let child_end = child_start + inner["dur"].as_u64().unwrap();
    assert!(child_start >= outer["ts"].as_u64().unwrap());
    assert!(child_end <= outer["dur"].as_u64().unwrap());
    let records = traces[0].path().with_extension("d");
    let records: Vec<_> = std::fs::read_dir(records)
        .unwrap()
        .map(|entry| entry.unwrap().file_name())
        .collect();
    assert_eq!(
        records.len(),
        3,
        "all participants published complete records"
    );
    assert!(
        records
            .iter()
            .all(|name| name.to_string_lossy().ends_with(".jsonl"))
    );
}

#[cfg(unix)]
#[test]
fn concurrent_sibling_runners_keep_distinct_tracks_and_complete_parent_flows() {
    let fixture = tempfile::tempdir().unwrap();
    let script = fixture.path().join("siblings.py");
    std::fs::write(
        &script,
        r#"import subprocess, sys
runner = sys.argv[1]
children = [
    subprocess.Popen([runner, '--name', name, '--', 'sleep', '0.4'])
    for name in ('sibling-one', 'sibling-two')
]
raise SystemExit(max(child.wait() for child in children))
"#,
    )
    .unwrap();
    let output = root_command()
        .current_dir(fixture.path())
        .args(["--name", "parent", "--", "python3"])
        .arg(&script)
        .arg(executable())
        .output()
        .unwrap();
    assert!(output.status.success());

    let events = final_trace(fixture.path())["traceEvents"]
        .as_array()
        .unwrap()
        .clone();
    let spans: Vec<_> = events.iter().filter(|event| event["ph"] == "X").collect();
    let flows: Vec<_> = events
        .iter()
        .filter(|event| event["ph"] == "s" || event["ph"] == "f")
        .collect();
    assert_eq!(spans.len(), 3);
    assert_eq!(flows.len(), 4);
    assert_complete_unique_flows(&flows, 2);
    let parent = spans
        .iter()
        .find(|event| event["name"] == "parent")
        .unwrap();
    let siblings: Vec<_> = spans
        .iter()
        .filter(|event| event["name"] == "sibling-one" || event["name"] == "sibling-two")
        .collect();
    assert_eq!(siblings.len(), 2);
    assert_ne!(siblings[0]["pid"], siblings[1]["pid"]);
    for sibling in &siblings {
        assert_eq!(sibling["args"]["parent_span_id"], parent["args"]["span_id"]);
        assert_flow_pair(&flows, parent, sibling);
    }
    let first_start = siblings[0]["ts"].as_u64().unwrap();
    let first_end = first_start + siblings[0]["dur"].as_u64().unwrap();
    let second_start = siblings[1]["ts"].as_u64().unwrap();
    let second_end = second_start + siblings[1]["dur"].as_u64().unwrap();
    assert!(first_start.max(second_start) < first_end.min(second_end));
}

#[test]
fn externally_captured_launcher_output_belongs_to_the_outer_runner() {
    let fixture = tempfile::tempdir().unwrap();
    let output = root_command()
        .current_dir(fixture.path())
        .args([
            "--name",
            "capture-owner",
            "--",
            executable(),
            "--launcher",
            "--name",
            "cargo-xtask",
            "--",
            "printf",
            "outer-capture\\n",
        ])
        .output()
        .unwrap();
    assert!(output.status.success());
    let result: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    let captured_path = result["stdout"]["path"].as_str().unwrap();
    assert_eq!(std::fs::read(captured_path).unwrap(), b"outer-capture\n");
    let events = final_trace(fixture.path())["traceEvents"]
        .as_array()
        .unwrap()
        .clone();
    assert_eq!(events.iter().filter(|event| event["ph"] == "X").count(), 2);
    assert_eq!(events.iter().filter(|event| event["ph"] == "s").count(), 1);
    assert_eq!(events.iter().filter(|event| event["ph"] == "f").count(), 1);
    assert!(events.iter().any(|event| event["name"] == "capture-owner"));
    assert!(events.iter().any(|event| event["name"] == "cargo-xtask"));
}

#[test]
fn launcher_streams_without_wrapper_json_or_capture_files() {
    let fixture = tempfile::tempdir().unwrap();
    let output = root_command()
        .current_dir(fixture.path())
        .args([
            "--launcher",
            "--name",
            "launcher-test",
            "--",
            "printf",
            "streamed\\n",
        ])
        .output()
        .unwrap();
    assert!(output.status.success());
    assert_eq!(output.stdout, b"streamed\n");
    assert!(output.stderr.is_empty());
    let artifacts: Vec<_> = std::fs::read_dir(fixture.path().join(".xtask/run"))
        .unwrap()
        .filter_map(Result::ok)
        .map(|entry| entry.file_name())
        .collect();
    assert_eq!(
        artifacts.len(),
        2,
        "one final trace plus its per-run records"
    );
    assert!(
        artifacts
            .iter()
            .any(|name| name.to_string_lossy().ends_with(".trace.json"))
    );
    assert!(
        artifacts
            .iter()
            .any(|name| name.to_string_lossy().ends_with(".trace.d"))
    );
}

#[cfg(unix)]
#[test]
fn launcher_output_is_observable_before_the_child_finishes() {
    let fixture = tempfile::tempdir().unwrap();
    let script = fixture.path().join("stream.py");
    let finished = fixture.path().join("finished");
    std::fs::write(&script, "#!/usr/bin/env python3\nimport pathlib, sys, time\nprint('early', flush=True)\ntime.sleep(1)\npathlib.Path(sys.argv[1]).touch()\n").unwrap();
    use std::os::unix::fs::PermissionsExt;
    std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o755)).unwrap();
    let mut child = root_command()
        .current_dir(fixture.path())
        .args([
            "--launcher",
            "--name",
            "early-stream",
            "--",
            script.to_str().unwrap(),
            finished.to_str().unwrap(),
        ])
        .stdout(Stdio::piped())
        .spawn()
        .unwrap();
    let mut line = String::new();
    BufReader::new(child.stdout.take().unwrap())
        .read_line(&mut line)
        .unwrap();
    assert_eq!(line, "early\n");
    assert!(
        !finished.exists(),
        "the runner must stream before child completion"
    );
    assert!(child.wait().unwrap().success());
}

#[cfg(unix)]
#[test]
fn killed_named_descendant_withholds_root_trace() {
    let fixture = tempfile::tempdir().unwrap();
    let output = root_command()
        .current_dir(fixture.path())
        .args([
            "--name",
            "outer",
            "--timeout",
            "1",
            "--",
            executable(),
            "--name",
            "inner",
            "--",
            "sleep",
            "10",
        ])
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(124));
    let summary: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(summary["timed_out"], true);
    assert_unfinished_participant(fixture.path(), &output.stderr);
    let run_dir = fixture.path().join(".xtask/run");
    assert!(
        !std::fs::read_dir(run_dir)
            .unwrap()
            .filter_map(Result::ok)
            .any(|entry| {
                entry
                    .path()
                    .file_name()
                    .is_some_and(|name| name.to_string_lossy().ends_with(".trace.json"))
            })
    );
}

fn assert_unfinished_participant(root: &std::path::Path, stderr: &[u8]) {
    assert!(
        String::from_utf8_lossy(stderr).contains("unfinished participants; merged trace withheld")
    );
    let artifacts: Vec<_> = std::fs::read_dir(root.join(".xtask/run"))
        .unwrap()
        .map(|entry| entry.unwrap().path())
        .collect();
    assert!(
        !artifacts
            .iter()
            .any(|path| path.to_string_lossy().ends_with(".trace.json"))
    );
    let records = artifacts
        .iter()
        .find(|path| path.extension().is_some_and(|ext| ext == "d"))
        .unwrap();
    let records: Vec<_> = std::fs::read_dir(records)
        .unwrap()
        .map(|entry| entry.unwrap().file_name())
        .collect();
    assert_eq!(
        records
            .iter()
            .filter(|name| name.to_string_lossy().starts_with("active-"))
            .count(),
        1,
        "killed participant remains registered"
    );
    assert_eq!(
        records
            .iter()
            .filter(|name| name.to_string_lossy().ends_with(".jsonl"))
            .count(),
        1,
        "finished outer record is preserved"
    );
}

#[cfg(unix)]
#[test]
fn killed_named_descendant_with_outer_failure_preserves_partial_records() {
    use std::os::unix::fs::PermissionsExt;
    let fixture = tempfile::tempdir().unwrap();
    let script = fixture.path().join("kill-participant.py");
    std::fs::write(
        &script,
        r"#!/usr/bin/env python3
import pathlib, subprocess, sys, time
child = subprocess.Popen([sys.argv[1], '--name', 'killed-inner', '--', 'sleep', '1'])
deadline = time.monotonic() + 5
while len(list(pathlib.Path('.xtask/run').glob('*.trace.d/active-*'))) < 2:
    if time.monotonic() >= deadline:
        child.kill()
        child.wait()
        sys.exit(99)
    time.sleep(0.005)
child.kill()
child.wait()
sys.exit(17)
",
    )
    .unwrap();
    std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o755)).unwrap();
    let output = root_command()
        .current_dir(fixture.path())
        .args(["--name", "failed-outer", "--"])
        .arg(script)
        .arg(executable())
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(17));
    let summary: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(summary["exit_code"], 17);
    assert_eq!(summary["ok"], false);
    assert_unfinished_participant(fixture.path(), &output.stderr);
}

#[test]
fn launcher_timeout_and_spawn_failure_still_publish_final_traces() {
    let fixture = tempfile::tempdir().unwrap();
    for (name, args, status) in [
        ("nonzero", vec!["--", "false"], 1),
        ("timeout", vec!["--timeout", "1", "--", "sleep", "10"], 124),
        (
            "spawn-error",
            vec!["--", "/definitely/not/a/real/program"],
            64,
        ),
    ] {
        let mut invocation = vec!["--launcher", "--name", name];
        invocation.extend(args);
        let output = root_command()
            .current_dir(fixture.path())
            .args(invocation)
            .output()
            .unwrap();
        assert_eq!(output.status.code(), Some(status));
    }
    let traces = std::fs::read_dir(fixture.path().join(".xtask/run"))
        .unwrap()
        .filter_map(Result::ok)
        .filter(|entry| {
            entry
                .path()
                .file_name()
                .is_some_and(|name| name.to_string_lossy().ends_with(".trace.json"))
        })
        .count();
    assert_eq!(traces, 3);
}

#[test]
fn concurrent_roots_publish_isolated_traces() {
    let fixture = tempfile::tempdir().unwrap();
    let root = fixture.path().to_owned();
    let children: Vec<_> = (0..2)
        .map(|index| {
            let root = root.clone();
            std::thread::spawn(move || {
                root_command()
                    .current_dir(root)
                    .args([
                        "--name",
                        if index == 0 {
                            "parallel-one"
                        } else {
                            "parallel-two"
                        },
                        "--",
                        "true",
                    ])
                    .output()
                    .unwrap()
                    .status
            })
        })
        .collect();
    for child in children {
        assert!(child.join().unwrap().success());
    }
    let run_dir = fixture.path().join(".xtask/run");
    let traces: Vec<_> = std::fs::read_dir(run_dir)
        .unwrap()
        .filter_map(Result::ok)
        .filter(|entry| {
            entry
                .path()
                .file_name()
                .is_some_and(|name| name.to_string_lossy().ends_with(".trace.json"))
        })
        .collect();
    assert_eq!(traces.len(), 2);
    let ids: std::collections::HashSet<_> = traces
        .iter()
        .map(|trace| {
            let document: serde_json::Value =
                serde_json::from_slice(&std::fs::read(trace.path()).unwrap()).unwrap();
            document["traceEvents"][0]["args"]["run_id"]
                .as_str()
                .unwrap()
                .to_owned()
        })
        .collect();
    assert_eq!(ids.len(), 2);
}

#[test]
fn ordinary_child_is_not_fabricated_as_a_trace_span() {
    let fixture = tempfile::tempdir().unwrap();
    let output = root_command()
        .current_dir(fixture.path())
        .args(["--name", "opaque-parent", "--", "true"])
        .output()
        .unwrap();
    assert!(output.status.success());
    let events = final_trace(fixture.path())["traceEvents"]
        .as_array()
        .unwrap()
        .clone();
    assert_eq!(events.len(), 1);
    assert_eq!(events[0]["name"], "opaque-parent");
}

#[test]
fn forcibly_killed_root_does_not_publish_a_complete_trace() {
    let fixture = tempfile::tempdir().unwrap();
    let mut root = root_command()
        .current_dir(fixture.path())
        .args(["--name", "abrupt-root", "--", "sleep", "1"])
        .spawn()
        .unwrap();
    let run_dir = fixture.path().join(".xtask/run");
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(2);
    while !run_dir.exists() && std::time::Instant::now() < deadline {
        std::thread::sleep(std::time::Duration::from_millis(5));
    }
    assert!(
        run_dir.exists(),
        "root run directory should be allocated before spawn"
    );
    root.kill().unwrap();
    assert!(!root.wait().unwrap().success());
    std::thread::sleep(std::time::Duration::from_secs(1));
    let entries: Vec<_> = std::fs::read_dir(&run_dir)
        .unwrap()
        .filter_map(Result::ok)
        .collect();
    assert!(
        entries
            .iter()
            .any(|entry| entry.path().extension().is_some_and(|ext| ext == "d"))
    );
    assert!(!entries.iter().any(|entry| {
        entry
            .path()
            .file_name()
            .is_some_and(|name| name.to_string_lossy().ends_with(".trace.json"))
    }));
}

#[test]
fn help_and_malformed_cli_are_distinct() {
    let help = root_command().arg("--help").output().unwrap();
    assert!(help.status.success());
    assert!(String::from_utf8_lossy(&help.stdout).contains("Usage: traced-run"));
    let malformed = root_command()
        .args(["--launcher", "--", "true"])
        .output()
        .unwrap();
    assert_eq!(malformed.status.code(), Some(64));
    assert!(malformed.stdout.is_empty());
    assert!(String::from_utf8_lossy(&malformed.stderr).contains("requires --name"));
}
