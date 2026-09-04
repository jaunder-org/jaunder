# Issue #988: Split xtask CLI grammar from dispatch — implementation outline

**Approved specification:**
`docs/superpowers/specs/2026-09-04-issue-988-split-xtask-cli-spec.md`

## 1. Extract the CLI grammar owner

**Files:** `xtask/src/lib.rs`, new `xtask/src/cli.rs`

- Move `Cli`, `Command`, every command enum currently defined in `lib.rs`, their
  Clap attributes/help text, backend/browser string conversion, command naming,
  JSON-payload policy, and grammar-only tests into `cli.rs` without rewriting
  their contents.
- Keep `issue::IssueCommand` with the issue workflow and import that nested
  grammar from `cli.rs`.
- Declare private `mod cli` in `lib.rs` and explicitly re-export every command
  type that was public at the crate root.
- Compile through the focused xtask suite after the move; resolve imports
  through semantic owners rather than aliases.

## 2. Extract lifecycle and gate orchestration

**Files:** `xtask/src/lib.rs`, new `xtask/src/lifecycle.rs`, new
`xtask/src/gate.rs`, `xtask/src/steps/static_checks.rs`, `xtask/src/issue.rs`

- Move hook installation, clean-tree precheck, precommit
  snapshot/reconciliation, and result finalization into `lifecycle.rs`.
  Re-export only `ensure_hooks_installed`; expose finalization and precommit
  helpers crate-visibly to their actual consumers.
- Update `issue::execute` to call `crate::lifecycle::finalize`, eliminating its
  former implicit dependency on a crate-root helper.
- Move `ExecutionPolicy`, `run_with_policy`, host-step catalogs/runners,
  Markdown routing, prepush phases, and gate test helpers into `gate.rs`.
  Replace `Command::execution_policy` with a crate-visible
  `gate::execution_policy(&Command)` consumed by dispatch, and move its tests
  with the policy owner; this keeps `cli` independent of `gate`.
- Remove `static_checks::{run_phase_with, run_markdown_phase_with}`. Have
  `gate.rs` apply `run_with_policy` directly to
  `static_checks::specs_for_phase`, filtered by `StepSpec::markdown_eligible`
  for the narrow path, and execute each spec through `static_checks::run_spec`.
  This preserves the existing order and callback behavior while making the
  dependency one-way: `gate -> steps`.
- Move gate-policy, catalog/order, Markdown route, prepush, and static-phase
  fail-fast/exhaustive tests to `gate.rs`. Keep pure catalog/spec-construction
  tests in `steps/static_checks.rs`; move precommit reconciliation and
  clean-tree lifecycle tests to `lifecycle.rs`.

## 3. Extract dispatch and finish the facade

**Files:** `xtask/src/lib.rs`, new `xtask/src/dispatch.rs`, `xtask/src/git.rs`

- Move `run(Cli)` and the malformed-trace result adapter into `dispatch.rs` as
  one exhaustive command-to-runner mapping.
- Preserve each match arm verbatim except for owner-qualified imports and calls.
  In particular, keep validate clean-tree short-circuiting, the #824
  `verify_after_validate`/`verify_after_combo` split, issue-owned result
  finalization, and PR cleanup's distinct result construction.
- Move full-dispatch/result-adaptation tests to `dispatch.rs`.
- Move the Git environment-scrubbing regression test beside `git::at` in
  `git.rs`.
- Reduce `lib.rs` to module declarations plus explicit re-exports of the
  existing root API: command types, `run`, `ensure_hooks_installed`, and result
  types. Add no compatibility shims and expose no leaf module publicly.

## 4. Verify behavior and repository standards

- Run the focused xtask suite:
  `devtool run -- cargo xtask test-local -- --manifest-path xtask/Cargo.toml`.
- Run `devtool run -- cargo xtask --help` and inspect the actual CLI surface.
- Smoke a structured read-only command through dispatch and result finalization:
  `devtool run -- cargo xtask --json census`.
- Run `devtool run -- cargo xtask check --no-test` once for broader static and
  Clippy feedback; apply any formatter changes before staging.
- Review the final diff against the approved spec, including explicit root
  re-exports, absence of inline `lib.rs` implementation/tests, one-way
  gate-to-steps dependencies, exact test retention, and #824 call edges.
- Stage the intended semantic tree and commit it. The pre-commit hook is the
  authoritative repository gate; if it changes files, inspect and restage only
  the intended changes before retrying the commit.

## 5. Deliver and archive

- Move the approved spec and this outline from `docs/superpowers/` to
  `docs/archive/` as the last semantic change, format them with the pinned
  Prettier, stage, and commit through the same pre-commit gate.
- Push the branch, open the PR using the repository template, and monitor with
  `cargo xtask pr watch` until it reports `ready-to-land` or a concrete failure.
- Stop at the explicit merge-approval gate. Do not run `cargo xtask pr land`
  without per-PR human approval.
