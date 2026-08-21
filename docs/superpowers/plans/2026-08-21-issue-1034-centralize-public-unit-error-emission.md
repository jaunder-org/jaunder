# Centralize Public Unit Error Emission Implementation Plan

> **For agentic workers:** Execute this plan task-by-task with `jaunder-iterate`
> (delegating individual tasks to a subagent via `jaunder-dispatch` when
> useful). Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Centralize the duplicated public unit error codegen used by
`NumNewtype` and `#[text_enum]` while preserving the emitted public token shape.

**Architecture:** Add one private helper in `macros/src/lib.rs` that emits the
shared public unit error struct, derives, `Display`, and `Error` impl. Keep
`num_newtype::error_type` and `text_enum::error_type` as caller-owned adapters
that compute each caller's error name, doc text, and already-resolved message
tokens.

**Tech Stack:** Rust proc-macro codegen with `proc_macro2`, `quote`, and `syn`;
unit tests in `macros/src/lib.rs` / `macros/src/text_enum.rs`; integration tests
in `macros/tests`.

**Scope In:** `macros/src/lib.rs`, `macros/src/num_newtype.rs`,
`macros/src/text_enum.rs`, and focused macro tests.

**Scope Out:** Public macro arguments, generated error names, generated
messages, generated visibility, emitted paths, `thiserror`, unrelated
hand-written errors, and reopening ADR-0091.

**Key risks/decisions:** Preserve fully-qualified emitted paths exactly enough
for current token assertions; keep the `#[text_enum]` parse error function
returning a bare unit expression; prove both audited adapters delegate to the
shared helper rather than only proving behavior.

## Global Constraints

- The shared helper lives in `macros/src/lib.rs`.
- `num_newtype::error_type` and `text_enum::error_type` both route through that
  helper.
- Generated errors remain public unit structs.
- Generated derives remain `Debug`, `Clone`, `Copy`, `PartialEq`, and `Eq` using
  the current fully-qualified emitted paths.
- Generated `Display` and `std::error::Error` impls preserve current behavior
  and emitted paths.
- `#[text_enum]` errors remain constructible as bare unit expressions,
  preserving ADR-0091.
- No public call sites or macro arguments change.
- No new dependency is introduced.

---

### Task 1: Add Cross-Caller Error Shape Coverage

**Files:**

- Modify: `macros/src/lib.rs`
- Modify: `macros/src/text_enum.rs`

**Interfaces:**

- Consumes: existing `num_newtype::expand`, `text_enum::expand`, and
  `sqlx_bridge::tests::norm`.
- Produces: failing tests that require the same generated public unit error
  shape for both `NumNewtype` and `#[text_enum]`.

- [x] **Step 1: Add a `NumNewtype` public unit error shape test**

  In `macros/src/lib.rs`'s existing `#[cfg(test)] mod tests`, add:

  ```rust
  #[test]
  fn num_newtype_generates_public_unit_error_shape() {
      let input: DeriveInput = parse_quote! {
          #[num_newtype(inner = u32, min = 1, error = "count must be positive")]
          struct Count(u32);
      };
      let out = sqlx_bridge::tests::norm(&num_newtype::expand(&input));
      assert!(out.contains("pubstructInvalidCount;"));
      assert!(out.contains(&sqlx_bridge::tests::norm_s(
          "#[derive(::core::fmt::Debug, ::core::clone::Clone, ::core::marker::Copy, \
           ::core::cmp::PartialEq, ::core::cmp::Eq)]"
      )));
      assert!(out.contains("::core::fmt::DisplayforInvalidCount"));
      assert!(out.contains("f.write_str(\"countmustbepositive\")"));
      assert!(out.contains("::std::error::ErrorforInvalidCount"));
  }
  ```

