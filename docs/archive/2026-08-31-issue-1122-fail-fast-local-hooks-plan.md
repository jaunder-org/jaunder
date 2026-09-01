# #1122 Fail-Fast Local Hooks Implementation Outline

> Execute with `jaunder-iterate`; delegate an individual task with
> `jaunder-dispatch` when useful. This outline exists because the change adds a
> durable execution-policy boundary to the ADR-0029 Git-hook gate.

## Scope

In:

- Production-owned exhaustive versus fail-fast execution policy.
- Short-circuiting within static phases, across host-gate steps, and across
  prepush phases.
- Mandatory precommit Git/index reconciliation after early failure.
- ADR-0029, architecture, and contributor policy projection.

Out:

- Gate reordering, changed-path routing, parallelism, receipts, retries,
  timeouts, new checks, or altered command membership.
- Changes to child-process capture, individual remediation text, CI, or explicit
  `check`/`validate` exhaustiveness.

## Task outline

- [x] Task 1: Add the production execution-policy boundary
  - Contract: one orchestration-owned `ExecutionPolicy` selects `Exhaustive` or
    `FailFast`; the same policy controls individual static checks and ordered
    host-gate steps. Failure is the first newly appended blocking step result,
    while an exhaustive run continues through the unchanged graph.
  - Verification: focused xtask tests use production execution definitions and
    synthetic runners to prove first-failure stopping, later-step absence, and
    exhaustive continuation without executing live tools.
- [x] Task 2: Apply fail-fast policy to both local hooks
  - Contract: `precommit` and `prepush` select `FailFast`; `check` and
    `validate` select `Exhaustive`. Prepush stops between host, product-test,
    and workspace doctest phases. Precommit always performs its after-snapshot
    and staging reconciliation exactly once after early gate failure.
  - Verification: production-command synthetic tests prove clean-tree
    short-circuit; host failure preventing product/doctest work; product failure
    preventing doctest work; preserved diagnostics; and both `check` and
    `validate` continuing through later static and host steps after an injected
    early failure. Precommit tests prove reconciliation after early failure and
    rerun the existing safe-restage, mixed-state, user-unstaged, delete/rename,
    and untracked regression matrix.
- [x] Task 3: Publish the local-versus-exhaustive policy
  - Contract: append the #1122 refinement to ADR-0029 and project it into
    `docs/ARCHITECTURE.md` and `CONTRIBUTING.md`; documents state that
    unexecuted steps are absent, not green-skipped, and that reconciliation
    still runs.
  - Verification: focused xtask tests and `cargo xtask check --no-test` pass;
    all three documents describe the same command split.

## Risk checks

- The policy is explicit at every command caller; adding a future command cannot
  silently inherit fail-fast behavior.
- A runner that appends more than one result cannot hide a failure when the
  orchestration loop decides whether to continue.
- Early return never bypasses precommit's after-snapshot, stage plan, or
  fail-closed diagnostics.
- `CommandResult`, `StepResult`, step-specific details, and the existing gate
  order remain unchanged unless the approved spec explicitly requires it.
- Tests consume production policy/phase definitions rather than mirrored name
  lists.
