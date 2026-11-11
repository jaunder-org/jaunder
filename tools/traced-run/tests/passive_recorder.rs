//! Public record-only CLI: the fixture, not the recorder, owns the workload.
use serde_json::Value;
use std::path::Path;
use std::process::{Command, Output};

fn recorder(root: &Path, args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_traced-run"))
        .current_dir(root)
        .env(
            "JAUNDER_TRACED_RUN_CONTEXT",
            r#"{"run_id":"inherited","trace_dir":"/must-not-join","origin_ns":1,"parent_span_id":"parent","parent_pid":1,"parent_started_ns":1}"#,
        )
        .args(args)
        .output()
        .unwrap()
}

fn start(root: &Path) -> String {
    let output = recorder(root, &["record-start", "tsc-request", "verify"]);
    assert!(output.status.success(), "{output:?}");
    String::from_utf8(output.stdout).unwrap().trim().to_owned()
}

fn trace(root: &Path, handle: &str) -> Value {
    serde_json::from_slice(
        &std::fs::read(root.join(format!(".xtask/run/{handle}.trace.json"))).unwrap(),
    )
    .unwrap()
}

fn leaf(root: &Path, handle: &str) -> std::path::PathBuf {
    root.join(format!(".xtask/run/{handle}.trace.d"))
}

#[test]
fn owner_fixture_preserves_once_only_execution_streams_and_status_when_recorder_fails() {
    for failure in ["none", "missing-recorder", "start-sink", "finish-sink"] {
        for expected in [0, 17] {
            let fixture = tempfile::tempdir().unwrap();
            if failure == "start-sink" {
                std::fs::create_dir(fixture.path().join(".xtask")).unwrap();
                std::fs::write(fixture.path().join(".xtask/run"), "blocked").unwrap();
            }
            // The workload blocks final publication, without controlling its own verdict.
            let workload = if failure == "finish-sink" {
                "printf x >> count; printf diagnostic; printf error >&2; for d in .xtask/run/*.trace.d; do mkdir \"${d%.d}.json\"; done; exit \"$1\""
            } else {
                "printf x >> count; printf diagnostic; printf error >&2; exit \"$1\""
            };
            let output = Command::new("bash")
                .arg(concat!(
                    env!("CARGO_MANIFEST_DIR"),
                    "/tests/fixtures/passive-owner.sh"
                ))
                .arg(if failure == "missing-recorder" {
                    "/missing/recorder"
                } else {
                    env!("CARGO_BIN_EXE_traced-run")
                })
                .args(["sh", "-c", workload, "owner", &expected.to_string()])
                .current_dir(fixture.path())
                .output()
                .unwrap();
            assert_eq!(
                output.status.code(),
                Some(expected),
                "{failure}: {output:?}"
            );
            assert_eq!(output.stdout, b"diagnostic");
            assert!(String::from_utf8_lossy(&output.stderr).contains("error"));
            assert_eq!(std::fs::read(fixture.path().join("count")).unwrap(), b"x");
            if failure == "finish-sink" {
                assert!(String::from_utf8_lossy(&output.stderr).contains("recorder:"));
            }
        }
    }
}

#[test]
fn malformed_missing_and_mismatched_context_is_refused_without_completion() {
    let fixture = tempfile::tempdir().unwrap();
    for handle in ["../escape", "/absolute", "", "1-2-3"] {
        assert!(
            !recorder(fixture.path(), &["record-finish", handle, "1", "request"])
                .status
                .success()
        );
    }
    for corruption in ["missing", "json", "origin", "future", "clock", "mode"] {
        let handle = start(fixture.path());
        let path = leaf(fixture.path(), &handle).join("start.jsonl");
        if corruption == "missing" {
            std::fs::remove_file(&path).unwrap();
        } else if corruption == "json" {
            std::fs::write(&path, "not json").unwrap();
        } else {
            let mut event: Value = serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
            match corruption {
                "origin" => event["args"]["origin_ns"] = Value::Null,
                "future" => event["args"]["origin_ns"] = u64::MAX.into(),
                "clock" => event["args"]["clock_identity"] = "another boot".into(),
                _ => event["args"]["mode"] = "arbitrary".into(),
            }
            std::fs::write(&path, event.to_string()).unwrap();
        }
        assert!(
            !recorder(fixture.path(), &["record-finish", &handle, "1", "request"])
                .status
                .success()
        );
        assert!(!leaf(fixture.path(), &handle).join("finish.jsonl").exists());
    }
    for args in [
        vec!["record-start", "tsc-request", "verify", "--", "true"],
        vec!["record-start", "other", "fix"],
        vec!["record-finish", "1-2-3", "256", "request"],
        vec!["record-finish", "1-2-3", "0", "setup"],
    ] {
        assert!(!recorder(fixture.path(), &args).status.success());
    }
}

