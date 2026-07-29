# Issue #520 — web endgame ratchet: Implementation Plan

> **For agentic workers:** Execute this plan task-by-task with `jaunder-iterate`
> (delegating individual tasks to a subagent via `jaunder-dispatch` when
> useful). Steps use checkbox (`- [ ]`) syntax for tracking.

**Spec:** `docs/superpowers/specs/2026-07-28-issue-520-endgame-ratchet.md` — the
"what/why". This plan is the "how"; it does not restate the spec's analysis.

**Goal:** Make milestone #14's end state compile-enforced — `web` names no
browser API directly — and delete the interim coverage-exemption machinery that
no longer has any tenant.

**Architecture:** Four mechanical moves, ordered so every commit is
independently green. First relocate the one leaf-file `target_arch` violation,
then install the syn check that forbids its shape, then move `web`'s last two
`web_sys` call sites into `client` primitives and drop the `web-sys` + `macros`
dependencies, then delete the `#[client_only]` macro and the
`#[component]`/`#[client_only]` coverage exemption, and finally reconcile the
four ADRs and `CONTRIBUTING.md`.

**Tech Stack:** Rust, `syn` 2 (xtask static checks), Leptos 0.8 (CSR),
`server_fn` 0.8 `multipart` codec, `cargo xtask` gate ladder.

## Review header

**Scope — in:** `web/Cargo.toml` (drop `web-sys`, `macros`);
`web/src/reactive.rs` → `reactive/{mod,scope}.rs`; `web/src/media/component.rs`
(both `web_sys` sites); two new `client` primitives; a new `xtask` static check;
deletion of `macros::client_only` and the `#[component]`/`#[client_only]` arms
of `xtask/src/coverage/exempt.rs` + `coverage/mod.rs`; amendments to
ADR-0050/0062/0069/0070 and `CONTRIBUTING.md`.

Also in scope, two one-line dependency-honesty fixes in files Task 4 already
edits (spec §D1): drop the redundant `"leptos/ssr"` from `web/Cargo.toml:66`,
and rewrite `client/Cargo.toml:20-23`'s unsubstantiated comment.

