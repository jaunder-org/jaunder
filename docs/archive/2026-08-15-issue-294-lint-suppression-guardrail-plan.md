# Lint Suppression Guardrail Implementation Plan

> **For agentic workers:** Execute this plan task-by-task with `jaunder-iterate`
> (delegating individual tasks to a subagent via `jaunder-dispatch` when
> useful). Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Make every Rust lint suppression an exact, reviewed member of the
expect-only inventory.

**Architecture:** Add one structural `syn` source gate in `xtask`, following
`target_arch_placement_check`'s fail-closed file walk and pure diagnostic seam.
The gate traverses parsed attributes, requires approved `#[expect]` sites to
carry adjacent `lint-suppression:allow` source markers, and is invoked from both
existing check pipelines. Eliminate the four existing test-only `allow`s,
retaining only lint members whose replacement `expect` is fulfilled.

**Tech Stack:** Rust 2024, `syn` 2 visitor API, `proc_macro2` span locations,
`quote`, existing `xtask` `CommandResult`/`StepResult` framework.

## Review

**Scope — in:** Structural Rust-attribute scanning across every first-party
source, test, and build-script path; exact source-site marker approval; hard
failure diagnostics; both xtask pipelines; elimination of four test-only
`allow`s with only fulfilled expectation replacements; contributor
documentation.

**Scope — out:** Re-auditing existing expects, changing panic sites/test logic,
non-Rust or command-line lint suppressions, new ADRs, and marker-framework
replacement beyond using ADR-0094's existing source-marker mechanism.

**Tasks:**

1. Add and test the fail-closed `lint-suppression` gate; register it; migrate
   the four test-only sites; document its approval protocol; commit the complete
   passing cutover.

**Key decisions/risks:**

- Attribute recognition uses `syn::Attribute::path()` and parsed `cfg_attr`
  nested meta exactly, never text matching: documentation and strings cannot
  create a false suppression.
- Every policed directory, standalone build script, file read, and parse is
  mandatory. A missing path or bad source must fail rather than silently reduce
  enforcement.
- Approval identity is the lint attribute's adjacent source marker plus
  `{relative_path, start_line, attribute_kind, normalized_tokens}` diagnostics.
  This makes every addition and relocation visible at the changed site.
- `#[allow]` is unconditionally rejected; only `#[expect]` sites with non-empty
  `lint-suppression:allow` markers may pass.

## File Structure

- Create: `xtask/src/steps/lint_suppression_check.rs` — structural collector,
  source-marker approval, diagnostics, recursive scan, and in-file unit tests.
- Modify: `xtask/src/lib.rs` — declare the step module and run it in both
  `Command::Check` and `Command::Validate` before the remaining static gates.
- Modify: `web/src/test_support.rs:8-11`, `web/src/posts/api.rs` test module,
  `web/src/subscriptions/server.rs` test module,
  `web/src/timeline/server.rs:165-169` — eliminate the test-scoped inner `allow`
  attributes and retain only fulfilled `expect` replacements.
- Modify: `CONTRIBUTING.md:131-136` — explain the mechanically enforced
  expect-only source-marker approval rule.

## Global Constraints

- Implement
  [the approved specification](../specs/2026-08-15-issue-294-lint-suppression-guardrail.md)
  exactly; no new suppression or compatibility path is permitted.
- Scan these directories exactly: `client/src`, `common/src`, `csr/src`,
  `host/src`, `macros/src`, `macros/tests`, `server/src`, `server/tests`,
  `storage/src`, `test-support/src`, `test-support/tests`, `web/src`,
  `tools/coverage/src`, `tools/devtool/src`, `tools/devtool/tests`,
  `tools/doctests/src`, and `xtask/src`. Also scan the standalone build script
  `server/build.rs`.
- Use `syn` structural parsing and `proc_macro2::Span::start`; `xtask` already
  enables `syn` `full`/`visit` and `proc-macro2` `span-locations`.
- A lint-suppression marker requires explicit user approval. Never create one
  merely to silence a gate.
- Preserve the four migrated attributes' lint lists, comments, and test
  behavior.
- One clean commit; stage the checked tree before committing; no
  `Co-Authored-By` trailer.

---

### Task 1: Enforce the approved expect-only suppression inventory

**Files:**

- Create: `xtask/src/steps/lint_suppression_check.rs`
- Modify: `xtask/src/lib.rs:27-58, 469-495, 515-543`
- Modify: `web/src/test_support.rs:8-11`
- Modify: `web/src/posts/api.rs` test-module inner attribute
- Modify: `web/src/subscriptions/server.rs` test-module inner attribute
- Modify: `web/src/timeline/server.rs:165-169`
- Modify: `CONTRIBUTING.md:131-136`
- Test: `xtask/src/steps/lint_suppression_check.rs` in-file `#[cfg(test)]`

