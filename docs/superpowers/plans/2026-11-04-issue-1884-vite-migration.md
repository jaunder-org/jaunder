# Vite CSR migration implementation outline

> Execute with `dev-cycle-iterate`. Outline needed because this migration
> changes the CSR build inventory and ADR-0175 architecture across host/Nix
> production and multiple consumers.

## Execution summary

After approval, integrate the Vite 8.3.5 / `vite-plugin-compression2` 2.5.4
pipeline as the single bundle producer, then migrate consumers and remove
superseded machinery. First establish the shared locked host/Nix output contract
and preserve classic early-fetch/diagnostic behavior; the isolated probe did not
prove fetch ordering, HTTP serving, or consumer migration. Use one serialized
writer for the coupled producer, consumer, and ADR changes.

## Task outline

- [ ] Task 1: Establish the production Vite pipeline and shell/projector
      contract.
  - Contract: exact package/tool pins and lockfile; release wasm-bindgen output
    followed by existing `wasm-opt -Oz` flags; Vite owns HTML, graph and
    ordinary content-sensitive filenames/manifest. Host and Nix invoke the same
    production bundle pipeline. Use the current supported Just host
    entry/interface after PR #1880 lands; do not restore the deleted
    `xtask build-csr` module or add a replacement Rust CLI. Coordinate any
    required shared-recipe interface with the #1872 owner; do not edit that PR's
    Just work.
  - Verification: host and Nix builds consume the same locked inputs; unchanged
    inputs retain URLs and mutated WASM/runtime bytes invalidate URLs without
    custom hashing; assert URLs in static shell and public-projector output.
    Browser checks cover auth prepaint, stylesheet URL, classic WASM fetch
    before module entry, one fetch, no WASM preload, `initMeasured` API
    timing/module-shape diagnostics, and existing boot-diagnostic
    reconciliation.
- [ ] Task 2: Migrate build inventory consumers and remove superseded producer
      machinery.
  - Contract: Vite manifest is build-only and excluded from served/embedded
    output. Staging remains safe; server build, audit, qualification and
    coverage read standard manifest/output paths directly, without recreating
    role/representation schema. Diagnostic coverage gets the standard entry
    needed for its exports; preserve the separate coverage-artifact
    reconciliation format. Keep system-theme/style URL and raw optimized-WASM
    measurement responsibilities. Remove obsolete custom parser/graph rewriter,
    hash/schema implementation, crate/workspace references, dependencies, tests,
    and Nix source-filter exclusions where no longer used.
  - Verification: consumer tests cover manifest lookup, stage/build, audit,
    qualification and diagnostic coverage exports; ensure manifest is not served
    and no consumer relies on the removed CSR role schema.
- [ ] Task 3: Preserve embedded serving and reconcile architecture.
  - Contract: retain `rust-embed` and the existing `site.rs` handler; use Vite's
    output paths/manifest for asset membership. Preserve identity/gzip/Brotli
    selection, `q=0`, decoded equivalence, logical MIME, `Vary`, immutable
    cache, representation ETags and matching 304. Do not replace with external
    `ServeDir` or copy serving logic. Reconcile ADR-0175 via the approved
    numberless draft; keep ADR-0106 raw post-`-Oz` budget and ADR-0121
    no-preload invariant. In the initial docs-only commit before implementation,
    project current implementation facts accurately and label Vite as the
    approved, not-yet-implemented direction, citing the draft path. Update
    `docs/ARCHITECTURE.md` to verified implemented facts only after migration
    evidence exists; do not claim acceptance or that the old
    schema/implementation is gone prematurely. Never hand-assign an ADR number
    or edit generated `docs/README.md`.
  - Verification: exercise generated sidecars through the existing serving seam
    for all listed response behaviors; run the relevant boot-diagnostic
    reconciliation. Verify the initial projection distinguishes current state
    from approved direction; verify the later projection states only
    demonstrated implementation facts.
- [ ] Task 4: Measure net maintenance change and certify the integrated
      migration.
  - Contract: count before/after production, consumers, configuration/adapters,
    and tests separately. The roughly 670 producer lines are deletion candidates
    only, not measured savings or a quota; include retained `initMeasured`
    extraction and every adapter/test. No graph/hash/role-schema
    reimplementation or wrapper substitution.
  - Verification: report measured removals/additions and unresolved behavior;
    run focused proofs and current supported project gates for the final tree,
    with pre-commit on each commit and pre-push/ship validation at their
    lifecycle boundaries. Do not expand to unrelated platform/e2e matrices
    absent a stated requirement.

## Integration and delivery gates

Before implementation, obtain outline approval, then commit the approved
migration spec, ADR draft with its architecture projection, and outline through
normal hooks. Rebase the issue branch onto current `origin/main` before
integration updates; inspect and preserve unrelated staged/dirty/untracked
state, use no merge commits, and confirm the intended tree after each
rebase/hook. PR #1880 is currently OPEN and unmerged (read-only
`gh pr view 1880`, observed 2026-11-04); do not poll or depend on its Just edits
before they land. Recheck status at the implementation boundary, then adapt only
the host invocation interface needed by the landed recipe, coordinating that
overlap with its owner.

## Risk checks

- The probe changed fetch scheduling: production must retain the classic early
  fetch before module execution; one-fetch evidence alone is insufficient.
- Probe evidence does not prove full consumer migration, diagnostic
  reconciliation, projector URL stability/invalidation, or HTTP negotiation;
  test each at its stated seam.
- Vite hashes are content-sensitive but not a cryptographic integrity promise;
  do not add a custom hash path to restore ADR-0175's full-SHA naming.
- Keep Vite manifest out of runtime serving and preserve raw-WASM budget, auth
  prepaint, CSS marker, and existing HTTP cache/validator behavior.
