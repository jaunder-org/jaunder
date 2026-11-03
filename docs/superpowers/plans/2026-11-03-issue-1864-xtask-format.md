# #1864 standalone xtask-format Implementation Outline

> Execute with jaunder-iterate and jaunder-dispatch. Outline needed because this
> adds a new Nix source boundary and hermetic consumer while preserving the
> host-only xtask execution boundary.

## Execution summary

Implement the already human-approved spec at
`docs/superpowers/specs/2026-11-03-issue-1864-xtask-format.md`: a separate
terminal Nix Verify leaf runs only pinned Cargo/rustfmt with the existing
command `cargo fmt --manifest-path xtask/Cargo.toml --check`. First prove the
source filter can provide Cargo's metadata/target view without admitting
dependency implementation sources or building devtool; then wire the leaf to
both required consumers and update the maintained boundary documentation. Do not
implement the unapproved Rust-format or Clippy proposals.

## Task outline

- [x] Task 1: Add the raw xtask-format Nix leaf and bounded source closure.
  - Contract: In `nix/checks.nix`, define a separately named terminal verdict
    that runs the exact Cargo command above: no `--all`, no `cargo xtask`, no
    compiled devtool, and no product/tools offline Cargo homes. Admit xtask's
    Cargo target sources, `xtask/Cargo.toml` and `xtask/Cargo.lock`, plus the
    applicable root rust-toolchain, rustfmt and `.cargo` configuration. Locked
    metadata currently reports xtask lib/bin, six integration-test targets, and
    build script; preserve the command's actual selected xtask targets. Cargo
    also resolves path-dependency manifests (current metadata closure includes
    local packages such as common, host, coverage, csr_bundle, doctests,
    performance and macros). Supply only the target placeholders Cargo needs for
    those dependencies' metadata; exclude their implementation sources. Resolve
    any additional manifest/placeholder need from locked metadata and keep the
    filter bounded. Reuse existing source-filter/placeholder machinery where it
    fits; no generic framework.
  - Verification: Confirm the raw Cargo invocation and target population against
    `.xtask/evidence/formatter-cargo-populations.json` and the fresh locked
    metadata evidence; establish that the leaf works from the filtered source
    without devtool, xtask execution, path-dependency implementation sources, or
    unrelated workspace target bodies. Show relevant xtask source/config/tool
    inputs affect identity and a valid local-store reuse case; demonstrate
    unrelated source retention only where the full derivation closure allows.
    Use a disposable copied fixture for a deliberate formatting violation,
    verify the leaf fails, restore/discard it and confirm the working source
    tree is unchanged.

- [x] Task 2: Require the leaf in both consumers and reconcile maintained
      documentation.
  - Contract: Add the leaf exactly once to `nix flake check` exposure in the
    checks set and to the hermetic `validate --no-e2e` static-check consumer in
    `xtask/src/steps/nix.rs`; retain the existing host `xtask-fmt` step and all
    host Fix/index/prepush behavior. Update `CONTRIBUTING.md`,
    `docs/ARCHITECTURE.md`, and ADR-0146's blanket xtask-source exclusion
    narrowly to record this Cargo/rustfmt-only exception. Keep ADR-0028's
    prohibition on executing xtask inside Nix. Do not create a new ADR by
    default, alter hooks/CI routing, or introduce an aggregate API.
  - Verification: Show the named leaf occurs once in each required consumer;
    prove a deliberate formatting failure reaches the validation verdict, while
    a clean restored fixture passes. Check relevant source/config/tool
    invalidation, valid local reuse, unrelated-input retention only where the
    full closure permits, and ordinary terminal exit propagation. Report
    derivation identity, Nix request duration/reuse, and existing
    current-execution timing semantics separately; do not add a timing protocol
    or infer execution from cached logs. Keep focused proof artifacts ignored
    and verify the checkout is restored.

## Risk checks

- Cargo formatting without `--all` is the accepted command; do not accidentally
  format local dependency implementation sources. Metadata manifests and target
  placeholders are necessary only to make Cargo's package/target resolution
  complete, not permission to admit those sources.
- The Nix leaf must never invoke xtask; ADR-0028's host-only execution boundary
  remains binding. ADR-0146 needs an explicit narrow source-admission
  reconciliation, not a broad xtask-source inclusion.
- Keep the existing host `xtask-fmt` check, `static-code` population, and all
  unrelated work unchanged. The Clippy and Rust-format proposals are unapproved
  and nonblocking. #1872's build-csr design/Just pin is human-approved (request
  `e0d243bd-007d-40cc-b670-5e6e6fe35820`, packet `issue1859-build-csr-spec-v1`,
  “Approve design and Just pin”); its outline is pending and is not a
  dependency.
- No implementation starts until this outline is approved. The approved spec
  remains authoritative; material source/consumer scope changes return to design
  approval. Implementation checkout admission belongs to the Manager after
  approval; do not create a speculative checkout now. Independent work need not
  wait for TS/ERT completion or unrelated shared-file changes; coordinate exact
  conflicting units and rebase actual conflicts.

## Delivery and acceptance boundary

Lead reviews and accepts the final integrated diff and focused proof. Follow
applicable repository gates and hook-backed staging/commit boundaries; inspect
and resolve any hook mutations before delivery. Commit the approved spec,
outline, implementation, and maintained documentation, then use the normal PR/CI
handoff. Keep proof artifacts ignored and include their exact references, the
delivered revision, and a fresh checkout preservation inventory in the handoff.
This does not require blanket full-E2E execution or worker-run broad gates.

**Outline approval record:** Separate human approval request
`881f60c8-5c6d-471c-85f9-906093888121` / packet
`issue1864-xtask-format-outline-v1`; exact answer “Approve outline and begin.”
Approval is explicitly for implementation in the isolated assigned checkout
`/home/mdorman/src/jaunder/issue-1864-xtask-format`; merge approval remains
separate. This provenance records the separate outline approval and does not
change the design approval recorded in the spec.
