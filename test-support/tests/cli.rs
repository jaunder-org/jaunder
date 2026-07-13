//! Out-of-process smoke test for the `test-support` binary's `main`.
//!
//! Coverage is process-scoped and a spawned instrumented binary's `.profraw`
//! MERGES into the run (the same mechanism `server/tests/misc/cli_subprocess.rs`
//! uses to cover `jaunder`'s `main`). Driving the no-database `reset-mail` and
//! `capture-path` subcommands here exercises `main`'s entry, `Cli::parse`, the
//! dispatch `match`, those two arms, and the final `Ok(())` — so those lines no
//! longer need a `cov:ignore`. The three database-backed arms stay marked (a
//! subprocess can't reach them without a live DB).
//!
//! These set `JAUNDER_CAPTURE_DIR` on the spawned child's env (not the parent
//! process), so there is no process-global `set_var` and nothing to serialize.

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

/// An unset `JAUNDER_CAPTURE_DIR` is a misconfiguration, not a silent no-op: the
/// e2e-only tool must fail loudly (preserving the loud failure the old required
/// `--path` arg gave).
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