#[test]
fn fix_mode_missing_completion_retains_explicit_unknown_start() {
    let fixture = tempfile::tempdir().unwrap();
    let output = recorder(fixture.path(), &["record-start", "tsc-request", "fix"]);
    assert!(output.status.success());
    let handle = String::from_utf8(output.stdout).unwrap().trim().to_owned();
    let begin: Value = serde_json::from_slice(
        &std::fs::read(leaf(fixture.path(), &handle).join("start.jsonl")).unwrap(),
    )
    .unwrap();
    assert_eq!(begin["args"]["mode"], "fix");
    assert_eq!(begin["args"]["outcome"], "unknown");
    assert_eq!(begin["args"]["boundary"], "unknown");
    assert!(begin["args"].get("signal").is_none());
    assert!(begin["args"].get("exit_code").is_none());
    assert!(
        !fixture
            .path()
            .join(format!(".xtask/run/{handle}.trace.json"))
            .exists()
    );
    assert!(
        recorder(fixture.path(), &["record-finish", &handle, "0", "none"])
            .status
            .success()
    );
}

#[test]
fn concurrent_finishes_are_exclusive_and_published_bytes_never_change() {
    let fixture = tempfile::tempdir().unwrap();
    let handle = start(fixture.path());
    let workers: Vec<_> = (0..8)
        .map(|_| {
            let root = fixture.path().to_owned();
            let handle = handle.clone();
            std::thread::spawn(move || {
                recorder(&root, &["record-finish", &handle, "17", "setup"])
                    .status
                    .success()
            })
        })
        .collect();
    assert_eq!(
        workers
            .into_iter()
            .map(|t| usize::from(t.join().unwrap()))
            .sum::<usize>(),
        1
    );
    let path = fixture
        .path()
        .join(format!(".xtask/run/{handle}.trace.json"));
    let before = std::fs::read(&path).unwrap();
    assert!(
        !recorder(fixture.path(), &["record-finish", &handle, "0", "none"])
            .status
            .success()
    );
    assert_eq!(std::fs::read(path).unwrap(), before);
    assert_eq!(
        trace(fixture.path(), &handle)["traceEvents"]
            .as_array()
            .unwrap()
            .iter()
            .find(|e| e["ph"] == "E")
            .unwrap()["args"]["boundary"],
        "setup"
    );
}

#[test]
fn unique_private_starts_and_failed_publication_retain_partial_evidence() {
    use std::os::unix::fs::{PermissionsExt, symlink};
    let fixture = tempfile::tempdir().unwrap();
    let workers: Vec<_> = (0..12)
        .map(|_| {
            let root = fixture.path().to_owned();
            std::thread::spawn(move || start(&root))
        })
        .collect();
    let handles: std::collections::HashSet<_> =
        workers.into_iter().map(|t| t.join().unwrap()).collect();
    assert_eq!(handles.len(), 12);
    for handle in &handles {
        let dir = leaf(fixture.path(), handle);
        assert_eq!(
            std::fs::metadata(&dir).unwrap().permissions().mode() & 0o777,
            0o700
        );
        assert_eq!(
            std::fs::metadata(dir.join("start.jsonl"))
                .unwrap()
                .permissions()
                .mode()
                & 0o777,
            0o600
        );
    }
    let handle = handles.iter().next().unwrap();
    let destination = fixture
        .path()
        .join(format!(".xtask/run/{handle}.trace.json"));
    std::fs::write(&destination, b"prior evidence").unwrap();
    assert!(
        !recorder(fixture.path(), &["record-finish", handle, "0", "none"])
            .status
            .success()
    );
    assert_eq!(std::fs::read(&destination).unwrap(), b"prior evidence");
    assert!(leaf(fixture.path(), handle).join("start.jsonl").exists());
    let other = handles.iter().find(|h| *h != handle).unwrap();
    std::fs::create_dir(leaf(fixture.path(), other).join("finish.jsonl")).unwrap();
    assert!(
        !recorder(fixture.path(), &["record-finish", other, "1", "request"])
            .status
            .success()
    );
    assert!(
        !fixture
            .path()
            .join(format!(".xtask/run/{other}.trace.json"))
            .exists()
    );
    let outside = fixture.path().join("outside");
    std::fs::create_dir(&outside).unwrap();
    symlink(&outside, fixture.path().join(".xtask/run/1-2-3.trace.d")).unwrap();
    assert!(
        !recorder(fixture.path(), &["record-finish", "1-2-3", "0", "none"])
            .status
            .success()
    );
    assert_eq!(std::fs::read_dir(outside).unwrap().count(), 0);
}