**Scope — out:** linting feature-gated code in the gate ladder (**#678**);
archiving stale planning docs for shipped issues (**#679**). Actually shedding
the leptos SSR stack by replacing `leptos_axum` (**#677**).

**Tasks:**

1. File the two separable concerns as issues.
2. Split `invalidator_scope!` into the gated leaf `web/src/reactive/scope.rs`.
3. Add the `target-arch-placement` xtask check and wire it into the ladder.
4. Move both `web_sys` sites into `client`; drop `web-sys` + `macros` from
   `web`.
5. Delete the `macros::client_only` proc-macro and its identity test.
6. Delete the `#[component]`/`#[client_only]` coverage exemption and the inert
   `cov:ignore` markers.
7. Reconcile ADR-0050/0062/0069/0070 and `CONTRIBUTING.md`.

**Key risks / decisions:**

- **Task 4 is the risk.** If a leptos macro expands to an unqualified
  `web_sys::` path, dropping the dependency breaks the wasm build. `csr`
  compiles leptos views with no `web-sys` dependency, which is strong evidence
  it won't. If it does, stop and surface it — the fallback (keep `web-sys` with
  four features) reopens spec D1 and is not a call to take silently.
- **Task ordering is load-bearing.** Task 2 precedes Task 3 because the check
  would otherwise fail on `reactive.rs:64,81` and block the commit (the
  pre-commit hook runs the full `cargo xtask check`). Tasks 5–7 land last per
  the spec's sequencing constraint.
- **`client` code cannot be host-tested.** `client` is
  `#![cfg(target_arch = "wasm32")]`, so it is an empty rlib on the host and
  invisible to coverage. Task 4's primitives are verified by the wasm build,
  `wasm-clippy`, and the media-upload e2e — not by unit tests. This is
  ADR-0069's design, not a gap to paper over.
- **Recognition is anchored on the `cfg` attribute path, never on token text.**
  syn models `//!` and `///` as `#[doc = "…"]` attributes, and eight files in
  `web/src` quote the gate in prose. A token-text scan would flag every one of
  them.

## Global Constraints

- **No `Co-Authored-By` trailer** on any commit.
- **Run `cargo xtask check` before every commit** — the pre-commit hook runs the
  full gate (fmt + clippy + Nix coverage/tests); see `jaunder-commit`.
  `cargo xtask check` auto-fixes formatting, so re-check
  `git status --porcelain` after it goes green.
- **Never edit files during a gated commit** — Nix builds the working tree
  mid-commit; serialize edit → gate → commit.
- **New files must be `git add`ed before any Nix-backed gate**, even on a dirty
  tree — the flake ignores untracked files.
- **`web` lints deny `unwrap_used` / `expect_used`** (`web/Cargo.toml:78-79`).
- **Wasm clippy — use the gate's exact invocation**, copied from
  `xtask/src/steps/static_checks.rs:76-98`. Host `cargo check` does not lint
  wasm-only code, and a shortened command fails for unrelated reasons (without
  `--features csr`, `client::reactive` does not exist):

  ```bash
  cargo clippy -p web -p client -p csr --features csr --target wasm32-unknown-unknown \
    -- -D warnings -A clippy::too_many_arguments -A unfulfilled_lint_expectations
  ```

- **`xtask` is excluded from the workspace** — test it with
  `--manifest-path xtask/Cargo.toml`. It is also excluded from the Nix coverage
  source (`flake.nix:1180-1183`), so the new check module carries no
  coverage-gate obligation.
- **`leptosfmt` relocates comments inside `view!`** — put intent comments
  outside the macro.

---

### Task 1: File the separable concerns

Neither belongs to #520, and both should be pickable concurrently rather than
deferred behind this cycle.

**Files:** none (tracker only).

**Interfaces:**

- Consumes: nothing.
- Produces: two issue numbers, referenced in the spec's "Out of scope" section.

- [x] **Step 1: File the feature-gated-lint gap** → **#678**

Use `jaunder-issues`. Title:
`xtask: gate ladder never lints feature-gated code`. Body must state:
`xtask/src/steps/static_checks.rs:56` runs `clippy --all-targets -- -D warnings`
with **default features only**, so `web`'s `feature = "server"` paths are never
clippy-linted by `cargo xtask check` or `validate`. #520 verified `web`'s server
paths once by hand (its criterion A5) but the standing gap remains, and it
covers other crates too.

Record the constraint that makes this non-trivial — **as an open question, not a
fact**: a blanket `--all-features` would light `leptos/csr` and `leptos/ssr`
together. `client/Cargo.toml:20-23` claims leptos forbids that combination, but
#520 could not substantiate it (no `compile_error!` in leptos 0.8.20;
`leptos_macro`'s `csr = []` and `ssr = ["server_fn_macro/ssr"]` are not mutually
exclusive at the Cargo level). So the first step of that issue is to _settle_
whether `--all-features` actually breaks. If it does, the fix is a small matrix
of per-crate feature selections, e.g. `-p web --features server` plus the
existing `--features csr` wasm pass; if it doesn't, a blanket flag may suffice.
Cross-reference #677, which carries the same open question. Label `ci`.

- [x] **Step 2: File the stale planning-doc housekeeping** → **#679**

Filed. Two corrections to this step as written: it is **seven** files, not six
(#303 also left a spec, with no plan — it was an umbrella issue), and the label
is `documentation`, not `docs`. The issue also flags two undated design specs
(`2026-06-16-emacs-blogging-frontend-design.md`,
`2026-06-19-content-visibility-layer-c-design.md`) as needing a decision rather
than blind archiving — they may be living documents.

- [x] **Step 3: Record the issue numbers**

Recorded in the Review header. No commit yet — folded into Task 2's commit.

---

### Task 2: Split `invalidator_scope!` into a gated leaf file

Spec §D2, criteria B1–B4. This must land **before** Task 3, or the new check
fails on `reactive.rs` and blocks its own commit.

**Files:**

- Create: `web/src/reactive/mod.rs` — from `web/src/reactive.rs:1-46` (module
  doc + `Invalidator` + its `Default`) and the **whole** `tests` module
  (`:84-127`)
- Create: `web/src/reactive/scope.rs` — from `web/src/reactive.rs:48-82` (the
  macro, its docs, the re-export). **No tests.**

> **Deviation from the plan as approved, forced by `unused_imports` (denied).**
> Both macro tests live in `mod.rs`, not `scope.rs`. A test inside `scope.rs`
> reaches the macro by _textual_ scope, which leaves `scope.rs`'s own
> `pub(crate) use` with no consumer on the host test build; gating that `use`
> wasm-only would put a `target_arch` cfg inside a leaf file — precisely what
> Task 3 forbids. Putting the test in `mod.rs` and importing via
> `super::invalidator_scope` makes the whole re-export chain consumed on host
> **and** on wasm. Consequently both gate lines in `mod.rs` carry the _same_
> `any(target_arch = "wasm32", test)` gate, not the asymmetric pair the approved
> plan specified. No acceptance criterion changes: `scope.rs` still carries zero
> cfgs (B2), `scope_newtype_derefs_to_its_invalidator` still runs on host (B3),
> and `audiences/component.rs:13` is still untouched (B4).

- Delete: `web/src/reactive.rs`
- Unchanged (verify): `web/src/audiences/component.rs:13`

**Interfaces:**

- Consumes: nothing.
- Produces: `crate::reactive::Invalidator` (unchanged path) and
  `crate::reactive::invalidator_scope` (unchanged path, now via a re-export).

- [x] **Step 1: Create `web/src/reactive/scope.rs` with the macro and its test**

The file carries **no `target_arch` cfg** — its `mod` declaration supplies the
gate.

````rust
//! The `invalidator_scope!` context-scope newtype macro. Wasm-only by its `mod`
//! declaration in [`super`] (ADR-0070's file-level split, one level down), plus a
//! `test` arm so the generated newtype stays host-tested and coverage-measured
//! rather than exempted.

/// Declares a distinct context-scope newtype over an [`Invalidator`](super::Invalidator),
/// with `Deref` so the full `Invalidator` API is available on it. Use one per
/// **cross-component** refetch scope and `provide_context` / `expect_context` it, so
/// scopes never collide by type (a bare `Invalidator` in context would). A *local*
/// scope needs no newtype — a bare `Invalidator` suffices.
///
/// ```ignore
/// invalidator_scope! {
///     /// The audience-list refetch scope.
///     struct AudienceList
/// }
/// ```
macro_rules! invalidator_scope {
    ($(#[$meta:meta])* $vis:vis struct $name:ident) => {
        $(#[$meta])*
        #[derive(Clone, Copy)]
        $vis struct $name($vis $crate::reactive::Invalidator);

        impl ::core::ops::Deref for $name {
            type Target = $crate::reactive::Invalidator;
            fn deref(&self) -> &Self::Target {
                &self.0
            }
        }
    };
}

pub(crate) use invalidator_scope;

#[cfg(test)]
mod tests {
    use super::invalidator_scope;
    use crate::reactive::Invalidator;
    use leptos::reactive::owner::Owner;

    invalidator_scope! {
        /// Throwaway scope exercising the macro-generated newtype (`Deref` + `Copy`).
        struct TestScope
    }

    // The macro-generated newtype is trivial, pure code (`Deref` to the inner
    // `Invalidator` + `Copy`), so it is covered here rather than exempted.
    #[test]
    fn scope_newtype_derefs_to_its_invalidator() {
        let owner = Owner::new();
        owner.set();
        let scope = TestScope(Invalidator::new());
        let copied = scope; // Copy
        let v0 = scope.track(); // via Deref
        copied.notify(); // both wrap the same inner signal
        let v1 = scope.track();
        drop(owner);
        assert_ne!(v1, v0, "Deref reaches the inner Invalidator");
    }
}
````

- [x] **Step 2: Create `web/src/reactive/mod.rs`**

Carries `Invalidator` verbatim from `reactive.rs:11-46` and
`notify_changes_the_tracked_revision` verbatim from `reactive.rs:94-111`, plus
the gated `mod` + re-export. The module doc updates its "This module owns…"
sentence to mention the `scope` leaf.

**The two gate lines are deliberately asymmetric** — copy them exactly:

```rust
// The macro's consumers are wasm-only `component.rs` files; the `test` arm keeps the
// generated newtype host-tested. Gating the leaf here (not inside it) is ADR-0070's
// file-level split — `scope.rs` carries no cfg of its own.
#[cfg(any(target_arch = "wasm32", test))]
mod scope;
// Wasm-only, NOT `any(…, test)`: `scope.rs`'s own tests reach the macro through
// `use super::invalidator_scope`, so on a host test build this re-export would have no
// consumer and trip `unused_imports` (denied). The pre-split file gated it the same way
// for the same reason (`reactive.rs:79-82`).
#[cfg(target_arch = "wasm32")]
pub(crate) use scope::invalidator_scope;
```

Both lines are permitted form 2 under Task 3's rule (a `mod`/`use` item in a
`mod.rs`).

- [x] **Step 3: Delete `web/src/reactive.rs`**

```bash
git rm web/src/reactive.rs
git add web/src/reactive/mod.rs web/src/reactive/scope.rs
```

Stage the new directory now — the Nix-backed gate ignores untracked files.

- [x] **Step 4: Verify the consumer is untouched and both tests still run**

Run: `rg -n 'invalidator_scope' web/src/audiences/component.rs` Expected:
`13:use crate::reactive::{invalidator_scope, Invalidator};` — **unchanged**
(criterion B4).

Run: `cargo nextest run -p web reactive` Expected: PASS — both
`notify_changes_the_tracked_revision` and
`scope_newtype_derefs_to_its_invalidator` (criterion B3).

- [x] **Step 5: Verify the wasm target still builds**

Run the Global Constraints wasm-clippy command. Expected: PASS — the macro
resolves for `audiences/component.rs` through the re-export.

- [x] **Step 6: Commit** → `25f26d81`

Run `cargo xtask check` first; confirm `git status --porcelain` is clean after
its fmt auto-fix. Stage the spec and plan alongside the code — this is the
cycle's first commit, so the planning docs enter git here (`jaunder-ship`
archives them at the end).

```bash
git add web/src/reactive/ docs/superpowers/specs/2026-07-28-issue-520-endgame-ratchet.md docs/superpowers/plans/2026-07-28-issue-520-endgame-ratchet.md
git commit -m "refactor(web): move invalidator_scope! to a gated reactive leaf (#520)"
```

---

### Task 3: The `target-arch-placement` xtask check

Spec §D3, criteria C1–C6.

**Files:**

- Create: `xtask/src/steps/target_arch_placement_check.rs`
- Modify: `xtask/src/lib.rs:15-29` (module declaration), `:298` and `:330`
  (registration)

**Interfaces:**

- Consumes: nothing from earlier tasks.
- Produces:
  `pub fn violations(file_name: &str, src: &str) -> syn::Result<Vec<u32>>`
  (sorted 1-based lines),
  `pub fn problems(scanned: &[(String, String)]) -> Option<String>`,
  `pub fn run(result: &mut CommandResult)` — the same trio-shape as
  `no_full_reload_check`.

**Design:** compute the set of every line carrying a `target_arch` **cfg**
attribute, and the subset permitted by the three forms; violations are the
difference. Set-difference is used rather than an inline decision walk because
it makes "permitted" a positive, independently-testable predicate and cannot
double-report a line.

> **Note on step order:** the `pub mod target_arch_placement_check;` declaration
> in `xtask/src/lib.rs` had to move from Step 5 into Step 1 — without it the
> module is not compiled, so Step 2 would have reported "0 tests run" rather
> than a real failure. Step 5 therefore only adds the two `run(&mut result)`
> call sites.

- [x] **Step 1: Write the failing tests**

Add to `xtask/src/steps/target_arch_placement_check.rs`:

```rust
#[cfg(test)]
mod tests {
    use super::{problems, violations};

    #[test]
    fn lib_rs_crate_level_inner_attr_is_permitted() {
        // Form 1 — the whole-crate gate `client` and `csr` use.
        let src = "#![cfg(target_arch = \"wasm32\")]\npub mod storage;\n";
        assert!(violations("lib.rs", src).unwrap().is_empty());
    }

    #[test]
    fn inner_attr_outside_lib_rs_is_flagged() {
        let src = "#![cfg(target_arch = \"wasm32\")]\nfn a() {}\n";
        assert_eq!(violations("mod.rs", src).unwrap(), vec![1]);
    }

    #[test]
    fn gated_mod_and_use_in_mod_rs_are_permitted() {
        // Form 2 — the real `auth/mod.rs` shape (attribute on its own line).
        let src = "#[cfg(target_arch = \"wasm32\")]\nmod component;\n\
                   #[cfg(target_arch = \"wasm32\")]\npub use component::{LoginPage};\n";
        assert!(violations("mod.rs", src).unwrap().is_empty());
    }

    #[test]
    fn cfg_any_wasm_or_test_on_a_mod_is_permitted() {
        // The real `feed_discovery/mod.rs:8` shape — `any(...)` still counts as form 2.
        let src = "#[cfg(any(target_arch = \"wasm32\", test))]\nmod labels;\n";
        assert!(violations("mod.rs", src).unwrap().is_empty());
    }

    #[test]
    fn gated_use_in_a_leaf_file_is_flagged() {
        // Item-scope alone would pass this; the file-scope half is what catches it.
        let src = "#[cfg(target_arch = \"wasm32\")]\npub(crate) use foo;\n";
        assert_eq!(violations("reactive.rs", src).unwrap(), vec![1]);
    }

    #[test]
    fn gated_fn_in_a_wiring_file_is_flagged() {
        // File-scope alone would pass this; the item-scope half is what catches it.
        let src = "#[cfg(target_arch = \"wasm32\")]\nfn helper() {}\n";
        assert_eq!(violations("mod.rs", src).unwrap(), vec![1]);
    }

    #[test]
    fn gated_macro_rules_is_flagged() {
        let src = "#[cfg(any(target_arch = \"wasm32\", test))]\nmacro_rules! m { () => {} }\n";
        assert_eq!(violations("reactive.rs", src).unwrap(), vec![1]);
    }

    #[test]
    fn cfg_on_a_statement_inside_a_body_is_flagged() {
        let src = "fn a() {\n    #[cfg(target_arch = \"wasm32\")]\n    let x = 1;\n}\n";
        assert_eq!(violations("mod.rs", src).unwrap(), vec![2]);
    }

    #[test]
    fn doc_comments_quoting_the_gate_are_not_flagged() {
        // syn models `//!` and `///` as `#[doc = "…"]` attributes, so a token-text
        // scan would flag both of these. Recognition is anchored on the `cfg` path.
        // These are the real `auth/mod.rs:9` and `media/component.rs:2` shapes, and
        // they are checked in NON-`lib.rs` files, where form 1 cannot mask the bug.
        let inner = "//! The UI is wasm-only (`#[cfg(target_arch = \"wasm32\")]`).\nmod api;\n";
        assert!(violations("mod.rs", inner).unwrap().is_empty());
        let outer = "/// Declared `#[cfg(target_arch = \"wasm32\")] mod component;`.\n\
                     pub fn f() {}\n";
        assert!(violations("component.rs", outer).unwrap().is_empty());
    }

    #[test]
    fn cfg_attr_carrying_target_arch_is_recognized() {
        let src = "#[cfg_attr(target_arch = \"wasm32\", allow(dead_code))]\nfn a() {}\n";
        assert_eq!(violations("mod.rs", src).unwrap(), vec![1]);
    }

    #[test]
    fn non_target_arch_cfgs_are_ignored() {
        // The check polices the host/wasm boundary only — `feature`/`test` gates on
        // any item are none of its business.
        let src = "#[cfg(feature = \"csr\")]\npub fn f() {}\n#[cfg(test)]\nmod t {}\n";
        assert!(violations("dom.rs", src).unwrap().is_empty());
    }

    #[test]
    fn pre_fix_reactive_shape_is_reported() {
        // Criterion C5 — the exact shape Task 2 removed must be caught.
        let src = "#[cfg(any(target_arch = \"wasm32\", test))]\n\
                   macro_rules! invalidator_scope { () => {} }\n\
                   #[cfg(target_arch = \"wasm32\")]\n\
                   pub(crate) use invalidator_scope;\n";
        assert_eq!(violations("reactive.rs", src).unwrap(), vec![1, 3]);
    }

    #[test]
    fn unparseable_file_is_an_error_not_a_silent_pass() {
        assert!(violations("mod.rs", "fn (").is_err());
    }

    #[test]
    fn problems_reports_a_parse_failure_rather_than_passing_silently() {
        let detail = problems(&[("web/src/broken.rs".to_string(), "fn (".to_string())])
            .expect("a problem");
        assert!(detail.contains("web/src/broken.rs"));
        assert!(detail.contains("parse"));
    }

    #[test]
    fn problems_reports_path_line_and_the_rule() {
        let detail = problems(&[(
            "web/src/reactive.rs".to_string(),
            "#[cfg(target_arch = \"wasm32\")]\npub(crate) use foo;\n".to_string(),
        )])
        .expect("a problem");
        assert!(detail.contains("web/src/reactive.rs:1"));
        assert!(detail.contains("mod.rs"));
    }

    #[test]
    fn clean_tree_reports_none() {
        assert_eq!(
            problems(&[(
                "web/src/auth/mod.rs".to_string(),
                "#[cfg(target_arch = \"wasm32\")]\nmod component;\n".to_string()
            )]),
            None
        );
    }
}
```

- [x] **Step 2: Run the tests, verify they fail**

Run: `cargo nextest run --manifest-path xtask/Cargo.toml target_arch_placement`
Expected: FAIL — `violations` / `problems` not defined. Confirmed:
`E0432 unresolved imports super::problems, super::violations`.

- [x] **Step 3: Implement against the tests**

> **Implementation note:** `xtask` has no `quote` dependency, so
> `to_token_stream()` is unavailable. `is_target_arch_cfg` instead reads
> `syn::Meta::List`'s `tokens` via `proc_macro2::TokenStream`'s `Display` — same
> effect (handles `any`/`all`/`not` nesting), no new dependency.

Write the module to these signatures:

```rust
pub fn violations(file_name: &str, src: &str) -> syn::Result<Vec<u32>>
pub fn problems(scanned: &[(String, String)]) -> Option<String>
pub fn run(result: &mut CommandResult)
```

Recognition predicate — **anchored on the attribute path, never on token text**:

```rust
/// True for `#[cfg(…)]` / `#[cfg_attr(…)]` whose arguments mention `target_arch`.
/// The path anchor is load-bearing: syn models `//!` and `///` as `#[doc = "…"]`
/// attributes, so a bare token-text scan flags the eight `web/src` module docs that
/// quote the gate in prose.
fn is_target_arch_cfg(attr: &syn::Attribute) -> bool {
    (attr.path().is_ident("cfg") || attr.path().is_ident("cfg_attr"))
        && attr.to_token_stream().to_string().contains("target_arch")
}
```

`violations` parses with `syn::parse_file`, propagating the parse error (a
`syn::Result`, so an unparseable file is a hard failure — **not** the
fail-closed "nothing exempt" posture `coverage/exempt.rs` uses; here silence
would disable the guard). It returns the sorted difference of two line sets:

- **all** — every line spanned by an `is_target_arch_cfg` attribute, gathered by
  a `syn::visit::Visit` over the whole file plus `file.attrs` (so attributes on
  statements and expressions inside bodies are included).
- **permitted** — `file.attrs` lines when `file_name == "lib.rs"` (form 1),
  plus, when `file_name` is `"lib.rs"` or `"mod.rs"`, the attribute lines of
  every `syn::Item::Mod` / `syn::Item::Use`, including inside inline
  `mod x { … }` blocks (form 2).

Every branch is pinned by a Step 1 test: form 1 accepted and rejected by file
name, form 2 accepted for `mod`/`use` and rejected for `fn`/`macro_rules!`,
leaf-file rejection, in-body statements, `cfg_attr`, non-`target_arch` cfgs, doc
comments in both inner and outer form, and the parse error.

`problems` mirrors `no_full_reload_check.rs:45-56`, with one addition: a
`violations(...)` `Err` becomes its own reported line —
`"{path}: cannot parse — the target_arch placement guard cannot verify this file: {e}"`
— so a file that stops parsing fails loudly instead of dropping out of the scan.
`run` mirrors `no_full_reload_check.rs:74-97` exactly, including the hard
failure on an unreadable root. `POLICED_ROOTS` is
`["web/src", "client/src", "csr/src"]`. The violation detail names the file, the
line, and the rule:
`` `target_arch` is permitted only on a `mod`/`use` item in a `mod.rs`/`lib.rs`, or as a crate-level `#![cfg(…)]` in `lib.rs` (#520) ``.

