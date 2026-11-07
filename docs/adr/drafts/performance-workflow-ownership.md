# ADR-DRAFT: Just and Nix Own Performance Workflow Orchestration

- Status: proposed
- Date: 2026-11-06
- Issue: [#1894](https://github.com/jaunder-org/jaunder/issues/1894)

## Context

The current performance command in host-only `xtask` selects and invokes Nix
producers, retains their outputs, and applies the shared performance evidence
rules. Issue #1894 chose native Just parameters for the user-facing workflow and
a narrow ownership split: Nix selects producers, a small serial Just sequence
builds and retains them, and a host-only Rust command owns the
performance-domain decisions. This is a performance-specific clarification of
[ADR-0028](../0028-devtool-vs-xtask-boundary.md), not a general relocation of
host analysis.

## Decision

For performance runs, Just is the public workflow and invokes Nix-selected
producers serially, retaining each producer's complete evidence before starting
the next. This avoids overlapping measurements and preserves completed producer
evidence if a later build or producer fails. A host-only Rust performance
command in the `tools/performance` auxiliary package owns typed status/fragment
validation, compatibility and diagnostic checks, run assembly, advisory
comparison, trusted baseline-import admission, and atomic publication; it never
selects or schedules producers. Nix derivations never invoke xtask or this host
command. This decision leaves the separate workspaces and auxiliary test
requirements in [ADR-0141](../0141-cargo-workspace-execution-boundaries.md)
unchanged.

## Consequences

Native Just parameters preserve the existing profile, backend, browser,
workload, and count capabilities and their canonicality rules without a
replacement CLI parser. Nix remains the pinned producer environment; Just
retains the explicit serial build-and-retain sequence; Rust retains only the
domain policy that cannot safely be replaced by artifact-download or shell
plumbing. The workflow, callers, and current contributor/architecture
documentation must migrate together when this proposed design is approved and
implemented. Update `CONTRIBUTING.md` with the new command interface during
implementation. This decision adds no ubiquitous-language terms, so `CONTEXT.md`
remains unchanged. Until then, the existing `cargo xtask perf` flow remains the
current runtime behavior.
