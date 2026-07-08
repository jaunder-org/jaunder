# Spec — #303: converge `web` on the canonical co-located Leptos CSR layout

**Status:** design resolved; umbrella for a per-vertical migration. **Decision
record:** `docs/adr/0056-web-canonical-colocated-leptos.md`. **Supersedes
framing of:** #303 ("split the web crate"), which is reframed to this.
**Subsumes:** #299 (arg-struct / wire-shape work folds into posts/media).
**Supersedes mechanism of:** ADR-0055 (module-level wasm-only gating).

## Problem

`web` splits every feature across two homes by _technology_: `#[component]` UI
in `pages/<x>.rs` (gated `#[cfg(target_arch = "wasm32")]` wasm-only, ADR-0055)
and `#[server]` fns + wire types in `<x>/mod.rs`. This is non-idiomatic for a
Leptos CSR-with-server-functions app (cf. `examples/todo_app_sqlite_csr`, which
co-locates both in one file with **zero `target_arch` gates**), and the
segregation — a house style predating CSR — imposes a whole `pages/` gate
apparatus to accommodate a handful of genuinely-browser-only spots.

## Verified premises (why this is safe)

- **P1 — coverage.** The gate's `#[component]` exemption is purely syntactic
  (`xtask/src/coverage/exempt.rs`), not target-conditional. `feed_discovery.rs`
  already host-compiles ungated `#[component]`s at 0% host coverage, gate green.
  ⇒ host-compiling UI adds no coverage debt.
- **P2 — cfg surface.** Raw `js_sys`/`wasm_bindgen` use is two spots (`ui.rs`
  datetime #70, `upload.rs` fetch glue); `web-sys` already host-compiles; ~13/15
  feature pages touch no browser API. ⇒ co-location needs ~zero cfg.
- **P3 — layout.** `web` = canonical `app`, `csr` = `frontend`, `server` =
  `server`. The convergence is internal to `web`; no crate split, no consumer
  re-pointing, no server-fn re-registration, no leptos feature-unification cost.

## Target end state (acceptance floor for the umbrella)

Observable criteria; individual verticals inherit the relevant ones:

1. Each migrated feature's `#[component]` UI, `#[server]` fns, and wire types
   live in **one** feature module; no `pages/<x>.rs` counterpart remains for it.
2. No `#[cfg(target_arch = "wasm32")]` remains in `web/src` **except** at most a
   single localized gate on the `upload_file` leaf (or zero, if media chooses
   the multipart-`#[server]` route). The `#[cfg(target_arch)] pub mod pages`
   module gate is deleted.
3. The client/server split is expressed only via `feature`-gates + the
   `#[server]` macro. `#[component]` UI compiles under **both** the `csr` (wasm)
   and `server` (host) builds.
4. `cargo xtask validate` is green throughout — static + `wasm-clippy` +
   coverage (P1 holds: component bodies exempt) + the full e2e matrix.
5. No fake-value host stub is introduced for any wasm-only logic (ADR-0055
   principle retained); pure logic keeps its host-compiled, coverage-measured
   home.

## Shape of the work (details in the issues)

- **Prerequisite (blocks all verticals):** relocate the shared UI widgets
  (`pages/ui.rs` — `Topbar`, layout helpers) and the datetime helper (→
  `chrono`) to a host-compiled home, so co-located host-compiling pages can
  import them. Establishes and proves the pattern (gate stays green with a real
  co-located, host-compiling page).
- **Per-vertical issues (parallelizable once the prereq lands):** audiences,
  auth (localStorage marker), backup, cockpit, email, home, invites, media
  (`upload_file` decision), password_reset, posts (largest; compose form +
  datetime consumer), profile, sessions, site, timeline. Each is a **broad,
  human-directed cleanup** of its vertical, not a mechanical move — scope is
  deliberately open under the user's direction; the acceptance floor above is
  the invariant, not a ceiling.
- **Cleanup (blocked by all verticals):** move `App` + Router to the app entry,
  delete the empty `pages/` module and its `target_arch` gate, simplify the
  `wasm-clippy` step (host + Nix) to target the whole crate.
- **ADR:** promote `docs/adr/0056-web-canonical-colocated-leptos.md` at ship of
  the umbrella (or the prereq), setting ADR-0055 → superseded.

## Explicitly out of scope / dropped

- The #303 crate split (shared-pure / server-fn-API / wasm-client crates). If a
  crate boundary is ever wanted, canonical single-crate `web` is the base; the
  only high-value peel (pure `render` leaf for `server`) is a separate later
  call.
- Revisiting ADR-0040's CSR-vs-SSR rendering decision — out of scope; the
  canonical layout is achieved _within_ CSR.
