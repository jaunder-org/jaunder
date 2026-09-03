# Split identifier-gate internals implementation outline

> Execute with `jaunder-iterate` and `jaunder-dispatch`. This outline exists
> because parallel leaf extraction requires stable module contracts before
> assembly.

## Scope

In:

- Decompose the existing identifier-gate implementation and tests into the four
  spec-defined leaves.
- Assemble the directory module with the caller-facing `Gate`, `Report`, and
  `run_scan` paths unchanged, least visibility for leaf-only items, and
  unchanged documentation.
- Verify the focused xtask behavior and repository gate.

Out:

- Changes to gate algorithms, marker policy, scan roots, diagnostics, ordering,
  or consumers.
- Changes to `steps::scan` or introduction of another traversal engine.

## Task outline

### Parallel extraction protocol

- The four extraction tasks run from the same committed `ident_gate.rs` snapshot
  and create only their owned leaf file.
- Extraction agents do not edit or delete `ident_gate.rs`, create
  `ident_gate/mod.rs`, or modify another leaf.
- The assembly task alone removes `ident_gate.rs`, creates `ident_gate/mod.rs`,
  reconciles qualified sibling imports, and performs any caller-neutral
  integration edits.
- Every existing test and shared test helper has exactly one owner in the
  inventory below; extraction must preserve each owned test rather than
  reconstructing reduced coverage.

- [x] Extract owner and membership resolution into `resolution.rs`.
  - Contract: owns `owner_aliases`, `Resolver`, `Membership`, and `type_name`;
    sibling-used declarations have `pub(super)` visibility, including
    `type_name` for traversal; owns the corresponding tests.
  - Verification: focused xtask tests prove aliases, qualified/imported/Self
    membership, unresolved fail-closed handling, and parse failures.
- [x] Extract syntax traversal into `traversal.rs`.
  - Contract: depends on `resolution`; owns `Scan`, `Mention`, `MentionContext`,
    `scan`, scanner internals, test-range detection, and macro-token walking;
    provides marker policy the existing test-range query without widening the
    assembled façade.
  - Verification: focused xtask tests prove owned-site detection,
    foreign-definition suppression, impl/function context, macros, test ranges,
    and cross-file aliases.
- [x] Extract marker classification into `marker_policy.rs`.
  - Contract: depends on traversal output and `crate::markers`; owns `classify`,
    `Classified`, `Marked`, `Unexempt`, `Why`, required reasons, shared-site
    handling, and orphan detection.
  - Verification: focused xtask tests prove placement, reasons, shared lines,
    orphans, test-code handling, strings/comments, macros, and census ordering.
- [x] Extract reporting and execution into `orchestration.rs`.
  - Contract: depends on resolution, traversal, and marker policy; owns `Gate`,
    `Report`, `run_scan`, diagnostics, and test-only assembled violations.
  - Verification: existing raw-HTML-door and HTML-sink tests exercise the
    unchanged façade and report behavior.
- [x] Assemble and verify `ident_gate/mod.rs`.
  - Contract: module documentation plus declarations and explicit re-exports
    only; expose exactly `Gate`, `Report`, and `run_scan`, while leaf-only items
    retain the least visibility required by sibling callers.
  - Verification: focused xtask library tests pass, repository static checks
    pass, and the commit gate passes on the staged tree.

### Narrow internal façade

- The user-approved contract preserves only the consumer imports used by
  `raw_html_door_check.rs` and `html_sink_check.rs`: `Gate`, `Report`, and
  `run_scan`.
- `mod.rs` re-exports exactly those three orchestration items. It does not
  provide a public or crate-visible `steps::ident_gate` path for resolution,
  traversal, marker-policy, or test-helper internals.
- Each leaf keeps its implementation private unless a sibling needs it; those
  cross-leaf declarations, fields, and test-only methods use `pub(super)`.
- Cross-leaf imports and intra-doc links name their leaf owners directly rather
  than recreating a façade re-export.

### Test ownership inventory

- `resolution.rs`: the alias-harvest tests at original lines 931–1027 and
  resolver-membership tests at 1070–1159; helpers `src`, `first_policed_path`,
  and `resolve`.
- `traversal.rs`: `mentions_come_back_in_line_order` and the owned/unowned
  traversal integration tests at 1183–1309; helpers `classified_owned` and
  `classified_unowned`.
- `marker_policy.rs`: marker-classification tests at 1325–1500; helper
  `classified`.
- `orchestration.rs`: no tests move from `ident_gate.rs`; the existing
  raw-HTML-door and HTML-sink tests remain in their consumer modules and
  continue exercising `Gate::violations`, `Gate::problems`, and `run_scan`.

## Risk checks

- Preserve ADR-0085 structural enumeration, site-scoped reasons, orphan
  detection, unreadable-input failure, and documented blind spots.
- Preserve ADR-0110 structural membership and fail-closed unresolved cases.
- Keep raw-HTML-door and HTML-sink on one traversal implementation.
- Keep traversal replaceable without moving marker policy or orchestration into
  that seam.
- Keep `mod.rs` free of implementation items and tests under ADR-0128.
- Preserve caller imports in `raw_html_door_check.rs` and `html_sink_check.rs`
  and the existing `steps::ident_gate` façade.