- [x] **Step 2: Tighten the existing `#[text_enum]` shape test**

  In `macros/src/text_enum.rs`, keep
  `generates_a_unit_error_matching_the_num_newtype_precedent` but make it assert
  the fully-qualified emitted paths and the bare unit parse-error return:

  ```rust
  assert!(out.contains("::core::fmt::DisplayforInvalidX"));
  assert!(out.contains("f.write_str(\"badx\")"));
  assert!(out.contains("::std::error::ErrorforInvalidX"));
  assert!(out.contains("fn__x_parse_err(_: &str)->InvalidX"));
  assert!(out.contains("InvalidX}"));
  ```

  Keep the existing assertions that the struct is bare, the derive list is
  unchanged, and `thiserror` is absent.

- [x] **Step 3: Run the focused tests and verify the expected failure**

  Run:

  ```bash
  devtool run -- cargo test -p macros num_newtype_generates_public_unit_error_shape
  ```

  Expected: FAIL before implementation only if the new assertion exposes a
  missing or misspelled current token. If it passes immediately, keep it: it is
  a regression pin for the subsequent refactor.

  Run:

  ```bash
  devtool run -- cargo test -p macros generates_a_unit_error_matching_the_num_newtype_precedent
  ```

  Expected: PASS or token-adjust FAIL that is fixed before implementation. This
  task's deliverable is accurate coverage, not the shared helper.

- [x] **Step 4: Run existing public macro integration tests**

  Run:

  ```bash
  devtool run -- cargo test -p macros --test text_enum
  ```

  Expected: PASS. This keeps the real `#[text_enum]` call-site behavior pinned.

  Run:

  ```bash
  devtool run -- cargo test -p macros --test num_newtype
  ```

  Expected: PASS. This keeps the real `NumNewtype` call-site behavior pinned.

- [x] **Step 5: Commit the coverage task**

  Tick this task's checkbox before the commit gate. Then run:

  ```bash
  devtool run -- cargo xtask check
  ```

  Inspect/stage any mechanical fixes from the gate, then commit the staged
  coverage changes:

  ```bash
  git add macros/src/lib.rs macros/src/text_enum.rs docs/superpowers/plans/2026-08-21-issue-1034-centralize-public-unit-error-emission.md
  git commit -m "test(macros): pin public unit error emission"
  ```

### Task 2: Introduce the Shared Public Unit Error Helper

**Files:**

- Modify: `macros/src/lib.rs`
- Modify: `macros/src/num_newtype.rs`
- Modify: `macros/src/text_enum.rs`

**Interfaces:**

- Produces:

  ```rust
  pub(crate) fn public_unit_error_type(
      error: &syn::Ident,
      doc: &str,
      message: &proc_macro2::TokenStream,
  ) -> proc_macro2::TokenStream
  ```

- Consumes: the Task 1 tests and existing macro expansion tests.

- [x] **Step 1: Add the shared helper**

  In `macros/src/lib.rs`, near the other shared codegen helpers, add:

  ```rust
  pub(crate) fn public_unit_error_type(
      error: &syn::Ident,
      doc: &str,
      message: &proc_macro2::TokenStream,
  ) -> proc_macro2::TokenStream {
      quote::quote! {
          #[doc = #doc]
          #[derive(::core::fmt::Debug, ::core::clone::Clone, ::core::marker::Copy, ::core::cmp::PartialEq, ::core::cmp::Eq)]
          pub struct #error;

          #[automatically_derived]
          impl ::core::fmt::Display for #error {
              fn fmt(&self, f: &mut ::core::fmt::Formatter<'_>) -> ::core::fmt::Result {
                  f.write_str(#message)
              }
          }

          #[automatically_derived]
          impl ::std::error::Error for #error {}
      }
  }
  ```

  The signature keeps the display text as already-resolved tokens so
  `NumNewtype` can pass either a literal override or generated literal tokens.

- [x] **Step 2: Route `NumNewtype` through the helper**

  In `macros/src/num_newtype.rs`, replace the body of `error_type` with a
  delegation that preserves the current doc and message:

  ```rust
  fn error_type(err_name: &Ident, message: &TokenStream) -> TokenStream {
      crate::public_unit_error_type(
          err_name,
          "Error returned when a value is out of the declared numeric bounds.",
          message,
      )
  }
  ```

