//! Command-root endpoint handoff to both canonical seed process kinds.
#![cfg(unix)]

use std::ffi::OsStr;
use std::os::unix::ffi::OsStrExt;
use std::os::unix::fs::PermissionsExt;
use std::process::Command;

type TestResult = Result<(), Box<dyn std::error::Error>>;

fn assert_seed_endpoint(input: Option<&OsStr>, expected: &[u8]) -> TestResult {
    let dir = tempfile::tempdir()?;
    let fixture = dir.path().join("seed-child");
    // The child boundary records bytes without interpreting telemetry policy.
    std::fs::write(
        &fixture,
        "#!/bin/sh\nprintf '%s\\n' \"$JAUNDER_OTEL_EXPORTER_OTLP_ENDPOINT\" >> \"$JAUNDER_CAPTURE_DIR/$JAUNDER_E2E_SEED_PROCESS\"\n",
    )?;
    std::fs::set_permissions(&fixture, std::fs::Permissions::from_mode(0o700))?;
    let mut command = Command::new(env!("CARGO_BIN_EXE_test-support"));
    command
        .arg("seed-e2e")
        .args(["--db", "sqlite:/tmp/seed-wiring.db", "--test-support-bin"])
        .arg(&fixture)
        .arg("--jaunder-bin")
        .arg(&fixture)
        .env("JAUNDER_CAPTURE_DIR", dir.path())
        .env(
            "OTEL_EXPORTER_OTLP_ENDPOINT",
            "http://unrelated.example:4317",
        )
        .env_remove("JAUNDER_OTEL_EXPORTER_OTLP_ENDPOINT");
    if let Some(input) = input {
        command.env("JAUNDER_OTEL_EXPORTER_OTLP_ENDPOINT", input);
    }
    let output = command.output()?;
    assert!(output.status.success(), "{output:?}");
    for process in ["e2e.seed.jaunder", "e2e.seed.test-support"] {
        let delivered = std::fs::read(dir.path().join(process))?;
        assert!(!delivered.is_empty(), "{process} must run");
        for endpoint in delivered
            .strip_suffix(b"\n")
            .ok_or("unterminated child endpoint record")?
            .split(|b| *b == b'\n')
        {
            assert_eq!(endpoint, expected, "endpoint delivered to {process}");
        }
    }
    Ok(())
}

#[test]
fn caller_endpoint_reaches_both_seed_processes() -> TestResult {
    assert_seed_endpoint(
        Some(OsStr::new("http://127.0.0.1:54317")),
        b"http://127.0.0.1:54317",
    )?;
    Ok(())
}

#[test]
fn absent_endpoint_preserves_standalone_fallback() -> TestResult {
    assert_seed_endpoint(None, b"http://127.0.0.1:4317")?;
    Ok(())
}

#[test]
fn explicit_vm_endpoint_reaches_both_seed_processes() -> TestResult {
    assert_seed_endpoint(
        Some(OsStr::new("http://127.0.0.1:4317")),
        b"http://127.0.0.1:4317",
    )?;
    Ok(())
}

#[test]
fn present_blank_endpoint_is_not_replaced_by_fallback() -> TestResult {
    assert_seed_endpoint(Some(OsStr::new("")), b"")?;
    assert_seed_endpoint(Some(OsStr::new(" \t ")), b" \t ")?;
    Ok(())
}

#[test]
fn configured_endpoint_whitespace_is_left_to_child_telemetry() -> TestResult {
    assert_seed_endpoint(
        Some(OsStr::new("  http://127.0.0.1:54317 \t")),
        b"  http://127.0.0.1:54317 \t",
    )?;
    Ok(())
}

#[test]
fn non_unicode_endpoint_is_left_to_child_telemetry() -> TestResult {
    assert_seed_endpoint(Some(OsStr::from_bytes(b"\xff")), b"\xff")?;
    Ok(())
}

#[test]
fn omitted_test_support_bin_runs_this_executable_after_jaunder_steps() {
    let dir = tempfile::tempdir().expect("seed fixture directory");
    let jaunder = dir.path().join("jaunder");
    std::fs::write(
        &jaunder,
        "#!/bin/sh\nprintf '%s\\n' \"$*\" >> \"$JAUNDER_CAPTURE_DIR/jaunder-steps\"\n",
    )
    .expect("write jaunder fixture");
    std::fs::set_permissions(&jaunder, std::fs::Permissions::from_mode(0o700))
        .expect("make jaunder fixture executable");
    // An unopenable database makes the first self-run user step fail, so the
    // fatal diagnostic names the executable the default actually selected.
    let output = Command::new(env!("CARGO_BIN_EXE_test-support"))
        .arg("seed-e2e")
        .arg("--db")
        .arg(format!(
            "sqlite:{}/missing/jaunder.db",
            dir.path().display()
        ))
        .arg("--jaunder-bin")
        .arg(&jaunder)
        .env("JAUNDER_CAPTURE_DIR", dir.path())
        .output()
        .expect("run real test-support seed-e2e");
    assert!(!output.status.success(), "{output:?}");
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains(&format!(
            "{} create-user --username testlogin",
            env!("CARGO_BIN_EXE_test-support")
        )),
        "{stderr}"
    );
    let steps = std::fs::read_to_string(dir.path().join("jaunder-steps")).expect("jaunder steps");
    assert_eq!(steps.lines().count(), 3, "{steps}");
}
