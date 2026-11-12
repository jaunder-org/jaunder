//! ERT retains its bounded Just-request event; TSC now belongs to the shared leaf.
#![cfg(unix)]
use std::{fs, os::unix::fs::PermissionsExt, path::Path, process::Command, time::Instant};

fn fixture() -> tempfile::TempDir {
    let root = tempfile::tempdir().unwrap();
    fs::create_dir(root.path().join("bin")).unwrap();
    fs::create_dir(root.path().join("recipes")).unwrap();
    fs::copy(
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../../recipes/static.just"),
        root.path().join("recipes/static.just"),
    )
    .unwrap();
    let child = root.path().join("bin/emacs");
    fs::write(&child, "#!/bin/sh\nprintf '%s\\n' \"$@\" >> argv\nprintf 'workload stdout\\n'\nprintf 'workload stderr\\n' >&2\nsleep 0.03\nexit \"$CHECK_EXIT\"\n").unwrap();
    fs::set_permissions(child, fs::Permissions::from_mode(0o755)).unwrap();
    root
}

fn completion(stderr: &[u8]) -> serde_json::Value {
    let stderr = std::str::from_utf8(stderr).unwrap();
    let events: Vec<_> = stderr
        .lines()
        .filter(|line| line.contains("devtool-check-timing"))
        .collect();
    assert_eq!(events.len(), 1, "{stderr}");
    assert!(events[0].len() < 256);
    serde_json::from_str(events[0]).unwrap()
}

#[test]
fn ert_preserves_arguments_streams_status_and_elapsed_completion() {
    for (exit, fix) in [("0", false), ("0", true), ("23", true)] {
        let root = fixture();
        let mut paths = vec![root.path().join("bin")];
        paths.extend(std::env::split_paths(&std::env::var_os("PATH").unwrap()));
        let mut command = Command::new(env!("CARGO_BIN_EXE_devtool"));
        command.args(["check", "ert"]);
        if fix {
            command.arg("--fix");
        }
        let outer = Instant::now();
        let output = command
            .current_dir(root.path())
            .env("PATH", std::env::join_paths(paths).unwrap())
            .env("CHECK_EXIT", exit)
            .output()
            .unwrap();
        let outer_ms = outer.elapsed().as_millis();
        assert_eq!(output.status.success(), exit == "0");
        assert_eq!(
            fs::read_to_string(root.path().join("argv")).unwrap(),
            "--batch\n-Q\n-l\nelisp/scripts/run-tests.el\n"
        );
        assert!(String::from_utf8_lossy(&output.stdout).contains("workload stdout"));
        assert!(String::from_utf8_lossy(&output.stderr).contains("workload stderr"));
        let event = completion(&output.stderr);
        assert_eq!(event["schema_version"], 2);
        assert_eq!(event["check"], "ert");
        assert_eq!(event["timing_scope"], "request");
        assert_eq!(
            event["outcome"],
            if exit == "0" { "success" } else { "failure" }
        );
        let duration = u128::from(event["duration_ms"].as_u64().unwrap());
        assert!(duration >= 20 && duration <= outer_ms, "{event}");
    }
}

#[test]
fn missing_just_is_spawn_failure_without_fabricated_completion() {
    let root = fixture();
    let output = Command::new(env!("CARGO_BIN_EXE_devtool"))
        .args(["check", "ert"])
        .current_dir(root.path())
        .env("PATH", root.path().join("bin"))
        .output()
        .unwrap();
    assert!(!output.status.success());
    assert!(!root.path().join("argv").exists());
    assert!(!String::from_utf8_lossy(&output.stderr).contains("devtool-check-timing"));
}

#[test]
fn recipe_failure_is_a_completed_failed_request_not_spawn_failure() {
    let root = fixture();
    fs::write(
        root.path().join("recipes/static.just"),
        "not a valid recipe\n",
    )
    .unwrap();
    let output = Command::new(env!("CARGO_BIN_EXE_devtool"))
        .args(["check", "ert"])
        .current_dir(root.path())
        .output()
        .unwrap();
    assert!(!output.status.success());
    assert!(!root.path().join("argv").exists());
    let event = completion(&output.stderr);
    assert_eq!(event["outcome"], "failure");
    assert_eq!(event["timing_scope"], "request");
}
