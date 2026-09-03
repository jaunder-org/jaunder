# #1236 — delegate `devtool run` supervision to processkit

Issue: [#1236](https://github.com/jaunder-org/jaunder/issues/1236). Milestone:
Developer tooling & DX.

## Summary

`tools/devtool/src/run.rs` currently supervises a child directly: it redirects
stdout and stderr to `.xtask/run/`, polls `Child::try_wait` every 50 ms when a
timeout is configured, and kills and waits for only the direct child at the
deadline. That leaves descendants outside teardown and provides no kill-on-drop
backstop if the synchronous runner unwinds.

Replace that lifecycle code with processkit 3.3.4, following xtask's established
private synchronous-to-async boundary. Keep the `devtool run` command line,
parked artifacts, JSON schema, and process exit behavior unchanged.

## Decisions

| ID     | Decision                                                                                                                                                                                                                                                                                                                                                                          |
| ------ | --------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| **D1** | `devtool` owns a private one-worker Tokio runtime and drives processkit through `block_on`; no asynchronous type or API crosses the existing synchronous `execute`/`run` boundary.                                                                                                                                                                                                |
| **D2** | Build a `processkit::Command` from the existing argv, cwd, inherited environment, and null stdin. Configure `stdout_file` and `stderr_file` truncate redirects to the allocated `.xtask/run/<id>.out` and `.err` paths. These redirects preserve the current byte-exact, separate, file-backed presentation without an unnecessary userspace tee or in-memory capture.            |
| **D3** | Apply the existing optional integer-seconds timeout with processkit's command timeout. Wait for processkit's terminal `Outcome`; processkit owns whole-tree containment, deadline teardown, reaping, teardown confirmation, and kill-on-drop behavior. Remove `wait_with_timeout`, its 50 ms polling, and the direct-child kill/wait helper.                                      |
| **D4** | Translate `Outcome` back into the existing `Capture`/`RunResult` contract. Natural exits retain their code. Unix signal exits retain the existing signal number/name and `128 + signal` process exit. A timeout retains `timed_out: true`, process exit 124, and the current Unix `SIGKILL` signal presentation produced by the former hard-kill path; it remains non-successful. |
| **D5** | Keep artifact allocation, post-run byte/LF counting, pruning, sanitized ancillary warnings, argv validation, pretty JSON output, and runner-error `{error, kind}`/exit-64 behavior in Jaunder code. They are presentation and policy, not generic process supervision.                                                                                                            |
| **D6** | Persistent behavioral tests cover finite success, non-zero exit, signal termination, and timeout while asserting the externally visible result fields and parked output where relevant. Existing validation, counting, pruning, and warning tests remain.                                                                                                                         |
| **D7** | Whole-tree timeout and unwind/drop teardown are checked once against a real parent-plus-descendant process during implementation. No permanent test duplicates processkit's own containment suite; the repository tests only Jaunder's adapter and contract.                                                                                                                      |
| **D8** | No public cancellation option is added. Dropping the private process owner is the unwind/cancellation path. Ordinary finite `Command::status`/`output` call sites and PostgreSQL `pg_ctl` lifecycle are unchanged.                                                                                                                                                                |
| **D9** | No shared adapter is extracted from xtask. The workspaces are separate, both bridges are private, and their orchestration needs differ. No ADR: adopting the already-selected library at a second process-lifecycle seam is local, unsurprising, and reversible.                                                                                                                  |

## Acceptance criteria

- **AC1 — contract preserved.** For successful, non-zero, signalled, and
  timed-out children, `devtool run` retains its existing JSON field names and
  omission rules, command argv, exit-code/signal mapping, `ok`, `duration_ms`,
  parked stdout/stderr paths, byte counts, LF counts, and process exit status.
  Timeout still exits 124 and reports `timed_out: true`.
- **AC2 — processkit owns lifecycle.** The launched command runs in processkit's
  private process group. Natural completion and timeout resolve only after the
  child tree is reaped or teardown failure is reported. Dropping during unwind
  terminates the contained tree.
- **AC3 — manual supervisor removed.** `run.rs` no longer polls
  `Child::try_wait`, sleeps between polls, or directly calls child kill/wait.
- **AC4 — focused tests.** Repository tests exercise finite success, non-zero,
  signal, and timeout outcomes through the synchronous adapter and assert the
  visible devtool contract rather than processkit internals.
- **AC5 — scope held.** No other command runner and no PostgreSQL cluster
  lifecycle changes.

## Verification

1. Run the focused devtool test target through `cargo xtask test-local`,
   including the four terminal-outcome scenarios.
2. Run a one-off real-process probe that creates a descendant, then demonstrate
   both timeout teardown and unwind/drop teardown leave neither process alive.
   Record the observed result in the PR; do not commit the probe.
3. Run `cargo xtask check --no-test` during iteration.
4. Stage the intended tree and let the commit gate run `cargo xtask precommit`.
5. Before delivery, run `cargo xtask validate --no-e2e`; the change affects a
   command wrapper and process lifecycle but not browser behavior.

## Risks

- **Outcome translation drift.** Processkit represents timeout as
  `Outcome::TimedOut`, not the killed child's raw status. The adapter must
  retain devtool's established exit-124 and Unix signal presentation explicitly.
- **Runtime/drop ordering.** The `RunningProcess` must drop while its owning
  runtime can still service processkit teardown. The private owner field order
  and one-off unwind probe make this boundary explicit.
- **Platform containment differs.** Processkit selects the host mechanism (for
  example Linux cgroup v2 with process-group fallback or a Windows Job Object).
  The adapter must consume processkit's outcome/accessors rather than recreate
  platform status logic beyond devtool's existing signal presentation.
