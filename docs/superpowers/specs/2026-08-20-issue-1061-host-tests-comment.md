# #1061 — correct the host test workspace-boundary comment

Issue: [#1061](https://github.com/jaunder-org/jaunder/issues/1061). Milestone:
Developer tooling & DX. Relevant decisions:
[ADR-0028](../../adr/0028-devtool-vs-xtask-boundary.md),
[ADR-0141](../../adr/0141-cargo-workspace-execution-boundaries.md).

## Summary

`xtask/src/steps/host_tests.rs` overstates why the host-side unit-test steps
exist. It currently says both `xtask` and the whole `tools/` workspace are
excluded from every Nix check. That is false for `tools/`: `tools/devtool` and
other auxiliary tooling can be built or run by Nix static checks.

The intended invariant is narrower and is already recorded by ADR-0141: root
application coverage and Nix test gates do not execute the `xtask` and `tools/`
unit-test suites, so the host ladder runs explicit `cargo test --manifest-path`
steps for both auxiliary workspaces.

## Decisions

| ID  | Decision                                                                                                                                                                                                                           |
| --- | ---------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| D1  | Change only comments in `xtask/src/steps/host_tests.rs`; do not change `xtask-tests` or `tools-test` commands, step names, ordering, workspace membership, Nix source filters, or coverage behavior.                               |
| D2  | Phrase the top-level `run` doc comment around unit-suite execution, not around all code coverage by Nix: these host steps compensate for unit tests that application coverage/Nix test gates otherwise do not run.                 |
| D3  | Keep the `tools-test` inline comment specific to the `tools/` unit suite. It may mention that tool crates can still be compiled or used elsewhere by static checks, but it must not claim `tools/` is absent from every Nix check. |
| D4  | No ADR change. ADR-0141 already records the workspace boundary and names #1061 as the stale-comment follow-up.                                                                                                                     |

## Acceptance criteria

- **AC1 — false exclusion claim removed.** `xtask/src/steps/host_tests.rs` no
  longer says or implies that the entire `tools/` workspace is excluded from
  every Nix check.
- **AC2 — compensating-test rationale retained.** The comments still explain
  that `xtask-tests` and `tools-test` exist because root application coverage
  and Nix test gates do not execute those auxiliary workspaces' unit-test
  suites.
- **AC3 — behavior unchanged.** The two `result.push(step(...))` calls remain
  behaviorally identical: same step names, commands, arguments, and order.
- **AC4 — decision consistency.** The wording remains consistent with ADR-0028's
  `devtool`/`xtask` execution boundary and ADR-0141's cargo-workspace execution
  boundary.
- **AC5 — gate proof.** `devtool run -- cargo xtask check --no-test` passes
  after the comment-only change.

## Out of scope

- Changing `host_tests` commands or adding/removing gate steps.
- Changing workspace membership, Nix source filters, static-check inputs, or
  coverage behavior.
- Adding a new ADR.
