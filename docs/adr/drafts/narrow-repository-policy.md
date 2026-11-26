# ADR-DRAFT: Narrow repository-policy to source contracts and documentation checks

- Status: proposed
- Date: 2026-11-25
- Issue: [#1975](https://github.com/jaunder-org/jaunder/issues/1975)

## Context

[ADR-0246](../0246-retire-xtask-devtool-endpoints.md) made
`tools/repository-policy` the home of the fixed source contracts and
documentation checks. It also made the package the owner of three things those
checks do not use:

- the shared Git and checkout-admission library;
- the informational repository census (`dev census`);
- the Org timezone extractor (`dev org-timezone-manifest`).

host-proof and coverage reached the Git and checkout helpers only through this
package. Those two tools already share `tools/host-tool`, the report contract of
[ADR-0247](../0247-host-tool-json-report-contract.md).

The census was built for the codebase-audit method (#1222) and gated nothing. By
#1975 its report ordered candidates alphabetically or by hash rather than by
signal, and both semantic cells reported "unavailable". Its churn cell emitted
over 600,000 candidates. It had no caller other than that document.

The timezone extractor listed post-ID→timezone pairs from original Org files
after #1921's UTC backfill. Nothing consumed its output. It was the policy
package's only dependency on the product crate `common`.

## Decision

- **Git and checkout admission belong to `tools/host-tool`.** host-tool is the
  shared contract of the host tools: their JSON report, and how they select and
  admit the caller's checkout and repair `core.hooksPath`. coverage and
  repository-policy import these helpers from host-tool. Its one executable,
  `admit-inputs` (`packages.host-tool`, #2102), is that admission as a command
  for `dev test-local` and `dev e2e-local`.
- **One checkout entry point.** Every host tool calls host-tool's
  `checkout::prepare` with its name and, for repository-policy, the source
  predicate compiled into that package (`JAUNDER_REPOSITORY_POLICY_SOURCE`). No
  tool keeps its own checkout wrapper.
- **host-tool's sources stay out of the shared `toolsSrc`.** No in-derivation
  producer links host-tool, so an edit to the helpers rebuilds only host-run
  tool packages.
- **The census and the Org timezone extractor are retired** with their `dev`
  commands. The human confirmed both capability losses, as ADR-0246 requires.
  The codebase-audit method remains, using ordinary tools for its census step.
- **repository-policy holds only** the source contracts, the documentation and
  ADR checks, `scoped-leptosfmt`, and the shared source-policy contracts (gate
  markers, server-function inventory). Its Cargo closure no longer includes
  `common` or `macros`.

This narrows three earlier decisions:

- ADR-0246's repository-policy clause, which put Org-timezone extraction and the
  checkout-admission library in the package;
- ADR-0247's statement that host-tool exists "so repository-policy keeps owning
  no reports or process execution". host-tool now also runs `git` and `nix eval`
  for checkout admission. repository-policy depends on it but still emits no
  host-tool report.
- ADR-0258's binary list for the `repository-policy` package, which no longer
  includes `org-timezone-manifest`.

## Consequences

- coverage still depends on repository-policy, for the gate markers. It still
  compiles its syntax-tree dependencies. The Git move gives the helpers a
  clearer owner but removes no dependency edge.
- repository-policy now also links host-tool's `clap` and `rustix`.
- A later audit that wants a machine-generated census must build a new tool
  justified by a consumer, not revive this one.
