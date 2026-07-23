# Spec — Move the CSR boot (`mount_csr` + `read_dom_seed`) out of `web` (#519)

## Summary

The CSR boot lives in `web/src/lib.rs:62-103` — `mount_csr` (gated
`#[cfg(feature = "csr")]` + inner `#[cfg(target_arch = "wasm32")]`, carrying
`cov:ignore`) and `read_dom_seed`. `mount_csr` adopts the projector's DOM blob
(#178/ #179): reads `#jaunder-seed`, removes the server-painted `#app`, and
`mount_to_body`s `App` with the seed in context. `read_dom_seed` mixes a
testable JSON deserialize with `web_sys` DOM traversal. This relocates the boot
to inherently-wasm homes, leaving `web/src/lib.rs` free of `feature = "csr"`
item gates and `cov:ignore`.

## Design (resolved)

Three-way split:

1. **`client::dom` (new module) — generic DOM primitives.** The wasm-only
   `client` crate (`#![cfg(target_arch = "wasm32")]`, ADR-0069) gains a `dom`
   module of raw `web_sys` free functions carrying **no domain types** (the
   `storage`/`navigation`/ `dialog` precedent):
   - `text_content_by_id(id: &str) -> Option<String>` —
     `window → document → get_element_by_id(id) → text_content`.
   - `remove_element_by_id(id: &str)` — removes the element if present; no-op if
     absent (subsumes `mount_csr`'s current `if let Some(el)` guard).

   Unconditional within `client` (uses only `web_sys`, no leptos) — like
   `navigation`/ `dialog`, not behind the `csr` feature.

2. **`web::render` stays untouched — `csr` parses the existing `PageSeed`
   inline.** `web::render` is being eliminated (#312), and `PageSeed`'s final
   home is a separate, cross-cutting question filed as **#610** (move
   `PageSeed` + the public-surface wire DTOs to `common`). So #519 adds
   **nothing** to `web::render`: the boot deserializes the already-public type
   directly — `serde_json::from_str::<web::render::PageSeed>(&json).ok()`. The
   typed decode stays covered by `web::render`'s existing `PageSeed` serde
   round-trip test; when #610 relocates `PageSeed`, `csr`'s one import line
   follows it.

3. **`csr` — the boot.** `mount_csr`'s logic moves into the `csr` crate (which
   already owns `web::App`, the domain type `client` must not know): read the
   seed via `client::dom::text_content_by_id("jaunder-seed")` +
   `serde_json::from_str:: <web::render::PageSeed>`, drop `#app` via
   `client::dom::remove_element_by_id("app")`, then
   `leptos::mount::mount_to_body` `App` with the seed in context. The `csr`
   crate becomes **crate-level wasm-only** (`#![cfg(target_arch = "wasm32")]`),
   so on the host it is an empty rlib — the boot needs **no `cov:ignore`** and
   `csr` stops host-compiling wasm-only dead code. `csr` gains `client` and
   `serde_json` dependencies (`serde_json` is already in the wasm bundle via
   `web`, so no size change).

## In scope

- New `client::dom` module (two primitives).
- Boot relocated into `csr`; `csr` made crate-level wasm-only; `csr` gains
  `client` and `serde_json` deps. `web::render` is **not** touched.
- `client/Cargo.toml` gains the `web-sys` features the primitives force —
  `Document`, `Element`, `Node` (it currently enables only
  `Window`/`Storage`/`Location`).
- **Extend `wasm-clippy` to lint `-p csr`.** Because `csr` becomes wasm-only
  (empty on host), the host `clippy --all-targets` step no longer lints it, and
  `wasm-clippy` today lints only `-p web -p client` — so the relocated boot (the
  one piece of new logic) would be linted by **nothing** (`cargo build -p csr`
  catches compile errors, not lints). Add `-p csr` to `wasm-clippy` in both
  homes it is declared, exactly as ADR-0069 did when it added `-p client`.
- `web/src/lib.rs` stripped of `mount_csr`, `read_dom_seed`, the now-dead gated
  `use leptos::prelude::*`, its `feature = "csr"` item gate, and its
  `cov:ignore`.

## Out of scope

- The `pub use pages::App` wasm gate — untouched; `App` moves with #330 and
  stays wasm-gated in its new home.
- Any other `web` module or vertical (#526 file-split of other verticals is
  separate).
- `PageSeed`'s eventual relocation to `common` (#610) — `csr` reads the existing
  `web::render::PageSeed` type as-is; the follow-on move only changes one import
  later.
- #520 endgame (dropping js-sys/wasm-bindgen, retiring `client_only`).

## Acceptance criteria

Stated so ship-time conformance review can tell delivered from not.

1. **AC1 — `web/src/lib.rs` no longer carries the boot.** `mount_csr` and
   `read_dom_seed` are gone from `web/src/lib.rs`; a search
   (`rg 'mount_csr|read_dom_seed|cov:ignore|feature = "csr"' web/src/lib.rs`)
   returns no matches. The only `target_arch` gates left in the file are on
   `mod`-declaration / re-export lines
   (`#[cfg(target_arch = "wasm32")] pub mod pages;` and `pub use pages::App;`),
   which are unchanged.

2. **AC2 — `client::dom` provides the two primitives.** `client/src/dom.rs`
   defines `text_content_by_id(&str) -> Option<String>` and
   `remove_element_by_id(&str)` using only `web_sys`, with no leptos/domain
   types, declared `pub mod dom;` in `client/src/lib.rs` (unconditional, no
   `csr` feature gate).

3. **AC3 — the seed parse is typed and `web::render` is untouched.** #519 adds
   no new item to `web/src/render/mod.rs` (its `git diff` is empty). The `csr`
   boot deserializes the existing `web::render::PageSeed` via
   `serde_json::from_str`; the typed decode stays covered by `web::render`'s
   existing `PageSeed` serde round-trip test. No `web_sys` DOM traversal for
   reading the seed remains anywhere in `web` (it moved to `client::dom`).
   `PageSeed`'s eventual relocation is #610, out of scope here.

4. **AC4 — the boot lives in `csr`, which is wasm-only.** `csr/src/lib.rs`
   begins with `#![cfg(target_arch = "wasm32")]`; the boot (read seed → drop
   `#app` → mount `App` with seed context) is implemented there via
   `client::dom` + `serde_json::from_str::<web::render::PageSeed>`; the file
   contains no `cov:ignore`. `csr/Cargo.toml` depends on `client` and
   `serde_json`.

5. **AC5 — behavior is preserved.** The two boot paths are unchanged: (a) with
   the projector blob present, the seed is adopted (context-provided) and `#app`
   is removed before mount; (b) on the static SPA shell (no blob, no `#app`),
   the seed is `None` and it is an ordinary `mount_to_body`. `mark_ready()`
   still fires after mount. Verified by the existing e2e matrix, which exercises
   both paths incidentally: the seed-adoption path via the anon public-`/` specs
   (e.g. `authed-flash.spec.ts` "anonymous: / has no authed sidebar chrome") and
   the static-shell path via the authed drafts/composer specs (`posts.spec.ts`).
   Every page load runs the boot and waits on `body[data-hydrated]` (set by
   `mark_ready`), so a broken boot fails the suite.

6. **AC6 — `wasm-clippy` lints the relocated boot.** After the change, the
   `wasm-clippy` step lints `-p csr` in addition to `-p web -p client`: the
   `StepSpec` in `xtask/src/steps/static_checks.rs` and the crane `wasm-clippy`
   derivation in `flake.nix` both include `-p csr`, and the `static_checks` unit
   test asserting the `wasm-clippy` package set is updated (renamed)
   accordingly. Running `wasm-clippy` over the new `csr` boot passes
   `-D warnings`.

7. **AC7 — the gate is green.** `cargo xtask validate` (incl. the e2e matrix and
   `wasm-clippy`) passes; coverage is clean with no new `cov:ignore` markers and
   no regression.

## Non-goals / explicitly not added

- No change to `PageSeed` at all — `web::render` is untouched; the boot
  deserializes the existing type inline. `PageSeed`'s relocation is #610.
- No new e2e test — the existing seed-adoption + static-shell specs already
  exercise both boot paths (AC5); this is a pure relocation with identical
  behavior.
