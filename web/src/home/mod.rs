//! The home vertical (#319, ADR-0070): the routed `/` public Local-timeline
//! landing page. Module wiring only — its host-compiled `render` masthead twin
//! and `page_state` public-navigation fold keep projector/CSR rendering logic
//! independently testable, while the wasm-only `component` composes
//! `crate::timeline` and paints that masthead.

mod page_state;
pub(crate) mod render;
pub use page_state::site_destination;

#[cfg(target_arch = "wasm32")]
mod component;
#[cfg(target_arch = "wasm32")]
pub use component::HomePage;