#[test]
fn observations_and_output_survive_former_fifty_item_limit() {
    let fixture = tempfile::tempdir().unwrap();
    let partial = start(fixture.path());
    let mut complete = Vec::new();
    let mut output_paths = Vec::new();
    for _ in 0..52 {
        let handle = start(fixture.path());
        assert!(
            recorder(fixture.path(), &["record-finish", &handle, "0", "none"])
                .status
                .success()
        );
        complete.push(handle);
        let output = Command::new(env!("CARGO_BIN_EXE_traced-run"))
            .current_dir(fixture.path())
            .env_remove("JAUNDER_TRACED_RUN_CONTEXT")
            .args(["--name", "retained", "--", "true"])
            .output()
            .unwrap();
        assert!(output.status.success());
        let summary: Value = serde_json::from_slice(&output.stdout).unwrap();
        output_paths.push(summary["stdout"]["path"].as_str().unwrap().to_owned());
        output_paths.push(summary["stderr"]["path"].as_str().unwrap().to_owned());
    }
    assert!(
        fixture
            .path()
            .join(format!(".xtask/run/{partial}.trace.d/start.jsonl"))
            .exists()
    );
    assert!(
        !fixture
            .path()
            .join(format!(".xtask/run/{partial}.trace.json"))
            .exists()
    );
    for handle in complete {
        assert_eq!(trace(fixture.path(), &handle)["displayTimeUnit"], "ms");
    }
    for path in output_paths {
        assert!(Path::new(&path).exists(), "lost {path}");
    }
    let finals = std::fs::read_dir(fixture.path().join(".xtask/run"))
        .unwrap()
        .filter_map(Result::ok)
        .filter(|e| e.file_name().to_string_lossy().ends_with(".trace.json"))
        .count();
    assert_eq!(finals, 104);
}

#[test]
fn recorder_processes_share_boot_identity_and_monotonic_origin() {
    let fixture = tempfile::tempdir().unwrap();
    let first = start(fixture.path());
    let second = start(fixture.path());
    let begin = |handle: &str| -> Value {
        serde_json::from_slice(
            &std::fs::read(leaf(fixture.path(), handle).join("start.jsonl")).unwrap(),
        )
        .unwrap()
    };
    let first = begin(&first);
    let second = begin(&second);
    assert_ne!(first["pid"], second["pid"]);
    assert_eq!(first["args"]["clock"], "CLOCK_MONOTONIC");
    assert_eq!(second["args"]["clock"], "CLOCK_MONOTONIC");
    assert_eq!(
        first["args"]["clock_identity"],
        second["args"]["clock_identity"]
    );
    assert_eq!(first["args"]["clock_identity"].as_str().unwrap().len(), 36);
    assert!(
        second["args"]["origin_ns"].as_u64().unwrap()
            >= first["args"]["origin_ns"].as_u64().unwrap()
    );
}

#[test]
fn two_process_recorder_observes_direct_owner_success_and_failure() {
    use std::os::unix::fs::PermissionsExt;
    let fixture = tempfile::tempdir().unwrap();
    for (program, expected, boundary) in [("true", 0, "none"), ("false", 1, "request")] {
        let handle = start(fixture.path());
        // This is a real command directly invoked by its owner, not recorder argv.
        let status = Command::new(program).status().unwrap();
        let saved = status.code().unwrap();
        let finish = recorder(
            fixture.path(),
            &["record-finish", &handle, &saved.to_string(), boundary],
        );
        assert_eq!(saved, expected);
        assert!(finish.status.success(), "{finish:?}");
        let document = trace(fixture.path(), &handle);
        let events = document["traceEvents"].as_array().unwrap();
        let begin = events.iter().find(|e| e["ph"] == "B").unwrap();
        let end = events.iter().find(|e| e["ph"] == "E").unwrap();
        assert_eq!(begin["name"], "tsc-request");
        assert_eq!(begin["args"]["mode"], "verify");
        assert_eq!(begin["args"]["outcome"], "unknown");
        assert_eq!(begin["args"]["source_identity"], "unknown");
        assert_eq!(begin["args"]["tool_identity"], "unknown");
        assert_ne!(begin["args"]["run_id"], "inherited");
        assert!(begin["args"].get("parent_span_id").is_none());
        assert!(begin["args"].get("signal").is_none());
        assert_eq!(
            std::fs::metadata(
                fixture
                    .path()
                    .join(format!(".xtask/run/{handle}.trace.json"))
            )
            .unwrap()
            .permissions()
            .mode()
                & 0o777,
            0o600
        );
        assert_eq!(begin["args"]["output"], "raw output not archived");
        assert_eq!(end["args"]["exit_code"], expected);
        assert_eq!(end["args"]["boundary"], boundary);
        assert!(end["ts"].as_u64().unwrap() >= begin["ts"].as_u64().unwrap());
        assert_eq!(begin["pid"], end["pid"]);
    }
}
