# #1122 — Fail-fast local hook execution

Issue: [#1122](https://github.com/jaunder-org/jaunder/issues/1122). Milestone:
Developer tooling & DX.

## Outcome

`cargo xtask precommit` and `cargo xtask prepush` stop after the first blocking
failure, preserving the failed step's actionable output instead of paying for
work that cannot make the hook pass. Explicit `check` and `validate` commands
remain exhaustive diagnostic gates.

## Load-bearing decisions

- Fail-fast is an execution policy selected by both local hook commands, not a
  new meaning for `StepResult` and not an implicit consequence of
  `CommandResult::ok`.
- The policy applies at every ordered local-hook boundary, including individual
  static checks, host-gate steps, and prepush phases. A failure anywhere stops
  later work; it is not restricted to a hard-coded list of “cheap” steps.
- Prepush retains its existing clean-tree precondition as the first boundary. A
  clean-tree failure still prevents every gate phase from starting.
- Precommit always takes its after-snapshot, computes the conservative staging
  plan, and applies or rejects that plan after gate execution stops. Safe
  formatter mutations remain eligible for restaging; unsafe or ambiguous Git
  state continues to fail closed.
- Unexecuted work is absent from the command result. It is not represented as a
  green skipped step. The first failed step retains its existing name, detail,
  duration, and remediation, followed by the normal failed-command summary.
- `cargo xtask check`, `cargo xtask validate`, and their CI surfaces retain
  exhaustive execution. Their existing ordered diagnostics and authority do not
  change.
- The policy is orchestration-owned. Individual check implementations continue
  to return ordinary step results and do not gain hook-specific branching.
- ADR-0029 gains a #1122 supplement recording fail-fast local hooks and
  mandatory precommit reconciliation. `docs/ARCHITECTURE.md` and
  `CONTRIBUTING.md` project the same command-policy split. No new ADR is
  required because this refines the accepted Git-hook gate.

## Acceptance

- A synthetic failure in the first static check of `precommit` prevents every
  later host check from running while the post-gate Git/index reconciliation
  still runs exactly once.
- A synthetic host-gate failure in `prepush` prevents product tests and
  workspace doctests from running.
- A synthetic product-test failure in `prepush` prevents workspace doctests from
  running.
- The first failed step remains in the result with its original diagnostic;
  later unexecuted steps are absent rather than marked skipped.
- A failed prepush clean-tree precondition continues to prevent all gate work.
- Equivalent synthetic failures under exhaustive execution still allow later
  steps to run, proving `check` and `validate` policy is unchanged.
- Production command-graph tests consume the same execution-policy and phase
  definitions used by the commands rather than a separately maintained shadow
  list.
- Existing safe-restage, mixed-state, user-unstaged, delete/rename, and
  untracked precommit tests remain green.
- Focused xtask tests and `cargo xtask check --no-test` pass.

## Boundaries

- No changed-path routing, parallel step execution, receipt cache, retry policy,
  timeout policy, or reordering of the existing gate graph.
- No change to which checks belong to precommit, prepush, check, validate, or
  CI.
- No weakening of clean-tree, staged-subset, formatter restaging, or fail-closed
  Git/index safety.
- No synthetic skipped results for work that did not execute.
- No change to child-process output capture or step-specific remediation text.
