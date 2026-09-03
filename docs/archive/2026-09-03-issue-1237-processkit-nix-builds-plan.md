# #1237 — processkit-supervised Nix builds

Spec: `docs/superpowers/specs/2026-09-03-issue-1237-processkit-nix-builds.md`

## Approval view

**Planning trigger:** concurrency and ownership risk at the synchronous-runtime,
process-drop, and asynchronous-output boundaries.

**In scope:** extract the private process owner, migrate E2E-local callers,
implement Nix's dual raw-tee policy, migrate `build_check`, and preserve its
observable diagnostics and failure precedence.

**Out of scope:** one-shot Nix evaluation, VM artifact handling, rescue-order
changes, public cancellation, generic command wrappers, and ADR work.

**Execution:** drive each task through `jaunder-iterate`; use `jaunder-dispatch`
only for an independently owned task. Tick each task after its focused evidence,
then use `jaunder-commit`. No lint suppressions are planned.

## Tasks

- [x] **Extract the shared synchronous process owner**
  - Add private `xtask/src/steps/process.rs` and register it in the inline
    `steps` module in `xtask/src/lib.rs`.
  - Move the generic `Process` owner out of `e2e_local/process.rs`; retain E2E
    server/collector policy and constants there.
  - Add consuming `wait` beside the existing readiness, shutdown, and stopped
    operations. Ensure `RunningProcess` is always dropped before its runtime,
    including failed start, normal wait/shutdown, and unwind/drop paths.
  - Migrate all E2E-local callers without changing their commands, raw tees,
    readiness probes, shutdown outcomes, or diagnostics.
  - Focused evidence: shared-owner wait/outcome and parent-plus-descendant drop
    cleanup fixtures; existing E2E-local process tests.

- [x] **Replace Nix stderr supervision with processkit**
  - Replace `MultiWriter`, `drain_build_stderr`, direct stderr extraction, and
    `Child::wait` with a Nix-local raw-tee sink plus the shared `Process`.
  - Configure processkit with inherited stdin, inherited stdout, and piped
    stderr. Preserve the existing Nix executable, argument order, out-link, and
    installable.
  - The sink synchronously writes each processkit-provided byte chunk to the
    diagnostic writer and then the primary writer. It always lets processkit
    finish draining, independently records write/flush failures in shared state,
    and performs no per-chunk allocation or copy beyond the two writes.
  - Map processkit start/wait errors and terminal outcomes through the spec's
    exact precedence. Preserve diagnostic warnings, reliable-path omission,
    excerpt-before-rescue ordering, rescue conditions, duration, and
    `StepResult` detail.
  - Focused evidence: success, non-zero and signalled children, inherited
    stdin/stdout, byte-exact ordered fanout, and each isolated sink failure.
    Cover primary-sink-plus-child failure and wait/teardown-plus-primary-sink
    failure to prove the complete precedence chain and skipped excerpt/rescue;
    cover start and wait/teardown errors separately. A cancellation/drop fixture
    retains already-captured output. Retain existing excerpt, rescue, directory,
    and warning-policy tests.

- [x] **Verify the integrated change**
  - Run focused xtask tests through
    `cargo xtask test-local --manifest-path xtask/Cargo.toml` while iterating.
  - Run `cargo xtask check --no-test` after the focused behavior is green.
  - Run `cargo xtask validate --no-e2e` because this changes the Nix build gate
    itself; preserve its diagnostics for review.
  - Inspect the final diff against the approved spec and repository Rust
    ownership/import rules before the commit gate.

## Key contracts

- `steps::process::Process` is private infrastructure, not a policy layer. It
  owns exactly one runtime and one `RunningProcess`; wait/shutdown consume the
  process handle before runtime teardown.
- Processkit alone owns child-tree containment, child stderr pumping, and
  wait/reap. Nix owns fanout destinations and all diagnostic/rescue decisions.
- A diagnostic sink failure cannot stop the primary sink or child drain. A
  primary sink failure cannot stop the diagnostic sink or child drain, but it
  wins over the eventual child outcome.
- Only a definitive failed child outcome may create an excerpt or run
  failed-outPath rescue. Processkit errors never do.

## Risk checks

- Test descendant cleanup with bounded polling and deterministic process IDs;
  never use pattern-based killing.
- Test arbitrary byte chunks, including non-UTF-8 and no trailing newline.
- Verify simultaneous failures do not reorder the documented precedence or leak
  sensitive underlying diagnostic errors.
- Verify E2E wrappers retain their current stopped-state behavior after the
  owner moves.
