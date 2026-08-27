//! Out-of-process smoke test for the `test-support` binary's `main`.
//!
//! Coverage is process-scoped and a spawned instrumented binary's `.profraw`
//! MERGES into the run (the same mechanism `server/tests/misc/cli_subprocess.rs`
//! uses to cover `jaunder`'s `main`). Driving the no-database `reset-mail` and
//! `capture-path` subcommands here exercises `main`'s entry, `Cli::parse`, the
//! dispatch `match`, those two arms, and the final `Ok(())` — so those lines need
//! no coverage exemption. The three database-backed arms stay marked (a
//! subprocess can't reach them without a live DB).
//!
//! These pass `JAUNDER_CAPTURE_DIR` to the spawned child through `Command::env`,
//! never mutating this process's inherited configuration.

#[cfg(unix)]
use std::os::unix::ffi::OsStringExt as _;
use std::process::Command;

/// `reset-mail` derives `<JAUNDER_CAPTURE_DIR>/mail.jsonl`, deletes it, and exits 0 —
/// no database required, so a plain subprocess can drive `main` end to end.
#[test]
fn reset_mail_removes_the_derived_capture_file_and_exits_ok() {
    let dir = tempfile::tempdir().expect("tempdir");
    let capture_dir = dir.path().join("capture");
    std::fs::create_dir_all(&capture_dir).expect("mk capture dir");
    let mail = capture_dir.join("mail.jsonl");
    std::fs::write(&mail, b"queued mail").expect("seed capture file");

    let status = Command::new(env!("CARGO_BIN_EXE_test-support"))
        .arg("reset-mail")
        .env("JAUNDER_CAPTURE_DIR", &capture_dir)
        .status()
        .expect("spawn test-support binary");

    assert!(status.success(), "reset-mail should exit 0, got {status:?}");
    assert!(
        !mail.exists(),
        "reset-mail should have deleted <dir>/mail.jsonl"
    );
}

/// Both capture commands construct the configured directory before deriving a
/// leaf path, so a path below a regular file fails before any path is used.
#[test]
fn capture_commands_fail_for_an_uncreatable_capture_directory() {
    let dir = tempfile::tempdir().expect("tempdir");
    let not_a_directory = dir.path().join("not-a-directory");
    std::fs::write(&not_a_directory, b"file").expect("blocking file");
    let blocked_capture_dir = not_a_directory.join("capture");

    for args in [
        ["reset-mail"].as_slice(),
        ["capture-path", "mail"].as_slice(),
    ] {
        let out = Command::new(env!("CARGO_BIN_EXE_test-support"))
            .args(args)
            .env("JAUNDER_CAPTURE_DIR", &blocked_capture_dir)
            .output()
            .expect("spawn test-support binary");

        assert!(
            !out.status.success(),
            "{args:?} must reject an uncreatable capture directory"
        );
        let stderr = String::from_utf8_lossy(&out.stderr);
        assert!(
            stderr.contains("could not create capture directory"),
            "{args:?} must name the capture setup failure, got: {stderr}"
        );
        assert!(
            out.stdout.is_empty(),
            "{args:?} must not advertise a path after capture setup fails"
        );
        assert!(
            !String::from_utf8_lossy(&out.stderr).contains("reset mail-capture file"),
            "{args:?} must not use a path after capture setup fails"
        );
    }
}

/// An explicitly configured non-Unicode directory is invalid rather than a
/// disabled capture setting, and the child must fail without echoing its bytes.
#[cfg(unix)]
#[test]
fn capture_path_rejects_a_non_unicode_capture_directory() {
    let out = Command::new(env!("CARGO_BIN_EXE_test-support"))
        .args(["capture-path", "mail"])
        .env(
            "JAUNDER_CAPTURE_DIR",
            std::ffi::OsString::from_vec(vec![0xff]),
        )
        .output()
        .expect("spawn test-support binary");

    assert!(
        !out.status.success(),
        "capture-path must exit non-zero for a non-Unicode capture directory"
    );
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        stderr.contains("capture directory is not valid Unicode"),
        "stderr should name the redacted invalid-directory failure, got: {stderr}"
    );
}

/// An unset `JAUNDER_CAPTURE_DIR` is a misconfiguration, not a silent no-op: the
/// e2e-only tool must fail loudly.
#[test]
fn reset_mail_errors_without_capture_dir() {
    let out = Command::new(env!("CARGO_BIN_EXE_test-support"))
        .arg("reset-mail")
        .env_remove("JAUNDER_CAPTURE_DIR")
        .output()
        .expect("spawn test-support binary");

    assert!(
        !out.status.success(),
        "reset-mail must exit non-zero when JAUNDER_CAPTURE_DIR is unset"
    );
}

