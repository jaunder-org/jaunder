//! Workspace proc-macros: a target-agnostic, host-compiled build-time crate — the home
//! for the workspace's proc-macros — distinct from the `common`/`host`/`client` runtime
//! trio.

use proc_macro::TokenStream;

/// Marks a **client-only reactive helper**: code that runs only in the browser (a
/// `server_resource` fetch, or an `Effect` that fires only client-side) and is exercised
/// by e2e, not host tests. It is an **identity** attribute — it expands to the annotated
/// item unchanged. Its sole purpose is to be a syntactic marker the coverage framework
/// (`xtask/src/coverage/exempt.rs`) recognizes and exempts, generalizing the `#[component]`
/// rule to non-component helpers (a macro-backed peer of the `cov:ignore` comment marker).
///
/// Interim until wasm-bindgen-test can cover these in a headless browser (Test-infra epic).
#[proc_macro_attribute]
pub fn client_only(_attr: TokenStream, item: TokenStream) -> TokenStream {
    item
}
