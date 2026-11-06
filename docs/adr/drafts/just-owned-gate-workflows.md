# ADR-DRAFT: Just Owns Developer Gate Workflows

- Status: proposed
- Date: 2026-11-05
- Issue: [#1889](https://github.com/jaunder-org/jaunder/issues/1889)

## Context

Before #1889, the gate workflow was spread across Rust catalogs and dispatch in
`xtask/src/gate.rs`, `xtask/src/dispatch.rs`, `xtask/src/cli.rs`, and
`xtask/src/steps/static_checks.rs`, Git-hook callers, CI lane callers, and
structured per-step result plumbing. `Justfile` is already the repository's Just
entrypoint and `pkgs.just` is already pinned in the development shell. Keeping
workflow ownership in Rust while adding a second recipe layer would preserve
duplicated sequencing and reporting instead of simplifying the system.

Existing check functions include substantive parsers/censuses, small textual
policies and consumer/execution glue; their current Rust implementation alone
does not establish language necessity. Precommit also has proven Git/index
reconciliation and failure-path obligations. Replacing these with a mirrored
shell implementation or deleting their real guarantees would lose behavior.
Separately, #1887 identifies three Prettier definitions and an exclusive
docs-only `docsDevtoolBin` source/vendor/build path that can be removed while
preserving host and hermetic verdicts.

## Decision

The root `Justfile` owns the default one-command developer, hook, local
validation, and CI-lane workflow definitions. Just composes direct pinned tools
and callable checks/adapters. Existing algorithms and residual
function-signature adapters remain callable through a minimal single-leaf
boundary; this does not claim every entry requires Rust or permanently belongs
in xtask. Ordinary clean admission is direct Just/Git, while raw/index safety
stays specialized. The explicit server-fn-flow bridge awaits #1890's actual E2E
migration without retiring its substantive verifier. This boundary does not
select a gate population, sequence a workflow, or produce a gate-wide report.
Nix provides pinned Just and tool runtimes as needed; it does not define a
parallel workflow framework or invoke host-only `xtask` from derivations.

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

The repository cutover retires gate-wide result envelopes and the old
`cargo xtask check/precommit/prepush/validate/ci-validate` aliases. Hooks and CI
call the explicit Just recipes: `check-no-test`, `check`, `precommit`,
`prepush`, `validate-no-e2e`, `validate`, and
`ci-validate-{host,hermetic,test-checks,coverage}`. Just options are not
forwarded; `just check --no-test` and `just validate --no-e2e` are not
interfaces.

Manager confirmed the coordinated external owner, exact mapping and post-landing
handoff, not pre-deployed guidance. The human rollout correction is to “feed in
the changes as the PRs land, to stay consistent.” External guidance therefore
follows each confirmed landing incrementally; older active checkouts retain
assignment-specific old-interface instructions. This change does not edit or
activate external guidance, probe recipes, or add a fallback/registry. There is
no permanent compatibility forwarding layer.

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

## Approval provenance

The human approved design-v2 (`issue1889-spec-v2`, Manager relay
`1d21c0df-6e73-4c89-a40e-ddf5b67d92ba`, “Approve revised design”) and the
risk-triggered outline-v1 (`issue1889-outline-v1`, relay
`38808dc8-5ac2-422f-a9fe-3b3843758f43`, “Approve outline and begin”). They
approve Just-first ownership, alias and gate-wide JSON retirement, local
fail-fast behavior, retained Git/index safety and real producer/CI verdicts. The
completed planning documents are removed before PR preparation; their branch
history retains the full approved text. The later Manager readiness confirmation
applies the human incremental-landings correction above.

## Consequences

- One Justfile owns workflow composition rather than parallel Rust, CI, and Nix
  catalogs. Rust remains the implementation home for the checks that require it;
  the leaf boundary adds invocation, not a second algorithm or workflow engine.
- Repository hook, developer and CI callers cut over with alias deletion.
  Manager owns exact post-landing external guidance updates and older-checkout
  instructions; pre-deployment is not a circular prerequisite to landing.
- Gate recipes have the approved fail-fast behavior: local check/validate runs
  stop at the first blocking leaf instead of collecting later diagnostics.
  Mandatory precommit reconciliation still runs after failure; producer evidence
  and independent CI job results remain authoritative.
- The precommit safety boundary remains more specialized than ordinary Just
  dependencies because reconciliation and post-request verification must run
  after failure.
- The docs-only Nix source/vendor/build pipeline disappears, while all other
  `devtool` definitions and validation leaf populations remain in place.
