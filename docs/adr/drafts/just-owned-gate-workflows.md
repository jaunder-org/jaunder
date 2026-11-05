# ADR-DRAFT: Just Owns Developer Gate Workflows

- Status: proposed
- Date: 2026-11-05
- Issue: [#1889](https://github.com/jaunder-org/jaunder/issues/1889)

## Context

The current gate workflow is spread across Rust catalogs and dispatch in
`xtask/src/gate.rs`, `xtask/src/dispatch.rs`, `xtask/src/cli.rs`, and
`xtask/src/steps/static_checks.rs`, Git-hook callers, CI lane callers, and
structured per-step result plumbing. `Justfile` is already the repository's Just
entrypoint and `pkgs.just` is already pinned in the development shell. Keeping
workflow ownership in Rust while adding a second recipe layer would preserve
duplicated sequencing and reporting instead of simplifying the system.

Several check algorithms are Rust-only functions with no command-line leaf, and
precommit has proven Git/index reconciliation and failure-path obligations.
Moving either into a mirrored shell implementation or deleting them would lose
behavior. Separately, #1887 identifies three Prettier definitions and an
exclusive docs-only `docsDevtoolBin` source/vendor/build path that can be
removed while preserving host and hermetic verdicts.

## Decision

The root `Justfile` owns the default one-command developer, hook, local
validation, and CI-lane workflow definitions. Just composes direct pinned tools
and callable leaf checks; existing Rust check algorithms remain Rust-owned and
are exposed only through a minimal single-leaf invocation boundary where Just
cannot call them directly. That boundary does not select a gate population,
sequence a workflow, or produce a gate-wide report. Nix provides pinned Just and
tool runtimes as needed; it does not define a parallel workflow framework or
invoke host-only `xtask` from derivations.

A focused single-process precommit owner retains only necessary Git/index
classification, snapshots, safe staging reconciliation, and docs admission /
post-request checks. Uncertain status takes the broad host route. It reconciles
exactly once even after host failure, preserves that failure, and admits the
x86_64-linux `static-docs` request only after successful staged-Markdown-only
host checks and reconciliation. The post-request state check still runs after a
failed request. Existing untracked and delete/rename state is never staged;
unsafe reconciliation fails closed.

Gate recipes fail fast on the first blocking leaf; no later local diagnostics
run after that failure. This narrowly supersedes ADR-0029's exhaustive
continuation policy for `check` and `validate`; their check populations and
success-path verdicts remain unchanged. Precommit still performs mandatory
failure-path Git/index reconciliation exactly once, preserves the original gate
failure, and performs the docs post-request state check even when the request
fails. Producer evidence remains required, and independent CI job results and
aggregates remain authoritative. CI sidecars are diagnostic only.

Retire gate-wide result envelopes and old `cargo xtask` workflow aliases when
hooks, repository documentation, CI, and the Manager-owned external agent
selector/policy have reached the coordinated cutover checkpoint. Do not edit
external policy in this change. There is no permanent compatibility forwarding
layer.

As the approved #1887 component, the three Prettier command definitions for
Markdown, `end2end`, and CSS move to the shared Just definitions; the exclusive
docs-only `docsDevtoolBin` build/source pipeline and consumer are removed. Other
`devtool` command definitions, including compiling checks, ast-grep, and Leptos
formatting, remain with `devtool`.

The approved CI lane topology and required aggregates remain. E2E retains
`e2e-lanes.json` authority and its producer/reconciliation ownership; use its
approved full Just entrypoint only after #1890 lands. No sibling implementation
is imported or taken over by this decision.

This draft supersedes only the gate workflow ownership described by ADR-0028
(host gate orchestration/result role), ADR-0029 (Rust ownership of hook-routed
gate workflows, not its Git/index and verify guarantees), and ADR-0203 (the
shared local/CI validation composition/catalog, not its CI lanes or E2E
contracts). For ADR-0052 and ADR-0178, only the three Prettier command ownership
clauses and #1887's docs-only build path change. ADR-0146 and ADR-0161 remain
unchanged; all other `devtool` leaf ownership and hermetic boundaries remain
unchanged. ADR-0110's structural population/fail-closed rule continues to apply.

## Consequences

- One Justfile owns workflow composition rather than parallel Rust, CI, and Nix
  catalogs. Rust remains the implementation home for the checks that require it;
  the leaf boundary adds invocation, not a second algorithm or workflow engine.
- Hook, developer, CI, and external agent-selector callers must cut over
  together. External policy/selector readiness is a Manager-owned checkpoint,
  not a reason to retain indefinite aliases.
- Gate recipes have the approved fail-fast behavior: local check/validate runs
  stop at the first blocking leaf instead of collecting later diagnostics.
  Mandatory precommit reconciliation still runs after failure; producer evidence
  and independent CI job results remain authoritative.
- The precommit safety boundary remains more specialized than ordinary Just
  dependencies because reconciliation and post-request verification must run
  after failure.
- The docs-only Nix source/vendor/build pipeline disappears, while all other
  `devtool` definitions and validation leaf populations remain in place.
