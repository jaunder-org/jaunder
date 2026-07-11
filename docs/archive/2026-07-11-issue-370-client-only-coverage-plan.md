# `#[client_only]` Coverage Exemption — Implementation Plan

> **For agentic workers:** Execute this plan task-by-task with `jaunder-iterate`
> (delegating individual tasks to a subagent via `jaunder-dispatch` when
> useful). Steps use checkbox (`- [ ]`) syntax for tracking.

**Spec:**
[`docs/archive/2026-07-11-issue-370-client-only-coverage-spec.md`](./2026-07-11-issue-370-client-only-coverage-spec.md)
— read it; this plan is the "how," the spec is the "what/why." **Issue:**
jaunder-org/jaunder#370.

**Goal:** Replace the hand-written `cov:ignore` block over the four client-only
`Invalidator` helpers with a principled `#[client_only]` attribute the coverage
framework recognizes, generalizing the existing `#[component]` rule to methods.

**Architecture:** A new `macros` proc-macro crate exports an identity
`#[client_only]` attribute (D1). `xtask/src/coverage/exempt.rs` recognizes
`client_only` (alongside `component`) on both free functions and methods (D2).
The four `Invalidator` helpers carry the bare-ident attribute; the `cov:ignore`
block is deleted (D6). The A1 guard and gate messaging generalize with no
`gate.rs` change (D3/D5).

**Tech Stack:** Rust (edition 2021), `proc_macro`, `syn`, `cargo xtask` gate,
Nix/crane coverage.

## Global Constraints

- **No `Co-Authored-By` trailer** on any commit.
- Rust **edition 2021**, `version = "0.1.0"`, matching the workspace's other
  crates. Exact crate name: **`macros`** (terse/unprefixed, per D1).
- **`git add` every new file before gating** — the Nix flake ignores untracked
  files even on a dirty tree, so an un-added `macros/` file yields a build that
  can't see it.
- **Serialize edit → gate → commit.** Never edit tracked files while a gated
  commit is running (the Nix build reads the working tree mid-commit).
- The marker in `reactive.rs` must be the **bare-ident** `#[client_only]`
  (path-anchored recognition — a `#[macros::client_only]` path compiles but is
  not recognized → gate failure). Import it: `use macros::client_only;`.
- Per-commit gate is `cargo xtask check` (fmt + clippy + Nix coverage/tests);
  run it clean before each commit (**`jaunder-commit`**). xtask's own tests run
  via `--manifest-path xtask/Cargo.toml` (xtask is excluded from the workspace).

---

## Review header

**Scope — in:** the `macros` crate; the `exempt.rs` recognition rule + tests;
the `coverage/mod.rs` message wording; the `reactive.rs` marker swap; a
`macros`-charter ADR draft; a green full gate. **Scope — out:** `gate.rs`, the
CRAP threshold, the `unreachable!` rule, wasm-bindgen-test coverage of these
helpers, adopting `#[client_only]` anywhere beyond the four `Invalidator`
helpers. (No separable concerns surfaced — no issue-filing task.)

**Tasks:**

1. ADR draft — the `macros` crate charter (A1a).
2. Create the `macros` proc-macro crate + wire the workspace (A1).
3. Coverage framework recognizes `#[client_only]` — `exempt.rs` + `mod.rs`
   messaging + tests (A2, D5).
4. Adopt `#[client_only]` in `web/src/reactive.rs`; delete the `cov:ignore`
   block (A3); confirm the coverage gate (A5).
5. Full gate — `cargo xtask validate` green, incl. audiences e2e (A6).

**Key risks/decisions:** (a) **A5** — a proc-macro crate is not linked into the
instrumented test binaries, so `macros` is expected to add no gate-failing
coverage line; verified by Task 4's `cargo xtask check`, with a documented
single-`cov:ignore` fallback if a 0-count line appears. (b) Recognition is
**path-anchored** (`is_ident`), so the marker must be a bare ident (Global
Constraints). (c) The A1 guard generalizes for free — Task 3 touches no
`gate.rs`.

---

### Task 1: ADR draft — the `macros` crate charter

**Files:**

