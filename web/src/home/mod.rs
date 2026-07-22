//! The home vertical (#319, ADR-0070): the routed `/` public Local-timeline
//! landing page. Module wiring only — a server-less, logic-free vertical (no
//! `api.rs`/`server.rs`/`state.rs`); its `component` composes `crate::timeline`
//! and the shared `crate::render` masthead.

#[cfg(target_arch = "wasm32")]
mod component;
#[cfg(target_arch = "wasm32")]
pub use component::HomePage;
