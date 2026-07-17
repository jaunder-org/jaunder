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
//! See docs/adr/drafts/client-crate-wasm-only-home.md.
#![cfg(target_arch = "wasm32")]
