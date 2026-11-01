#![cfg(target_os = "linux")]

use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

#[test]
fn signal_child_fixture() {
    let Ok(mode) = std::env::var("TRACED_RUN_SIGNAL_TEST") else {
        return;
    };
    let mut signals = signal_hook::iterator::Signals::new([
        signal_hook::consts::SIGINT,
        signal_hook::consts::SIGTERM,
    ])
    .unwrap();
    let mut descendant = Command::new("sleep")
        .arg("60")
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .unwrap();
    std::fs::write(
        "pids",
        format!("{} {}", std::process::id(), descendant.id()),
    )
    .unwrap();
    let reaper = std::thread::spawn(move || descendant.wait().unwrap());
    for signal in signals.forever() {
        std::fs::write("forwarded", signal.to_string()).unwrap();
        if mode == "handled" {
            reaper.join().unwrap();
            std::process::exit(23);
        }
        if mode == "default" {
            signal_hook::low_level::emulate_default_handler(signal).unwrap();
        }
    }
}

fn signal_case(signal: i32, mode: &str, expected: i32) {
    let fixture = tempfile::tempdir().unwrap();
    let mut root = Command::new(env!("CARGO_BIN_EXE_traced-run"))
        // Hooks themselves run under the cargo-xtask traced launcher. Each
        // fixture owns an independent root, not a span in the hook's trace.
        .env_remove("JAUNDER_TRACED_RUN_CONTEXT")
        .current_dir(fixture.path())
        .env("TRACED_RUN_SIGNAL_TEST", mode)
        .args(["--launcher", "--name", "signal-test", "--"])
        .arg(std::env::current_exe().unwrap())
        .args(["--exact", "signal_child_fixture", "--nocapture"])
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .unwrap();
    let deadline = Instant::now() + Duration::from_secs(10);
    while !fixture.path().join("pids").exists() {
        if Instant::now() >= deadline {
            root.kill().unwrap();
            root.wait().unwrap();
            panic!("child did not become ready");
        }
        std::thread::sleep(Duration::from_millis(10));
    }
    // Target only this test's owned launcher, never a name/pattern or ambient group.
    assert!(
        Command::new("kill")
            .args([format!("-{signal}"), root.id().to_string()])
            .status()
            .unwrap()
            .success()
    );
    let status = loop {
        if let Some(status) = root.try_wait().unwrap() {
            break status;
        }
        if Instant::now() >= deadline {
            root.kill().unwrap();
            root.wait().unwrap();
            panic!("launcher failed to bound signal teardown");
        }
        std::thread::sleep(Duration::from_millis(10));
    };
    assert_eq!(status.code(), Some(expected));
    assert_eq!(
        std::fs::read_to_string(fixture.path().join("forwarded")).unwrap(),
        signal.to_string()
    );
    for pid in std::fs::read_to_string(fixture.path().join("pids"))
        .unwrap()
        .split_whitespace()
    {
        let stat = std::fs::read_to_string(format!("/proc/{pid}/stat"));
        assert!(
            stat.is_err()
                || stat
                    .unwrap()
                    .split(')')
                    .nth(1)
                    .unwrap()
                    .trim()
                    .starts_with('Z'),
            "owned child {pid} remains running"
        );
    }
    let artifacts: Vec<_> = std::fs::read_dir(fixture.path().join(".xtask/run"))
        .unwrap()
        .map(|entry| entry.unwrap().path())
        .collect();
    let trace = artifacts
        .iter()
        .find(|path| path.to_string_lossy().ends_with(".trace.json"))
        .expect("handled interruption must finalize the root trace");
    let trace: serde_json::Value = serde_json::from_slice(&std::fs::read(trace).unwrap()).unwrap();
    assert_eq!(trace["traceEvents"][0]["name"], "signal-test");
    assert!(!artifacts.iter().any(|path| matches!(
        path.extension().and_then(|s| s.to_str()),
        Some("out" | "err")
    )));
}

#[test]
fn launcher_forwards_sigint_and_preserves_signal_outcome() {
    signal_case(2, "default", 130);
}

#[test]
fn launcher_forwards_sigterm_and_preserves_signal_outcome() {
    signal_case(15, "default", 143);
}

#[test]
fn launcher_preserves_signal_handlers_exit_status() {
    signal_case(15, "handled", 23);
}

#[test]
fn launcher_kills_signal_ignoring_children_after_grace() {
    signal_case(2, "ignore", 137);
}
