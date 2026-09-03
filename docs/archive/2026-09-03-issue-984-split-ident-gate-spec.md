# Split identifier-gate internals

## Outcome

The shared identifier gate is decomposed into modules with independently named
responsibilities while preserving its caller-facing `Gate`, `Report`, and
`run_scan` interfaces, diagnostics, census behavior, and fail-closed safety
properties.

## Load-bearing decisions

- `xtask/src/steps/ident_gate.rs` becomes the directory module
  `xtask/src/steps/ident_gate/`.
- `mod.rs` owns the assembled gate documentation and contains only module
  declarations, explicit re-exports, documentation, and attributes, as required
  by ADR-0128.
- The implementation has four leaves:
  - `resolution.rs` owns owner aliases and structural type-membership
    resolution.
  - `traversal.rs` owns syntax traversal and the scan/mention data it produces.
  - `marker_policy.rs` owns marker classification, reasons, shared-site
    handling, and orphan detection.
  - `orchestration.rs` owns `Gate`, `Report`, source-scan orchestration, and
    reporting.
- The assembled `ident_gate` façade exports exactly `Gate`, `Report`, and
  `run_scan`. The user-approved narrow internal façade keeps all leaf-only items
  private or `pub(super)` at the least visibility their sibling callers require.
- There remains one shared traversal implementation for the raw-HTML-door and
  HTML-sink gates.
- Traversal remains the replaceable implementation seam; marker policy and
  orchestration do not become part of that seam.
- The assembled module documentation retains the structural-membership
  guarantees, fail-closed behavior, marker contract, and known blind spots
  required by ADR-0085 and ADR-0110.
- Tests move beside the responsibility or assembled contract they prove;
  `mod.rs` contains no tests.

## Acceptance

- Each implementation leaf has exactly the responsibility named above, with
  dependencies directed from resolution through traversal and marker policy to
  orchestration.
- Existing raw-HTML-door and HTML-sink callers compile without interface
  changes.
- Existing identifier-gate tests retain their behavioral coverage after moving
  to responsibility-local homes.
- Structural population membership, unresolved-membership handling, macro-token
  handling, test-range handling, marker placement/reasons, orphan detection,
  diagnostics, and census ordering remain observably unchanged.
- `ident_gate/mod.rs` satisfies ADR-0128 and exports exactly the unchanged
  caller-facing `Gate`, `Report`, and `run_scan` contract; leaf internals expose
  no wider path.
- The repository gate passes.

## Boundaries

- No identifier-gate algorithm, exemption policy, scan root, diagnostic wording,
  or ordering is changed.
- No new gate, marker form, or public crate API is introduced.
- The lower-level shared source discovery in `steps::scan` remains unchanged.
- Closed issue #894 is coordination context only; this work does not introduce
  ast-grep or another traversal engine.