Pass the file's terminal component to `violations` via
`path.file_name().and_then(|n| n.to_str())`.

- [x] **Step 4: Run the tests, verify they pass**

Run: `cargo nextest run --manifest-path xtask/Cargo.toml target_arch_placement`
Expected: PASS — 16 tests. Confirmed: 16/16.

- [x] **Step 5: Wire it into the ladder**

`xtask/src/lib.rs`: add `pub mod target_arch_placement_check;` to the `steps`
block — alphabetically **last**, after `test_pattern_check` (`:29`) — and
`steps::target_arch_placement_check::run(&mut result);` after
`steps::no_full_reload_check::run(&mut result);` at **both** `:298` (Check) and
`:330` (Validate).

- [x] **Step 6: Verify it passes on the real tree** —
      `[ ok ] target-arch-placement`, and no module-doc false positives (the
      `cfg`-path anchor holds).

**Also proved it bites on the real scan, not just in unit tests.** A check whose
`rust_files` walk found nothing would report `ok` too, so a throwaway
`web/src/_tmp_probe.rs` containing a gated `fn` was planted:
`cargo xtask check --no-test` exited 1 with
`[FAIL] target-arch-placement — web/src/_tmp_probe.rs:1: …(#520)`. Probe deleted
and un-staged (the git-add hook had auto-staged it).

