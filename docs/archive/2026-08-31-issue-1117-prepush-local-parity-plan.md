# #1117 Fast Local Prepush Parity Implementation Outline

> Execute with `jaunder-iterate`; delegate an individual task with
> `jaunder-dispatch` when useful. This outline exists because the change extends
> the ADR-0029 git-hook contract and shares doctest policy between host and Nix
> execution.

## Scope

In:

- Host execution and bidirectional reconciliation of the root-workspace doctest
  population.
- Prepush integration after cache-warming product tests.
- ADR-0029, architecture, and contributor parity documentation.

Out:

- Nix, coverage/CRAP, wasm, Elisp coverage, wasm budget, e2e, server-function
  flow verification, routing, fail-fast, or receipt-cache changes.

## Task outline

- [x] Task 1: Add the `workspace-doctests` host verdict
  - Contract: `xtask/src/steps/doctest_fences.rs` owns a root-workspace runner
    that executes exactly `cargo test --workspace --doc`, scans
    `doctests::roots::WORKSPACE`, and reconciles run output against scanned
    executable fences in both directions. It reports the stable
    `workspace-doctests` result step and preserves the existing xtask/tools
    host-root verdict without duplicating either population.
  - Verification: focused xtask tests prove exact arguments, command failure,
    passing execution, a reported failed entry, scanned-without-run,
    run-without-scan, duplicate-run, and misclassified-fence outcomes.
- [x] Task 2: Wire the new verdict into prepush exactly once
  - Contract: `run_local_push_gate` retains the clean-tree-first verify host
    surface and existing auxiliary tests, runs `test-local`, then
    `doctest_fences`' `workspace-doctests` verdict with the existing
    compilation-cache environment where applicable; no Nix step or command
    enters prepush.
  - Verification: focused command-graph tests prove ordering, uniqueness, cache
    environment, clean-tree precedence, and Nix absence; an exercised prepush
    path reports `workspace-doctests`.
- [x] Task 3: Publish the per-surface authority contract
  - Contract: append the #1117 supplement to ADR-0029 and project the same table
    into `docs/ARCHITECTURE.md` and `CONTRIBUTING.md`. Each `validate --no-e2e`
    surface names prepush coverage or its precise hermetic-only rationale;
    full-validate server-function flow verification is a separate row.
  - Verification: `cargo xtask check --no-test` and applicable documentation
    checks pass; the three documents do not claim literal environment parity.

## Risk checks

- The local and Nix doctest paths share `doctests::roots::WORKSPACE` and the
  same reconciliation policy; neither can silently shrink the fence population.
- Root doctests run after product tests so ordinary prepushes benefit from warm
  Cargo artifacts; auxiliary host tests remain exactly once.
- The Nix doctest producer/gate remains unchanged and authoritative for the
  pinned sandbox/offline environment.
- Prepush remains verify-only, clean-tree-gated, and Nix-free.
- `CONTEXT.md` is unchanged: this modifies developer verification policy, not
  Jaunder's ubiquitous language.