**Interfaces:**

- Produces: `pub fn problems(scanned: &[(String, String)]) -> Option<String>`.
  It returns one newline-delimited detail string containing every violation or
  `None` only when discovered expectations have valid source-site markers.
- Produces: `pub fn run(result: &mut CommandResult)`, which collects every `.rs`
  file below `POLICED_ROOTS`, reports missing roots/read failures, and pushes
  one `StepResult::{ok,fail}("lint-suppression")`.
- Internal record:
  `SuppressionSite { path: String, line: u32, kind: SuppressionKind, tokens: String }`,
  ordered for deterministic reconciliation. `SuppressionKind` has exactly
  `Allow` and `Expect`; `tokens` is
  `attr.meta.require_list()?.tokens.to_string()` for a recognized attribute.
- Consumes: `crate::result::{CommandResult, StepResult}`, `syn::visit::Visit`,
  `syn::spanned::Spanned`, and `quote::ToTokens` only if it is needed to
  normalize the attribute meta consistently.

- [x] **Step 1: Write the failing structural-gate tests.** Add these unit tests
      before the implementation:

  ```rust
  #[test]
  fn approved_expect_marker_is_clean() {
      let scanned = vec![(
          "common/src/test_support/content.rs".to_owned(),
          "// lint-suppression:allow approved test expectation\n#![expect(clippy::expect_used)]\n"
              .to_owned(),
      )];
      assert_eq!(problems_against(&scanned), None);
  }

  #[test]
  fn unapproved_expect_names_exact_site_and_approval_protocol() {
      let scanned = vec![(
          "web/src/new.rs".to_owned(),
          "#[expect(clippy::too_many_lines)]\nfn long() {}\n".to_owned(),
      )];
      let detail = problems_against(&scanned).expect("unapproved expectation fails");
      assert!(detail.contains("web/src/new.rs:1"));
      assert!(detail.contains("explicit user approval"));
      assert!(detail.contains("lint-suppression marker"));
  }

  #[test]
  fn allow_is_rejected() {
      let scanned = vec![(
          "web/src/test_support.rs".to_owned(),
          "#![allow(clippy::expect_used)]\nfn fixture() {}\n".to_owned(),
      )];
      assert!(problems_against(&scanned)
          .expect("allow fails")
          .contains("#[allow] is forbidden"));
  }

  #[test]
  fn marker_shape_failures_are_reported() {
      let scanned = vec![(
          "web/src/example.rs".to_owned(),
          "// lint-suppression:allow\n#[expect(dead_code)]\n// lint-suppression:allow orphan\nfn f() {}\n// lint-suppression:allow two sites\n#[expect(dead_code)] #[expect(unused)] fn g() {}\n"
              .to_owned(),
      )];
      let detail = problems_against(&scanned).expect("marker failures fail");
      assert!(detail.contains("marker needs a reason"));
      assert!(detail.contains("marker is orphaned"));
      assert!(detail.contains("marker points at multiple lint attributes"));
  }

  #[test]
  fn comments_strings_and_other_attributes_are_not_suppressions() {
      let scanned = vec![(
          "web/src/example.rs".to_owned(),
          r##"// #[expect(clippy::unwrap_used)]
  const S: &str = "#[allow(dead_code)]";
  #[derive(Debug)]
  struct Example;
  "##
          .to_owned(),
      )];
      assert_eq!(problems_against(&scanned), None);
  }

  #[test]
  fn invalid_rust_is_reported_not_skipped() {
      let scanned = vec![("web/src/broken.rs".to_owned(), "fn {".to_owned())];
      assert!(problems_against(&scanned)
          .expect("parse failure fails")
          .contains("web/src/broken.rs: cannot parse"));
  }
  ```

  Add an integration-seam test that `problems` accepts the actual committed
  source-site markers only after every source site's expected attribute is
  marked, including `server/tests/main.rs`'s crate-level expect. Add direct
  `rust_files` tests using `tempfile::tempdir()` that a missing root returns an
  error and a nested `.rs` file is included, plus a direct standalone-file test
  that confirms `server/build.rs`-style paths are scanned. Use the module's real
  helper names consistently.

- [x] **Step 2: Run the focused tests and confirm red.**

  Run:
  `devtool run -- cargo nextest run --manifest-path xtask/Cargo.toml lint_suppression_check`

  Expected: FAIL because `lint_suppression_check`, `problems_against`, the site
  model, and the gate are absent.

