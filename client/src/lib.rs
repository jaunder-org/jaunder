//! `client` — strictly-client (wasm/browser) shared infrastructure.
//!
//! The symmetric wasm peer of `host`: holds only raw browser glue
//! (`web_sys` / `js_sys` / `wasm_bindgen` / wasm-side leptos plumbing) and
//! never our domain types. Depends on no workspace crate except `common`
//! (+ `macros`). `web`/`csr` depend on `client`, never the reverse.
//!
//! Wasm-only: every module that touches the browser carries
//! `#[cfg(target_arch = "wasm32")]`, so on the host this is an all-but-empty
//! rlib (zero coverage-measured lines from the browser glue). The one exception
//! is [`perf`], whose mark-name contract is plain `&str` data and is therefore
//! host-testable — it compiles on both targets, with the browser call behind its
//! own `#[cfg]`.
//!
//! See docs/adr/0069-client-crate-wasm-only-home.md.

/// Generic browser `localStorage` key/value primitive (#514). Raw string KV, no
/// domain types — the single `web`/`csr` home for `web_sys::Storage` access.
#[cfg(target_arch = "wasm32")]
pub mod storage;

/// Raw browser confirm-dialog primitive (`window.confirm`, #516).
/// `web-sys` only, no domain types — unconditional (no `csr` gate).
#[cfg(target_arch = "wasm32")]
pub mod dialog;

/// Generic browser DOM primitives (`text_content_by_id`, `remove_element_by_id`) —
/// raw `web_sys`, no domain types. The CSR boot reads the projector seed blob and
/// drops the server-painted `#app` through these (#519).
#[cfg(target_arch = "wasm32")]
pub mod dom;

/// `performance.mark` names and emitter for the CSR boot phases (#794). Not
/// wasm-gated as a whole: the names are the cross-language contract the e2e
/// harness discovers by prefix, and they are pinned by host tests.
pub mod perf;

/// Reactive revalidation helpers — the browser-bound `Effect`/`Resource` plumbing behind
/// `web`'s `Invalidator` idiom (#515). Behind the `csr` feature because they
/// need `leptos`; a host/server build of `client` stays leptos-free.
#[cfg(all(target_arch = "wasm32", feature = "csr"))]
pub mod reactive;

/// Browser file-picker → `MultipartData` glue (#520), living here so `web` names
/// no `web_sys` type. Behind `csr` because it needs leptos's `NodeRef`.
#[cfg(all(target_arch = "wasm32", feature = "csr"))]
pub mod upload;
