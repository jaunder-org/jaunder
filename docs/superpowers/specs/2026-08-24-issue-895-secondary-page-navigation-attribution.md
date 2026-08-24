# Issue #895 — Secondary-page navigation attribution

## Outcome

Secondary pages opened through `tracedContext` expose the same per-navigation
URL and timing attribution that the default test page already exposes, without
widening or redefining the `e2e.test` span. Trace analysis and the observability
guide state how default-page and secondary-page navigation counts reconcile, so
a reader can recover the true per-test document-load total from trace artifacts.

## Load-bearing decisions

- The owner of a secondary-page document load is the Playwright test whose
  `tracedContext` created the browser context; the owning identity is carried by
  the existing lifecycle tree, `e2e.file` / `e2e.test` / `e2e.project`
  attributes, and the per-test traceparent applied by `tracedContext`.
- `e2e.test` span id, time range, `e2e.navigation_count`, and
  `e2e.navigation_top_json` semantics stay unchanged: they describe the default
  test context only, preserving the comparability promised by ADR-0096 and
  documented after #788/#794.
- Secondary-page navigation detail belongs on `e2e.page` spans, using the
  existing capture path from `attachTraceCapture`; no page-level instrumentation
  bypass is introduced in `_autoPerfSpan` or specs.
- `e2e.page` navigation records include URLs and the same honest lifecycle
  fields available from the captured secondary context. Fields that cannot be
  computed for secondary contexts without a wider action/page identity change
  are either omitted/null by the same schema rules or documented as a known
  approximation; they are not filled with guessed values.
- Analyzer output treats navigation-bearing spans as the union of `e2e.test`
  plus `e2e.page` where the section claims to answer “which document loads
  occurred and where.” If a section remains default-page-only, its title and
  documentation say so explicitly.
- Top-N truncation remains explicit: any navigation JSON list emitted for
  secondary pages has a matching dropped-count attribute, and analysis that
  treats the list as a census accounts for the dropped count.
- The implementation records the corrected deterministic total from the
  certified `~/measurements/jaunder/issue-866-preload/traces/` corpus in
  `docs/observability.md`, including the arithmetic that relates default-page
  loads, secondary-page loads, and the combined total.

## Acceptance

- A test or focused trace fixture proves an `e2e.page` span from a
  `tracedContext` page carries `e2e.navigation_top_json` entries with URLs for
  its document loads and a matching dropped-count attribute.
- `cargo xtask traces analyze` includes secondary-page navigation JSON in the
  navigation URL/phase report, or a new adjacent reconciliation report states
  default-page count, secondary-page count, combined count, and any dropped
  count.
- Existing analyzer tests cover at least one trace containing both an `e2e.test`
  navigation and an `e2e.page` navigation, and fail if the secondary-page URL is
  ignored.
- The delivered docs state the corrected total for the issue corpus and explain
  whether `e2e.test.navigation_count + e2e.page.navigation_count` is the
  canonical total or why a different relationship is required.
- Verification runs the smallest targeted checks that exercise the changed
  harness/analyzer path; no e2e navigation behavior is changed to make the
  numbers move.

## Boundaries

- This cycle does not reduce or add document loads in the suite; #867 owns count
  reduction and the one-boot-per-page policy.
- This cycle does not widen `e2e.test`, move server request attribution, or
  change the `traceparent` contract.
- This cycle does not introduce a new tracing vocabulary for “feed,” “client,”
  or publication concepts; it stays within the existing e2e observability model.
- This cycle does not add thresholds or gates for navigation count budgets
  beyond the focused regression coverage needed for attribution.
