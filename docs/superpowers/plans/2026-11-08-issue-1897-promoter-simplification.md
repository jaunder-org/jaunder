# #1897 Promoter Simplification Implementation Outline

> Execute with dev-cycle-iterate only after independent outline review and human
> approval. Needed because package/admission boundaries and public Rust command
> retirement have ordering and compatibility risks.

> Approval: human response `89ec8287-4f38-479c-bcbd-24cf540bfc7c` /
> `issue-1897-simplification-outline-v2` approves this outline and
> implementation for the exact outline reviewed at SHA-256
> `3bfd75b7641ae666155bc2fc9f9d4c8177b52aa24415770dc3a3d1415118758c`. Scope is
> this one cohesive PR #1917 and its stated rollout acceptance; the exact-head
> PR merge gate and post-merge trusted-main observation remain separate gates.
> This approval does not authorize scope beyond this outline.

## Execution summary

Implement the approved revised design in one cohesive PR #1917: simplify SDK and
package/admission paths, migrate actual integrated callers, and retire the
obsolete Rust mutation commands together while preserving application policy and
safety. One exact-head human gate precedes merge; observe trusted-main operation
after merge as rollout acceptance before completion. The approved contract is
[the revised spec](../specs/2026-11-08-issue-1897-promoter-simplification.md);
this outline does not authorize implementation.

## Task outline

- [ ] Task 1: Deliver the cohesive package and caller migration in PR #1917
  - Contract: pinned PyGithub 2.8.1 supplies SDK mechanics; custom code retains
    consumed-evidence selection, correlation, refusals, outcomes, mutation
    safety, and the authoritative Nix source-membership predicate. Invoke the
    selected package directly. In this same PR, migrate actual integrated
    callers, retire public `cargo xtask adr promote` and
    `cargo xtask adr sync-readme`, and move documented recovery to package
    `sync-readme`; preserve independent Rust read-only gates and do not assume
    or modify cross-project consumers. Tests exercise application-owned behavior
    and trust documented dependencies; no library probes or
    per-field/provider/retry-engine matrices. Remove redundant route tests and
    use one package smoke for our integration.
  - Verification: focused application-boundary behavior tests, one
    selected-package smoke, and ordinary PR checks. Confirm preserved refusal,
    byte-edit/staging, outcome, read-only-gate, caller migration, and complete
    policy/mutation safety contracts.
- [ ] Task 2: Preserve the exact-head merge gate
  - Contract: the cohesive PR has one human merge gate for its exact head;
    promotion remains acceptance and the merge queue alone writes to `main`.
  - Verification: record ordinary required checks and separate human approval
    for the exact PR head before merge. This is a future gate, not work
    authorized by this outline's preparation.
- [ ] Task 3: Observe trusted-main operation as rollout acceptance
  - Contract: observation occurs after merge and before completion; no dormant
    Rust fallback remains. If operation fails, diagnose and use scoped repair or
    revert.
  - Verification: record trusted-main operation observation through the
    established lifecycle; preserve failure evidence and resolve any failure
    with scoped repair or revert before completion.

## Risk checks

- Preserve trusted App scope, immutable attempts, exact
  event/PR/head/ref/group/check correlation, fail-closed ambiguity, leased
  recovery, mutation postconditions, and promotion-as-acceptance.
- Preserve complete byte-range edits and staging policy; feature authors do not
  promote.
- Keep independent read-only gates, one exact-head PR merge gate, and post-merge
  trusted-main observation before completion; failures require diagnosis and
  scoped repair or revert, never a dormant fallback.
- No per-field, provider, or retry-engine matrices; no library probes or fake
  route framework; one package smoke proves our integration only.
- No implementation, build/test/probe, push/watch, privilege, merge, or cleanup
  is authorized before the separate outline approval and applicable later gates.
