//! `client` — strictly-client (wasm/browser) shared infrastructure.
//!
//! The symmetric wasm peer of `host`: holds only raw browser glue
//! (`web_sys` / `js_sys` / `wasm_bindgen` / wasm-side leptos plumbing) and
//! never our domain types. Depends on no workspace crate except `common`
//! (+ `macros`). `web`/`csr` depend on `client`, never the reverse.
//!
//! Browser-bound modules carry `#[cfg(target_arch = "wasm32")]`, so their glue
//! contributes no host coverage. Two modules also compile host-testable
//! contracts: [`perf`] owns its mark-name table, while [`telemetry`] owns the
//! transport-independent one-flight state machine. Their browser calls remain
//! behind module-wiring cfgs.
//!
//! See docs/adr/0069-client-crate-wasm-only-home.md.

/// Generic browser `localStorage` key/value primitive (#514).
#[cfg(target_arch = "wasm32")]
pub mod storage;

/// Bounded swallowed-error reporting. The transport-independent one-flight
/// state machine host-compiles; its fetch adapter is wasm-only.
pub mod telemetry;

/// Raw browser confirm-dialog primitive (`window.confirm`, #516).
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
/// no `web_sys` type.
#[cfg(all(target_arch = "wasm32", feature = "csr"))]
pub mod upload;