Run: `cargo xtask check --no-test` Expected: PASS, with a
`target-arch-placement` step in `jq '.steps' .xtask/last-result.json`. Criterion
C6.

**If this reports the `web/src` module docs** (`auth/mod.rs:9`,
`registration/mod.rs:12`, `profile/mod.rs:8`, `sessions/mod.rs:9`, and the four
`component.rs` headers), the `is_target_arch_cfg` path anchor was dropped — fix
the predicate, do not add an allowlist.

- [ ] **Step 7: Commit**

```bash
git add xtask/src/steps/target_arch_placement_check.rs xtask/src/lib.rs
git commit -m "feat(xtask): enforce target_arch placement in web/client/csr (#520)"
```

---

### Task 4: Move both `web_sys` sites to `client`; drop `web-sys` + `macros`

Spec §D1, criteria A1–A6. **The risky task** — see the Review header.

**Files:**

- Create: `client/src/upload.rs`
- Modify: `client/src/dom.rs` (append one primitive), `client/src/lib.rs`
  (declare `upload`), `client/Cargo.toml` (add `server_fn`, extend `web-sys`
  features)
- Modify: `web/src/media/component.rs:42-100,126-146`, `web/Cargo.toml:27,36-45`
- Modify: `web/src/tags/component.rs:7`, `web/src/tags/input_state.rs:5` (stale
  prose)

