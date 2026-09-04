# Issue #986 implementation outline

Spec: `docs/superpowers/specs/2026-09-03-issue-986-split-trace-analysis.md`

## Task 1: Extract the five trace-analysis owners

- [ ] Replace `xtask/src/traces/analyze.rs` with
      `xtask/src/traces/analyze/mod.rs`, `model.rs`, `browser.rs`,
      `span_tree.rs`, `summary.rs`, and `orchestrate.rs`.
- [ ] Put only private leaf declarations and explicit re-exports in `mod.rs`.
      Keep the production-consumed facade items (`Analysis`, every row type, and
      `analyze`) in non-test builds. Re-export `LIFECYCLE_SPAN_NAME`,
      `span_coverage`, and `analyze_spans` only under `#[cfg(test)]`, preserving
      their test paths without carrying unused non-test xtask internals.
- [ ] Move `Analysis`, every row type, and `LIFECYCLE_SPAN_NAME` to `model.rs`
      without changing fields, derives, visibility, documentation, or defaults.
- [ ] Move action, navigation, boot-coverage, long-task, and resource algorithms
      and their tests to `browser.rs`. Keep their attribute names,
      parsing/default rules, filtering, ordering, and `(source, project)`
      behavior byte-for-byte equivalent. Mark only the section functions called
      by orchestration `pub(super)`; keep their remaining helpers private.
- [ ] Move lifecycle-tree coverage, its empty-section note, and their tests to
      `span_tree.rs`. Preserve interval-union, retry/source attempt joins, note
      text, and descending uncovered-time ordering. Keep facade-re-exported
      `span_coverage` public, mark only the orchestration-facing coverage-note
      function `pub(super)`, and keep all other helpers private. Import
      `span_coverage` and `LIFECYCLE_SPAN_NAME` through the public
      `crate::traces::analyze` facade in the tests.
- [ ] Extract direct aggregate functions and their tests in `summary.rs` for
      slowest spans, slowest `e2e.test` spans, per-project duration, and
      per-trace totals. Keep insertion-order tie behavior and fully sorted rows.
      Mark only the functions called by orchestration `pub(super)`; keep their
      helpers private.
- [ ] Give `browser.rs`, `span_tree.rs`, and `summary.rs` their own small
      private scalar/label/sort helpers. Do not create a utility leaf or a
      sibling dependency solely to share them.
- [ ] Put `analyze_spans`, section assembly, file-reading `analyze`, and their
      full-assembly/file-error tests in `orchestrate.rs`. Call nonlocal free
      functions through the owning sibling module (`browser::…`, `span_tree::…`,
      `summary::…`) and call same-file functions directly; import data types
      directly. Keep orchestration free of section algorithms. Split
      `analyze_project_filter_over_fixture`: retain its filter and `Analysis`
      assertions here, and move the existing render-header assertion with
      sufficient setup to `render.rs`.

## Task 2: Audit tests and verify the stable seam

- [ ] Keep `render.rs`'s production imports through `super::analyze::{…}` so
      every model row remains facade-compiled. Keep both CLI callers on
      `traces::analyze::analyze`.
- [ ] Audit all moved tests against the pre-split inventory: no test case or
      assertion disappears; each owner has the tests for its contract; no
      reverse analysis-to-render dependency remains.
- [ ] Run
      `devtool run -- cargo xtask test-local -- --manifest-path xtask/Cargo.toml traces::analyze`.
- [ ] Run
      `devtool run -- cargo xtask test-local -- --manifest-path xtask/Cargo.toml traces::render`.
- [ ] Commit the coherent split as
      `refactor(xtask): split trace analysis reports`; let the pre-commit hook
      run the repository gate.

## Delivery

- [ ] Review the whole branch against repository standards and the approved
      spec. Resolve every finding and repeat both review axes until clean.
- [ ] Archive the approved spec and outline as the final semantic change, commit
      the archive, rebase onto current `origin/main`, and repeat affected
      focused tests if the base changed relevant trace tooling.
- [ ] Push, open a PR titled `refactor(xtask): split trace analysis reports`
      with `Closes #986`, and monitor required checks to `ready-to-land` before
      requesting merge approval.