- Create: `docs/adr/drafts/macros-crate-proc-macro-home.md` (numberless draft;
  `cargo xtask adr promote` numbers it at ship — **`jaunder-adr`** flow).

**Interfaces:** none (documentation).

- [x] **Step 1: Write the draft.** A short ADR, Status `proposed`, situated in
      the crate-layering family (references ADR-0055/0056/0058). Content:
  - **Context:** #370 needs a real `#[client_only]` attribute; a custom
    attribute must be proc-macro-backed, and a `proc-macro = true` crate can
    hold only proc-macros (host build-time, no runtime code) — so it cannot live
    in `web` or a future `client` crate.
  - **Decision:** introduce a **`macros`** workspace crate as the home for the
    workspace's proc-macros — target-agnostic _build-time_ tooling, deliberately
    unprefixed/ungeneralized (not `web-`/`client-`/`coverage-`). It is
    **orthogonal to** the `common`/`host`/`client` _runtime_ trio (ADR-0058),
    not a fourth member of it. First tenant: `#[client_only]`, an identity
    attribute the coverage framework (`xtask/src/coverage/exempt.rs`) recognizes
    syntactically — a macro-backed peer of the `cov:ignore` / `crap:allow`
    comment markers.
  - **Consequences:** future workspace proc-macros land here; no explicit
    coverage/CI wiring (the Nix source filter auto-admits any new top-level
    crate; nextest/clippy run workspace-wide); a proc-macro crate is not linked
    into instrumented test binaries, so it adds no coverage surface.

- [x] **Step 2: Verify prettier-clean — NO commit (draft is out-of-git).** Ran
      `prettier -w docs/adr/drafts/macros-crate-proc-macro-home.md`.
      **Correction to the plan:** `docs/adr/drafts/*` is gitignored (ADR-0048
      out-of-git draft workflow) — the draft lives on disk untracked and
      `cargo xtask adr promote` numbers + commits it as `docs/adr/NNNN-*.md` at
      ship (`jaunder-ship`). So there is no commit for the draft here; the
      deliverable is the on-disk draft.

---

### Task 2: Create the `macros` proc-macro crate + wire the workspace

**Files:**

- Create: `macros/Cargo.toml`
- Create: `macros/src/lib.rs`
- Create: `macros/tests/identity.rs` (integration test — a `tests/` file is a
  separate crate that may invoke the proc-macro)
- Modify: `Cargo.toml` (root) — add `"macros"` to `members`
- Modify: `web/Cargo.toml` — add the path dependency

**Interfaces:**

- Produces: `macros::client_only` — a `#[proc_macro_attribute]` identity
  attribute (item in → item out, unchanged), consumed by Task 4.

