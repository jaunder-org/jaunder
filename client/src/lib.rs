//! `client` — strictly-client (wasm/browser) shared infrastructure.
//!
//! The symmetric wasm peer of `host`: holds only raw browser glue
//! (`web_sys` / `js_sys` / `wasm_bindgen` / wasm-side leptos plumbing) and
//! never our domain types. Depends on no workspace crate except `common`
//! (+ `macros`). `web`/`csr` depend on `client`, never the reverse.
//!
//! Wasm-only: the crate-level `#![cfg(target_arch = "wasm32")]` below makes it
//! an empty rlib on the host target (zero coverage-measured lines) and active
//! only on wasm. Every module relocated here inherits that gate, so it needs no
//! per-item `#[cfg]` and no `#[client_only]` marker.
//!
//! See docs/adr/0069-client-crate-wasm-only-home.md.
#![cfg(target_arch = "wasm32")]

/// Generic browser `localStorage` key/value primitive (#514). Raw string KV, no
/// domain types — the `web`/`csr` home for what were scattered `web_sys::Storage`
/// call sites.
pub mod storage;

/// Raw browser confirm-dialog primitive (`window.confirm`) relocated from `web` (#516).
/// `web-sys` only, no domain types — unconditional (no `csr` gate).
pub mod dialog;

/// Generic browser DOM primitives (`text_content_by_id`, `remove_element_by_id`) —
/// raw `web_sys`, no domain types. The CSR boot reads the projector seed blob and
/// drops the server-painted `#app` through these (#519).
pub mod dom;

/// Reactive revalidation helpers — the browser-bound `Effect`/`Resource` plumbing behind
/// `web`'s `Invalidator` idiom, relocated here (#515). Behind the `csr` feature because they
/// need `leptos`; a host/server build of `client` stays leptos-free.
#[cfg(feature = "csr")]
pub mod reactive;
