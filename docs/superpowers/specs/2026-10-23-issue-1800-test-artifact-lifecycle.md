# Issue #1800: own devtool run test fixtures

## Outcome

The `devtool-run-test.*` and `websub-capture-*` unit-test directories and their
captured output are removed at ordinary test scope exit, including panic
unwinding. Tests retain access to their fixtures until all dependent assertions
finish.

## Approach and tradeoffs

Return the existing `tempfile::TempDir` owner from the test helper in
`tools/devtool/src/run.rs`; callers borrow its path while retaining the owner.
Bind owners explicitly at temporary-expression `tmp().join(...)` callers too.
Use the actual helper and `execute()` seam for lifecycle regressions, not a
replacement cleanup implementation. Existing `TempDir` drop behavior is enough;
no custom cleanup framework or production change is needed.

In `server/src/websub/file_capture.rs`, use a unique test-owned `TempDir` rather
than a PID-named directory and manual file-only deletion. Retain the guard
through the existing append/content assertions. Prove directory removal after
normal scope exit and after a deliberate panic with the guard inside the
unwinding scope. Production capture retention is unchanged.

## Acceptance

- Successful and failed child runs retain directories and captured output while
  the fixture is owned, preserving their existing exit/result assertions.
- After each owner scope exits, its fixture directory is absent.
- A caught intentional panic after child execution proves unwind cleanup and
  verifies the expected panic, without masking setup or assertion failures.
- Obtain focused red/green runtime proof and run the affected `run::tests`
  slice. Keep logs and diff/review artifacts in ignored `.xtask/`, not source
  docs.
- Confine invocation-owned `TMPDIR` and fixtures to this checkout's `.xtask/`.

## Boundaries and approval

Direct User **“1800: Sure”**, relayed by Manager against the reviewed
`issue-1800-repair-proposal.md`, approves this test-only helper/callers repair
and successful/failed-child plus panic-unwind regression seam. Soundness review
reported no findings. This document records that unchanged approved contract.
Direct User **“1800: approved.”**, relayed by Manager immediately against the
reviewed WebSub proposal, additionally approves that test-only fixture repair
and normal/unwind regression seam (`result:soundness#3`: no findings).

The current execution assignment explicitly withholds staging, spec commit,
other commits, gates, pushes and merge; record the spec now without committing.
No outline is needed for this narrow local ownership repair.

No production retention sites, artifact producers beyond these two test scopes,
existing artifacts, permissions, global temp policy, configuration or dependency
changes are in scope. Ordinary drop/unwind cleanup does not promise
abort/SIGKILL cleanup. The other reported directory families and
unwritable-checkout complaint remain separate diagnostics owned outside this
patch.