**Interfaces:**

- Consumes: nothing from earlier tasks.
- Produces:
  - `client::upload::picked_file_multipart(input: leptos::prelude::NodeRef<leptos::html::Input>) -> Option<server_fn::codec::MultipartData>`
  - `client::dom::select_event_target_text(ev: &web_sys::Event)`

`web` never names a `web_sys` type at either call site: the first takes a leptos
`NodeRef`, and the second's argument is the closure parameter leptos already
infers.

- [ ] **Step 1: Add the `server_fn` dependency to `client`**

`client/Cargo.toml` — make it explicit rather than inheriting the `multipart`
feature from `web`'s manifest via cargo feature unification (criterion A4):

```toml
# `MultipartData` sits behind server_fn's `multipart` feature. `client` names it in
# `upload::picked_file_multipart`, so it declares the feature itself rather than
# relying on `web`'s manifest lighting it through feature unification.
server_fn = { workspace = true, features = ["multipart"], optional = true }
```

and extend the `csr` feature:
`csr = ["dep:leptos", "leptos/csr", "dep:server_fn"]`.

Extend the existing `web-sys` feature list (`client/Cargo.toml:9-17`) with the
five newly-needed features — `File`, `FileList`, `FormData`, `HtmlInputElement`,
`Event` — alongside the current
`Window`/`Storage`/`Location`/`Document`/`Element`/`Node`/`NodeList`.
(`EventTarget` is already implied by `Element`; add it if the build asks.)

