# #1800: fixture ownership follow-up

## Outcome and approval

Tests reclaim their own temporary Git repositories and the media file created by
an intentional cleanup-failure test. Whole #1800 remains open: this follow-up
does not resolve the separately unqualified checkout-permission complaint.

Direct User decision: **“1800: both repairs approved”**, relayed by Manager as
**1800-fixture-ownership-both-v1**. This approves the two reviewed scopes and
regression seams below, not a future PR merge. Commit this working contract
through the normal hook before implementation; no separate outline is needed.

## Approach and tradeoffs

The shared `xtask/src/test_support.rs` Git fixture currently returns a path
without a teardown owner. Use the existing `tempfile` library to establish an
RAII owner before Git initialization, so normal scope exit and panic unwinding
also cover initialization. Migrate its actual callers in `git`, `doc_links`,
`adr/promote`, `lifecycle`, `steps/doctest_fences`, and `e2e_provenance`,
including wrappers that return ownership. Preserve Git-environment scrubbing and
all existing behavioral assertions. Remove redundant manual teardown only in
these migrated callers. Owners must survive final Git/filesystem assertions;
provenance owners must outlive `Request` destruction and subsequent sidecar
reads.

In `elisp/test/jaunder-pull-media-test.el`, change only the cleanup-warning
test: bind `temporary-file-directory` to its existing owned root using
`file-name-as-directory`. Restore mocks before outer directory cleanup. Preserve
its primary acquisition-error and cleanup-warning assertions. The production
cleanup failure remains intentionally exercised, but the test root can reclaim
the transient afterward. This avoids a new framework or production change.

The source explains all 24 recorded Git fixture names (19 links, five
provenance, PID 1159791) within one retained invocation, not global temporary
files. The media mechanism matches the recorded prefix, not proven attribution
of its random suffix. Neither merged #1811 nor #1813 repairs these paths.

## Acceptance

- Exercise the actual shared Git helper: the repository is valid while owned,
  and its directory is absent after normal return, error return, and panic
  unwinding. Retain ownership before initialization and through dependent uses.
- Run focused regressions and affected xtask suites through existing host-test
  coverage without weakening Git scrubbing or existing assertions.
- In the existing media GET mock, capture the transient path. Assert confinement
  inside the owned root and presence despite injected deletion failure. After
  fixture teardown, assert both transient and root are absent. Preserve cleanup
  if an assertion fails, and preserve both existing error/warning assertions.
- Run focused ERT proof of that test before broader required lifecycle gates.
  Record real commands, exits, and logs in ignored `.xtask/`, not source
  evidence.

## Boundaries and lifecycle

No production behavior, historical-root cleanup, cache policy, permission
assumptions, resource tuning, framework, or unrelated refactor. No abort/SIGKILL
cleanup guarantee. Preserve the assigned semantic branch, checkout, ignored
evidence, and protected residual roots. Keep media/cache/permission attribution
limits explicit. A new follow-up PR requires separate merge approval; do not
revive merged #1813. Delete this working spec after final conformance under the
normal artifact lifecycle rather than archiving it.
