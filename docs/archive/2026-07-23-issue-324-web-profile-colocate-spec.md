# Spec — #324: converge the `profile` vertical onto the file-level host/wasm split

**Status:** awaiting approval. **Parent:** #303 (umbrella). **Decision
records:** `docs/adr/0070-web-vertical-wasm-only-component-files.md` (the
file-level host/wasm split, amended by #530 to put `#[server]` fns + wire DTOs
in `api.rs` and make `mod.rs` wiring-only), and `docs/adr/0065-*` (the
typed-wire-arg / client-validated direct-bind form pattern) for the
format-control modernization. Layout template: the already-shipped
`auth`/`media`/`registration` verticals and `docs/web-style-guide.md` §8. **No
new ADR** — this vertical applies existing decisions, it does not make novel
ones.

## Problem

The `profile` vertical is split across files by _technology_, not feature, and
its `mod.rs` is not yet wiring-only:

- `web/src/profile/mod.rs` (114 lines) — holds the `ProfileData` wire DTO, the
  vertical's grouped `#[cfg(feature = "server")]` use-block, all four
  `#[server]` fns (`get_profile`, `update_profile`, `get_default_post_format`,
  `set_default_post_format`), and a `#[cfg(test)]` wire-decode test. Under
  ADR-0070 (amended #530) this content belongs in `api.rs`; `mod.rs` should be
  module wiring + re-exports only.
- `web/src/pages/profile.rs` (155 lines) — the `#[component]` UI (`ProfilePage`
  - the private `DefaultPostFormatControl`). The old `pages/` home; imports
    `Topbar` from the `crate::pages::` re-export shim rather than its canonical
    `crate::topbar` home.

So the feature's UI and server fns live in separate homes — the split ADR-0070
eliminates. Separately, the profile form was already migrated to the ADR-0065
typed direct-bind pattern (`Field<T>` + `.dispatch`), but the co-located
`DefaultPostFormatControl` still uses the older `<ActionForm>` + stringly
`<select name="format">` submit path — an inconsistency this convergence closes.

## Decisions (interview-resolved)

1. **Three-file vertical, no `server.rs`, no extraction file.** Profile has no
   profile-specific host-only support (its `#[server]` bodies call shared
   `require_auth`/storage traits inline) and no substantial pure signal logic in
   the component worth extracting for host testing. So the vertical converges to
   the `mod.rs` / `api.rs` / `component.rs` shape (as `media`/`email`/
   `registration` already are), _not_ the four-file `posts` shape.
2. **Modernize `DefaultPostFormatControl` to the ADR-0065 typed direct-bind
   pattern.** Beyond the mechanical split, the format control is converted from
   `<ActionForm>` + string-select to a `RwSignal<PostFormat>` bound to a
   `<select>` whose `on:change` parses the token to `PostFormat`, with a plain
   `type="button"` "Save" that dispatches the typed `SetDefaultPostFormat`
   action — mirroring the audience select in `posts/component.rs` and the
   profile form's own Update button. This is the invariant floor + this one
   authorized cleanup; no other behavior changes.

## Target end state (acceptance floor — observable criteria)

1. **Co-located, `pages/` home gone.** `profile`'s UI, `#[server]` fns, and wire
   types live under `web/src/profile/`; **no `web/src/pages/profile.rs`
   remains**, and the `pub mod profile;` at `web/src/pages/mod.rs:1` is deleted.
2. **`mod.rs` is wiring only.** `web/src/profile/mod.rs` contains only an
   ADR-0070-style `//!` doc, module declarations, and re-exports — **no
   `ProfileData`, no `#[server]` fn, no test, no server use-block** of its own.
3. **`api.rs` holds the endpoints + DTO + test.** `web/src/profile/api.rs`
   contains `ProfileData`, the four `#[server]` fns, the vertical's single
   grouped `#[cfg(feature = "server")]` use-block, and the wire-decode unit test
   (see criterion 8). These are dual-compiled (host + wasm) and stay
   coverage-measured on host.
4. **`component.rs` is wasm-only, cfg-free inside.**
   `web/src/profile/component.rs` holds `ProfilePage` +
   `DefaultPostFormatControl`, declared
   `#[cfg(target_arch = "wasm32")] mod component;` on the `mod` line only —
   **zero cfg gates inside the file**; the components **do not host-compile**
   and are **not** dead-but-exempt. **No `cov:ignore` and no
   `#[component]`-exemption reliance is added** to satisfy host compilation of
   UI.
5. **`target_arch` gate only on wiring lines.** `target_arch = "wasm32"` appears
   in the vertical **only** on the `mod component;` declaration and its paired
   `pub use component::ProfilePage;` — never on an item inside a leaf file
   (ADR-0070 §2).
6. **Stable public paths.** Re-exports keep `crate::profile::*` paths stable:
   `web/src/email/component.rs`'s `use crate::profile::get_profile;` and the
   server-fn registrar paths compile **unchanged**; the router import in
   `web/src/pages/mod.rs` reads `use crate::profile::ProfilePage;`, backed by
   the gated `pub use component::ProfilePage;`. The `<Route>` line for
   `/profile` is otherwise unchanged (the sibling `/profile/email` → `EmailPage`
   route is untouched — it belongs to the `email` vertical).
7. **Canonical `Topbar` import.** The component imports the shared `Topbar` from
   `crate::topbar::Topbar` (the canonical home used by `auth`/`media`
   `component.rs`), not the `crate::pages::` re-export shim.
8. **Format control modernized (Decision 2).** `DefaultPostFormatControl`
   renders a `<select id="default-post-format" class="j-field-val">` whose
   `<option>`s use `value=<PostFormat>.as_str()` with a reactive `selected`, an
   `on:change` that parses the selected token to `PostFormat` (invalid tokens
   ignored), and a plain `type="button"` "Save" that dispatches
   `SetDefaultPostFormat { format }` — mirroring the audience
   `<select id="audience-base" class="j-field-val">` hook convention in
   `posts/component.rs`. **No `<ActionForm>` and no `<select name="format">`
   string-submit path remain** in the vertical; the stable e2e hook is the `id`,
   not a form `name`.
9. **Endpoint decode contract preserved and tested.** The `api.rs` unit test
   still asserts `set_default_post_format`'s wire type accepts a valid
   `format=<token>` and **rejects a bogus token at decode** (the endpoint's
   Url-codec contract is independent of the client widget); its comment is
   updated to describe the typed-dispatch path rather than `<ActionForm>`.
10. **e2e green and honest.** `end2end/tests/profile.spec.ts`'s #498 test
    round-trips the selected default post format (org ↔ markdown) through
    `set_default_post_format`/`get_default_post_format`, with its
    `FORMAT_SELECT` locator updated to `select#default-post-format` (criterion
    8's `id` hook) and its comment/title describing the direct-bind dispatch
    (not `<ActionForm>`); the Save locator stays `button:has-text("Save")`. All
    other profile e2e tests (display-name persist/over-long/clear #401, bio
    persist/over-long/clear #545) pass **unchanged**.
11. **Gate green.** `cargo xtask validate` is green — static + wasm-clippy +
    coverage + the full e2e matrix — including the profile flows. Because the
    components are now wasm-only, `wasm-clippy` is load-bearing gate surface for
    this vertical's UI type-checking (ADR-0070 §Consequences).

## Shape of the work

- **`api.rs`:** move `ProfileData`, the grouped server use-block, the four
  `#[server]` fns, and the wire-decode test out of `mod.rs` into a new
  `web/src/profile/api.rs`. Bodies move essentially verbatim.
- **`component.rs`:** move `ProfilePage` + `DefaultPostFormatControl` out of
  `web/src/pages/profile.rs` into a new wasm-only
  `web/src/profile/component.rs`; repoint `Topbar` to `crate::topbar::Topbar`;
  rewrite `DefaultPostFormatControl` to the direct-bind pattern (Decision 2).
  Bodies otherwise move unchanged.
- **`mod.rs`:** reduce to wiring — `//!` doc + `mod api;`,
  `#[cfg(target_arch = "wasm32")] mod component;`, and the re-exports
  (`pub use api::{ProfileData, get_profile, GetProfile, update_profile, UpdateProfile, get_default_post_format, GetDefaultPostFormat, set_default_post_format, SetDefaultPostFormat};`
  and the gated `pub use component::ProfilePage;`), trimmed to exactly what
  external call sites and the registrar need.
- **Rewire `pages/mod.rs`:** delete `pub mod profile;`; delete
  `web/src/pages/profile.rs`; change the router import to
  `use crate::profile::ProfilePage;`.
- **e2e + test comments:** update `profile.spec.ts` #498
  (comment/title/selector) and the `api.rs` unit-test comment to the direct-bind
  reality.
- **Module docs:** ADR-0070-style `//!` doc on `profile/mod.rs`.

## Out of scope

- Moving `App`/Router out of `pages/mod.rs` to the app entry — that is **#330**.
- Dissolving `pages/ui.rs` / `web::render` — that is **#312**. This issue only
  stops _importing_ profile's UI from `pages/` and stops using the
  `crate::pages::Topbar` shim; it does not remove the `pages/mod.rs` re-export
  shim itself.
- The `email` vertical and the `/profile/email` → `EmailPage` route (a separate
  vertical, already converged).
- Any change to the profile `#[server]` fn semantics or storage calls beyond
  what the file move forces.

## Verification

`cargo xtask validate` (static + wasm-clippy + coverage + full e2e matrix). The
load-bearing behavioral checks are the profile e2e flows: display-name and bio
persist/over-long/clear (#401/#545) and the default-post-format round-trip
(#498), each driven in a real browser. `wasm-clippy` is load-bearing for the
now-wasm-only UI. Because criterion 9 keeps a host unit test on the endpoint
decode contract, the format-control modernization cannot silently weaken the
endpoint's rejection of bogus tokens.
