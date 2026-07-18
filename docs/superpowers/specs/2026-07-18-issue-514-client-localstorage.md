# Spec — issue #514: `client` localStorage primitive; move browser storage glue out of `web`

- Issue: [#514](https://github.com/jaunder-org/jaunder/issues/514)
- Milestone: _client crate: browser glue out of web_ (Milestone 14; ADR-0069)
- Date: 2026-07-18
- Status: proposed (awaiting approval)

## Goal

Give the wasm-only `client` crate a generic localStorage key/value primitive,
and rewire `web`'s three direct `web_sys::Storage` touchpoints to call it — so
no raw browser-storage glue remains in `web`, while the auth-marker's
sync-critical codec stays host-tested in `web`.

## Background / current state (verified against `main`)

`web` has exactly **three** direct `web_sys` localStorage sites (every other
`Storage` token in `web` is the `storage` crate's DB traits, unrelated):

1. `web/src/auth/marker.rs:35-62` — `storage()` + `read()` / `set()` /
   `clear()`, all `#[cfg(target_arch = "wasm32")]` `web_sys::Storage` glue over
   the pure marker codec.
2. `web/src/pages/mod.rs:92-99` — `App` restores the theme from `localStorage`
   (`get_item("jaunder_theme")`) on boot.
3. `web/src/pages/mod.rs:103-109` — `App` persists the theme to `localStorage`
   (`set_item("jaunder_theme", …)`) in an `Effect` on change.

The issue's looser wording ("theme/**seed** persistence", "App/**Topbar** … 3
sites") does not match the code: there is **no seed localStorage write-site**
anywhere in `web` (`jaunder_home_redirect` is _read-only_, in `PREPAINT_SCRIPT`
at `render/mod.rs:44`; nothing in Rust writes it), and the theme glue lives in
`App`, not `Topbar`. The three sites enumerated above are the complete set of
raw `web_sys::Storage` glue in `web`.

Structural facts that shape the design:

- `web/src/lib.rs:31` gates the **entire `pages` module**
  `#[cfg(target_arch = "wasm32")]`. So all seven marker call sites
  (`pages/auth.rs` ×3, `pages/ui.rs` ×4) and both theme sites are already
  wasm-only code.
- `marker.rs` is dual-target (host-compiled + host-tested) **only for its
  codec**: `MARKER_KEY`, `encode_marker`, `decode_marker`, and their three
  tests. The `storage()`/`read()`/`set()`/`clear()` fns are the wasm-gated part.
- **ADR-0044 sync invariant:** `MARKER_KEY` (`"jaunder_auth"`) + the
  `{"username":…}` JSON shape must stay byte-identical to
  `render::PREPAINT_SCRIPT` (`web/src/render/mod.rs:40-45`), the pre-paint
  `<head>` script. Keeping that codec host-tested in `web` is the explicit point
  of this split.
- **Charter (ADR-0069):** `client` is `#![cfg(target_arch = "wasm32")]` — empty
  rlib on host, active only on wasm; holds raw browser infrastructure (`web_sys`
  etc.), **never domain types**; depends on no workspace crate except
  `common`(+`macros`); `web`/`csr` depend on `client`, never the reverse; **pure
  logic never moves here** (stays host-testable in `web`/`common`); no fake host
  substitutes.

### Why the marker glue cannot fully move into `client`

`read`/`set`/`clear` pair the browser primitive with `MARKER_KEY` **and the
codec**. The codec must stay in `web` (host-tested, ADR-0044). `client` may not
depend on `web` (acyclic graph). Therefore only the **generic** primitive moves
to `client`; the marker's thin `read/set/clear` wrappers stay in `web` — still
wasm-gated, but relocated out of the codec file `marker.rs` so that file becomes
cfg-free. This is the issue's "callers that pair the primitive with their own
(host-tested) key + codec" path. The theme sites, by contrast, have no codec —
they become direct callers of the primitive.

## Decisions (from the design interview)

- **D1 — Primitive.** `client::storage` module, free fns:
  - `get(key: &str) -> Option<String>`
  - `set(key: &str, value: &str)` — best-effort, ignores errors
  - `remove(key: &str)` — best-effort, ignores errors

  Raw `String` key/value only (no domain types). Module inherits the crate-level
  wasm gate, so it carries no per-item `#[cfg]`. (Path is always written
  `client::storage::…`, so there is no collision with the workspace `storage`
  crate — which `client` does not depend on.)

- **D2 — `client` gains `web-sys`.** Add
  `web-sys = { workspace = true, features = ["Window", "Storage"] }` to
  `client/Cargo.toml` (its first dependency). No
  `common`/`js-sys`/`wasm-bindgen` needed: the primitive constructs no
  `JsValue`.
- **D3 — `web` depends on `client`.** Add `client = { path = "../client" }` to
  `web/Cargo.toml` `[dependencies]` (unconditional — empty rlib on host).
- **D4 — Remove `"Storage"` from `web`'s `web-sys` features** (manifest hygiene:
  `web` no longer references `web_sys::Storage` directly, so its manifest should
  stop declaring the feature). (Kept `web-sys` features for `web`: `Window`,
  `Document`, `Element`, `Location`, `File`, `FileList`, `FormData`,
  `HtmlInputElement`, `Request`, `RequestInit`, `RequestMode`, `Response` — i.e.
  all current features minus `Storage`.) **This is not a compile-time
  backstop:** Cargo unifies `web-sys` features across the workspace, and D2 has
  `client` enable `web-sys/Storage` while D3 makes `web` depend on `client`
  unconditionally, so `web_sys::Storage` remains _available_ to `web` on every
  target regardless of D4. The done-when is enforced by the AC1 `rg` check, not
  by this feature removal.
- **D5 — `marker.rs` becomes codec-only.** Strip
  `storage()`/`read()`/`set()`/`clear()`, the `target_arch` cfg, and the
  `web_sys` reference. Keep `MARKER_KEY`, `encode_marker`, `decode_marker`, and
  the three host tests. Zero `target_arch` cfg in the file.
- **D6 — Marker glue → new `web/src/auth/marker_storage.rs`.** A wasm-only
  module (`#[cfg(target_arch = "wasm32")] pub mod marker_storage;` in
  `auth/mod.rs`) holding `read()` / `set()` / `clear()`, each calling
  `client::storage::{get,set,remove}` paired with
  `super::marker::{MARKER_KEY, encode_marker, decode_marker}`. The seven call
  sites repoint from `crate::auth::marker::{read,set,clear}` to
  `crate::auth::marker_storage::{read,set,clear}`. `MARKER_KEY`/codec references
  (e.g. the pre-paint doc cross-refs) are unaffected. Also update the now-stale
  `auth/mod.rs:9-11` doc comment (which attributes `read`/`set`/`clear` to
  `pub mod marker`) to point at `marker_storage`.
- **D7 — Theme sites → direct primitive callers.** In `web/src/pages/mod.rs`
  `App`, replace the two `web_sys::window()…local_storage()` blocks with
  `client::storage::get("jaunder_theme")` /
  `client::storage::set("jaunder_theme", &val)`, preserving the existing
  non-empty guard on the restored value. The `"jaunder_theme"` literal is not
  shared with any other layer (unlike `MARKER_KEY`), so it stays a local
  literal.

## Acceptance criteria (observable)

1. **No raw storage glue in `web`.**
   `rg 'web_sys::Storage|local_storage\(' web/src` returns **zero** matches.
   This `rg` check is the enforcement (see D4: the `web-sys` feature removal is
   manifest hygiene, not a compile-time backstop — feature unification keeps
   `web_sys::Storage` available to `web` regardless).
2. **`marker.rs` is cfg-free.** `web/src/auth/marker.rs` contains no
   `target_arch` token and no `web_sys` reference; it holds only `MARKER_KEY` +
   the two codec fns + tests.
3. **Marker codec tests still run host-side.** The three `#[test]` fns in
   `marker.rs` (`round_trips_username`, `decode_rejects_malformed`,
   `encode_escapes_json`) run in the host `cargo nextest` build (unchanged, not
   wasm-gated).
4. **`client::storage` exists with the three fns** (`get`/`set`/`remove` over
   `String`), with no per-item `#[cfg]` and no domain types, and
   `client/Cargo.toml` declares `web-sys` with the `Window`+`Storage` features.
5. **Marker behavior preserved (e2e).** `end2end/tests/authed-flash.spec.ts`
   passes: a logged-in owner's reload marks `html.authed` +
   `data-user=<username>` (marker `set`→`read` round-trip through
   `client::storage`), and the anonymous case has no authed chrome.
6. **Theme behavior preserved (e2e).** `end2end/tests/theme.spec.ts` passes:
   `.j-root` carries a real `data-theme` after CSR hydration (the boot-time
   theme read path now routes through `client::storage`).
7. **ADR-0044 drift guard intact.** `render::PREPAINT_SCRIPT` and `MARKER_KEY`
   are unchanged; the existing `csr/index.html` drift-guard unit test in
   `render/mod.rs` passes.
8. **Full local gate green.** `cargo xtask check` (host static + clippy +
   coverage) and wasm-clippy (which lints `-p client`, the only place
   `client::storage` is compiled) pass; no coverage-gate regression (the
   relocated wrappers were already wasm-only / unmeasured; `client` is
   wasm-only, auto-admitted with zero measured lines).

## Non-goals / out of scope

- No change to `MARKER_KEY`, the JSON marker shape, or `PREPAINT_SCRIPT`.
- No move of the codec to `common` or `client` (it stays host-tested in `web` —
  the point of the split).
- No `sessionStorage`, no typed/JSON convenience layer, no error surfacing — the
  primitive is raw best-effort string KV, matching today's swallow-the-error
  behavior.
- Not touching the other `web` `web_sys` usages
  (`Window`/`Document`/`Location`/upload `File`/`FormData`/`Request…`) — those
  are later milestone issues (#516–#520).
- Not resolving the PR #508 overlap here; if #508 lands first, rebase this
  branch over its version of `marker.rs`.

## Risks

- **PR #508 overlap** (issue #315, open): also edits `marker.rs` (drops its cfg
  gate, `cov:ignore`s the web-sys wrappers in place — opposite direction). Not
  merging imminently. Mitigation: build against `main`; rebase over #508 if it
  lands first.
- **`client::storage` is wasm-only and thus not host-unit-testable.**
  Mitigation: behavior is e2e-verified (AC 5/6), per ADR-0069's charter that
  `client` is the e2e-verified home for browser glue. The pure, testable part
  (the codec) stays host-tested in `web`.
