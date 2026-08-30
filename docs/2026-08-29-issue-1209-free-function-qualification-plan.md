# Free-function Path Qualification Implementation Outline

> Execute with dev-cycle-iterate. This outline exists because the complete
> production cleanup spans eight source roots and needs explicit multi-agent
> ownership and exception contracts.

## Scope

In:

- Direct nonlocal free-function imports and repeated long free-function paths in
  `client/src`, `common/src`, `csr/src`, `host/src`, `macros/src`, `server/src`,
  `storage/src`, and `web/src`.
- Review evidence that proves the complete candidate population was
  dispositioned.

Out:

- Tests, `cfg(test)` modules, `test-support`, `xtask`, `tools`, generated
  output, lint policy, compatibility layers, public API changes, and unrelated
  imports.

## Task outline

- [x] Task 1: Capture the pre-rewrite candidate baseline
  - Contract: Analyze the approved base revision across all eight roots before
    source edits. Persist the exact syntax/name-aware procedure, candidate
    paths, exclusions, and initial dispositions in a controller-owned local
    artifact that Tasks 2–5 can consume and the pull request can summarize.
  - Verification: Every reported baseline count is reproducible against the base
    revision, and each candidate names one owning source-root slice.
- [x] Task 2: Normalize foundational and target-gated runtime crates
  - Contract: Own only `client/src`, `common/src`, `csr/src`, `host/src`, and
    `macros/src`. Direct nonlocal free functions use an imported owner module or
    `super::function()`; collision-required item aliases remain unchanged. Two
    or more long calls from one owner module in a file use a module import.
  - Verification: Controller syntax-aware diff inspection confirms every changed
    call resolves to the same free function and only path/import shape changed.
- [x] Task 3: Normalize server free-function ownership
  - Contract: Own only `server/src`. Preserve AtomPub/feed vertical façades and
    generated server-function names while qualifying their free-function calls;
    two or more long calls from one owner module in a file use a module import.
  - Verification: Controller syntax-aware diff inspection confirms every changed
    callee resolves to the same function and no protocol/runtime behavior
    changed.
- [x] Task 4: Normalize storage and web free-function ownership
  - Contract: Own only `storage/src` and `web/src`. Preserve backend symmetry,
    Leptos generated names, documented collision aliases, and target gates;
    apply the same direct-import and repeated-long-path rules as Tasks 2 and 3.
  - Verification: Controller syntax-aware diff inspection confirms backend
    symmetry, web vertical ownership, identical callees, and unchanged behavior.
- [x] Task 5: Prove integrated conformance and repair residual candidates
  - Contract: After Tasks 2–4 are integrated, rerun the Task 1 procedure over
    all eight roots. Compare it with the base-revision artifact; record
    disposition totals and every intentional exception for the pull request.
    Repair every confirmed residual in its owning root; introduce no lint or
    checked-in inventory.
  - Verification: The comparison accounts for every baseline candidate, reports
    zero confirmed residual violations, and makes every exception reviewable.
    Only after the parallel slices are integrated, the controller runs the
    repository `cargo xtask check` feedback command and the commit hook's
    `cargo xtask precommit`.

## Risk checks

- Parallel implementers have disjoint source-root ownership; the outline/spec
  files and cross-root public interfaces remain controller-owned.
- A path-only rewrite must resolve to the identical function and preserve
  generic arguments, target gates, visibility, and evaluation order.
- Associated functions, methods, enum variants, macros, types, traits, generated
  names, and collision-required aliases never enter the free-function set.
- `super::function()` preserves a deliberate parent façade; it does not flatten
  or bypass module ownership.
- Repetition is measured within one source file and one owner module, not across
  unrelated files.
- Final evidence covers every specified source root and distinguishes excluded
  candidates from clean results.
