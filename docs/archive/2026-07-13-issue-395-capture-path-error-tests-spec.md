# Spec — issue #395: cover `capture-path` error branches in `tests/cli.rs`

## Problem

`test-support`'s `capture-path` subcommand (`test-support/src/main.rs`,
`cmd_capture_path`) has two error branches no test exercises:

- **unknown stream** — `capture::Stream::parse(stream)` returns `None`, yielding
  an `anyhow` error `unknown capture stream {stream:?}`.
- **unset capture dir** — `capture::file(stream)` returns `None` when
  `JAUNDER_CAPTURE_DIR` is unset, yielding `JAUNDER_CAPTURE_DIR is not set`.

`tests/cli.rs` only drives the success path (`capture-path mail`, dir set). The
error closures are single-line, so their lines already count as covered and the
coverage/CRAP gate is green — this is a **behavior-test gap, not a gate
blocker**. A regression in the parse / `None`-handling would slip through.

Surfaced during #232 (retiring `test-support::main`'s `crap:allow`); follow-up
from #232.

## Behavior to lock in

Two subprocess tests in `test-support/tests/cli.rs`, mirroring the existing
`reset_mail_errors_without_capture_dir` pattern (spawn the built binary via
`CARGO_BIN_EXE_test-support`, inspect exit status):

1. **unknown stream** — `capture-path <bogus-stream>` with `JAUNDER_CAPTURE_DIR`
   set exits non-zero, and stderr contains `unknown capture stream`.
2. **unset dir** — `capture-path mail` with `JAUNDER_CAPTURE_DIR` removed from
   the child env exits non-zero, and stderr contains
   `JAUNDER_CAPTURE_DIR is not set`.

Both assert stderr message content (not just the exit code, unlike the existing
`reset_mail_errors_without_capture_dir` which checks exit only) so a regression
in the parse / `None`-handling — the actual point of the branch — is caught, per
the acceptance criterion "exits non-zero with an ... message". `main` returns
`anyhow::Result`, so an `Err` is rendered `Error: <msg>` on stderr and exits 1.

## Non-goals / constraints

- **No production change**; `main.rs` untouched.
- Set/remove `JAUNDER_CAPTURE_DIR` on the spawned child's env only (no
  process-global `set_var`), matching the existing tests.

## Acceptance

- Both `capture-path` error paths are exercised by `test-support/tests/cli.rs`.
- No production change; `main.rs` untouched.
- The gate (`cargo xtask validate --no-e2e`) stays green.