- [x] **Step 1: Write the failing integration test.**
      `macros/tests/identity.rs`:

  ```rust
  //! The `#[client_only]` attribute must be a no-op: an annotated item compiles and
  //! behaves exactly as if unannotated.
  #[macros::client_only]
  fn answer() -> u32 {
      42
  }

  #[macros::client_only]
  fn add(a: u32, b: u32) -> u32 {
      a + b
  }

  #[test]
  fn client_only_is_identity() {
      assert_eq!(answer(), 42);
      assert_eq!(add(2, 3), 5);
  }
  ```

- [x] **Step 2: Run it, verify it fails.** Run: `cargo test -p macros` Expected:
      FAIL — `error: package(s) 'macros' not found in workspace` (the crate does
      not exist yet; the red is package-missing, established in Step 3).

- [x] **Step 3: Implement the crate.** `macros/Cargo.toml`:

  ```toml
  [package]
  name = "macros"
  version = "0.1.0"
  edition = "2021"
  license = "GPL-3.0-only"   # required — cargo-deny fails an unlicensed crate

  [lib]
  proc-macro = true
  ```

  `macros/src/lib.rs`:

  ```rust
  //! Workspace proc-macros (ADR: the `macros` proc-macro-home crate). A target-agnostic,
  //! host-compiled build-time crate — the home for the workspace's proc-macros — distinct
  //! from the `common`/`host`/`client` runtime trio.

  use proc_macro::TokenStream;

  /// Marks a **client-only reactive helper**: code that runs only in the browser (a
  /// `server_resource` fetch, or an `Effect` that fires only client-side) and is exercised
  /// by e2e, not host tests. It is an **identity** attribute — it expands to the annotated
  /// item unchanged. Its sole purpose is to be a syntactic marker the coverage framework
  /// (`xtask/src/coverage/exempt.rs`) recognizes and exempts, generalizing the `#[component]`
  /// rule to non-component helpers (a macro-backed peer of the `cov:ignore` comment marker).
  ///
  /// Interim until wasm-bindgen-test can cover these in a headless browser (Test-infra epic).
  #[proc_macro_attribute]
  pub fn client_only(_attr: TokenStream, item: TokenStream) -> TokenStream {
      item
  }
  ```

  Root `Cargo.toml` — add `"macros"` to `members` (keep the list sorted):

  ```toml
  members = [
    "common",
    "csr",
    "host",
    "macros",
    "server",
    "storage",
    "test-support",
    "web"
  ]
  ```

  `web/Cargo.toml` — add under `[dependencies]`:

  ```toml
  macros = { path = "../macros" }
  ```

- [x] **Step 4: Run the test, verify it passes.** Run: `cargo test -p macros`
      Expected: PASS (`client_only_is_identity`).

- [x] **Step 5: Verify the workspace still builds.** Run: `cargo build -p web` —
      Expected: PASS (web now depends on `macros`, not yet using the attribute).

- [x] **Step 6: Commit** (git-add the new crate so Nix sees it).
  ```bash
  git add macros/ Cargo.toml Cargo.lock web/Cargo.toml
  git commit -m "feat(macros): add proc-macro crate with identity #[client_only] (#370)"
  ```
  Run `cargo xtask check` first so it passes clean (**`jaunder-commit`**). The
  Nix build inside `check` compiles the whole workspace, so a green run proves
  the flake admits the new top-level `macros/` crate (**A4** — an un-admitted
  member fails the Nix workspace build).

---

### Task 3: Coverage framework recognizes `#[client_only]`

**Files:**

- Modify: `xtask/src/coverage/exempt.rs` (visitor + predicate + module docs +
  in-file tests)
- Modify: `xtask/src/coverage/mod.rs` (failure-report + doc wording — D5)

**Interfaces:**

- Consumes: nothing from Task 2 — `exempt.rs` parses source **text** with `syn`,
  so it needs neither the `macros` crate nor the attribute to compile.
- `exempt_lines(src: &str) -> syn::Result<BTreeSet<u32>>` — signature unchanged;
  now also exempts (signature + body) items carrying `#[client_only]`.

- [x] **Step 1: Write the failing tests.** Append to the
      `#[cfg(test)] mod tests` in `exempt.rs`:

  ```rust
  #[test]
  fn exempts_client_only_method() {
      // Client-only helpers are METHODS (ImplItemFn), not free fns — the visitor must
      // reach them via visit_impl_item_fn, exempting signature + body like #[component].
      let src = "\
  struct S;
  impl S {
      #[client_only]
      fn helper(&self) -> u32 {
          let x = 1;
          x
      }
  }
  ";
      let ex = exempt_lines(src).unwrap();
      // Body braces span lines 4..=7; the interior statements (5, 6) must be exempt.
      assert!(ex.contains(&5), "client_only method body exempt: {ex:?}");
      assert!(ex.contains(&6), "client_only method body exempt: {ex:?}");
  }

  #[test]
  fn exempts_client_only_free_fn() {
      let src = "\
  #[client_only]
  fn helper() -> u32 {
      let x = 1;
      x
  }
  ";
      let ex = exempt_lines(src).unwrap();
      assert!(ex.contains(&3), "client_only free-fn body exempt: {ex:?}");
      assert!(ex.contains(&4), "client_only free-fn body exempt: {ex:?}");
  }

  #[test]
  fn does_not_exempt_unmarked_method() {
      let src = "\
  struct S;
  impl S {
      fn helper(&self) -> u32 {
          let x = 1;
          x
      }
  }
  ";
      let ex = exempt_lines(src).unwrap();
      assert!(ex.is_empty(), "unmarked method stays measured: {ex:?}");
  }

  #[test]
  fn does_not_exempt_non_ident_client_only_path() {
      // Path-anchored (is_ident), matching has_component_attr: a multi-segment path
      // must NOT match, so the bare-ident marker is the only recognized form.
      let src = "\
  #[foo::client_only]
  fn helper() -> u32 {
      let x = 1;
      x
  }
  ";
      let ex = exempt_lines(src).unwrap();
      assert!(ex.is_empty(), "#[foo::client_only] must not match: {ex:?}");
  }
  ```