Rewrite the comment at `:20-23` (criterion A7). It currently justifies the
optional `leptos` dep with "which leptos forbids" — a claim #520 could not
substantiate (no `compile_error!` in leptos 0.8.20; `leptos_macro`'s `csr = []`
and `ssr = ["server_fn_macro/ssr"]` are not mutually exclusive at the Cargo
level). Replace it with the evidenced reason, keeping the dep optional either
way:

```toml
# Optional + behind the `csr` feature so a normal (host / server) build never pulls leptos:
# `web`'s server build pulls `leptos_axum`, which hard-requires `leptos/ssr`
# (leptos_axum-0.8.10/Cargo.toml:74-79), so an unconditional dep here would risk
# unifying `leptos/csr` into that same build. Whether leptos actually rejects the pair
# is an open question (#677). `web`'s `csr` feature forwards `client/csr` (#515).
```

- [ ] **Step 2: Write `client/src/upload.rs`**

```rust
//! Browser file-picker → multipart upload glue (#520). Raw browser API access plus
//! the `server_fn` transport type it produces; no domain types (ADR-0069). Relocated
//! from `web::media::component` so `web` carries no `web-sys` dependency at all.

use leptos::html::Input;
use leptos::prelude::NodeRef;
use server_fn::codec::MultipartData;

/// The first file currently chosen in `input`, wrapped as multipart form data under
/// the field name `file`. `None` when the ref is unmounted, no file is chosen, or the
/// browser refuses to build the `FormData`.
#[must_use]
pub fn picked_file_multipart(input: NodeRef<Input>) -> Option<MultipartData> {
    let el = input.get()?;
    let file = el.files()?.get(0)?;
    let form_data = web_sys::FormData::new().ok()?;
    form_data.append_with_blob("file", &file).ok()?;
    Some(form_data.into())
}
```