- [x] **Step 3: Route `#[text_enum]` through the helper**

  In `macros/src/text_enum.rs`, replace the body of `error_type` with a
  delegation that preserves the enum-specific doc:

  ```rust
  fn error_type(error: &syn::Ident, message: &syn::LitStr, enum_name: &syn::Ident) -> TokenStream {
      let doc = format!("Parse error for [`{enum_name}`]'s string token.");
      let message = quote! { #message };
      crate::public_unit_error_type(error, &doc, &message)
  }
  ```

- [x] **Step 4: Run focused macro tests**

  Run:

  ```bash
  devtool run -- cargo test -p macros num_newtype_generates_public_unit_error_shape
  ```

  Expected: PASS.

  Run:

  ```bash
  devtool run -- cargo test -p macros generates_a_unit_error_matching_the_num_newtype_precedent
  ```

  Expected: PASS.

- [x] **Step 5: Run existing public macro integration tests**

  Run:

  ```bash
  devtool run -- cargo test -p macros --test text_enum
  ```

  Expected: PASS.

  Run:

  ```bash
  devtool run -- cargo test -p macros --test num_newtype
  ```

  Expected: PASS.

- [x] **Step 6: Source-structure review**

  Inspect the diff and confirm:
  - the derive/doc/struct/`Display`/`Error` quote block exists only in
    `public_unit_error_type`;
  - `num_newtype::error_type` delegates to `crate::public_unit_error_type`;
  - `text_enum::error_type` delegates to `crate::public_unit_error_type`;
  - the `#[text_enum]` parse error function still returns the bare error ident.

- [x] **Step 7: Commit the implementation task**

  Tick this task's checkbox before the commit gate. Then run:

  ```bash
  devtool run -- cargo xtask check
  ```

  Inspect/stage any mechanical fixes from the gate, then commit:

  ```bash
  git add macros/src/lib.rs macros/src/num_newtype.rs macros/src/text_enum.rs docs/superpowers/plans/2026-08-21-issue-1034-centralize-public-unit-error-emission.md
  git commit -m "refactor(macros): share public unit error codegen"
  ```

### Task 3: Final Conformance Sweep

**Files:**

- Modify:
  `docs/superpowers/plans/2026-08-21-issue-1034-centralize-public-unit-error-emission.md`

**Interfaces:**

- Consumes: completed implementation from Tasks 1 and 2.
- Produces: checked evidence that the branch satisfies the approved spec.

- [x] **Step 1: Re-read the approved spec against the final diff**

  Confirm every acceptance criterion in
  `docs/superpowers/specs/2026-08-21-issue-1034-centralize-public-unit-error-emission.md`
  maps to the final diff and test evidence.

- [x] **Step 2: Run the full local check**

  Run:

  ```bash
  devtool run -- cargo xtask check
  ```

  Expected: PASS.

- [x] **Step 3: Record completion in this plan**

  Tick the final task checkbox and keep the concrete check evidence in the
  commit message or PR body. Do not add a separate evidence section unless the
  implementation uncovers an unusual caveat.

- [x] **Step 4: Commit final plan progress if needed**

  If only this plan file changed since Task 2, run the commit gate first:

  ```bash
  devtool run -- cargo xtask check
  ```

  Then stage and commit:

  ```bash
  git add docs/superpowers/plans/2026-08-21-issue-1034-centralize-public-unit-error-emission.md
  git commit -m "docs: record issue 1034 completion"
  ```

## Self-Review

- Spec coverage: Task 1 pins both affected macro surfaces; Task 2 introduces the
  shared helper and routes both audited adapters through it; Task 3 verifies the
  final branch against the approved spec and runs `cargo xtask check`.
- Placeholder scan: no `TODO`, `TBD`, or unspecified test steps remain.
- Type consistency: the helper signature is declared once and consumed
  consistently by both adapter steps.