- [x] **Step 2: Run them, verify they fail.** Run:
      `cargo test --manifest-path xtask/Cargo.toml exempt` Expected: FAIL —
      `exempts_client_only_method` / `_free_fn` fail (nothing exempt yet); the
      two negative tests already pass.

- [x] **Step 3: Implement the generalized recognition.** In `exempt.rs`:
  - Generalize the predicate (rename `has_component_attr` → `has_exempt_attr`;
    keep the doc):
    ```rust
    /// Matches `#[component]`/`#[component(...)]` OR `#[client_only]`/`#[client_only(...)]`
    /// — path-anchored, not a substring scan, so `#[my::client_only_thing]` does not
    /// falsely match.
    fn has_exempt_attr(attrs: &[syn::Attribute]) -> bool {
        attrs
            .iter()
            .any(|a| a.path().is_ident("component") || a.path().is_ident("client_only"))
    }
    ```
  - Point `visit_item_fn` at `has_exempt_attr` (replace the `has_component_attr`
    call).
  - Add the method arm to the `impl Visit`:
    ```rust
    /// A `#[client_only]` (or, in principle, `#[component]`) METHOD — the client-only
    /// `Invalidator` reactive helpers — is exempt signature + body, exactly as the free-fn
    /// arm does: the body runs only in the browser and the signature lines are equally
    /// un-exercised host-side.
    fn visit_impl_item_fn(&mut self, f: &'ast syn::ImplItemFn) {
        if has_exempt_attr(&f.attrs) {
            add_span(self.out, f.sig.span());
            add_span(self.out, f.block.span());
        }
        syn::visit::visit_impl_item_fn(self, f);
    }
    ```
  - Update the module doc (top of file): extend the first bullet so it reads
    that a `#[component]` **or** `#[client_only]` function/method (signature AND
    body) is exempt — `#[component]` for browser-rendered component bodies,
    `#[client_only]` for non-component client-only reactive helpers (e.g.
    `web::reactive::Invalidator`'s `resource`/`action`) exercised by e2e, not
    host tests. Note both are recognized on free functions and methods.

- [x] **Step 4: Run the tests, verify they pass.** Run:
      `cargo test --manifest-path xtask/Cargo.toml exempt` Expected: PASS (all
      four new tests + the existing `#[component]`/`unreachable!` tests).

- [x] **Step 5: Generalize the gate messaging (D5).** In `coverage/mod.rs`, name
      `#[client_only]` alongside `#[component]` in user-facing text —
      **without** changing the substrings the `mod.rs` tests assert
      (`cov:ignore`, `crap:allow`, `revisit the exemption`, `uncovered (`,
      `A1-guard`):
  - `failure_report`'s uncovered header:
    `"…not #[component]/#[client_only]-exempt, not an unreachable!(\"msg\"), not cov:ignore'd…"`.
  - `failure_report`'s A1 header:
    `"…covered line inside a #[component]/#[client_only] or unreachable! span:"`.
  - `failure_report`'s guard guidance: generalize "a #[component] body is being
    exercised natively…" to "a #[component]/#[client_only] body is being
    exercised natively…".
  - The module doc + `CoverageReport` field docs that say "#[component]" →
    "#[component]/#[client_only]". Then run
    `cargo test --manifest-path xtask/Cargo.toml coverage::mod` (or the whole
    xtask suite) — Expected: PASS (asserted substrings preserved).

- [x] **Step 6: Commit.**
  ```bash
  git add xtask/src/coverage/exempt.rs xtask/src/coverage/mod.rs
  git commit -m "feat(coverage): exempt #[client_only] fns and methods, generalizing #[component] (#370)"
  ```
  Run `cargo xtask check` first so it passes clean.

---

### Task 4: Adopt `#[client_only]` in `web/src/reactive.rs`

**Files:**

- Modify: `web/src/reactive.rs`

**Interfaces:**

- Consumes: `macros::client_only` (Task 2); the recognition rule (Task 3).

- [x] **Step 1: Make the edits.**
  - Add the import near the other `use`s at the top (after
    `use leptos::server_fn::ServerFn;`):
    ```rust
    use macros::client_only;
    ```
  - Delete the two-line `// cov:ignore-start — …` comment (currently
    `reactive.rs:45-46`) and the `// cov:ignore-stop` line (currently `:152`).
  - Add the bare-ident `#[client_only]` to each of the four helpers, immediately
    below the existing `#[must_use]` (so: doc comment, `#[must_use]`,
    `#[client_only]`, `pub fn …`): **`resource`**, **`action`**, **`patched`**,
    **`sticky`**.

- [x] **Step 2: Verify it compiles and there is no stray marker.**
  - Run: `cargo build -p web` — Expected: PASS (the attribute is a no-op).
  - Run: `rg 'cov:ignore' web/src/reactive.rs` — Expected: no matches.

- [x] **Step 3: Confirm the coverage gate (A5 + reactive-helper exemption).**
      Run: `cargo xtask check` (fmt + clippy + Nix coverage/tests). Expected:
      **PASS** — 0 coverage failures, 0 guard violations; the four helpers are
      now exempt via `#[client_only]` and `macros` contributes no gate-failing
      line. **If** the coverage report shows a 0-count `macros/src/lib.rs` line
      failing the gate (contrary to D4): apply the documented fallback — add a
      single `// cov:ignore` to the identity fn body line in `macros/src/lib.rs`
      (git-add it), noting it as the infra-level exclusion A5 anticipates — then
      re-run `cargo xtask check`. (The reactive-helper suppressions are removed
      either way.)

- [x] **Step 4: Commit** (serialize — do not edit tracked files while this
      commit's gate runs).
  ```bash
  git add web/src/reactive.rs
  git commit -m "web(reactive): mark client-only Invalidator helpers #[client_only], drop cov:ignore (#370)"
  ```

---

### Task 5: Full gate — `cargo xtask validate`

**Files:** none (final verification — A6).

- [x] **Step 1: Run the full local gate.** Run (Bash background mode — this is
      long/cold, includes e2e): `cargo xtask validate` Expected: **PASS** —
      static + clippy (host and wasm) + coverage + e2e across all
      `{sqlite,postgres}×{chromium,firefox}` combos. In particular the
      **audiences e2e** (which exercises `resource`/`action`/`patched`/`sticky`
      in the browser) passes, and the coverage step reports 0 failures / 0 guard
      violations.

- [x] **Step 2: Confirm completion.** Read the `xtask-done: … ok=true` sentinel
      and the `.xtask/last-result.json` sidecar
      (`jq '.ok, .steps[] | select(.ok==false)'`) — no failing step. Cycle ready
      for `jaunder-ship`. (No commit — verification only, unless Step 1 surfaces
      a fix.)

---

## Self-review

- **Spec coverage:** A1→Task 2; A1a→Task 1; A2→Task 3 (Steps 1–4 recognition +
  tests, Step 5 messaging=D5); A3→Task 4 (Steps 1–2); A4→Task 2 Step 6
  (git-add) + Global Constraints; A5→Task 4 Step 3 (+ fallback); A6→Task 5.
  D1/D1a/D2/D3/D4/D5/D6 all mapped. No spec requirement is unassigned.
- **Placeholder scan:** no TBD/"handle edge cases"/uncontracted steps — every
  implementation step carries the actual tests, signatures, or file text.
- **Type consistency:** `has_exempt_attr` (Task 3) is the renamed predicate used
  by both the free-fn and method arms; `macros::client_only` (Task 2) is the
  exact path imported in Task 4; the crate is `macros` everywhere. Consistent.
