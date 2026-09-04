# Issue #986: Split trace analysis reports

## Outcome

Replace the 1,489-line `xtask/src/traces/analyze.rs` with five focused leaves
under `xtask/src/traces/analyze/` and an assembly-only module surface. Existing
`traces::analyze::*` paths, report contents and ordering, parsing and error
behavior, and command output remain unchanged.

## Load-bearing decisions

- `xtask/src/traces/analyze/mod.rs` declares five private leaves and explicitly
  re-exports the production-consumed and test-consumed facade items under their
  existing paths. Test-only facade items are gated with `#[cfg(test)]`. The file
  contains no behavior, types, constants, implementations, or tests, per
  ADR-0128.
- `model.rs` owns `Analysis`, every report-row type, and `LIFECYCLE_SPAN_NAME`.
  It depends on no sibling leaf.
- `browser.rs` owns the sections derived from browser-capture JSON: action
  hotspots; navigation phase and target aggregation; boot-decomposition
  coverage; long-task aggregation; and resource initiator and asset aggregation.
  Their scalar parsing, browser-span selection, hotspot accumulation, and
  section-local accumulator types stay with this owner.
- `span_tree.rs` owns lifecycle-tree interval-union coverage, the Playwright
  attempt-duration join, and the explanatory note for an empty coverage section.
- `summary.rs` owns the direct span summaries: slowest spans, slowest `e2e.test`
  spans, duration grouped by project, and totals grouped by trace. Each section
  leaf owns private copies of the small scalar, labeling, grouping, and
  stable-sort helpers its algorithm needs; `browser` and `span_tree` do not
  depend on `summary`, and no cross-leaf utility module is introduced merely to
  deduplicate those helpers.
- `orchestrate.rs` owns `analyze_spans`, the assembly of all section results
  into `Analysis`, and the file-reading `analyze` entry point. It contains
  coordination only, not section algorithms.
- The dependency direction is acyclic: `model` depends on no sibling; `browser`,
  `span_tree`, and `summary` depend on `model` plus the existing external trace
  seams they consume; `orchestrate` depends on all four leaves. No section leaf
  calls orchestration, and the model does not call any section.
- The non-test facade under `traces::analyze` retains the items with production
  consumers: `Analysis`, all existing row types, and `analyze`.
  `LIFECYCLE_SPAN_NAME`, `span_coverage`, and `analyze_spans` have only test
  consumers and remain available through the same facade under `#[cfg(test)]`;
  xtask is an internal tool, so no unused non-test internal API is preserved.
  Leaf modules remain private, and no compatibility aliases are added.
- ADR-0011 remains exact: trace identity/source fields and fail-closed error
  propagation are preserved; malformed section JSON and unreadable inputs remain
  errors rather than empty reports.
- ADR-0096 remains exact: `e2e.test` retains its meaning, lifecycle coverage
  remains the interval union of named envelope children, attempt joins retain
  retry/source semantics, and an absent denominator or lifecycle match remains
  explicitly explained.
- ADR-0100 remains exact: boot decomposition uses document-frame marks;
  `commitToMountMs` remains only the mounted proxy/bridge and is never treated
  as an additive decomposition total; dropped top-navigation counts remain
  visible.
- Completed issue #831 remains independent: `boot_coverage_rows` continues to
  report per-`(source, project)` capture coverage, while the strict per-combo
  gate remains in `steps/boot_decomposition_coverage.rs`. This refactor does not
  merge or weaken those seams.
- Unit tests move with the implementation or contract they prove. Tests that
  exercise full `Analysis` assembly or file reading live with `orchestrate`;
  browser JSON-section tests live with `browser`; lifecycle-tree/report-join
  tests live with `span_tree`; direct aggregate tests live with `summary`;
  render contracts live in `render.rs`. The mixed
  `analyze_project_filter_over_fixture` test keeps its analyzer/filter
  assertions with orchestration, while its render-header assertion moves to the
  render owner rather than preserving a reverse dependency from analysis to
  rendering.

## Acceptance

- `xtask/src/traces/analyze/` contains exactly `mod.rs`, `model.rs`,
  `browser.rs`, `span_tree.rs`, `summary.rs`, and `orchestrate.rs`, each with
  the named responsibility above.
- `mod.rs` is assembly-only and uses explicit re-exports; every production- or
  test-consumed pre-split facade path resolves in its relevant build, with no
  new public leaf modules and no unused non-test re-exports.
- All pre-split analyzer cases and assertions remain present under the owning
  leaf. Existing render tests otherwise remain unchanged except for
  compilation-preserving imports.
- `Analysis` retains the same fields, field types, defaults, and section
  meaning. Every section remains fully sorted in the analyzer; `render` alone
  applies `--top` slicing and preserves canonical output order.
- Browser attribute names, absent-field defaults, finite/negative filtering, URL
  normalization, insertion-order tie behavior, and malformed-JSON errors are
  unchanged.
- Boot coverage remains grouped by `(source, project)`, retains
  legacy/current/full-mark/mounted/dropped accounting, and preserves the
  ADR-0100 clock-frame distinction.
- Span coverage retains lifecycle-child interval union, report-attempt matching,
  retry/source behavior, descending uncovered-time ordering, and all existing
  empty-section notes.
- `analyze` reads every input with the existing filters and delegates to the
  pure `analyze_spans` seam; the two CLI call sites and `render` continue
  consuming the same stable interface.
- The `traces::analyze` focused tests and repository pre-commit gate pass.

## Boundaries

- No trace schema, report section, aggregation formula, sort order, display
  text, CLI grammar, file format, threshold, or gate policy changes.
- No redesign of `parse`, `render`, `report`, `run`, `boot_phases`, e2e capture,
  or boot-decomposition verification.
- No generic helper or report framework is introduced. Small helpers stay with
  their semantic owner; duplication is preferable to a cross-leaf utility module
  without a domain responsibility.
- No ADR is needed: this projects ADR-0011, ADR-0096, ADR-0100, and ADR-0128
  into a more cohesive module layout.
