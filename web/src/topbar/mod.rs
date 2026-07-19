//! Topbar — the chrome bar. The pure `render` (server projector) and the
//! reactive `Topbar` (CSR client) are twins: the same `<div class="j-topbar">`
//! markup produced two ways. Co-located per ADR-0056.

#[cfg(target_arch = "wasm32")]
mod component;
mod markup;

#[cfg(target_arch = "wasm32")]
pub use component::Topbar;
pub(crate) use markup::render;
