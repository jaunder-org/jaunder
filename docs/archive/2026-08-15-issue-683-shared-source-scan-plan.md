# Shared xtask Rust-source scan Implementation Plan

> **For agentic workers:** Execute this plan task-by-task with `jaunder-iterate`
> (delegating individual tasks to a subagent via `jaunder-dispatch` when
> useful). Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Centralize the repeated xtask Rust-source scan so no migrated static
gate can silently pass after a policed source file becomes unreadable or
non-UTF-8.

**Architecture:** Add a crate-private `steps::scan` runner over the existing
sorted `files::with_extension` walker. The runner owns root traversal, UTF-8
reads, and `StepResult` construction; individual checks continue to own only
their roots and pure analyzer. Migrate the simple one-input scan family and
ident-gate runner, but retain the richer server-function and SQLx-decode scan
paths.

**Tech Stack:** Rust 2024; xtask; `std::fs`; `tempfile`; `cargo nextest`.

## Review header

**Scope — in:** the common source-scan module; migrations of no-full-reload,
proffered-filename, proffered-secret, SQLx-newtype-bind, test-pattern,
target-architecture-placement, and ident-gate; helper regression tests.

**Scope — out:** `web_server_fns` / server-fn-registrar and
`sqlx_newtype_decode_check`, their metadata/model-building paths, gate rules,
roots, ADRs, and domain glossary.

**Tasks:**

1. Add the tested closed-population scan module and migrate target-architecture
   placement so the new interface is live at its first commit.
2. Replace the remaining duplicated one-input read-and-run layers with the
   shared runner.

**Key decisions/risks:** A source read failure stops analysis rather than
producing a passing partial population. The full file population is lexically
sorted across all roots for every migrated check; this intentionally normalizes
target-architecture-placement's cross-file report order.
`sqlx_newtype_decode_check` remains specialized because its analyzer depends on
an approve-set derived from extra roots and the macro crate.

**Specification:**
[`2026-08-15-issue-683-shared-source-scan.md`](../specs/2026-08-15-issue-683-shared-source-scan.md).

---

## Global Constraints

- Use `crate::files::with_extension`; do not introduce another recursive walker.
- `run_source_scan` must fail its named `CommandResult` step on root traversal,
  file-read, or UTF-8-decoding failure, with the relevant root/path and I/O
  error.
- It must not invoke `problems` unless every discovered source was read.
- Sort the full combined `PathBuf` population lexically after walking all roots
  and before reading any source.
- Preserve each migrated analyzer's roots and pure `problems()` implementation.
- Keep `web_server_fns` / server-fn-registrar and SQLx-newtype-decode on their
  existing richer scan paths.
- Before every commit, tick this plan's completed task, run
  `devtool run -- cargo xtask check`, stage the checked tree, and commit without
  a `Co-Authored-By` trailer.

### Task 1: Build the closed-population source scanner

**Files:**

- Create: `xtask/src/steps/scan.rs`
- Modify: `xtask/src/lib.rs:30-57`
- Modify: `xtask/src/steps/target_arch_placement_check.rs`
- Test: inline `#[cfg(test)]` modules in `xtask/src/steps/scan.rs` and
  `xtask/src/steps/target_arch_placement_check.rs`

**Interfaces:**

- Consumes:
  `crate::files::with_extension(&Path, "rs") -> io::Result<Vec<PathBuf>>` and
  `crate::result::{CommandResult, StepResult}`.
- Produces:

  ```rust
  pub(super) fn run_source_scan(
      result: &mut CommandResult,
      step: &'static str,
      roots: &[&str],
      problems: impl FnOnce(&[(String, String)]) -> Option<String>,
  );
  ```

  It discovers every `.rs` source, lexically sorts the complete combined
  population across every root, reads each with `std::fs::read_to_string`, and
  pushes exactly one `StepResult::ok(step)` or
  `StepResult::fail(step).detail(...)`. The private read seam accepts
  `FnMut(&Path) -> io::Result<String>` for deterministic I/O-error tests.

- [x] **Step 1: Declare the module and write failing scanner tests** Add
      `pub mod scan;` in `xtask/src/lib.rs`'s existing `steps` declaration
      block, then create `scan.rs` with the test module below. Leave
      `run_source_scan` undefined so the test target compiles far enough to
      report the missing interface.

  Add inline tests defining these observable contracts:

  ```rust
  #[test]
  fn sources_across_roots_are_passed_to_the_analyzer_in_lexical_order() {
      // Build `z-root/b.rs` and `a-root/nested/a.rs` in two tempfile roots; supply
      // them in reverse lexical root order. Assert received paths are
      // `[a-root/nested/a.rs, z-root/b.rs]` and the result contains one successful
      // step.
  }

  #[test]
  fn injected_read_failure_fails_the_named_step_without_analyzing() {
      // Inject PermissionDenied for `blocked.rs`; assert the analyzer flag remains
      // false and the sole failed step names `source-scan-test`, `blocked.rs`, and
      // the permission error.
  }

  #[test]
  fn invalid_utf8_file_fails_the_named_step_without_analyzing() {
      // Write `[0xff]` to `invalid.rs`; assert the analyzer flag remains false and
      // the sole failed step names `invalid.rs` and the UTF-8/InvalidData read error.
  }

  #[test]
  fn analyzer_problem_fails_the_named_step_after_a_complete_scan() {
      // Supply readable source and an analyzer returning `Some("violation")`; assert
      // the sole failed step is named `source-scan-test` with exactly that detail.
  }
  ```

