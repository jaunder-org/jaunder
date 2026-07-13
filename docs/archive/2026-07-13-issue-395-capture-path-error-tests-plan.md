# Plan — issue #395: cover `capture-path` error branches in `tests/cli.rs`

Spec: `docs/superpowers/specs/2026-07-13-issue-395-capture-path-error-tests.md`

Single task, test-only. No production change.

## Task 1 — add the two error-path subprocess tests

- [x] In `test-support/tests/cli.rs`, add
      `capture_path_errors_on_unknown_stream`: spawn `capture-path zzz-bogus`
      with `JAUNDER_CAPTURE_DIR` set to a tempdir, `.output()`, assert
      `!status.success()` and stderr contains `unknown capture stream`.
- [x] Add `capture_path_errors_without_capture_dir`: spawn `capture-path mail`
      with `.env_remove("JAUNDER_CAPTURE_DIR")`, assert `!status.success()` and
      stderr contains `JAUNDER_CAPTURE_DIR is not set`.
- [x] Run the gate: `cargo xtask check` → green (23/23 steps).
- [x] Commit.

## Notes

- Mirror the existing `reset_mail_errors_without_capture_dir` structure; reuse
  `tempfile::tempdir()` as the other `capture-path` test does for the dir-set
  case.
- Decode stderr with `String::from_utf8_lossy(&out.stderr)` and assert
  `.contains(...)` — robust to the `anyhow` `Error:`/backtrace framing.
