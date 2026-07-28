//! The home vertical (#319, ADR-0070): the routed `/` public Local-timeline
//! landing page. Module wiring only — a server-less vertical (no
//! `api.rs`/`server.rs`/`state.rs`): just the pure host-tested `render` masthead
//! twin and the wasm-only reactive `component`, which composes `crate::timeline`
//! and paints that masthead so it coincides with the projector's.

pub(crate) mod render;

#[cfg(target_arch = "wasm32")]
mod component;
#[cfg(target_arch = "wasm32")]
pub use component::HomePage;
