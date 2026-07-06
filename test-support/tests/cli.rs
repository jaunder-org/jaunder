//! Out-of-process smoke test for the `test-support` binary's `main`.
//!
//! Coverage is process-scoped and a spawned instrumented binary's `.profraw`
//! MERGES into the run (the same mechanism `server/tests/misc/cli_subprocess.rs`
//! uses to cover `jaunder`'s `main`). Driving the no-database `reset-mail`
//! subcommand here exercises `main`'s entry, `Cli::parse`, the dispatch `match`,
//! the `ResetMail` arm, and the final `Ok(())` — so those lines no longer need a
//! `cov:ignore`. The three database-backed arms stay marked (a subprocess can't
//! reach them without a live DB).

use std::process::Command;

/// `reset-mail --path <file>` deletes the mail-capture file and exits 0 — no
/// database required, so a plain subprocess can drive `main` end to end.
#[test]
fn reset_mail_removes_the_capture_file_and_exits_ok() {
    let dir = tempfile::tempdir().expect("tempdir");
    let capture = dir.path().join("mail-capture.mbox");
    std::fs::write(&capture, b"queued mail").expect("seed capture file");
    assert!(capture.exists(), "capture file should exist before the run");

    let status = Command::new(env!("CARGO_BIN_EXE_test-support"))
        .args(["reset-mail", "--path"])
        .arg(&capture)
        .status()
        .expect("spawn test-support binary");

    assert!(status.success(), "reset-mail should exit 0, got {status:?}");
    assert!(
        !capture.exists(),
        "reset-mail should have deleted the capture file"
    );
}