- [x] **Step 3: Implement the structural collector and reconciliation.** Create
      `lint_suppression_check.rs` with:
  - `POLICED_ROOTS` exactly as listed in Global Constraints, plus a
    `POLICED_FILES` list containing `server/build.rs`; recursively collect each
    root with `rust_files(dir: &Path, out: &mut Vec<PathBuf>) -> io::Result<()>`
    and require each standalone file to exist and be a regular `.rs` file.
    Pattern the walk after `target_arch_placement_check` but sort paths before
    reporting.
  - A `syn::visit::Visit` implementation that visits all file/item/nested
    attributes, recognizes direct `allow`/`expect` attributes and
    `allow`/`expect` nested inside `cfg_attr`, and records
    `attr.span().start().line`. Parse each file with `syn::parse_file`; route
    parse errors into the same diagnostics rather than dropping the file.
  - `problems_against(scanned)` as the pure test seam. Collect all parsed sites;
    separately emit every `Allow` as forbidden; emit every discovered `Expect`
    whose immediately preceding line lacks a non-empty `lint-suppression:allow`
    marker; emit bare, orphan, and multiple-site markers as errors before
    deduplicating sites. Use ordered site records so the result is stable.
  - Source-site markers containing the final sorted expectation inventory:
    existing expected attributes, `web/src/test_support.rs`'s fulfilled pair,
    and `web/src/posts/api.rs`'s fulfilled `clippy::unwrap_used`. The gate must
    never accept `Allow` through a marker.
  - `problems` delegating to `problems_against(scanned)`, with an actionable
    rule string: new lint expectations require explicit user approval in a
    reviewed source-site marker. When failures occur, append the derived
    approved expectation census. `run` must fail and return after a missing
    root; it must collect unreadable-file diagnostics alongside parse/site
    diagnostics before pushing the single `lint-suppression` result.

- [x] **Step 4: Wire the gate into both command paths.** Add
      `pub mod lint_suppression_check;` to `xtask/src/lib.rs`'s `steps` module
      list. Call `steps::lint_suppression_check::run(&mut result);` in both
      `Check` and `Validate`, directly after `target_arch_placement_check` so
      the two pipelines expose the same cheap structural source gate.

- [x] **Step 5: Eliminate the four legacy test-scoped allows and freeze the
      final marker inventory.** Remove
      `#![allow(clippy::unwrap_used, clippy::expect_used)]` from all four listed
      web test modules. Retain
      `#![expect(clippy::unwrap_used,     clippy::expect_used)]` only in
      `web/src/test_support.rs`; retain only `#![expect(clippy::unwrap_used)]`
      in `web/src/posts/api.rs`; remove the unfulfilled attributes entirely from
      `web/src/subscriptions/server.rs` and `web/src/timeline/server.rs`.
      Convert the existing cfg-gated `allow` in `storage/src/lib.rs` to a marked
      `expect`. Preserve test behavior and add `lint-suppression:allow` markers
      to every retained `#[expect]` site.

- [x] **Step 6: Document the update protocol.** Extend the existing
      `CONTRIBUTING.md` lint-suppression bullet: `cargo xtask check`/`validate`
      rejects every `#[allow]` and every `#[expect]` without a source-site
      `lint-suppression:allow` marker; after explicit user approval, add that
      reviewed marker in the same change. Preserve the existing prohibition on
      creating suppressions merely to pass a gate.

- [x] **Step 7: Run focused tests and the gate; confirm green.**

  Run separately:

  ```bash
  devtool run -- cargo nextest run --manifest-path xtask/Cargo.toml lint_suppression_check
  devtool run -- cargo xtask check
  ```

  Expected: both PASS; the latter reports `lint-suppression: ok`, every static
  gate and clippy pass, and the four web test modules retain their prior test
  behavior with no `#[allow]` attributes.

- [x] **Step 8: Tick, stage, gate, inspect, and commit the complete cutover.**
      Mark Steps 1–7 complete in this plan before staging. Run
      `devtool run -- cargo xtask check`, inspect its mechanical edits, stage
      exactly `xtask/src/steps/lint_suppression_check.rs`, `xtask/src/lib.rs`,
      the four web modules, `CONTRIBUTING.md`, and this plan, then commit:

  ```bash
  git commit -m "ci: guard lint suppressions"
  ```

  Expected: pre-commit repeats the cached gate successfully; commit contains no
  `Co-Authored-By` trailer. Reinspect the staged diff before the commit so the
  committed tree is exactly the checked tree.
