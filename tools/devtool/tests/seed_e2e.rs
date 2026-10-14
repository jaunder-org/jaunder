//! Command-root endpoint handoff to both canonical seed process kinds.
#![cfg(unix)]

use std::ffi::OsStr;
use std::os::unix::ffi::OsStrExt;
use std::os::unix::fs::PermissionsExt;
use std::process::Command;

fn assert_seed_endpoint(input: Option<&OsStr>, expected: &[u8]) {
    let dir = tempfile::tempdir().expect("seed fixture directory");
    let fixture = dir.path().join("seed-child");
    // The child boundary records bytes without interpreting telemetry policy.
    std::fs::write(
        &fixture,
        "#!/bin/sh\nprintf '%s\\n' \"$JAUNDER_OTEL_EXPORTER_OTLP_ENDPOINT\" >> \"$JAUNDER_CAPTURE_DIR/$JAUNDER_E2E_SEED_PROCESS\"\n",
    )
    .expect("write seed child fixture");
    std::fs::set_permissions(&fixture, std::fs::Permissions::from_mode(0o700))
        .expect("make seed child executable");
    let mut command = Command::new(env!("CARGO_BIN_EXE_devtool"));
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
    let output = command.output().expect("run real devtool seed-e2e");
    assert!(output.status.success(), "{output:?}");
    for process in ["e2e.seed.jaunder", "e2e.seed.test-support"] {
        let delivered = std::fs::read(dir.path().join(process)).expect("child endpoint record");
        assert!(!delivered.is_empty(), "{process} must run");
        for endpoint in delivered
            .strip_suffix(b"\n")
            .expect("terminated record")
            .split(|b| *b == b'\n')
        {
            assert_eq!(endpoint, expected, "endpoint delivered to {process}");
        }
    }
}

#[test]
fn caller_endpoint_reaches_both_seed_processes() {
    assert_seed_endpoint(
        Some(OsStr::new("http://127.0.0.1:54317")),
        b"http://127.0.0.1:54317",
    );
}

#[test]
fn absent_endpoint_preserves_standalone_fallback() {
    assert_seed_endpoint(None, b"http://127.0.0.1:4317");
}

#[test]
fn explicit_vm_endpoint_reaches_both_seed_processes() {
    assert_seed_endpoint(
        Some(OsStr::new("http://127.0.0.1:4317")),
        b"http://127.0.0.1:4317",
    );
}

#[test]
fn present_blank_endpoint_is_not_replaced_by_fallback() {
    assert_seed_endpoint(Some(OsStr::new("")), b"");
    assert_seed_endpoint(Some(OsStr::new(" \t ")), b" \t ");
}

#[test]
fn configured_endpoint_whitespace_is_left_to_child_telemetry() {
    assert_seed_endpoint(
        Some(OsStr::new("  http://127.0.0.1:54317 \t")),
        b"  http://127.0.0.1:54317 \t",
    );
}

#[test]
fn non_unicode_endpoint_is_left_to_child_telemetry() {
    assert_seed_endpoint(Some(OsStr::from_bytes(b"\xff")), b"\xff");
}
