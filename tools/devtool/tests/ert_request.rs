//! ERT retains its bounded `dev`-request event; TSC now belongs to the shared leaf.
#![cfg(unix)]
use std::{fs, os::unix::fs::PermissionsExt, path::Path, process::Command, time::Instant};

/// Keeps every Git the fixture, `dev`'s shim or `dev` runs to the fixture: a
/// hook's exported repository overrides and the caller's configuration stay out.
fn isolated(command: &mut Command) -> &mut Command {
    for (name, _) in std::env::vars_os() {
        if name.to_string_lossy().starts_with("GIT_") {
            command.env_remove(name);
        }
    }
    command
        .env("GIT_CONFIG_GLOBAL", "/dev/null")
        .env("GIT_CONFIG_NOSYSTEM", "1")
}

/// A Git checkout whose `tools/dev` is this repository's runner: the devShell's
/// `dev` shim runs `tools/dev` from the checkout root it finds.
fn fixture() -> tempfile::TempDir {
    let root = tempfile::tempdir().unwrap();
    let init = isolated(Command::new("git").args(["init", "-q"]).arg(root.path()))
        .status()
        .unwrap();
    assert!(init.success());
    fs::create_dir_all(root.path().join("bin")).unwrap();
    fs::create_dir(root.path().join("tools")).unwrap();
    std::os::unix::fs::symlink(
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../dev"),
        root.path().join("tools/dev"),
    )
    .unwrap();
    let child = root.path().join("bin/emacs");
    fs::write(&child, "#!/bin/sh\nprintf '%s\\n' \"$@\" >> argv\nprintf 'workload stdout\\n'\nprintf 'workload stderr\\n' >&2\nsleep 0.03\nexit \"$CHECK_EXIT\"\n").unwrap();
    fs::set_permissions(child, fs::Permissions::from_mode(0o755)).unwrap();
    root
}

fn devtool(root: &Path) -> Command {
    let mut command = Command::new(env!("CARGO_BIN_EXE_devtool"));
    command.args(["check", "ert"]).current_dir(root);
    isolated(&mut command);
    command
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
        let mut command = devtool(root.path());
        if fix {
            command.arg("--fix");
        }
        let outer = Instant::now();
        let output = command
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
fn missing_dev_is_spawn_failure_without_fabricated_completion() {
    let root = fixture();
    let output = devtool(root.path())
        .env("PATH", root.path().join("bin"))
        .output()
        .unwrap();
    assert!(!output.status.success());
    assert!(!root.path().join("argv").exists());
    assert!(!String::from_utf8_lossy(&output.stderr).contains("devtool-check-timing"));
}

#[test]
fn runner_failure_is_a_completed_failed_request_not_spawn_failure() {
    let root = fixture();
    // `dev` starts but cannot load its runner, so ERT never runs.
    fs::remove_file(root.path().join("tools/dev")).unwrap();
    fs::create_dir(root.path().join("tools/dev")).unwrap();
    let output = devtool(root.path()).output().unwrap();
    assert!(!output.status.success());
    assert!(!root.path().join("argv").exists());
    let event = completion(&output.stderr);
    assert_eq!(event["outcome"], "failure");
    assert_eq!(event["timing_scope"], "request");
}