- [x] **Step 2: Run the new scanner tests and verify they fail**

  Run:

  ```bash
  devtool run -- cargo nextest run --manifest-path xtask/Cargo.toml -E 'test(/steps::scan::tests::/)'
  ```

  Expected: FAIL because `run_source_scan` does not exist.

- [x] **Step 3: Implement the scanner and migrate target-architecture
      placement**

  Implement `run_source_scan` and its private discovery/read seam in
  `xtask/src/steps/scan.rs`:
  - call `files::with_extension(Path::new(root), "rs")` once per supplied root;
    on the first error, push `StepResult::fail(step)` with
    `cannot scan {root}: {e}` and return;
  - lexically sort the complete `PathBuf` population across all roots, then
    collect `(path.display().to_string(), source)` pairs in that order;
  - on any read/decode error, push `StepResult::fail(step)` containing the path
    and error, then return without calling the analyzer;
  - otherwise call `problems` once and map `None` to `ok(step)` and
    `Some(detail)` to `fail(step).detail(detail)`.

  In `target_arch_placement_check`, remove its local `rust_files` and
  read/result loop, retain `POLICED_ROOTS` and `problems`, and call
  `run_source_scan(result, "target-arch-placement", POLICED_ROOTS, problems)`.

- [x] **Step 4: Run the scanner and target-placement tests and verify they
      pass**

  Run:

  ```bash
  devtool run -- cargo nextest run --manifest-path xtask/Cargo.toml -E 'test(/steps::(scan|target_arch_placement_check)::tests::/)'
  ```

  Expected: PASS.

- [x] **Step 5: Commit the live scanner task**

  Tick Task 1, run `devtool run -- cargo xtask check`, inspect the checked diff,
  stage the approved specification, this plan, `xtask/src/lib.rs`,
  `xtask/src/steps/scan.rs`, and
  `xtask/src/steps/target_arch_placement_check.rs`, then commit:

  ```bash
  git commit -m "refactor(xtask): centralize Rust source scans"
  ```

### Task 2: Migrate one-input static gates

**Files:**

- Modify: `xtask/src/steps/no_full_reload_check.rs`
- Modify: `xtask/src/steps/proffered_filename_check.rs`
- Modify: `xtask/src/steps/proffered_secret_check.rs`
- Modify: `xtask/src/steps/sqlx_newtype_bind_check.rs`
- Modify: `xtask/src/steps/test_pattern_check.rs`
- Modify: `xtask/src/steps/ident_gate.rs`
- Test: existing inline tests in the modified modules; `xtask/src/steps/scan.rs`

**Interfaces:**

- Consumes:

  ```rust
  crate::steps::scan::run_source_scan(
      result: &mut CommandResult,
      step: &'static str,
      roots: &[&str],
      problems: impl FnOnce(&[(String, String)]) -> Option<String>,
  );
  ```

- Produces: each existing `pub fn run(result: &mut CommandResult)` retains its
  step name, roots, and analyzer; `ident_gate::run_scan` retains its existing
  gate-specific inputs and delegates its source collection to `run_source_scan`.

- [x] **Step 1: Replace local collection loops**

  The shared scanner's direct tests from Task 1 already defend traversal,
  lexical ordering, permission failures, invalid UTF-8, and no partial analyzer
  invocation. Do not add caller-delegation tests that only couple tests to this
  refactor. Keep each existing direct `problems()` test unchanged as the
  gate-specific contract.

  In each listed remaining one-input check, remove its local `read_sources_with`
  / read loop and unnecessary `Path`, `files`, or `StepResult` imports; call
  `run_source_scan(result, STEP_NAME, POLICED_ROOTS, problems)`. In
  `ident_gate`, retain all structural parsing and marker policy but replace only
  its generic source collection/result construction with the helper. Do not
  change `web_server_fns`, server-fn-registrar, SQLx-newtype-decode, or the Task
  1 target-architecture migration.

- [x] **Step 2: Run the affected tests and verify they pass**

  Run:

  ```bash
  devtool run -- cargo nextest run --manifest-path xtask/Cargo.toml -E 'test(/steps::(scan|no_full_reload_check|proffered_filename_check|proffered_secret_check|sqlx_newtype_bind_check|test_pattern_check|target_arch_placement_check|ident_gate)::tests::/)'
  ```

  Expected: PASS, including existing pure analyzer tests and the common
  scanner's unreadable/invalid-UTF-8 contracts.

- [x] **Step 3: Commit the migration task**

  Tick Task 2, run `devtool run -- cargo xtask check`, inspect the checked diff,
  stage the modified step files and this plan, then commit:

  ```bash
  git commit -m "refactor(xtask): share source scan runners"
  ```

## Self-review

- **Spec coverage:** Task 1 implements D1–D3, AC2/AC5, and the
  target-architecture portion of D4/AC1/AC3. Task 2 completes D4, D6, AC1/AC3,
  and preserves D5/AC4 by listing those files as explicit non-targets. Both
  tasks require the repository gate for AC6.
- **Placeholder scan:** no deferred implementation, unnamed tests, or
  unspecified interfaces remain.
- **Type consistency:** every migration consumes the Task 1
  `run_source_scan(&mut CommandResult, &'static str, &[&str], FnOnce)`
  interface.

## Execution handoff

Plan complete and saved to
`/home/mdorman/src/jaunder/agent-7/docs/superpowers/plans/2026-08-15-issue-683-shared-source-scan.md`.

After approval, execute it task-by-task with `jaunder-iterate`; use
`jaunder-dispatch` for an individual task only when delegation improves the
implementation loop.
