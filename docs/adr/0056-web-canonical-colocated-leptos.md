# ADR-0056: web converges on the canonical co-located Leptos CSR layout

- Status: superseded
- Note: superseded by ADR-0070 (web verticals split host/wasm at the file level)
  — co-location retained; the feature-only / dead-but-exempt gating reversed
- Date: 2026-07-07
- Issue: [#303](https://github.com/jaunder-org/jaunder/issues/303)

## Context

ADR-0055 (#300) drew the `web` host/wasm boundary at the **module level**:
`pages/` (the Leptos `#[component]` UI) compiles
`#[cfg(target_arch = "wasm32")]` wasm-only, gated once at `lib.rs`, deleting ~20
per-line `target_arch` stub arms and the fake-value host stubs they invited.
#303 then proposed taking that module partition further into a **crate split**
(a shared-pure crate + a server-fn API crate + a wasm-only client crate).

Investigating #303 surfaced a stronger, cheaper direction. Three facts, each
verified against the source rather than assumed:

1. **This project's architecture already _is_ the canonical Leptos CSR-with-
   server-functions shape**, one step off. Leptos's own
   `examples/todo_app_sqlite_csr` — "client-side rendering with server functions
   … without server-side rendering and hydration", exactly ADR-0040/0041 — is
   **one crate** that compiles two ways by _cargo feature_ (`csr` → wasm client
   via `hydrate()`; `ssr` → the axum server). Its `todo.rs` **co-locates** the
   `#[component]` UI, the `#[server]` fns, and the shared wire types in one
   file, with a total cfg footprint of one `#[cfg(feature = "ssr")] mod ssr` for
   server-only imports plus a `cfg_attr` derive — **zero
   `#[cfg(target_arch)]`**. The client/server split is carried entirely by
   `feature = "ssr"` vs `csr` and the `#[server]` macro, never by `target_arch`.
   Our `web` = that `app` crate, `csr` = its `frontend`, `server` = its
   `server`.

2. **Nothing forces our `target_arch` gating; co-location does not require
   cfg.** Our `pages/<x>.rs` component files import their feature's server fns
   and wire types from the sibling `<x>/mod.rs` (e.g. `pages/audiences.rs` pulls
   11 items from `crate::audiences`) — two halves of one feature split by
   _technology_. The components themselves (`AudiencesPage`, …) use only Leptos
   reactive primitives (`ServerAction`, `ActionForm`, `Effect`, `RwSignal`,
   `server_resource`); ~13 of ~15 feature pages touch **no** browser API. Raw
   target-gated `js_sys`/`wasm_bindgen` use is confined to **two** spots — the
   `js_sys::Date` datetime helper (`ui.rs`, #70) and the `upload_file` fetch
   glue (`upload.rs`). `web-sys` is not target-gated and already host-compiles,
   so the localStorage marker's gate is defensive, not required. The `pages/`
   segregation predates CSR; it is a house style (organize by technology), not a
   Leptos or CSR constraint.

3. **Host-compiling the `#[component]` UI reintroduces no coverage debt.** The
   coverage gate's `#[component]` exemption (ADR-0050) is **purely syntactic** —
   `xtask/src/coverage/exempt.rs` exempts any fn whose attribute path is
   `component`, with no `target_arch`/feature/path condition.
   `web/src/ feed_discovery.rs` already ships **ungated** `#[component]`s that
   host-compile with 0% host coverage while the gate stays green — a standing
   natural experiment proving the exemption holds host-side.

The #303 crate split, by contrast, moves _away_ from the canonical shape (three
crates, not one) and carries irreducible build-wiring cost: leptos feature
unification across crates, relocating the `#[server]` macro + `boundary!` +
shared types, re-registering server fns, and re-pointing the projector / `csr` /
cargo-flow / Nix / xtask.

## Decision

`web` converges on the **canonical co-located Leptos CSR layout**, reversing
ADR-0055's module-level wasm-only gating. This is done **inside the existing
`web` crate** — no crate split, no new crates, no consumer re-pointing.

1. **Co-locate each feature.** A feature's `#[component]` UI, its `#[server]`
   fns, and its shared wire types live together in that feature's module
   (`audiences/`, `posts/`, …), the way `todo_app_sqlite_csr::todo` does — not
   split across `pages/<x>.rs` and `<x>/mod.rs`.

2. **The split is by cargo feature, never `target_arch`.** Server-only code sits
   behind `feature = "server"` (our name for `ssr`); the `#[server]` macro
   handles the client/server halves. `#[component]` UI is **ungated** and
   compiles for both targets (dead but exempt on the host, live on wasm) — as
   `feed_discovery.rs` already does.

3. **`pages/` and its module gate are deleted.** Shared UI widgets
   (`pages/ui.rs` — `Topbar`, …) move to a host-compiled home first, since a
   host-compiling co-located page must be able to import them; the `App` +
   Router shell moves to the crate's app entry. The
   `#[cfg(target_arch = "wasm32")] pub mod pages` gate goes away.

4. **The two real browser touchpoints are made dual-target-clean, not gated.**
   The datetime helper moves to `chrono` (`wasmbind`); the localStorage marker
   routes through host-compiling `web-sys` inside client effects. `upload_file`
   is the sole genuine wasm-only leaf (`wasm_bindgen_futures` + browser
   `fetch`): either localize a **single** `#[cfg(target_arch = "wasm32")]` on
   that one helper, or convert `/media/upload` to a multipart `#[server]` fn
   (the zero-cfg, fully-canonical option) — decided in the media vertical.

ADR-0055's surviving principles are retained: **pure logic keeps a
host-compiled, coverage-measured home, and no fake-value host stub is ever
substituted for wasm-only logic.** What changes is the mechanism for UI: instead
of gating it wasm-only, it host-compiles as exempt `#[component]` dead code.

The #303 **crate split is dropped** as the goal; if a crate boundary is ever
wanted later, the canonical single-crate `web` = `app` is the correct base for
it, and the only high-value peel (a pure `render` leaf so `server` need not
depend on all of leptos-csr `web`) can be revisited on its own merits.

## Consequences

- Supersedes ADR-0055's central decision (module-level wasm-only gating →
  dual-target co-location). Amends ADR-0040/0041's layout note toward the
  canonical `app`/`frontend`/`server` mapping. Retains ADR-0050's syntactic
  `#[component]` exemption (now load-bearing on the host) and ADR-0055's
  no-fake-stub / relocate-pure-logic rules.
- The work lands as a **human-directed, per-vertical migration** (one issue per
  feature) behind an umbrella, gated by a prerequisite that relocates the shared
  UI widgets + datetime helper to a host home, and closed by a cleanup that
  deletes `pages/` and the module gate. The vertical issues are deliberately
  **broad cleanup opportunities**, not narrow mechanical moves.
- `web` host build now compiles the reactive UI (dead, `#[component]`-exempt) —
  a modest host-compile cost accepted in exchange for the idiomatic layout and
  the deletion of the segregation.
- The `wasm-clippy` gate step (host + Nix) still applies — `web` still compiles
  to wasm for the client — but targets the whole crate uniformly rather than a
  `pages`-only surface.
- #303 is reframed from "split the web crate" to this convergence; #299's
  arg-struct / wire-shape work folds naturally into the posts/media verticals.