/// `capture-path <stream>` prints the `host`-derived absolute path — this is what
/// the Playwright readers shell out to instead of restating filenames.
#[test]
fn capture_path_prints_the_derived_absolute_path() {
    let dir = tempfile::tempdir().expect("tempdir");
    let capture_dir = dir.path().join("capture");

    let out = Command::new(env!("CARGO_BIN_EXE_test-support"))
        .args(["capture-path", "mail"])
        .env("JAUNDER_CAPTURE_DIR", &capture_dir)
        .output()
        .expect("spawn test-support binary");

    assert!(out.status.success(), "capture-path should exit 0");
    let printed = String::from_utf8(out.stdout).expect("utf8 stdout");
    assert_eq!(
        printed.trim(),
        capture_dir.join("mail.jsonl").to_string_lossy()
    );
}

#[test]
fn capture_path_initializes_telemetry_without_writing_diag_log() {
    let dir = tempfile::tempdir().expect("tempdir");
    let capture_dir = dir.path().join("capture");

    let out = Command::new(env!("CARGO_BIN_EXE_test-support"))
        .args(["capture-path", "mail"])
        .env("JAUNDER_CAPTURE_DIR", &capture_dir)
        .env(
            "JAUNDER_OTEL_EXPORTER_OTLP_ENDPOINT",
            "not a valid endpoint",
        )
        .env_remove("OTEL_EXPORTER_OTLP_ENDPOINT")
        .output()
        .expect("spawn test-support binary");

    assert!(out.status.success(), "capture-path should exit 0");
    assert!(!capture_dir.join("diag.log").exists());
    let stderr = String::from_utf8(out.stderr).expect("stderr utf8");
    assert!(
        stderr.contains("tracing export disabled")
            || stderr.contains("invalid configured value; export disabled"),
        "telemetry init fallback proves the guard ran; stderr: {stderr}"
    );
}

/// An unknown stream key is a caller error, not a silent empty path: `capture-path`
/// must reject it loudly so a typo in a Playwright reader fails fast rather than
/// shelling out to a bogus filename.
#[test]
fn capture_path_errors_on_unknown_stream() {
    let dir = tempfile::tempdir().expect("tempdir");
    let capture_dir = dir.path().join("capture");

    let out = Command::new(env!("CARGO_BIN_EXE_test-support"))
        .args(["capture-path", "zzz-bogus"])
        .env("JAUNDER_CAPTURE_DIR", &capture_dir)
        .output()
        .expect("spawn test-support binary");

    assert!(
        !out.status.success(),
        "capture-path must exit non-zero for an unknown stream"
    );
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        stderr.contains("unknown capture stream"),
        "stderr should name the unknown-stream failure, got: {stderr}"
    );
}

/// As with `reset-mail`, an unset `JAUNDER_CAPTURE_DIR` is a misconfiguration:
/// `capture-path` must fail loudly rather than derive a path from an empty base.
#[test]
fn capture_path_errors_without_capture_dir() {
    let out = Command::new(env!("CARGO_BIN_EXE_test-support"))
        .args(["capture-path", "mail"])
        .env_remove("JAUNDER_CAPTURE_DIR")
        .output()
        .expect("spawn test-support binary");

    assert!(
        !out.status.success(),
        "capture-path must exit non-zero when JAUNDER_CAPTURE_DIR is unset"
    );
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        stderr.contains("JAUNDER_CAPTURE_DIR is not set"),
        "stderr should name the unset-dir failure, got: {stderr}"
    );
}

#[test]
fn verify_no_panics_cli_accepts_clean_capture() {
    let dir = tempfile::tempdir().expect("tempdir");
    let capture = dir.path().join("capture");
    std::fs::create_dir_all(&capture).expect("capture dir");
    let server = dir.path().join("server.log");
    std::fs::write(&server, b"clean stderr\n").expect("server log");

    let out = Command::new(env!("CARGO_BIN_EXE_test-support"))
        .args(["verify-no-panics", "--capture-dir"])
        .arg(&capture)
        .arg("--server-log")
        .arg(&server)
        .output()
        .expect("spawn verifier");

    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
}

#[test]
fn verify_no_panics_cli_reports_panic_and_exits_nonzero() {
    let dir = tempfile::tempdir().expect("tempdir");
    let capture = dir.path().join("capture");
    std::fs::create_dir_all(&capture).expect("capture dir");
    let server = dir.path().join("server.log");
    std::fs::write(&server, b"panicked at src/cli.rs:8:3: boom\n").expect("server log");

    let out = Command::new(env!("CARGO_BIN_EXE_test-support"))
        .args(["verify-no-panics", "--capture-dir"])
        .arg(&capture)
        .arg("--server-log")
        .arg(&server)
        .output()
        .expect("spawn verifier");

    assert!(!out.status.success(), "panic must fail CLI");
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(stderr.contains("src/cli.rs:8:3"), "{stderr}");
}