No `JsCast` cast: `NodeRef<Input>::get()` already yields
`web_sys::HtmlInputElement` (tachys's `ElementType::Output`), so the
`unchecked_into()` at the current `web/src/media/component.rs:57` is a no-op
identity cast and is not carried over.

`client/src/lib.rs` gains the declaration:

```rust
/// Browser file-picker → `MultipartData` glue (#520), behind `csr` because it needs
/// leptos's `NodeRef`.
#[cfg(feature = "csr")]
pub mod upload;
```

- [ ] **Step 3: Append the select primitive to `client/src/dom.rs`**

Typed `&web_sys::Event`, **not** `leptos::ev::MouseEvent` — that keeps `dom.rs`
leptos-free and ungated alongside its three existing primitives, and
`&MouseEvent` coerces to `&Event` through web_sys's Deref-based inheritance at
the call site. `dom.rs` already imports `JsCast` at `:7`.

```rust
/// Select the full text of the `<input>` that raised `ev`; no-op when the event has no
/// target or the target is not an input element. Click-to-select readonly fields (#520).
pub fn select_event_target_text(ev: &web_sys::Event) {
    if let Some(input) = ev
        .target()
        .and_then(|t| t.dyn_into::<web_sys::HtmlInputElement>().ok())
    {
        input.select();
    }
}
```

Do **not** put `dom` behind `csr` — `csr/src/lib.rs:31,36,40` consume its three
existing primitives with no feature selection (`csr/Cargo.toml:12`).

- [ ] **Step 4: Rewrite the two `web` call sites**

`web/src/media/component.rs:48-70` collapses to:

```rust
let on_file_change = move |_| {
    use leptos::task::spawn_local;

    let Some(form_data) = client::upload::picked_file_multipart(file_input) else {
        return;
    };
    uploading.set(true);
    // … the existing spawn_local block from :74-100, with `upload_media(form_data)`
```

Note `upload_media(form_data)` — no `.into()`, since the primitive already
returns `MultipartData`. And `:136-142` collapses to
`on:click=move |ev| client::dom::select_event_target_text(&ev)`.

- [ ] **Step 5: Drop the dependencies**

`web/Cargo.toml`: delete `macros = { path = "../macros" }` (`:27`) and the whole
`web-sys = { … }` block (`:36-45`).

Also delete `"leptos/ssr",` from the `server` feature (`:66`) — criterion A7.
This is **behaviour-neutral**: `leptos_axum` hard-requires `leptos/ssr`
(`leptos_axum-0.8.10/Cargo.toml:74-79`), so feature unification supplies it
anyway. Removing it stops the manifest implying an SSR mode we don't have
(#487). If the server build breaks in Step 7, that premise was wrong — restore
the line and report, rather than chasing it; actually shedding the SSR stack is
#677's job.

- [ ] **Step 6: Verify no `web_sys` path survives in `web`**

Run: `rg -n 'web_sys::' web/src` Expected: **no matches** (criterion A2).

Then correct the two now-false prose claims (criterion A2):
`tags/component.rs:7` and `tags/input_state.rs:5` each assert an "irreducible
`web_sys` event touch stays inline in the component" — no such touch remains;
both use leptos's `event_target_value`. `web/src/lib.rs:46`'s "(no `web_sys`)"
stays — still true.

- [ ] **Step 7: Verify the build, the lints, and the browser**

Run the Global Constraints wasm-clippy command. Expected: PASS. **This is the
step that settles the plan's key risk** — a failure here naming an unqualified
`web_sys::` path from a leptos expansion means stop and surface it, per the
Review header. A failure naming `client::reactive` instead means the command was
shortened; use the full one.

Run: `cargo clippy -p web --features server --all-targets -- -D warnings`
Expected: PASS (criterion A5 — lints `web`'s server-gated paths, which the
ladder never does). This run **also proves criterion A7**: it is the server
build, so it fails if dropping `"leptos/ssr"` was not the no-op we expect. Do
not substitute `--all-features` — it would light `leptos/csr` alongside
`leptos/ssr`, whose safety is exactly the unsettled question #677 carries.

Run: `cargo xtask e2e-local media` Expected: PASS — exercises both relocated
primitives (criterion A6, ~3 min).

- [ ] **Step 8: Commit**

```bash
git add client/ web/Cargo.toml web/src/media/component.rs web/src/tags/component.rs web/src/tags/input_state.rs
git commit -m "refactor(web): move file-picker + select glue to client; drop web-sys and macros deps (#520)"
```

---

### Task 5: Delete the `client_only` proc-macro

Spec criterion D1. Lands after Task 4, which removed `web`'s `macros`
dependency.

**Files:**

- Modify: `macros/src/lib.rs:12-23`
- Delete: `macros/tests/identity.rs`

**Interfaces:**

- Consumes: `web` no longer depends on `macros` (Task 4, Step 5).
- Produces: nothing. `macros` retains only the three newtype derives.

- [ ] **Step 1: Confirm there are no tenants left**

Run: `rg -n 'client_only' --glob '!docs/**' .` Expected: only
`macros/src/lib.rs`, `macros/tests/identity.rs`,
`xtask/src/coverage/{mod,exempt}.rs`, `web/src/forms/field.rs:200`, and
`client/src/lib.rs:11` — all prose or xtask recognition, removed in Task 6. **No
live attribute use.**

- [ ] **Step 2: Delete the macro and its test**

Remove `macros/src/lib.rs:12-23` (the doc comment and the `client_only` fn).
Keep `use proc_macro::TokenStream;` — the three derives still take it.

```bash
git rm macros/tests/identity.rs
```

- [ ] **Step 3: Verify the crate still builds and its tests pass**

Run: `cargo nextest run -p macros` Expected: PASS — the ~40 derive tests in
`macros/src/lib.rs:276-688`, unaffected.

- [ ] **Step 4: Commit**

```bash
git add macros/
git commit -m "refactor(macros): retire the client_only identity attribute (#520)"
```

---

### Task 6: Delete the coverage exemption and the inert `cov:ignore` markers

Spec criteria D2–D6. The retirement the spec's sequencing constraint puts last.

**Files:**

- Modify: `xtask/src/coverage/exempt.rs` (module docs `:1-32`, visitor arms
  `:56-68`, helpers `:87-116`, tests)
- Modify: `xtask/src/coverage/mod.rs:5,8,49,96,179,190,220`
- Modify: `web/src/backup/component.rs` (delete `:50` and `:184`)
- Modify: `web/src/forms/field.rs:198-200`, `client/src/lib.rs:11`

**Interfaces:**

- Consumes: no `#[client_only]` attribute exists (Task 5).
- Produces: `exempt_lines` keeps its signature
  `pub fn exempt_lines(src: &str) -> syn::Result<BTreeSet<u32>>`; only the
  `unreachable!("msg")` rule remains behind it.

- [ ] **Step 1: Delete the exemption arms**

From `xtask/src/coverage/exempt.rs` remove: `visit_item_fn` (`:56-59`),
`visit_impl_item_fn` (`:61-68`), `has_exempt_attr` (`:87-94`), and
`exempt_marked_fn` (`:96-116`). `visit_macro` (`:78-84`) and `add_span`
(`:118-123`) stay — they are the `unreachable!` rule. Rewrite the module docs
(`:1-32`) to describe **one** recognized construct; the "no standalone `view!`
rule" paragraph (`:30-32`) becomes moot with components gone and goes too.

Delete the tests that exercise the removed rules — every test naming `component`
or `client_only`, including `exempts_plain_component_body` (`:129`),
`exempts_client_only_method` (`:353`), `exempts_client_only_free_fn` (`:379`),
and `does_not_exempt_non_ident_client_only_path` (`:408`). Keep every
`unreachable!` test.

- [ ] **Step 2: Update the gate's diagnostics**

`xtask/src/coverage/mod.rs` — the messages must stop naming removed machinery:

- `:179` → `"\n  uncovered (not an unreachable!(\"msg\"), not cov:ignore'd):"`
- `:190` → `"\n  A1-guard — covered line inside an unreachable! span:"`
- `:218-224` → the recovery text keeps only the `unreachable!` arm: an
  `unreachable!` assertion was actually reached, so revisit it.
- `:5`, `:8`, `:49`, `:96` — module docs and field docs lose their
  `#[component]`/`#[client_only]` mentions.

The A1 guard itself is **retained** (criterion D3) — it still guards the
`unreachable!` exemption.

- [ ] **Step 3: Delete the inert markers and the stale prose**

`web/src/backup/component.rs`: delete the `// cov:ignore-start` (`:50`) and
`// cov:ignore-stop` (`:184`) lines. They are inert — the file is wasm-only by
its `mod` declaration, so it never host-compiles.

`web/src/forms/field.rs:198-200`: the comment explains `Field<T>` is host-tested
"like `Invalidator::{new, notify, track}` … not `#[client_only]`-exempted".
Reword to drop the dead attribute; the host-tested-under-an-`Owner` rationale
stays.

`client/src/lib.rs:11`: "needs no per-item `#[cfg]` and no `#[client_only]`
marker" — drop the second clause; the per-item-`#[cfg]` point stands.

- [ ] **Step 4: Verify the retirement is complete**

Run: `rg client_only web/src macros/src xtask/src client/src` Expected: **no
matches** (criterion D2, plus the `client` prose).

Run: `rg '#\[component\]' xtask/src/coverage/` Expected: **no matches**
(criterion D2).

Run: `rg 'cov:ignore' web/src` Expected: **no matches** (criterion D4).

- [ ] **Step 5: Verify xtask's own tests and the coverage gate**

Run: `cargo nextest run --manifest-path xtask/Cargo.toml coverage` Expected:
PASS — the surviving `unreachable!` and `cov:ignore` tests.

Run: `cargo xtask check` Expected: PASS **including the Nix coverage step** —
this is criterion D5, the empirical proof the exemption was dead machinery. A
failure here means some `#[component]` body _was_ host-compiled after all; stop
and report rather than re-adding the exemption.

- [ ] **Step 6: Commit**

```bash
git add xtask/src/coverage/ web/src/backup/component.rs web/src/forms/field.rs client/src/lib.rs
git commit -m "refactor(xtask): retire the #[component]/#[client_only] coverage exemption (#520)"
```

---

### Task 7: Reconcile the ADRs and CONTRIBUTING

Spec criteria E1–E5. No new ADR (spec §D4) — these are amendments to existing
records.

**Files:**

- Modify: `docs/adr/0050-stateless-coverage-gate.md` (`:1`, `:6-8`, `:40`,
  `:51-60`, `:62`, `:78-79`, `:123`, `:130-139`, `:160`)
- Modify: `docs/adr/0062-macros-crate-proc-macro-home.md:17,49,80`
- Modify: `docs/adr/0069-client-crate-wasm-only-home.md`
- Modify: `docs/adr/0070-web-vertical-wasm-only-component-files.md` (§6,
  `:119-124`)
- Modify: `CONTRIBUTING.md:27,420,422-435,436-447,508-525,533-535`

**Interfaces:**

- Consumes: every code change from Tasks 2–6.
- Produces: nothing consumed by later tasks.

- [ ] **Step 1: Amend ADR-0050**

Its title (`:1`), the header amendment note (`:6-8`, which counts `unreachable!`
as "a **third** structural exemption" — arithmetically wrong once Decision 1
goes), Decision 1 (`:51-60`), the A1-guard text (`:78-79`), and the consequences
(`:123`, `:130-139`, `:160`) all describe the `#[component]` exemption as live.
Add a dated amendment note recording that Decision 1 retired in #520 — **not**
because the gate's architecture changed (stateless, marker-based, CRAP threshold
all stand) but because components no longer host-compile at all, so there is
nothing left to exempt. Cite ADR-0070:129-131, which already foreshadows exactly
this ("component lines leave the host denominator entirely — not-compiled beats
measured-but-exempt"). The retained `unreachable!` rule and the A1 guard stay
described as current.

- [ ] **Step 2: Amend ADR-0062**

`:17`, `:49` ("Its **first tenant** is `#[client_only]`"), and `:80` describe a
tenant that no longer exists. Record that it retired in #520 per its own interim
charter ("interim until wasm-bindgen-test can cover these in a headless
browser"), and that the crate's remaining tenants are the three newtype derives.
The ADR's actual decision — a target-agnostic proc-macro crate exists — is
unchanged.

- [ ] **Step 3: Amend ADR-0069**

Clarify that "never our domain types" means _ours_: framework transport such as
`server_fn`'s `MultipartData` is admissible, on the same footing as the `leptos`
dependency the crate already carries behind `csr`. Cite `client::upload` as the
instance (criterion E3).

- [ ] **Step 4: Amend ADR-0070**

Three edits (criterion E2): record that the file-level split is now
machine-enforced by the `target-arch-placement` check, in the three forms of
spec §D3; update §6's "rather than hiding behind the `#[component]` exemption"
phrasing, since no exemption remains — the argument for extracting logic into
host-tested files is now unconditional; and clarify at `:119-124` that
"**cross-vertical** browser primitives" describes `client`'s typical tenant
rather than an admission test, so a single-vertical primitive that is genuinely
raw browser glue (`client::upload`) belongs there. The operative test is
domain-freedom.

- [ ] **Step 5: Update CONTRIBUTING.md**

Four precise edits — the ranges matter, because two adjacent bullets have
opposite fates:

- `:420` — "unless one of **three** things exempts it" becomes **two**.
- `:422-435` — the `#[component]` structural-exemption bullet: **delete**.
- `:436-447` — the `unreachable!` bullet: **retain**, but `:443`'s "and, like
  `#[component]`, **fail-closed**" must lose the comparison to a rule that no
  longer exists.
- `:508-525` — "Component bodies are weaker" and its invariant tripwire:
  **delete, not soften**. The tradeoff no longer applies, because component
  lines are not in the host denominator at all.
- `:27` and `:533-535` — the four-file layout description still asserts
  `#[component]` bodies are "**structurally exempt** (no marker needed)".
  Reword: they are not-compiled on the host, which is why no marker is needed.

- [ ] **Step 6: Verify no doc describes removed machinery**

Run:
`rg -n 'client_only|structurally exempt|component.*exemption' CONTRIBUTING.md docs/adr/ client/src web/src`
Expected: matches only inside the historical/amendment notes added above, never
as a statement of current policy. Read each hit to confirm (criterion E5).

Run: `cargo xtask check` Expected: PASS — `adr_check` validates ADR structure
and the README table.

- [ ] **Step 7: Commit**

```bash
git add CONTRIBUTING.md docs/adr/
git commit -m "docs(adr): record the client_only + #[component] exemption retirement (#520)"
```

---

## Final verification

- [x] **Full local gate**

Run: `cargo xtask validate --no-e2e` Expected: PASS. Run in the **foreground**
with `timeout: 600000` — a backgrounded coverage rebuild gets killed.

- [x] **Spec conformance sweep**

Walk the spec's criteria A1–A7, B1–B4, C1–C6, D1–D6, E1–E5 and confirm each. The
`rg`-based ones (A2, D2, D4) are mechanical; A5, A6, A7, C6 and D5 are the five
that require an actual run.
