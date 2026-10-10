# Nix-cached host tooling implementation outline (#1734)

> Execute with jaunder-iterate; jaunder-dispatch is available for a bounded
> delegated task. This outline exists because tooling source identity,
> qualification admission and runtime harness ownership form a durable
> architecture boundary. The approved issue-1734 spec remains authoritative.

## Scope

In: the standalone tool package, current-checkout launcher/admission, Nix-owned
qualification inputs and evidence, and their regression/consumer updates.

Out: issue-query optimization, application refactoring, reduced gates, custom
fingerprints/identity ledgers, and a separate Cargo qualification lane.

## Task outline

- [x] Task 1: Deliver the independently cached `.#xtask` executable.
  - Contract: Nix declares the complete standalone workspace/local dependency
    closure and native runtime dependencies, with deterministic placeholders
    where Cargo only needs excluded workspace manifests. No Git build stamp may
    couple the output to unrelated commits. The package compiles xtask but does
    not execute its orchestrator or replace host self-tests.
  - Verification: real package build and milestone payload comparison; matched
    derivation/output probes for docs, server implementation, tooling, shared
    inputs, lockfiles, build configuration and embedded resources; preserve
    application coverage/E2E source/dependency exclusions. Evaluate supported
    platform boundaries and report unexecuted native platforms explicitly.
  - Acceptance coverage: 1–4 and 8.
- [ ] Task 2: Use the package safely from the active checkout.
  - Depends on Task 1. Contract: normal invocations select this checkout's flake
    and preserve arguments, exit codes, result sidecars and completion sentinel.
    Local target/root paths come from runtime checkout discovery. Missing or
    untracked build inputs are an actionable refusal, not stale-tool fallback;
    input admission remains owned by the Nix declaration rather than a second
    hand-maintained source inventory. Focused Cargo tests remain available.
  - Verification: relocated-executable/root tests, local resume-state proof,
    launcher argument/exit propagation and missing-input negative cases; warm
    command/build timing with explicit no-compilation evidence. Run focused
    local browser proof if checkout/path changes affect e2e-local execution.
  - Acceptance coverage: 1–3, 7 and launcher portions of 9.
- [ ] Task 3: Admit and execute the qualification graph through Nix.
  - Depends on Tasks 1–2. Contract: one Nix-selected harness output binds the
    tool, browser flows/configuration/dependencies and test-support using
    existing package owners. The qualification source pin selects that graph;
    actual executables/runtime inputs must match its standard store outputs
    before leases, VM mutation or publication. Runtime artifacts stay writable
    only in invocation-owned state, not store outputs.
  - Replace both ordinary discovery/acceptance and styling tool-commit checks.
    Git keeps its clean source/product and drift responsibilities. Preserve
    fixture semantics, manifest authorities, backend coverage, lifecycle safety
    and sanitization. Adapt existing evidence/schema conventions to standard Nix
    identities without a custom compatibility manifest or ledger.
  - Verification: host admission proofs for cross-commit identical-output
    acceptance, wrong/missing outputs, mixed runtime inputs and source drift;
    unchanged publication/canary failure-closed proofs. Execute focused browser/
    VM integration where immutable browser/seed wiring changes real consumers,
    covering ordinary and styling paths rather than declaring parity from unit
    fixtures alone. Do not substitute the exploration's existing rejection test
    for positive Nix-owned qualification proof.
  - Acceptance coverage: 5–7.
- [ ] Task 4: Reconcile consumers and certify the delivered boundary.
  - Depends on Tasks 1–3. Contract: operating docs, skills, hooks/workflows and
    normal launcher consumers describe one Nix-owned tooling/provenance path.
    The ADR/projection states delivered behavior rather than a proposed future
    implementation. Existing host and hermetic gates retain their populations.
  - Verification: focused consumer/parity and documentation checks, source
    invalidation probe reconciliation, host tests and affected runtime proofs;
    use broad diagnostics only where they answer an integration question and the
    normal hook/ship verification ladder for commit/PR boundaries. Recheck
    supported platform evaluation and disclose unavailable native execution.
  - Acceptance coverage: 4 and 7–9, plus final conformance across 1–9.

## Risk checks

- Never introduce a dependency from application coverage/E2E producers to the
  host xtask executable or invoke the orchestrator inside a derivation.
- Standalone installation must not make Crane query the root product workspace;
  preserve xtask's own lock/vendor resolution and native runtime closure.
- A Nix output path is not a fabricated current Git commit. Qualification must
  compare selected outputs, not retain Git equality under a different name.
- Do not execute live browser imports or ambient test-support outside the
  admitted graph, nor publish mixed/drifted source evidence. Keep all admission
  before lifecycle effects and retain source revalidation around execution.
- The source closure must include embedded application CSS/theme data, schema
  and lane catalog, and must not silently drop untracked/new tooling inputs.
- Keep host xtask/tools self-tests and existing required backend/browser/gate
  authorities. No lint/test/coverage suppression is part of this work.
