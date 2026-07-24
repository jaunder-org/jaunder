//! Sidebar — the left navigation chrome. The pure `render_sidebar` (server
//! projector, anonymous) and the reactive `Sidebar` (CSR client) are twins: the
//! same `<aside class="j-sidebar">` markup produced two ways. Co-located leaf
//! module (ADR-0070; mirrors `web::topbar`).

#[cfg(target_arch = "wasm32")]
mod component;
mod markup;

#[cfg(target_arch = "wasm32")]
pub use component::Sidebar;
pub(crate) use markup::render_sidebar;
